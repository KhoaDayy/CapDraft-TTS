//! Window wiring: state, callbacks, background jobs.

use crate::{AppWindow, CaptionItem, LogItem, SettingsWindow, Tr};
use capdraft_core::alignment::NativeAudioAlignmentSettings;
use capdraft_core::config::AppConfig;
use capdraft_core::i18n::{self, tr};
use capdraft_core::json::Value;
use capdraft_core::models::{
    CapCutProjectInfo, CaptionRow, GenerationLogEvent, GenerationResult, ToneModifyMode,
    VoiceOption,
};
use capdraft_core::pyfmt::format_g;
use capdraft_core::service::{GenerateRequest, ProjectTtsService};
use capdraft_core::tts::{TtsItemResult, UreqTransport};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel, Weak};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

const MAX_LOG_LINES: usize = 5000;

/// One caption row as the table shows it.
#[derive(Clone)]
struct RowState {
    caption: CaptionRow,
    checked: bool,
    /// `empty`, `has_tts`, `ready`, `generated`, `cached` or `failed`.
    status: String,
    duration: String,
    error: String,
    text: String,
    text_lower: String,
}

impl RowState {
    fn new(caption: CaptionRow) -> Self {
        let text = caption.text.replace('\n', " ");
        let (status, checked) = if caption.is_empty() {
            ("empty", false)
        } else if caption.has_existing_tts() {
            ("has_tts", true)
        } else {
            ("ready", true)
        };
        Self {
            text_lower: text.to_lowercase(),
            text,
            caption,
            checked,
            status: status.into(),
            duration: String::new(),
            error: String::new(),
        }
    }

    fn item(&self, source: usize) -> CaptionItem {
        let (status, tone) = match self.status.as_str() {
            "empty" => (tr("empty", &[]), 4),
            "has_tts" => (tr("has_tts", &[]), 0),
            "ready" => (tr("ready", &[]), 0),
            "failed" => (tr("error", &[]), 3),
            "cached" => ("Cache".to_owned(), 2),
            "generated" => ("OK".to_owned(), 1),
            other => (other.to_owned(), 0),
        };
        let c = &self.caption;
        CaptionItem {
            checked: self.checked,
            index: c.index.to_string().into(),
            start: fmt_us(c.start_us).into(),
            end: fmt_us(c.end_us()).into(),
            text: self.text.as_str().into(),
            has_tts: tr(if c.has_existing_tts() { "yes" } else { "no" }, &[]).into(),
            status: status.into(),
            duration: self.duration.as_str().into(),
            error: self.error.as_str().into(),
            tone,
            source: source as i32,
        }
    }
}

/// `MM:SS.mmm`, or `HH:MM:SS.mmm` from one hour.
fn fmt_us(us: i64) -> String {
    let total_ms = capdraft_core::pyfmt::round_half_even(us as f64 / 1000.0).max(0);
    let (h, rem) = (total_ms / 3_600_000, total_ms % 3_600_000);
    let (m, rem) = (rem / 60_000, rem % 60_000);
    let (s, ms) = (rem / 1000, rem % 1000);
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}.{ms:03}")
    } else {
        format!("{m:02}:{s:02}.{ms:03}")
    }
}

/// What workers hand to the UI thread.
#[derive(Default)]
struct Inbox {
    logs: Vec<(String, String, Option<String>)>,
    items: HashMap<i64, TtsItemResult>,
    progress: Option<(f64, String)>,
}

struct State {
    config: AppConfig,
    /// `None` while a background job holds it.
    service: Option<ProjectTtsService>,
    rows: Vec<RowState>,
    visible: Vec<usize>,
    by_index: HashMap<i64, usize>,
    voices: Vec<VoiceOption>,
    voice_view: Vec<usize>,
    language_codes: Vec<String>,
    info: Option<CapCutProjectInfo>,
    log_text: Vec<String>,
    busy: bool,
}

