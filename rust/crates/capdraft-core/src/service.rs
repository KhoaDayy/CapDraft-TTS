//! The whole workflow: load a CapCut project, generate TTS, attach it,
//! validate, commit (port of `core/capcut_project/tts_project_service.py`).
//!
//! Log lines, progress steps and results match the original; the log file
//! written into the project is the same text.

use crate::alignment::NativeAudioAlignmentSettings;
use crate::config::AppConfig;
use crate::draft_patcher::{DraftPatcher, PatchSettings};
use crate::draft_reader::{DraftReader, ProjectInspectionResult};
use crate::ids::{local_now, uuid_hex};
use crate::json::Value;
use crate::models::{
    CapCutProjectInfo, CaptionRow, CaptionTtsResult, GenerationLogEvent, GenerationResult,
    ToneModifyMode, compute_target_duration_us, format_duration_us, seconds_to_us,
};
use crate::paths::ensure_draftpath_placeholder;
use crate::project_exporter::{PathStyle, ProjectExporter};
use crate::pyfmt::format_g;
use crate::pyval::PyResult;
use crate::tts::{
    CancelCallback, CapCutTts, ItemCallback, SleepFn, Transport, TtsItemResult, TtsSettings,
};
use crate::voice_catalog::VoiceCatalog;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type LogCallback = Arc<dyn Fn(&GenerationLogEvent) + Send + Sync>;
pub type ProgressCallback = Arc<dyn Fn(f64, &str) + Send + Sync>;

/// The run log: lines for the project's log file, events for the UI.
#[derive(Default)]
struct RunLog {
    lines: Mutex<Vec<String>>,
}

impl RunLog {
    fn clear(&self) {
        if let Ok(mut l) = self.lines.lock() {
            l.clear();
        }
    }

    fn snapshot(&self) -> Vec<String> {
        self.lines.lock().map(|l| l.clone()).unwrap_or_default()
    }

    fn emit(
        &self,
        cb: Option<&LogCallback>,
        level: &str,
        message: &str,
        stage: Option<&str>,
        caption_index: Option<i64>,
    ) {
        let level_u = if level.is_empty() {
            "INFO".to_owned()
        } else {
            level.to_uppercase()
        };
        let line = match stage {
            Some(s) => format!("[{s}] {message}"),
            None => message.to_owned(),
        };
        let now = local_now();
        if let Ok(mut l) = self.lines.lock() {
            l.push(format!("{} {level_u:<7} {line}", now.format("%H:%M:%S")));
            if l.len() > 5000 {
                let cut = l.len() - 4000;
                l.drain(..cut);
            }
        }
        let lvl = match level_u.as_str() {
            "ERROR" => crate::logger::Level::Error,
            "WARNING" => crate::logger::Level::Warning,
            "DEBUG" => crate::logger::Level::Debug,
            _ => crate::logger::Level::Info,
        };
        crate::logger::log(lvl, file!(), line!(), &line);
        if let Some(cb) = cb {
            cb(&GenerationLogEvent {
                timestamp: now,
                level: level_u,
                message: message.to_owned(),
                stage: stage.map(str::to_owned),
                caption_index,
                progress: None,
            });
        }
    }
}

/// What to generate (the arguments of `generate_and_attach`).
#[derive(Debug, Clone)]
pub struct GenerateRequest {
    /// Segment ids, 1-based caption indexes as strings, or text material ids.
    pub selected_caption_ids: Vec<String>,
    pub voice_type: String,
    pub resource_id: String,
    pub voice_display_name: String,
    pub tts_rate: f64,
    pub capcut_clip_speed: f64,
    pub tone_modify_mode: ToneModifyMode,
    /// `replace_existing`, `skip_existing` or `selected_only`.
    pub existing_tts_mode: String,
    pub allow_partial_attach: bool,
    pub use_cache: bool,
    pub alignment: NativeAudioAlignmentSettings,
}

pub struct ProjectTtsService {
    pub config: AppConfig,
    pub reader: DraftReader,
    pub catalog: VoiceCatalog,
    pub exporter: ProjectExporter,
    pub info: Option<CapCutProjectInfo>,
    log: Arc<RunLog>,
    log_callback: Option<LogCallback>,
    transport: Arc<dyn Transport>,
    tts_sleep: Option<SleepFn>,
}

