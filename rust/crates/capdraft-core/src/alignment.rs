//! Native CapCut audio alignment: trim the TTS head through `source_timerange`
//! and add an `audio_fade` (port of `native_audio_alignment.py`). No re-encode.

use crate::ids::new_uuid_upper;
use crate::models::compute_target_duration_us;
use crate::pyfmt::round_half_even;
use crate::pyjson::{float, int};
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeAudioAlignmentSettings {
    pub enabled: bool,
    pub leading_trim_frames: f64,
    pub fade_in_ms: f64,
    pub minimum_source_duration_us: i64,
}

impl Default for NativeAudioAlignmentSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            leading_trim_frames: 3.0,
            fade_in_ms: 8.0,
            minimum_source_duration_us: 50_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeAlignmentResult {
    pub project_fps: f64,
    pub requested_trim_frames: f64,
    pub applied_trim_us: i64,
    pub raw_duration_us: i64,
    pub source_duration_us: i64,
    pub target_duration_us: i64,
    pub fade_in_us: i64,
    pub trim_reduced: bool,
}

/// FPS used when the project's is missing or not positive.
fn safe_fps(fps: Option<f64>) -> f64 {
    fps.filter(|f| *f > 0.0).unwrap_or(30.0)
}

#[must_use]
pub fn frames_to_us(frames: f64, fps: Option<f64>) -> i64 {
    round_half_even(frames / safe_fps(fps) * 1_000_000.0)
}

#[must_use]
pub fn ms_to_us(ms: f64) -> i64 {
    round_half_even(ms * 1000.0)
}

/// Mutates the segment, material and fade list in place. The material keeps
/// the full file duration; the segment never starts before the caption.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the original keyword arguments"
)]
pub fn apply_native_audio_alignment(
    audio_segment: &mut Map<String, Value>,
    audio_material: &mut Map<String, Value>,
    audio_fade_materials: &mut Vec<Value>,
    caption_start_us: i64,
    raw_duration_us: i64,
    project_fps: Option<f64>,
    clip_speed: Option<f64>,
    settings: &NativeAudioAlignmentSettings,
) -> NativeAlignmentResult {
    let fps = safe_fps(project_fps);
    let raw = raw_duration_us.max(0);
    audio_material.insert("duration".into(), int(raw));

    let requested_trim_us = if settings.enabled && settings.leading_trim_frames > 0.0 {
        frames_to_us(settings.leading_trim_frames, Some(fps))
    } else {
        0
    };
    let min_src = settings.minimum_source_duration_us.max(1_000);
    let safe_trim_us = requested_trim_us.min((raw - min_src).max(0));
    let trim_reduced = safe_trim_us < requested_trim_us && requested_trim_us > 0;

    let source_start_us = safe_trim_us;
    let source_duration_us = (raw - source_start_us).max(1_000);
    let speed = clip_speed.filter(|s| *s > 0.0).unwrap_or(1.0);
    let target_duration_us = compute_target_duration_us(source_duration_us, speed, fps);

    audio_segment.insert(
        "source_timerange".into(),
        json!({"start": source_start_us, "duration": source_duration_us}),
    );
    audio_segment.insert(
        "target_timerange".into(),
        json!({"start": caption_start_us, "duration": target_duration_us}),
    );
    audio_segment.insert("speed".into(), float(speed));

    let mut fade_in_us = 0;
    if settings.enabled && settings.fade_in_ms > 0.0 {
        fade_in_us = ms_to_us(settings.fade_in_ms).min((target_duration_us / 4).max(0));
        if fade_in_us > 0 {
            let fade_id = new_uuid_upper();
            audio_fade_materials.push(json!({
                "id": fade_id,
                "type": "audio_fade",
                "fade_type": 0,
                "fade_in_duration": fade_in_us,
                "fade_out_duration": 0,
            }));
            let mut refs: Vec<Value> = audio_segment
                .get("extra_material_refs")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if !refs.iter().any(|r| r.as_str() == Some(&fade_id)) {
                refs.push(Value::String(fade_id));
            }
            audio_segment.insert("extra_material_refs".into(), Value::Array(refs));
        }
    }

    NativeAlignmentResult {
        project_fps: fps,
        requested_trim_frames: settings.leading_trim_frames,
        applied_trim_us: source_start_us,
        raw_duration_us: raw,
        source_duration_us,
        target_duration_us,
        fade_in_us,
        trim_reduced,
    }
}
