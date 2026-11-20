//! Domain models (port of `core/capcut_project/models.py`).

use crate::pyfmt::{py_strip, round_half_even};
use std::path::PathBuf;

/// UI-facing pitch behavior when the CapCut clip speed is not 1.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToneModifyMode {
    /// Pitch follows speed (`is_tone_modify = true`).
    FollowSpeed,
    /// Keep the original pitch (`is_tone_modify = false`).
    PreservePitch,
}

impl ToneModifyMode {
    pub const ALL: [Self; 2] = [Self::FollowSpeed, Self::PreservePitch];

    /// The enum value, as stored in config and shown to the UI.
    #[must_use]
    pub const fn value(self) -> &'static str {
        match self {
            Self::FollowSpeed => "follow_speed",
            Self::PreservePitch => "preserve_pitch",
        }
    }

    /// `coerce_tone_modify_mode`: accepts the value or the name, any case,
    /// surrounding whitespace ignored.
    ///
    /// # Errors
    /// `ValueError: Unknown tone mode: '<mode>'` for anything else.
    pub fn parse(mode: &str) -> Result<Self, String> {
        let key = py_strip(mode).to_lowercase();
        Self::ALL
            .into_iter()
            .find(|m| m.value() == key)
            .ok_or_else(|| format!("ValueError: Unknown tone mode: {}", py_str_repr(mode)))
    }

    /// `map_tone_mode_to_capcut_flag`: the single place mapping to CapCut's
    /// `is_tone_modify`.
    #[must_use]
    pub const fn capcut_flag(self) -> bool {
        matches!(self, Self::FollowSpeed)
    }
}

/// Python `repr(str)`, enough for error messages.
fn py_str_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c if (0x80..0xa1).contains(&(c as u32)) || c == '\u{ad}' => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Microseconds as `HH:MM:SS.mmm` (negative clamps to zero).
#[must_use]
pub fn format_duration_us(duration_us: i64) -> String {
    let total_ms = round_half_even(duration_us as f64 / 1000.0).max(0);
    let (hours, rem) = (total_ms / 3_600_000, total_ms % 3_600_000);
    let (minutes, rem) = (rem / 60_000, rem % 60_000);
    let (secs, ms) = (rem / 1000, rem % 1000);
    format!("{hours:02}:{minutes:02}:{secs:02}.{ms:03}")
}

#[must_use]
pub fn seconds_to_us(seconds: f64) -> i64 {
    round_half_even(seconds * 1_000_000.0).max(0)
}

#[must_use]
pub fn us_to_seconds(us: i64) -> f64 {
    us as f64 / 1_000_000.0
}

/// Timeline duration under CapCut clip speed: frame-snapped `source / speed`
/// (matched 710/903 samples from a real CapCut project; plain rounding 67/903).
#[must_use]
pub fn compute_target_duration_us(source_duration_us: i64, clip_speed: f64, fps: f64) -> i64 {
    let speed = if clip_speed > 0.0 { clip_speed } else { 1.0 };
    if (speed - 1.0).abs() < 1e-9 {
        return source_duration_us;
    }
    let raw = source_duration_us as f64 / speed;
    if fps > 0.0 {
        let frame_us = 1_000_000.0 / fps;
        let frames = round_half_even(raw / frame_us) as f64;
        return round_half_even(frames * frame_us).max(1_000);
    }
    round_half_even(raw).max(1_000)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VoiceOption {
    pub language_code: String,
    pub locale: String,
    pub voice_type: String,
    pub display_name: String,
    pub resource_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CaptionRow {
    pub index: i64,
    pub text_track_id: String,
    pub text_segment_id: String,
    pub text_material_id: String,
    pub start_us: i64,
    pub duration_us: i64,
    pub text: String,
    pub existing_tts_segment_ids: Vec<String>,
}

impl CaptionRow {
    #[must_use]
    pub const fn end_us(&self) -> i64 {
        self.start_us + self.duration_us
    }

    #[must_use]
    pub fn has_existing_tts(&self) -> bool {
        !self.existing_tts_segment_ids.is_empty()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        py_strip(&self.text).is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CapCutProjectInfo {
    pub project_id: String,
    pub project_name: String,
    pub draft_path: PathBuf,
    pub project_directory: PathBuf,
    pub version: i64,
    pub new_version: Option<String>,
    pub width: i64,
    pub height: i64,
    pub fps: f64,
    pub duration_us: i64,
    pub video_track_count: usize,
    pub audio_track_count: usize,
    pub text_track_count: usize,
    pub video_segment_count: usize,
    pub audio_segment_count: usize,
    pub text_segment_count: usize,
    pub caption_count: usize,
    pub caption_with_tts_count: usize,
    pub empty_caption_count: usize,
}

impl CapCutProjectInfo {
    #[must_use]
    pub fn duration_display(&self) -> String {
        format_duration_us(self.duration_us)
    }

    #[must_use]
    pub fn resolution(&self) -> String {
        format!("{}×{}", self.width, self.height)
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn tone_mode_error_uses_python_repr() {
        assert_eq!(
            ToneModifyMode::parse("x").unwrap_err(),
            "ValueError: Unknown tone mode: 'x'"
        );
        assert_eq!(
            ToneModifyMode::parse(" Preserve_Pitch "),
            Ok(ToneModifyMode::PreservePitch)
        );
    }
}

/// The outcome of one caption's TTS, as the pipeline and the patcher see it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CaptionTtsResult {
    pub caption_index: i64,
    pub text_material_id: String,
    pub text_segment_id: String,
    pub start_us: i64,
    pub audio_path: String,
    pub source_duration_us: i64,
    pub target_duration_us: i64,
    /// `generated`, `cached`, `failed` or `skipped`.
    pub status: String,
    pub error: String,
    pub from_cache: bool,
}

impl CaptionTtsResult {
    /// `generated` or `cached`: audio exists and can be attached.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.status == "generated" || self.status == "cached"
    }
}

/// One log line of a generation run, as the UI receives it.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerationLogEvent {
    pub timestamp: chrono::NaiveDateTime,
    /// `DEBUG`, `INFO`, `WARNING`, `ERROR`, `SUCCESS` or `CACHED`.
    pub level: String,
    pub message: String,
    pub stage: Option<String>,
    pub caption_index: Option<i64>,
    pub progress: Option<f64>,
}

/// The outcome of `generate_and_attach`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GenerationResult {
    pub success: bool,
    pub project_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub selected: i64,
    pub generated: i64,
    pub cached: i64,
    pub skipped: i64,
    pub failed: i64,
    pub replaced: i64,
    pub attached: i64,
    pub audio_tracks_used: i64,
    pub audio_tracks_created: i64,
    pub processing_seconds: f64,
    pub validation_passed: bool,
    pub log_file: Option<PathBuf>,
    pub manifest_path: Option<PathBuf>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub items: Vec<CaptionTtsResult>,
    /// `{"cancelled": true}` when the user cancelled.
    pub cancelled: bool,
}

impl GenerationResult {
    #[must_use]
    pub fn completed_with_warnings(&self) -> bool {
        self.success && (self.failed > 0 || !self.warnings.is_empty())
    }
}
