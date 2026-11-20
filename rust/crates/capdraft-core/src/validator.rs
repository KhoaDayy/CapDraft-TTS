//! Integrity checks on a patched draft before it is committed
//! (port of `core/capcut_project/validator.py`).

use crate::paths::path_name;
use crate::pyval::{
    self as py, Key, PyNum, PyResult, get, get_or, in_str_keys, int_or0, is_dict, is_str, iter,
    iter_or_empty, key, num, or, repr, str_of, to_float, truthy,
};
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static EMPTY_DICT: LazyLock<Value> = LazyLock::new(|| json!({}));
static EMPTY_LIST: Value = Value::Array(Vec::new());
static ONE: LazyLock<Value> = LazyLock::new(|| crate::pyjson::float(1.0));

/// `"; "`-joined first five errors, as `ValidationError` words them.
#[must_use]
pub fn validation_error_message(errors: &[String]) -> String {
    let head = errors
        .iter()
        .take(5)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    if errors.len() > 5 {
        format!("{head} (+{} more)", errors.len() - 5)
    } else {
        head
    }
}

fn positive_number(v: &Value) -> Option<f64> {
    num(v).map(PyNum::as_f64)
}

/// Python `<` between two segment ids, for sorting `(start, end, id)` tuples.
fn id_cmp(a: &Value, b: &Value) -> Result<Ordering, String> {
    if py::eq(a, b) {
        return Ok(Ordering::Equal);
    }
    match (a, b) {
        (Value::String(x), Value::String(y)) => Ok(x.cmp(y)),
        _ => match (num(a), num(b)) {
            (Some(x), Some(y)) => Ok(x
                .as_f64()
                .partial_cmp(&y.as_f64())
                .unwrap_or(Ordering::Equal)),
            _ => Err(format!(
                "TypeError: '<' not supported between instances of '{}' and '{}'",
                py::type_name(a),
                py::type_name(b)
            )),
        },
    }
}

