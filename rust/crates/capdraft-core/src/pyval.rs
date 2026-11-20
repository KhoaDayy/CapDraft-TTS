//! Python value semantics over `serde_json::Value` (ADR-011).
//!
//! The Python core reads drafts loosely: `int(x or 0)`, `.get()` on whatever
//! value is there, `for x in v or []`, `==` across `int`/`float`/`bool`. These
//! helpers do the same and fail with the same `"<Type>: <message>"` text.

use crate::json::{Number, Object, Value};
use crate::pyfmt::{py_float_repr, py_strip};
use std::borrow::Cow;
use std::collections::HashSet;

/// `Err("TypeName: message")`, as the original raises it.
pub type PyResult<T> = Result<T, String>;

pub static NONE: Value = Value::Null;
static EMPTY_LIST: std::sync::LazyLock<Value> =
    std::sync::LazyLock::new(|| Value::Array(crate::json::Array::new()));

/// A number as Python sees it after `json.loads`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PyNum {
    Int(i128),
    Float(f64),
}

impl PyNum {
    #[must_use]
    pub fn as_f64(self) -> f64 {
        match self {
            Self::Int(i) => i as f64,
            Self::Float(f) => f,
        }
    }
}

#[must_use]
pub fn number(n: &Number) -> PyNum {
    match n {
        Number::Int(i) => PyNum::Int(i128::from(*i)),
        Number::BigInt(t) => PyNum::Int(t.parse().unwrap_or(if t.starts_with('-') {
            i128::MIN
        } else {
            i128::MAX
        })),
        Number::Float(f) => PyNum::Float(*f),
    }
}

/// `int`/`float`/`bool` value, the way `isinstance(x, (int, float))` sees it.
#[must_use]
pub fn num(v: &Value) -> Option<PyNum> {
    match v {
        Value::Bool(b) => Some(PyNum::Int(i128::from(*b))),
        Value::Number(n) => Some(number(n)),
        _ => None,
    }
}

#[must_use]
pub fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(n) => match number(n) {
            PyNum::Int(_) => "int",
            PyNum::Float(_) => "float",
        },
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    }
}

#[must_use]
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => match number(n) {
            PyNum::Int(i) => i != 0,
            PyNum::Float(f) => f != 0.0,
        },
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `v or default`.
#[must_use]
pub fn or<'a>(v: &'a Value, default: &'a Value) -> &'a Value {
    if truthy(v) { v } else { default }
}

/// `obj.get(key)`: `None` when missing, `AttributeError` when `obj` is not a dict.
///
/// # Errors
/// `AttributeError: '<type>' object has no attribute 'get'`.
pub fn get<'a>(obj: &'a Value, key: &str) -> PyResult<&'a Value> {
    match obj {
        Value::Object(m) => Ok(m.get(key).unwrap_or(&NONE)),
        other => Err(no_attr(other, "get")),
    }
}

/// `obj.get(key, default)`: `default` only when the key is missing.
///
/// # Errors
/// `AttributeError` when `obj` is not a dict.
pub fn get_or<'a>(obj: &'a Value, key: &str, default: &'a Value) -> PyResult<&'a Value> {
    match obj {
        Value::Object(m) => Ok(m.get(key).unwrap_or(default)),
        other => Err(no_attr(other, "get")),
    }
}

/// `obj.items()`.
///
/// # Errors
/// `AttributeError` when `obj` is not a dict.
pub fn items(obj: &Value) -> PyResult<&Object> {
    match obj {
        Value::Object(m) => Ok(m),
        other => Err(no_attr(other, "items")),
    }
}

#[must_use]
pub fn no_attr(v: &Value, attr: &str) -> String {
    format!(
        "AttributeError: '{}' object has no attribute '{attr}'",
        type_name(v)
    )
}

/// `for x in v`: list items, dict keys, string characters.
///
/// # Errors
/// `TypeError: '<type>' object is not iterable`.
pub fn iter(v: &Value) -> PyResult<Cow<'_, [Value]>> {
    match v {
        Value::Array(a) => Ok(Cow::Borrowed(a.as_slice())),
        Value::Object(m) => Ok(Cow::Owned(
            m.keys().map(|k| Value::String(k.clone())).collect(),
        )),
        Value::String(s) => Ok(Cow::Owned(
            s.chars()
                .map(|c| Value::String(c.to_string().into()))
                .collect(),
        )),
        other => Err(format!(
            "TypeError: '{}' object is not iterable",
            type_name(other)
        )),
    }
}

/// `for x in v or []`.
///
/// # Errors
/// `TypeError` when `v` is truthy and not iterable.
pub fn iter_or_empty(v: &Value) -> PyResult<Cow<'_, [Value]>> {
    iter(or(v, &EMPTY_LIST))
}

/// `len(v)`.
///
/// # Errors
/// `TypeError: object of type '<type>' has no len()`.
pub fn len(v: &Value) -> PyResult<usize> {
    match v {
        Value::Array(a) => Ok(a.len()),
        Value::Object(m) => Ok(m.len()),
        Value::String(s) => Ok(s.chars().count()),
        other => Err(format!(
            "TypeError: object of type '{}' has no len()",
            type_name(other)
        )),
    }
}

