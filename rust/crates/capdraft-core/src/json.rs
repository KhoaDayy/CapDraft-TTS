//! A JSON value with Python `dict`/`json` semantics, built for large CapCut drafts.
//!
//! - Objects keep insertion order; inserting an existing key replaces its value
//!   in place; duplicate keys while parsing keep the first position and the last
//!   value (as `json.loads` does).
//! - Numbers stay what Python would make of them: `int` (any size, `-0` is `0`)
//!   or `float` (`-0.0` keeps its sign, overflow becomes infinity).
//! - Arrays and objects are reference counted and copied on write, so cloning a
//!   19 MB draft is O(1) and only the parts that change are copied.
//! - Strings up to 24 bytes (almost every key, id and enum) are stored inline.
//!
//! [`parse`] reads `json.loads` input; [`dumps_compact`] and [`dumps_indent`]
//! write what `json.dumps(ensure_ascii=False)` writes, byte for byte.

use compact_str::CompactString;
use std::fmt::Write as _;
use std::ops::{Deref, DerefMut, Index};
use std::sync::{Arc, LazyLock};

pub type Str = CompactString;

/// A Python number after `json.loads`.
#[derive(Debug, Clone, PartialEq)]
pub enum Number {
    Int(i64),
    /// An `int` outside `i64`, kept as its decimal digits.
    BigInt(Arc<str>),
    Float(f64),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Number(Number),
    String(Str),
    Array(Array),
    Object(Object),
}

/// A copy-on-write list.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Array(Arc<Vec<Value>>);

/// A copy-on-write, insertion-ordered map with `dict` semantics.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Object(Arc<Vec<(Str, Value)>>);

pub static NULL: Value = Value::Null;
static EMPTY_OBJECT: LazyLock<Object> = LazyLock::new(Object::new);

impl Deref for Array {
    type Target = Vec<Value>;
    fn deref(&self) -> &Vec<Value> {
        &self.0
    }
}

impl DerefMut for Array {
    /// Copies the list first when it is shared.
    fn deref_mut(&mut self) -> &mut Vec<Value> {
        Arc::make_mut(&mut self.0)
    }
}

impl From<Vec<Value>> for Array {
    fn from(v: Vec<Value>) -> Self {
        Self(Arc::new(v))
    }
}

impl FromIterator<Value> for Array {
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        Self(Arc::new(iter.into_iter().collect()))
    }
}

impl Array {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Object {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_capacity(n: usize) -> Self {
        Self(Arc::new(Vec::with_capacity(n)))
    }

    fn position(&self, key: &str) -> Option<usize> {
        self.0.iter().position(|(k, _)| k.as_str() == key)
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0
            .iter()
            .find(|(k, _)| k.as_str() == key)
            .map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        let i = self.position(key)?;
        Some(&mut Arc::make_mut(&mut self.0)[i].1)
    }

    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.position(key).is_some()
    }

    /// `d[key] = value`: replaces in place, or appends a new key.
    pub fn insert(&mut self, key: impl Into<Str>, value: Value) {
        let key = key.into();
        match self.position(&key) {
            Some(i) => Arc::make_mut(&mut self.0)[i].1 = value,
            None => Arc::make_mut(&mut self.0).push((key, value)),
        }
    }

