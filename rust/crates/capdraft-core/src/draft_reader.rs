//! Read `draft_content.json`: captions, project info, TTS templates
//! (port of `core/capcut_project/draft_reader.py`).
//!
//! Python's loose reads are kept (ADR-011): each step fails where, and with
//! the message, the original raises.

use crate::json::{Object, Value};
use crate::jv as json;
use crate::models::{CapCutProjectInfo, CaptionRow};
use crate::pyfmt::py_strip;
use crate::pyval::{
    self as py, Key, NONE, PyResult, contains_str, get, int_or0, is_dict, is_str, iter,
    iter_or_empty, key, or, str_of, to_float, truthy,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static EMPTY_DICT: LazyLock<Value> = LazyLock::new(Value::empty_object);
static EMPTY_LIST: LazyLock<Value> = LazyLock::new(|| Value::Array(crate::json::Array::new()));
static BUILTIN_TEMPLATES: LazyLock<Value> = LazyLock::new(|| {
    // INFALLIBLE: embedded at compile time and checked by the replay tests.
    crate::json::parse(include_str!("builtin_tts_templates.json"))
        .expect("valid builtin_tts_templates.json")
});

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectInspectionResult {
    pub project_info: CapCutProjectInfo,
    pub valid_caption_count: usize,
    pub selected_caption_count: usize,
    pub skipped_empty_count: usize,
    pub existing_tts_count: usize,
    pub orphan_text_segment_count: usize,
    pub invalid_content_count: usize,
    pub warnings: Vec<String>,
    pub has_tts_template: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DraftReader {
    pub draft_path: Option<PathBuf>,
    pub project_directory: Option<PathBuf>,
    pub draft: Option<Value>,
    captions: Vec<CaptionRow>,
}

fn no_draft() -> String {
    "RuntimeError: No draft loaded".to_owned()
}

/// `draft_content.json` in a folder (or the first one under `Timelines/`), or the file itself.
///
/// # Errors
/// `FileNotFoundError` with the original's message.
pub fn resolve_draft_path(path: &Path) -> PyResult<PathBuf> {
    if path.is_dir() {
        let candidate = path.join("draft_content.json");
        if !candidate.exists() {
            let timelines = path.join("Timelines");
            if timelines.is_dir()
                && let Some(found) = find_first(&timelines, "draft_content.json")
            {
                return Ok(found);
            }
            return Err(format!(
                "FileNotFoundError: draft_content.json not found under {}",
                path.display()
            ));
        }
        return Ok(candidate);
    }
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    Err(format!(
        "FileNotFoundError: Path not found: {}",
        path.display()
    ))
}

/// `Path.rglob(name)`'s first hit: files of a folder before its subfolders,
/// entries in name order (NTFS order).
fn find_first(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    if let Some(hit) = entries
        .iter()
        .find(|p| p.is_file() && p.file_name().is_some_and(|n| n == name))
    {
        return Some(hit.clone());
    }
    entries
        .iter()
        .filter(|p| p.is_dir())
        .find_map(|p| find_first(p, name))
}

/// Read a JSON file the way `open(encoding="utf-8-sig")` + `json.load` does.
///
/// # Errors
/// `OSError`/`UnicodeDecodeError`/`JSONDecodeError`.
pub fn read_json_file(path: &Path) -> PyResult<Value> {
    let bytes = std::fs::read(path).map_err(|e| format!("OSError: {e}"))?;
    let text = String::from_utf8(bytes).map_err(|e| format!("UnicodeDecodeError: {e}"))?;
    crate::pyjson::loads(text.strip_prefix('\u{feff}').unwrap_or(&text))
}

/// `(text, content_ok)` of a caption material's `content`.
///
/// # Errors
/// `AttributeError` when `material` is not a dict.
pub fn parse_caption_text(material: &Value) -> PyResult<(String, bool)> {
    let raw = get(material, "content")?;
    match raw {
        Value::Null => Ok((String::new(), true)),
        Value::Object(_) => Ok((text_field(raw)?, true)),
        Value::String(s) => {
            if py_strip(s).is_empty() {
                return Ok((String::new(), true));
            }
            match crate::pyjson::loads(s) {
                Err(_) => Ok((s.to_string(), true)),
                Ok(content @ Value::Object(_)) => Ok((text_field(&content)?, true)),
                Ok(content) => Ok((str_of(&content), true)),
            }
        }
        _ => Ok((String::new(), false)),
    }
}

/// `str(d.get("text") or "")`.
fn text_field(d: &Value) -> PyResult<String> {
    let t = get(d, "text")?;
    Ok(if truthy(t) { str_of(t) } else { String::new() })
}

fn with_tts(captions: &[CaptionRow]) -> usize {
    captions.iter().filter(|c| c.has_existing_tts()).count()
}
fn empty_count(captions: &[CaptionRow]) -> usize {
    captions.iter().filter(|c| c.is_empty()).count()
}

impl DraftReader {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A reader over an already parsed draft (as `load_draft` leaves it).
    #[must_use]
    pub fn from_draft(draft: Value, draft_path: PathBuf) -> Self {
        let project_directory = project_directory_for(&draft_path);
        Self {
            draft_path: Some(draft_path),
            project_directory: Some(project_directory),
            draft: Some(draft),
            captions: vec![],
        }
    }

    /// # Errors
    /// File, JSON and shape errors with the original's messages.
    pub fn load_draft(&mut self, path: &Path) -> PyResult<&Value> {
        let draft_path = resolve_draft_path(path)?;
        let data = read_json_file(&draft_path)?;
        if !data.is_object() {
            return Err("ValueError: draft_content.json root must be an object".into());
        }
        if !contains_str(&data, "tracks")? || !contains_str(&data, "materials")? {
            return Err(
                "ValueError: draft_content.json missing required keys: tracks/materials".into(),
            );
        }
        let materials = or(get(&data, "materials")?, &EMPTY_DICT);
        if !contains_str(materials, "texts")? {
            return Err("ValueError: draft_content.json missing materials.texts".into());
        }
        self.project_directory = Some(project_directory_for(&draft_path));
        self.draft_path = Some(draft_path);
        self.draft = Some(data);
        self.captions = self.extract_captions()?;
        Ok(self.draft.as_ref().unwrap_or(&NONE))
    }

    /// # Errors
    /// `RuntimeError` when no draft is loaded.
    pub fn draft(&self) -> PyResult<&Value> {
        self.draft.as_ref().ok_or_else(no_draft)
    }

    /// # Errors
    /// `RuntimeError` when no draft is loaded.
    pub fn get_draft_copy(&self) -> PyResult<Value> {
        self.draft().cloned()
    }

    /// # Errors
    /// As the original: loose reads of the draft's fields.
    pub fn get_project_info(&mut self) -> PyResult<CapCutProjectInfo> {
        let (Some(draft_path), Some(project_directory)) =
            (self.draft_path.clone(), self.project_directory.clone())
        else {
            return Err(no_draft());
        };
        let d = self.draft.as_ref().ok_or_else(no_draft)?;
        let canvas = or(get(d, "canvas_config")?, &EMPTY_DICT);
        let tracks = or(get(d, "tracks")?, &EMPTY_LIST);
        let _materials = or(get(d, "materials")?, &EMPTY_DICT);

        let track_items = iter(tracks);
        let tracks_of = |kind: &str| -> PyResult<Vec<&Value>> {
            let mut out = Vec::new();
            for t in track_items.as_ref().map_err(Clone::clone)?.iter() {
                if is_str(get(t, "type")?, kind) {
                    out.push(t);
                }
            }
            Ok(out)
        };
        let video_tracks = tracks_of("video")?;
        let audio_tracks = tracks_of("audio")?;
        let text_tracks = tracks_of("text")?;
        let seg_count = |ts: &[&Value]| -> PyResult<usize> {
            let mut n = 0;
            for t in ts {
                n += py::len(or(get(t, "segments")?, &EMPTY_LIST))?;
            }
            Ok(n)
        };

        // `self._captions or self.extract_captions()`; the cache is stored once `d` is no longer borrowed.
        let fresh = if self.captions.is_empty() {
            Some(extract_captions(d)?)
        } else {
            None
        };
        let captions = fresh.as_ref().unwrap_or(&self.captions);
        let caption_with_tts_count = with_tts(captions);
        let empty_caption_count = empty_count(captions);
        let caption_count = captions.len();

        let name = get(d, "name")?;
        let project_name = if truthy(name) {
            str_of(name)
        } else {
            let dir_name = file_name(&project_directory);
            let parent_name = draft_path.parent().map(file_name).unwrap_or_default();
            if !dir_name.is_empty() {
                dir_name
            } else if !parent_name.is_empty() {
                parent_name
            } else {
                "Untitled".into()
            }
        };
        let project_id = str_of(or(get(d, "id")?, &Value::String(Default::default())));
        let version = int_or0(get(d, "version")?)?;
        let nv = get(d, "new_version")?;
        let new_version = if nv.is_null() { None } else { Some(str_of(nv)) };
        let width = int_or0(get(canvas, "width")?)?;
        let height = int_or0(get(canvas, "height")?)?;
        let fps_v = get(d, "fps")?;
        let fps = if truthy(fps_v) {
            to_float(fps_v)?
        } else {
            30.0
        };
        let duration_us = int_or0(get(d, "duration")?)?;
        let (video_segment_count, audio_segment_count, text_segment_count) = (
            seg_count(&video_tracks)?,
            seg_count(&audio_tracks)?,
            seg_count(&text_tracks)?,
        );
        if let Some(c) = fresh {
            self.captions = c;
        }
        Ok(CapCutProjectInfo {
            project_id,
            project_name,
            draft_path,
            project_directory,
            version,
            new_version,
            width,
            height,
            fps,
            duration_us,
            video_track_count: video_tracks.len(),
            audio_track_count: audio_tracks.len(),
            text_track_count: text_tracks.len(),
            video_segment_count,
            audio_segment_count,
            text_segment_count,
            caption_count,
            caption_with_tts_count,
            empty_caption_count,
        })
    }

    /// Subtitle captions in timeline order, re-indexed from 1.
    ///
    /// # Errors
    /// As the original: loose reads of tracks, segments and materials.
    pub fn extract_captions(&mut self) -> PyResult<Vec<CaptionRow>> {
        let rows = extract_captions(self.draft()?)?;
        self.captions.clone_from(&rows);
        Ok(rows)
    }

    /// # Errors
    /// As [`Self::extract_captions`].
    pub fn get_captions(&mut self) -> PyResult<Vec<CaptionRow>> {
        if self.captions.is_empty() && self.draft.is_some() {
            return self.extract_captions();
        }
        Ok(self.captions.clone())
    }

    /// Templates copied from the first TTS clip already in the draft.
    ///
    /// # Errors
    /// As the original: loose reads of materials and tracks.
    pub fn find_tts_templates(&self) -> PyResult<Option<Value>> {
        match &self.draft {
            None => Ok(None),
            Some(d) => find_tts_templates(d),
        }
    }

    /// # Errors
    /// As the original.
    pub fn find_speed_templates(&self) -> PyResult<Option<Value>> {
        let Some(d) = &self.draft else {
            return Ok(None);
        };
        let speeds = or(
            get(or(get(d, "materials")?, &EMPTY_DICT), "speeds")?,
            &EMPTY_LIST,
        );
        if !truthy(speeds) {
            return Ok(None);
        }
        if let Some(tpl) = find_tts_templates(d)?
            && let Some(s) = tpl.get("extras").and_then(|e| e.get("speeds"))
        {
            return Ok(Some(s.clone()));
        }
        py::index0(speeds).map(Some)
    }

    /// Fallback templates from a verified CapCut sample.
    #[must_use]
    pub fn builtin_tts_templates() -> Value {
        BUILTIN_TEMPLATES.clone()
    }

    /// # Errors
    /// As [`Self::find_tts_templates`].
    pub fn get_tts_templates(&self) -> PyResult<Value> {
        Ok(self
            .find_tts_templates()?
            .filter(truthy)
            .unwrap_or_else(Self::builtin_tts_templates))
    }

    /// Counts and warnings shown before generating.
    ///
    /// `selected_ids`: `None` selects every caption with text; otherwise
    /// segment ids or 1-based caption indexes as strings.
    ///
    /// # Errors
    /// As the original.
    pub fn inspect_project(
        &mut self,
        selected_ids: Option<&[String]>,
    ) -> PyResult<ProjectInspectionResult> {
        let info = self.get_project_info()?;
        let captions = self.get_captions()?;
        let empty = json!({});
        let d = self.draft.as_ref().unwrap_or(&empty);
        let mut warnings = Vec::new();
        let mut invalid_content = 0;
        let mut orphan_segments = 0;

        let materials = or(get(d, "materials")?, &EMPTY_DICT);
        let texts = or(get(materials, "texts")?, &EMPTY_LIST);
        let texts_items = iter(texts)?;
        let mut text_by_id: HashSet<Key> = HashSet::new();
        for t in texts_items.iter().filter(|t| is_dict(t)) {
            text_by_id.insert(key(get(t, "id")?)?);
        }
        let tracks = or(get(d, "tracks")?, &EMPTY_LIST);
        for track in iter(tracks)?.iter() {
            if !is_str(get(track, "type")?, "text") {
                continue;
            }
            for seg in iter_or_empty(get(track, "segments")?)?.iter() {
                if !text_by_id.contains(&key(get(seg, "material_id")?)?) {
                    orphan_segments += 1;
                }
            }
        }
        let mut all_seg_ids: HashSet<Key> = HashSet::new();
        for track in iter(tracks)?.iter() {
            for seg in iter_or_empty(get(track, "segments")?)?.iter() {
                let sid = get(seg, "id")?;
                if truthy(sid) {
                    all_seg_ids.insert(key(sid)?);
                }
            }
        }
        for t in texts_items.iter() {
            if !is_str(get(t, "type")?, "subtitle") {
                continue;
            }
            // `parse_caption_text(t)[1]`: false only for a content that is not None/dict/str.
            let ok = matches!(
                get(t, "content")?,
                Value::Null | Value::Object(_) | Value::String(_)
            );
            if !ok {
                invalid_content += 1;
            }
            for sid in iter_or_empty(get(t, "text_to_audio_ids")?)?.iter() {
                if !all_seg_ids.contains(&key(sid)?) {
                    warnings.push(format!("text_to_audio_ids orphan segment: {}", str_of(sid)));
                }
            }
        }
        let linked: HashSet<&str> = captions
            .iter()
            .map(|c| c.text_material_id.as_str())
            .collect();
        for t in texts_items.iter() {
            if is_str(get(t, "type")?, "subtitle") {
                let id = get(t, "id")?;
                let linked_hit = matches!(key(id)?, Key::Str(s) if linked.contains(s.as_str()));
                if !linked_hit {
                    warnings.push(format!("subtitle material without segment: {}", str_of(id)));
                }
            }
        }
        for a in iter_or_empty(get(materials, "audios")?)?.iter() {
            if is_str(get(a, "type")?, "text_to_audio") {
                let tid = get(a, "text_id")?;
                if truthy(tid) && !text_by_id.contains(&key(tid)?) {
                    warnings.push(format!(
                        "orphan TTS audio material text_id={} id={}",
                        str_of(tid),
                        str_of(get(a, "id")?)
                    ));
                }
            }
        }
        let mut seen: HashSet<Key> = HashSet::new();
        let mut dups = 0;
        for track in iter(tracks)?.iter() {
            for seg in iter_or_empty(get(track, "segments")?)?.iter() {
                let sid = get(seg, "id")?;
                if !truthy(sid) {
                    continue;
                }
                if !seen.insert(key(sid)?) {
                    dups += 1;
                }
            }
        }
        if dups > 0 {
            warnings.push(format!("duplicate segment UUIDs: {dups}"));
        }
        let has_tts_template = self.find_tts_templates()?.is_some();
        if !has_tts_template {
            warnings.push(
                "No existing TTS template in project; using built-in CapCut-verified template"
                    .into(),
            );
        }
        let valid = captions.iter().filter(|c| !c.is_empty()).count();
        let selected_caption_count = match selected_ids {
            None => valid,
            Some(ids) => {
                let set: HashSet<&str> = ids.iter().map(String::as_str).collect();
                captions
                    .iter()
                    .filter(|c| {
                        set.contains(c.text_segment_id.as_str())
                            || set.contains(c.index.to_string().as_str())
                    })
                    .count()
            }
        };
        Ok(ProjectInspectionResult {
            project_info: info,
            valid_caption_count: valid,
            selected_caption_count,
            skipped_empty_count: empty_count(&captions),
            existing_tts_count: with_tts(&captions),
            orphan_text_segment_count: orphan_segments,
            invalid_content_count: invalid_content,
            warnings,
            has_tts_template,
        })
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Under `Timelines/<id>/`, audio lives two levels up.
#[must_use]
pub fn project_directory_for(draft_path: &Path) -> PathBuf {
    let parent = draft_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    match parent.parent() {
        Some(gp) if gp.file_name().is_some_and(|n| n == "Timelines") => {
            gp.parent().map(Path::to_path_buf).unwrap_or_default()
        }
        _ => parent,
    }
}

/// # Errors
/// As [`DraftReader::extract_captions`].
pub fn extract_captions(draft: &Value) -> PyResult<Vec<CaptionRow>> {
    let materials = or(get(draft, "materials")?, &EMPTY_DICT);
    let texts = or(get(materials, "texts")?, &EMPTY_LIST);
    let mut text_by_id: HashMap<Key, &Value> = HashMap::new();
    let texts_items = iter(texts)?;
    for t in texts_items.iter() {
        if is_dict(t) && truthy(get(t, "id")?) {
            text_by_id.insert(key(get(t, "id")?)?, t);
        }
    }
    let tracks = or(get(draft, "tracks")?, &EMPTY_LIST);
    let mut all_segment_ids: HashSet<Key> = HashSet::new();
    for track in iter(tracks)?.iter() {
        for seg in iter_or_empty(get(track, "segments")?)?.iter() {
            let sid = get(seg, "id")?;
            if truthy(sid) {
                all_segment_ids.insert(key(sid)?);
            }
        }
    }

    let mut rows = Vec::new();
    // Caption texts are parsed after the walk, on all cores (parsing a dict
    // material's content cannot fail, so the order of errors is unchanged).
    let mut pending: Vec<&Value> = Vec::new();
    let mut index = 0;
    for track in iter(tracks)?.iter() {
        if !is_str(get(track, "type")?, "text") {
            continue;
        }
        let track_id = str_of(or(get(track, "id")?, &Value::String(Default::default())));
        for seg in iter_or_empty(get(track, "segments")?)?.iter() {
            let mat_id = get(seg, "material_id")?;
            let Some(material) = text_by_id.get(&key(mat_id)?).copied() else {
                continue;
            };
            if !truthy(material) || !is_str(get(material, "type")?, "subtitle") {
                continue;
            }
            index += 1;
            pending.push(material);
            let tr = or(get(seg, "target_timerange")?, &EMPTY_DICT);
            let start_us = int_or0(get(tr, "start")?)?;
            let duration_us = int_or0(get(tr, "duration")?)?;
            let tta = or(get(material, "text_to_audio_ids")?, &EMPTY_LIST);
            let existing = match tta {
                Value::Array(a) => a.iter().filter(|x| truthy(x)).map(str_of).collect(),
                _ => vec![],
            };
            let seg_id = get(seg, "id")?;
            let mat_own_id = get(material, "id")?;
            rows.push(CaptionRow {
                index,
                text_track_id: track_id.clone(),
                text_segment_id: str_of(or(seg_id, &Value::String(Default::default()))),
                text_material_id: str_of(or(mat_own_id, mat_id)),
                start_us,
                duration_us,
                text: String::new(),
                existing_tts_segment_ids: existing,
            });
        }
    }
    {
        use rayon::prelude::*;
        let texts: Vec<String> = pending
            .par_iter()
            .with_min_len(64)
            .map(|m| parse_caption_text(m).map(|(t, _)| t).unwrap_or_default())
            .collect();
        for (row, text) in rows.iter_mut().zip(texts) {
            row.text = text;
        }
    }
    rows.sort_by_key(|r| (r.start_us, r.index));
    for (i, row) in rows.iter_mut().enumerate() {
        row.index = i as i64 + 1;
    }
    Ok(rows)
}

/// # Errors
/// As [`DraftReader::find_tts_templates`].
pub fn find_tts_templates(draft: &Value) -> PyResult<Option<Value>> {
    let materials = or(get(draft, "materials")?, &EMPTY_DICT);
    let audios = or(get(materials, "audios")?, &EMPTY_LIST);
    let audio_items = iter(audios)?;
    let Some(mat) = audio_items
        .iter()
        .find(|a| is_dict(a) && is_str(&a["type"], "text_to_audio"))
    else {
        return Ok(None);
    };
    let mat_id = get(mat, "id")?;
    let mut segment: Option<&Value> = None;
    let mut track: Option<&Value> = None;
    let tracks = or(get(draft, "tracks")?, &EMPTY_LIST);
    let track_items = iter(tracks)?;
    for t in track_items.iter() {
        if !is_str(get(t, "type")?, "audio") {
            continue;
        }
        match or(get(t, "segments")?, &EMPTY_LIST) {
            Value::Array(segs) => {
                for s in segs {
                    if py::eq(get(s, "material_id")?, mat_id) {
                        segment = Some(s);
                        track = Some(t);
                        break;
                    }
                }
            }
            // A dict or string iterates as strings: `.get` fails on the first one.
            other => {
                if let Some(first) = iter(other)?.first() {
                    get(first, "material_id")?;
                }
            }
        }
        if segment.is_some_and(truthy) {
            break;
        }
    }
    let Some(segment) = segment.filter(|s| truthy(s)) else {
        return Ok(None);
    };

    let mut extras = Object::new();
    for r in iter_or_empty(get(segment, "extra_material_refs")?)?.iter() {
        for (bucket, items) in py::items(materials)? {
            let Value::Array(items) = items else { continue };
            if let Some(it) = items.iter().find(|it| is_dict(it) && py::eq(&it["id"], r)) {
                extras.insert(bucket.clone(), it.clone());
            }
        }
    }
    let mut audio_track = Object::new();
    if let Some(Value::Object(t)) = track {
        for (k, v) in t {
            if k != "segments" {
                audio_track.insert(k.clone(), v.clone());
            }
        }
    }
    Ok(Some(json!({
        "audio_material": mat.clone(),
        "audio_segment": segment.clone(),
        "audio_track": audio_track,
        "extras": extras,
    })))
}
