//! JSON read and written exactly like Python's `json` module.
//!
//! CapCut drafts are written with `json.dumps(d, ensure_ascii=False,
//! separators=(",", ":"))` and manifests with `indent=2`. Keys keep their
//! order (`preserve_order`), integers keep every digit (`arbitrary_precision`)
//! and floats are printed as Python's `repr` prints them.

use crate::pyfmt::py_float_repr;
use serde_json::{Number, Value};

/// Python `json.loads`.
///
/// # Errors
/// The `serde_json` error text when `text` is not valid JSON.
pub fn loads(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("JSONDecodeError: {e}"))
}

/// `json.dumps(v, ensure_ascii=False, separators=(",", ":"))`.
#[must_use]
pub fn dumps_compact(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v, None, 0);
    out
}

/// `json.dumps(v, ensure_ascii=False, indent=n)`.
#[must_use]
pub fn dumps_indent(v: &Value, indent: usize) -> String {
    let mut out = String::new();
    write_value(&mut out, v, Some(indent), 0);
    out
}

fn newline(out: &mut String, indent: Option<usize>, level: usize) {
    if let Some(n) = indent {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', n * level));
    }
}

fn write_value(out: &mut String, v: &Value, indent: Option<usize>, level: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(out, n),
        Value::String(s) => write_str(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, level + 1);
                write_value(out, item, indent, level + 1);
            }
            newline(out, indent, level);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, level + 1);
                write_str(out, k);
                out.push_str(if indent.is_some() { ": " } else { ":" });
                write_value(out, item, indent, level + 1);
            }
            newline(out, indent, level);
            out.push('}');
        }
    }
}

/// Python parses `1`, `-0` as `int` and anything with `.`/`e` as `float`.
fn write_number(out: &mut String, n: &Number) {
    let text = n.to_string();
    if text.contains(['.', 'e', 'E']) {
        out.push_str(&py_float_repr(text.parse::<f64>().unwrap_or(f64::NAN)));
    } else if text == "-0" {
        out.push('0');
    } else {
        out.push_str(&text);
    }
}

/// `py_encode_basestring`: escape `"`, `\` and C0 controls only.
fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A JSON number from an `i64`, for building draft values.
#[must_use]
pub fn int(v: i64) -> Value {
    Value::Number(Number::from(v))
}

/// A JSON number from an `f64` that Python would write as a float.
///
/// Non-finite values become `null` (CapCut never stores them).
#[must_use]
pub fn float(v: f64) -> Value {
    if !v.is_finite() {
        return Value::Null;
    }
    // With `arbitrary_precision` the text is kept verbatim, so store the
    // Python spelling to keep a later `dumps` byte-identical.
    py_float_repr(v)
        .parse::<Number>()
        .map_or(Value::Null, Value::Number)
}