struct Shared {
    state: Mutex<State>,
    inbox: Mutex<Inbox>,
    cancel: AtomicBool,
    notice_seq: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn app_root() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn level_kind(level: &str) -> i32 {
    match level {
        "SUCCESS" => 1,
        "WARNING" => 2,
        "ERROR" => 3,
        "CACHED" => 4,
        _ => 0,
    }
}

fn parse_f64(s: &str, default: f64, lo: f64, hi: f64) -> f64 {
    s.trim()
        .replace(',', ".")
        .parse::<f64>()
        .map_or(default, |v| v.clamp(lo, hi))
}

fn set_strings(ui: &AppWindow) {
    let t = ui.global::<Tr>();
    let s = |k: &str| SharedString::from(tr(k, &[]));
    t.set_window_title(s("window_title"));
    t.set_capcut_project(s("capcut_project"));
    t.set_choose_project(s("choose_project"));
    t.set_project_placeholder(s("project_placeholder"));
    t.set_reload_project(s("reload_project"));
    t.set_export_srt(s("export_srt"));
    t.set_split_project(s("split_project"));
    t.set_merge_videos(s("merge_videos"));
    t.set_voice_settings(s("voice_settings"));
    t.set_search_voice(s("search_voice"));
    t.set_language_label(s("language_label"));
    t.set_voice_label(s("voice_label"));
    t.set_voice_speed(s("voice_speed"));
    t.set_pitch(s("pitch"));
    t.set_existing_tts(s("existing_tts"));
    t.set_advanced_options(s("advanced_options"));
    t.set_use_cache(s("use_cache"));
    t.set_align_tts(s("align_tts"));
    t.set_trim_start(s("trim_start"));
    t.set_fade_in(s("fade_in"));
    t.set_search_caption(s("search_caption"));
    t.set_select_all(s("select_all"));
    t.set_deselect_all(s("deselect_all"));
    t.set_hide_empty(s("hide_empty"));
    t.set_without_tts(s("without_tts"));
    t.set_only_errors(s("only_errors"));
    t.set_table_start(s("table_start"));
    t.set_table_end(s("table_end"));
    t.set_table_content(s("table_content"));
    t.set_table_has_tts(s("table_has_tts"));
    t.set_table_status(s("table_status"));
    t.set_table_duration(s("table_duration"));
    t.set_table_error(s("table_error"));
    t.set_cancel(s("cancel"));
    t.set_app_settings(s("app_settings"));
    t.set_clear_log(s("clear_log"));
    t.set_copy_log(s("copy_log"));
    t.set_save_log(s("save_log"));
    ui.set_generate_text(s("generate_attach"));
    ui.set_progress_text(s("ready"));
    ui.set_empty_text("Chưa có caption — chọn project để bắt đầu.".into());
    ui.set_tone_options(ModelRc::new(VecModel::from(vec![
        SharedString::from("Giữ nguyên cao độ khi đổi tốc độ"),
        SharedString::from("Đổi cao độ theo tốc độ"),
    ])));
    ui.set_existing_options(ModelRc::new(VecModel::from(vec![
        s("replace_tts"),
        s("skip_existing_tts"),
    ])));
}

fn theme_index(mode: &str) -> i32 {
    match mode {
        "light" => 1,
        "dark" => 2,
        _ => 0,
    }
}

/// Start the app and run the event loop.
///
/// # Errors
/// When the window cannot be created.
pub fn run() -> Result<(), slint::PlatformError> {
    let config = AppConfig::load(&app_root());
    i18n::set_language(config.language());
    let ui = AppWindow::new()?;
    set_strings(&ui);
    ui.set_theme_mode(theme_index(config.theme_mode()));
    let clip = match config.get("default_clip_speed") {
        Some(v) => capdraft_core::pyval::to_float(v)
            .ok()
            .filter(|x| *x > 0.0)
            .unwrap_or(1.0),
        None => 1.0,
    };
    ui.set_clip_speed(format_g(clip).into());

    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            service: None,
            config: config.clone(),
            rows: vec![],
            visible: vec![],
            by_index: HashMap::new(),
            voices: vec![],
            voice_view: vec![],
            language_codes: vec![String::new()],
            info: None,
            log_text: vec![],
            busy: false,
        }),
        inbox: Mutex::new(Inbox::default()),
        cancel: AtomicBool::new(false),
        notice_seq: AtomicU64::new(0),
    });
    let rows_model: Rc<VecModel<CaptionItem>> = Rc::new(VecModel::default());
    let log_model: Rc<VecModel<LogItem>> = Rc::new(VecModel::default());
    ui.set_rows(ModelRc::from(rows_model.clone()));
    ui.set_log_lines(ModelRc::from(log_model.clone()));

    let log_shared = Arc::clone(&shared);
    let service = ProjectTtsService::new(
        config,
        transport(),
        Some(Arc::new(move |e: &GenerationLogEvent| {
            lock(&log_shared.inbox).logs.push((
                e.level.clone(),
                e.message.clone(),
                e.stage.clone(),
            ));
        })),
    );
    lock(&shared.state).service = Some(service);

    let ctx = Ctx {
        ui: ui.as_weak(),
        shared,
        rows: rows_model,
        logs: log_model,
        follow_ticks: Rc::default(),
    };
    ctx.register();
    ctx.wire();
    ctx.load_voices();
    update_align_hint(&ctx);
    // `CapDraft-TTS <project>`: open a project folder or draft_content.json (drag and drop onto the exe)
    if let Some(p) = std::env::args_os().nth(1) {
        let p = PathBuf::from(p);
        ui.set_project_path(p.to_string_lossy().into_owned().into());
        ctx.load_project(p);
    }

    // apply worker output every 100 ms
    let pump = Timer::default();
    {
        let ctx = ctx.clone();
        pump.start(TimerMode::Repeated, Duration::from_millis(100), move || {
            ctx.drain()
        });
    }
    ui.run()
}

#[derive(Clone)]
struct Ctx {
    ui: Weak<AppWindow>,
    shared: Arc<Shared>,
    rows: Rc<VecModel<CaptionItem>>,
    logs: Rc<VecModel<LogItem>>,
    /// Pump ticks left during which the log is re-scrolled to its newest line (the
    /// list and the window settle their sizes a few frames after a line arrives).
    follow_ticks: Rc<std::cell::Cell<u8>>,
}

impl Ctx {
    fn ui(&self) -> Option<AppWindow> {
        self.ui.upgrade()
    }

    fn append_log(&self, level: &str, message: &str, stage: Option<&str>) {
        let ts = chrono_now();
        let prefix = stage.map(|s| format!("[{s}] ")).unwrap_or_default();
        let line = format!("{ts} {level:<7} {prefix}{message}");
        self.logs.push(LogItem {
            text: line.as_str().into(),
            kind: level_kind(level),
        });
        if self.logs.row_count() > MAX_LOG_LINES {
            self.logs.remove(0);
        }
        if let Some(ui) = self.ui() {
            ui.set_log_revision(ui.get_log_revision().wrapping_add(1));
        }
        self.follow_ticks.set(5);
        let mut st = lock(&self.shared.state);
        st.log_text.push(line);
        if st.log_text.len() > MAX_LOG_LINES {
            st.log_text.remove(0);
        }
    }

    fn notify(&self, kind: i32, title: &str, body: &str, ms: u64) {
        let Some(ui) = self.ui() else { return };
        ui.set_notice_kind(kind);
        ui.set_notice_title(title.into());
        ui.set_notice_body(body.into());
        ui.set_notice_visible(true);
        let seq = self.shared.notice_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let (weak, shared) = (self.ui.clone(), Arc::clone(&self.shared));
        Timer::single_shot(Duration::from_millis(ms), move || {
            if shared.notice_seq.load(Ordering::Relaxed) == seq
                && let Some(ui) = weak.upgrade()
            {
                ui.set_notice_visible(false);
            }
        });
    }

    fn set_busy(&self, busy: bool) {
        lock(&self.shared.state).busy = busy;
        if let Some(ui) = self.ui() {
            ui.set_busy(busy);
            ui.set_cancelling(false);
            if busy {
                ui.set_generate_text(tr("generating", &[]).into());
            }
        }
    }

    /// Worker output -> window.
    fn drain(&self) {
        let inbox = std::mem::take(&mut *lock(&self.shared.inbox));
        let ticks = self.follow_ticks.get();
        if ticks > 0 {
            self.follow_ticks.set(ticks - 1);
            if let Some(ui) = self.ui() {
                ui.invoke_follow_log();
            }
        }
        for (level, msg, stage) in &inbox.logs {
            self.append_log(level, msg, stage.as_deref());
        }
        if let Some((p, msg)) = inbox.progress
            && let Some(ui) = self.ui()
        {
            let p = if self.shared.cancel.load(Ordering::Relaxed) {
                p.min(0.99)
            } else {
                p
            };
            ui.set_progress(p as f32);
            ui.set_progress_text(format!("{:.0}% {msg}", p * 100.0).trim().into());
        }
        if !inbox.items.is_empty() {
            let mut st = lock(&self.shared.state);
            let mut touched = Vec::new();
            for (idx, res) in &inbox.items {
                let Some(&row) = st.by_index.get(idx) else {
                    continue;
                };
                let r = &mut st.rows[row];
                match res.status {
                    "Cached" => r.status = "cached".into(),
                    "Failed" => {
                        r.status = "failed".into();
                        r.error = res.error.clone().unwrap_or_default();
                    }
                    _ => r.status = "generated".into(),
                }
                let dur = res.duration.unwrap_or(0.0);
                if dur > 0.0 {
                    r.duration = format!("{dur:.2}s");
                }
                touched.push(row);
            }
            let only_errors = self.ui().is_some_and(|u| u.get_only_errors());
            if only_errors {
                drop(st);
                self.apply_filter();
            } else {
                for row in touched {
                    if let Ok(pos) = st.visible.binary_search(&row) {
                        let item = st.rows[row].item(row);
                        self.rows.set_row_data(pos, item);
                    }
                }
            }
        }
    }

