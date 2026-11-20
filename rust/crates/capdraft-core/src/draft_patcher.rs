//! Attach generated TTS audio to a draft in memory
//! (port of `core/capcut_project/draft_patcher.py`).
//!
//! Per caption: a `text_to_audio` material, an audio segment, a speed material
//! and (with native alignment) an `audio_fade`, then greedy interval
//! partitioning into non-overlapping audio tracks.
//!
//! Templates are cloned copy-on-write, so building thousands of materials only
//! copies the fields that change.

use crate::alignment::{NativeAudioAlignmentSettings, apply_native_audio_alignment};
use crate::ids::new_uuid_upper;
use crate::json::{Array, Object, Value};
use crate::jv;
use crate::models::{CaptionRow, CaptionTtsResult, ToneModifyMode, seconds_to_us};
use crate::pyval::{
    Key, PyResult, eq, get, int_or0, is_dict, is_str, iter_or_empty, key, no_attr, or, to_float,
    truthy,
};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

pub const SAFE_TTS_VOICE_TYPE: &str = "BV074_streaming";
pub const SAFE_TTS_RESOURCE_ID: &str = "7102355709945188865";
pub const SAFE_TTS_DISPLAY_NAME: &str = "C\u{f4} G\u{e1}i Ho\u{1ea1}t Ng\u{f4}n";

const SAFE_TTS_BLANK_FIELDS: [&str; 18] = [
    "third_resource_id",
    "tone_effect_id",
    "tone_category_id",
    "tone_category_name",
    "tone_second_category_id",
    "tone_second_category_name",
    "tone_emotion_name_key",
    "tone_emotion_style",
    "tone_emotion_role",
    "tone_emotion_selection",
    "moyin_emotion",
    "request_id",
    "query",
    "search_id",
    "tts_task_id",
    "aigc_history_id",
    "aigc_item_id",
    "cloned_model_type",
];

/// Buckets cleaned when old TTS is removed.
pub const EXTRA_BUCKETS: [&str; 6] = [
    "speeds",
    "placeholder_infos",
    "beats",
    "sound_channel_mappings",
    "vocal_separations",
    "audio_fades",
];

/// Material fields emptied on every new TTS material (heavy, rewritten by CapCut).
const HEAVY_KEYS: [&str; 7] = [
    "intensifies_path",
    "aigc_history_id",
    "aigc_item_id",
    "request_id",
    "query",
    "search_id",
    "tts_task_id",
];

/// Segment fields dropped to keep 5k-caption drafts lean.
const DROP_KEYS: [&str; 7] = [
    "common_keyframes",
    "keyframe_refs",
    "lyric_keyframes",
    "caption_info",
    "hdr_settings",
    "clip",
    "uniform_scale",
];

static BENEFIT_INFO: LazyLock<Value> = LazyLock::new(
    || jv!({"benefit_type": "none", "benefit_log_id": "", "benefit_log_extra": "", "benefit_amount": -1}),
);
static DEFAULT_TRACK: LazyLock<Value> = LazyLock::new(
    || jv!({"id": "", "type": "audio", "flag": 0, "attribute": 0, "name": "", "is_default_name": true, "segments": []}),
);
static DEFAULT_SPEED: LazyLock<Value> = LazyLock::new(
    || jv!({"id": "", "type": "speed", "mode": 0, "speed": 1.0, "curve_speed": null}),
);
static EMPTY_DICT: LazyLock<Value> = LazyLock::new(Value::empty_object);
static EMPTY_LIST: LazyLock<Value> = LazyLock::new(|| Value::Array(Array::new()));

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PatchStats {
    pub materials_created: i64,
    pub segments_created: i64,
    pub speeds_created: i64,
    pub fades_created: i64,
    pub alignment_applied: i64,
    pub alignment_trim_reduced: i64,
    pub replaced: i64,
    pub skipped: i64,
    pub tracks_used: i64,
    pub tracks_created: i64,
    pub orphans_removed: i64,
    pub created_ids: Vec<String>,
    pub removed_ids: Vec<String>,
}

/// What the user chose for this run.
#[derive(Debug, Clone)]
pub struct PatchSettings<'a> {
    pub voice_type: &'a str,
    pub resource_id: &'a str,
    pub voice_display_name: &'a str,
    pub capcut_clip_speed: f64,
    pub tone_modify_mode: ToneModifyMode,
    /// `replace_existing`, `skip_existing` or `selected_only`.
    pub existing_tts_mode: &'a str,
}

