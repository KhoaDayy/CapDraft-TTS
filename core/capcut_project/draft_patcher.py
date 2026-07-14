"""Patch CapCut draft JSON with generated TTS audio (in-memory)."""

from __future__ import annotations

import copy
import json
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from core.logger import logger
from .models import (
    CaptionRow,
    CaptionTtsResult,
    ToneModifyMode,
    coerce_tone_modify_mode,
    compute_target_duration_us,
    map_tone_mode_to_capcut_flag,
    seconds_to_us,
)
from .native_audio_alignment import (
    NativeAlignmentResult,
    NativeAudioAlignmentSettings,
    apply_native_audio_alignment,
)


SAFE_TTS_VOICE_TYPE = "BV074_streaming"
SAFE_TTS_RESOURCE_ID = "7102355709945188865"
SAFE_TTS_DISPLAY_NAME = "C\u00f4 G\u00e1i Ho\u1ea1t Ng\u00f4n"
SAFE_TTS_BENEFIT_INFO = {
    "benefit_type": "none",
    "benefit_log_id": "",
    "benefit_log_extra": "",
    "benefit_amount": -1,
}
SAFE_TTS_BLANK_FIELDS = (
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
)

def new_uuid() -> str:
    """CapCut-style mixed-case UUID string."""
    return str(uuid.uuid4()).upper()


@dataclass
class PatchStats:
    materials_created: int = 0
    segments_created: int = 0
    speeds_created: int = 0
    fades_created: int = 0
    alignment_applied: int = 0
    alignment_trim_reduced: int = 0
    replaced: int = 0
    skipped: int = 0
    tracks_used: int = 0
    tracks_created: int = 0
    orphans_removed: int = 0
    created_ids: list[str] = field(default_factory=list)
    removed_ids: list[str] = field(default_factory=list)