    fn apply_filter(&self) {
        let Some(ui) = self.ui() else { return };
        let q = capdraft_core::pyfmt::py_strip(&ui.get_caption_search()).to_lowercase();
        let (hide_empty, no_tts, errors) = (
            ui.get_hide_empty(),
            ui.get_only_no_tts(),
            ui.get_only_errors(),
        );
        let mut st = lock(&self.shared.state);
        let visible: Vec<usize> = st
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                (q.is_empty() || r.text_lower.contains(&q))
                    && !(hide_empty && (r.caption.is_empty() || r.status == "empty"))
                    && !(no_tts && r.caption.has_existing_tts())
                    && !(errors && !(r.status == "failed" || !r.error.trim().is_empty()))
            })
            .map(|(i, _)| i)
            .collect();
        let items: Vec<CaptionItem> = visible.iter().map(|&i| st.rows[i].item(i)).collect();
        st.visible = visible;
        let (selected, total, shown) = (
            st.rows.iter().filter(|r| r.checked).count(),
            st.rows.len(),
            st.visible.len(),
        );
        let has_rows = !st.rows.is_empty();
        drop(st);
        self.rows.set_vec(items);
        ui.set_selection_text(
            tr(
                "selection",
                &[
                    ("selected", &selected.to_string()),
                    ("total", &total.to_string()),
                    ("visible", &shown.to_string()),
                ],
            )
            .into(),
        );
        ui.set_empty_text(
            if has_rows {
                tr("empty_filter", &[])
            } else {
                "Chưa có caption — chọn project để bắt đầu.".into()
            }
            .into(),
        );
    }

    fn refresh_selection_label(&self) {
        let Some(ui) = self.ui() else { return };
        let st = lock(&self.shared.state);
        let (selected, total, shown) = (
            st.rows.iter().filter(|r| r.checked).count(),
            st.rows.len(),
            st.visible.len(),
        );
        ui.set_selection_text(
            tr(
                "selection",
                &[
                    ("selected", &selected.to_string()),
                    ("total", &total.to_string()),
                    ("visible", &shown.to_string()),
                ],
            )
            .into(),
        );
    }

    fn wire(&self) {
        let Some(ui) = self.ui() else { return };
        // The ComboBox resets its index (deferred) whenever its model property
        // changes, so install the models once and only edit them in place.
        ui.set_languages(ModelRc::new(VecModel::<SharedString>::default()));
        ui.set_voices(ModelRc::new(VecModel::<SharedString>::default()));
        macro_rules! on {
            ($setter:ident, |$c:ident $(, $a:ident)*| $body:expr) => {{
                let $c = self.clone();
                ui.$setter(move |$($a),*| $body);
            }};
        }
        on!(on_browse_project, |c| c.browse_project());
        on!(on_reload_project, |c| c.reload_project());
        on!(on_export_srt, |c| c.export_srt());
        on!(on_split_project, |c| c.split_project());
        on!(on_merge_videos, |c| c.merge_videos());
        on!(on_language_changed, |c, _i| c.refresh_voices());
        on!(on_voice_search_edited, |c, _t| c.refresh_voices());
        on!(on_voice_changed, |c, _i| c.show_voice_meta());
        on!(on_trim_edited, |c| update_align_hint(&c));
        on!(on_generate, |c| c.start_generate());
        on!(on_cancel, |c| c.cancel());
        on!(on_open_settings, |c| c.open_settings());
        on!(on_dismiss_notice, |c| {
            if let Some(ui) = c.ui() {
                ui.set_notice_visible(false);
            }
        });
        on!(on_clear_log, |c| {
            c.logs.set_vec(Vec::new());
            lock(&c.shared.state).log_text.clear();
        });
        on!(on_copy_log, |c| {
            let text = lock(&c.shared.state).log_text.join("\n");
            match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text)) {
                Ok(()) => c.notify(1, &tr("copy_log", &[]), "", 2000),
                Err(e) => c.notify(3, &tr("copy_log", &[]), &e.to_string(), 4000),
            }
        });
        on!(on_save_log, |c| c.save_log());
        on!(on_select_visible, |c, on| {
            {
                let mut st = lock(&c.shared.state);
                let visible = st.visible.clone();
                for i in visible {
                    let r = &mut st.rows[i];
                    if on && r.caption.is_empty() {
                        continue;
                    }
                    r.checked = on;
                }
            }
            c.apply_filter();
        });
        on!(on_row_toggled, |c, source, on| {
            if let Some(r) = lock(&c.shared.state).rows.get_mut(source as usize) {
                r.checked = on;
            }
            c.refresh_selection_label();
        });
        // Filtering 5k rows is cheap; debounce typing anyway.
        let debounce = Rc::new(Timer::default());
        {
            let (c, t) = (self.clone(), Rc::clone(&debounce));
            ui.on_filters_changed(move || {
                let c2 = c.clone();
                t.start(
                    TimerMode::SingleShot,
                    Duration::from_millis(120),
                    move || c2.apply_filter(),
                );
            });
        }
        {
            let c = self.clone();
            ui.window().on_close_requested(move || c.on_close());
        }
        std::mem::forget(debounce); // lives as long as the window
    }

    fn on_close(&self) -> slint::CloseRequestResponse {
        if !lock(&self.shared.state).busy {
            return slint::CloseRequestResponse::HideWindow;
        }
        self.shared.cancel.store(true, Ordering::Relaxed);
        self.append_log(
            "WARNING",
            "Đóng ứng dụng — đang hủy tác vụ…",
            Some("cancelled"),
        );
        for _ in 0..80 {
            if !lock(&self.shared.state).busy {
                return slint::CloseRequestResponse::HideWindow;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.append_log(
            "ERROR",
            "Tác vụ vẫn chạy sau 8s — chưa đóng cửa sổ.",
            Some("cancelled"),
        );
        slint::CloseRequestResponse::KeepWindowShown
    }

    // ------------------------------------------------------------ voices

    fn load_voices(&self) {
        let url = lock(&self.shared.state).config.voice_catalog_url();
        let weak = self.ui.clone();
        std::thread::spawn(move || {
            let mut cat = capdraft_core::voice_catalog::VoiceCatalog::new(Some(&url));
            let res = cat
                .load(None, &UreqTransport, Duration::from_secs(20))
                .map(|v| (v, cat.languages()));
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let Some(ctx) = ctx_of(&ui) else { return };
                match res {
                    Ok((voices, langs)) => {
                        let mut names = vec![SharedString::from(tr("all", &[]))];
                        let mut codes = vec![String::new()];
                        for (lan, locale) in langs {
                            names.push(if locale.is_empty() {
                                if lan.is_empty() {
                                    "?".into()
                                } else {
                                    lan.as_str().into()
                                }
                            } else {
                                locale.as_str().into()
                            });
                            codes.push(if lan.is_empty() { locale } else { lan });
                        }
                        {
                            let mut st = lock(&ctx.shared.state);
                            st.voices = voices;
                            st.language_codes = codes;
                        }
                        let shown = names[0].clone();
                        set_model_rows(&ui.get_languages(), names);
                        ui.set_language_index(0);
                        ui.set_language_value(shown);
                        ctx.refresh_voices();
                    }
                    Err(e) => ctx.append_log(
                        "ERROR",
                        &format!("Không load được danh sách giọng: {e}"),
                        None,
                    ),
                }
            });
        });
    }

    fn refresh_voices(&self) {
        let Some(ui) = self.ui() else { return };
        let mut st = lock(&self.shared.state);
        let lan = st
            .language_codes
            .get(ui.get_language_index().max(0) as usize)
            .cloned()
            .unwrap_or_default();
        let q = capdraft_core::pyfmt::py_strip(&ui.get_voice_search()).to_lowercase();
        let view: Vec<usize> = st
            .voices
            .iter()
            .enumerate()
            .filter(|(_, v)| {
                (lan.is_empty()
                    || v.language_code == lan
                    || v.locale == lan
                    || v.locale.starts_with(&lan))
                    && (q.is_empty()
                        || format!("{} {} {}", v.display_name, v.voice_type, v.resource_id)
                            .to_lowercase()
                            .contains(&q))
            })
            .map(|(i, _)| i)
            .collect();
        let names: Vec<SharedString> = view
            .iter()
            .map(|&i| st.voices[i].display_name.as_str().into())
            .collect();
        // keep the configured default voice selected when it is listed
        let default_voice = st.config.default_voice();
        let pick = view
            .iter()
            .position(|&i| st.voices[i].voice_type == default_voice)
            .unwrap_or(0);
        st.voice_view = view;
        drop(st);
        let shown = names.get(pick).cloned().unwrap_or_default();
        set_model_rows(&ui.get_voices(), names);
        ui.set_voice_index(pick as i32);
        ui.set_voice_value(shown);
        self.show_voice_meta();
    }

    fn selected_voice(&self) -> Option<VoiceOption> {
        let ui = self.ui()?;
        let st = lock(&self.shared.state);
        let i = *st
            .voice_view
            .get(usize::try_from(ui.get_voice_index()).ok()?)?;
        st.voices.get(i).cloned()
    }

    fn show_voice_meta(&self) {
        let meta = self
            .selected_voice()
            .map(|v| format!("{} · resource_id={}", v.voice_type, v.resource_id))
            .unwrap_or_default();
        if let Some(ui) = self.ui() {
            ui.set_voice_meta(meta.into());
        }
    }

    // ------------------------------------------------------------ project

    fn browse_project(&self) {
        let Some(ui) = self.ui() else { return };
        let start = {
            let cur = ui.get_project_path().to_string();
            let p = PathBuf::from(cur.trim());
            if p.is_file() {
                p.parent().map(Path::to_path_buf)
            } else if p.is_dir() {
                Some(p)
            } else {
                lock(&self.shared.state).config.capcut_projects_path()
            }
        };
        let mut file = rfd::FileDialog::new()
            .set_title("Chọn draft_content.json (hoặc Hủy để chọn thư mục)")
            .add_filter("CapCut Draft", &["json"]);
        if let Some(s) = &start {
            file = file.set_directory(s);
        }
        let picked = file.pick_file().or_else(|| {
            let mut dir = rfd::FileDialog::new().set_title("Chọn thư mục project CapCut");
            if let Some(s) = &start {
                dir = dir.set_directory(s);
            }
            dir.pick_folder()
        });
        if let Some(p) = picked {
            ui.set_project_path(p.to_string_lossy().into_owned().into());
            self.load_project(p);
        }
    }

    fn reload_project(&self) {
        let Some(ui) = self.ui() else { return };
        let p = ui.get_project_path().to_string();
        if !p.trim().is_empty() {
            self.load_project(PathBuf::from(p.trim()));
        }
    }

    /// Load in the background (a 20 MB draft takes ~50 ms, but keep the UI free).
    fn load_project(&self, path: PathBuf) {
        let taken = lock(&self.shared.state).service.take();
        let Some(mut svc) = taken else {
            // another job holds the service for a moment: try again shortly
            let c = self.clone();
            Timer::single_shot(Duration::from_millis(150), move || c.load_project(path));
            return;
        };
        let (weak, shared) = (self.ui.clone(), Arc::clone(&self.shared));
        std::thread::spawn(move || {
            let res = svc.load_project(&path).and_then(|info| {
                let caps = svc.get_captions()?;
                svc.inspect_project(None)?;
                Ok((info, caps))
            });
            lock(&shared.state).service = Some(svc);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let Some(ctx) = ctx_of(&ui) else { return };
                ctx.drain();
                match res {
                    Ok((info, caps)) => ctx.show_project(&ui, info, caps),
                    Err(e) => {
                        ctx.notify(3, "Không đọc được project", &e, 5000);
                        ui.set_has_project(false);
                    }
                }
            });
        });
    }

    fn show_project(&self, ui: &AppWindow, info: CapCutProjectInfo, caps: Vec<CaptionRow>) {
        let line1 = format!(
            "{} · {} · {} FPS · {} · caption={} · đã có TTS={} · rỗng={}",
            info.project_name,
            info.resolution(),
            format_g(info.fps),
            info.duration_display(),
            info.caption_count,
            info.caption_with_tts_count,
            info.empty_caption_count
        );
        ui.set_project_info(format!("{line1}\n{}", info.draft_path.display()).into());
        ui.set_has_project(true);
        ui.set_generate_text(tr("generate_attach", &[]).into());
        let pd = info.project_directory.clone();
        if !(pd.join("draft_meta_info.json").exists() || pd.join("Timelines").is_dir()) {
            self.append_log(
                "WARNING",
                "Folder này không giống project CapCut đầy đủ (thiếu draft_meta_info.json / Timelines). Hãy chọn thư mục trong AppData\\Local\\CapCut\\User Data\\Projects\\com.lveditor.draft\\… nếu không CapCut sẽ không thấy audio.",
                Some("Project"),
            );
        }
        {
            let mut st = lock(&self.shared.state);
            st.by_index = caps.iter().enumerate().map(|(i, c)| (c.index, i)).collect();
            st.rows = caps.into_iter().map(RowState::new).collect();
            st.info = Some(info.clone());
        }
        self.apply_filter();
        update_align_hint(self);
        for line in [
            format!("Project: {}", info.project_name),
            format!("ID: {}", info.project_id),
            format!(
                "Version: {} / {}",
                info.version,
                info.new_version.clone().unwrap_or_else(|| "None".into())
            ),
            format!(
                "Canvas: {}×{} @ {} FPS",
                info.width,
                info.height,
                format_g(info.fps)
            ),
            format!("Duration: {}", info.duration_display()),
            format!(
                "Tracks: video={}, audio={}, text={}",
                info.video_track_count, info.audio_track_count, info.text_track_count
            ),
            format!(
                "Captions: total={}, existing_tts={}, empty={}",
                info.caption_count, info.caption_with_tts_count, info.empty_caption_count
            ),
        ] {
            self.append_log("INFO", &line, Some("Project"));
        }
    }

    fn export_srt(&self) {
        let info = lock(&self.shared.state).info.clone();
        let Some(info) = info else {
            self.notify(2, "Chưa có project", "Hãy chọn project CapCut trước.", 3000);
            return;
        };
        let name = info
            .project_directory
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(mut out) = rfd::FileDialog::new()
            .set_title("Xuất caption thành SRT")
            .set_directory(&info.project_directory)
            .set_file_name(format!("{name}.srt"))
            .add_filter("SubRip Subtitle", &["srt"])
            .save_file()
        else {
            return;
        };
        if !out
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("srt"))
        {
            out = PathBuf::from(format!("{}.srt", out.display()));
        }
        let res = {
            let mut st = lock(&self.shared.state);
            st.service.as_mut().map(|s| s.export_srt(&out))
        };
        self.drain();
        match res {
            Some(Ok(n)) => {
                self.append_log(
                    "SUCCESS",
                    &format!("Đã xuất {n} caption: {}", out.display()),
                    Some("SRT"),
                );
                self.notify(
                    1,
                    "Đã xuất SRT",
                    &format!("{n} caption · {}", out.display()),
                    4000,
                );
            }
            Some(Err(e)) => self.notify(3, "Không xuất được SRT", &e, 5000),
            None => {}
        }
    }

    // ------------------------------------------------------------ generate

    fn start_generate(&self) {
        let Some(ui) = self.ui() else { return };
        if lock(&self.shared.state).busy {
            return;
        }
        if capdraft_core::capcut_process::is_capcut_running() {
            let ans = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("CapCut đang chạy")
                .set_description("CapCut đang chạy.\n\nHãy đóng project CapCut trước khi tiếp tục để tránh project bị ghi đè.")
                .set_buttons(rfd::MessageButtons::OkCancelCustom("Kiểm tra lại".into(), "Hủy".into()))
                .show();
            if !matches!(ans, rfd::MessageDialogResult::Custom(ref s) if s == "Kiểm tra lại") {
                return;
            }
            if capdraft_core::capcut_process::is_capcut_running() {
                self.notify(2, "CapCut vẫn đang chạy", "Đóng CapCut rồi thử lại.", 3000);
                return;
            }
        }
        let selected: Vec<String> = lock(&self.shared.state)
            .rows
            .iter()
            .filter(|r| r.checked && !r.caption.text_segment_id.is_empty())
            .map(|r| r.caption.text_segment_id.clone())
            .collect();
        if selected.is_empty() {
            self.notify(
                2,
                "Chưa chọn caption",
                "Chọn ít nhất một caption để tạo TTS.",
                3000,
            );
            return;
        }
        let Some(voice) = self.selected_voice() else {
            self.notify(
                2,
                "Chưa chọn giọng đọc",
                "Chọn một giọng trong danh sách.",
                3000,
            );
            return;
        };
        let req = GenerateRequest {
            selected_caption_ids: selected,
            voice_type: voice.voice_type,
            resource_id: voice.resource_id,
            voice_display_name: voice.display_name,
            tts_rate: 1.0, // the API rate stays 1.0; the UI exposes the timeline clip speed
            capcut_clip_speed: parse_f64(&ui.get_clip_speed(), 1.0, 0.1, 10.0),
            tone_modify_mode: if ui.get_tone_index() == 1 {
                ToneModifyMode::FollowSpeed
            } else {
                ToneModifyMode::PreservePitch
            },
            existing_tts_mode: if ui.get_existing_index() == 1 {
                "skip_existing"
            } else {
                "replace_existing"
            }
            .into(),
            allow_partial_attach: true,
            use_cache: ui.get_use_cache(),
            alignment: NativeAudioAlignmentSettings {
                enabled: ui.get_align(),
                leading_trim_frames: parse_f64(&ui.get_trim_frames(), 3.0, 0.0, 12.0),
                fade_in_ms: parse_f64(&ui.get_fade_ms(), 8.0, 0.0, 20.0),
                ..Default::default()
            },
        };
        let Some(mut svc) = lock(&self.shared.state).service.take() else {
            return;
        };
        self.shared.cancel.store(false, Ordering::Relaxed);
        ui.set_progress(0.0);
        ui.set_progress_text("0%".into());
        self.set_busy(true);
        let (weak, shared) = (self.ui.clone(), Arc::clone(&self.shared));
        std::thread::spawn(move || {
            let (s1, s2, s3) = (
                Arc::clone(&shared),
                Arc::clone(&shared),
                Arc::clone(&shared),
            );
            let res = svc.generate_and_attach(
                &req,
                Some(Arc::new(move |p, m| {
                    lock(&s1.inbox).progress = Some((p, m.to_owned()))
                })),
                Some(Arc::new(move |i, r: &TtsItemResult| {
                    lock(&s2.inbox).items.insert(i, r.clone());
                })),
                Some(Arc::new(move || s3.cancel.load(Ordering::Relaxed))),
            );
            lock(&shared.state).service = Some(svc);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let Some(ctx) = ctx_of(&ui) else { return };
                ctx.drain();
                lock(&ctx.shared.state).busy = false;
                ui.set_busy(false);
                ui.set_cancelling(false);
                match res {
                    Ok(r) => ctx.on_generated(&ui, &r),
                    Err(e) => {
                        ui.set_generate_text(tr("generate_attach", &[]).into());
                        ui.set_progress_text("Lỗi".into());
                        ctx.notify(3, "Lỗi", &e, 8000);
                    }
                }
            });
        });
    }

    fn on_generated(&self, ui: &AppWindow, r: &GenerationResult) {
        let cancelled = r.cancelled || r.errors.iter().any(|e| e.to_lowercase().contains("cancel"));
        ui.set_generate_text(tr("generate_attach", &[]).into());
        if cancelled {
            ui.set_progress(ui.get_progress().min(0.99));
            ui.set_progress_text("Đã hủy".into());
            self.notify(
                2,
                "Đã hủy",
                &format!(
                    "Đã tạo {}, cache {}, lỗi {}, bỏ qua {}.",
                    r.generated, r.cached, r.failed, r.skipped
                ),
                5000,
            );
            return;
        }
        ui.set_progress(1.0);
        ui.set_progress_text("Hoàn tất".into());
        if r.success {
            let warn = r.completed_with_warnings();
            self.notify(
                if warn { 2 } else { 1 },
                if warn {
                    "Hoàn thành với cảnh báo"
                } else {
                    "Thành công"
                },
                &format!(
                    "Gắn {}/{}, lỗi {}, tạo mới {}, cache {}, bỏ qua {}. Chi tiết trong nhật ký.",
                    r.attached, r.selected, r.failed, r.generated, r.cached, r.skipped
                ),
                6500,
            );
            self.reload_project();
            ui.set_generate_text("Tạo lại".into());
        } else {
            let body = if r.errors.is_empty() {
                format!("generated={} failed={}", r.generated, r.failed)
            } else {
                r.errors
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            self.notify(3, "Thất bại", &body, 8000);
        }
    }

    fn cancel(&self) {
        if lock(&self.shared.state).busy {
            self.shared.cancel.store(true, Ordering::Relaxed);
            if let Some(ui) = self.ui() {
                ui.set_cancelling(true);
                ui.set_generate_text(tr("cancelling", &[]).into());
            }
            self.append_log("WARNING", "Đang hủy…", Some("cancelled"));
        }
    }

    // ------------------------------------------------------------ split / merge

    fn split_project(&self) {
        if lock(&self.shared.state).busy {
            return;
        }
        let (info, draft, edges) = {
            let st = lock(&self.shared.state);
            let draft = st.service.as_ref().and_then(|s| s.reader.draft.clone());
            let edges: Vec<i64> = st
                .rows
                .iter()
                .map(|r| r.caption.start_us)
                .filter(|s| *s > 0)
                .collect();
            (st.info.clone(), draft, edges)
        };
        let (Some(info), Some(draft)) = (info, draft) else {
            self.notify(2, "Chưa có project", "Hãy chọn project CapCut trước.", 3000);
            return;
        };
        let total = capdraft_core::pyval::int_or0(&draft["duration"]).unwrap_or(0);
        let total = if total > 0 { total } else { info.duration_us };
        if total < 2_000_000 {
            self.notify(
                2,
                "Project quá ngắn",
                "Cần ít nhất ~2s để tách 2 phần.",
                3000,
            );
            return;
        }
        let pd = info.project_directory.clone();
        let name = pd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = pd.parent().map(Path::to_path_buf).unwrap_or_default();
        let (p1, p2) = (
            parent.join(format!("{name}_part1")),
            parent.join(format!("{name}_part2")),
        );
        if p1.exists() || p2.exists() {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title(tr("split_project", &[]))
                .set_description(format!(
                    "Thư mục đích đã tồn tại:\n{name}_part1 / {name}_part2\n\nXóa chúng rồi chạy lại, hoặc đổi tên project nguồn."
                ))
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
            return;
        }
        let ok = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Info)
            .set_title(tr("split_project", &[]))
            .set_description(format!(
                "Tách project thành 2 bản copy (part1 / part2) theo mốc giữa timeline.\nProject gốc không bị sửa.\n\nNguồn: {name}\nThời lượng: {}\nXuất ra:\n  {}\n  {}\n\nSau đó mở từng part trong CapCut → Export → dùng Ghép video.",
                info.duration_display(),
                p1.display(),
                p2.display()
            ))
            .set_buttons(rfd::MessageButtons::OkCancelCustom("Tách".into(), tr("cancel", &[])))
            .show();
        if !matches!(ok, rfd::MessageDialogResult::Custom(ref s) if s == "Tách") {
            return;
        }
        if let Some(ui) = self.ui() {
            ui.set_progress(0.0);
            ui.set_progress_text(tr("split_project", &[]).into());
        }
        self.set_busy(true);
        let (weak, shared) = (self.ui.clone(), Arc::clone(&self.shared));
        std::thread::spawn(move || {
            let s1 = Arc::clone(&shared);
            let res = capdraft_core::project_split::split_project(
                &pd,
                &draft,
                &info.draft_path,
                None,
                Some(&edges),
                None,
                &mut |p, m| {
                    lock(&s1.inbox).progress = Some((p, m.to_owned()));
                },
            );
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let Some(ctx) = ctx_of(&ui) else { return };
                ctx.drain();
                lock(&ctx.shared.state).busy = false;
                ui.set_busy(false);
                ui.set_generate_text(tr("generate_attach", &[]).into());
                match res {
                    Ok(r) => {
                        ui.set_progress(1.0);
                        ui.set_progress_text(tr("ready", &[]).into());
                        let n = |p: &Path| {
                            p.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        };
                        ctx.append_log(
                            "SUCCESS",
                            &format!(
                                "Đã tách tại {:.2}s → {} ({} seg) + {} ({} seg)",
                                r.cut_us as f64 / 1e6,
                                n(&r.part1_dir),
                                r.part1_segments,
                                n(&r.part2_dir),
                                r.part2_segments
                            ),
                            Some("Split"),
                        );
                        ctx.append_log(
                            "INFO",
                            &format!("Part1: {}", r.part1_dir.display()),
                            Some("Split"),
                        );
                        ctx.append_log(
                            "INFO",
                            &format!("Part2: {}", r.part2_dir.display()),
                            Some("Split"),
                        );
                        ctx.notify(
                            1,
                            "Đã tách project",
                            &format!("{} + {}", n(&r.part1_dir), n(&r.part2_dir)),
                            6000,
                        );
                    }
                    Err(e) => {
                        ui.set_progress_text("Lỗi".into());
                        ctx.notify(3, "Lỗi", &e, 8000);
                    }
                }
            });
        });
    }

    fn merge_videos(&self) {
        if lock(&self.shared.state).busy {
            return;
        }
        let Some(paths) = rfd::FileDialog::new()
            .set_title(tr("merge_videos", &[]))
            .add_filter("Video", &["mp4", "mov", "mkv", "m4v"])
            .pick_files()
        else {
            return;
        };
        if paths.len() < 2 {
            self.notify(
                2,
                &tr("merge_videos", &[]),
                "Chọn ít nhất 2 video (theo thứ tự part1 → part2).",
                3000,
            );
            return;
        }
        let first = &paths[0];
        let stem = first
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut dlg = rfd::FileDialog::new()
            .set_title("Lưu video đã ghép")
            .set_file_name(format!("{stem}_merged.mp4"))
            .add_filter("MP4", &["mp4"]);
        if let Some(dir) = first.parent() {
            dlg = dlg.set_directory(dir);
        }
        let Some(out) = dlg.save_file() else { return };
        let (ffmpeg, ffprobe) = {
            let st = lock(&self.shared.state);
            (st.config.ffmpeg_path(), st.config.ffprobe_path())
        };
        if let Some(ui) = self.ui() {
            ui.set_progress(0.0);
            ui.set_progress_text(tr("merge_videos", &[]).into());
        }
        self.set_busy(true);
        let (weak, shared) = (self.ui.clone(), Arc::clone(&self.shared));
        std::thread::spawn(move || {
            let s1 = Arc::clone(&shared);
            let res = capdraft_core::video_merge::merge_videos(
                &paths,
                &out,
                Some(&ffmpeg),
                Some(&ffprobe),
                false,
                &mut |p, m| {
                    lock(&s1.inbox).progress = Some((p, m.to_owned()));
                },
            );
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let Some(ctx) = ctx_of(&ui) else { return };
                ctx.drain();
                lock(&ctx.shared.state).busy = false;
                ui.set_busy(false);
                ui.set_generate_text(tr("generate_attach", &[]).into());
                match res {
                    Ok(r) => {
                        ui.set_progress(1.0);
                        ui.set_progress_text(tr("ready", &[]).into());
                        let mode = if r.used_reencode {
                            "re-encode"
                        } else {
                            "stream-copy"
                        };
                        ctx.append_log(
                            "SUCCESS",
                            &format!("Đã ghép video ({mode}): {}", r.output_path.display()),
                            Some("Merge"),
                        );
                        ctx.notify(
                            1,
                            "Đã ghép video",
                            &format!("{mode} · {}", r.output_path.display()),
                            6000,
                        );
                    }
                    Err(e) => {
                        ui.set_progress_text("Lỗi".into());
                        ctx.notify(3, "Lỗi", &e, 8000);
                    }
                }
            });
        });
    }

    fn save_log(&self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr("save_log", &[]))
            .set_file_name("capdraft_tts_log.txt")
            .add_filter("Text", &["txt", "log"])
            .save_file()
        else {
            return;
        };
        let text = lock(&self.shared.state).log_text.join("\n") + "\n";
        match std::fs::write(&path, text) {
            Ok(()) => self.notify(1, &tr("save_log", &[]), &path.display().to_string(), 3000),
            Err(e) => self.notify(3, &tr("save_log", &[]), &e.to_string(), 5000),
        }
    }

    // ------------------------------------------------------------ settings

    fn open_settings(&self) {
        let Ok(win) = SettingsWindow::new() else {
            return;
        };
        let cfg = lock(&self.shared.state).config.clone();
        let s = |k: &str| SharedString::from(tr(k, &[]));
        win.set_heading(s("settings"));
        win.set_description(s("settings_description"));
        let langs = i18n::language_names();
        win.set_language_names(ModelRc::new(VecModel::from(
            langs
                .iter()
                .map(|(_, n)| SharedString::from(n.as_str()))
                .collect::<Vec<_>>(),
        )));
        win.set_language_index(
            langs
                .iter()
                .position(|(c, _)| *c == cfg.language())
                .unwrap_or(0) as i32,
        );
        win.set_theme_names(ModelRc::new(VecModel::from(vec![
            s("theme_auto"),
            s("theme_light"),
            s("theme_dark"),
        ])));
        win.set_theme_index(theme_index(cfg.theme_mode()));
        win.set_restart_note(s("restart_language"));
        let get = |k: &str, d: &str| {
            cfg.get(k)
                .map_or_else(|| d.to_owned(), capdraft_core::pyval::str_of)
        };
        win.set_device_json_path(get("device_json_path", "").into());
        win.set_ffprobe_path(get("ffprobe_path", "ffprobe").into());
        win.set_ffmpeg_path(get("ffmpeg_path", "ffmpeg").into());
        win.set_voice_catalog_url(cfg.voice_catalog_url().into());
        win.set_chunk_size(cfg.int("tts_chunk_size", 25).to_string().into());
        win.set_parallel_chunks(cfg.int("tts_parallel_chunks", 4).to_string().into());
        win.set_download_workers(cfg.int("tts_download_workers", 8).to_string().into());
        win.set_poll_interval(
            format_g(
                cfg.get("tts_poll_interval_sec")
                    .and_then(|v| capdraft_core::pyval::to_float(v).ok())
                    .unwrap_or(1.0),
            )
            .into(),
        );
        win.set_max_backups(cfg.int("max_backups", 10).to_string().into());
        win.set_profile_names(ModelRc::new(VecModel::from(vec![
            SharedString::from("upstream (MacBookPro17,4)"),
            SharedString::from("legacy (Python app)"),
        ])));
        let legacy = cfg
            .get("tts_device_profile")
            .map(capdraft_core::pyval::str_of)
            .is_some_and(|p| p.eq_ignore_ascii_case("legacy"));
        win.set_profile_index(i32::from(legacy));
        for (setter, key) in [
            (
                SettingsWindow::set_t_appearance as fn(&SettingsWindow, SharedString),
                "appearance",
            ),
            (SettingsWindow::set_t_language, "language"),
            (SettingsWindow::set_t_theme, "theme"),
            (SettingsWindow::set_t_paths, "paths"),
            (SettingsWindow::set_t_voices, "voices"),
            (SettingsWindow::set_t_performance, "performance"),
            (SettingsWindow::set_t_voice_url, "voice_catalog_url"),
            (SettingsWindow::set_t_reload_list, "reload_list"),
            (SettingsWindow::set_t_chunk, "captions_per_batch"),
            (SettingsWindow::set_t_parallel, "parallel_batches"),
            (SettingsWindow::set_t_workers, "audio_workers"),
            (SettingsWindow::set_t_poll, "poll_interval"),
            (SettingsWindow::set_t_backups, "backup_count"),
            (SettingsWindow::set_t_save, "save_settings"),
            (SettingsWindow::set_t_cancel, "cancel"),
            (SettingsWindow::set_t_choose, "choose"),
        ] {
            setter(&win, s(key));
        }

        let w = win.as_weak();
        win.on_cancel(move || {
            if let Some(w) = w.upgrade() {
                let _ = w.hide();
            }
        });
        let w = win.as_weak();
        win.on_browse_device(move || {
            if let (Some(w), Some(p)) = (
                w.upgrade(),
                rfd::FileDialog::new()
                    .add_filter("JSON", &["json"])
                    .pick_file(),
            ) {
                w.set_device_json_path(p.to_string_lossy().into_owned().into());
            }
        });
        let w = win.as_weak();
        win.on_reload_voices(move || {
            let Some(win) = w.upgrade() else { return };
            win.set_voice_status("Đang tải danh sách giọng...".into());
            let url = win.get_voice_catalog_url().to_string();
            let w2 = win.as_weak();
            std::thread::spawn(move || {
                let mut cat = capdraft_core::voice_catalog::VoiceCatalog::new(Some(&url));
                let msg = match cat.load(None, &UreqTransport, Duration::from_secs(20)) {
                    Ok(v) => format!("Đã tải {} giọng từ {}", v.len(), cat.url),
                    Err(e) => format!("Lỗi: {e}"),
                };
                let _ = w2.upgrade_in_event_loop(move |w| w.set_voice_status(msg.into()));
            });
        });
        let (w, ctx) = (win.as_weak(), self.clone());
        win.on_save(move || {
            let Some(win) = w.upgrade() else { return };
            let langs = i18n::language_names();
            let lang = langs
                .get(win.get_language_index().max(0) as usize)
                .map_or("vi", |(c, _)| *c);
            let theme = ["auto", "light", "dark"][win.get_theme_index().clamp(0, 2) as usize];
            let num = |s: SharedString, d: i64, lo: i64, hi: i64| {
                s.trim().parse::<i64>().map_or(d, |v| v.clamp(lo, hi))
            };
            let url = win.get_voice_catalog_url().trim().to_owned();
            let ffprobe = win.get_ffprobe_path().trim().to_owned();
            let ffmpeg = win.get_ffmpeg_path().trim().to_owned();
            {
                let mut st = lock(&ctx.shared.state);
                let c = &mut st.config;
                c.set(
                    "device_json_path",
                    Value::from(win.get_device_json_path().trim()),
                );
                c.set(
                    "voice_catalog_url",
                    Value::from(if url.is_empty() {
                        capdraft_core::voice_catalog::DEFAULT_VOICE_CATALOG_URL.to_owned()
                    } else {
                        url
                    }),
                );
                c.set(
                    "ffprobe_path",
                    Value::from(if ffprobe.is_empty() {
                        "ffprobe".to_owned()
                    } else {
                        ffprobe
                    }),
                );
                c.set(
                    "ffmpeg_path",
                    Value::from(if ffmpeg.is_empty() {
                        "ffmpeg".to_owned()
                    } else {
                        ffmpeg
                    }),
                );
                c.set(
                    "tts_chunk_size",
                    Value::from(num(win.get_chunk_size(), 25, 1, 100)),
                );
                c.set(
                    "tts_parallel_chunks",
                    Value::from(num(win.get_parallel_chunks(), 4, 1, 16)),
                );
                c.set(
                    "tts_download_workers",
                    Value::from(num(win.get_download_workers(), 8, 1, 32)),
                );
                c.set(
                    "tts_poll_interval_sec",
                    Value::from(parse_f64(&win.get_poll_interval(), 1.0, 0.1, 10.0)),
                );
                c.set(
                    "max_backups",
                    Value::from(num(win.get_max_backups(), 10, 1, 100)),
                );
                c.set(
                    "tts_device_profile",
                    Value::from(if win.get_profile_index() == 1 {
                        "legacy"
                    } else {
                        "upstream"
                    }),
                );
                c.set("language", Value::from(lang));
                c.set("theme_mode", Value::from(theme));
                let cfg = c.clone();
                if let Some(svc) = st.service.as_mut() {
                    svc.config = cfg;
                }
            }
            if let Some(ui) = ctx.ui() {
                ui.set_theme_mode(theme_index(theme));
            }
            ctx.notify(1, &tr("settings", &[]), &tr("restart_language", &[]), 3000);
            let _ = win.hide();
        });
        let _ = win.show();
    }
}

