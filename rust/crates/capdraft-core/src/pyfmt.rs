//! Python semantics that Rust's standard library spells differently.

/// Python `round(x)` for a float: round half to even, returned as an integer.
///
/// Saturates outside the `i64` range (Python would return a big int; CapCut
/// timestamps never get there).
#[must_use]
pub fn round_half_even(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// Python `str.isspace()` for one character.
///
/// Rust's `char::is_whitespace` (Unicode `White_Space`) leaves out
/// U+001C..U+001F, which Python counts as whitespace (bidi class B/S).
#[must_use]
pub fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python `str.strip()` with no arguments.
#[must_use]
pub fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// Python `repr(float)` (what `json.dumps` writes for a float).
///
/// Shortest round-trip digits; fixed notation when the decimal exponent is in
/// `-4..16`, otherwise `d.ddde+XX` with at least two exponent digits.
/// Non-finite values follow `json.dumps` (`NaN`, `Infinity`, `-Infinity`).
#[must_use]
pub fn py_float_repr(f: f64) -> String {
    if f.is_nan() {
        return "NaN".to_owned();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.to_owned();
    }
    // Rust's `{:e}` gives the shortest round-trip digits, e.g. "-1.2345e-5".
    let sci = format!("{f:e}");
    let (neg, body) = match sci.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, sci.as_str()),
    };
    let (mantissa, exp) = body.split_once('e').unwrap_or((body, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();

    let mut out = String::with_capacity(digits.len() + 8);
    if neg {
        out.push('-');
    }
    if (-4..16).contains(&exp) {
        if exp >= 0 {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                out.push_str(&digits);
                out.extend(std::iter::repeat_n('0', int_len - digits.len()));
                out.push_str(".0");
            } else {
                out.push_str(&digits[..int_len]);
                out.push('.');
                out.push_str(&digits[int_len..]);
            }
        } else {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-exp - 1) as usize));
            out.push_str(&digits);
        }
    } else {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exp.unsigned_abs()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_repr_matches_python() {
        let cases = [
            (1.0, "1.0"),
            (0.1, "0.1"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (1.5e-7, "1.5e-07"),
            (-2.5, "-2.5"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (123456789.123, "123456789.123"),
        ];
        for (f, want) in cases {
            assert_eq!(py_float_repr(f), want, "{f}");
        }
    }

    #[test]
    fn round_is_bankers() {
        assert_eq!(round_half_even(0.5), 0);
        assert_eq!(round_half_even(1.5), 2);
        assert_eq!(round_half_even(2.5), 2);
        assert_eq!(round_half_even(-1.5), -2);
    }

    #[test]
    fn strip_includes_info_separators() {
        assert_eq!(py_strip("\u{1f}x\u{1c}\u{a0}"), "x");
        assert_eq!(py_strip("\u{200b}x"), "\u{200b}x");
    }
}
