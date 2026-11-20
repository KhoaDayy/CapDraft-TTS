//! Replays every case recorded from the Python original (`rust/golden/*.jsonl`,
//! written by `rust/tools/record_wave1.py`). A mismatch is a port bug: fix the
//! port, never the recording.

#![expect(clippy::unwrap_used, reason = "test harness: a malformed golden file should fail loudly")]

use capdraft_core::models::{self, CaptionRow, ToneModifyMode};
use capdraft_core::{i18n, paths, preferences, pyfmt, pyjson, srt};
use serde_json::{Value, json};
use std::path::PathBuf;

fn golden(name: &str) -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden")
        .join(format!("{name}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// Python truthiness + `str()` for the loosely typed `normalize_*` inputs.
fn py_str_or_empty(v: &Value) -> String {
    match v {
        Value::Null | Value::Bool(false) => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) if n.as_f64() == Some(0.0) => String::new(),
        other => other.to_string(),
    }
}

fn expected(case: &Value) -> Result<Value, String> {
    match (case.get("ok"), case.get("err")) {
        (Some(v), _) => Ok(v.clone()),
        (None, Some(e)) => Err(e.as_str().unwrap().to_owned()),
        _ => panic!("bad case {case}"),
    }
}

fn i(v: &Value) -> i64 {
    v.as_i64().unwrap()
}
fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

fn captions(v: &Value) -> Vec<CaptionRow> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|c| CaptionRow {
            index: i(&c["index"]),
            text_track_id: "t".into(),
            text_segment_id: "s".into(),
            text_material_id: "m".into(),
            start_us: i(&c["start_us"]),
            duration_us: i(&c["duration_us"]),
            text: c["text"].as_str().unwrap().into(),
            existing_tts_segment_ids: vec![],
        })
        .collect()
}

fn run(case: &Value) -> Result<Value, String> {
    let a = case["args"].as_array().unwrap();
    let got = match case["fn"].as_str().unwrap() {
        "format_duration_us" => json!(models::format_duration_us(i(&a[0]))),
        "seconds_to_us" => json!(models::seconds_to_us(f(&a[0]))),
        "us_to_seconds" => json!(models::us_to_seconds(i(&a[0]))),
        "compute_target_duration_us" => {
            json!(models::compute_target_duration_us(
                i(&a[0]),
                f(&a[1]),
                f(&a[2])
            ))
        }
        "map_tone_mode_to_capcut_flag" => {
            json!(ToneModifyMode::parse(a[0].as_str().unwrap())?.capcut_flag())
        }
        "render_captions_srt" => json!(srt::render_captions_srt(&captions(&a[0]))),
        "count_srt_captions" => json!(srt::count_srt_captions(&captions(&a[0]))),
        "dumps_compact" => json!(pyjson::dumps_compact(&pyjson::loads(
            a[0].as_str().unwrap()
        )?)),
        "dumps_indent2" => json!(pyjson::dumps_indent(
            &pyjson::loads(a[0].as_str().unwrap())?,
            2
        )),
        "dumps_indent4" => json!(pyjson::dumps_indent(
            &pyjson::loads(a[0].as_str().unwrap())?,
            4
        )),
        "normalize_language" => json!(i18n::normalize_language(&py_str_or_empty(&a[0]))),
        "normalize_theme_mode" => json!(preferences::normalize_theme_mode(&py_str_or_empty(&a[0]))),
        "translate" => {
            let values: Vec<(String, String)> = a
                .get(2)
                .and_then(Value::as_object)
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.to_string())).collect())
                .unwrap_or_default();
            let refs: Vec<(&str, &str)> = values
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            json!(i18n::translate(
                a[0].as_str().unwrap(),
                a[1].as_str(),
                &refs
            ))
        }
        "find_draftpath_placeholder" => json!(paths::find_draftpath_placeholder(&a[0])),
        "capcut_text_reading_path" => json!(paths::capcut_text_reading_path(
            a[0].as_str().unwrap(),
            a[1].as_str().unwrap()
        )),
        other => panic!("unknown fn {other}"),
    };
    Ok(got)
}

/// Numbers compare by value so `1.0` (f64) equals a recorded `1.0`.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

fn replay(name: &str) {
    let cases = golden(name);
    let mut failures = Vec::new();
    for case in &cases {
        let want = expected(case);
        let got = run(case);
        let ok = match (&want, &got) {
            (Ok(w), Ok(g)) => same(w, g),
            (Err(w), Err(g)) => w == g,
            _ => false,
        };
        if !ok {
            failures.push(format!("{case}\n   got: {got:?}"));
        }
    }
    let shown: Vec<_> = failures.iter().take(5).collect();
    assert!(
        failures.is_empty(),
        "{name}: {}/{} cases match\n{}",
        cases.len() - failures.len(),
        cases.len(),
        shown
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    eprintln!("{name}: {0}/{0} cases match", cases.len());
}

#[test]
fn models_match_python() {
    replay("models");
}
#[test]
fn srt_matches_python() {
    replay("srt");
}
#[test]
fn pyjson_matches_python() {
    replay("pyjson");
}
#[test]
fn i18n_matches_python() {
    replay("i18n");
}
#[test]
fn paths_match_python() {
    replay("paths");
}

#[test]
fn isspace_matches_python() {
    let case = &golden("isspace")[0];
    let want: Vec<u32> = case["ok"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u32)
        .collect();
    let got: Vec<u32> = (0..0x11_0000u32)
        .filter_map(char::from_u32)
        .filter(|c| pyfmt::is_py_space(*c))
        .map(|c| c as u32)
        .collect();
    assert_eq!(got, want);
}
