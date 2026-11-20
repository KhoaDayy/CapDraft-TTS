//! `config.json` next to the app (port of `core/config.py`).
//!
//! Same file, same keys, same defaults and the same save format (`indent=4`)
//! as the Python app, so either build reads what the other wrote (ADR-009).

use crate::json::{Object, Value, dumps_indent};
use crate::pyfmt::py_strip;
use crate::pyjson::text_mode;
use crate::pyval::{str_of, to_float, truthy};
use crate::voice_catalog::DEFAULT_VOICE_CATALOG_URL;
use std::path::{Path, PathBuf};

/// `DEFAULT_CONFIG`, in the original's key order.
#[must_use]
pub fn default_config() -> Object {
    let mut o = Object::new();
    let s = |v: &str| Value::from(v);
    o.insert("ffprobe_path", s("ffprobe"));
    o.insert("ffmpeg_path", s("ffmpeg"));
    o.insert("capcut_tts_path", s("external/capcut-tts-api"));
    o.insert("device_json_path", s("external/capcut-tts-api/device.json"));
    o.insert("voice_catalog_url", s(DEFAULT_VOICE_CATALOG_URL));
    o.insert("default_voice", s("BV074_streaming"));
    o.insert("default_resource_id", s("7102355709945188865"));
    o.insert("default_rate", Value::from(1.0));
    o.insert("default_clip_speed", Value::from(1.0));
    o.insert("tts_chunk_size", Value::from(25));
    o.insert("tts_parallel_chunks", Value::from(4));
    o.insert("tts_poll_interval_sec", Value::from(1.0));
    o.insert("tts_download_workers", Value::from(8));
    o.insert("cache_path", s("cache"));
    o.insert("project_output_path", s("projects"));
    o.insert("capcut_projects_path", s(""));
    o.insert("max_backups", Value::from(10));
    o.insert("language", s("vi"));
    o.insert("theme_mode", s("auto"));
    o
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub app_root: PathBuf,
    pub config_path: PathBuf,
    data: Object,
    /// The last load or save problem, as the original prints it.
    pub last_error: Option<String>,
}

fn recursive_merge(base: &mut Object, over: &Object) {
    for (k, v) in over {
        match (base.get_mut(k), v) {
            (Some(Value::Object(b)), Value::Object(o)) => recursive_merge(b, o),
            _ => base.insert(k.clone(), v.clone()),
        }
    }
}

fn blank(v: Option<&Value>) -> bool {
    v.is_none_or(|v| py_strip(&if truthy(v) { str_of(v) } else { String::new() }).is_empty())
}

impl AppConfig {
    /// Load `config.json` from `app_root` (the folder of the executable),
    /// creating it with the defaults when it does not exist.
    #[must_use]
    pub fn load(app_root: &Path) -> Self {
        let mut c = Self {
            app_root: app_root.to_path_buf(),
            config_path: PathBuf::new(),
            data: default_config(),
            last_error: None,
        };
        c.config_path = c.resolve_app_path("config.json");
        if c.config_path.exists() {
            match crate::draft_reader::read_json_file(&c.config_path) {
                Ok(Value::Object(loaded)) => recursive_merge(&mut c.data, &loaded),
                Ok(other) => {
                    c.last_error = Some(format!(
                        "Error loading config.json: '{}' object has no attribute 'items'",
                        crate::pyval::type_name(&other)
                    ));
                }
                Err(e) => c.last_error = Some(format!("Error loading config.json: {e}")),
            }
        }
        c.normalize_legacy_keys();
        if !c.config_path.exists() {
            c.save();
        }
        c
    }

    fn normalize_legacy_keys(&mut self) {
        if blank(self.data.get("ffmpeg_path")) {
            self.data.insert("ffmpeg_path", Value::from("ffmpeg"));
        }
        let legacy = self.data.remove("voice_catalog_update_url");
        self.data.remove("voice_catalog_path");
        if blank(self.data.get("voice_catalog_url"))
            && let Some(l) = legacy.filter(truthy)
        {
            self.data.insert("voice_catalog_url", l);
        }
        if blank(self.data.get("voice_catalog_url")) {
            self.data
                .insert("voice_catalog_url", Value::from(DEFAULT_VOICE_CATALOG_URL));
        }
    }

