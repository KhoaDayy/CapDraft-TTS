//! UI translations (port of `core/i18n.py`).
//!
//! `i18n.json` is exported from the Python catalogs: `zh` and `ja` already
//! include the English fallbacks they inherit there.

use serde_json::{Map, Value};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};

static CATALOG: LazyLock<Value> = LazyLock::new(|| {
    // INFALLIBLE: the file is embedded at compile time and checked by tests.
    serde_json::from_str(include_str!("i18n.json")).expect("valid i18n.json")
});

/// Language codes in the order the original lists them.
pub const SUPPORTED: [&str; 4] = ["vi", "en", "zh", "ja"];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// Display names of the supported languages, in order.
#[must_use]
pub fn language_names() -> Vec<(&'static str, String)> {
    SUPPORTED
        .iter()
        .map(|code| {
            let name = CATALOG["languages"][code]
                .as_str()
                .unwrap_or(code)
                .to_owned();
            (*code, name)
        })
        .collect()
}

/// `en_US`, `EN-gb`, ` ja ` → `en`, `en`, `ja`; unknown or empty → `vi`.
#[must_use]
pub fn normalize_language(value: &str) -> &'static str {
    let lowered = crate::pyfmt::py_strip(if value.is_empty() { "vi" } else { value })
        .to_lowercase()
        .replace('_', "-");
    let code = lowered.split('-').next().unwrap_or("");
    SUPPORTED.into_iter().find(|c| *c == code).unwrap_or("vi")
}

pub fn set_language(language: &str) -> &'static str {
    let code = normalize_language(language);
    let idx = SUPPORTED.iter().position(|c| *c == code).unwrap_or(0);
    CURRENT.store(idx, Ordering::Relaxed);
    code
}

#[must_use]
pub fn get_language() -> &'static str {
    SUPPORTED[CURRENT.load(Ordering::Relaxed)]
}

fn catalog(lang: &str) -> Option<&'static Map<String, Value>> {
    CATALOG["catalogs"].get(lang).and_then(Value::as_object)
}

/// The text for `key` in `language`, else Vietnamese, else the key itself;
/// `{name}` placeholders are filled from `values`.
#[must_use]
pub fn translate(key: &str, language: Option<&str>, values: &[(&str, &str)]) -> String {
    let lang = match language {
        Some(l) => normalize_language(l),
        None => get_language(),
    };
    let text = catalog(lang)
        .and_then(|m| m.get(key))
        .or_else(|| catalog("vi").and_then(|m| m.get(key)))
        .and_then(Value::as_str)
        .unwrap_or(key);
    let mut out = text.to_owned();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// `translate` in the current language.
#[must_use]
pub fn tr(key: &str, values: &[(&str, &str)]) -> String {
    translate(key, None, values)
}