/// `(material, segment, extras by bucket, alignment)` for one caption.
type TtsObjects = (
    Value,
    Value,
    Vec<(&'static str, Value)>,
    crate::alignment::NativeAlignmentResult,
);

#[derive(Debug, Clone)]
pub struct DraftPatcher {
    mat_tpl: Value,
    seg_tpl: Value,
    extras_tpl: Value,
    track_tpl: Value,
    fps: f64,
    alignment: NativeAudioAlignmentSettings,
}

/// `obj[key] = value` on a value that must be a dict.
fn set(obj: &mut Value, k: &str, v: Value) -> PyResult<()> {
    match obj.as_object_mut() {
        Some(o) => {
            o.insert(k, v);
            Ok(())
        }
        None => Err(format!(
            "TypeError: '{}' object does not support item assignment",
            crate::pyval::type_name(obj)
        )),
    }
}

/// `d.setdefault(key, default)` for a dict `d`.
fn setdefault<'a>(d: &'a mut Value, k: &str, default: Value) -> PyResult<&'a mut Value> {
    let o = d
        .as_object_mut()
        .ok_or_else(|| no_attr(&Value::Null, "setdefault"))?;
    if !o.contains_key(k) {
        o.insert(k, default);
    }
    o.get_mut(k).ok_or_else(|| "KeyError".to_owned())
}

/// `lst.append(v)` for a value that must be a list.
fn append(lst: &mut Value, v: Value) -> PyResult<()> {
    match lst {
        Value::Array(a) => {
            a.push(v);
            Ok(())
        }
        other => Err(no_attr(other, "append")),
    }
}

/// The value as an owned `str` id (`str(x)` of a value known to be a string).
fn sid(v: &Value) -> String {
    crate::pyval::str_of(v)
}

impl DraftPatcher {
    #[must_use]
    pub fn new(templates: &Value, fps: f64, alignment: NativeAudioAlignmentSettings) -> Self {
        let pick = |k: &str, default: &Value| or(&templates[k], default).clone();
        Self {
            mat_tpl: pick("audio_material", &EMPTY_DICT),
            seg_tpl: pick("audio_segment", &EMPTY_DICT),
            extras_tpl: pick("extras", &EMPTY_DICT),
            track_tpl: pick("audio_track", &DEFAULT_TRACK),
            fps: if fps == 0.0 { 30.0 } else { fps },
            alignment,
        }
    }