thread_local! {
    static CTX: std::cell::RefCell<Option<Ctx>> = const { std::cell::RefCell::new(None) };
}

/// The UI thread's context (closures sent from workers find it here).
/// Replace the rows of a string model installed by `wire` (keeps the model identity).
fn set_model_rows(model: &ModelRc<SharedString>, rows: Vec<SharedString>) {
    if let Some(vm) = model.as_any().downcast_ref::<VecModel<SharedString>>() {
        vm.set_vec(rows);
    }
}

/// The HTTP transport for TTS. Debug builds honour `CAPDRAFT_DEBUG_HTTP=http://host:port`,
/// which sends every `https://<host>/<path>` request to `<base>/<host>/<path>` (a local
/// fake CapCut server for GUI testing); release builds always talk to CapCut.
fn transport() -> Arc<dyn capdraft_core::tts::Transport> {
    #[cfg(debug_assertions)]
    if let Ok(base) = std::env::var("CAPDRAFT_DEBUG_HTTP") {
        return Arc::new(debug_http::Rewrite(base.trim_end_matches('/').to_owned()));
    }
    Arc::new(UreqTransport)
}

#[cfg(debug_assertions)]
mod debug_http {
    use capdraft_core::tts::{HttpResponse, Transport, UreqTransport};
    use std::time::Duration;