    /// `d.pop(key, None)`.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let i = self.position(key)?;
        Some(Arc::make_mut(&mut self.0).remove(i).1)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Str, &Value)> {
        self.0.iter().map(|(k, v)| (k, v))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Str, &mut Value)> {
        Arc::make_mut(&mut self.0).iter_mut().map(|(k, v)| (&*k, v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &Str> {
        self.0.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.0.iter().map(|(_, v)| v)
    }

    /// Keep the entries `keep` accepts, in order.
    pub fn retain(&mut self, mut keep: impl FnMut(&str, &Value) -> bool) {
        if self.0.iter().all(|(k, v)| keep(k, v)) {
            return;
        }
        Arc::make_mut(&mut self.0).retain(|(k, v)| keep(k, v));
    }
}

impl<'a> IntoIterator for &'a Object {
    type Item = (&'a Str, &'a Value);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (Str, Value)>,
        fn(&'a (Str, Value)) -> (&'a Str, &'a Value),
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().map(|(k, v)| (k, v))
    }
}

impl FromIterator<(String, Value)> for Object {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        iter.into_iter().map(|(k, v)| (Str::from(k), v)).collect()
    }
}

impl FromIterator<(Str, Value)> for Object {
    fn from_iter<I: IntoIterator<Item = (Str, Value)>>(iter: I) -> Self {
        let mut o = Self::new();
        for (k, v) in iter {
            o.insert(k, v);
        }
        o
    }
}

impl<'a> IntoIterator for &'a Array {
    type Item = &'a Value;
    type IntoIter = std::slice::Iter<'a, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl serde::Serialize for Object {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(self.len()))?;
        for (k, v) in self {
            map.serialize_entry(k.as_str(), v)?;
        }
        map.end()
    }
}

impl serde::Serialize for Array {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.0.iter())
    }
}

impl Value {
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match self {
            Self::Object(o) => o.get_mut(key),
            _ => None,
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Object(o) => o.get(key),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Number(Number::Int(i)) => Some(*i),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(Number::Int(i)) => Some(*i as f64),
            Self::Number(Number::Float(f)) => Some(*f),
            Self::Number(Number::BigInt(t)) => t.parse().ok(),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_array(&self) -> Option<&Array> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_array_mut(&mut self) -> Option<&mut Array> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }
    #[must_use]
    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Self::Object(o) => Some(o),
            _ => None,
        }
    }
    pub fn as_object_mut(&mut self) -> Option<&mut Object> {
        match self {
            Self::Object(o) => Some(o),
            _ => None,
        }
    }
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
    #[must_use]
    pub const fn is_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }
    #[must_use]
    pub const fn is_array(&self) -> bool {
        matches!(self, Self::Array(_))
    }
    #[must_use]
    pub const fn is_boolean(&self) -> bool {
        matches!(self, Self::Bool(_))
    }
    #[must_use]
    pub const fn is_number(&self) -> bool {
        matches!(self, Self::Number(_))
    }
    /// An empty `{}` shared value.
    #[must_use]
    pub fn empty_object() -> Self {
        Self::Object(EMPTY_OBJECT.clone())
    }
}

impl Index<&str> for Value {
    type Output = Value;
    /// `None`-like `Null` when missing or not an object.
    fn index(&self, key: &str) -> &Value {
        self.get(key).unwrap_or(&NULL)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Number(Number::Int(v))
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Self::Number(Number::Int(i64::from(v)))
    }
}
impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Self::Number(Number::Int(i64::from(v)))
    }
}
impl std::fmt::Display for Value {
    /// The `json.dumps` compact form.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&dumps_compact(self))
    }
}
impl From<usize> for Value {
    fn from(v: usize) -> Self {
        i64::try_from(v).map_or_else(
            |_| Self::Number(Number::BigInt(v.to_string().into())),
            Self::from,
        )
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Number(Number::Float(v))
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::String(v.into())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::String(v.into())
    }
}
impl From<Str> for Value {
    fn from(v: Str) -> Self {
        Self::String(v)
    }
}
impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Self::Array(v.into())
    }
}
impl From<Vec<String>> for Value {
    fn from(v: Vec<String>) -> Self {
        Self::Array(v.into_iter().map(Value::from).collect())
    }
}
impl From<Object> for Value {
    fn from(v: Object) -> Self {
        Self::Object(v)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Self::Null, Into::into)
    }
}

/// Literals: `serde_json::json!` syntax, converted (for small values only).
impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(b),
            serde_json::Value::Number(n) => match (n.as_i64(), n.as_f64()) {
                (Some(i), _) => Self::Number(Number::Int(i)),
                _ if n.is_u64() => Self::Number(Number::BigInt(n.to_string().into())),
                (_, Some(f)) => Self::Number(Number::Float(f)),
                _ => Self::Null,
            },
            serde_json::Value::String(s) => Self::String(s.into()),
            serde_json::Value::Array(a) => Self::Array(a.into_iter().map(Value::from).collect()),
            serde_json::Value::Object(m) => {
                let mut o = Object::with_capacity(m.len());
                for (k, v) in m {
                    o.insert(Str::from(k), Value::from(v));
                }
                Self::Object(o)
            }
        }
    }
}