    /// Attach every successful result to `draft` (mutated in place).
    ///
    /// `captions` are the reader's rows; replaced captions lose their
    /// `existing_tts_segment_ids`. `results` get the aligned durations.
    ///
    /// # Errors
    /// Where the original raises on a malformed draft or template.
    pub fn patch(
        &self,
        draft: &mut Value,
        captions: &mut [CaptionRow],
        results: &mut [CaptionTtsResult],
        settings: &PatchSettings<'_>,
        audio_rel_paths: &HashMap<i64, String>,
    ) -> PyResult<PatchStats> {
        let mut stats = PatchStats::default();
        {
            let materials = setdefault(draft, "materials", Value::empty_object())?;
            for bucket in EXTRA_BUCKETS.iter().chain(&["audios"]) {
                setdefault(materials, bucket, Value::Array(Array::new()))?;
            }
        }
        let caption_by_index: HashMap<i64, usize> = captions
            .iter()
            .enumerate()
            .map(|(i, c)| (c.index, i))
            .collect();

        // text material id -> index in materials.texts (last one wins, as in a dict)
        let mut text_by_id: HashMap<Key, usize> = HashMap::new();
        for (i, t) in iter_or_empty(&draft["materials"]["texts"])?
            .iter()
            .enumerate()
        {
            if is_dict(t) && truthy(&t["id"]) {
                text_by_id.insert(key(&t["id"])?, i);
            }
        }

        let replace = settings.existing_tts_mode == "replace_existing";
        if replace {
            let to_replace: Vec<usize> = results
                .iter()
                .filter_map(|r| {
                    caption_by_index
                        .get(&r.caption_index)
                        .copied()
                        .filter(|&c| captions[c].has_existing_tts() && r.is_success())
                })
                .collect();
            stats.orphans_removed = remove_existing_tts(draft, captions, &to_replace)?;
            stats.replaced = to_replace.len() as i64;
        }

        let mut attach: Vec<(usize, usize, String)> = Vec::new();
        for (ri, r) in results.iter().enumerate() {
            let Some(&ci) = caption_by_index.get(&r.caption_index) else {
                continue;
            };
            if !r.is_success() {
                if r.status == "skipped" {
                    stats.skipped += 1;
                }
                continue;
            }
            if settings.existing_tts_mode == "skip_existing" && captions[ci].has_existing_tts() {
                stats.skipped += 1;
                continue;
            }
            let Some(rel) = audio_rel_paths
                .get(&r.caption_index)
                .filter(|p| !p.is_empty())
            else {
                continue;
            };
            attach.push((ci, ri, rel.clone()));
        }

        let is_tone = settings.tone_modify_mode.capcut_flag();
        let clip_speed = if settings.capcut_clip_speed > 0.0 {
            settings.capcut_clip_speed
        } else {
            1.0
        };
        let mut pending: Vec<Value> = Vec::with_capacity(attach.len());

        for (ci, ri, rel) in attach {
            let cap = &captions[ci];
            let (mat, seg, extras, align) = self.build_tts_objects(
                cap,
                &results[ri],
                &rel,
                settings.voice_display_name,
                clip_speed,
                is_tone,
            )?;
            let mat_id = sid(&mat["id"]);
            let seg_id = sid(&seg["id"]);
            {
                let materials = draft
                    .get_mut("materials")
                    .ok_or_else(|| "KeyError: 'materials'".to_owned())?;
                append(
                    materials
                        .get_mut("audios")
                        .ok_or_else(|| "KeyError: 'audios'".to_owned())?,
                    mat,
                )?;
                stats.materials_created += 1;
                stats.created_ids.push(mat_id);
                for (bucket, obj) in extras {
                    let id = sid(&obj["id"]);
                    append(
                        setdefault(materials, bucket, Value::Array(Array::new()))?,
                        obj,
                    )?;
                    stats.created_ids.push(id);
                    if bucket == "speeds" {
                        stats.speeds_created += 1;
                    }
                    if bucket == "audio_fades" {
                        stats.fades_created += 1;
                    }
                }
            }
            stats.alignment_applied += 1;
            if align.trim_reduced {
                stats.alignment_trim_reduced += 1;
            }
            results[ri].source_duration_us = align.source_duration_us;
            results[ri].target_duration_us = align.target_duration_us;

            if let Some(&ti) = text_by_id.get(&Key::Str(cap.text_material_id.clone())) {
                let texts = draft
                    .get_mut("materials")
                    .and_then(|m| m.get_mut("texts"))
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| "KeyError: 'texts'".to_owned())?;
                let text_mat = &mut texts[ti];
                let new_ids: Vec<Value> = if replace {
                    vec![Value::from(seg_id.as_str())]
                } else {
                    let mut ids: Vec<Value> =
                        iter_or_empty(&text_mat["text_to_audio_ids"])?.to_vec();
                    let sv = Value::from(seg_id.as_str());
                    if !ids.iter().any(|x| eq(x, &sv)) {
                        ids.push(sv);
                    }
                    ids
                };
                set(text_mat, "text_to_audio_ids", Value::from(new_ids))?;
            }
            pending.push(seg);
            stats.segments_created += 1;
            stats.created_ids.push(seg_id);
        }

        if let Some(Value::Array(audios)) =
            draft.get_mut("materials").and_then(|m| m.get_mut("audios"))
        {
            for mat in audios.iter_mut() {
                let needs = mat.is_object()
                    && is_str(&mat["type"], "text_to_audio")
                    && !(is_str(&mat["resource_id"], SAFE_TTS_RESOURCE_ID)
                        && is_str(&mat["tone_speaker"], SAFE_TTS_VOICE_TYPE)
                        && is_str(&mat["mock_tone_speaker"], SAFE_TTS_VOICE_TYPE)
                        && is_str(&mat["copyright_limit_type"], "none"));
                if needs && let Some(o) = mat.as_object_mut() {
                    apply_safe_tts_voice(o, None);
                }
            }
        }

