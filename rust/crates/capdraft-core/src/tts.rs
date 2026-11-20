//! Batch TTS through CapCut's `common_task` API, with the audio cache
//! (port of `core/capcut_tts.py`, `CapCutTtsWrapper`).
//!
//! Captions are split into chunks (one `tts-new` task each, with a fresh random
//! device id), chunks run in parallel, each task is polled with backoff, and the
//! audio files are downloaded in parallel into the cache. The network goes
//! through [`Transport`] so tests can serve it.

use crate::cache::TtsCache;
use crate::capcut_client::{DeviceProfile, Request, build_request_for, device_for, load_device};
use crate::config::AppConfig;
use crate::ids::randint;
use crate::json::{Object, Value};
use crate::pyfmt::py_strip;
use crate::pyval::{PyResult, str_of, to_float, truthy};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// An HTTP response as the TTS code needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub reason: String,
    pub body: Vec<u8>,
}

impl HttpResponse {
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// The network.
pub trait Transport: Send + Sync {
    /// # Errors
    /// Connection, TLS and timeout errors (not HTTP error statuses).
    fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
        timeout: Duration,
    ) -> Result<HttpResponse, String>;
    /// # Errors
    /// Connection, TLS and timeout errors (not HTTP error statuses).
    fn get(&self, url: &str, timeout: Duration) -> Result<HttpResponse, String>;
}

/// The real network, through `ureq`.
/// Retries of an HTTP call that failed below HTTP (connection, TLS, timeout); see ADR-023.
pub const TRANSPORT_RETRIES: u32 = 2;

/// How many times the request timeout a response body may take (see `UreqTransport::agent`).
pub const BODY_TIMEOUT_FACTOR: u32 = 10;

#[derive(Debug, Clone, Default)]
pub struct UreqTransport;

impl UreqTransport {
    /// `requests`' `timeout=t` bounds the connect and every socket read, not the whole
    /// request: a slow but flowing download never times out. ureq has no per-read timer, so
    /// each phase gets `t` (connect, send, wait for the response head) and the body gets
    /// `BODY_TIMEOUT_FACTOR * t` as a guard against a stalled transfer.
    fn agent(timeout: Duration) -> ureq::Agent {
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(timeout))
            .timeout_send_request(Some(timeout))
            .timeout_send_body(Some(timeout))
            .timeout_recv_response(Some(timeout))
            .timeout_recv_body(Some(timeout * BODY_TIMEOUT_FACTOR))
            .build()
            .into()
    }

    fn finish(
        resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<HttpResponse, String> {
        let mut resp = resp.map_err(|e| e.to_string())?;
        let status = resp.status();
        let body = resp
            .body_mut()
            .with_config()
            .limit(256 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| e.to_string())?;
        Ok(HttpResponse {
            status: status.as_u16(),
            reason: status.canonical_reason().unwrap_or("").to_owned(),
            body,
        })
    }
}

impl Transport for UreqTransport {
    fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
        timeout: Duration,
    ) -> Result<HttpResponse, String> {
        let mut req = Self::agent(timeout).post(url);
        for (k, v) in headers {
            // ureq negotiates and decodes compression itself.
            if !k.eq_ignore_ascii_case("accept-encoding") {
                req = req.header(k.as_str(), v.as_str());
            }
        }
        Self::finish(req.send(body))
    }

    fn get(&self, url: &str, timeout: Duration) -> Result<HttpResponse, String> {
        Self::finish(Self::agent(timeout).get(url).call())
    }
}

/// The result of one caption.
#[derive(Debug, Clone, PartialEq)]
pub struct TtsItemResult {
    pub audio_path: String,
    /// Seconds; `None` when unknown (a cache hit without metadata and no ffprobe).
    pub duration: Option<f64>,
    /// `Cached`, `Downloaded` or `Failed`.
    pub status: &'static str,
    pub error: Option<String>,
}

impl TtsItemResult {
    fn failed(error: &str) -> Self {
        Self {
            audio_path: String::new(),
            duration: Some(0.0),
            status: "Failed",
            error: Some(error.to_owned()),
        }
    }
}

