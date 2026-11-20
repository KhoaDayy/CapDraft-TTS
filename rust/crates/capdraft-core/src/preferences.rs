//! Appearance preferences (port of `core/preferences.py`).

/// `auto`, `light` or `dark`; anything else (or empty) is `auto`.
#[must_use]
pub fn normalize_theme_mode(value: &str) -> &'static str {
    match crate::pyfmt::py_strip(value).to_lowercase().as_str() {
        "light" => "light",
        "dark" => "dark",
        _ => "auto",
    }
}