        let max_end = pending.iter().try_fold(0i64, |acc, seg| -> PyResult<i64> {
            let tgt = or(&seg["target_timerange"], &EMPTY_DICT);
            Ok(acc.max(int_or0(get(tgt, "start")?)? + int_or0(get(tgt, "duration")?)?))
        })?;
        let (created, used) = self.assign_tracks(draft, pending)?;
        stats.tracks_created = created;
        stats.tracks_used = used;
        if max_end > int_or0(&draft["duration"])? {
            set(draft, "duration", Value::from(max_end))?;
        }
        Ok(stats)
    }

    fn build_tts_objects(
        &self,
        caption: &CaptionRow,
        result: &CaptionTtsResult,
        rel_path: &str,
        voice_display_name: &str,
        clip_speed: f64,
        is_tone_modify: bool,
    ) -> PyResult<TtsObjects> {
        let mut mat = self.mat_tpl.clone();
        let mut seg = self.seg_tpl.clone();
        let mat_id = new_uuid_upper();
        let seg_id = new_uuid_upper();
        let raw_us = if result.source_duration_us <= 0 {
            seconds_to_us(0.1)
        } else {
            result.source_duration_us
        };

        let name_src = [caption.text.as_str(), voice_display_name, "TTS"]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("TTS");
        let name: String = name_src.chars().take(40).collect();
        {
            let m = mat
                .as_object_mut()
                .ok_or_else(|| no_attr(&self.mat_tpl, "__setitem__"))?;
            m.insert("id", Value::from(mat_id.as_str()));
            m.insert("unique_id", Value::from(mat_id.as_str()));
            m.insert("duration", Value::from(raw_us));
            m.insert("path", Value::from(rel_path));
            m.insert("name", Value::from(name));
            m.insert("local_material_id", Value::from(mat_id.as_str()));
            m.insert("music_id", Value::from(mat_id.as_str()));
            m.insert("wave_points", Value::Array(Array::new()));
            for k in HEAVY_KEYS {
                if m.contains_key(k) {
                    m.insert(k, Value::from(""));
                }
            }
            apply_safe_tts_voice(m, Some(&caption.text_material_id));
        }

        let mut extras: Vec<(&'static str, Value)> = Vec::new();
        let mut extra_ids: Vec<Value> = Vec::new();
        {
            let tpl = get(&self.extras_tpl, "speeds")?;
            let mut obj = if tpl.is_null() {
                DEFAULT_SPEED.clone()
            } else {
                tpl.clone()
            };
            let id = new_uuid_upper();
            let mode = int_or0(&obj["mode"])?;
            let o = obj
                .as_object_mut()
                .ok_or_else(|| no_attr(tpl, "__setitem__"))?;
            o.insert("id", Value::from(id.as_str()));
            o.insert("type", Value::from("speed"));
            o.insert("speed", Value::from(clip_speed));
            o.insert("mode", Value::from(mode));
            o.insert("curve_speed", Value::Null);
            extra_ids.push(Value::from(id));
            extras.push(("speeds", obj));
        }

        let volume = {
            let v = &seg["volume"];
            if v.is_null() { 1.0 } else { to_float(v)? }
        };
        let s = seg
            .as_object_mut()
            .ok_or_else(|| no_attr(&self.seg_tpl, "__setitem__"))?;
        s.insert("id", Value::from(seg_id.as_str()));
        s.insert("material_id", Value::from(mat_id.as_str()));
        s.insert("render_timerange", jv!({"start": 0, "duration": 0}));
        s.insert("is_tone_modify", Value::from(is_tone_modify));
        s.insert("extra_material_refs", Value::from(extra_ids));
        s.insert("render_index", Value::from(0));
        s.insert("volume", Value::from(volume));
        s.insert("visible", Value::from(true));
        for k in DROP_KEYS {
            if s.contains_key(k) {
                let empty = if k.ends_with("refs") || k.ends_with("keyframes") {
                    Value::Array(Array::new())
                } else {
                    Value::Null
                };
                s.insert(k, empty);
            }
        }

        let mut fades: Vec<Value> = Vec::new();
        let m = mat
            .as_object_mut()
            .ok_or_else(|| no_attr(&self.mat_tpl, "__setitem__"))?;
        let align = apply_native_audio_alignment(
            s,
            m,
            &mut fades,
            caption.start_us,
            raw_us,
            Some(self.fps),
            Some(clip_speed),
            &self.alignment,
        );
        extras.extend(fades.into_iter().map(|f| ("audio_fades", f)));
        let seg_speed = {
            let v = &seg["speed"];
            if truthy(v) { to_float(v)? } else { clip_speed }
        };
        for (bucket, obj) in &mut extras {
            if *bucket == "speeds" {
                set(obj, "speed", Value::from(seg_speed))?;
            }
        }
        Ok((mat, seg, extras, align))
    }

    /// Greedy interval partitioning of the new segments into audio tracks
    /// (earliest-ending free lane first). Returns `(tracks_created, tracks_used)`.
    fn assign_tracks(&self, draft: &mut Value, segments: Vec<Value>) -> PyResult<(i64, i64)> {
        if segments.is_empty() {
            return Ok((0, 0));
        }
        let mut keyed: Vec<(i64, i64, Value)> = Vec::with_capacity(segments.len());
        for seg in segments {
            let tgt = or(&seg["target_timerange"], &EMPTY_DICT);
            let start = int_or0(get(tgt, "start")?)?;
            let end = start + int_or0(get(tgt, "duration")?)?;
            keyed.push((start, end, seg));
        }
        keyed.sort_by_key(|k| k.0);

        let tracks = setdefault(draft, "tracks", Value::Array(Array::new()))?;
        let Value::Array(tracks) = tracks else {
            return Err(no_attr(tracks, "append"));
        };
        let mut free: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for (i, t) in tracks.iter().enumerate() {
            if is_str(get(t, "type")?, "audio") && !truthy(&t["segments"]) {
                free.push_back(i);
            }
        }

        // lane = (track index, last end, segments placed)
        let mut lanes: Vec<(usize, i64, Vec<Value>)> = Vec::new();
        let mut created = 0;
        for (start, end, seg) in keyed {
            let mut placed: Option<usize> = None;
            for (li, lane) in lanes.iter().enumerate() {
                if lane.1 <= start && placed.is_none_or(|p| lane.1 < lanes[p].1) {
                    placed = Some(li);
                }
            }
            let li = match placed {
                Some(li) => li,
                None => {
                    let ti = if let Some(ti) = free.pop_front() {
                        ti
                    } else {
                        let mut tpl = self.track_tpl.clone();
                        let o = tpl
                            .as_object_mut()
                            .ok_or_else(|| no_attr(&self.track_tpl, "__setitem__"))?;
                        o.insert("id", Value::from(new_uuid_upper()));
                        o.insert("type", Value::from("audio"));
                        o.insert("segments", Value::Array(Array::new()));
                        o.insert("name", Value::from(""));
                        o.insert("is_default_name", Value::from(true));
                        tracks.push(tpl);
                        created += 1;
                        tracks.len() - 1
                    };
                    lanes.push((ti, 0, Vec::new()));
                    lanes.len() - 1
                }
            };
            lanes[li].2.push(seg);
            lanes[li].1 = end;
        }

        let mut non_audio = 0;
        for t in tracks.iter() {
            if !is_str(get(t, "type")?, "audio") {
                non_audio += 1;
            }
        }
        let base_index = non_audio.max(1) as i64;
        let used = lanes.len() as i64;
        for (i, (ti, _, segs)) in lanes.into_iter().enumerate() {
            let track = &mut tracks[ti];
            let o = track
                .as_object_mut()
                .ok_or_else(|| no_attr(&Value::Null, "setdefault"))?;
            if !o.contains_key("segments") {
                o.insert("segments", Value::Array(Array::new()));
            }
            let list = o
                .get_mut("segments")
                .ok_or_else(|| "KeyError: 'segments'".to_owned())?;
            for mut seg in segs {
                if let Some(s) = seg.as_object_mut() {
                    s.insert("track_render_index", Value::from(base_index + i as i64));
                    s.insert("render_index", Value::from(0));
                }
                append(list, seg)?;
            }
        }
        Ok((created, used))
    }
}

