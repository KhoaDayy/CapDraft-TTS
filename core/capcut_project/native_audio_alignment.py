"""Native CapCut audio alignment: source_timerange trim + audio_fades.

No FFmpeg. No re-encode. No WAV. Does not modify MP3 bytes.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from typing import Any

from core.logger import logger
from .models import compute_target_duration_us


def new_uuid() -> str:
    return str(uuid.uuid4()).upper()


@dataclass(frozen=True)
class NativeAudioAlignmentSettings:
    enabled: bool = True
    leading_trim_frames: float = 3.0
    fade_in_ms: float = 8.0
    minimum_source_duration_us: int = 50_000


@dataclass(frozen=True)
class NativeAlignmentResult:
    project_fps: float
    requested_trim_frames: float
    applied_trim_us: int
    raw_duration_us: int
    source_duration_us: int
    target_duration_us: int
    fade_in_us: int
    trim_reduced: bool = False


def frames_to_us(frames: float, fps: float) -> int:
    safe_fps = float(fps) if fps and fps > 0 else 30.0
    return int(round(float(frames) / safe_fps * 1_000_000))


def ms_to_us(ms: float) -> int:
    return int(round(float(ms) * 1000.0))


def apply_native_audio_alignment(
    *,
    audio_segment: dict[str, Any],
    audio_material: dict[str, Any],
    audio_fade_materials: list[dict[str, Any]],
    caption_start_us: int,
    raw_duration_us: int,
    project_fps: float,
    clip_speed: float,
    settings: NativeAudioAlignmentSettings | None = None,
) -> NativeAlignmentResult:
    """Mutate segment/material/fades in place. Material duration stays raw."""
    settings = settings or NativeAudioAlignmentSettings()
    fps = float(project_fps) if project_fps and project_fps > 0 else 30.0
    if not project_fps or project_fps <= 0:
        logger.warning("[Alignment] Invalid project FPS; falling back to 30.0")

    raw = max(0, int(raw_duration_us))
    # material always keeps full file duration
    audio_material["duration"] = raw

    requested_trim_us = 0
    if settings.enabled and settings.leading_trim_frames > 0:
        requested_trim_us = frames_to_us(settings.leading_trim_frames, fps)

    min_src = max(1_000, int(settings.minimum_source_duration_us))
    safe_trim_us = min(
        requested_trim_us,
        max(0, raw - min_src),
    )
    trim_reduced = safe_trim_us < requested_trim_us and requested_trim_us > 0

    source_start_us = int(safe_trim_us)
    source_duration_us = max(1_000, raw - source_start_us)
    target_duration_us = compute_target_duration_us(
        source_duration_us, clip_speed, fps=fps
    )

    audio_segment["source_timerange"] = {
        "start": source_start_us,
        "duration": source_duration_us,
    }
    # NEVER shift target start earlier than caption
    audio_segment["target_timerange"] = {
        "start": int(caption_start_us),
        "duration": int(target_duration_us),
    }
    audio_segment["speed"] = float(clip_speed) if clip_speed and clip_speed > 0 else 1.0

    fade_in_us = 0
    if settings.enabled and settings.fade_in_ms > 0:
        requested_fade = ms_to_us(settings.fade_in_ms)
        fade_in_us = min(requested_fade, max(0, target_duration_us // 4))
        if fade_in_us > 0:
            fade_id = new_uuid()
            fade_obj = {
                "id": fade_id,
                "type": "audio_fade",
                "fade_type": 0,
                "fade_in_duration": int(fade_in_us),
                "fade_out_duration": 0,
            }
            audio_fade_materials.append(fade_obj)
            refs = list(audio_segment.get("extra_material_refs") or [])
            if fade_id not in refs:
                refs.append(fade_id)
            audio_segment["extra_material_refs"] = refs

    return NativeAlignmentResult(
        project_fps=fps,
        requested_trim_frames=float(settings.leading_trim_frames),
        applied_trim_us=source_start_us,
        raw_duration_us=raw,
        source_duration_us=source_duration_us,
        target_duration_us=target_duration_us,
        fade_in_us=fade_in_us,
        trim_reduced=trim_reduced,
    )
