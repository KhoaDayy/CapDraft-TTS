//! JSON read and written exactly like Python's `json` module (ADR-002), on top
//! of [`crate::json`].

pub use crate::json::{Value, dumps_compact, dumps_indent};

/// Python `json.loads`.
///
/// # Errors
/// `JSONDecodeError: …` when `text` is not valid JSON.
pub fn loads(text: &str) -> Result<Value, String> {
    crate::json::parse(text)
}

/// A JSON `int`.
#[must_use]
pub fn int(v: i64) -> Value {
    Value::from(v)
}

/// A JSON `float`, written as Python's `repr`.
#[must_use]
pub fn float(v: f64) -> Value {
    Value::from(v)
}

/// Text written through Python's `open(..., "w")`: `\n` becomes `\r\n` on Windows.
#[must_use]
pub fn text_mode(s: &str) -> String {
    if cfg!(windows) {
        s.replace('\n', "\r\n")
    } else {
        s.to_owned()
    }
}