/// `v[0]`.
///
/// # Errors
/// `TypeError: '<type>' object is not subscriptable`, `KeyError: 0`, `IndexError`.
pub fn index0(v: &Value) -> PyResult<Value> {
    match v {
        Value::Array(a) => a
            .first()
            .cloned()
            .ok_or_else(|| "IndexError: list index out of range".to_owned()),
        Value::String(s) => s
            .chars()
            .next()
            .map(|c| Value::String(c.to_string().into()))
            .ok_or_else(|| "IndexError: string index out of range".to_owned()),
        Value::Object(_) => Err("KeyError: 0".to_owned()),
        other => Err(format!(
            "TypeError: '{}' object is not subscriptable",
            type_name(other)
        )),
    }
}

/// `key in container` for a string key.
///
/// # Errors
/// `TypeError: argument of type '<type>' is not iterable`.
pub fn contains_str(container: &Value, key: &str) -> PyResult<bool> {
    match container {
        Value::Object(m) => Ok(m.contains_key(key)),
        Value::Array(a) => Ok(a.iter().any(|x| x.as_str() == Some(key))),
        Value::String(s) => Ok(s.contains(key)),
        other => Err(format!(
            "TypeError: argument of type '{}' is not iterable",
            type_name(other)
        )),
    }
}

/// `x == "literal"`.
#[must_use]
pub fn is_str(v: &Value, s: &str) -> bool {
    v.as_str() == Some(s)
}

#[must_use]
pub fn is_dict(v: &Value) -> bool {
    v.is_object()
}

/// Python `==` (`1 == 1.0 == True`).
#[must_use]
pub fn eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| eq(p, q))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| eq(v, w)))
        }
        _ => match (num(a), num(b)) {
            (Some(PyNum::Int(x)), Some(PyNum::Int(y))) => x == y,
            (Some(x), Some(y)) => x.as_f64() == y.as_f64(),
            _ => false,
        },
    }
}

/// A hashable Python value, for sets and dicts keyed by draft values.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    None,
    Int(i128),
    Float(u64),
    Str(String),
}

/// `hash(v)`: numbers that compare equal share a key.
///
/// # Errors
/// `TypeError: unhashable type: 'list'` (or `'dict'`).
pub fn key(v: &Value) -> PyResult<Key> {
    match v {
        Value::Null => Ok(Key::None),
        Value::String(s) => Ok(Key::Str(s.to_string())),
        Value::Array(_) | Value::Object(_) => {
            Err(format!("TypeError: unhashable type: '{}'", type_name(v)))
        }
        _ => Ok(match num(v) {
            Some(PyNum::Int(i)) => Key::Int(i),
            Some(PyNum::Float(f)) if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e38 => {
                Key::Int(f as i128)
            }
            Some(PyNum::Float(f)) => Key::Float(f.to_bits()),
            None => Key::None,
        }),
    }
}

/// `v in d` where every key of `d` is a `str` (built with `str(id)`).
///
/// # Errors
/// `TypeError: unhashable type` for list and dict values.
pub fn in_str_keys<V>(v: &Value, d: &std::collections::HashMap<String, V>) -> PyResult<bool> {
    Ok(matches!(key(v)?, Key::Str(s) if d.contains_key(&s)))
}

/// `v in s` where every member of `s` is a `str`.
///
/// # Errors
/// `TypeError: unhashable type` for list and dict values.
pub fn in_str_set(v: &Value, s: &HashSet<String>) -> PyResult<bool> {
    Ok(matches!(key(v)?, Key::Str(k) if s.contains(&k)))
}

fn int_parse(s: &str) -> Option<i128> {
    let t = py_strip(s);
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
    {
        return None;
    }
    let clean: String = digits.chars().filter(|c| *c != '_').collect();
    if !clean.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i128 = clean.parse().ok()?;
    Some(if neg { -n } else { n })
}

