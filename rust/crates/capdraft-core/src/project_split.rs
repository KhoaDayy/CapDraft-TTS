//! Split a CapCut project into two time halves CapCut can open and export
//! (port of `core/capcut_project/project_split.py`).
//!
//! Handles flat drafts and nested combination drafts (`materials.drafts[].draft`).

use crate::ids::{new_uuid_upper, now_unix};
use crate::json::{Object, Value};
use crate::jv as json;
use crate::pyfmt::{py_strip, round_half_even};
use crate::pyjson::{dumps_compact, dumps_indent, int, text_mode};
use crate::pyval::{
    Key, NONE, PyResult, get, get_or, int_or0, is_dict, is_int, is_str, iter, iter_or_empty, key,
    or, str_of, to_float, to_int, truthy,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static EMPTY_DICT: LazyLock<Value> = LazyLock::new(Value::empty_object);
static EMPTY_LIST: LazyLock<Value> = LazyLock::new(|| Value::Array(crate::json::Array::new()));

const IGNORE_NAMES: [&str; 6] = [
    "backups",
    "logs",
    ".DS_Store",
    "Thumbs.db",
    "tts_generation_manifest.json",
    "capdraft_split_manifest.json",
];

/// Marker CapCut leaves next to timelines (observed on working projects).
const DRAFT_EXTRA: &[u8] =
    b"i\x06\x00\x00\x00i\x00\x00\x00\x00ei\x00\x00\x00\x00\x02\x00\x00\x00{}\x99\xc0e\xc3";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitResult {
    pub cut_us: i64,
    pub part1_dir: PathBuf,
    pub part2_dir: PathBuf,
    pub part1_draft: PathBuf,
    pub part2_draft: PathBuf,
    pub part1_duration_us: i64,
    pub part2_duration_us: i64,
    pub part1_segments: i64,
    pub part2_segments: i64,
}

/// Midpoint cut, snapped to the nearest caption edge when both halves stay long enough.
///
/// # Errors
/// `ValueError: Project duration too short to split`.
pub fn choose_cut_us(duration_us: i64, caption_edges_us: &[i64]) -> PyResult<i64> {
    let duration_us = duration_us.max(0);
    if duration_us <= 1 {
        return Err("ValueError: Project duration too short to split".into());
    }
    let mid = duration_us / 2;
    let Some(best) = caption_edges_us
        .iter()
        .copied()
        .filter(|e| 0 < *e && *e < duration_us)
        .min_by_key(|e| ((e - mid).abs(), *e))
    else {
        return Ok(mid);
    };
    let min_half = (duration_us / 50).max(1_000_000);
    if best < min_half || duration_us - best < min_half {
        return Ok(mid);
    }
    Ok(best)
}

fn as_obj(v: &mut Value) -> Option<&mut Object> {
    v.as_object_mut()
}

fn seg_range(seg: &Value) -> PyResult<(i64, i64)> {
    let tr = or(get(seg, "target_timerange")?, &EMPTY_DICT);
    let start = int_or0(get(tr, "start")?)?;
    let dur = int_or0(get(tr, "duration")?)?.max(0);
    Ok((start, start + dur))
}

fn set_target(seg: &mut Object, start: i64, duration: i64) {
    if !seg.get("target_timerange").is_some_and(Value::is_object) {
        seg.insert("target_timerange", json!({}));
    }
    if let Some(tr) = seg.get_mut("target_timerange").and_then(as_obj) {
        tr.insert("start", int(start));
        tr.insert("duration", int(duration.max(0)));
    }
}

/// `float(seg.get("speed") or 1.0)`, falling back to 1.0 on bad values.
fn clip_speed(seg: &Object) -> f64 {
    let v = seg.get("speed").unwrap_or(&NONE);
    let sp = if truthy(v) {
        to_float(v).unwrap_or(1.0)
    } else {
        1.0
    };
    if sp > 0.0 { sp } else { 1.0 }
}

fn trim_source_head(seg: &mut Object, trim_target_us: i64) -> PyResult<()> {
    if trim_target_us <= 0 {
        return Ok(());
    }
    let speed = clip_speed(seg);
    let Some(src) = seg.get_mut("source_timerange").and_then(as_obj) else {
        return Ok(());
    };
    let src_trim = round_half_even(trim_target_us as f64 * speed);
    let start = int_or0(src.get("start").unwrap_or(&NONE))? + src_trim;
    src.insert("start", int(start));
    let dur = (int_or0(src.get("duration").unwrap_or(&NONE))? - src_trim).max(0);
    src.insert("duration", int(dur));
    Ok(())
}

fn trim_source_tail(seg: &mut Object, new_target_dur: i64, old_target_dur: i64) -> PyResult<()> {
    if new_target_dur >= old_target_dur || old_target_dur <= 0 {
        return Ok(());
    }
    let speed = clip_speed(seg);
    let Some(src) = seg.get_mut("source_timerange").and_then(as_obj) else {
        return Ok(());
    };
    let alt = round_half_even(new_target_dur as f64 * speed).max(0);
    if alt > 0 {
        src.insert("duration", int(alt));
    } else {
        let ratio = new_target_dur as f64 / old_target_dur as f64;
        let old = int_or0(src.get("duration").unwrap_or(&NONE))?;
        src.insert("duration", int(round_half_even(old as f64 * ratio).max(0)));
    }
    Ok(())
}

fn slice_segment(seg: &Value, cut_us: i64, part: i64) -> PyResult<Option<Value>> {
    let (start, end) = seg_range(seg)?;
    let mut out = seg.clone();
    let Some(o) = out.as_object_mut() else {
        return Ok(None);
    };
    if part == 1 {
        if start >= cut_us || end <= start {
            return Ok(None);
        }
        if end <= cut_us {
            return Ok(Some(out));
        }
        let (old_dur, new_dur) = (end - start, cut_us - start);
        set_target(o, start, new_dur);
        trim_source_tail(o, new_dur, old_dur)?;
    } else {
        if end <= cut_us || end <= start {
            return Ok(None);
        }
        if start >= cut_us {
            set_target(o, start - cut_us, end - start);
            return Ok(Some(out));
        }
        let old_dur = end - start;
        let trim = cut_us - start;
        let new_dur = end - cut_us;
        set_target(o, 0, new_dur);
        trim_source_head(o, trim)?;
        let remain_old = if old_dur > trim {
            old_dur - trim
        } else {
            new_dur
        };
        trim_source_tail(o, new_dur, remain_old)?;
    }
    Ok(Some(out))
}

fn filter_tracks(tracks: &Value, cut_us: i64, part: i64) -> PyResult<(Vec<Value>, i64)> {
    let mut kept = Vec::new();
    let mut seg_count = 0;
    for track in iter(tracks)?.iter() {
        if !is_dict(track) {
            continue;
        }
        let mut t = track.clone();
        let mut segs_out = Vec::new();
        for seg in iter_or_empty(&t["segments"])?.iter() {
            if !is_dict(seg) {
                continue;
            }
            let Some(sliced) = slice_segment(seg, cut_us, part)? else {
                continue;
            };
            let (s0, e0) = seg_range(&sliced)?;
            if e0 <= s0 {
                continue;
            }
            segs_out.push(sliced);
            seg_count += 1;
        }
        if let Some(o) = t.as_object_mut() {
            o.insert("segments", Value::from(segs_out));
        }
        kept.push(t);
    }
    Ok((kept, seg_count))
}

fn collect_refs(tracks: &[Value]) -> PyResult<HashSet<String>> {
    let mut ids = HashSet::new();
    for track in tracks.iter().filter(|t| is_dict(t)) {
        for seg in iter_or_empty(&track["segments"])?.iter() {
            if !is_dict(seg) {
                continue;
            }
            if truthy(&seg["material_id"]) {
                ids.insert(str_of(&seg["material_id"]));
            }
            for r in iter_or_empty(&seg["extra_material_refs"])?.iter() {
                if truthy(r) {
                    ids.insert(str_of(r));
                }
            }
        }
    }
    Ok(ids)
}

fn prune_materials(materials: &Object, keep_ids: &HashSet<String>) -> Object {
    let mut out = Object::new();
    for (bucket, items) in materials {
        let Value::Array(items) = items else {
            out.insert(bucket.clone(), items.clone());
            continue;
        };
        let kept = items
            .iter()
            .filter(|it| {
                is_dict(it) && (it["id"].is_null() || keep_ids.contains(&str_of(&it["id"])))
            })
            .cloned()
            .collect();
        out.insert(bucket.clone(), Value::Array(kept));
    }
    out
}

/// Shrink a full-span video/audio material to the new timeline length.
fn material_duration_us(item: &mut Object, new_duration: i64) {
    let Some(d) = item.get("duration").filter(|d| !d.is_null()) else {
        return;
    };
    let Ok(old) = int_or0(d) else { return };
    if old > 0 {
        item.insert(
            "duration",
            int(if old >= new_duration {
                old.min(new_duration)
            } else {
                old
            }),
        );
    }
}

/// `(sliced_draft, segment_count)` for `part` 1 or 2, nested combinations included.
///
/// # Errors
/// `ValueError` for a bad part or cut, and where the original raises on a malformed draft.
pub fn slice_draft(
    draft: &Value,
    cut_us: i64,
    part: i64,
    name_suffix: &str,
    new_content_id: Option<&str>,
    rename_content: bool,
) -> PyResult<(Value, i64)> {
    if part != 1 && part != 2 {
        return Err("ValueError: part must be 1 or 2".into());
    }
    let total = int_or0(get(draft, "duration")?)?;
    if cut_us <= 0 || (total > 0 && cut_us >= total) {
        if total > 1 {
            return Err(format!(
                "ValueError: cut_us must be inside (0, duration); got {cut_us} / {total}"
            ));
        }
        return Err("ValueError: Project duration too short to split".into());
    }

    let mut out = draft.clone();
    let (tracks, mut seg_count) = filter_tracks(or(&out["tracks"], &EMPTY_LIST), cut_us, part)?;
    let mut keep_ids = collect_refs(&tracks)?;
    let o = out
        .as_object_mut()
        .ok_or_else(|| crate::pyval::no_attr(draft, "get"))?;
    o.insert("tracks", Value::from(tracks));

    let new_dur = if part == 1 {
        cut_us
    } else {
        (total - cut_us).max(0)
    };
    let mut sliced_combo_ids: HashSet<String> = HashSet::new();
    let mats_is_dict = o.get("materials").is_some_and(Value::is_object);
    if let Some(mats) = o.get_mut("materials").and_then(as_obj) {
        if let Some(Value::Array(nested)) = mats.get("drafts").cloned() {
            let mut new_list = Vec::new();
            for item in &nested {
                if !is_dict(item) {
                    continue;
                }
                let iid = &item["id"];
                let referenced = iid.is_null() || keep_ids.contains(&str_of(iid));
                if item["draft"].is_object() && (referenced || keep_ids.is_empty()) {
                    let (nd, nseg) =
                        slice_draft(&item["draft"], cut_us, part, name_suffix, None, false)?;
                    let mut item = item.clone();
                    if let Some(io) = item.as_object_mut() {
                        io.insert("draft", nd);
                    }
                    seg_count += nseg;
                    new_list.push(item);
                    if truthy(iid) {
                        keep_ids.insert(str_of(iid));
                        sliced_combo_ids.insert(str_of(iid));
                    }
                } else if referenced {
                    new_list.push(item.clone());
                }
            }
            mats.insert("drafts", Value::from(new_list));
        }
        let mut pruned = prune_materials(mats, &keep_ids);
        for bucket in ["videos", "audios"] {
            let items = pruned
                .get_mut(bucket)
                .map(std::mem::take)
                .unwrap_or(Value::Null);
            let mut items = items;
            // `for it in mats.get(bucket) or []`: a truthy non-list fails here.
            iter_or_empty(&items)?;
            if let Value::Array(list) = &mut items {
                for it in list.iter_mut().filter_map(as_obj) {
                    material_duration_us(it, new_dur);
                }
            }
            if let Some(slot) = pruned.get_mut(bucket) {
                *slot = items;
            }
        }
        *mats = pruned;
    }

    if !sliced_combo_ids.is_empty() {
        let mut video_updates: Vec<(String, i64)> = Vec::new();
        if let Some(Value::Array(tracks)) = o.get_mut("tracks") {
            for track in tracks.iter_mut().filter_map(as_obj) {
                let Some(Value::Array(segs)) = track.get_mut("segments") else {
                    continue;
                };
                for seg in segs.iter_mut().filter_map(as_obj) {
                    let mut refs: HashSet<String> = HashSet::new();
                    for x in iter_or_empty(seg.get("extra_material_refs").unwrap_or(&NONE))?.iter()
                    {
                        if truthy(x) {
                            refs.insert(str_of(x));
                        }
                    }
                    let mid_v = seg.get("material_id").unwrap_or(&NONE);
                    let mid = if truthy(mid_v) {
                        str_of(mid_v)
                    } else {
                        String::new()
                    };
                    if !(refs.iter().any(|r| sliced_combo_ids.contains(r))
                        || sliced_combo_ids.contains(&mid))
                    {
                        continue;
                    }
                    if !seg.get("source_timerange").is_some_and(Value::is_object) {
                        seg.insert("source_timerange", json!({}));
                    }
                    let tgt = or(seg.get("target_timerange").unwrap_or(&NONE), &EMPTY_DICT).clone();
                    let d = get(&tgt, "duration")?;
                    let tdur = if truthy(d) { to_int(d)? } else { new_dur };
                    if let Some(src) = seg.get_mut("source_timerange").and_then(as_obj) {
                        src.insert("start", int(0));
                        src.insert("duration", int(tdur));
                    }
                    if mats_is_dict {
                        video_updates.push((mid, tdur));
                    }
                }
            }
        }
        if let Some(Value::Array(videos)) = o.get_mut("materials").and_then(|m| m.get_mut("videos"))
        {
            for (mid, tdur) in video_updates {
                for it in videos.iter_mut().filter_map(as_obj) {
                    let id = it.get("id").unwrap_or(&NONE);
                    let id = if truthy(id) {
                        str_of(id)
                    } else {
                        String::new()
                    };
                    if id == mid {
                        it.insert("duration", int(tdur));
                    }
                }
            }
        }
    }

    o.insert("duration", int(new_dur));
    if rename_content {
        let id = new_content_id
            .filter(|s| !s.is_empty())
            .map_or_else(new_uuid_upper, str::to_owned);
        o.insert("id", Value::from(id));
        let name = o.get("name").unwrap_or(&NONE);
        let base_name = py_strip(&if truthy(name) {
            str_of(name)
        } else {
            String::new()
        })
        .to_owned();
        if !name_suffix.is_empty() {
            let new_name = if base_name.is_empty() {
                py_strip(name_suffix).to_owned()
            } else {
                base_name + name_suffix
            };
            o.insert("name", Value::from(new_name));
        }
    }
    Ok((out, seg_count))
}

/// Subtitle start times from the draft and its nested combinations.
///
/// # Errors
/// Where the original raises on a malformed draft.
pub fn caption_edges_from_draft(draft: &Value) -> PyResult<Vec<i64>> {
    let mut edges = Vec::new();
    walk_edges(draft, &mut edges)?;
    Ok(edges)
}

fn walk_edges(d: &Value, edges: &mut Vec<i64>) -> PyResult<()> {
    let mats = or(get(d, "materials")?, &EMPTY_DICT);
    let mut texts: HashMap<Key, &Value> = HashMap::new();
    let text_items = iter_or_empty(get(mats, "texts")?)?;
    for t in text_items.iter() {
        if is_dict(t) && truthy(&t["id"]) {
            texts.insert(key(&t["id"])?, t);
        }
    }
    for track in iter_or_empty(get(d, "tracks")?)?.iter() {
        if !is_dict(track) || !is_str(&track["type"], "text") {
            continue;
        }
        for seg in iter_or_empty(&track["segments"])?.iter() {
            if !is_dict(seg) {
                continue;
            }
            if let Some(mat) = texts.get(&key(&seg["material_id"])?) {
                let ty = get(mat, "type")?;
                if !(ty.is_null() || is_str(ty, "subtitle") || is_str(ty, "text")) {
                    continue;
                }
            }
            let tr = or(&seg["target_timerange"], &EMPTY_DICT);
            let start = int_or0(get(tr, "start")?)?;
            if start > 0 {
                edges.push(start);
            }
        }
    }
    for item in iter_or_empty(get(mats, "drafts")?)?.iter() {
        if is_dict(item) && item["draft"].is_object() {
            walk_edges(&item["draft"], edges)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- files

pub(crate) fn io_err(e: &std::io::Error, path: &Path) -> String {
    let kind = match e.kind() {
        std::io::ErrorKind::NotFound => "FileNotFoundError",
        std::io::ErrorKind::AlreadyExists => "FileExistsError",
        std::io::ErrorKind::PermissionDenied => "PermissionError",
        std::io::ErrorKind::IsADirectory => "IsADirectoryError",
        std::io::ErrorKind::NotADirectory => "NotADirectoryError",
        _ => "OSError",
    };
    format!("{kind}: {e}: '{}'", path.display())
}

pub use crate::paths::resolve;

fn norm_fold(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_owned()
}

fn norm_path(p: &Path) -> String {
    norm_fold(&p.to_string_lossy())
}

/// CapCut's `root_meta` mixes separators: `C:/.../folder\file`.
fn capcut_join(fold_fwd: &str, filename: &str) -> String {
    format!("{}\\{filename}", fold_fwd.trim_end_matches('/'))
}

fn write_json(path: &Path, data: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, dumps_compact(data))
}

fn read_json(path: &Path) -> PyResult<Value> {
    crate::draft_reader::read_json_file(path)
}

fn copy_project_tree(src: &Path, dest: &Path) -> PyResult<()> {
    if dest.exists() {
        return Err(format!(
            "FileExistsError: Split output already exists: {}",
            dest.display()
        ));
    }
    copy_tree(src, dest).map_err(|e| io_err(&e, src))
}

/// `shutil.copytree` with the split's ignore list; files are copied in parallel.
fn copy_tree(src: &Path, dest: &Path) -> std::io::Result<()> {
    use rayon::prelude::*;
    std::fs::create_dir_all(dest)?;
    let mut entries: Vec<_> = std::fs::read_dir(src)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    entries.retain(|e| {
        let n = e.file_name();
        let n = n.to_string_lossy();
        !(IGNORE_NAMES.contains(&n.as_ref()) || n.ends_with(".tmp") || n.ends_with(".precommit"))
    });
    entries.par_iter().try_for_each(|entry| {
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)
        } else {
            copy_file(&from, &to)
        }
    })
}

/// `shutil.copy2`: contents plus modification time.
pub(crate) fn copy_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        // `shutil.copy2` on a folder: IsADirectoryError, not Rust's InvalidInput.
        return Err(std::io::Error::new(
            std::io::ErrorKind::IsADirectory,
            "Is a directory",
        ));
    }
    std::fs::copy(from, to)?;
    if let Ok(modified) = std::fs::metadata(from).and_then(|m| m.modified())
        && let Ok(f) = std::fs::File::options().write(true).open(to)
    {
        let _ = f.set_modified(modified);
    }
    Ok(())
}

