//! TTS audio cache: `<cache>/<sha256>.mp3` plus `<sha256>.meta.json`
//! (port of `core/cache.py`, same layout, ADR-009).

use crate::project_split::{copy_file, io_err};
use crate::pyval::{PyResult, get_or, to_float};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TtsCache {
    pub cache_dir: PathBuf,
}

/// `f"{rate:.2f}"`: the exact binary value rounded half to even, as Python does.
#[must_use]
pub fn format_2f(rate: f64) -> String {
    format!("{rate:.2}")
}

impl TtsCache {
    /// # Errors
    /// When the folder cannot be created.
    pub fn new(cache_dir: &Path) -> PyResult<Self> {
        fs::create_dir_all(cache_dir).map_err(|e| io_err(&e, cache_dir))?;
        Ok(Self {
            cache_dir: cache_dir.to_path_buf(),
        })
    }

    /// sha256 of `text|voice|rate(2 decimals)|resource_id`.
    #[must_use]
    pub fn generate_hash(text: &str, voice: &str, rate: f64, resource_id: &str) -> String {
        let raw = format!("{text}|{voice}|{}|{resource_id}", format_2f(rate));
        let digest = Sha256::digest(raw.as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn meta_path(&self, hash: &str) -> PathBuf {
        self.cache_dir.join(format!("{hash}.meta.json"))
    }

    /// The cached file (non-empty) and its recorded duration in seconds.
    #[must_use]
    pub fn get_cached_file(
        &self,
        text: &str,
        voice: &str,
        rate: f64,
        resource_id: &str,
    ) -> (Option<PathBuf>, Option<f64>) {
        let hash = Self::generate_hash(text, voice, rate, resource_id);
        let path = self.cache_dir.join(format!("{hash}.mp3"));
        match fs::metadata(&path) {
            Ok(m) if m.is_file() && m.len() > 0 => (Some(path), self.read_duration(&hash)),
            _ => (None, None),
        }
    }

    fn read_duration(&self, hash: &str) -> Option<f64> {
        let meta = self.meta_path(hash);
        let text = fs::read_to_string(&meta).ok()?;
        let data = crate::json::parse(&text).ok()?;
        let zero = crate::json::Value::from(0.0);
        to_float(get_or(&data, "duration", &zero).ok()?).ok()
    }

    fn write_duration(&self, hash: &str, duration: f64) {
        let mut s = String::from("{\"duration\": ");
        crate::json::write_float(&mut s, duration);
        s.push('}');
        let _ = fs::write(self.meta_path(hash), s);
    }

    /// Copy `src` into the cache (via `.part`, then rename) and record its duration.
    ///
    /// # Errors
    /// `ValueError` for an empty or missing source; the copy's error otherwise.
    pub fn cache_audio(
        &self,
        src: &Path,
        text: &str,
        voice: &str,
        rate: f64,
        resource_id: &str,
        duration: f64,
    ) -> PyResult<PathBuf> {
        if !fs::metadata(src).is_ok_and(|m| m.len() > 0) {
            return Err(format!(
                "ValueError: Refuse to cache empty/missing audio: {}",
                src.display()
            ));
        }
        let hash = Self::generate_hash(text, voice, rate, resource_id);
        let dest = self.cache_dir.join(format!("{hash}.mp3"));
        let part = self.cache_dir.join(format!("{hash}.mp3.part"));
        let result = copy_file(src, &part).and_then(|()| {
            if fs::metadata(&part)?.len() == 0 {
                return Err(std::io::Error::other("Cache copy produced empty file"));
            }
            fs::rename(&part, &dest)
        });
        if let Err(e) = result {
            let _ = fs::remove_file(&part);
            return Err(io_err(&e, src));
        }
        self.write_duration(&hash, duration);
        Ok(dest)
    }
}