impl serde::Serialize for Value {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self {
            Self::Null => s.serialize_unit(),
            Self::Bool(b) => s.serialize_bool(*b),
            Self::Number(Number::Int(i)) => s.serialize_i64(*i),
            Self::Number(Number::Float(f)) => s.serialize_f64(*f),
            Self::Number(Number::BigInt(t)) => s.serialize_str(t),
            Self::String(t) => s.serialize_str(t),
            Self::Array(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a.iter() {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            Self::Object(o) => {
                let mut map = s.serialize_map(Some(o.len()))?;
                for (k, v) in o {
                    map.serialize_entry(k.as_str(), v)?;
                }
                map.end()
            }
        }
    }
}

/// `serde_json::json!` syntax producing a [`Value`]; interpolated values may be
/// [`Value`]s. Meant for small literals.
#[macro_export]
macro_rules! jv {
    ($($t:tt)+) => {
        $crate::json::Value::from(::serde_json::json!($($t)+))
    };
}

// ------------------------------------------------------------------ parse

/// `json.loads`.
///
/// # Errors
/// `JSONDecodeError: <what> at line L column C`. Unlike Python, `NaN`,
/// `Infinity` and lone surrogates are rejected (ADR-002).
pub fn parse(text: &str) -> Result<Value, String> {
    parse_with(text, true)
}

/// [`parse`] on one thread only (same result, for tests and small inputs).
///
/// # Errors
/// As [`parse`].
pub fn parse_sequential(text: &str) -> Result<Value, String> {
    parse_with(text, false)
}

fn parse_with(text: &str, parallel: bool) -> Result<Value, String> {
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
        depth: 0,
        parallel,
        entries: Vec::new(),
        items: Vec::new(),
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("Extra data"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
    depth: u32,
    parallel: bool,
    /// Scratch stacks shared by all nesting levels: each container collects its
    /// children here and takes them with one exact-size allocation at its end.
    entries: Vec<(Str, Value)>,
    items: Vec<Value>,
}

const MAX_DEPTH: u32 = 900;

/// Bytes that end a plain run inside a string: `"`, `\` and C0 controls.
static STOP: [bool; 256] = {
    let mut t = [false; 256];
    let mut i = 0;
    while i < 0x20 {
        t[i] = true;
        i += 1;
    }
    t[b'"' as usize] = true;
    t[b'\\' as usize] = true;
    t
};

impl Parser<'_> {
    fn err(&self, what: &str) -> String {
        let before = &self.s[..self.i.min(self.s.len())];
        let line = before.iter().filter(|b| **b == b'\n').count() + 1;
        let col = before.iter().rev().take_while(|b| **b != b'\n').count() + 1;
        format!(
            "JSONDecodeError: {what}: line {line} column {col} (char {})",
            self.i
        )
    }

    #[inline]
    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.i) {
            self.i += 1;
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.s.get(self.i) {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'n') => self.lit("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.err("Expecting value")),
        }
    }