fn clamp(i: i128) -> i64 {
    i.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// Python `int(v)`.
///
/// # Errors
/// `TypeError` for `None`/list/dict, `ValueError` for a string that is not an integer.
pub fn to_int(v: &Value) -> PyResult<i64> {
    match v {
        Value::String(s) => int_parse(s).map(clamp).ok_or_else(|| {
            format!(
                "ValueError: invalid literal for int() with base 10: {}",
                repr_str(s)
            )
        }),
        Value::Null | Value::Array(_) | Value::Object(_) => Err(format!(
            "TypeError: int() argument must be a string, a bytes-like object or a real number, not '{}'",
            type_name(v)
        )),
        _ => match num(v) {
            Some(PyNum::Int(i)) => Ok(clamp(i)),
            Some(PyNum::Float(f)) if f.is_nan() => {
                Err("ValueError: cannot convert float NaN to integer".into())
            }
            Some(PyNum::Float(f)) if f.is_infinite() => {
                Err("OverflowError: cannot convert float infinity to integer".into())
            }
            Some(PyNum::Float(f)) => Ok(f.trunc() as i64),
            None => Ok(0),
        },
    }
}

/// Python `int(v or 0)`.
///
/// # Errors
/// As [`to_int`].
pub fn int_or0(v: &Value) -> PyResult<i64> {
    if truthy(v) { to_int(v) } else { Ok(0) }
}

/// Python `float(v)`.
///
/// # Errors
/// `TypeError` for `None`/list/dict, `ValueError` for a string that is not a number.
pub fn to_float(v: &Value) -> PyResult<f64> {
    match v {
        Value::String(s) => {
            let t: String = py_strip(s).chars().filter(|c| *c != '_').collect();
            let ok_syntax = !t.is_empty() && !s.contains("__");
            t.parse::<f64>().ok().filter(|_| ok_syntax).ok_or_else(|| {
                format!(
                    "ValueError: could not convert string to float: {}",
                    repr_str(s)
                )
            })
        }
        Value::Null | Value::Array(_) | Value::Object(_) => Err(format!(
            "TypeError: float() argument must be a string or a real number, not '{}'",
            type_name(v)
        )),
        _ => Ok(num(v).map_or(0.0, PyNum::as_f64)),
    }
}

/// `str(f)` for a float (`inf`/`nan` spelled as Python's `str` does).
#[must_use]
pub fn float_str(f: f64) -> String {
    if f.is_nan() {
        "nan".into()
    } else if f.is_infinite() {
        if f > 0.0 { "inf".into() } else { "-inf".into() }
    } else {
        py_float_repr(f)
    }
}

fn num_str(n: &Number) -> String {
    match n {
        Number::Int(i) => i.to_string(),
        Number::BigInt(t) => t.to_string(),
        Number::Float(f) => float_str(*f),
    }
}

/// Python `str(v)`.
#[must_use]
pub fn str_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        other => repr(other),
    }
}

/// Python `repr(v)`.
#[must_use]
pub fn repr(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) => num_str(n),
        Value::String(s) => repr_str(s),
        Value::Array(a) => format!("[{}]", a.iter().map(repr).collect::<Vec<_>>().join(", ")),
        Value::Object(m) => format!(
            "{{{}}}",
            m.iter()
                .map(|(k, v)| format!("{}: {}", repr_str(k), repr(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// `str.isprintable()` for one character: false for controls (Cc), format
/// characters (Cf), separators other than space (Zs, Zl, Zp), surrogates and
/// private use. Unassigned code points are treated as printable.
fn is_printable(c: char) -> bool {
    let cp = c as u32;
    let not_printable = matches!(cp,
        0x00..=0x1f | 0x7f..=0xa0 | 0xad
        | 0x600..=0x605 | 0x61c | 0x6dd | 0x70f | 0x890..=0x891 | 0x8e2 | 0x180e
        | 0x1680 | 0x2000..=0x200f | 0x2028..=0x202f | 0x205f..=0x2064 | 0x2066..=0x206f
        | 0x3000 | 0xe000..=0xf8ff | 0xfeff | 0xfff9..=0xfffb
        | 0x110bd | 0x110cd | 0x13430..=0x1343f | 0x1bca0..=0x1bca3 | 0x1d173..=0x1d17a
        | 0xe0001 | 0xe0020..=0xe007f | 0xf0000..);
    !not_printable
}

/// Python `repr(str)`.
#[must_use]
pub fn repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            ' ' => out.push(' '),
            c if !is_printable(c) => {
                let cp = c as u32;
                if cp < 0x100 {
                    out.push_str(&format!("\\x{cp:02x}"));
                } else if cp < 0x10000 {
                    out.push_str(&format!("\\u{cp:04x}"));
                } else {
                    out.push_str(&format!("\\U{cp:08x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `isinstance(v, int)` (bools included, floats not).
#[must_use]
pub fn is_int(v: &Value) -> bool {
    matches!(num(v), Some(PyNum::Int(_)))
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;
    use crate::jv as json;

    #[test]
    fn int_matches_python() {
        assert_eq!(to_int(&json!(" 7 ")).unwrap(), 7);
        assert_eq!(to_int(&json!("1_000")).unwrap(), 1000);
        assert_eq!(to_int(&json!(-1.5)).unwrap(), -1);
        assert_eq!(to_int(&json!(true)).unwrap(), 1);
        assert_eq!(
            to_int(&json!("1.5")).unwrap_err(),
            "ValueError: invalid literal for int() with base 10: '1.5'"
        );
    }

    #[test]
    fn repr_matches_python() {
        assert_eq!(
            repr(&json!({"a": [1, 2.0, null, true, "x'y"]})),
            "{'a': [1, 2.0, None, True, \"x'y\"]}"
        );
        assert_eq!(repr_str("\u{a0}\u{200b}"), "'\\xa0\\u200b'");
    }

    #[test]
    fn eq_and_key_cross_numeric_types() {
        assert!(eq(&json!(1), &json!(1.0)));
        assert!(eq(&json!(true), &json!(1)));
        assert_eq!(key(&json!(1.0)).unwrap(), key(&json!(1)).unwrap());
        assert!(key(&json!([1])).is_err());
    }
}