/// Errors found in `draft` (empty means it is safe to write).
///
/// With `project_directory` and `require_audio_files`, every TTS audio file
/// must exist and be non-empty.
///
/// # Errors
/// Where the original raises on a malformed draft (ADR-011).
pub fn validate(
    draft: &Value,
    project_directory: Option<&Path>,
    require_audio_files: bool,
) -> PyResult<Vec<String>> {
    let mut errors: Vec<String> = Vec::new();
    let materials = or(get(draft, "materials")?, &EMPTY_DICT);
    let tracks = or(get(draft, "tracks")?, &EMPTY_LIST);

    let mut by_id: HashMap<String, (String, &Value)> = HashMap::new();
    let mut duplicate_ids: Vec<String> = Vec::new();
    for (bucket, items) in py::items(materials)? {
        let Value::Array(items) = items else { continue };
        for it in items.iter().filter(|it| is_dict(it)) {
            let iid = get(it, "id")?;
            if !truthy(iid) {
                continue;
            }
            if in_str_keys(iid, &by_id)? {
                duplicate_ids.push(str_of(iid));
            }
            by_id.insert(str_of(iid), (bucket.clone(), it));
        }
    }

    let track_items = iter(tracks)?;
    let mut segments_by_id: HashMap<String, ()> = HashMap::new();
    for track in track_items.iter() {
        for seg in iter_or_empty(get(track, "segments")?)?.iter() {
            let sid = get(seg, "id")?;
            if !truthy(sid) {
                continue;
            }
            if in_str_keys(sid, &segments_by_id)? {
                duplicate_ids.push(str_of(sid));
            }
            segments_by_id.insert(str_of(sid), ());
            if in_str_keys(sid, &by_id)? {
                duplicate_ids.push(str_of(sid));
            }
        }
    }
    if !duplicate_ids.is_empty() {
        let mut uniq: Vec<String> = duplicate_ids
            .into_iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        uniq.sort();
        errors.push(format!(
            "Duplicate UUIDs ({}): e.g. {}",
            uniq.len(),
            uniq[0]
        ));
    }

    let dicts = |v: &Value| -> PyResult<Vec<Value>> {
        Ok(iter_or_empty(v)?
            .iter()
            .filter(|x| is_dict(x))
            .cloned()
            .collect())
    };
    let texts = dicts(get(materials, "texts")?)?;
    let audios = dicts(get(materials, "audios")?)?;
    let tts_audios: Vec<&Value> = audios
        .iter()
        .filter(|a| is_str(&a["type"], "text_to_audio"))
        .collect();
    let mut text_ids: HashSet<Key> = HashSet::new();
    for t in &texts {
        if truthy(&t["id"]) {
            text_ids.insert(key(&t["id"])?);
        }
    }

    for t in &texts {
        for sid in iter_or_empty(&t["text_to_audio_ids"])?.iter() {
            if !in_str_keys(sid, &segments_by_id)? {
                errors.push(format!(
                    "Caption material {} text_to_audio_ids -> missing segment {}",
                    str_of(&t["id"]),
                    str_of(sid)
                ));
            }
        }
    }

    let mut audio_tracks: Vec<&Value> = Vec::new();
    for t in track_items.iter() {
        if is_str(get(t, "type")?, "audio") {
            audio_tracks.push(t);
        }
    }

    for track in &audio_tracks {
        for seg in iter_or_empty(get(track, "segments")?)?.iter() {
            check_audio_segment(seg, &by_id, &mut errors)?;
        }
    }

    for a in &tts_audios {
        let id = str_of(&a["id"]);
        let tid = &a["text_id"];
        if !truthy(tid) || !text_ids.contains(&key(tid)?) {
            errors.push(format!("TTS audio {id} text_id invalid: {}", str_of(tid)));
        }
        let dur = &a["duration"];
        if positive_number(dur).is_none_or(|d| d <= 0.0) {
            errors.push(format!("TTS audio {id} duration invalid: {}", str_of(dur)));
        }
        if require_audio_files && let Some(pd) = project_directory {
            let path = if truthy(&a["path"]) {
                str_of(&a["path"])
            } else {
                String::new()
            };
            if path.is_empty() {
                errors.push(format!("TTS audio {id} missing path"));
            } else {
                let ok = resolve_audio_path(&path, pd)
                    .and_then(|p| std::fs::metadata(p).ok())
                    .is_some_and(|m| m.len() > 0);
                if !ok {
                    errors.push(format!("TTS audio file missing: {path}"));
                }
            }
        }
    }

    let mut referenced_extras: HashSet<String> = HashSet::new();
    for track in track_items.iter() {
        for seg in iter_or_empty(get(track, "segments")?)?.iter() {
            for r in iter_or_empty(get(seg, "extra_material_refs")?)?.iter() {
                referenced_extras.insert(str_of(r));
            }
        }
    }
    for fade in iter_or_empty(get(materials, "audio_fades")?)?.iter() {
        if !is_dict(fade) {
            continue;
        }
        let fid = &fade["id"];
        if truthy(fid) && !referenced_extras.contains(&str_of(fid)) {
            errors.push(format!("Orphan audio_fade material: {}", str_of(fid)));
        }
    }

    for track in &audio_tracks {
        let mut intervals: Vec<(i64, i64, Value)> = Vec::new();
        for seg in iter_or_empty(get(track, "segments")?)?.iter() {
            let tgt = or(get(seg, "target_timerange")?, &EMPTY_DICT);
            let start = int_or0(get(tgt, "start")?)?;
            let dur = int_or0(get(tgt, "duration")?)?;
            intervals.push((start, start + dur, get(seg, "id")?.clone()));
        }
        let mut type_error: Option<String> = None;
        intervals.sort_by(|a, b| {
            (a.0, a.1)
                .cmp(&(b.0, b.1))
                .then_with(|| match id_cmp(&a.2, &b.2) {
                    Ok(o) => o,
                    Err(e) => {
                        type_error.get_or_insert(e);
                        Ordering::Equal
                    }
                })
        });
        if let Some(e) = type_error {
            return Err(e);
        }
        for w in intervals.windows(2) {
            if w[1].0 < w[0].1 {
                errors.push(format!(
                    "Overlap on audio track {}: {} and {}",
                    str_of(get(track, "id")?),
                    str_of(&w[0].2),
                    str_of(&w[1].2)
                ));
            }
        }
    }
    Ok(errors)
}