    pub struct Rewrite(pub String);

    impl Rewrite {
        fn map(&self, url: &str) -> String {
            url.strip_prefix("https://")
                .map_or_else(|| url.to_owned(), |rest| format!("{}/{rest}", self.0))
        }
    }

    impl Transport for Rewrite {
        fn post(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
            timeout: Duration,
        ) -> Result<HttpResponse, String> {
            UreqTransport.post(&self.map(url), headers, body, timeout)
        }
        fn get(&self, url: &str, timeout: Duration) -> Result<HttpResponse, String> {
            UreqTransport.get(&self.map(url), timeout)
        }
    }
}

fn ctx_of(_ui: &AppWindow) -> Option<Ctx> {
    CTX.with(|c| c.borrow().clone())
}

impl Ctx {
    fn register(&self) {
        CTX.with(|c| *c.borrow_mut() = Some(self.clone()));
    }
}

fn update_align_hint(ctx: &Ctx) {
    ctx.register();
    let Some(ui) = ctx.ui() else { return };
    let frames = parse_f64(&ui.get_trim_frames(), 3.0, 0.0, 12.0);
    let fps = lock(&ctx.shared.state)
        .info
        .as_ref()
        .map(|i| i.fps)
        .filter(|f| *f > 0.0)
        .unwrap_or(30.0);
    let ms = capdraft_core::pyfmt::round_half_even(frames / fps * 1000.0);
    ui.set_align_hint(
        format!(
            "{} khung hình @ {} FPS = {ms} ms",
            format_g(frames),
            format_g(fps)
        )
        .into(),
    );
}

fn chrono_now() -> String {
    capdraft_core::ids::local_now()
        .format("%H:%M:%S")
        .to_string()
}