fn rewrite_obj(obj: &Value, old: &str, new: &str) -> Value {
    match obj {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    let nv = if v.as_str() == Some(old) {
                        Value::from(new)
                    } else {
                        rewrite_obj(v, old, new)
                    };
                    (k.clone(), nv)
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|x| rewrite_obj(x, old, new)).collect()),
        Value::String(s) if s == old => Value::from(new),
        other => other.clone(),
    }
}

/// Point `Timelines/project.json` (+ `.bak`) and `timeline_layout.json` at the new content id.
fn rewrite_timeline_index(project_dir: &Path, old_content_id: &str, new_content_id: &str) {
    if old_content_id.is_empty() || new_content_id.is_empty() || old_content_id == new_content_id {
        return;
    }
    let targets = [
        project_dir.join("Timelines").join("project.json"),
        project_dir.join("Timelines").join("project.json.bak"),
        project_dir.join("timeline_layout.json"),
    ];
    for path in targets {
        if !path.is_file() {
            continue;
        }
        let Ok(data) = read_json(&path) else { continue };
        let mut new_data = rewrite_obj(&data, old_content_id, new_content_id);
        let is_project = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("project.json"));
        if is_project && let Some(o) = new_data.as_object_mut() {
            o.insert("id", Value::from(new_uuid_upper()));
        }
        let _ = write_json(&path, &new_data);
    }
}

