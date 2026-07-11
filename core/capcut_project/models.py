"""Domain models for CapCut Project TTS."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from enum import Enum
from pathlib import Path
from typing import Any


class ToneModifyMode(str, Enum):
    """UI-facing tone behavior when CapCut clip speed != 1.0.

    Mapping to CapCut's `is_tone_modify` is isolated in
    `map_tone_mode_to_capcut_flag`.

    CapCut treats `is_tone_modify=True` as enabling pitch changes with speed.
    False keeps the original pitch when clip speed changes.
    """

    FOLLOW_SPEED = "follow_speed"
    PRESERVE_PITCH = "preserve_pitch"


_TONE_FLAG_MAP: dict[ToneModifyMode, bool] = {
    ToneModifyMode.FOLLOW_SPEED: True,
    ToneModifyMode.PRESERVE_PITCH: False,
}

TONE_MODIFY_MAPPING_VERIFIED = True


def coerce_tone_modify_mode(mode: ToneModifyMode | str) -> ToneModifyMode:
    """Accept enum or string from UI/QComboBox."""
    if isinstance(mode, ToneModifyMode):
        return mode
    if isinstance(mode, str):
        key = mode.strip().lower()
        # allow both enum value and name
        for m in ToneModifyMode:
            if key in {m.value, m.name.lower()}:
                return m
    raise ValueError(f"Unknown tone mode: {mode!r}")


def map_tone_mode_to_capcut_flag(mode: ToneModifyMode | str) -> bool:
    """Single choke-point for UI tone mode -> CapCut `is_tone_modify`."""
    mode = coerce_tone_modify_mode(mode)
    return _TONE_FLAG_MAP[mode]


def format_duration_us(duration_us: int) -> str:
    """Format microseconds as HH:MM:SS.mmm."""
    total_ms = max(0, int(round(duration_us / 1000.0)))
    hours, rem = divmod(total_ms, 3_600_000)
    minutes, rem = divmod(rem, 60_000)
    secs, ms = divmod(rem, 1000)
    return f"{hours:02d}:{minutes:02d}:{secs:02d}.{ms:03d}"


def seconds_to_us(seconds: float) -> int:
    return max(0, int(round(float(seconds) * 1_000_000)))


def us_to_seconds(us: int) -> float:
    return float(us) / 1_000_000.0


def compute_target_duration_us(
    source_duration_us: int,
    clip_speed: float,
    *,
    fps: float = 30.0,
) -> int:
    """Timeline duration under CapCut clip speed.

    Verified from CapCut sample (speed≈1.3, 903 TTS clips):
      target ≈ frame-snapped(source / speed)
    where frame = 1_000_000 / fps. Frame-snap matched 710/903 samples;
    plain round() only 67/903. Material + source_timerange keep source duration.
    """
    speed = float(clip_speed) if clip_speed and clip_speed > 0 else 1.0
    if abs(speed - 1.0) < 1e-9:
        return int(source_duration_us)
    raw = source_duration_us / speed
    if fps and fps > 0:
        frame_us = 1_000_000.0 / float(fps)
        return max(1_000, int(round(round(raw / frame_us) * frame_us)))
    return max(1_000, int(round(raw)))


@dataclass(frozen=True)
class VoiceOption:
    language_code: str
    locale: str
    voice_type: str
    display_name: str
    resource_id: str


@dataclass
class CaptionRow:
    index: int
    text_track_id: str
    text_segment_id: str
    text_material_id: str
    start_us: int
    duration_us: int
    text: str
    existing_tts_segment_ids: list[str] = field(default_factory=list)

    @property
    def end_us(self) -> int:
        return self.start_us + self.duration_us

    @property
    def has_existing_tts(self) -> bool:
        return bool(self.existing_tts_segment_ids)

    @property
    def is_empty(self) -> bool:
        return not (self.text or "").strip()


@dataclass(frozen=True)
class CapCutProjectInfo:
    project_id: str
    project_name: str
    draft_path: Path
    project_directory: Path

    version: int
    new_version: str | None

    width: int
    height: int
    fps: float
    duration_us: int

    video_track_count: int
    audio_track_count: int
    text_track_count: int

    video_segment_count: int
    audio_segment_count: int
    text_segment_count: int
    caption_count: int

    caption_with_tts_count: int
    empty_caption_count: int

    @property
    def duration_display(self) -> str:
        return format_duration_us(self.duration_us)

    @property
    def resolution(self) -> str:
        return f"{self.width}×{self.height}"


@dataclass
class ProjectInspectionResult:
    project_info: CapCutProjectInfo
    valid_caption_count: int
    selected_caption_count: int
    skipped_empty_count: int
    existing_tts_count: int
    orphan_text_segment_count: int
    invalid_content_count: int
    warnings: list[str] = field(default_factory=list)
    has_tts_template: bool = False


@dataclass
class TtsGenerationSettings:
    voice_type: str
    resource_id: str
    voice_display_name: str

    tts_rate: float
    capcut_clip_speed: float
    tone_modify_mode: ToneModifyMode

    existing_tts_mode: str = "replace_existing"  # skip_existing | replace_existing | selected_only


@dataclass(frozen=True)
class GenerationLogEvent:
    timestamp: datetime
    level: str
    message: str
    stage: str | None = None
    caption_index: int | None = None
    progress: float | None = None


@dataclass
class CaptionTtsResult:
    caption_index: int
    text_material_id: str
    text_segment_id: str
    start_us: int
    audio_path: str
    source_duration_us: int
    target_duration_us: int
    status: str  # generated | cached | failed | skipped
    error: str = ""
    from_cache: bool = False


@dataclass
class GenerationResult:
    success: bool
    project_path: Path
    backup_path: Path | None
    selected: int = 0
    generated: int = 0
    cached: int = 0
    skipped: int = 0
    failed: int = 0
    replaced: int = 0
    attached: int = 0
    audio_tracks_used: int = 0
    audio_tracks_created: int = 0
    processing_seconds: float = 0.0
    validation_passed: bool = False
    log_file: Path | None = None
    manifest_path: Path | None = None
    warnings: list[str] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    items: list[CaptionTtsResult] = field(default_factory=list)
    extra: dict[str, Any] = field(default_factory=dict)

    @property
    def completed_with_warnings(self) -> bool:
        return self.success and (self.failed > 0 or bool(self.warnings))
