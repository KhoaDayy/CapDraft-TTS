//! The voice list, from the remote `Voice.json`
//! (port of `core/capcut_project/voice_catalog.py`; download lands with the HTTP client).

use crate::json::Value;
use crate::models::VoiceOption;
use crate::pyfmt::py_strip;
use crate::pyval::{PyResult, str_of, truthy};
use std::collections::HashSet;

pub const DEFAULT_VOICE_CATALOG_URL: &str =
    "https://raw.githubusercontent.com/KhoaDayy/CapDraft-TTS/refs/heads/main/Voice.json";

#[derive(Debug, Clone, Default)]
pub struct VoiceCatalog {
    pub url: String,
    voices: Vec<VoiceOption>,
}

fn field(item: &Value, k: &str) -> String {
    let v = &item[k];
    py_strip(&if truthy(v) { str_of(v) } else { String::new() }).to_owned()
}

impl VoiceCatalog {
    #[must_use]
    pub fn new(url: Option<&str>) -> Self {
        Self {
            url: py_strip(
                url.filter(|u| !u.is_empty())
                    .unwrap_or(DEFAULT_VOICE_CATALOG_URL),
            )
            .to_owned(),
            voices: vec![],
        }
    }

    /// Read voices from a parsed catalog (a JSON array of objects).
    /// Incomplete entries are skipped; duplicates are kept.
    ///
    /// # Errors
    /// `VoiceCatalogError` when `raw` is not an array or has no usable voice.
    pub fn load_data(&mut self, raw: &Value, source: &str) -> PyResult<Vec<VoiceOption>> {
        let Value::Array(items) = raw else {
            return Err(format!(
                "VoiceCatalogError: Voice catalog must be a JSON array: {source}"
            ));
        };
        let mut voices = Vec::new();
        for (i, item) in items.iter().enumerate() {
            if !item.is_object() {
                continue;
            }
            let voice_type = field(item, "voice_type");
            let resource_id = field(item, "resource_id");
            let dn = &item["display_name"];
            let display_name = if truthy(dn) {
                py_strip(&str_of(dn)).to_owned()
            } else if !voice_type.is_empty() {
                voice_type.clone()
            } else {
                format!("Voice {}", i + 1)
            };
            if voice_type.is_empty() || resource_id.is_empty() {
                continue;
            }
            voices.push(VoiceOption {
                language_code: field(item, "lan"),
                locale: field(item, "lang"),
                voice_type,
                display_name,
                resource_id,
            });
        }
        if voices.is_empty() {
            return Err(format!(
                "VoiceCatalogError: Voice catalog has no usable voices: {source}"
            ));
        }
        self.voices.clone_from(&voices);
        Ok(voices)
    }

    #[must_use]
    pub fn voices(&self) -> &[VoiceOption] {
        &self.voices
    }

    /// Unique `(language_code, locale)` pairs, first seen first.
    #[must_use]
    pub fn languages(&self) -> Vec<(String, String)> {
        let mut seen = HashSet::new();
        self.voices
            .iter()
            .filter(|v| {
                seen.insert(if v.language_code.is_empty() {
                    v.locale.clone()
                } else {
                    v.language_code.clone()
                })
            })
            .map(|v| (v.language_code.clone(), v.locale.clone()))
            .collect()
    }

    /// Voices matching a language (code, locale or locale prefix), a locale
    /// and a case-insensitive text query over name, type, id and locale.
    #[must_use]
    pub fn filter(
        &self,
        language_code: Option<&str>,
        locale: Option<&str>,
        query: Option<&str>,
    ) -> Vec<&VoiceOption> {
        let q = py_strip(query.unwrap_or("")).to_lowercase();
        self.voices
            .iter()
            .filter(|v| {
                if let Some(lc) = language_code.filter(|s| !s.is_empty())
                    && v.language_code != lc
                    && v.locale != lc
                    && !v.locale.starts_with(lc)
                {
                    return false;
                }
                if let Some(loc) = locale.filter(|s| !s.is_empty())
                    && v.locale != loc
                    && v.language_code != loc
                {
                    return false;
                }
                q.is_empty()
                    || format!(
                        "{} {} {} {}",
                        v.display_name, v.voice_type, v.resource_id, v.locale
                    )
                    .to_lowercase()
                    .contains(&q)
            })
            .collect()
    }

    #[must_use]
    pub fn find(&self, voice_type: &str, resource_id: Option<&str>) -> Option<&VoiceOption> {
        self.voices
            .iter()
            .find(|v| v.voice_type == voice_type && resource_id.is_none_or(|r| v.resource_id == r))
    }
}