fn sorted_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|p| p.file_name().map(std::ffi::OsStr::to_os_string));
    out
}

fn rename_timeline_folder(
    project_dir: &Path,
    old_content_id: &str,
    new_content_id: &str,
) -> PyResult<Option<PathBuf>> {
    let timelines = project_dir.join("Timelines");
    if !timelines.is_dir() {
        return Ok(None);
    }
    let new_dir = timelines.join(new_content_id);
    let mut candidates = Vec::new();
    if !old_content_id.is_empty() {
        let p = timelines.join(old_content_id);
        if p.is_dir() {
            candidates.push(p);
        }
    }
    candidates.extend(
        sorted_dirs(&timelines)
            .into_iter()
            .filter(|p| p.file_name().is_none_or(|n| n != new_content_id)),
    );
    let Some(src) = candidates.into_iter().next() else {
        std::fs::create_dir_all(&new_dir).map_err(|e| io_err(&e, &new_dir))?;
        return Ok(Some(new_dir));
    };
    if resolve(&src) == resolve(&new_dir) {
        return Ok(Some(new_dir));
    }
    if new_dir.exists() && src != new_dir {
        let _ = std::fs::remove_dir_all(&new_dir);
    }
    std::fs::rename(&src, &new_dir).map_err(|e| io_err(&e, &src))?;
    for p in sorted_dirs(&timelines) {
        if p.file_name().is_some_and(|n| n != new_content_id) {
            let _ = std::fs::remove_dir_all(&p);
        }
    }
    Ok(Some(new_dir))
}

