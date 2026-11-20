//! Run every ported function on a real `draft_content.json` and write the
//! results to a folder, for `rust/tools/check_real_draft.py` to compare with
//! the Python original byte for byte.
//!
//! ```sh
//! cargo run --release --example real_draft -- <draft_content.json> <out_dir>
//! ```

use capdraft_core::draft_reader::{DraftReader, extract_captions};
use capdraft_core::models::CaptionRow;
use capdraft_core::project_split::{
    caption_edges_from_draft, choose_cut_us, slice_draft, split_project,
};
use capdraft_core::pyjson::{dumps_compact, dumps_indent, float, int, loads};
use capdraft_core::{ids, srt, validator};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const CLOCK: f64 = 1_700_000_000.25;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let [_, draft_path, out_dir] = args.as_slice() else {
        return Err("usage: real_draft <draft_content.json> <out_dir>".into());
    };
    let out = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut timings = Map::new();
    let mut time = |name: &str, start: Instant| {
        timings.insert(
            name.into(),
            float((start.elapsed().as_secs_f64() * 1e4).round() / 1e4),
        );
    };
    let write =
        |name: &str, text: &str| std::fs::write(out.join(name), text).map_err(|e| e.to_string());

    let t = Instant::now();
    let bytes = std::fs::read(draft_path).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    let draft = loads(text.strip_prefix('\u{feff}').unwrap_or(&text))?;
    time("load", t);
    let t = Instant::now();
    let dumped = dumps_compact(&draft);
    time("dumps_compact", t);
    write("roundtrip.json", &dumped)?;

    let t = Instant::now();
    let captions = extract_captions(&draft)?;
    time("extract_captions", t);
    write(
        "captions.json",
        &dumps_compact(&Value::Array(captions.iter().map(caption_json).collect())),
    )?;
    write("captions.srt", &srt::render_captions_srt(&captions))?;

    let fake_path = PathBuf::from("/p/Proj/draft_content.json");
    let mut reader = DraftReader::from_draft(draft.clone(), fake_path);
    let info = reader.get_project_info()?;
    let t = Instant::now();
    let inspect = reader.inspect_project(None)?;
    time("inspect_project", t);
    let t = Instant::now();
    let errors = validator::validate(&draft, None, false)?;
    time("validate", t);
    let edges = caption_edges_from_draft(&draft)?;
    let total = info.duration_us;
    let cut = choose_cut_us(total, &edges)?;
    let summary = json!({
        "project_info": {
            "project_id": info.project_id, "project_name": info.project_name, "version": info.version,
            "new_version": info.new_version, "width": info.width, "height": info.height, "fps": float(info.fps),
            "duration_us": info.duration_us, "duration_display": info.duration_display(), "resolution": info.resolution(),
            "video_track_count": info.video_track_count, "audio_track_count": info.audio_track_count,
            "text_track_count": info.text_track_count, "video_segment_count": info.video_segment_count,
            "audio_segment_count": info.audio_segment_count, "text_segment_count": info.text_segment_count,
            "caption_count": info.caption_count, "caption_with_tts_count": info.caption_with_tts_count,
            "empty_caption_count": info.empty_caption_count,
        },
        "inspect": {
            "valid_caption_count": inspect.valid_caption_count, "selected_caption_count": inspect.selected_caption_count,
            "skipped_empty_count": inspect.skipped_empty_count, "existing_tts_count": inspect.existing_tts_count,
            "orphan_text_segment_count": inspect.orphan_text_segment_count,
            "invalid_content_count": inspect.invalid_content_count, "warnings": inspect.warnings,
            "has_tts_template": inspect.has_tts_template,
        },
        "find_tts_templates": reader.find_tts_templates()?,
        "find_speed_templates": reader.find_speed_templates()?,
        "get_tts_templates": reader.get_tts_templates()?,
        "validate": errors,
        "caption_edges_count": edges.len(),
        "cut_us": cut,
    });
    write("summary.json", &dumps_indent(&summary, 1))?;

    for part in [1, 2] {
        ids::deterministic(CLOCK);
        let t = Instant::now();
        let (d, n) = slice_draft(&draft, cut, part, "", None, true)?;
        time(&format!("slice_part{part}"), t);
        ids::reset();
        write(
            &format!("part{part}.json"),
            &dumps_compact(&json!([d, int(n)])),
        )?;
    }

    // Whole split on a scratch copy of the project.
    let parent = std::env::temp_dir().join(format!("capdraft-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    let proj = parent.join("Proj");
    std::fs::create_dir_all(proj.join("Timelines").join(info.project_id.as_str()))
        .map_err(|e| e.to_string())?;
    let parent = std::fs::canonicalize(&parent).map_err(|e| e.to_string())?;
    let proj = parent.join("Proj");
    let root = parent.to_string_lossy().into_owned();
    std::fs::write(proj.join("draft_content.json"), &text).map_err(|e| e.to_string())?;
    std::fs::write(
        proj.join("Timelines")
            .join(&info.project_id)
            .join("draft_content.json"),
        &text,
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(proj.join("draft_meta_info.json"), META).map_err(|e| e.to_string())?;
    std::fs::write(
        parent.join("root_meta_info.json"),
        ROOT_META.replace("<ROOT>", &root),
    )
    .map_err(|e| e.to_string())?;
    ids::deterministic(CLOCK);
    let t = Instant::now();
    split_project(
        &proj,
        &draft,
        &proj.join("draft_content.json"),
        None,
        None,
        None,
        &mut |_, _| {},
    )?;
    time("split_project", t);
    ids::reset();
    let mut tree = BTreeMap::new();
    snapshot(&parent, &parent, &root, &mut tree);
    write(
        "tree.json",
        &dumps_indent(&Value::Object(tree.into_iter().collect()), 1),
    )?;
    let _ = std::fs::remove_dir_all(&parent);

    write("timings.json", &dumps_indent(&Value::Object(timings), 1))?;
    Ok(())
}

pub const META: &str =
    r#"{"draft_id":"SRCMETA","draft_name":"Proj","duration":1,"draft_cover":"","tm_duration":1}"#;
pub const ROOT_META: &str = r#"{"all_draft_store":[{"draft_fold_path":"<ROOT>/Proj","draft_name":"Proj","draft_id":"SRCMETA","tm_duration":1,"tm_draft_create":5}],"draft_ids":1,"root_path":"<ROOT>"}"#;

fn caption_json(c: &CaptionRow) -> Value {
    json!({
        "index": c.index, "text_track_id": c.text_track_id, "text_segment_id": c.text_segment_id,
        "text_material_id": c.text_material_id, "start_us": c.start_us, "duration_us": c.duration_us,
        "text": c.text, "existing_tts_segment_ids": c.existing_tts_segment_ids,
    })
}

/// Every file under `base`: its size and a short fingerprint (FNV-1a, 64-bit).
fn snapshot(base: &Path, dir: &Path, root: &str, out: &mut BTreeMap<String, Value>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            snapshot(base, &p, root, out);
        } else if let Ok(data) = std::fs::read(&p) {
            let normalized = match String::from_utf8(data) {
                Ok(t) => t.replace(root, "<ROOT>").into_bytes(),
                Err(e) => e.into_bytes(),
            };
            let rel = p
                .strip_prefix(base)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            out.insert(
                rel,
                json!([normalized.len(), format!("{:016x}", fnv1a(&normalized))]),
            );
        }
    }
}

fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}