/// Results by caption index, in the order they arrived (a dict in the original).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Results {
    order: Vec<(i64, TtsItemResult)>,
    pos: HashMap<i64, usize>,
}

impl Results {
    pub fn insert(&mut self, idx: i64, r: TtsItemResult) {
        match self.pos.get(&idx) {
            Some(&i) => self.order[i].1 = r,
            None => {
                self.pos.insert(idx, self.order.len());
                self.order.push((idx, r));
            }
        }
    }
    pub fn extend(&mut self, other: Self) {
        for (k, v) in other.order {
            self.insert(k, v);
        }
    }
    #[must_use]
    pub fn get(&self, idx: i64) -> Option<&TtsItemResult> {
        self.pos.get(&idx).map(|&i| &self.order[i].1)
    }
    #[must_use]
    pub fn contains(&self, idx: i64) -> bool {
        self.pos.contains_key(&idx)
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = &(i64, TtsItemResult)> {
        self.order.iter()
    }
}

/// The `tts_*` settings of `config.json`.
#[derive(Debug, Clone)]
pub struct TtsSettings {
    pub chunk_size: usize,
    pub parallel_chunks: usize,
    pub poll_interval_sec: f64,
    pub download_workers: usize,
    pub default_voice: String,
    pub default_resource_id: String,
    pub default_rate: f64,
    pub cache_dir: PathBuf,
    pub projects_dir: PathBuf,
    pub device_json_path: PathBuf,
    pub ffprobe_path: String,
    pub device_profile: DeviceProfile,
}

impl TtsSettings {
    #[must_use]
    pub fn from_config(c: &AppConfig) -> Self {
        let poll = match c.get("tts_poll_interval_sec") {
            Some(v) if truthy(v) => to_float(v).unwrap_or(1.0),
            _ => 1.0,
        };
        Self {
            chunk_size: c.int("tts_chunk_size", 15).max(1) as usize,
            parallel_chunks: c.int("tts_parallel_chunks", 2).max(1) as usize,
            poll_interval_sec: poll,
            download_workers: c.int("tts_download_workers", 8).max(1) as usize,
            default_voice: c.default_voice(),
            default_resource_id: c.default_resource_id(),
            default_rate: c.default_rate(),
            cache_dir: c.cache_dir(),
            projects_dir: c.projects_dir(),
            device_json_path: c.device_json_path(),
            ffprobe_path: c.ffprobe_path(),
            device_profile: DeviceProfile::parse(
                &c.get("tts_device_profile").map(str_of).unwrap_or_default(),
            ),
        }
    }
}

type ChunkQueue = Arc<Mutex<std::collections::VecDeque<Vec<(i64, String)>>>>;
pub type ItemCallback = Arc<dyn Fn(i64, &TtsItemResult) + Send + Sync>;
pub type CancelCallback = Arc<dyn Fn() -> bool + Send + Sync>;
pub type SleepFn = Arc<dyn Fn(Duration) + Send + Sync>;

/// `CancelledError` of the original: stop quietly.
const CANCELLED: &str = "CancelledError: Cancelled by user";

#[derive(Clone)]
pub struct CapCutTts {
    pub settings: TtsSettings,
    pub project_name: String,
    pub cache: TtsCache,
    transport: Arc<dyn Transport>,
    sleep: SleepFn,
}

struct Job<'a> {
    voice: &'a str,
    resource_id: &'a str,
    rate: f64,
    rate_text: String,
    cancelled: &'a CancelCallback,
    on_item: &'a Option<ItemCallback>,
}

impl Job<'_> {
    fn is_cancelled(&self) -> bool {
        (self.cancelled)()
    }
    fn emit(&self, idx: i64, r: &TtsItemResult) {
        if let Some(cb) = self.on_item {
            cb(idx, r);
        }
    }
}

fn numeric_id(len: usize) -> String {
    let mut s = randint(1, 9).to_string();
    for _ in 1..len {
        s.push_str(&randint(0, 9).to_string());
    }
    s
}