/// CapCut refuses to open a timeline without `template.tmp` (a small canvas stub).
fn ensure_template_tmp(tl_dir: &Path, draft: &Value) -> PyResult<()> {
    let path = tl_dir.join("template.tmp");
    if path.is_file() && std::fs::metadata(&path).is_ok_and(|m| m.len() > 50) {
        return Ok(());
    }
    let canvas = if draft["canvas_config"].is_object() {
        &draft["canvas_config"]
    } else {
        &*EMPTY_DICT
    };
    let config = if draft["config"].is_object() {
        draft["config"].clone()
    } else {
        json!({})
    };
    let s = |v: &Value| if truthy(v) { str_of(v) } else { String::new() };
    let (height, width) = (int_or0(&canvas["height"])?, int_or0(&canvas["width"])?);
    let (duration, update_time) = (
        int_or0(&draft["duration"])?,
        int_or0(&draft["update_time"])?,
    );
    let version = if truthy(&draft["version"]) {
        to_int(&draft["version"])?
    } else {
        360_000
    };
    let stub = json!({
        "canvas_config": {
            "background": canvas["background"].clone(),
            "height": height,
            "ratio": or(&canvas["ratio"], &Value::from("original")).clone(),
            "width": width,
        },
        "color_space": get_or(draft, "color_space", &json!(-1))?.clone(),
        "config": config,
        "cover": null,
        "duration": duration,
        "extra_info": null,
        "fps": or(&draft["fps"], &crate::pyjson::float(30.0)).clone(),
        "free_render_index_mode_on": truthy(&draft["free_render_index_mode_on"]),
        "group_container": null,
        "id": s(&draft["id"]),
        "keyframe_graph_list": [],
        "keyframes": {"adjusts": [], "audios": [], "effects": [], "filters": [], "handwrites": [], "stickers": [], "texts": [], "videos": []},
        "last_modified_platform": draft["last_modified_platform"].clone(),
        "materials": {},
        "mutable_config": null,
        "name": "",
        "new_version": s(&draft["new_version"]),
        "platform": draft["platform"].clone(),
        "relationships": [],
        "render_index_track_mode_on": truthy(&draft["render_index_track_mode_on"]),
        "retouch_cover": null,
        "source": or(&draft["source"], &Value::from("default")).clone(),
        "static_cover_image_path": "",
        "time_marks": null,
        "tracks": [],
        "update_time": update_time,
        "version": version,
    });
    write_json(&path, &stub).map_err(|e| io_err(&e, &path))
}

