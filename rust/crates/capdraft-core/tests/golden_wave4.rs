//! Replays wave-4 cases recorded by `rust/tools/record_wave4.py`: CapCut TTS
//! requests byte for byte, and the TTS batch against `FakeCapCut` (the same fake
//! server the recorder used, rewritten here).

#![expect(
    clippy::unwrap_used,
    reason = "test harness: a malformed golden file should fail loudly"
)]

use capdraft_core::capcut_client::{
    Request, TTS_SIGN_PUBLIC_KEY_PEM, build_request, device_with, escape_xml, make_sign_header,
    md5_hex, rsa_encrypt_pkcs1v15, rsa_public_numbers_from_pem,
};
use capdraft_core::ids;
use capdraft_core::json::{self as cj, Object, Value, dumps_compact};
use capdraft_core::jv as json;
use capdraft_core::tts::{CapCutTts, HttpResponse, Transport, TtsItemResult, TtsSettings};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const CLOCK: f64 = 1_700_000_000.25;

fn golden(name: &str) -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden")
        .join(format!("{name}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines().map(|l| cj::parse(l).unwrap()).collect()
}

fn outcome(r: Result<Value, String>) -> Value {
    let mut o = Object::new();
    match r {
        Ok(v) => o.insert("ok", v),
        Err(e) => o.insert("err", Value::from(e)),
    }
    Value::Object(o)
}

fn recorded(case: &Value) -> Value {
    let mut o = Object::new();
    match (case.get("ok"), case.get("err")) {
        (Some(v), _) => o.insert("ok", v.clone()),
        (None, Some(e)) => o.insert("err", e.clone()),
        _ => panic!("bad case"),
    }
    Value::Object(o)
}

fn det<T>(f: impl FnOnce() -> T) -> T {
    ids::deterministic(CLOCK);
    let out = f();
    ids::reset();
    out
}

struct Report {
    name: &'static str,
    total: usize,
    failures: Vec<String>,
}

impl Report {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            total: 0,
            failures: vec![],
        }
    }
    fn check(&mut self, label: &str, want: &Value, got: &Value) {
        self.total += 1;
        let (w, g) = (dumps_compact(want), dumps_compact(got));
        if w != g {
            let at = w
                .bytes()
                .zip(g.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(w.len().min(g.len()));
            let cut = |s: &str| {
                let (mut a, mut b) = (at.saturating_sub(150), (at + 150).min(s.len()));
                while !s.is_char_boundary(a) {
                    a -= 1;
                }
                while !s.is_char_boundary(b) {
                    b -= 1;
                }
                s[a..b].to_owned()
            };
            self.failures
                .push(format!("{label}\n  want …{}\n   got …{}", cut(&w), cut(&g)));
        }
    }
    fn finish(self) {
        let shown: Vec<_> = self.failures.iter().take(4).cloned().collect();
        assert!(
            self.failures.is_empty(),
            "{}: {}/{} match\n{}",
            self.name,
            self.total - self.failures.len(),
            self.total,
            shown.join("\n")
        );
        eprintln!("{}: {1}/{1} match", self.name, self.total);
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap().to_owned()
}

// ------------------------------------------------------------------ client

fn built_json(b: &capdraft_core::capcut_client::BuiltRequest) -> Value {
    let headers: Vec<Value> = b
        .headers
        .iter()
        .map(|(k, v)| Value::from(vec![Value::from(k.as_str()), v.clone()]))
        .collect();
    json!({"url": b.url, "headers": headers, "body": b.body})
}

#[test]
fn client_matches_python() {
    let mut r = Report::new("capcut_client");
    for case in golden("capcut_client") {
        let a = case["args"].as_array().unwrap();
        let got = det(|| match case["fn"].as_str().unwrap() {
            "escape_xml" => Ok(Value::from(escape_xml(&s(&a[0])))),
            "rsa_public_numbers" => {
                let (n, e) = rsa_public_numbers_from_pem(TTS_SIGN_PUBLIC_KEY_PEM)?;
                Ok(json!([
                    n.to_str_radix(16),
                    e.to_string().parse::<i64>().unwrap()
                ]))
            }
            "rsa_encrypt_pkcs1v15" => rsa_encrypt_pkcs1v15(s(&a[0]).as_bytes()).map(Value::from),
            "make_sign_header" => Ok(Value::from(make_sign_header(
                &s(&a[0]),
                &s(&a[1]),
                &s(&a[2]),
                &s(&a[3]),
            ))),
            "build_request" => {
                let device = device_with(a[0].as_object());
                match a[1].as_str().unwrap() {
                    "tts-new" => {
                        let texts: Vec<String> = a[2].as_array().unwrap().iter().map(s).collect();
                        let (voice, rid, rate) = (s(&a[3]), s(&a[4]), s(&a[5]));
                        build_request(
                            &device,
                            &Request::TtsNew {
                                texts: &texts,
                                voice: &voice,
                                resource_id: &rid,
                                rate: &rate,
                            },
                        )
                        .map(|b| built_json(&b))
                    }
                    _ => {
                        let (tid, tok, bind) = (s(&a[2]), s(&a[3]), s(&a[4]));
                        build_request(
                            &device,
                            &Request::TtsQuery {
                                task_id: &tid,
                                token: &tok,
                                bind_id: &bind,
                            },
                        )
                        .map(|b| built_json(&b))
                    }
                }
            }
            other => panic!("{other}"),
        });
        r.check(
            &format!("{} {:?}", case["fn"], a.first().map(dumps_compact)),
            &recorded(&case),
            &outcome(got),
        );
    }
    r.finish();
}

// ------------------------------------------------------------------ fake CapCut

#[derive(Default)]
struct FakeState {
    tasks: HashMap<String, (Vec<String>, u32)>,
    log: Vec<Value>,
}

#[derive(Default)]
struct FakeCapCut(Mutex<FakeState>);

fn resp(status: u16, body: &str) -> HttpResponse {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "",
    };
    HttpResponse {
        status,
        reason: reason.into(),
        body: body.as_bytes().to_vec(),
    }
}