/// Point a TTS material at the one voice verified to export from CapCut.
fn apply_safe_tts_voice(mat: &mut Object, text_id: Option<&str>) {
    mat.insert("type", Value::from("text_to_audio"));
    if let Some(t) = text_id {
        mat.insert("text_id", Value::from(t));
    }
    mat.insert("resource_id", Value::from(SAFE_TTS_RESOURCE_ID));
    mat.insert("tone_speaker", Value::from(SAFE_TTS_VOICE_TYPE));
    mat.insert("mock_tone_speaker", Value::from(SAFE_TTS_VOICE_TYPE));
    mat.insert("tone_type", Value::from(SAFE_TTS_DISPLAY_NAME));
    mat.insert("tone_effect_name", Value::from(SAFE_TTS_DISPLAY_NAME));
    mat.insert("tone_platform", Value::from("sami"));
    mat.insert("tone_emotion_scale", Value::from(0.0));
    mat.insert("is_ai_clone_tone", Value::from(false));
    mat.insert("is_ai_clone_tone_post", Value::from(false));
    mat.insert("copyright_limit_type", Value::from("none"));
    let scene = mat
        .get("tts_generate_scene")
        .filter(|v| truthy(v))
        .cloned()
        .unwrap_or_else(|| Value::from("audio_panel"));
    mat.insert("tts_generate_scene", scene);
    mat.insert("tts_benefit_info", BENEFIT_INFO.clone());
    for k in SAFE_TTS_BLANK_FIELDS {
        mat.insert(k, Value::from(""));
    }
}