/// Write the root and `Timelines/<content_id>` drafts; rename the timeline folder to the new id.
fn write_draft_targets(
    project_dir: &Path,
    draft: &mut Value,
    old_content_id: &str,
    payload: Option<&str>,
) -> PyResult<PathBuf> {
    let content_id = {
        let v = &draft["id"];
        if truthy(v) { str_of(v) } else { String::new() }
    };
    if let Some(o) = draft.as_object_mut() {
        o.insert("name", Value::String(Default::default()));
    }
    let tl_dir = rename_timeline_folder(project_dir, old_content_id, &content_id)?;
    rewrite_timeline_index(project_dir, old_content_id, &content_id);

    let mut payload_paths = vec![
        project_dir.join("draft_content.json"),
        project_dir.join("draft_content.json.bak"),
        project_dir.join("template-2.tmp"),
    ];
    if let Some(tl) = &tl_dir {
        payload_paths.extend([
            tl.join("draft_content.json"),
            tl.join("draft_content.json.bak"),
            tl.join("template-2.tmp"),
        ]);
        let cover_src = project_dir.join("draft_cover.jpg");
        let cover_dst = tl.join("draft_cover.jpg");
        if cover_src.is_file() && !cover_dst.is_file() {
            let _ = copy_file(&cover_src, &cover_dst);
        }
        let extra = tl.join("draft.extra");
        if !extra.is_file() {
            let _ = std::fs::write(&extra, DRAFT_EXTRA);
        }
        ensure_template_tmp(tl, draft)?;
    }
    // One serialization for the six identical files.
    let owned;
    let payload = match payload {
        Some(p) => p,
        None => {
            owned = dumps_compact(draft);
            &owned
        }
    };
    for path in payload_paths {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, payload);
    }
    Ok(project_dir.join("draft_content.json"))
}