/// `str(e)` of the original for this crate's `"Type: message"` errors.
fn exc_str(e: &str) -> String {
    match e.split_once(": ") {
        Some((ty, msg))
            if !ty.is_empty() && ty.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
        {
            msg.to_owned()
        }
        _ => e.to_owned(),
    }
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

impl ProjectTtsService {
    /// The catalog is not downloaded here (the original did, blocking); use
    /// [`Self::get_voice_catalog`] from a background thread.
    #[must_use]
    pub fn new(
        config: AppConfig,
        transport: Arc<dyn Transport>,
        log_callback: Option<LogCallback>,
    ) -> Self {
        let catalog = VoiceCatalog::new(Some(&config.voice_catalog_url()));
        let exporter = ProjectExporter::new(config.int("max_backups", 10).max(0) as usize);
        Self {
            config,
            reader: DraftReader::new(),
            catalog,
            exporter,
            info: None,
            log: Arc::new(RunLog::default()),
            log_callback,
            transport,
            tts_sleep: None,
        }
    }

    /// Replace the TTS waits (tests).
    #[must_use]
    pub fn with_tts_sleep(mut self, sleep: SleepFn) -> Self {
        self.tts_sleep = Some(sleep);
        self
    }

    pub fn set_log_callback(&mut self, cb: Option<LogCallback>) {
        self.log_callback = cb;
    }

    fn emit(&self, level: &str, message: &str, stage: Option<&str>) {
        self.log
            .emit(self.log_callback.as_ref(), level, message, stage, None);
    }

    /// The lines that go into the project's log file.
    #[must_use]
    pub fn log_lines(&self) -> Vec<String> {
        self.log.snapshot()
    }

    /// # Errors
    /// As [`DraftReader::load_draft`].
    pub fn load_project(&mut self, draft_path: &Path) -> PyResult<CapCutProjectInfo> {
        self.log.clear();
        self.emit("INFO", "Loading CapCut project...", Some("Project"));
        self.reader.load_draft(draft_path)?;
        let info = self.reader.get_project_info()?;
        self.info = Some(info.clone());
        let p = Some("Project");
        let name = info
            .draft_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.emit("INFO", &format!("Loaded {name}"), p);
        self.emit("INFO", &format!("ID: {}", info.project_id), p);
        let nv = info
            .new_version
            .clone()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "-".into());
        self.emit("INFO", &format!("Version: {} / {nv}", info.version), p);
        self.emit(
            "INFO",
            &format!(
                "Canvas: {}×{} @ {} FPS",
                info.width,
                info.height,
                format_g(info.fps)
            ),
            p,
        );
        self.emit(
            "INFO",
            &format!("Duration: {}", format_duration_us(info.duration_us)),
            p,
        );
        self.emit(
            "INFO",
            &format!(
                "Tracks: video={}, audio={}, text={}",
                info.video_track_count, info.audio_track_count, info.text_track_count
            ),
            p,
        );
        self.emit(
            "INFO",
            &format!(
                "Captions: total={}, existing_tts={}, empty={}",
                info.caption_count, info.caption_with_tts_count, info.empty_caption_count
            ),
            p,
        );
        Ok(info)
    }

    /// # Errors
    /// As [`DraftReader::get_captions`].
    pub fn get_captions(&mut self) -> PyResult<Vec<CaptionRow>> {
        self.reader.get_captions()
    }

    /// # Errors
    /// `RuntimeError` without a project; write errors.
    pub fn export_srt(&mut self, output_path: &Path) -> PyResult<usize> {
        if self.reader.draft_path.is_none() {
            return Err("RuntimeError: No project loaded. Call load_project() first.".into());
        }
        let caps = self.reader.get_captions()?;
        let n = crate::srt::export_captions_srt(&caps, output_path)
            .map_err(|e| crate::project_split::io_err(&e, output_path))?;
        self.emit(
            "SUCCESS",
            &format!("Exported {n} captions to {}", path_str(output_path)),
            Some("SRT"),
        );
        Ok(n)
    }

    /// The voice catalog, downloaded when empty, stale or asked to `reload`.
    ///
    /// # Errors
    /// `VoiceCatalogError` from the download.
    pub fn get_voice_catalog(&mut self, reload: bool) -> PyResult<&VoiceCatalog> {
        let url = self.config.voice_catalog_url();
        if reload || self.catalog.voices().is_empty() || self.catalog.url != url {
            self.catalog
                .load(Some(&url), self.transport.as_ref(), Duration::from_secs(20))?;
        }
        Ok(&self.catalog)
    }

    /// # Errors
    /// As [`DraftReader::inspect_project`].
    pub fn inspect_project(
        &mut self,
        selected: Option<&[String]>,
    ) -> PyResult<ProjectInspectionResult> {
        let r = self.reader.inspect_project(selected)?;
        for w in &r.warnings {
            self.emit("WARNING", w, Some("Project"));
        }
        self.emit(
            "INFO",
            &format!(
                "Inspection: valid={}, empty={}, existing_tts={}, orphan_segments={}",
                r.valid_caption_count,
                r.skipped_empty_count,
                r.existing_tts_count,
                r.orphan_text_segment_count
            ),
            Some("Project"),
        );
        Ok(r)
    }

    #[must_use]
    pub fn is_capcut_running(&self) -> bool {
        crate::capcut_process::is_capcut_running()
    }

    fn cancel_result(&self, project_dir: &Path, started: Instant) -> GenerationResult {
        self.emit("WARNING", "Generation cancelled", Some("cancelled"));
        GenerationResult {
            success: false,
            project_path: project_dir.to_path_buf(),
            processing_seconds: started.elapsed().as_secs_f64(),
            errors: vec!["Cancelled by user".into()],
            cancelled: true,
            ..Default::default()
        }
    }

    /// Generate TTS for the selected captions and attach it to the project.
    ///
    /// # Errors
    /// `RuntimeError` without a loaded project, and reader errors while
    /// reloading the draft. Everything later is reported in the result.
    #[expect(
        clippy::too_many_lines,
        reason = "one workflow, kept in the original's order"
    )]
    pub fn generate_and_attach(
        &mut self,
        req: &GenerateRequest,
        progress: Option<ProgressCallback>,
        item_completed: Option<ItemCallback>,
        is_cancelled: Option<CancelCallback>,
    ) -> PyResult<GenerationResult> {
        let started = Instant::now();
        let cb = self.log_callback.clone();
        let log = Arc::clone(&self.log);
        let emit = |level: &str, msg: &str, stage: &str| {
            log.emit(cb.as_ref(), level, msg, Some(stage), None)
        };
        let prog = |p: f64, msg: &str| {
            if let Some(f) = &progress {
                f(p.clamp(0.0, 1.0), msg);
            }
        };
        let cancel: CancelCallback = is_cancelled.clone().unwrap_or_else(|| Arc::new(|| false));
        let cancelled = || cancel();

        let Some(draft_path) = self.reader.draft_path.clone() else {
            return Err("RuntimeError: No project loaded. Call load_project() first.".into());
        };
        let project_dir = self.reader.project_directory.clone().unwrap_or_default();

        emit(
            "INFO",
            "Reloading project from disk before generate",
            "preparing",
        );
        prog(0.01, "reload");
        self.reader.load_draft(&draft_path)?;
        let info = self.reader.get_project_info()?;
        self.info = Some(info.clone());
        let mut captions = self.reader.get_captions()?;

        let selected_set: HashSet<&str> = req
            .selected_caption_ids
            .iter()
            .map(String::as_str)
            .collect();
        let selected_caps: Vec<CaptionRow> = captions
            .iter()
            .filter(|c| {
                selected_set.contains(c.text_segment_id.as_str())
                    || selected_set.contains(c.index.to_string().as_str())
                    || selected_set.contains(c.text_material_id.as_str())
            })
            .cloned()
            .collect();
        let ids: Vec<String> = selected_caps
            .iter()
            .map(|c| c.text_segment_id.clone())
            .collect();
        self.inspect_project(Some(&ids))?;
        prog(0.04, "inspect");

        let st = "Settings";
        emit("INFO", &format!("Voice: {}", req.voice_display_name), st);
        emit("INFO", &format!("Voice type: {}", req.voice_type), st);
        emit("INFO", &format!("Resource ID: {}", req.resource_id), st);
        emit(
            "INFO",
            &format!("TTS generation rate: {:.2}x", req.tts_rate),
            st,
        );
        emit(
            "INFO",
            &format!("CapCut clip speed: {:.2}x", req.capcut_clip_speed),
            st,
        );
        let flag = if req.tone_modify_mode.capcut_flag() {
            "True"
        } else {
            "False"
        };
        emit(
            "INFO",
            &format!(
                "Tone mode: {} (is_tone_modify={flag}, mapping_verified=True)",
                req.tone_modify_mode.value()
            ),
            st,
        );
        emit(
            "INFO",
            &format!("Existing TTS mode: {}", req.existing_tts_mode),
            st,
        );
        emit(
            "INFO",
            &format!(
                "Selected captions: {}/{}",
                selected_caps.len(),
                info.caption_count
            ),
            st,
        );
        emit(
            "INFO",
            &format!(
                "Cache: {}",
                if req.use_cache { "enabled" } else { "disabled" }
            ),
            st,
        );
        emit("INFO", &format!("Project: {}", path_str(&project_dir)), st);

        let mut to_generate: Vec<CaptionRow> = Vec::new();
        let mut items: Vec<CaptionTtsResult> = Vec::new();
        let mut skipped = 0i64;
        let skip_row = |cap: &CaptionRow, why: &str| CaptionTtsResult {
            caption_index: cap.index,
            text_material_id: cap.text_material_id.clone(),
            text_segment_id: cap.text_segment_id.clone(),
            start_us: cap.start_us,
            status: "skipped".into(),
            error: why.into(),
            ..Default::default()
        };
        for cap in &selected_caps {
            if cap.is_empty() {
                skipped += 1;
                items.push(skip_row(cap, "empty text"));
                log.emit(
                    cb.as_ref(),
                    "WARNING",
                    &format!("Caption #{} skipped · empty text", cap.index),
                    Some("TTS"),
                    Some(cap.index),
                );
                continue;
            }
            if req.existing_tts_mode == "skip_existing" && cap.has_existing_tts() {
                skipped += 1;
                items.push(skip_row(cap, "existing TTS"));
                continue;
            }
            to_generate.push(cap.clone());
        }
        if cancelled() {
            return Ok(self.cancel_result(&project_dir, started));
        }

        emit(
            "INFO",
            &format!("Starting TTS for {} captions", to_generate.len()),
            "Generate",
        );
        prog(0.05, "tts");

        let mut tts = CapCutTts::new(
            TtsSettings::from_config(&self.config),
            "",
            Arc::clone(&self.transport),
        )?;
        if let Some(s) = &self.tts_sleep {
            tts = tts.with_sleep(Arc::clone(s));
        }
        let tts_items: Vec<(i64, String)> = to_generate
            .iter()
            .map(|c| (c.index, c.text.clone()))
            .collect();
        let total = to_generate.len().max(1);
        let log_every = if total <= 40 {
            1
        } else if total <= 400 {
            25
        } else {
            50
        };
        let prog_every = if total <= 40 { 1 } else { (total / 40).max(5) };
        let tts_results: Arc<Mutex<HashMap<i64, TtsItemResult>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let last_bucket = Arc::new(Mutex::new(-1i64));
        let on_item: ItemCallback = {
            let (tts_results, log, cb, item_completed, progress) = (
                Arc::clone(&tts_results),
                Arc::clone(&log),
                cb.clone(),
                item_completed.clone(),
                progress.clone(),
            );
            let last_bucket = Arc::clone(&last_bucket);
            Arc::new(move |idx: i64, res: &TtsItemResult| {
                let done = {
                    let Ok(mut m) = tts_results.lock() else {
                        return;
                    };
                    m.insert(idx, res.clone());
                    m.len()
                };
                let dur = res.duration.unwrap_or(0.0);
                let log_this =
                    res.status == "Failed" || done == 1 || done == total || done % log_every == 0;
                if log_this {
                    let (level, msg) = match res.status {
                        "Cached" => (
                            "CACHED",
                            format!(
                                "Caption #{idx} loaded from cache · duration={dur:.2}s · {done}/{total}"
                            ),
                        ),
                        "Failed" => (
                            "ERROR",
                            format!(
                                "Caption #{idx} failed · {}",
                                res.error.as_deref().unwrap_or("unknown")
                            ),
                        ),
                        _ => (
                            "SUCCESS",
                            format!(
                                "Caption #{idx} generated · duration={dur:.2}s · {done}/{total}"
                            ),
                        ),
                    };
                    log.emit(cb.as_ref(), level, &msg, Some("TTS"), Some(idx));
                }
                if let Some(f) = &item_completed {
                    f(idx, res);
                }
                let bucket = (done / prog_every) as i64;
                if let Ok(mut last) = last_bucket.lock()
                    && (done == 1 || done == total || bucket != *last)
                {
                    *last = bucket;
                    if let Some(f) = &progress {
                        f(
                            (0.05 + 0.70 * (done as f64 / total as f64)).clamp(0.0, 1.0),
                            "tts",
                        );
                    }
                }
            })
        };
        if !to_generate.is_empty() {
            let batch = tts.generate_tts_batch(
                &tts_items,
                Some(&req.voice_type),
                Some(&req.resource_id),
                Some(req.tts_rate),
                &cancel,
                &Some(on_item),
                req.use_cache,
            );
            if let Ok(mut m) = tts_results.lock() {
                for (k, v) in batch.iter() {
                    m.insert(*k, v.clone());
                }
            }
        }
        if cancelled() {
            return Ok(self.cancel_result(&project_dir, started));
        }

        let tts_results = tts_results.lock().map(|m| m.clone()).unwrap_or_default();
        let (mut generated, mut cached, mut failed) = (0i64, 0i64, 0i64);
        for cap in &to_generate {
            let res = tts_results.get(&cap.index);
            let status_raw = res.map_or("Failed", |r| r.status);
            let audio_path = res.map(|r| r.audio_path.clone()).unwrap_or_default();
            let dur_s = res.and_then(|r| r.duration).unwrap_or(0.0);
            let source_us = seconds_to_us(dur_s);
            let target_us = compute_target_duration_us(source_us, req.capcut_clip_speed, info.fps);
            let (st, from_cache) = if status_raw == "Cached" && !audio_path.is_empty() {
                cached += 1;
                ("cached", true)
            } else if matches!(status_raw, "Downloaded" | "Generated" | "OK")
                || (!audio_path.is_empty() && Path::new(&audio_path).exists())
            {
                generated += 1;
                ("generated", false)
            } else {
                failed += 1;
                ("failed", false)
            };
            items.push(CaptionTtsResult {
                caption_index: cap.index,
                text_material_id: cap.text_material_id.clone(),
                text_segment_id: cap.text_segment_id.clone(),
                start_us: cap.start_us,
                audio_path,
                source_duration_us: source_us,
                target_duration_us: target_us,
                status: st.into(),
                error: res.and_then(|r| r.error.clone()).unwrap_or_default(),
                from_cache,
            });
        }

        let fail_result = |items: Vec<CaptionTtsResult>,
                           generated,
                           cached,
                           failed,
                           errors: Vec<String>,
                           warnings: Vec<String>,
                           backup: Option<PathBuf>,
                           validation: bool| GenerationResult {
            success: false,
            project_path: project_dir.clone(),
            backup_path: backup,
            selected: selected_caps.len() as i64,
            generated,
            cached,
            skipped,
            failed,
            processing_seconds: started.elapsed().as_secs_f64(),
            validation_passed: validation,
            warnings,
            errors,
            items,
            ..Default::default()
        };

        let success_count = items.iter().filter(|r| r.is_success()).count();
        if success_count == 0 {
            emit("ERROR", "No captions generated successfully", "Generate");
            return Ok(fail_result(
                items,
                generated,
                cached,
                failed,
                vec!["No captions generated successfully".into()],
                vec![],
                None,
                false,
            ));
        }
        emit(
            "INFO",
            &format!(
                "Pre-commit stats · generated={generated} · cached={cached} · failed={failed} · skipped={skipped} · ok={success_count}"
            ),
            "Generate",
        );
        if failed > 0 {
            let tail = if req.allow_partial_attach {
                ""
            } else {
                " · partial attach disabled"
            };
            emit(
                "WARNING",
                &format!("{failed} caption(s) failed before attach{tail}"),
                "Generate",
            );
        }
        if !req.allow_partial_attach && failed > 0 {
            emit(
                "ERROR",
                "Partial attach disabled and some captions failed — not committing",
                "Generate",
            );
            let err = format!(
                "Partial attach disabled: generated={generated}, cached={cached}, failed={failed}, skipped={skipped}"
            );
            return Ok(fail_result(
                items,
                generated,
                cached,
                failed,
                vec![err],
                vec![format!("{failed} caption(s) failed")],
                None,
                false,
            ));
        }

        prog(0.75, "copy_audio");
        emit(
            "INFO",
            "Copying audio into project textReading/",
            "copying_audio",
        );
        let mut draft = self.reader.get_draft_copy()?;
        let placeholder = ensure_draftpath_placeholder(&draft);
        emit(
            "INFO",
            &format!("Audio path placeholder: {placeholder}"),
            "copying_audio",
        );
        if req.alignment.enabled {
            let trim_ms = crate::pyfmt::round_half_even(
                req.alignment.leading_trim_frames / info.fps.max(1e-6) * 1000.0,
            );
            emit(
                "INFO",
                &format!(
                    "Native alignment on · FPS={} · trim={} frames ≈ {trim_ms}ms · fade={}ms",
                    format_g(info.fps),
                    format_g(req.alignment.leading_trim_frames),
                    format_g(req.alignment.fade_in_ms)
                ),
                "Alignment",
            );
        }

        let mut audio_rel: HashMap<i64, String> = HashMap::new();
        let mut new_audio_paths: Vec<PathBuf> = Vec::new();
        let mut jobs: Vec<(usize, PathBuf, String)> = Vec::new();
        for (i, r) in items.iter_mut().enumerate() {
            if !r.is_success() {
                continue;
            }
            let src = PathBuf::from(&r.audio_path);
            if !src.exists() {
                r.status = "failed".into();
                r.error = "audio file missing after generate".into();
                failed += 1;
                if r.from_cache {
                    cached = (cached - 1).max(0)
                } else {
                    generated = (generated - 1).max(0)
                }
                continue;
            }
            let short = &uuid_hex()[..8];
            let suffix = src.extension().map_or_else(
                || ".mp3".to_owned(),
                |e| format!(".{}", e.to_string_lossy()),
            );
            jobs.push((
                i,
                src,
                format!("tts_{:06}_{short}{suffix}", r.caption_index),
            ));
        }
        let workers = self.config.int("tts_download_workers", 8).max(1) as usize;
        let batch: Vec<(PathBuf, String)> = jobs
            .iter()
            .map(|(_, s, n)| (s.clone(), n.clone()))
            .collect();
        let copied = self.exporter.copy_audio_batch(
            &batch,
            &project_dir,
            Some(&draft),
            Some(&placeholder),
            PathStyle::Absolute,
            workers,
        );
        for ((i, _, dest_name), (rel, err)) in jobs.iter().zip(copied) {
            let r = &mut items[*i];
            if err.is_some() || rel.is_empty() {
                r.status = "failed".into();
                r.error = err.unwrap_or_else(|| "copy failed".into());
                failed += 1;
                if r.from_cache {
                    cached = (cached - 1).max(0)
                } else {
                    generated = (generated - 1).max(0)
                }
                log.emit(
                    cb.as_ref(),
                    "ERROR",
                    &format!("Copy failed caption #{}: {}", r.caption_index, r.error),
                    Some("copying_audio"),
                    Some(r.caption_index),
                );
                continue;
            }
            audio_rel.insert(r.caption_index, rel);
            let dest = project_dir.join("textReading").join(dest_name);
            new_audio_paths.push(dest.clone());
            r.audio_path = path_str(&dest);
        }
        let success_count = items
            .iter()
            .filter(|r| r.is_success() && audio_rel.contains_key(&r.caption_index))
            .count();
        if success_count == 0 {
            return Ok(fail_result(
                items,
                generated,
                cached,
                failed,
                vec!["No audio files copied".into()],
                vec![],
                None,
                false,
            ));
        }

        prog(0.78, "patch");
        emit("INFO", "Creating audio materials", "Patch");
        let templates = self.reader.get_tts_templates()?;
        let patcher = DraftPatcher::new(&templates, info.fps, req.alignment);
        let settings = PatchSettings {
            voice_type: &req.voice_type,
            resource_id: &req.resource_id,
            voice_display_name: &req.voice_display_name,
            capcut_clip_speed: req.capcut_clip_speed,
            tone_modify_mode: req.tone_modify_mode,
            existing_tts_mode: &req.existing_tts_mode,
        };
        let remove_new = |paths: &[PathBuf]| {
            for p in paths {
                let _ = std::fs::remove_file(p);
            }
        };
        let stats =
            match patcher.patch(&mut draft, &mut captions, &mut items, &settings, &audio_rel) {
                Ok(s) => s,
                Err(e) => {
                    let msg = exc_str(&e);
                    emit("ERROR", &format!("Patch failed: {msg}"), "patching");
                    remove_new(&new_audio_paths);
                    return Ok(fail_result(
                        items,
                        generated,
                        cached,
                        failed,
                        vec![format!("Patch failed: {msg}")],
                        vec![],
                        None,
                        false,
                    ));
                }
            };

        prog(0.88, "validate");
        emit("INFO", "Validating patched project", "Validate");
        let missing: Vec<String> = new_audio_paths
            .iter()
            .filter(|p| !std::fs::metadata(p).is_ok_and(|m| m.len() > 0))
            .map(|p| path_str(p))
            .collect();
        if !missing.is_empty() {
            for m in missing.iter().take(20) {
                emit("ERROR", &format!("New audio file missing: {m}"), "Validate");
            }
            let errs = missing
                .iter()
                .map(|m| format!("New audio file missing: {m}"))
                .collect();
            return Ok(fail_result(
                items,
                generated,
                cached,
                failed,
                errs,
                vec![],
                None,
                false,
            ));
        }
        let errors = crate::validator::validate(&draft, Some(&project_dir), false)?;
        if !errors.is_empty() {
            for e in errors.iter().take(20) {
                emit("ERROR", e, "Validate");
            }
            remove_new(&new_audio_paths);
            return Ok(fail_result(
                items,
                generated,
                cached,
                failed,
                errors,
                vec![],
                None,
                false,
            ));
        }
        emit("INFO", "Project integrity passed", "Validate");
        prog(0.95, "backup");

        let mut manifest = crate::jv!({
            "project_info": {
                "project_id": info.project_id,
                "resolution": format!("{}x{}", info.width, info.height),
                "fps": info.fps,
                "duration_us": info.duration_us,
                "caption_count": info.caption_count,
            },
            "settings": {
                "voice_type": req.voice_type,
                "resource_id": req.resource_id,
                "voice_display_name": req.voice_display_name,
                "tts_rate": req.tts_rate,
                "capcut_clip_speed": req.capcut_clip_speed,
                "tone_modify_mode": req.tone_modify_mode.value(),
                "existing_tts_mode": req.existing_tts_mode,
            },
            "summary": {
                "selected": selected_caps.len(),
                "generated": generated,
                "cached": cached,
                "skipped": skipped,
                "failed": failed,
                "attached": success_count,
                "audio_tracks": stats.tracks_used,
                "tracks_created": stats.tracks_created,
                "replaced": stats.replaced,
                "alignment_applied": stats.alignment_applied,
                "fades_created": stats.fades_created,
            },
            "alignment": {
                "enabled": req.alignment.enabled,
                "leading_trim_frames": req.alignment.leading_trim_frames,
                "fade_in_ms": req.alignment.fade_in_ms,
            },
            "audio_path_placeholder": placeholder,
            "created_ids_count": stats.created_ids.len(),
            "timestamp": local_now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        });

        let mut backup_path: Option<PathBuf> = None;
        let commit = match self.exporter.create_backup(
            &draft_path,
            &project_dir,
            &manifest,
            &info.project_id,
        ) {
            Ok(b) => {
                backup_path = Some(b);
                self.exporter.atomic_write_draft(
                    &draft_path,
                    &draft,
                    Some(&project_dir),
                    &info.project_id,
                )
            }
            Err(e) => Err(e),
        };
        let written = match commit {
            Ok(w) => w,
            Err(e) => {
                let msg = exc_str(&e);
                emit("ERROR", &format!("Commit failed: {msg}"), "Commit");
                self.exporter.rollback(
                    &draft_path,
                    backup_path.as_deref(),
                    &new_audio_paths,
                    Some(&project_dir),
                    &info.project_id,
                );
                let errs = vec![format!("Commit failed and rolled back: {msg}")];
                return Ok(fail_result(
                    items,
                    generated,
                    cached,
                    failed,
                    errs,
                    vec![],
                    backup_path,
                    true,
                ));
            }
        };
        for w in &written {
            emit("INFO", &format!("Wrote {}", path_str(w)), "Commit");
        }
        if let Err(e) = self.reader.load_draft(&draft_path) {
            emit(
                "WARNING",
                &format!("Post-commit reload warning: {}", exc_str(&e)),
                "Commit",
            );
        }

        let log_file = self
            .exporter
            .write_log_file(&project_dir, &self.log.snapshot())?;
        let rel = log_file
            .strip_prefix(&project_dir)
            .map_or_else(|_| path_str(&log_file), path_str);
        if let Some(m) = manifest.as_object_mut() {
            m.insert("log_file", Value::from(rel));
        }
        let manifest_path = self.exporter.write_manifest(&project_dir, &manifest)?;

        let elapsed = started.elapsed().as_secs_f64();
        let mut warnings = Vec::new();
        if failed > 0 {
            warnings.push(format!(
                "{failed} caption(s) failed · attached {success_count}/{} (generated={generated}, cached={cached}, skipped={skipped})",
                selected_caps.len()
            ));
        }
        let result = GenerationResult {
            success: true,
            project_path: project_dir.clone(),
            backup_path: backup_path.clone(),
            selected: selected_caps.len() as i64,
            generated,
            cached,
            skipped,
            failed,
            replaced: stats.replaced,
            attached: success_count as i64,
            audio_tracks_used: stats.tracks_used,
            audio_tracks_created: stats.tracks_created,
            processing_seconds: elapsed,
            validation_passed: true,
            log_file: Some(log_file),
            manifest_path: Some(manifest_path),
            warnings,
            errors: vec![],
            items,
            cancelled: false,
        };

        let secs = elapsed as i64;
        let c = "Complete";
        if failed == 0 {
            emit("SUCCESS", "TTS attached successfully", c);
        } else {
            emit("WARNING", "Completed with warnings", c);
        }
        emit("INFO", &format!("Selected: {}", result.selected), c);
        emit("INFO", &format!("Generated: {}", result.generated), c);
        emit("INFO", &format!("Cache hits: {}", result.cached), c);
        emit("INFO", &format!("Skipped: {}", result.skipped), c);
        emit("INFO", &format!("Failed: {}", result.failed), c);
        emit("INFO", &format!("Attached: {}", result.attached), c);
        emit(
            "INFO",
            &format!("Audio tracks: {}", result.audio_tracks_used),
            c,
        );
        emit(
            "INFO",
            &format!(
                "Native trim applied: {} · fades: {} · short-trim reduced: {}",
                stats.alignment_applied, stats.fades_created, stats.alignment_trim_reduced
            ),
            "Alignment",
        );
        emit(
            "INFO",
            &format!("Processing time: {:02}:{:02}", secs / 60, secs % 60),
            c,
        );
        emit(
            "INFO",
            &format!(
                "Backup: {}",
                backup_path
                    .as_deref()
                    .map_or_else(|| "None".into(), path_str)
            ),
            c,
        );
        emit("INFO", &format!("Project: {}", path_str(&project_dir)), c);
        if !(project_dir.join("draft_meta_info.json").exists()
            || project_dir.join("Timelines").is_dir())
        {
            emit(
                "WARNING",
                "Project folder does not look like a CapCut draft root (missing draft_meta_info.json / Timelines). Open the CapCut project folder under AppData\\Local\\CapCut\\User Data\\Projects\\com.lveditor.draft\\… and generate again, otherwise CapCut will not show the audio.",
                c,
            );
        }
        prog(1.0, "done");
        Ok(result)
    }
}