    fn lit(&mut self, word: &str, v: Value) -> Result<Value, String> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("Expecting value"))
        }
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Err(self.err("Too deeply nested"))
        } else {
            Ok(())
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.enter()?;
        self.i += 1;
        self.ws();
        if self.s.get(self.i) == Some(&b'}') {
            self.i += 1;
            self.depth -= 1;
            return Ok(Value::Object(Object::new()));
        }
        let base = self.entries.len();
        // 256-bit Bloom filter over the keys seen so far: an exact comparison
        // runs only when a key's bit is already set.
        let mut bloom = [0u64; 4];
        let mut maybe_dup = false;
        loop {
            self.ws();
            if self.s.get(self.i) != Some(&b'"') {
                self.entries.truncate(base);
                return Err(self.err("Expecting property name enclosed in double quotes"));
            }
            let k = self.string()?;
            let h = key_hash(k.as_bytes());
            let (word, bit) = ((h >> 62) as usize, 1u64 << ((h >> 56) & 63));
            if bloom[word] & bit != 0 && !maybe_dup {
                let ks = k.as_str();
                maybe_dup = self.entries[base..].iter().any(|(e, _)| e.as_str() == ks);
            }
            bloom[word] |= bit;
            self.ws();
            if self.s.get(self.i) != Some(&b':') {
                self.entries.truncate(base);
                return Err(self.err("Expecting ':' delimiter"));
            }
            self.i += 1;
            self.ws();
            let v = self.value()?;
            self.entries.push((k, v));
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                _ => {
                    self.entries.truncate(base);
                    return Err(self.err("Expecting ',' delimiter"));
                }
            }
        }
        self.depth -= 1;
        let mut entries: Vec<(Str, Value)> = self.entries.drain(base..).collect();
        if maybe_dup {
            entries = dedupe(entries);
        }
        Ok(Value::Object(Object(Arc::new(entries))))
    }

    fn array(&mut self) -> Result<Value, String> {
        self.enter()?;
        self.i += 1;
        self.ws();
        if self.s.get(self.i) == Some(&b']') {
            self.i += 1;
            self.depth -= 1;
            return Ok(Value::Array(Array::new()));
        }
        if self.parallel
            && self.depth <= PARALLEL_MAX_DEPTH
            && self.s.len() - self.i >= PARALLEL_MIN_BYTES
            && let Some(v) = self.array_parallel()
        {
            self.depth -= 1;
            return Ok(v);
        }
        let base = self.items.len();
        loop {
            self.ws();
            let v = self.value()?;
            self.items.push(v);
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                _ => {
                    self.items.truncate(base);
                    return Err(self.err("Expecting ',' delimiter"));
                }
            }
        }
        self.depth -= 1;
        let items: Vec<Value> = self.items.drain(base..).collect();
        Ok(Value::Array(Array(Arc::new(items))))
    }

    /// Parse a large array's elements on all cores. `None` (nothing consumed)
    /// when the array is small or anything looks off: the sequential parser
    /// then runs and reports errors exactly.
    fn array_parallel(&mut self) -> Option<Value> {
        use rayon::prelude::*;
        let spans = element_spans(self.s, self.i)?;
        let (Some(first), Some(last)) = (spans.first(), spans.last()) else {
            return None;
        };
        if spans.len() < PARALLEL_MIN_ITEMS || last.1 - first.0 < PARALLEL_MIN_BYTES {
            return None;
        }
        let (s, text, depth) = (self.s, self.text, self.depth);
        let chunk = spans
            .len()
            .div_ceil(rayon::current_num_threads() * 4)
            .max(16);
        let parts: Vec<Option<Vec<Value>>> = spans
            .par_chunks(chunk)
            .map(|chunk| {
                let mut p = Parser {
                    s,
                    text,
                    i: 0,
                    depth,
                    parallel: true,
                    entries: Vec::new(),
                    items: Vec::new(),
                };
                let mut out = Vec::with_capacity(chunk.len());
                for &(start, end, _) in chunk {
                    p.i = start;
                    out.push(p.value().ok()?);
                    if p.i != end {
                        return None;
                    }
                }
                Some(out)
            })
            .collect();
        let mut items = Vec::with_capacity(spans.len());
        for part in parts {
            items.extend(part?);
        }
        self.i = spans.last().map_or(self.i, |l| l.2);
        Some(Value::Array(Array(Arc::new(items))))
    }

    fn string(&mut self) -> Result<Str, String> {
        self.i += 1; // opening quote
        let start = self.i;
        // Fast path: no escapes.
        let run = find_stop(self.s, start);
        match run {
            Some(end) if self.s[end] == b'"' => {
                self.i = end + 1;
                return Ok(Str::from(&self.text[start..end]));
            }
            None => return Err(self.err("Unterminated string starting at")),
            _ => {}
        }
        let mut out = String::new();
        loop {
            let end = find_stop(self.s, self.i);
            let Some(end) = end else {
                return Err(self.err("Unterminated string starting at"));
            };
            out.push_str(&self.text[self.i..end]);
            self.i = end;
            match self.s[end] {
                b'"' => {
                    self.i += 1;
                    return Ok(Str::from(out));
                }
                b'\\' => {
                    let esc = *self
                        .s
                        .get(end + 1)
                        .ok_or_else(|| self.err("Unterminated string starting at"))?;
                    self.i = end + 2;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi)
                                && self.s[self.i..].starts_with(b"\\u")
                            {
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                                } else {
                                    return Err(self.err("Lone surrogate"));
                                }
                            } else {
                                hi
                            };
                            out.push(char::from_u32(cp).ok_or_else(|| self.err("Lone surrogate"))?);
                        }
                        _ => return Err(self.err("Invalid \\escape")),
                    }
                }
                _ => return Err(self.err("Invalid control character at")),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self
            .s
            .get(self.i..self.i + 4)
            .ok_or_else(|| self.err("Invalid \\uXXXX escape"))?;
        let s = std::str::from_utf8(h).map_err(|_| self.err("Invalid \\uXXXX escape"))?;
        let v = u32::from_str_radix(s, 16).map_err(|_| self.err("Invalid \\uXXXX escape"))?;
        self.i += 4;
        Ok(v)
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        let neg = self.s[self.i] == b'-';
        if neg {
            self.i += 1;
        }
        let int_start = self.i;
        match self.s.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                while let Some(b'0'..=b'9') = self.s.get(self.i) {
                    self.i += 1;
                }
            }
            _ => return Err(self.err("Expecting value")),
        }
        let int_end = self.i;
        let mut is_float = false;
        if self.s.get(self.i) == Some(&b'.') && matches!(self.s.get(self.i + 1), Some(b'0'..=b'9'))
        {
            is_float = true;
            self.i += 1;
            while let Some(b'0'..=b'9') = self.s.get(self.i) {
                self.i += 1;
            }
        }
        if let Some(b'e' | b'E') = self.s.get(self.i) {
            let mut j = self.i + 1;
            if let Some(b'+' | b'-') = self.s.get(j) {
                j += 1;
            }
            if matches!(self.s.get(j), Some(b'0'..=b'9')) {
                is_float = true;
                self.i = j;
                while let Some(b'0'..=b'9') = self.s.get(self.i) {
                    self.i += 1;
                }
            }
        }
        let text = &self.text[start..self.i];
        if is_float {
            return text
                .parse::<f64>()
                .map(|f| Value::Number(Number::Float(f)))
                .map_err(|_| self.err("Expecting value"));
        }
        let digits = &self.s[int_start..int_end];
        if digits.len() <= 18 {
            let mut n: i64 = 0;
            for d in digits {
                n = n * 10 + i64::from(d - b'0');
            }
            return Ok(Value::Number(Number::Int(if neg { -n } else { n })));
        }
        Ok(match text.parse::<i64>() {
            Ok(n) => Value::Number(Number::Int(n)),
            Err(_) => Value::Number(Number::BigInt(text.into())),
        })
    }
}