/// Patch `draft_meta_info.json`; returns the new meta `draft_id`.
fn patch_meta(project_dir: &Path, content_draft: &Value, folder_name: &str) -> String {
    let meta_path = project_dir.join("draft_meta_info.json");
    let meta_id = new_uuid_upper();
    if !meta_path.exists() {
        return meta_id;
    }
    let attempt = || -> PyResult<()> {
        let mut meta = read_json(&meta_path)?;
        let Some(m) = meta.as_object_mut() else {
            return Ok(());
        };
        m.insert("draft_id", Value::from(meta_id.clone()));
        m.insert("draft_name", Value::from(folder_name));
        let resolved = resolve(project_dir);
        m.insert("draft_fold_path", Value::from(norm_path(&resolved)));
        let root = resolved.parent().map(norm_path).unwrap_or_default();
        m.insert("draft_root_path", Value::from(root));
        let dur = int_or0(&content_draft["duration"])?;
        m.insert("tm_duration", int(dur));
        if m.contains_key("duration") {
            m.insert("duration", int(dur));
        }
        m.insert("draft_is_invisible", Value::Bool(false));
        if !truthy(m.get("draft_cover").unwrap_or(&NONE)) {
            m.insert("draft_cover", Value::from("draft_cover.jpg"));
        }
        write_json(&meta_path, &meta).map_err(|e| io_err(&e, &meta_path))
    };
    let _ = attempt();
    meta_id
}