fn check_audio_segment(
    seg: &Value,
    by_id: &HashMap<String, (String, &Value)>,
    errors: &mut Vec<String>,
) -> PyResult<()> {
    let sid = str_of(get(seg, "id")?);
    let mid = get(seg, "material_id")?;
    let mut mat: Option<&Value> = None;
    if in_str_keys(mid, by_id)? {
        let (bucket, m) = &by_id[&str_of(mid)];
        mat = Some(m);
        if bucket != "audios" {
            errors.push(format!(
                "Audio segment {sid} material {} is in {bucket}",
                str_of(mid)
            ));
        }
    } else {
        errors.push(format!(
            "Audio segment {sid} references missing material {}",
            str_of(mid)
        ));
    }

    let src = or(get(seg, "source_timerange")?, &EMPTY_DICT);
    let tgt = or(get(seg, "target_timerange")?, &EMPTY_DICT);
    if !is_dict(src) || !is_dict(tgt) {
        errors.push(format!("Audio segment {sid} missing timerange objects"));
    } else {
        for (name, tr) in [("source", src), ("target", tgt)] {
            let dur = &tr["duration"];
            let start = &tr["start"];
            if positive_number(dur).is_none_or(|d| d <= 0.0) {
                errors.push(format!(
                    "Audio segment {sid} {name}_timerange.duration invalid: {}",
                    str_of(dur)
                ));
            }
            if positive_number(start).is_none_or(|s| s < 0.0) {
                errors.push(format!(
                    "Audio segment {sid} {name}_timerange.start invalid: {}",
                    str_of(start)
                ));
            }
            if positive_number(dur).is_some_and(|d| d > 0.0 && d < 1000.0) {
                errors.push(format!(
                    "Audio segment {sid} {name}_timerange.duration looks like seconds not us: {}",
                    str_of(dur)
                ));
            }
        }
    }

    let tone = get(seg, "is_tone_modify")?;
    if !tone.is_boolean() {
        errors.push(format!(
            "Audio segment {sid} is_tone_modify not bool: {}",
            repr(tone)
        ));
    }

    let refs = iter_or_empty(get(seg, "extra_material_refs")?)?;
    let mut speed_refs: Vec<&Value> = Vec::new();
    for r in refs.iter() {
        if !in_str_keys(r, by_id)? {
            errors.push(format!(
                "Audio segment {sid} extra ref missing: {}",
                str_of(r)
            ));
            continue;
        }
        let (bucket, obj) = &by_id[&str_of(r)];
        if bucket == "speeds" || is_str(&obj["type"], "speed") {
            speed_refs.push(obj);
        }
    }
    let mut fade_refs: Vec<&Value> = Vec::new();
    for r in refs.iter() {
        if !in_str_keys(r, by_id)? {
            continue;
        }
        let (bucket, obj) = &by_id[&str_of(r)];
        if bucket == "audio_fades" || is_str(&obj["type"], "audio_fade") {
            fade_refs.push(obj);
        }
    }

    let Some(mat) = mat.filter(|m| truthy(m) && is_str(&m["type"], "text_to_audio")) else {
        return Ok(());
    };
    if speed_refs.len() == 1 {
        let material_speed = to_float(get_or(speed_refs[0], "speed", &ONE)?)?;
        let seg_speed = get(seg, "speed")?;
        let segment_speed = to_float(or(seg_speed, &ONE))?;
        if (material_speed - segment_speed).abs() > 1e-6 {
            errors.push(format!(
                "TTS segment {sid} speed={} != speed_material={}",
                str_of(seg_speed),
                str_of(&speed_refs[0]["speed"])
            ));
        }
    } else {
        errors.push(format!(
            "TTS segment {sid} must have exactly one speed material, got {}",
            speed_refs.len()
        ));
    }
    let mat_dur = int_or0(&mat["duration"])?;
    let src_tr = or(get(seg, "source_timerange")?, &EMPTY_DICT);
    let src_start = int_or0(get(src_tr, "start")?)?;
    let src_dur = int_or0(get(src_tr, "duration")?)?;
    if src_start < 0 || src_dur <= 0 {
        errors.push(format!("TTS segment {sid} invalid source_timerange"));
    } else if mat_dur > 0 && src_start + src_dur > mat_dur + 1 {
        errors.push(format!(
            "TTS segment {sid} source range exceeds material duration ({src_start}+{src_dur} > {mat_dur})"
        ));
    }
    for fade in fade_refs {
        let fi = int_or0(&fade["fade_in_duration"])?;
        let tgt_dur = int_or0(get(
            or(get(seg, "target_timerange")?, &EMPTY_DICT),
            "duration",
        )?)?;
        if fi < 0 || (tgt_dur > 0 && fi > tgt_dur) {
            errors.push(format!(
                "TTS segment {sid} fade_in_duration {fi} invalid for target {tgt_dur}"
            ));
        }
    }
    Ok(())
}

/// Where a TTS audio `path` (absolute, project-relative or behind a
/// draftpath placeholder) lives on disk.
#[must_use]
pub fn resolve_audio_path(path: &str, project_directory: &Path) -> Option<PathBuf> {
    let p = Path::new(path);
    if p.is_file() {
        return Some(p.to_path_buf());
    }
    let text_reading = project_directory.join("textReading");
    if path.contains("##_draftpath_placeholder_")
        || path.starts_with("textReading/")
        || path.contains("/textReading/")
    {
        if let Some((_, rel)) = path.split_once("textReading/") {
            let candidate = text_reading.join(rel);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        let candidate = text_reading.join(path_name(path));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    [
        project_directory.join(path),
        project_directory.join(path_name(path)),
    ]
    .into_iter()
    .find(|c| c.exists())
}
