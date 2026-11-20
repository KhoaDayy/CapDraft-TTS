//! Replays wave-2 cases recorded from the Python original by
//! `rust/tools/record_wave2.py` (alignment, draft reader, validator, split).
//! Drafts are compared as the bytes `json.dumps` would write.

#![expect(
    clippy::unwrap_used,
    reason = "test harness: a malformed golden file should fail loudly"
)]

use capdraft_core::alignment::{
    NativeAudioAlignmentSettings, apply_native_audio_alignment, frames_to_us, ms_to_us,
};
use capdraft_core::draft_reader::{DraftReader, ProjectInspectionResult};
use capdraft_core::models::{CapCutProjectInfo, CaptionRow};
use capdraft_core::project_split::{
    caption_edges_from_draft, choose_cut_us, slice_draft, split_project,
};
use capdraft_core::pyjson::{dumps_compact, float, int};
use capdraft_core::{ids, pyval, validator};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CLOCK: f64 = 1_700_000_000.25;

fn golden(name: &str) -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden")
        .join(format!("{name}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// `{"ok": v}` / `{"err": e}` as recorded, compared through `json.dumps` bytes.
fn outcome(r: Result<Value, String>) -> Value {
    match r {
        Ok(v) => json!({"ok": v}),
        Err(e) => json!({"err": e}),
    }
}

fn recorded(case: &Value) -> Value {
    match (case.get("ok"), case.get("err")) {
        (Some(v), _) => json!({"ok": v}),
        (None, Some(e)) => json!({"err": e}),
        _ => panic!("bad case"),
    }
}

/// Run `f` with ids `…0001, …0002, …` and a fixed clock, as the recorder did.
fn det<T>(f: impl FnOnce() -> T) -> T {
    ids::deterministic(CLOCK);
    let out = f();
    ids::reset();
    out
}

struct Report {
    name: &'static str,
    total: usize,
    failures: Vec<String>,
}

impl Report {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            total: 0,
            failures: vec![],
        }
    }
    fn check(&mut self, label: &str, want: &Value, got: &Value) {
        self.total += 1;
        let (w, g) = (dumps_compact(want), dumps_compact(got));
        if w != g {
            let at = w
                .bytes()
                .zip(g.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(w.len().min(g.len()));
            let from = at.saturating_sub(120);
            self.failures.push(format!(
                "{label}\n  want …{}\n   got …{}",
                &w[floor(&w, from)..floor(&w, (at + 120).min(w.len()))],
                &g[floor(&g, from)..floor(&g, (at + 120).min(g.len()))]
            ));
        }
    }
    fn finish(self) {
        let shown: Vec<_> = self.failures.iter().take(4).cloned().collect();
        assert!(
            self.failures.is_empty(),
            "{}: {}/{} match\n{}",
            self.name,
            self.total - self.failures.len(),
            self.total,
            shown.join("\n")
        );
        eprintln!("{}: {1}/{1} match", self.name, self.total);
    }
}

fn floor(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn opt_f64(v: &Value) -> Option<f64> {
    if pyval::truthy(v) { v.as_f64() } else { None }
}

// ------------------------------------------------------------------ alignment

#[test]
fn alignment_matches_python() {
    let mut r = Report::new("alignment");
    for case in golden("alignment") {
        let a = case["args"].as_array().unwrap();
        let got = match case["fn"].as_str().unwrap() {
            "frames_to_us" => outcome(Ok(json!(frames_to_us(
                a[0].as_f64().unwrap(),
                opt_f64(&a[1])
            )))),
            "ms_to_us" => outcome(Ok(json!(ms_to_us(a[0].as_f64().unwrap())))),
            "apply_native_audio_alignment" => {
                let x = &a[0];
                let mut seg = x["segment"].as_object().unwrap().clone();
                let mut mat = x["material"].as_object().unwrap().clone();
                let mut fades = x["fades"].as_array().unwrap().clone();
                let st = &x["settings"];
                let settings = NativeAudioAlignmentSettings {
                    enabled: st["enabled"].as_bool().unwrap(),
                    leading_trim_frames: st["leading_trim_frames"].as_f64().unwrap(),
                    fade_in_ms: st["fade_in_ms"].as_f64().unwrap(),
                    minimum_source_duration_us: st["minimum_source_duration_us"].as_i64().unwrap(),
                };
                let res = det(|| {
                    apply_native_audio_alignment(
                        &mut seg,
                        &mut mat,
                        &mut fades,
                        x["caption_start_us"].as_i64().unwrap(),
                        x["raw_duration_us"].as_i64().unwrap(),
                        opt_f64(&x["project_fps"]),
                        opt_f64(&x["clip_speed"]),
                        &settings,
                    )
                });
                outcome(Ok(json!({
                    "segment": seg, "material": mat, "fades": fades,
                    "result": {
                        "project_fps": float(res.project_fps),
                        "requested_trim_frames": float(res.requested_trim_frames),
                        "applied_trim_us": res.applied_trim_us,
                        "raw_duration_us": res.raw_duration_us,
                        "source_duration_us": res.source_duration_us,
                        "target_duration_us": res.target_duration_us,
                        "fade_in_us": res.fade_in_us,
                        "trim_reduced": res.trim_reduced,
                    }
                })))
            }
            other => panic!("{other}"),
        };
        r.check(&format!("{} {}", case["fn"], a[0]), &recorded(&case), &got);
    }
    r.finish();
}

// ------------------------------------------------------------------ reader

fn caption_json(c: &CaptionRow) -> Value {
    json!({
        "index": c.index, "text_track_id": c.text_track_id, "text_segment_id": c.text_segment_id,
        "text_material_id": c.text_material_id, "start_us": c.start_us, "duration_us": c.duration_us,
        "text": c.text, "existing_tts_segment_ids": c.existing_tts_segment_ids,
    })
}

fn info_json(i: &CapCutProjectInfo) -> Value {
    json!({
        "project_id": i.project_id, "project_name": i.project_name,
        "draft_path": i.draft_path.to_string_lossy(), "project_directory": i.project_directory.to_string_lossy(),
        "version": i.version, "new_version": i.new_version, "width": i.width, "height": i.height,
        "fps": float(i.fps), "duration_us": i.duration_us,
        "video_track_count": i.video_track_count, "audio_track_count": i.audio_track_count,
        "text_track_count": i.text_track_count, "video_segment_count": i.video_segment_count,
        "audio_segment_count": i.audio_segment_count, "text_segment_count": i.text_segment_count,
        "caption_count": i.caption_count, "caption_with_tts_count": i.caption_with_tts_count,
        "empty_caption_count": i.empty_caption_count,
        "duration_display": i.duration_display(), "resolution": i.resolution(),
    })
}

fn inspection_json(r: &ProjectInspectionResult) -> Value {
    json!({
        "project_info": info_json(&r.project_info),
        "valid_caption_count": r.valid_caption_count, "selected_caption_count": r.selected_caption_count,
        "skipped_empty_count": r.skipped_empty_count, "existing_tts_count": r.existing_tts_count,
        "orphan_text_segment_count": r.orphan_text_segment_count,
        "invalid_content_count": r.invalid_content_count, "warnings": r.warnings,
        "has_tts_template": r.has_tts_template,
    })
}

#[test]
fn reader_matches_python() {
    let mut r = Report::new("reader");
    for case in golden("reader") {
        let a = case["args"].as_array().unwrap();
        match case["fn"].as_str().unwrap() {
            "parse_caption_text" => {
                let got = capdraft_core::draft_reader::parse_caption_text(&a[0])
                    .map(|(t, ok)| json!([t, ok]));
                r.check(&format!("parse {}", a[0]), &recorded(&case), &outcome(got));
            }
            "builtin_tts_templates" => {
                r.check(
                    "builtin",
                    &recorded(&case),
                    &outcome(Ok(DraftReader::builtin_tts_templates())),
                );
            }
            "reader" => {
                let (d, dp) = (&a[0], PathBuf::from(a[1].as_str().unwrap()));
                let fresh = || DraftReader::from_draft(d.clone(), dp.clone());
                let want = &case["ok"];
                let label = |f: &str| {
                    format!(
                        "{f} {}",
                        dumps_compact(d).chars().take(200).collect::<String>()
                    )
                };
                let got = det(|| {
                    fresh()
                        .extract_captions()
                        .map(|cs| Value::Array(cs.iter().map(caption_json).collect()))
                });
                r.check(
                    &label("extract_captions"),
                    &want["extract_captions"],
                    &outcome(got),
                );
                let got = det(|| fresh().get_project_info().map(|i| info_json(&i)));
                r.check(&label("project_info"), &want["project_info"], &outcome(got));
                let got = det(|| {
                    fresh()
                        .find_tts_templates()
                        .map(|t| t.unwrap_or(Value::Null))
                });
                r.check(
                    &label("find_tts_templates"),
                    &want["find_tts_templates"],
                    &outcome(got),
                );
                let got = det(|| {
                    fresh()
                        .find_speed_templates()
                        .map(|t| t.unwrap_or(Value::Null))
                });
                r.check(
                    &label("find_speed_templates"),
                    &want["find_speed_templates"],
                    &outcome(got),
                );
                let got = det(|| {
                    let mut rd = fresh();
                    let sel: Option<Vec<String>> = match &a[2] {
                        Value::Null => None,
                        Value::Array(s) if !s.is_empty() && s != &vec![json!("x")] => {
                            let caps = rd.get_captions()?;
                            let mut ids: Vec<String> = caps[..caps.len() / 2]
                                .iter()
                                .map(|c| c.text_segment_id.clone())
                                .collect();
                            ids.push("3".into());
                            Some(ids)
                        }
                        Value::Array(s) => {
                            Some(s.iter().map(|x| x.as_str().unwrap().to_owned()).collect())
                        }
                        _ => unreachable!(),
                    };
                    rd.inspect_project(sel.as_deref())
                        .map(|i| inspection_json(&i))
                });
                r.check(
                    &label("inspect_project"),
                    &want["inspect_project"],
                    &outcome(got),
                );
            }
            other => panic!("{other}"),
        }
    }
    r.finish();
}

// ------------------------------------------------------------------ fixtures

struct Fixture {
    base: PathBuf,
    base_str: String,
}

impl Fixture {
    fn new(spec: &Value) -> Self {
        let raw = std::env::temp_dir().join(format!("capdraft-golden-{}", uuid_like()));
        std::fs::create_dir_all(&raw).unwrap();
        let base = std::fs::canonicalize(&raw).unwrap();
        let base_str = base.to_string_lossy().into_owned();
        if let Some(files) = spec.get("files").and_then(Value::as_object) {
            for (rel, content) in files {
                let p = base.join(rel);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, content.as_str().unwrap().replace("<ROOT>", &base_str)).unwrap();
            }
        }
        Self { base, base_str }
    }
    fn norm(&self, v: &Value) -> Value {
        serde_json::from_str(&dumps_compact(v).replace(&self.base_str, "<ROOT>")).unwrap()
    }
    fn snapshot(&self) -> Value {
        let mut out = BTreeMap::new();
        walk(&self.base, &self.base, &mut out, &self.base_str);
        Value::Object(
            out.into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect::<Map<_, _>>(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>, base_str: &str) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(base, &p, out, base_str);
        } else {
            let data = std::fs::read(&p).unwrap();
            let text = match String::from_utf8(data.clone()) {
                Ok(t) => t,
                Err(_) => format!(
                    "hex:{}",
                    data.iter().map(|b| format!("{b:02x}")).collect::<String>()
                ),
            };
            let rel = p
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, text.replace(base_str, "<ROOT>"));
        }
    }
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

#[test]
fn reader_load_matches_python() {
    let mut r = Report::new("reader_load");
    for case in golden("reader_load") {
        let a = case["args"].as_array().unwrap();
        let fx = Fixture::new(&a[0]);
        let mut rd = DraftReader::new();
        let got = rd
            .load_draft(&fx.base.join(a[1].as_str().unwrap()))
            .map(|_| ())
            .and_then(|()| {
                Ok(json!({
                    "draft_path": rd.draft_path.as_ref().unwrap().to_string_lossy(),
                    "project_directory": rd.project_directory.as_ref().unwrap().to_string_lossy(),
                    "captions": rd.get_captions()?.len(),
                }))
            });
        let got = match got {
            Err(e) if e.starts_with("JSONDecodeError") => Err("JSONDecodeError".to_owned()),
            other => other,
        };
        r.check(
            &format!("load {}", a[1]),
            &recorded(&case),
            &fx.norm(&outcome(got)),
        );
    }
    r.finish();
}

// ------------------------------------------------------------------ validator

#[test]
fn validator_matches_python() {
    let mut r = Report::new("validator");
    for case in golden("validator") {
        let a = case["args"].as_array().unwrap();
        let req = a[2].as_bool().unwrap();
        let got = if a[1].is_null() {
            outcome(validator::validate(&a[0], None, req).map(|e| json!(e)))
        } else {
            let fx = Fixture::new(&a[1]);
            fx.norm(&outcome(
                validator::validate(&a[0], Some(&fx.base), req).map(|e| json!(e)),
            ))
        };
        r.check(
            &format!(
                "validate {}",
                dumps_compact(&a[0]).chars().take(300).collect::<String>()
            ),
            &recorded(&case),
            &got,
        );
    }
    r.finish();
}

// ------------------------------------------------------------------ split

#[test]
fn split_matches_python() {
    let mut r = Report::new("split");
    for case in golden("split") {
        let a = case["args"].as_array().unwrap();
        match case["fn"].as_str().unwrap() {
            "choose_cut_us" => {
                let got = (|| {
                    let dur = pyval::int_or0(&a[0])?;
                    let mut edges = Vec::new();
                    for x in pyval::iter_or_empty(&a[1])?.iter() {
                        edges.push(pyval::to_int(x)?);
                    }
                    choose_cut_us(dur, &edges).map(|c| json!(c))
                })();
                r.check(&format!("choose {a:?}"), &recorded(&case), &outcome(got));
            }
            "split" => {
                let d = &a[0];
                let want = &case["ok"];
                let label = dumps_compact(d).chars().take(200).collect::<String>();
                let got = det(|| caption_edges_from_draft(d).map(|e| json!(e)));
                r.check(
                    &format!("edges {label}"),
                    &want["caption_edges"],
                    &outcome(got),
                );
                for s in want["slices"].as_array().unwrap() {
                    let sa = s["args"].as_array().unwrap();
                    let got = det(|| {
                        slice_draft(
                            d,
                            sa[0].as_i64().unwrap(),
                            sa[1].as_i64().unwrap(),
                            sa[2].as_str().unwrap(),
                            sa[3].as_str(),
                            sa[4].as_bool().unwrap(),
                        )
                        .map(|(out, n)| json!([out, int(n)]))
                    });
                    r.check(
                        &format!("slice {sa:?} {label}"),
                        &recorded(s),
                        &outcome(got),
                    );
                }
            }
            other => panic!("{other}"),
        }
    }
    r.finish();
}

#[test]
fn split_project_matches_python() {
    let mut r = Report::new("split_project");
    for (i, case) in golden("split_project").into_iter().enumerate() {
        let a = case["args"].as_array().unwrap();
        let fx = Fixture::new(&a[0]);
        let proj = fx.base.join("Proj");
        let draft: Value =
            serde_json::from_str(a[0]["files"]["Proj/draft_content.json"].as_str().unwrap())
                .unwrap();
        let mut events = Vec::new();
        let got = det(|| {
            split_project(&proj, &draft, &proj.join("draft_content.json"), a[1].as_i64(), None, None, &mut |p, m| {
                events.push((p, m.to_owned()));
            })
            .map(|res| {
                json!({"result": {
                    "cut_us": res.cut_us,
                    "part1_dir": res.part1_dir.to_string_lossy(), "part2_dir": res.part2_dir.to_string_lossy(),
                    "part1_draft": res.part1_draft.to_string_lossy(), "part2_draft": res.part2_draft.to_string_lossy(),
                    "part1_duration_us": res.part1_duration_us, "part2_duration_us": res.part2_duration_us,
                    "part1_segments": res.part1_segments, "part2_segments": res.part2_segments,
                }})
            })
        });
        r.check(
            &format!("split_project #{i} result"),
            &recorded(&case),
            &fx.norm(&outcome(got)),
        );
        r.check(
            &format!("split_project #{i} tree"),
            &case["tree"],
            &fx.snapshot(),
        );
    }
    r.finish();
}