/// Drop `all_draft_store` rows whose folder is gone (ghost project cards).
fn purge_stale_root_meta(root_meta: &Path) -> usize {
    if !root_meta.is_file() {
        return 0;
    }
    let Ok(mut data) = read_json(root_meta) else {
        return 0;
    };
    let Some(d) = data.as_object_mut() else {
        return 0;
    };
    let Some(Value::Array(store)) = d.get("all_draft_store") else {
        return 0;
    };
    let mut kept = Vec::new();
    let mut removed = 0;
    for item in store.iter().filter(|i| is_dict(i)) {
        let fold = {
            let v = &item["draft_fold_path"];
            if truthy(v) { str_of(v) } else { String::new() }
        };
        let fp = Path::new(&fold);
        if !fold.is_empty() && fp.is_dir() && fp.join("draft_content.json").is_file() {
            kept.push(item.clone());
        } else {
            removed += 1;
        }
    }
    if removed > 0 {
        let len = kept.len();
        d.insert("all_draft_store", Value::from(kept));
        if d.get("draft_ids").is_some_and(is_int) {
            d.insert("draft_ids", json!(len));
        }
        if write_json(root_meta, &data).is_err() {
            return 0;
        }
    }
    removed
}

/// Add or update the project's row in the parent `root_meta_info.json` so CapCut lists it.
fn register_root_meta(
    project_dir: &Path,
    meta_id: &str,
    folder_name: &str,
    duration_us: i64,
    source_project_dir: &Path,
) {
    let Some(parent) = project_dir.parent() else {
        return;
    };
    let root_meta = parent.join("root_meta_info.json");
    if !root_meta.is_file() {
        return;
    }
    purge_stale_root_meta(&root_meta);
    let Ok(mut data) = read_json(&root_meta) else {
        return;
    };
    let Some(d) = data.as_object_mut() else {
        return;
    };
    let store: Vec<Value> = match d.get("all_draft_store") {
        Some(Value::Array(a)) => a.to_vec(),
        _ => {
            d.insert("all_draft_store", json!([]));
            vec![]
        }
    };
    let resolved = resolve(project_dir);
    let fold = norm_path(&resolved);
    let root = resolved.parent().map(norm_path).unwrap_or_default();
    let src_fold = norm_path(&resolve(source_project_dir));
    let fold_of = |item: &Value| {
        let v = &item["draft_fold_path"];
        norm_fold(&if truthy(v) { str_of(v) } else { String::new() })
    };

    let mut template = store
        .iter()
        .find(|i| is_dict(i) && fold_of(i) == src_fold)
        .or_else(|| store.iter().find(|i| is_dict(i)))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let now_us = (now_unix() * 1_000_000.0) as i64;
    if let Some(t) = template.as_object_mut() {
        t.insert("draft_id", json!(meta_id));
        t.insert("draft_name", json!(folder_name));
        t.insert("draft_fold_path", json!(fold));
        t.insert("draft_root_path", json!(root));
        t.insert("tm_duration", int(duration_us));
        if t.contains_key("duration") {
            t.insert("duration", int(duration_us));
        }
        t.insert(
            "draft_json_file",
            json!(capcut_join(&fold, "draft_content.json")),
        );
        t.insert("draft_cover", json!(capcut_join(&fold, "draft_cover.jpg")));
        t.insert("draft_is_invisible", json!(false));
        t.insert("streaming_edit_draft_ready", json!(true));
        t.insert("tm_draft_removed", json!(0));
        t.insert("tm_draft_modified", int(now_us));
        if !truthy(t.get("tm_draft_create").unwrap_or(&NONE)) {
            t.insert("tm_draft_create", int(now_us));
        }
    }

    let mut new_store = Vec::new();
    let mut replaced = false;
    for item in store.iter().filter(|i| is_dict(i)) {
        let item_name = {
            let v = &item["draft_name"];
            if truthy(v) { str_of(v) } else { String::new() }
        };
        if fold_of(item) == fold || item_name == folder_name {
            if !replaced {
                new_store.push(template.clone());
                replaced = true;
            }
            continue;
        }
        new_store.push(item.clone());
    }
    if !replaced {
        new_store.insert(0, template);
    }
    let count = new_store.len();
    d.insert("all_draft_store", Value::from(new_store));
    let meta_v = json!(meta_id);
    match d.get_mut("draft_ids") {
        Some(Value::Array(ids)) => {
            if !ids.iter().any(|x| crate::pyval::eq(x, &meta_v)) {
                ids.insert(0, meta_v);
            }
        }
        _ => {
            d.insert("draft_ids", json!(count));
        }
    }
    let _ = write_json(&root_meta, &data);
}