/// Python's `str(e)` for errors this crate words as `"Type: message"`.
fn exc_str(e: &str) -> String {
    match e.split_once(": ") {
        Some((ty, msg))
            if !ty.is_empty()
                && ty
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') =>
        {
            msg.to_owned()
        }
        _ => e.to_owned(),
    }
}

impl CapCutTts {
    /// # Errors
    /// When the cache folder cannot be created.
    pub fn new(
        settings: TtsSettings,
        project_name: &str,
        transport: Arc<dyn Transport>,
    ) -> PyResult<Self> {
        let cache = TtsCache::new(&settings.cache_dir)?;
        Ok(Self {
            settings,
            project_name: project_name.to_owned(),
            cache,
            transport,
            sleep: Arc::new(std::thread::sleep),
        })
    }

    /// Replace the waits between polls and chunks (tests).
    #[must_use]
    pub fn with_sleep(mut self, sleep: SleepFn) -> Self {
        self.sleep = sleep;
        self
    }

    fn project_dir(&self, parts: &[&str]) -> PathBuf {
        let mut p = self.settings.projects_dir.join(&self.project_name);
        for part in parts {
            p = p.join(part);
        }
        p
    }

    fn copy_to_project_tts(&self, source: &Path, dest_name: &str) -> PyResult<PathBuf> {
        let dir = self.project_dir(&["tts", "audio"]);
        std::fs::create_dir_all(&dir).map_err(|e| crate::project_split::io_err(&e, &dir))?;
        let dest = dir.join(dest_name);
        if crate::paths::resolve(source) != crate::paths::resolve(&dest) {
            crate::project_split::copy_file(source, &dest)
                .map_err(|e| crate::project_split::io_err(&e, source))?;
        }
        Ok(dest)
    }