    /// Write `config.json` (`indent=4`, UTF-8).
    pub fn save(&mut self) {
        let text = text_mode(&dumps_indent(&Value::Object(self.data.clone()), 4));
        if let Err(e) = std::fs::write(&self.config_path, text) {
            self.last_error = Some(format!("Error saving config.json: {e}"));
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    #[must_use]
    pub const fn data(&self) -> &Object {
        &self.data
    }

    /// Set and save.
    pub fn set(&mut self, key: &str, value: Value) {
        self.data.insert(key, value);
        self.save();
    }

    /// Absolute paths as they are; relative ones under the app folder.
    #[must_use]
    pub fn resolve_app_path(&self, path: impl AsRef<Path>) -> PathBuf {
        let p = path.as_ref();
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.app_root.join(p)
        }
    }

    fn str_key(&self, key: &str) -> String {
        self.get(key).map(str_of).unwrap_or_else(|| "None".into())
    }

    fn str_or(&self, key: &str, default: &str) -> String {
        match self.get(key) {
            Some(v) if truthy(v) => str_of(v),
            _ => default.to_owned(),
        }
    }

    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.resolve_app_path(self.str_key("cache_path"))
    }

    #[must_use]
    pub fn projects_dir(&self) -> PathBuf {
        self.resolve_app_path(
            self.get("project_output_path")
                .map_or_else(|| "projects".into(), str_of),
        )
    }

    /// The CapCut projects folder, when set and present.
    #[must_use]
    pub fn capcut_projects_path(&self) -> Option<PathBuf> {
        let raw = self.str_or("capcut_projects_path", "");
        let raw = py_strip(&raw);
        if raw.is_empty() {
            return None;
        }
        let p = PathBuf::from(raw);
        p.is_dir().then_some(p)
    }

    #[must_use]
    pub fn ffprobe_path(&self) -> String {
        self.str_key("ffprobe_path")
    }

    #[must_use]
    pub fn ffmpeg_path(&self) -> String {
        self.str_or("ffmpeg_path", "ffmpeg")
    }

    #[must_use]
    pub fn capcut_tts_path(&self) -> PathBuf {
        self.resolve_app_path(self.str_key("capcut_tts_path"))
    }

    #[must_use]
    pub fn device_json_path(&self) -> PathBuf {
        self.resolve_app_path(self.str_key("device_json_path"))
    }

    #[must_use]
    pub fn default_voice(&self) -> String {
        self.str_key("default_voice")
    }

    #[must_use]
    pub fn default_resource_id(&self) -> String {
        self.str_key("default_resource_id")
    }

    /// `float(default_rate or 1.0)`; a value that is not a number reads as 1.0.
    #[must_use]
    pub fn default_rate(&self) -> f64 {
        match self.get("default_rate") {
            Some(v) if truthy(v) => to_float(v).unwrap_or(1.0),
            _ => 1.0,
        }
    }

    #[must_use]
    pub fn voice_catalog_url(&self) -> String {
        py_strip(&self.str_or("voice_catalog_url", DEFAULT_VOICE_CATALOG_URL)).to_owned()
    }

    #[must_use]
    pub fn language(&self) -> &'static str {
        crate::i18n::normalize_language(&self.str_or("language", ""))
    }

    #[must_use]
    pub fn theme_mode(&self) -> &'static str {
        crate::preferences::normalize_theme_mode(&self.str_or("theme_mode", ""))
    }

    /// An integer setting, or `default` when missing or not a number.
    #[must_use]
    pub fn int(&self, key: &str, default: i64) -> i64 {
        match self.get(key) {
            Some(v) if truthy(v) => crate::pyval::to_int(v).unwrap_or(default),
            _ => default,
        }
    }
}
