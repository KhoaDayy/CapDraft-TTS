//! Replays wave-3 cases recorded from the Python original by
//! `rust/tools/record_wave3.py` (draft patcher, exporter, config, cache, voice catalog).

#![expect(
    clippy::unwrap_used,
    reason = "test harness: a malformed golden file should fail loudly"
)]

use capdraft_core::alignment::NativeAudioAlignmentSettings;
use capdraft_core::cache::TtsCache;
use capdraft_core::config::AppConfig;
use capdraft_core::draft_patcher::{DraftPatcher, PatchSettings, PatchStats};
use capdraft_core::json::{self as cj, Object, Value, dumps_compact};
use capdraft_core::jv as json;
use capdraft_core::models::{CaptionRow, CaptionTtsResult, ToneModifyMode};
use capdraft_core::project_exporter::{PathStyle, ProjectExporter};
use capdraft_core::voice_catalog::VoiceCatalog;
use capdraft_core::{ids, pyval};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

const CLOCK: f64 = 1_700_000_000.25;

fn golden(name: &str) -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden")
        .join(format!("{name}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines().map(|l| cj::parse(l).unwrap()).collect()
}

fn outcome(r: Result<Value, String>) -> Value {
    let mut o = Object::new();
    match r {
        Ok(v) => o.insert("ok", v),
        Err(e) => o.insert("err", Value::from(e)),
    }
    Value::Object(o)
}

fn recorded(case: &Value) -> Value {
    let mut o = Object::new();
    match (case.get("ok"), case.get("err")) {
        (Some(v), _) => o.insert("ok", v.clone()),
        (None, Some(e)) => o.insert("err", e.clone()),
        _ => panic!("bad case"),
    }
    Value::Object(o)
}

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
            let cut = |s: &str| {
                let (mut a, mut b) = (at.saturating_sub(150), (at + 150).min(s.len()));
                while !s.is_char_boundary(a) {
                    a -= 1;
                }
                while !s.is_char_boundary(b) {
                    b -= 1;
                }
                s[a..b].to_owned()
            };
            self.failures
                .push(format!("{label}\n  want …{}\n   got …{}", cut(&w), cut(&g)));
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

// ------------------------------------------------------------------ fixtures

struct Fixture {
    base: PathBuf,
    base_str: String,
}

impl Fixture {
    fn new(spec: &Value) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let raw = std::env::temp_dir().join(format!(
            "capdraft-w3-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&raw).unwrap();
        let base = std::fs::canonicalize(&raw).unwrap();
        let base_str = base.to_string_lossy().into_owned();
        if let Some(files) = spec.get("files").and_then(Value::as_object) {
            for (rel, content) in files {
                let p = base.join(rel.as_str());
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(&p, content.as_str().unwrap().replace("<ROOT>", &base_str)).unwrap();
            }
        }
        if let Some(dirs) = spec.get("dirs").and_then(Value::as_array) {
            for d in dirs.iter() {
                std::fs::create_dir_all(base.join(d.as_str().unwrap())).unwrap();
            }
        }
        Self { base, base_str }
    }
    fn norm(&self, v: &Value) -> Value {
        cj::parse(&dumps_compact(v).replace(&self.base_str, "<ROOT>")).unwrap()
    }
    fn snapshot(&self) -> Value {
        let mut out = BTreeMap::new();
        walk(&self.base, &self.base, &mut out, &self.base_str);
        Value::Object(
            out.into_iter()
                .map(|(k, v)| (k, Value::from(v)))
                .collect::<Object>(),
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
        let rel = p
            .strip_prefix(base)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if p.is_dir() {
            if std::fs::read_dir(&p).unwrap().next().is_none() {
                out.insert(format!("{rel}/"), "<dir>".into());
            }
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
            out.insert(rel, text.replace(base_str, "<ROOT>"));
        }
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap().to_owned()
}
fn i(v: &Value) -> i64 {
    v.as_i64().unwrap()
}

// ------------------------------------------------------------------ patcher

fn caption_from(v: &Value) -> CaptionRow {
    CaptionRow {
        index: i(&v["index"]),
        text_track_id: s(&v["text_track_id"]),
        text_segment_id: s(&v["text_segment_id"]),
        text_material_id: s(&v["text_material_id"]),
        start_us: i(&v["start_us"]),
        duration_us: i(&v["duration_us"]),
        text: s(&v["text"]),
        existing_tts_segment_ids: v["existing_tts_segment_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(s)
            .collect(),
    }
}

fn caption_json(c: &CaptionRow) -> Value {
    json!({
        "index": c.index, "text_track_id": c.text_track_id, "text_segment_id": c.text_segment_id,
        "text_material_id": c.text_material_id, "start_us": c.start_us, "duration_us": c.duration_us,
        "text": c.text, "existing_tts_segment_ids": c.existing_tts_segment_ids,
    })
}

fn result_from(v: &Value) -> CaptionTtsResult {
    CaptionTtsResult {
        caption_index: i(&v["caption_index"]),
        text_material_id: s(&v["text_material_id"]),
        text_segment_id: s(&v["text_segment_id"]),
        start_us: i(&v["start_us"]),
        audio_path: s(&v["audio_path"]),
        source_duration_us: i(&v["source_duration_us"]),
        target_duration_us: i(&v["target_duration_us"]),
        status: s(&v["status"]),
        error: s(&v["error"]),
        from_cache: v["from_cache"].as_bool().unwrap(),
    }
}

fn result_json(r: &CaptionTtsResult) -> Value {
    json!({
        "caption_index": r.caption_index, "text_material_id": r.text_material_id, "text_segment_id": r.text_segment_id,
        "start_us": r.start_us, "audio_path": r.audio_path, "source_duration_us": r.source_duration_us,
        "target_duration_us": r.target_duration_us, "status": r.status, "error": r.error, "from_cache": r.from_cache,
    })
}

fn stats_json(st: &PatchStats) -> Value {
    json!({
        "materials_created": st.materials_created, "segments_created": st.segments_created,
        "speeds_created": st.speeds_created, "fades_created": st.fades_created,
        "alignment_applied": st.alignment_applied, "alignment_trim_reduced": st.alignment_trim_reduced,
        "replaced": st.replaced, "skipped": st.skipped, "tracks_used": st.tracks_used,
        "tracks_created": st.tracks_created, "orphans_removed": st.orphans_removed,
        "created_ids": st.created_ids, "removed_ids": st.removed_ids,
    })
}

#[test]
fn patcher_matches_python() {
    let mut r = Report::new("patcher");
    for (n, case) in golden("patcher").into_iter().enumerate() {
        let a = &case["args"][0];
        let st = &a["settings"];
        let al = &st["alignment"];
        let alignment = NativeAudioAlignmentSettings {
            enabled: al["enabled"].as_bool().unwrap(),
            leading_trim_frames: al["leading_trim_frames"].as_f64().unwrap(),
            fade_in_ms: al["fade_in_ms"].as_f64().unwrap(),
            minimum_source_duration_us: i(&al["minimum_source_duration_us"]),
        };
        let mut draft = a["draft"].clone();
        let mut captions: Vec<CaptionRow> = a["captions"]
            .as_array()
            .unwrap()
            .iter()
            .map(caption_from)
            .collect();
        let mut results: Vec<CaptionTtsResult> = a["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(result_from)
            .collect();
        let rel: HashMap<i64, String> = a["audio_rel_paths"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.parse().unwrap(), s(v)))
            .collect();
        let settings = PatchSettings {
            voice_type: st["voice_type"].as_str().unwrap(),
            resource_id: st["resource_id"].as_str().unwrap(),
            voice_display_name: st["voice_display_name"].as_str().unwrap(),
            capcut_clip_speed: st["capcut_clip_speed"].as_f64().unwrap(),
            tone_modify_mode: ToneModifyMode::parse(st["tone_modify_mode"].as_str().unwrap())
                .unwrap(),
            existing_tts_mode: st["existing_tts_mode"].as_str().unwrap(),
        };
        let got = det(|| {
            let patcher =
                DraftPatcher::new(&a["templates"], st["fps"].as_f64().unwrap(), alignment);
            patcher
                .patch(&mut draft, &mut captions, &mut results, &settings, &rel)
                .map(|stats| {
                    json!({
                        "draft": draft.clone(), "stats": stats_json(&stats),
                        "results": results.iter().map(result_json).collect::<Vec<_>>(),
                        "captions": captions.iter().map(caption_json).collect::<Vec<_>>(),
                    })
                })
        });
        r.check(
            &format!("patch #{n} mode={}", settings.existing_tts_mode),
            &recorded(&case),
            &outcome(got),
        );
    }
    r.finish();
}

// ------------------------------------------------------------------ exporter

/// `"Type: message"` -> `"Type"` (OS error texts differ between languages).
fn err_type(r: Result<Value, String>) -> Value {
    outcome(r.map_err(|e| e.split(':').next().unwrap_or("").to_owned()))
}

#[test]
fn exporter_matches_python() {
    let mut r = Report::new("exporter");
    for (n, case) in golden("exporter").into_iter().enumerate() {
        let a = case["args"].as_array().unwrap();
        let fx = Fixture::new(&a[0]);
        let proj = fx.base.join("Proj");
        let draft_path = fx.base.join(a[1].as_str().unwrap());
        let ex = ProjectExporter::new(a[2].as_i64().unwrap() as usize);
        let style = PathStyle::parse(a[3].as_str().unwrap());
        let placeholder = a[4].as_str();
        let workers = a[5].as_i64().unwrap() as usize;
        let draft_b = json!({"id": "PID", "tracks": [{"id": "t", "type": "audio", "segments": []}],
                             "materials": {"texts": [], "f": 1.5}, "name": "B — Tiếng Việt"});
        let mut steps: Vec<Value> = Vec::new();
        let mut push = |name: &str, res: Value| {
            let mut o = Object::new();
            o.insert("step", Value::from(name));
            for (k, v) in res.as_object().unwrap() {
                o.insert(k.clone(), v.clone());
            }
            steps.push(fx.norm(&Value::Object(o)));
        };
        det(|| {
            let backup = ex.create_backup(&draft_path, &proj, &json!({"m": 1, "é": [1.5]}), "PID");
            let backup_ok = backup.as_ref().ok().cloned();
            push(
                "create_backup",
                err_type(backup.map(|p| Value::from(p.to_string_lossy().into_owned()))),
            );
            let items = vec![
                (
                    fx.base.join("audio/src1.mp3"),
                    "tts_000001_aaaa.mp3".to_owned(),
                ),
                (
                    fx.base.join("audio/missing.mp3"),
                    "tts_000002_bbbb.mp3".to_owned(),
                ),
                (
                    fx.base.join("audio/src2.mp3"),
                    "tts_000003_cccc.mp3".to_owned(),
                ),
            ];
            let batch =
                ex.copy_audio_batch(&items, &proj, Some(&draft_b), placeholder, style, workers);
            let batch: Vec<Value> = batch
                .into_iter()
                .map(|(rel, err)| Value::from(vec![Value::from(rel), Value::from(err)]))
                .collect();
            push("copy_audio_batch", outcome(Ok(Value::from(batch))));
            let written = ex.atomic_write_draft(&draft_path, &draft_b, Some(&proj), "PID");
            push(
                "atomic_write_draft",
                err_type(written.map(|w| {
                    Value::from(
                        w.iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect::<Vec<_>>(),
                    )
                })),
            );
            let m = ex.write_manifest(&proj, &json!({"a": [1, {"b": null}], "t": "ố"}));
            push(
                "write_manifest",
                err_type(m.map(|p| Value::from(p.to_string_lossy().into_owned()))),
            );
            let lines_case = &case["ok"].as_array().unwrap();
            // The recorder chose the log lines at random; recover them from the tree.
            let log = case["tree"]
                .as_object()
                .unwrap()
                .iter()
                .find(|(k, _)| k.starts_with("Proj/logs/"))
                .map(|(_, v)| s(v))
                .unwrap_or_default();
            let lines: Vec<String> = if log.is_empty() {
                vec![]
            } else {
                log.trim_end_matches('\n')
                    .split('\n')
                    .map(str::to_owned)
                    .collect()
            };
            let _ = lines_case;
            let l = ex.write_log_file(&proj, &lines);
            push(
                "write_log_file",
                err_type(l.map(|p| Value::from(p.to_string_lossy().into_owned()))),
            );
            if let Some(b) = backup_ok.filter(|_| n % 3 == 1) {
                ex.rollback(
                    &draft_path,
                    Some(&b),
                    &[proj.join("textReading/tts_000001_aaaa.mp3")],
                    Some(&proj),
                    "PID",
                );
                push("rollback", outcome(Ok(Value::Null)));
            }
        });
        r.check(
            &format!("exporter #{n} steps"),
            &case["ok"],
            &Value::from(steps),
        );
        r.check(
            &format!("exporter #{n} tree"),
            &case["tree"],
            &fx.snapshot(),
        );
    }
    r.finish();
}

// ------------------------------------------------------------------ config

#[test]
fn config_matches_python() {
    let mut r = Report::new("config");
    for (n, case) in golden("config").into_iter().enumerate() {
        let text = &case["args"][0];
        let spec = match text {
            Value::String(t) => json!({"files": {"config.json": t.as_str()}}),
            _ => json!({}),
        };
        let fx = Fixture::new(&spec);
        let mut c = AppConfig::load(&fx.base);
        let p = |x: PathBuf| Value::from(x.to_string_lossy().into_owned());
        let got = json!({
            "data": Value::Object(c.data().clone()),
            "cache_path": p(c.cache_dir()),
            "projects_dir": p(c.projects_dir()),
            "capcut_projects_path": c.capcut_projects_path().map(p),
            "ffmpeg_path": c.ffmpeg_path(),
            "default_rate": c.default_rate(),
            "voice_catalog_url": c.voice_catalog_url(),
            "language": c.language(),
            "theme_mode": c.theme_mode(),
            "device_json_path": p(c.device_json_path()),
            "capcut_tts_path": p(c.capcut_tts_path()),
        });
        r.check(
            &format!("config #{n} {text}"),
            &recorded(&case),
            &fx.norm(&outcome(Ok(got))),
        );
        c.set("language", Value::from("ja"));
        r.check(&format!("config #{n} tree"), &case["tree"], &fx.snapshot());
    }
    r.finish();
}

// ------------------------------------------------------------------ cache

#[test]
fn cache_matches_python() {
    let mut r = Report::new("cache");
    for case in golden("cache") {
        let a = case["args"].as_array().unwrap();
        match case["fn"].as_str().unwrap() {
            "generate_hash" => {
                let got = TtsCache::generate_hash(
                    &s(&a[0]),
                    &s(&a[1]),
                    a[2].as_f64().unwrap(),
                    &s(&a[3]),
                );
                r.check(
                    &format!("hash rate={}", a[2]),
                    &recorded(&case),
                    &outcome(Ok(Value::from(got))),
                );
            }
            "cache" => {
                let k = a[0].as_i64().unwrap() as usize;
                let fx = Fixture::new(&json!({}));
                let b = &fx.base;
                std::fs::write(b.join("src.mp3"), b"ID3".repeat(k + 1)).unwrap();
                std::fs::write(b.join("empty.mp3"), b"").unwrap();
                let c = TtsCache::new(&b.join("cache")).unwrap();
                let pair = |(p, d): (Option<PathBuf>, Option<f64>)| {
                    Value::from(vec![
                        Value::from(p.map(|p| p.to_string_lossy().into_owned())),
                        Value::from(d),
                    ])
                };
                let dur = [0.0, 1.5, 2.0, 1e-7, 3.25, 100.0][k];
                let mut steps = vec![outcome(Ok(pair(c.get_cached_file("t", "v", 1.0, "r"))))];
                steps.push(outcome(
                    c.cache_audio(&b.join("src.mp3"), "t", "v", 1.0, "r", dur)
                        .map(|p| Value::from(p.to_string_lossy().into_owned())),
                ));
                steps.push(outcome(Ok(pair(c.get_cached_file("t", "v", 1.0, "r")))));
                steps.push(err_type(
                    c.cache_audio(&b.join("empty.mp3"), "t2", "v", 1.0, "r", 0.0)
                        .map(|p| Value::from(p.to_string_lossy().into_owned())),
                ));
                let h = TtsCache::generate_hash("t3", "v", 1.0, "r");
                std::fs::write(
                    c.cache_dir.join(format!("{h}.mp3")),
                    if k % 2 == 1 { &b""[..] } else { &b"X"[..] },
                )
                .unwrap();
                let meta = [
                    "{bad",
                    "{\"duration\": \"2\"}",
                    "[]",
                    "{\"x\": 1}",
                    "",
                    "{\"duration\": 4}",
                ][k];
                std::fs::write(c.cache_dir.join(format!("{h}.meta.json")), meta).unwrap();
                steps.push(outcome(Ok(pair(c.get_cached_file("t3", "v", 1.0, "r")))));
                let steps: Vec<Value> = steps.iter().map(|v| fx.norm(v)).collect();
                r.check(&format!("cache #{k}"), &case["ok"], &Value::from(steps));
                r.check(&format!("cache #{k} tree"), &case["tree"], &fx.snapshot());
            }
            other => panic!("{other}"),
        }
    }
    r.finish();
}

// ------------------------------------------------------------------ voice catalog

#[test]
fn voice_catalog_matches_python() {
    let mut r = Report::new("voice_catalog");
    for (n, case) in golden("voice_catalog").into_iter().enumerate() {
        let raw = &case["args"][0];
        let mut cat = VoiceCatalog::new(Some("u"));
        let got = cat.load_data(raw, "test").map(|voices| {
            let v = |o: &capdraft_core::models::VoiceOption| {
                json!({"language_code": o.language_code, "locale": o.locale, "voice_type": o.voice_type,
                       "display_name": o.display_name, "resource_id": o.resource_id})
            };
            let mut filters = Vec::new();
            for lc in [None, Some(""), Some("vi"), Some("en"), Some("en-US"), Some("zh"), Some("x")] {
                for loc in [None, Some("vi-VN"), Some("en-US")] {
                    for q in [None, Some(""), Some("  NHỎ "), Some("bv"), Some("7252"), Some("zzz")] {
                        let hits: Vec<String> = cat.filter(lc, loc, q).iter().map(|o| o.voice_type.clone()).collect();
                        filters.push(json!([lc, loc, q, hits]));
                    }
                }
            }
            let finds: Vec<Value> = [("BV421_vivn_streaming", None), ("BV421_vivn_streaming", Some("7252594014782755330")),
                ("BV421_vivn_streaming", Some("x")), ("nope", None), ("b", Some("7"))]
                .iter()
                .map(|(t, id)| cat.find(t, *id).map_or(Value::Null, v))
                .collect();
            json!({
                "voices": voices.iter().map(v).collect::<Vec<_>>(),
                "languages": cat.languages().into_iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
                "filters": filters, "find": finds,
            })
        });
        r.check(&format!("catalog #{n}"), &recorded(&case), &outcome(got));
    }
    let _ = pyval::truthy(&Value::Null);
    r.finish();
}