/// Arrays at most this deep whose elements span this many bytes, in at least
/// this many elements, are parsed in parallel.
const PARALLEL_MAX_DEPTH: u32 = 6;
const PARALLEL_MIN_BYTES: usize = 256 * 1024;
const PARALLEL_MIN_ITEMS: usize = 64;

/// `(start, end, after_close)` of each element of the array whose first
/// element starts at `i` (just after `[` and whitespace). `None` when the
/// array is malformed (the sequential parser reports why).
fn element_spans(s: &[u8], mut i: usize) -> Option<Vec<(usize, usize, usize)>> {
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    loop {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = s.get(i) {
            i += 1;
        }
        let start = i;
        i = skip_value(s, i)?;
        spans.push((start, i, 0));
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = s.get(i) {
            i += 1;
        }
        match s.get(i)? {
            b',' => i += 1,
            b']' => {
                let after = i + 1;
                for sp in &mut spans {
                    sp.2 = after;
                }
                return Some(spans);
            }
            _ => return None,
        }
    }
}

/// End of the value starting at `i`, found by matching brackets and skipping
/// strings without building anything.
fn skip_value(s: &[u8], mut i: usize) -> Option<usize> {
    let mut depth = 0usize;
    loop {
        match *s.get(i)? {
            b'"' => {
                i = skip_string(s, i)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'{' | b'[' => {
                depth += 1;
                i += 1;
            }
            b'}' | b']' => {
                depth = depth.checked_sub(1)?;
                i += 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ if depth == 0 => {
                while let Some(c) = s.get(i) {
                    if matches!(c, b',' | b']' | b'}' | b' ' | b'\t' | b'\n' | b'\r') {
                        break;
                    }
                    i += 1;
                }
                return Some(i);
            }
            _ => i += 1,
        }
    }
}

fn skip_string(s: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    loop {
        let k = find_stop(s, j)?;
        match s[k] {
            b'"' => return Some(k + 1),
            b'\\' => j = k + 2,
            _ => return None,
        }
    }
}

/// Cheap key hash for the duplicate-key filter: length, first and last 8 bytes.
#[inline]
fn key_hash(b: &[u8]) -> u64 {
    let word = |s: &[u8]| {
        let mut w = [0u8; 8];
        w[..s.len()].copy_from_slice(s);
        u64::from_le_bytes(w)
    };
    let n = b.len();
    let head = word(&b[..n.min(8)]);
    let tail = word(&b[n.saturating_sub(8)..]);
    (head ^ tail.rotate_left(29) ^ (n as u64)).wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

const LO: u64 = 0x0101_0101_0101_0101;
const HI: u64 = 0x8080_8080_8080_8080;

/// Bit 7 of each byte of `w` that is `"`, `\` or below 0x20 (exact up to the
/// first such byte, which is all the caller uses).
#[inline]
const fn stop_mask(w: u64) -> u64 {
    let q = w ^ (LO * b'"' as u64);
    let bs = w ^ (LO * b'\\' as u64);
    (q.wrapping_sub(LO) & !q & HI)
        | (bs.wrapping_sub(LO) & !bs & HI)
        | (w.wrapping_sub(LO * 0x20) & !w & HI)
}

/// Next `"`, `\` or C0 control byte at or after `from`, 8 bytes at a time.
#[inline]
fn find_stop(s: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 8 <= s.len() {
        let mut w = [0u8; 8];
        w.copy_from_slice(&s[i..i + 8]);
        let m = stop_mask(u64::from_le_bytes(w));
        if m != 0 {
            return Some(i + (m.trailing_zeros() / 8) as usize);
        }
        i += 8;
    }
    s[i..].iter().position(|b| STOP[*b as usize]).map(|n| i + n)
}

/// `dict` semantics for duplicate keys: first position, last value.
fn dedupe(entries: Vec<(Str, Value)>) -> Vec<(Str, Value)> {
    let mut out: Vec<(Str, Value)> = Vec::with_capacity(entries.len());
    let mut index: std::collections::HashMap<Str, usize> = std::collections::HashMap::new();
    for (k, v) in entries {
        if let Some(&i) = index.get(&k) {
            out[i].1 = v;
        } else {
            index.insert(k.clone(), out.len());
            out.push((k, v));
        }
    }
    out
}

// ------------------------------------------------------------------ dump

/// `json.dumps(v, ensure_ascii=False, separators=(",", ":"))`.
#[must_use]
pub fn dumps_compact(v: &Value) -> String {
    let mut out = String::with_capacity(4096);
    write_value(&mut out, v, None, 0);
    out
}

/// `json.dumps(v, ensure_ascii=False, indent=n)`.
#[must_use]
pub fn dumps_indent(v: &Value, indent: usize) -> String {
    let mut out = String::with_capacity(4096);
    write_value(&mut out, v, Some(indent), 0);
    out
}

/// Append the compact form of `v` to `out` (reuse one buffer for many writes).
pub fn dump_compact_into(out: &mut String, v: &Value) {
    write_value(out, v, None, 0);
}

fn newline(out: &mut String, indent: Option<usize>, level: usize) {
    if let Some(n) = indent {
        out.push('\n');
        for _ in 0..n * level {
            out.push(' ');
        }
    }
}

fn write_value(out: &mut String, v: &Value, indent: Option<usize>, level: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => write_number(out, n),
        Value::String(s) => write_str(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            if indent.is_none() && items.len() >= PARALLEL_MIN_ITEMS * 4 {
                write_array_parallel(out, items);
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

/// Large arrays: dump chunks on all cores, then join in order.
fn write_array_parallel(out: &mut String, items: &[Value]) {
    use rayon::prelude::*;
    let chunk = items
        .len()
        .div_ceil(rayon::current_num_threads() * 4)
        .max(64);
    let parts: Vec<String> = items
        .par_chunks(chunk)
        .map(|c| {
            let mut s = String::with_capacity(c.len() * 256);
            for (i, v) in c.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                write_value(&mut s, v, None, 0);
            }
            s
        })
        .collect();
    out.reserve(parts.iter().map(String::len).sum::<usize>() + parts.len() + 2);
    out.push('[');
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(p);
    }
    out.push(']');
}

fn write_number(out: &mut String, n: &Number) {
    match n {
        Number::Int(i) => out.push_str(itoa::Buffer::new().format(*i)),
        Number::BigInt(t) => out.push_str(t),
        Number::Float(f) => write_float(out, *f),
    }
}

/// Python `repr(float)` as `json.dumps` writes it.
pub fn write_float(out: &mut String, f: f64) {
    if f.is_nan() {
        out.push_str("NaN");
        return;
    }
    if f.is_infinite() {
        out.push_str(if f > 0.0 { "Infinity" } else { "-Infinity" });
        return;
    }
    if f == 0.0 {
        out.push_str(if f.is_sign_negative() { "-0.0" } else { "0.0" });
        return;
    }
    // Shortest round-trip digits from ryu, re-laid out the way Python does.
    let mut buf = ryu::Buffer::new();
    let s = buf.format_finite(f);
    let (neg, s) = s.strip_prefix('-').map_or((false, s), |r| (true, r));
    let (mant, exp) = s.split_once(['e', 'E']).unwrap_or((s, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let mut digits = [0u8; 32];
    let mut nd = 0;
    let mut point = None;
    for b in mant.bytes() {
        if b == b'.' {
            point = Some(nd);
        } else if nd < digits.len() {
            digits[nd] = b;
            nd += 1;
        }
    }
    let mut point = point.unwrap_or(nd) as i32 + exp;
    // strip leading zeros
    let mut first = 0;
    while first < nd - 1 && digits[first] == b'0' {
        first += 1;
        point -= 1;
    }
    let mut last = nd;
    while last > first + 1 && digits[last - 1] == b'0' {
        last -= 1;
    }
    let d = &digits[first..last];
    let sci_exp = point - 1;
    if neg {
        out.push('-');
    }
    let dstr = std::str::from_utf8(d).unwrap_or("0");
    if (-4..16).contains(&sci_exp) {
        if point <= 0 {
            out.push_str("0.");
            for _ in 0..-point {
                out.push('0');
            }
            out.push_str(dstr);
        } else if point as usize >= d.len() {
            out.push_str(dstr);
            for _ in 0..(point as usize - d.len()) {
                out.push('0');
            }
            out.push_str(".0");
        } else {
            out.push_str(&dstr[..point as usize]);
            out.push('.');
            out.push_str(&dstr[point as usize..]);
        }
    } else {
        out.push_str(&dstr[..1]);
        if d.len() > 1 {
            out.push('.');
            out.push_str(&dstr[1..]);
        }
        let _ = write!(
            out,
            "e{}{:02}",
            if sci_exp < 0 { '-' } else { '+' },
            sci_exp.unsigned_abs()
        );
    }
}

/// `py_encode_basestring`: escape `"`, `\` and C0 controls only, in runs.
fn write_str(out: &mut String, s: &str) {
    out.push('"');
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if !STOP[b as usize] {
            continue;
        }
        out.push_str(&s[start..i]);
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x08 => out.push_str("\\b"),
            0x0c => out.push_str("\\f"),
            c => {
                let _ = write!(out, "\\u{:04x}", c);
            }
        }
        start = i + 1;
    }
    out.push_str(&s[start..]);
    out.push('"');
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    fn rt(s: &str) -> String {
        dumps_compact(&parse(s).unwrap())
    }

    #[test]
    fn python_number_semantics() {
        assert_eq!(
            rt("[-0, -0.0, 1.0, 1, 1E5, 1e16, 1e-5, 123456789012345678901234, 1e400]"),
            "[0,-0.0,1.0,1,100000.0,1e+16,1e-05,123456789012345678901234,Infinity]"
        );
        assert_eq!(
            rt("[0.1, 100.0, 1e15, 5e-324, 1.7976931348623157e308, 0.0001, 9999999999999998.0]"),
            "[0.1,100.0,1000000000000000.0,5e-324,1.7976931348623157e+308,0.0001,9999999999999998.0]"
        );
    }

    #[test]
    fn dict_semantics() {
        assert_eq!(rt(r#"{"b":1,"a":2,"b":3}"#), r#"{"b":3,"a":2}"#);
        let mut v = parse(r#"{"b":1,"a":2}"#).unwrap();
        let copy = v.clone();
        v.as_object_mut().unwrap().insert("b", Value::from(9));
        v.as_object_mut().unwrap().insert("c", Value::Null);
        assert_eq!(dumps_compact(&v), r#"{"b":9,"a":2,"c":null}"#);
        assert_eq!(dumps_compact(&copy), r#"{"b":1,"a":2}"#);
    }

    /// A large array (parallel path) with traps in every element.
    fn big(n: usize, broken_at: Option<(usize, &str)>) -> String {
        let mut s = String::from("{\"materials\":{\"texts\":[");
        for i in 0..n {
            if i > 0 {
                s.push(',');
            }
            if let Some((at, bad)) = broken_at
                && at == i
            {
                s.push_str(bad);
                continue;
            }
            s.push_str(&format!(
                r#"{{"id":"ID-{i}","content":"{{\"text\":\"line ] }} \\\" {i} é😀\"}}","n":[{i},-0,1.5e-7,{{"a":[[]],"b":"]"}}],"dup":1,"dup":{i}, "ws" : [ 1 , 2 ] }}"#
            ));
        }
        s.push_str("]},\"tail\":[1,2,3]}");
        s
    }

    #[test]
    fn parallel_parse_matches_sequential() {
        let text = big(3000, None);
        assert!(text.len() > PARALLEL_MIN_BYTES);
        let (p, q) = (parse(&text).unwrap(), parse_sequential(&text).unwrap());
        assert_eq!(dumps_compact(&p), dumps_compact(&q));
        assert_eq!(p, q);
    }

    #[test]
    fn parallel_parse_reports_the_sequential_error() {
        for bad in [
            r#"{"id":"x""#,
            r#"{"id":"\q"}"#,
            "{\"id\":\"a\u{1}\"}",
            "[1,]",
            r#"{"a" 1}"#,
            "nul",
            "]",
        ] {
            let text = big(3000, Some((1234, bad)));
            assert_eq!(
                parse(&text).unwrap_err(),
                parse_sequential(&text).unwrap_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn strings() {
        assert_eq!(
            rt(r#""a\"b\\c\/d\né😀\u0001""#),
            "\"a\\\"b\\\\c/d\\n\u{e9}\u{1f600}\\u0001\""
        );
        assert!(parse(r#""\ud800""#).is_err());
        assert!(parse("NaN").is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse("1 2").is_err());
    }
}