impl Transport for FakeCapCut {
    fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
        _t: Duration,
    ) -> Result<HttpResponse, String> {
        let mut st = self.0.lock().unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        let hs: Vec<Value> = headers.iter().map(|(k, v)| json!([k, v])).collect();
        st.log
            .push(json!({"method": "POST", "url": url, "headers": hs, "body": text.as_str()}));
        let req = cj::parse(&text).unwrap();
        if url.contains("/lv/v1/common_task/new") {
            let payload = cj::parse(req["tasks"][0]["payload"].as_str().unwrap()).unwrap();
            let ssml = s(&payload["ssml"]);
            let re = regex::Regex::new(r#"(?s)<prosody rate="[^"]*">(.*?)</prosody>"#).unwrap();
            let texts: Vec<String> = re.captures_iter(&ssml).map(|c| c[1].to_owned()).collect();
            let joined = texts.concat();
            if joined.contains("#HTTP500") {
                return Ok(resp(500, "oops"));
            }
            if joined.contains("#RET1") {
                return Ok(resp(200, r#"{"ret":"1","errmsg":"bad request"}"#));
            }
            if joined.contains("#NOTASK") {
                return Ok(resp(200, r#"{"ret":"0","data":{"tasks":[]}}"#));
            }
            let tid = format!("T{}", &md5_hex(ssml.as_bytes())[..10]);
            st.tasks.insert(tid.clone(), (texts, 0));
            return Ok(resp(
                200,
                &dumps_compact(
                    &json!({"ret": "0", "data": {"tasks": [{"id": tid, "token": format!("K{tid}")}]}}),
                ),
            ));
        }
        let tid = s(&req["tasks"][0]["id"]);
        let t = st.tasks.get_mut(&tid).unwrap();
        t.1 += 1;
        let (texts, polls) = (t.0.clone(), t.1);
        let joined = texts.concat();
        if joined.contains("#QUERY500") && polls == 1 {
            return Ok(resp(500, "busy"));
        }
        if joined.contains("#QRET1") && polls == 3 {
            return Ok(resp(200, r#"{"ret":"5","errmsg":"poll broke"}"#));
        }
        if polls < 3 {
            return Ok(resp(
                200,
                &dumps_compact(
                    &json!({"ret": "0", "data": {"tasks": [{"id": tid, "status": "running"}]}}),
                ),
            ));
        }
        if joined.contains("#FAILTASK") {
            return Ok(resp(
                200,
                &dumps_compact(
                    &json!({"ret": "0", "data": {"tasks": [{"id": tid, "status": "failed"}]}}),
                ),
            ));
        }
        let mut subs: Vec<Value> = texts
            .iter()
            .enumerate()
            .map(|(i, tx)| {
                if tx.contains("#NOURL") {
                    json!({"duration": 1000})
                } else {
                    json!({"speech_url": format!("https://cdn.fake/{tid}/{i}.mp3"), "duration": 1000 + 37 * tx.chars().count() as i64})
                }
            })
            .collect();
        if joined.contains("#SHORT") {
            subs.pop();
        }
        let payload = dumps_compact(&json!({"audio_subtitles": subs}));
        Ok(resp(
            200,
            &dumps_compact(
                &json!({"ret": "0", "data": {"tasks": [{"id": tid, "status": "succeed", "payload": payload}]}}),
            ),
        ))
    }

    fn get(&self, url: &str, _t: Duration) -> Result<HttpResponse, String> {
        let mut st = self.0.lock().unwrap();
        st.log.push(json!({"method": "GET", "url": url}));
        let re = regex::Regex::new(r"https://cdn\.fake/(\w+)/(\d+)\.mp3").unwrap();
        let c = re.captures(url).unwrap();
        let (tid, i) = (c[1].to_owned(), c[2].parse::<usize>().unwrap());
        let text = st.tasks[&tid].0[i].clone();
        if text.contains("#DLFAIL") {
            return Ok(resp(404, ""));
        }
        if text.contains("#DL500") {
            return Ok(resp(500, ""));
        }
        if text.contains("#EMPTY") {
            return Ok(resp(200, ""));
        }
        Ok(resp(200, &format!("MP3:{tid}:{i}:{text}")))
    }
}

// ------------------------------------------------------------------ batch

fn result_json(r: &TtsItemResult) -> Value {
    let mut o = Object::new();
    o.insert("audio_path", Value::from(r.audio_path.as_str()));
    o.insert("duration", Value::from(r.duration));
    o.insert("status", Value::from(r.status));
    if let Some(e) = &r.error {
        o.insert("error", Value::from(e.as_str()));
    }
    Value::Object(o)
}

fn snapshot(base: &Path) -> Value {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>, root: &str) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out, root);
            } else {
                let data = std::fs::read(&p).unwrap();
                let text = String::from_utf8(data.clone()).unwrap_or_else(|_| {
                    format!(
                        "hex:{}",
                        data.iter().map(|b| format!("{b:02x}")).collect::<String>()
                    )
                });
                out.insert(
                    p.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    text.replace(root, "<ROOT>"),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(base, base, &mut out, &base.to_string_lossy());
    Value::Object(
        out.into_iter()
            .map(|(k, v)| (k, Value::from(v)))
            .collect::<Object>(),
    )
}

#[test]
fn tts_batch_matches_python() {
    let mut r = Report::new("tts_batch");
    let device_json = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../external/capcut-tts-api/device.json");
    for (n, case) in golden("tts_batch").into_iter().enumerate() {
        let a = case["args"].as_array().unwrap();
        let texts: Vec<String> = a[0].as_array().unwrap().iter().map(s).collect();
        let (chunk, par, workers) = (
            a[1].as_i64().unwrap() as usize,
            a[2].as_i64().unwrap() as usize,
            a[3].as_i64().unwrap() as usize,
        );
        let use_cache = a[4].as_bool().unwrap();
        let cancel_after = a[5].as_i64();
        let pre: Vec<String> = a[6].as_array().unwrap().iter().map(s).collect();
        let project = s(&a[7]);

        let raw = std::env::temp_dir().join(format!("capdraft-w4-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&raw);
        std::fs::create_dir_all(&raw).unwrap();
        let base = std::fs::canonicalize(&raw).unwrap();
        let settings = TtsSettings {
            chunk_size: chunk,
            parallel_chunks: par,
            poll_interval_sec: 1.0,
            download_workers: workers,
            default_voice: "BV074_streaming".into(),
            default_resource_id: "7102355709945188865".into(),
            default_rate: 1.0,
            cache_dir: base.join("cache"),
            projects_dir: base.join("projects"),
            device_json_path: device_json.clone(),
            ffprobe_path: "ffprobe".into(),
            device_profile: capdraft_core::capcut_client::DeviceProfile::Legacy,
        };
        let fake = Arc::new(FakeCapCut::default());
        let sleeps: Arc<Mutex<Vec<f64>>> = Arc::new(Mutex::new(Vec::new()));
        let sl = sleeps.clone();
        let tts = CapCutTts::new(settings, &project, fake.clone())
            .unwrap()
            .with_sleep(Arc::new(move |d: Duration| {
                sl.lock()
                    .unwrap()
                    .push((d.as_secs_f64() * 1e9).round() / 1e9)
            }));
        for t in &pre {
            let src = base.join("pre.mp3");
            std::fs::write(&src, format!("CACHED:{t}")).unwrap();
            tts.cache
                .cache_audio(&src, t, "BV074_streaming", 1.0, "7102355709945188865", 2.5)
                .unwrap();
            std::fs::remove_file(&src).unwrap();
        }
        let checks = Arc::new(AtomicUsize::new(0));
        let events: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let (c2, e2) = (checks.clone(), events.clone());
        let cancelled: capdraft_core::tts::CancelCallback = Arc::new(move || {
            let k = c2.fetch_add(1, Ordering::SeqCst) + 1;
            cancel_after.is_some_and(|l| k as i64 > l)
        });
        let on_item: Option<capdraft_core::tts::ItemCallback> =
            Some(Arc::new(move |i, res: &TtsItemResult| {
                e2.lock().unwrap().push(json!([i, result_json(res)]))
            }));
        let items: Vec<(i64, String)> = texts
            .iter()
            .enumerate()
            .map(|(i, t)| (i as i64 + 1, t.clone()))
            .collect();
        let res = det(|| {
            tts.generate_tts_batch(
                &items,
                Some("BV074_streaming"),
                Some("7102355709945188865"),
                Some(1.0),
                &cancelled,
                &on_item,
                use_cache,
            )
        });

        let nonblank = texts.iter().filter(|t| !t.trim().is_empty()).count();
        let sequential = par == 1 || nonblank.div_ceil(chunk) <= 1;
        let mut results: Vec<Value> = res
            .iter()
            .map(|(k, v)| json!([k, result_json(v)]))
            .collect();
        let mut evs = events.lock().unwrap().clone();
        if !sequential {
            results.sort_by_key(|v| v[0].as_i64());
            evs.sort_by_key(|v| v[0].as_i64());
        }
        let log = fake.0.lock().unwrap().log.clone();
        let got = json!({
            "results": results,
            "events": evs,
            "cancel_checks": if sequential { Value::from(checks.load(Ordering::SeqCst)) } else { Value::Null },
            "requests": if sequential { Value::from(log) } else { Value::Null },
            "sleeps": if sequential { Value::from(sleeps.lock().unwrap().iter().map(|x| Value::from(*x)).collect::<Vec<_>>()) } else { Value::Null },
        });
        let root = base.to_string_lossy().into_owned();
        let got = cj::parse(&dumps_compact(&got).replace(&root, "<ROOT>")).unwrap();
        r.check(&format!("tts_batch #{n} {texts:?}"), &case["ok"], &got);
        r.check(
            &format!("tts_batch #{n} tree"),
            &case["tree"],
            &snapshot(&base),
        );
        let _ = std::fs::remove_dir_all(&base);
    }
    r.finish();
}

/// The upstream profile (ADR-018): `MacBookPro17,4`, one 20-digit id for
/// `device_id`/`iid`/`tdid` per chunk, and the poll uses that same device.
#[test]
fn upstream_device_profile() {
    let raw = std::env::temp_dir().join(format!("capdraft-w4-up-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&raw);
    std::fs::create_dir_all(&raw).unwrap();
    let base = std::fs::canonicalize(&raw).unwrap();
    let device_json = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../external/capcut-tts-api/device.json");
    let settings = TtsSettings {
        chunk_size: 2,
        parallel_chunks: 1,
        poll_interval_sec: 1.0,
        download_workers: 1,
        default_voice: "BV074_streaming".into(),
        default_resource_id: "7102355709945188865".into(),
        default_rate: 1.0,
        cache_dir: base.join("cache"),
        projects_dir: base.join("projects"),
        device_json_path: device_json,
        ffprobe_path: "ffprobe".into(),
        device_profile: capdraft_core::capcut_client::DeviceProfile::Upstream,
    };
    let fake = Arc::new(FakeCapCut::default());
    let tts = CapCutTts::new(settings, "", fake.clone())
        .unwrap()
        .with_sleep(Arc::new(|_| {}));
    let never: capdraft_core::tts::CancelCallback = Arc::new(|| false);
    let items: Vec<(i64, String)> = (1..=4).map(|i| (i, format!("câu {i}"))).collect();
    let res = tts.generate_tts_batch(&items, None, None, None, &never, &None, true);
    assert_eq!(
        res.iter().filter(|(_, r)| r.status == "Downloaded").count(),
        4
    );

    let log = fake.0.lock().unwrap().log.clone();
    let mut chunk_ids = Vec::new();
    let mut current = String::new();
    for req in log.iter().filter(|r| r["method"].as_str() == Some("POST")) {
        let url = s(&req["url"]);
        let q: HashMap<String, String> = url
            .split_once('?')
            .unwrap()
            .1
            .split('&')
            .map(|kv| {
                let (k, v) = kv.split_once('=').unwrap();
                (k.to_owned(), v.to_owned())
            })
            .collect();
        assert_eq!(q["device_type"], "MacBookPro17%2C4");
        assert_eq!(q["device_brand"], "MacBookPro17%2C4");
        assert_eq!(q["device_id"].len(), 20);
        assert_eq!(q["device_id"], q["iid"]);
        let tdid = req["headers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h[0].as_str() == Some("tdid"))
            .unwrap()[1]
            .clone();
        assert_eq!(s(&tdid), q["device_id"]);
        if url.contains("/common_task/new") {
            current = q["device_id"].clone();
            chunk_ids.push(current.clone());
        } else {
            assert_eq!(q["device_id"], current, "polls use the chunk's device");
        }
    }
    assert_eq!(chunk_ids.len(), 2);
    assert_ne!(chunk_ids[0], chunk_ids[1], "a fresh device per chunk");
    let _ = std::fs::remove_dir_all(&base);
}

/// A server that answers the upstream way: numeric `ret`, no `ret` on polls,
/// a `bind_id`, status `Success`, `audio_url` instead of `speech_url`.
#[derive(Default)]
struct UpstreamStyle(Mutex<(u32, Vec<String>)>);

impl Transport for UpstreamStyle {
    fn post(
        &self,
        url: &str,
        _h: &[(String, String)],
        body: &[u8],
        _t: Duration,
    ) -> Result<HttpResponse, String> {
        let mut st = self.0.lock().unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        st.1.push(text.clone());
        if url.contains("/common_task/new") {
            return Ok(resp(
                200,
                r#"{"ret":0,"data":{"bind_id":"BIND-9","tasks":[{"id":"T1","token":"K1"}]}}"#,
            ));
        }
        st.0 += 1;
        if st.0 < 2 {
            return Ok(resp(
                200,
                r#"{"data":{"tasks":[{"id":"T1","status":"running"}]}}"#,
            ));
        }
        let payload = dumps_compact(
            &json!({"audio_subtitles": [{"audio_url": "https://cdn.fake/a.mp3", "duration": 1500}]}),
        );
        Ok(resp(
            200,
            &dumps_compact(
                &json!({"data": {"tasks": [{"id": "T1", "status": "Success", "payload": payload}]}}),
            ),
        ))
    }
    fn get(&self, _url: &str, _t: Duration) -> Result<HttpResponse, String> {
        Ok(resp(200, "MP3"))
    }
}

#[test]
fn upstream_protocol_changes() {
    let raw = std::env::temp_dir().join(format!("capdraft-w4-proto-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&raw);
    std::fs::create_dir_all(&raw).unwrap();
    let base = std::fs::canonicalize(&raw).unwrap();
    let mk = |profile| TtsSettings {
        chunk_size: 5,
        parallel_chunks: 1,
        poll_interval_sec: 1.0,
        download_workers: 1,
        default_voice: "v<1>".into(),
        default_resource_id: "r&1".into(),
        default_rate: 1.0,
        cache_dir: base.join("cache"),
        projects_dir: base.join("projects"),
        device_json_path: base.join("none.json"),
        ffprobe_path: "ffprobe".into(),
        device_profile: profile,
    };
    let never: capdraft_core::tts::CancelCallback = Arc::new(|| false);
    let items = vec![(1i64, "xin chào".to_owned())];

    let up = Arc::new(UpstreamStyle::default());
    let tts = CapCutTts::new(
        mk(capdraft_core::capcut_client::DeviceProfile::Upstream),
        "",
        up.clone(),
    )
    .unwrap()
    .with_sleep(Arc::new(|_| {}));
    let res = tts.generate_tts_batch(&items, None, None, None, &never, &None, false);
    assert_eq!(res.get(1).unwrap().status, "Downloaded");
    assert_eq!(res.get(1).unwrap().duration, Some(1.5));
    let bodies = up.0.lock().unwrap().1.clone();
    let ssml = cj::parse(
        cj::parse(&bodies[0]).unwrap()["tasks"][0]["payload"]
            .as_str()
            .unwrap(),
    )
    .unwrap()["ssml"]
        .clone();
    assert!(
        s(&ssml).contains(r#"name="v&lt;1&gt;""#) && s(&ssml).contains(r#"resource_id="r&amp;1""#)
    );
    assert_eq!(
        cj::parse(&bodies[1]).unwrap()["tasks"][0]["bind_id"].as_str(),
        Some("BIND-9")
    );

    // The legacy profile keeps the Python app's rules: a numeric `ret` is an error there.
    let _ = std::fs::remove_dir_all(base.join("cache"));
    let tts = CapCutTts::new(
        mk(capdraft_core::capcut_client::DeviceProfile::Legacy),
        "",
        Arc::new(UpstreamStyle::default()),
    )
    .unwrap()
    .with_sleep(Arc::new(|_| {}));
    assert!(
        tts.generate_tts_batch(&items, None, None, None, &never, &None, false)
            .is_empty()
    );
    let _ = std::fs::remove_dir_all(&base);
}
