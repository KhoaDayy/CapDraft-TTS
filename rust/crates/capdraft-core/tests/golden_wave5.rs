//! Replays wave-5 cases recorded by `rust/tools/record_wave5.py`: the whole
//! generate-and-attach pipeline against the fake CapCut server (results, log
//! events, progress, callbacks, requests, every file written) and ffmpeg merges.

#![expect(
    clippy::unwrap_used,
    reason = "test harness: a malformed golden file should fail loudly"
)]

use capdraft_core::alignment::NativeAudioAlignmentSettings;
use capdraft_core::cache::TtsCache;
use capdraft_core::config::AppConfig;
use capdraft_core::ids;
use capdraft_core::json::{self as cj, Object, Value, dumps_compact};
use capdraft_core::jv as json;
use capdraft_core::models::{CaptionTtsResult, GenerationLogEvent, ToneModifyMode};
use capdraft_core::service::{GenerateRequest, ProjectTtsService};
use capdraft_core::tts::TtsItemResult;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

mod common;
use common::FakeCapCut;

const CLOCK: f64 = 1_700_000_000.25;

fn golden(name: &str) -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden")
        .join(format!("{name}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines().map(|l| cj::parse(l).unwrap()).collect()
}

fn s(v: &Value) -> String {
    v.as_str().unwrap().to_owned()
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
                let (mut a, mut b) = (at.saturating_sub(200), (at + 200).min(s.len()));
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

fn snapshot(base: &Path) -> Value {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>, root: &str) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out, root);
            } else if p.file_name().is_some_and(|n| n != "config.json") {
                let data = std::fs::read(&p).unwrap();
                let text = String::from_utf8(data.clone()).unwrap_or_else(|_| {
                    format!(
                        "hex:{}",
                        data.iter().map(|b| format!("{b:02x}")).collect::<String>()
                    )
                });
                out.insert(
                    p.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    text.replace(root, "<ROOT>"),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(base, base, &mut out, &base.to_string_lossy());
    Value::Object(
        out.into_iter()
            .map(|(k, v)| (k, Value::from(v)))
            .collect::<Object>(),
    )
}

fn tts_json(r: &TtsItemResult) -> Value {
    let mut o = Object::new();
    o.insert("audio_path", Value::from(r.audio_path.as_str()));
    o.insert("duration", Value::from(r.duration));
    o.insert("status", Value::from(r.status));
    if let Some(e) = &r.error {
        o.insert("error", Value::from(e.as_str()));
    }
    Value::Object(o)
}

fn item_json(r: &CaptionTtsResult) -> Value {
    json!({
        "caption_index": r.caption_index, "text_material_id": r.text_material_id, "text_segment_id": r.text_segment_id,
        "start_us": r.start_us, "audio_path": r.audio_path, "source_duration_us": r.source_duration_us,
        "target_duration_us": r.target_duration_us, "status": r.status, "error": r.error, "from_cache": r.from_cache,
    })
}

fn opt_path(p: Option<&PathBuf>) -> Value {
    p.map_or(Value::Null, |p| {
        Value::from(p.to_string_lossy().into_owned())
    })
}

#[test]
fn pipeline_matches_python() {
    let mut r = Report::new("pipeline");
    for (n, case) in golden("pipeline").into_iter().enumerate() {
        let sc = &case["args"][0];
        let files = case["args"][1].as_object().unwrap();
        let name = s(&sc["name"]);
        let raw = std::env::temp_dir().join(format!("capdraft-w5-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&raw);
        std::fs::create_dir_all(&raw).unwrap();
        let base = std::fs::canonicalize(&raw).unwrap();
        for (rel, text) in files {
            let p = base.join(rel.as_str());
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            let mut t = s(text);
            if rel.as_str() == "config.json" {
                // the recorder's absolute repo paths -> this checkout's
                let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
                let repo = std::fs::canonicalize(repo).unwrap();
                let mut cfg = cj::parse(&t).unwrap();
                let o = cfg.as_object_mut().unwrap();
                o.insert(
                    "device_json_path",
                    Value::from(
                        repo.join("external/capcut-tts-api/device.json")
                            .to_string_lossy()
                            .into_owned(),
                    ),
                );
                t = dumps_compact(&cfg);
            }
            std::fs::write(&p, t).unwrap();
        }
        let config = AppConfig::load(&base);
        let rate = sc["rate"].as_f64().unwrap_or(1.0);
        for t in sc["pre_cache"]
            .as_array()
            .map(|a| a.to_vec())
            .unwrap_or_default()
        {
            let src = base.join("pre.mp3");
            std::fs::write(&src, format!("CACHED:{}", s(&t))).unwrap();
            TtsCache::new(&config.cache_dir())
                .unwrap()
                .cache_audio(
                    &src,
                    &s(&t),
                    "BV421_vivn_streaming",
                    rate,
                    "7252594014782755330",
                    1.75,
                )
                .unwrap();
            std::fs::remove_file(&src).unwrap();
        }
        let events: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let fake = Arc::new(FakeCapCut::default());
        let mut svc = ProjectTtsService::new(
            config,
            fake.clone(),
            Some(Arc::new(move |e: &GenerationLogEvent| {
                let msg = if e.message.starts_with("Processing time:") {
                    "Processing time: <t>".to_owned()
                } else {
                    e.message.clone()
                };
                ev.lock().unwrap().push(json!([
                    e.level,
                    msg,
                    e.stage,
                    e.caption_index,
                    e.progress
                ]));
            })),
        )
        .with_tts_sleep(Arc::new(|_| {}));
        ids::deterministic(CLOCK); // the recorder's clock is fixed from the start
        svc.load_project(&base.join("Proj")).unwrap();
        let selection: Vec<String> = match sc["selection"].as_array() {
            Some(a) if !a.is_empty() => a.iter().map(s).collect(),
            _ => svc
                .get_captions()
                .unwrap()
                .iter()
                .map(|c| c.text_segment_id.clone())
                .collect(),
        };
        let al = &sc["alignment"];
        let alignment = if al.is_null() {
            NativeAudioAlignmentSettings::default()
        } else {
            NativeAudioAlignmentSettings {
                enabled: al["enabled"].as_bool().unwrap(),
                leading_trim_frames: al["leading_trim_frames"].as_f64().unwrap(),
                fade_in_ms: al["fade_in_ms"].as_f64().unwrap(),
                ..Default::default()
            }
        };
        let req = GenerateRequest {
            selected_caption_ids: selection,
            voice_type: "BV421_vivn_streaming".into(),
            resource_id: "7252594014782755330".into(),
            voice_display_name: "Nhỏ Ngọt Ngào".into(),
            tts_rate: rate,
            capcut_clip_speed: sc["clip_speed"].as_f64().unwrap_or(1.0),
            tone_modify_mode: ToneModifyMode::parse(
                sc["tone"].as_str().unwrap_or("preserve_pitch"),
            )
            .unwrap(),
            existing_tts_mode: sc["mode"].as_str().unwrap_or("replace_existing").to_owned(),
            allow_partial_attach: sc["allow_partial"].as_bool().unwrap_or(true),
            use_cache: sc["use_cache"].as_bool().unwrap_or(true),
            alignment,
        };
        let progress: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let items: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let checks = Arc::new(AtomicUsize::new(0));
        let limit = sc["cancel_after"].as_i64();
        let (pg, it, ck) = (progress.clone(), items.clone(), checks.clone());
        let delete_of: Vec<i64> = sc["delete_audio_of"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();
        ids::deterministic(CLOCK);
        let res = svc.generate_and_attach(
            &req,
            Some(Arc::new(move |p, m| pg.lock().unwrap().push(json!([p, m])))),
            Some(Arc::new(move |i, res: &TtsItemResult| {
                it.lock().unwrap().push(json!([i, tts_json(res)]));
                if delete_of.contains(&i) {
                    std::fs::remove_file(&res.audio_path).unwrap();
                }
            })),
            Some(Arc::new(move || {
                let k = ck.fetch_add(1, Ordering::SeqCst) + 1;
                limit.is_some_and(|l| k as i64 > l)
            })),
        );
        ids::reset();
        let result = match res {
            Ok(g) => {
                let extra = if g.cancelled {
                    json!({"cancelled": true})
                } else {
                    json!({})
                };
                json!({"ok": {
                    "success": g.success, "project_path": g.project_path.to_string_lossy(),
                    "backup_path": opt_path(g.backup_path.as_ref()), "selected": g.selected, "generated": g.generated,
                    "cached": g.cached, "skipped": g.skipped, "failed": g.failed, "replaced": g.replaced,
                    "attached": g.attached, "audio_tracks_used": g.audio_tracks_used,
                    "audio_tracks_created": g.audio_tracks_created, "validation_passed": g.validation_passed,
                    "log_file": opt_path(g.log_file.as_ref()), "manifest_path": opt_path(g.manifest_path.as_ref()),
                    "warnings": g.warnings, "errors": g.errors,
                    "items": g.items.iter().map(item_json).collect::<Vec<_>>(), "extra": extra,
                }})
            }
            Err(e) => json!({"err": e}),
        };
        let got = json!({
            "result": result,
            "events": events.lock().unwrap().clone(),
            "progress": progress.lock().unwrap().clone(),
            "items": items.lock().unwrap().clone(),
            "cancel_checks": checks.load(Ordering::SeqCst),
            "requests": fake.0.lock().unwrap().log.clone(),
        });
        let root = base.to_string_lossy().into_owned();
        let got = cj::parse(&dumps_compact(&got).replace(&root, "<ROOT>")).unwrap();
        r.check(&name, &case["ok"], &got);
        r.check(&format!("{name} tree"), &case["tree"], &snapshot(&base));
        let _ = std::fs::remove_dir_all(&base);
    }
    r.finish();
}

#[cfg(unix)]
fn make_video(ff: &str, path: &Path, freq: u32) {
    let st = std::process::Command::new(ff)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=0.5:size=32x32:rate=10",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("sine=frequency={freq}:duration=0.5"))
        .args(["-shortest", "-c:v", "libx264", "-c:a", "aac"])
        .arg(path)
        .status()
        .unwrap();
    assert!(st.success());
}

#[cfg(unix)]
#[test]
fn video_merge_matches_python() {
    use std::os::unix::fs::PermissionsExt;
    let Some(ff) = capdraft_core::video_merge::which("ffmpeg") else {
        eprintln!("video_merge: skipped (no ffmpeg)");
        return;
    };
    let ff = ff.to_string_lossy().into_owned();
    let mut r = Report::new("video_merge");
    for (n, case) in golden("video_merge").into_iter().enumerate() {
        let a = case["args"].as_array().unwrap();
        let raw = std::env::temp_dir().join(format!("capdraft-w5m-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&raw);
        std::fs::create_dir_all(&raw).unwrap();
        let base = std::fs::canonicalize(&raw).unwrap();
        for (name, freq) in [("a.mp4", 440), ("b.mp4", 660), ("it's a.mp4", 550)] {
            make_video(&ff, &base.join(name), freq);
        }
        let tool = base.join("tools/ffmpeg");
        std::fs::create_dir_all(tool.parent().unwrap()).unwrap();
        // the recorder's wrapper, pointed at this machine's ffmpeg
        let script = s(&a[4]);
        let script = script
            .lines()
            .map(|l| {
                if l.starts_with("exec ") {
                    format!("exec {ff} -hide_banner -loglevel error \"$@\"")
                } else {
                    l.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&tool, script).unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let inputs: Vec<PathBuf> = a[1]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| base.join(s(v)))
            .collect();
        let mut progress = Vec::new();
        let res = capdraft_core::video_merge::merge_videos(
            &inputs,
            &base.join(s(&a[2])),
            Some(&tool.to_string_lossy()),
            None,
            a[3].as_bool().unwrap(),
            &mut |p, m| progress.push(json!([p, m])),
        );
        let result = match res {
            Ok(m) => {
                json!({"ok": {"output_path": m.output_path.to_string_lossy(), "used_reencode": m.used_reencode,
                                   "ffmpeg_path": m.ffmpeg_path, "output_size_positive": std::fs::metadata(&m.output_path).unwrap().len() > 0}})
            }
            Err(e) => json!({"err": e}),
        };
        let log = std::fs::read_to_string(base.join("tools/ffmpeg.log")).unwrap_or_default();
        let root = base.to_string_lossy().into_owned();
        let log = log.replace(&root, "<ROOT>");
        let log: String = log
            .split('\n')
            .map(|line| {
                if line.starts_with("ARGS:") {
                    line.split(' ')
                        .map(|t| {
                            if t.contains("capdraft_concat_") {
                                "[<LIST>]"
                            } else {
                                t
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let got = json!({"result": result, "progress": progress, "ffmpeg_log": log});
        let got = cj::parse(&dumps_compact(&got).replace(&root, "<ROOT>")).unwrap();
        r.check(&format!("merge #{n} {}", a[0]), &case["ok"], &got);
        let _ = std::fs::remove_dir_all(&base);
    }
    r.finish();
}