/// Copy the project twice and write the time-sliced drafts.
///
/// `progress` receives `(0.0..=1.0, message)`.
///
/// # Errors
/// As the original: a missing project folder, a bad cut, file errors while
/// copying or renaming, and malformed drafts.
pub fn split_project(
    source_project_dir: &Path,
    source_draft: &Value,
    source_draft_path: &Path,
    cut_us: Option<i64>,
    caption_edges_us: Option<&[i64]>,
    output_parent: Option<&Path>,
    progress: &mut dyn FnMut(f64, &str),
) -> PyResult<SplitResult> {
    let source_project_dir = resolve(source_project_dir);
    if !source_project_dir.is_dir() {
        return Err(format!(
            "FileNotFoundError: Project directory not found: {}",
            source_project_dir.display()
        ));
    }
    let total = int_or0(get(source_draft, "duration")?)?;
    let mut edges: Vec<i64> = caption_edges_us.map(<[i64]>::to_vec).unwrap_or_default();
    if edges.is_empty() {
        edges = caption_edges_from_draft(source_draft)?;
    }
    let cut_us = match cut_us {
        Some(c) => c,
        None => choose_cut_us(total, &edges)?,
    };
    if cut_us <= 0 || cut_us >= total {
        return Err(format!(
            "ValueError: Invalid cut point {cut_us} for duration {total}"
        ));
    }

    let parent = output_parent.map(Path::to_path_buf).unwrap_or_else(|| {
        source_project_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    });
    let base = source_project_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let part1_dir = parent.join(format!("{base}_part1"));
    let part2_dir = parent.join(format!("{base}_part2"));
    let old_content_id = {
        let v = &source_draft["id"];
        if truthy(v) { str_of(v) } else { String::new() }
    };

    purge_stale_root_meta(&parent.join("root_meta_info.json"));
    for d in [&part1_dir, &part2_dir] {
        if d.exists() {
            std::fs::remove_dir_all(d).map_err(|e| io_err(&e, d))?;
        }
    }

    // Both copies, then both slices, run in parallel; ids are drawn in the
    // original's order (part 1, then part 2) before the parallel work.
    progress(0.05, "Copying part 1…");
    progress(0.30, "Copying part 2…");
    let (c1, c2) = rayon::join(
        || copy_project_tree(&source_project_dir, &part1_dir),
        || copy_project_tree(&source_project_dir, &part2_dir),
    );
    c1?;
    c2?;

    progress(0.50, "Slicing drafts (incl. nested)…");
    let id1 = new_uuid_upper();
    let id2 = new_uuid_upper();
    let (s1, s2) = rayon::join(
        || slice_draft(source_draft, cut_us, 1, "", Some(&id1), true),
        || slice_draft(source_draft, cut_us, 2, "", Some(&id2), true),
    );
    let (mut d1, n1) = s1?;
    set_name(&mut d1, &format!("{base}_part1"));
    let (mut d2, n2) = s2?;
    set_name(&mut d2, &format!("{base}_part2"));
    // `_write_draft_targets` blanks the name before writing; serialize both now, in parallel.
    set_name(&mut d1, "");
    set_name(&mut d2, "");
    let (payload1, payload2) = rayon::join(|| dumps_compact(&d1), || dumps_compact(&d2));

    progress(0.70, "Writing part 1…");
    let p1_draft = write_draft_targets(&part1_dir, &mut d1, &old_content_id, Some(&payload1))?;
    let name1 = format!("{base}_part1");
    let meta1 = patch_meta(&part1_dir, &d1, &name1);
    let dur1 = to_int(&d1["duration"])?;
    register_root_meta(&part1_dir, &meta1, &name1, dur1, &source_project_dir);

    progress(0.88, "Writing part 2…");
    let p2_draft = write_draft_targets(&part2_dir, &mut d2, &old_content_id, Some(&payload2))?;
    let name2 = format!("{base}_part2");
    let meta2 = patch_meta(&part2_dir, &d2, &name2);
    let dur2 = to_int(&d2["duration"])?;
    register_root_meta(&part2_dir, &meta2, &name2, dur2, &source_project_dir);

    let manifest = json!({
        "source_project": source_project_dir.to_string_lossy(),
        "source_draft": source_draft_path.to_string_lossy(),
        "cut_us": cut_us,
        "total_duration_us": total,
        "part1": {"dir": part1_dir.to_string_lossy(), "duration_us": d1["duration"].clone(), "segments": n1,
                  "content_id": d1["id"].clone(), "meta_id": meta1},
        "part2": {"dir": part2_dir.to_string_lossy(), "duration_us": d2["duration"].clone(), "segments": n2,
                  "content_id": d2["id"].clone(), "meta_id": meta2},
    });
    let manifest_text = text_mode(&dumps_indent(&manifest, 2));
    for d in [&part1_dir, &part2_dir] {
        let _ = std::fs::write(d.join("capdraft_split_manifest.json"), &manifest_text);
    }
    progress(1.0, "Split complete");
    Ok(SplitResult {
        cut_us,
        part1_dir,
        part2_dir,
        part1_draft: p1_draft,
        part2_draft: p2_draft,
        part1_duration_us: dur1,
        part2_duration_us: dur2,
        part1_segments: n1,
        part2_segments: n2,
    })
}

fn set_name(d: &mut Value, name: &str) {
    if let Some(o) = d.as_object_mut() {
        o.insert("name", Value::from(name));
    }
}