class DraftPatcher:
    """Deep-copy template → new UUIDs → remap refs → attach TTS."""

    # Used when removing old TTS extras (keep broad so orphans get cleaned).
    EXTRA_BUCKETS = (
        "speeds",
        "placeholder_infos",
        "beats",
        "sound_channel_mappings",
        "vocal_separations",
        "audio_fades",
    )
    # Only create what CapCut needs for clip-speed TTS. Extra template buckets
    # (placeholder/beats/channel/vocal) are ~4 objects × N captions and make
    # CapCut export crawl or fail on multi-thousand caption projects.
    CREATE_EXTRA_BUCKETS = ("speeds",)

    def __init__(
        self,
        templates: dict[str, Any],
        *,
        fps: float = 30.0,
        alignment: NativeAudioAlignmentSettings | None = None,
    ):
        self.templates = templates
        self.fps = fps or 30.0
        self.alignment = alignment or NativeAudioAlignmentSettings()
        # Snapshot templates once so per-caption clone is cheap (json round-trip
        # is faster than deepcopy for CapCut's large nested material dicts).
        self._mat_tpl = templates.get("audio_material") or {}
        self._seg_tpl = templates.get("audio_segment") or {}
        self._extras_tpl = templates.get("extras") or {}
        self._track_tpl = templates.get("audio_track") or {
            "id": "",
            "type": "audio",
            "flag": 0,
            "attribute": 0,
            "name": "",
            "is_default_name": True,
            "segments": [],
        }

    @staticmethod
    def _clone(obj: Any) -> Any:
        """Fast deep clone for JSON-like dict/list trees."""
        if obj is None or isinstance(obj, (str, int, float, bool)):
            return obj
        try:
            # C-accelerated path; CapCut materials are pure JSON types.
            return json.loads(json.dumps(obj, ensure_ascii=False, separators=(",", ":")))
        except (TypeError, ValueError):
            return copy.deepcopy(obj)

    def patch(
        self,
        draft: dict[str, Any],
        *,
        captions: list[CaptionRow],
        results: list[CaptionTtsResult],
        voice_type: str,
        resource_id: str,
        voice_display_name: str,
        capcut_clip_speed: float,
        tone_modify_mode: ToneModifyMode,
        existing_tts_mode: str,
        audio_rel_paths: dict[int, str],
        project_directory: Path,
    ) -> tuple[dict[str, Any], PatchStats]:
        """Mutate a deep-copied draft. Returns (draft, stats)."""
        stats = PatchStats()
        materials = draft.setdefault("materials", {})
        for bucket in list(self.EXTRA_BUCKETS) + ["audios"]:
            materials.setdefault(bucket, [])

        caption_by_index = {c.index: c for c in captions}
        text_by_id = {
            t.get("id"): t
            for t in materials.get("texts") or []
            if isinstance(t, dict) and t.get("id")
        }

        if existing_tts_mode == "replace_existing":
            to_replace: list[CaptionRow] = []
            for r in results:
                cap = caption_by_index.get(r.caption_index)
                if cap and cap.has_existing_tts and r.status in {"generated", "cached"}:
                    to_replace.append(cap)
            removed = self._remove_existing_tts(draft, to_replace)
            stats.replaced = len(to_replace)
            stats.orphans_removed = removed
            logger.info("[Patch] Removing %s existing TTS links", len(to_replace))

        attach_items: list[tuple[CaptionRow, CaptionTtsResult, str]] = []
        for r in results:
            cap = caption_by_index.get(r.caption_index)
            if cap is None:
                continue
            if r.status not in {"generated", "cached"}:
                if r.status == "skipped":
                    stats.skipped += 1
                continue
            if existing_tts_mode == "skip_existing" and cap.has_existing_tts:
                stats.skipped += 1
                continue
            rel = audio_rel_paths.get(r.caption_index)
            if not rel:
                continue
            attach_items.append((cap, r, rel))

        pending_segments: list[dict[str, Any]] = []
        is_tone = map_tone_mode_to_capcut_flag(coerce_tone_modify_mode(tone_modify_mode))
        clip_speed = float(capcut_clip_speed) if capcut_clip_speed and capcut_clip_speed > 0 else 1.0

        if self.alignment.enabled:
            trim_us = int(round(self.alignment.leading_trim_frames / self.fps * 1_000_000))
            logger.info(
                "[Alignment] Native CapCut alignment enabled · FPS=%g · trim=%.1f frames (%sms) · fade=%.0fms",
                self.fps,
                self.alignment.leading_trim_frames,
                trim_us // 1000,
                self.alignment.fade_in_ms,
            )

        for cap, result, rel_path in attach_items:
            mat, seg, extra_objs, align_res = self._build_tts_objects(
                caption=cap,
                result=result,
                rel_path=rel_path,
                voice_type=voice_type,
                resource_id=resource_id,
                voice_display_name=voice_display_name,
                clip_speed=clip_speed,
                is_tone_modify=is_tone,
            )
            materials["audios"].append(mat)
            stats.materials_created += 1
            stats.created_ids.append(mat["id"])

            for bucket, obj in extra_objs:
                materials.setdefault(bucket, []).append(obj)
                stats.created_ids.append(obj["id"])
                if bucket == "speeds":
                    stats.speeds_created += 1
                if bucket == "audio_fades":
                    stats.fades_created += 1

            if align_res is not None:
                stats.alignment_applied += 1
                if align_res.trim_reduced:
                    stats.alignment_trim_reduced += 1
                # keep result fields in sync for UI/manifest
                result.source_duration_us = align_res.source_duration_us
                result.target_duration_us = align_res.target_duration_us

            text_mat = text_by_id.get(cap.text_material_id)
            if text_mat is not None:
                ids = list(text_mat.get("text_to_audio_ids") or [])
                if existing_tts_mode == "replace_existing":
                    ids = [seg["id"]]
                else:
                    if seg["id"] not in ids:
                        ids.append(seg["id"])
                text_mat["text_to_audio_ids"] = ids

            pending_segments.append(seg)
            stats.segments_created += 1
            stats.created_ids.append(seg["id"])

        normalized = self._normalize_tts_voice_materials(materials)
        if normalized:
            logger.info(
                "[Patch] Normalized %s TTS materials to export-safe voice",
                normalized,
            )

        logger.info("[Patch] Created %s audio materials", stats.materials_created)
        logger.info("[Patch] Created %s audio segments", stats.segments_created)
        logger.info("[Patch] Created %s speed materials", stats.speeds_created)
        if self.alignment.enabled:
            logger.info(
                "[Alignment] Applied native trim to %s/%s audio segments",
                stats.alignment_applied,
                len(attach_items),
            )
            logger.info("[Alignment] Audio fades created: %s", stats.fades_created)
            if stats.alignment_trim_reduced:
                logger.info(
                    "[Alignment] Reduced trim for short audio: %s",
                    stats.alignment_trim_reduced,
                )

        tracks_created, tracks_used = self._assign_tracks(draft, pending_segments)
        stats.tracks_created = tracks_created
        stats.tracks_used = tracks_used
        logger.info("[Patch] Assigned audio into %s non-overlapping tracks", tracks_used)
        if tracks_created:
            logger.info("[Patch] Created %s new audio tracks", tracks_created)
        # Rough export-cost signal for large jobs (CapCut struggles on object count).
        if stats.segments_created >= 500:
            extras_n = sum(
                len(materials.get(b) or [])
                for b in ("speeds", "audio_fades", "placeholder_infos", "beats")
            )
            logger.info(
                "[Patch] Large job · segments=%s · audio_tracks=%s · speed+fade+misc materials≈%s",
                stats.segments_created,
                tracks_used,
                extras_n,
            )

        max_end = 0
        for seg in pending_segments:
            tgt = seg.get("target_timerange") or {}
            max_end = max(max_end, int(tgt.get("start") or 0) + int(tgt.get("duration") or 0))
        if max_end > int(draft.get("duration") or 0):
            draft["duration"] = max_end

        return draft, stats

    def _build_tts_objects(
        self,
        *,
        caption: CaptionRow,
        result: CaptionTtsResult,
        rel_path: str,
        voice_type: str,
        resource_id: str,
        voice_display_name: str,
        clip_speed: float,
        is_tone_modify: bool,
    ) -> tuple[dict, dict, list[tuple[str, dict]], NativeAlignmentResult | None]:
        mat = self._clone(self._mat_tpl)
        seg = self._clone(self._seg_tpl)
        extras_tpl = self._extras_tpl

        mat_id = new_uuid()
        seg_id = new_uuid()
        raw_us = int(result.source_duration_us)
        if raw_us <= 0:
            raw_us = seconds_to_us(0.1)

        mat["id"] = mat_id
        mat["unique_id"] = mat_id
        mat["duration"] = raw_us  # always raw file duration
        mat["path"] = rel_path
        mat["name"] = (caption.text or voice_display_name or "TTS")[:40]
        mat["local_material_id"] = mat_id
        mat["music_id"] = mat_id
        # Drop heavy/unused template payload CapCut rewrites on open anyway.
        mat["wave_points"] = []
        for heavy_key in (
            "intensifies_path",
            "aigc_history_id",
            "aigc_item_id",
            "request_id",
            "query",
            "search_id",
            "tts_task_id",
        ):
            if heavy_key in mat:
                mat[heavy_key] = ""
        self._apply_safe_tts_voice(mat, text_id=caption.text_material_id)

        extra_objs: list[tuple[str, dict]] = []
        extra_ids: list[str] = []
        for bucket in self.CREATE_EXTRA_BUCKETS:
            tpl = extras_tpl.get(bucket)
            if tpl is None:
                if bucket == "speeds":
                    tpl = {"id": "", "type": "speed", "mode": 0, "speed": 1.0, "curve_speed": None}
                else:
                    continue
            obj = self._clone(tpl)
            obj["id"] = new_uuid()
            if bucket == "speeds":
                obj["type"] = "speed"
                obj["speed"] = float(clip_speed)
                obj["mode"] = int(obj.get("mode") or 0)
                obj["curve_speed"] = None
            extra_objs.append((bucket, obj))
            extra_ids.append(obj["id"])

        seg["id"] = seg_id
        seg["material_id"] = mat_id
        seg["render_timerange"] = {"start": 0, "duration": 0}
        seg["is_tone_modify"] = bool(is_tone_modify)
        seg["extra_material_refs"] = extra_ids
        seg["render_index"] = 0
        seg["volume"] = float(seg.get("volume") if seg.get("volume") is not None else 1.0)
        seg["visible"] = True
        # Keep segment payload lean — unused video/keyframe fields bloat 5k× drafts.
        for drop_key in (
            "common_keyframes",
            "keyframe_refs",
            "lyric_keyframes",
            "caption_info",
            "hdr_settings",
            "clip",
            "uniform_scale",
        ):
            if drop_key in seg:
                seg[drop_key] = [] if drop_key.endswith("refs") or drop_key.endswith("keyframes") else None

        fade_bucket: list[dict[str, Any]] = []
        align_res = apply_native_audio_alignment(
            audio_segment=seg,
            audio_material=mat,
            audio_fade_materials=fade_bucket,
            caption_start_us=int(caption.start_us),
            raw_duration_us=raw_us,
            project_fps=self.fps,
            clip_speed=clip_speed,
            settings=self.alignment,
        )
        for fade in fade_bucket:
            extra_objs.append(("audio_fades", fade))

        # ensure speed material matches segment.speed after alignment
        for bucket, obj in extra_objs:
            if bucket == "speeds":
                obj["speed"] = float(seg.get("speed") or clip_speed)

        return mat, seg, extra_objs, align_res

    def _apply_safe_tts_voice(
        self,
        mat: dict[str, Any],
        *,
        text_id: str | None = None,
    ) -> None:
        # ponytail: one verified free CapCut voice; replace with a free-voice
        # whitelist when more voices are confirmed export-safe.
        mat["type"] = "text_to_audio"
        if text_id is not None:
            mat["text_id"] = text_id
        mat["resource_id"] = SAFE_TTS_RESOURCE_ID
        mat["tone_speaker"] = SAFE_TTS_VOICE_TYPE
        mat["mock_tone_speaker"] = SAFE_TTS_VOICE_TYPE
        mat["tone_type"] = SAFE_TTS_DISPLAY_NAME
        mat["tone_effect_name"] = SAFE_TTS_DISPLAY_NAME
        mat["tone_platform"] = "sami"
        mat["tone_emotion_scale"] = 0.0
        mat["is_ai_clone_tone"] = False
        mat["is_ai_clone_tone_post"] = False
        mat["copyright_limit_type"] = "none"
        mat["tts_generate_scene"] = mat.get("tts_generate_scene") or "audio_panel"
        mat["tts_benefit_info"] = dict(SAFE_TTS_BENEFIT_INFO)
        for key in SAFE_TTS_BLANK_FIELDS:
            mat[key] = ""

    def _normalize_tts_voice_materials(self, materials: dict[str, Any]) -> int:
        audios = materials.get("audios") or []
        normalized = 0
        for mat in audios:
            if not isinstance(mat, dict) or mat.get("type") != "text_to_audio":
                continue
            # Cheap dirty check — skip materials already on the export-safe voice.
            if (
                mat.get("resource_id") == SAFE_TTS_RESOURCE_ID
                and mat.get("tone_speaker") == SAFE_TTS_VOICE_TYPE
                and mat.get("mock_tone_speaker") == SAFE_TTS_VOICE_TYPE
                and mat.get("copyright_limit_type") == "none"
            ):
                continue
            self._apply_safe_tts_voice(mat)
            normalized += 1
        return normalized

    def _remove_existing_tts(self, draft: dict[str, Any], captions: list[CaptionRow]) -> int:
        """Remove old TTS segments/materials/extras linked to captions."""
        materials = draft.setdefault("materials", {})
        tracks = draft.get("tracks") or []

        segment_ids: set[str] = set()
        for cap in captions:
            segment_ids.update(cap.existing_tts_segment_ids)
        if not segment_ids:
            return 0

        material_ids: set[str] = set()
        extra_ids: set[str] = set()
        # refs still used by remaining segments
        remaining_extra_refs: set[str] = set()

        for track in tracks:
            if track.get("type") != "audio":
                continue
            keep = []
            for seg in track.get("segments") or []:
                if seg.get("id") in segment_ids:
                    if seg.get("material_id"):
                        material_ids.add(seg["material_id"])
                    for ref in seg.get("extra_material_refs") or []:
                        extra_ids.add(ref)
                else:
                    keep.append(seg)
                    for ref in seg.get("extra_material_refs") or []:
                        remaining_extra_refs.add(ref)
            track["segments"] = keep

        # only delete extras not shared
        deletable_extras = extra_ids - remaining_extra_refs

        removed = 0
        for bucket in list(materials.keys()):
            items = materials.get(bucket)
            if not isinstance(items, list):
                continue
            if bucket == "audios":
                before = len(items)
                materials[bucket] = [a for a in items if a.get("id") not in material_ids]
                removed += before - len(materials[bucket])
            elif bucket in self.EXTRA_BUCKETS:
                before = len(items)
                materials[bucket] = [x for x in items if x.get("id") not in deletable_extras]
                removed += before - len(materials[bucket])

        text_by_id = {
            t.get("id"): t for t in materials.get("texts") or [] if isinstance(t, dict)
        }
        for cap in captions:
            t = text_by_id.get(cap.text_material_id)
            if t is not None:
                t["text_to_audio_ids"] = []
            cap.existing_tts_segment_ids = []

        new_tracks = []
        for t in tracks:
            if t.get("type") == "audio" and not (t.get("segments") or []):
                removed += 1
                continue
            new_tracks.append(t)
        draft["tracks"] = new_tracks

        logger.info("[Patch] Removed %s orphan extra-material objects", removed)
        return removed

    def _assign_tracks(
        self, draft: dict[str, Any], segments: list[dict[str, Any]]
    ) -> tuple[int, int]:
        """Greedy interval partitioning using target_timerange after trim+speed."""
        if not segments:
            return 0, 0

        segments_sorted = sorted(
            segments,
            key=lambda s: int((s.get("target_timerange") or {}).get("start") or 0),
        )

        tracks = draft.setdefault("tracks", [])
        audio_tracks = [t for t in tracks if t.get("type") == "audio"]
        free_tracks = [t for t in audio_tracks if not (t.get("segments") or [])]

        lanes: list[dict[str, Any]] = []
        tracks_created = 0

        def ensure_lane() -> dict[str, Any]:
            nonlocal tracks_created
            if free_tracks:
                t = free_tracks.pop(0)
                lane = {"track": t, "last_end": 0}
                lanes.append(lane)
                return lane
            tpl = self._clone(self._track_tpl)
            tpl["id"] = new_uuid()
            tpl["type"] = "audio"
            tpl["segments"] = []
            tpl["name"] = ""
            tpl["is_default_name"] = True
            tracks.append(tpl)
            tracks_created += 1
            lane = {"track": tpl, "last_end": 0}
            lanes.append(lane)
            return lane

        for seg in segments_sorted:
            tgt = seg.get("target_timerange") or {}
            start = int(tgt.get("start") or 0)
            end = start + int(tgt.get("duration") or 0)
            placed = None
            # Prefer earliest-ending free lane (keeps track count low → faster CapCut export)
            for lane in lanes:
                if lane["last_end"] <= start:
                    if placed is None or lane["last_end"] < placed["last_end"]:
                        placed = lane
            if placed is None:
                placed = ensure_lane()
            placed["track"].setdefault("segments", []).append(seg)
            placed["last_end"] = end

        non_audio = [t for t in tracks if t.get("type") != "audio"]
        base_index = max(1, len(non_audio))
        used_lanes = [l for l in lanes if l["track"].get("segments")]
        seg_ids = {s.get("id") for s in segments}
        for i, lane in enumerate(used_lanes):
            tri = base_index + i
            for seg in lane["track"]["segments"]:
                if seg.get("id") in seg_ids:
                    seg["track_render_index"] = tri
                    seg["render_index"] = 0

        return tracks_created, len(used_lanes)