    /// Duration in seconds from `ffprobe`, `None` when it cannot be read.
    fn audio_duration(&self, path: &Path) -> Option<f64> {
        let out = std::process::Command::new(&self.settings.ffprobe_path)
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "json",
            ])
            .arg(path)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let v = crate::json::parse(&String::from_utf8_lossy(&out.stdout)).ok()?;
        to_float(&v["format"]["duration"]).ok().or(Some(0.0))
    }

    /// TTS for `items` (`(caption index, text)`): cache hits first, the rest
    /// through CapCut in chunks. `on_item` fires for every result as it lands.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the original's keyword arguments"
    )]
    pub fn generate_tts_batch(
        &self,
        items: &[(i64, String)],
        voice: Option<&str>,
        resource_id: Option<&str>,
        rate: Option<f64>,
        cancelled: &CancelCallback,
        on_item: &Option<ItemCallback>,
        use_cache: bool,
    ) -> Results {
        let voice = voice
            .filter(|v| !v.is_empty())
            .unwrap_or(&self.settings.default_voice)
            .to_owned();
        let resource_id = resource_id
            .filter(|v| !v.is_empty())
            .unwrap_or(&self.settings.default_resource_id)
            .to_owned();
        let rate = rate.unwrap_or(self.settings.default_rate);
        let job = Job {
            voice: &voice,
            resource_id: &resource_id,
            rate,
            rate_text: crate::pyval::float_str(rate),
            cancelled,
            on_item,
        };

        let mut results = Results::default();
        let mut pending: Vec<(i64, String)> = Vec::new();
        for (idx, text) in items {
            let (file, dur) = if use_cache {
                self.cache.get_cached_file(text, &voice, rate, &resource_id)
            } else {
                (None, None)
            };
            if let Some(file) = file {
                let path = if self.project_name.is_empty() {
                    Ok(file)
                } else {
                    self.copy_to_project_tts(&file, &format!("tts_{idx:06}.mp3"))
                };
                let Ok(path) = path else { continue };
                let duration = dur.or_else(|| self.audio_duration(&path));
                let r = TtsItemResult {
                    audio_path: path.to_string_lossy().into_owned(),
                    duration,
                    status: "Cached",
                    error: None,
                };
                results.insert(*idx, r.clone());
                job.emit(*idx, &r);
            } else if !py_strip(text).is_empty() {
                pending.push((*idx, text.clone()));
            }
        }
        if pending.is_empty() {
            return results;
        }

        let chunks: Vec<Vec<(i64, String)>> = pending
            .chunks(self.settings.chunk_size)
            .map(<[_]>::to_vec)
            .collect();
        let parallel = self.settings.parallel_chunks;
        if parallel > 1 && chunks.len() > 1 {
            self.run_parallel(chunks, &job, &mut results);
            return results;
        }
        let n = chunks.len();
        for (i, chunk) in chunks.iter().enumerate() {
            if job.is_cancelled() {
                return results;
            }
            results.extend(self.process_chunk(chunk, &job));
            if i + 1 < n {
                (self.sleep)(Duration::from_millis(500));
            }
        }
        results
    }

    /// Chunks on `min(parallel, chunks)` threads; on cancel, unstarted chunks are
    /// dropped and the call returns at once (in-flight chunks finish on their own).
    fn run_parallel(&self, chunks: Vec<Vec<(i64, String)>>, job: &Job<'_>, results: &mut Results) {
        let workers = self.settings.parallel_chunks.min(chunks.len());
        let queue: ChunkQueue = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<Results>();
        let mut submitted = 0usize;
        for chunk in chunks {
            if job.is_cancelled() {
                break;
            }
            if let Ok(mut q) = queue.lock() {
                q.push_back(chunk);
            }
            submitted += 1;
        }
        let ids = crate::ids::snapshot();
        for _ in 0..workers {
            let (me, queue, stop, tx) = (
                self.clone(),
                Arc::clone(&queue),
                Arc::clone(&stop),
                tx.clone(),
            );
            let (voice, rid, rate, cancelled, on_item) = (
                job.voice.to_owned(),
                job.resource_id.to_owned(),
                job.rate,
                Arc::clone(job.cancelled),
                job.on_item.clone(),
            );
            std::thread::spawn(move || {
                crate::ids::restore(ids);
                let job = Job {
                    voice: &voice,
                    resource_id: &rid,
                    rate,
                    rate_text: crate::pyval::float_str(rate),
                    cancelled: &cancelled,
                    on_item: &on_item,
                };
                loop {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                    let Some(chunk) = next else { break };
                    if tx.send(me.process_chunk(&chunk, &job)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let mut done = 0;
        while done < submitted {
            if job.is_cancelled() {
                stop.store(true, Ordering::Relaxed);
                if let Ok(mut q) = queue.lock() {
                    q.clear();
                }
                // short drain for chunks that are just finishing
                if let Ok(r) = rx.recv_timeout(Duration::from_secs(1)) {
                    results.extend(r);
                }
                return;
            }
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(r) => {
                    results.extend(r);
                    done += 1;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn call(&self, device: &Object, req: &Request<'_>, job: &Job<'_>) -> PyResult<HttpResponse> {
        if job.is_cancelled() {
            return Err(CANCELLED.into());
        }
        let built = build_request_for(self.settings.device_profile, device, req)?;
        if job.is_cancelled() {
            return Err(CANCELLED.into());
        }
        let headers = built.string_headers();
        let resp = self
            .with_transport_retry(&|| job.is_cancelled(), || {
                self.transport.post(
                    &built.url,
                    &headers,
                    built.body.as_bytes(),
                    Duration::from_secs(60),
                )
            })
            .map_err(|e| format!("RequestException: {e}"))?;
        if job.is_cancelled() {
            return Err(CANCELLED.into());
        }
        Ok(resp)
    }

    /// Run one HTTP call, retrying connection, TLS and timeout errors (never HTTP statuses)
    /// up to [`TRANSPORT_RETRIES`] times, 1 s then 2 s apart. The original gives up at the
    /// first such error, which fails a whole 25-caption chunk on one dropped request (ADR-023).
    fn with_transport_retry<T>(
        &self,
        cancelled: &dyn Fn() -> bool,
        call: impl Fn() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut attempt = 0;
        loop {
            match call() {
                Err(e) if attempt < TRANSPORT_RETRIES && !cancelled() => {
                    attempt += 1;
                    crate::logger::log(
                        crate::logger::Level::Warning,
                        file!(),
                        line!(),
                        &format!("HTTP error, retry {attempt}/{TRANSPORT_RETRIES}: {e}"),
                    );
                    (self.sleep)(Duration::from_secs(u64::from(attempt)));
                }
                other => return other,
            }
        }
    }

    /// The chunk's device: `device.json` over the profile's defaults, with fresh ids.
    fn temp_device(&self) -> Object {
        let profile = self.settings.device_profile;
        let (rand_id, rand_iid) = match profile {
            DeviceProfile::Legacy => (numeric_id(19), numeric_id(19)),
            DeviceProfile::Upstream => {
                let id = numeric_id(20);
                (id.clone(), id)
            }
        };
        let mut data = match crate::draft_reader::read_json_file(&self.settings.device_json_path) {
            Ok(Value::Object(o)) if self.settings.device_json_path.exists() => o,
            _ => Object::new(),
        };
        data.insert("device_id", Value::from(rand_id.as_str()));
        data.insert("tdid", Value::from(rand_id.as_str()));
        data.insert("iid", Value::from(rand_iid));
        device_for(profile, Some(&data))
    }

    /// One `tts-new` task for `pending`, polled to completion, then downloads.
    /// Any failure before the downloads leaves the chunk's captions without a result.
    fn process_chunk(&self, pending: &[(i64, String)], job: &Job<'_>) -> Results {
        let mut results = Results::default();
        let _ = self.process_chunk_inner(pending, job, &mut results);
        results
    }

    fn process_chunk_inner(
        &self,
        pending: &[(i64, String)],
        job: &Job<'_>,
        results: &mut Results,
    ) -> PyResult<()> {
        let device = self.temp_device();
        let texts: Vec<String> = pending.iter().map(|(_, t)| t.clone()).collect();
        let new_req = Request::TtsNew {
            texts: &texts,
            voice: job.voice,
            resource_id: job.resource_id,
            rate: &job.rate_text,
        };
        let resp = match self.call(&device, &new_req, job) {
            Err(e) if e == CANCELLED => return Ok(()),
            other => other?,
        };
        if resp.status != 200 {
            return Err(format!(
                "RuntimeError: TTS submission failed: HTTP {}",
                resp.status
            ));
        }
        let body = resp.text();
        let json = crate::json::parse(&body)?;
        if !self.ret_ok(&json) {
            return Err(format!(
                "ValueError: API Error {}: {}",
                str_of(&json["ret"]),
                str_of(&json["errmsg"])
            ));
        }
        let task = &json["data"]["tasks"][0];
        let (task_id, token) = (
            task["id"].as_str().unwrap_or(""),
            task["token"].as_str().unwrap_or(""),
        );
        if task.is_null() || task_id.is_empty() || token.is_empty() {
            return Err("ValueError: Missing task id/token in response".into());
        }
        // Upstream polls with the bind id the server returned (task, then data).
        let bind_id = match self.settings.device_profile {
            DeviceProfile::Legacy => String::new(),
            DeviceProfile::Upstream => [&task["bind_id"], &json["data"]["bind_id"]]
                .into_iter()
                .find(|v| truthy(v))
                .map(str_of)
                .unwrap_or_default(),
        };

        let base = self.settings.poll_interval_sec.max(0.3);
        let max_polls = ((300.0 / base) as usize).max(1);
        let mut interval = base;
        // Legacy polls with the fixed default device, upstream with the chunk's own.
        let poll_device = match self.settings.device_profile {
            DeviceProfile::Legacy => device_for(DeviceProfile::Legacy, None),
            DeviceProfile::Upstream => device.clone(),
        };
        for _ in 1..=max_polls {
            if job.is_cancelled() {
                return Ok(());
            }
            (self.sleep)(Duration::from_secs_f64(interval));
            interval = (interval * 1.15).min(3.0);
            let subs = match self.query_task(&poll_device, task_id, token, &bind_id, job) {
                Err(e) if e == CANCELLED => return Ok(()),
                Err(e) => return Err(e),
                Ok(None) => continue,
                Ok(Some(s)) => s,
            };
            self.collect_audio(pending, &subs, task_id, job, results);
            return Ok(());
        }
        Ok(())
    }

    /// `None` while the task runs (or on a transient HTTP error).
    /// `ret` that means success: `"0"` (legacy); also `0`, `""` or missing (upstream).
    fn ret_ok(&self, json: &Value) -> bool {
        let ret = &json["ret"];
        match self.settings.device_profile {
            DeviceProfile::Legacy => str_of(ret) == "0",
            DeviceProfile::Upstream => {
                ret.is_null() || ret.as_i64() == Some(0) || matches!(ret.as_str(), Some("0" | ""))
            }
        }
    }

    fn succeeded(&self, status: &Value) -> bool {
        match self.settings.device_profile {
            DeviceProfile::Legacy => status.as_str() == Some("succeed"),
            DeviceProfile::Upstream => {
                let s = if truthy(status) {
                    str_of(status).to_lowercase()
                } else {
                    String::new()
                };
                matches!(
                    s.as_str(),
                    "success" | "succeed" | "succeeded" | "completed" | "done"
                )
            }
        }
    }

    fn query_task(
        &self,
        device: &Object,
        task_id: &str,
        token: &str,
        bind_id: &str,
        job: &Job<'_>,
    ) -> PyResult<Option<Vec<Value>>> {
        let resp = self.call(
            device,
            &Request::TtsQuery {
                task_id,
                token,
                bind_id,
            },
            job,
        )?;
        if resp.status != 200 {
            return Ok(None);
        }
        let json = crate::json::parse(&resp.text())?;
        if !self.ret_ok(&json) {
            return Err(format!(
                "ValueError: Poll API Error {}: {}",
                str_of(&json["ret"]),
                str_of(&json["errmsg"])
            ));
        }
        let task = &json["data"]["tasks"][0];
        if task.is_null() {
            return Ok(None);
        }
        match task["status"].as_str() {
            _ if self.succeeded(&task["status"]) => {}
            Some("failed" | "cancelled") => {
                return Err(format!(
                    "ValueError: CapCut TTS task {task_id} failed on server side"
                ));
            }
            _ => return Ok(None),
        }
        let payload = task["payload"]
            .as_str()
            .filter(|p| !p.is_empty())
            .ok_or("ValueError: Task status is succeed, but payload string is empty.")?;
        let subs = crate::json::parse(payload)?["audio_subtitles"]
            .as_array()
            .map(|a| a.to_vec())
            .unwrap_or_default();
        if subs.is_empty() {
            return Err("ValueError: Missing audio_subtitles in payload".into());
        }
        Ok(Some(subs))
    }

    fn collect_audio(
        &self,
        pending: &[(i64, String)],
        subs: &[Value],
        task_id: &str,
        job: &Job<'_>,
        results: &mut Results,
    ) {
        for (idx, _) in pending.iter().skip(subs.len()) {
            let r = TtsItemResult::failed("Server did not return audio for this subtitle");
            results.insert(*idx, r.clone());
            job.emit(*idx, &r);
        }
        // (index, text, url, segment task id, duration)
        let mut downloads: Vec<(i64, String, String, String, f64)> = Vec::new();
        for ((idx, text), info) in pending.iter().zip(subs) {
            let duration =
                to_float(info.get("duration").unwrap_or(&Value::from(0))).unwrap_or(0.0) / 1000.0;
            // Upstream also takes `audio_url` / `url` when `speech_url` is absent.
            let url = match self.settings.device_profile {
                DeviceProfile::Legacy => &info["speech_url"],
                DeviceProfile::Upstream => ["speech_url", "audio_url", "url"]
                    .iter()
                    .map(|k| &info[*k])
                    .find(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                    .unwrap_or(&info["speech_url"]),
            };
            if !truthy(url) {
                let r = TtsItemResult::failed("Missing speech_url in server response");
                results.insert(*idx, r.clone());
                job.emit(*idx, &r);
                continue;
            }
            downloads.push((
                *idx,
                text.clone(),
                str_of(url),
                format!("{task_id}_{idx}"),
                duration,
            ));
        }

        // All downloads run (as the original's pool drains them); results are read
        // in completion order and the reading stops on cancel.
        let (tx, rx) = mpsc::channel::<(usize, Result<TtsItemResult, String>)>();
        let queue = Arc::new(Mutex::new(
            (0..downloads.len()).collect::<std::collections::VecDeque<usize>>(),
        ));
        let downloads = Arc::new(downloads);
        let ids = crate::ids::snapshot();
        let handles: Vec<_> = (0..self.settings.download_workers.min(downloads.len()).max(1))
            .map(|_| {
                let (me, queue, downloads, tx) = (
                    self.clone(),
                    Arc::clone(&queue),
                    Arc::clone(&downloads),
                    tx.clone(),
                );
                let (voice, rid, rate) =
                    (job.voice.to_owned(), job.resource_id.to_owned(), job.rate);
                std::thread::spawn(move || {
                    crate::ids::restore(ids);
                    loop {
                        let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                        let Some(i) = next else { break };
                        let (idx, text, url, seg, dur) = &downloads[i];
                        let r =
                            me.download_and_cache(url, seg, text, &voice, rate, &rid, *dur, *idx);
                        if tx.send((i, r)).is_err() {
                            break;
                        }
                    }
                })
            })
            .collect();
        drop(tx);
        let mut cancelled = false;
        for (i, r) in rx.iter() {
            if job.is_cancelled() {
                cancelled = true;
                break;
            }
            let idx = downloads[i].0;
            let r = r.unwrap_or_else(|e| TtsItemResult::failed(&e));
            results.insert(idx, r.clone());
            job.emit(idx, &r);
        }
        if cancelled {
            for (idx, ..) in downloads.iter() {
                if !results.contains(*idx) {
                    let r = TtsItemResult::failed("Cancelled by user");
                    results.insert(*idx, r.clone());
                    job.emit(*idx, &r);
                }
            }
        }
        for h in handles {
            let _ = h.join();
        }
    }

    #[expect(clippy::too_many_arguments, reason = "mirrors the original")]
    fn download_and_cache(
        &self,
        url: &str,
        seg_task_id: &str,
        text: &str,
        voice: &str,
        rate: f64,
        rid: &str,
        duration: f64,
        idx: i64,
    ) -> Result<TtsItemResult, String> {
        // The original downloads into projects/tts/temp, fsyncs that temp file, copies it
        // into the cache (without fsync) and deletes it. Writing the bytes straight into the
        // cache gives the same cache file with one write instead of three and no fsync.
        let resp = self.with_transport_retry(&|| false, || {
            self.transport.get(url, Duration::from_secs(30))
        })?;
        if resp.status >= 400 {
            let kind = if resp.status < 500 {
                "Client"
            } else {
                "Server"
            };
            return Err(format!(
                "{} {kind} Error: {} for url: {url}",
                resp.status, resp.reason
            ));
        }
        if resp.body.is_empty() {
            return Err(format!("Empty download from {url}"));
        }
        let _ = seg_task_id;
        let cached = self
            .cache
            .store_bytes(&resp.body, text, voice, rate, rid, duration)
            .map_err(|e| exc_str(&e))?;
        let path = if self.project_name.is_empty() {
            cached
        } else {
            self.copy_to_project_tts(&cached, &format!("tts_{idx:06}.mp3"))
                .map_err(|e| exc_str(&e))?
        };
        Ok(TtsItemResult {
            audio_path: path.to_string_lossy().into_owned(),
            duration: Some(duration),
            status: "Downloaded",
            error: None,
        })
    }

    /// Load the device file once to check it (the original reads it per chunk).
    ///
    /// # Errors
    /// As [`load_device`].
    pub fn check_device(&self) -> PyResult<Object> {
        load_device(Some(&self.settings.device_json_path))
    }
}