/// Remove the old TTS segments, materials and unshared extras of `caps`.
/// Returns the number of objects removed (materials, extras, empty audio tracks).
fn remove_existing_tts(
    draft: &mut Value,
    captions: &mut [CaptionRow],
    caps: &[usize],
) -> PyResult<i64> {
    let segment_ids: HashSet<&str> = caps
        .iter()
        .flat_map(|&c| {
            captions[c]
                .existing_tts_segment_ids
                .iter()
                .map(String::as_str)
        })
        .collect();
    if segment_ids.is_empty() {
        return Ok(0);
    }
    let segment_ids: HashSet<String> = segment_ids.into_iter().map(str::to_owned).collect();
    let mut material_ids: HashSet<Key> = HashSet::new();
    let mut extra_ids: HashSet<Key> = HashSet::new();
    let mut remaining: HashSet<Key> = HashSet::new();

    let mut tracks: Vec<Value> = iter_or_empty(&draft["tracks"])?.to_vec();
    for track in &mut tracks {
        if !is_str(get(track, "type")?, "audio") {
            continue;
        }
        let mut keep = Vec::new();
        for seg in iter_or_empty(get(track, "segments")?)?.iter() {
            let id = get(seg, "id")?;
            if matches!(key(id)?, Key::Str(s) if segment_ids.contains(&s)) {
                if truthy(&seg["material_id"]) {
                    material_ids.insert(key(&seg["material_id"])?);
                }
                for r in iter_or_empty(&seg["extra_material_refs"])?.iter() {
                    extra_ids.insert(key(r)?);
                }
            } else {
                for r in iter_or_empty(&seg["extra_material_refs"])?.iter() {
                    remaining.insert(key(r)?);
                }
                keep.push(seg.clone());
            }
        }
        set(track, "segments", Value::from(keep))?;
    }
    let deletable: HashSet<&Key> = extra_ids.difference(&remaining).collect();

    let mut removed = 0i64;
    {
        let materials = setdefault(draft, "materials", Value::empty_object())?;
        let o = materials
            .as_object_mut()
            .ok_or_else(|| no_attr(&Value::Null, "keys"))?;
        let buckets: Vec<String> = o.keys().map(ToString::to_string).collect();
        for bucket in buckets {
            let Some(Value::Array(items)) = o.get(&bucket) else {
                continue;
            };
            let is_audios = bucket == "audios";
            if !is_audios && !EXTRA_BUCKETS.contains(&bucket.as_str()) {
                continue;
            }
            let mut kept = Vec::with_capacity(items.len());
            for it in items.iter() {
                let k = key(get(it, "id")?)?;
                let gone = if is_audios {
                    material_ids.contains(&k)
                } else {
                    deletable.contains(&k)
                };
                if !gone {
                    kept.push(it.clone());
                }
            }
            removed += (items.len() - kept.len()) as i64;
            o.insert(bucket.as_str(), Value::from(kept));
        }

        let mut text_by_id: HashMap<Key, usize> = HashMap::new();
        for (i, t) in iter_or_empty(&o.get("texts").cloned().unwrap_or_default())?
            .iter()
            .enumerate()
        {
            if is_dict(t) {
                text_by_id.insert(key(&t["id"])?, i);
            }
        }
        for &c in caps {
            if let Some(&ti) = text_by_id.get(&Key::Str(captions[c].text_material_id.clone()))
                && let Some(Value::Array(texts)) = o.get_mut("texts")
            {
                set(
                    &mut texts[ti],
                    "text_to_audio_ids",
                    Value::Array(Array::new()),
                )?;
            }
            captions[c].existing_tts_segment_ids.clear();
        }
    }

    let mut new_tracks = Vec::with_capacity(tracks.len());
    for t in tracks {
        if is_str(get(&t, "type")?, "audio") && !truthy(or(&t["segments"], &EMPTY_LIST)) {
            removed += 1;
            continue;
        }
        new_tracks.push(t);
    }
    set(draft, "tracks", Value::from(new_tracks))?;
    Ok(removed)
}
