"""Read CapCut draft_content.json — captions, project info, templates."""

from __future__ import annotations

import copy
import json
import re
from pathlib import Path
from typing import Any

from core.logger import logger
from .models import (
    CaptionRow,
    CapCutProjectInfo,
    ProjectInspectionResult,
)


class DraftReader:
    def __init__(self):
        self.draft_path: Path | None = None
        self.project_directory: Path | None = None
        self.draft: dict[str, Any] | None = None
        self._captions: list[CaptionRow] = []

    # ------------------------------------------------------------------
    # Load
    # ------------------------------------------------------------------
    def resolve_draft_path(self, path: Path | str) -> Path:
        p = Path(path)
        if p.is_dir():
            candidate = p / "draft_content.json"
            if not candidate.exists():
                # CapCut modern layout: Timelines/<id>/draft_content.json
                timelines = p / "Timelines"
                if timelines.is_dir():
                    found = list(timelines.rglob("draft_content.json"))
                    if found:
                        return found[0]
                raise FileNotFoundError(f"draft_content.json not found under {p}")
            return candidate
        if p.is_file():
            return p
        raise FileNotFoundError(f"Path not found: {p}")

    def load_draft(self, path: Path | str) -> dict[str, Any]:
        draft_path = self.resolve_draft_path(path)
        with open(draft_path, "r", encoding="utf-8-sig") as f:
            data = json.load(f)
        if not isinstance(data, dict):
            raise ValueError("draft_content.json root must be an object")
        if "tracks" not in data or "materials" not in data:
            raise ValueError("draft_content.json missing required keys: tracks/materials")
        materials = data.get("materials") or {}
        if "texts" not in materials:
            raise ValueError("draft_content.json missing materials.texts")

        self.draft_path = draft_path
        self.project_directory = draft_path.parent
        # If under Timelines/<id>/, project dir is two levels up for audio storage
        if draft_path.parent.parent.name == "Timelines":
            self.project_directory = draft_path.parent.parent.parent
        self.draft = data
        self._captions = self.extract_captions()
        logger.info("Loaded draft: %s", draft_path)
        return data

    def get_draft_copy(self) -> dict[str, Any]:
        if self.draft is None:
            raise RuntimeError("No draft loaded")
        # json clone is faster than deepcopy on multi-MB CapCut drafts
        try:
            return json.loads(json.dumps(self.draft, ensure_ascii=False, separators=(",", ":")))
        except (TypeError, ValueError):
            return copy.deepcopy(self.draft)

    # ------------------------------------------------------------------
    # Project info
    # ------------------------------------------------------------------
    def get_project_info(self) -> CapCutProjectInfo:
        if self.draft is None or self.draft_path is None or self.project_directory is None:
            raise RuntimeError("No draft loaded")
        d = self.draft
        canvas = d.get("canvas_config") or {}
        tracks = d.get("tracks") or []
        materials = d.get("materials") or {}

        video_tracks = [t for t in tracks if t.get("type") == "video"]
        audio_tracks = [t for t in tracks if t.get("type") == "audio"]
        text_tracks = [t for t in tracks if t.get("type") == "text"]

        def seg_count(ts: list) -> int:
            return sum(len(t.get("segments") or []) for t in ts)

        captions = self._captions or self.extract_captions()
        with_tts = sum(1 for c in captions if c.has_existing_tts)
        empty = sum(1 for c in captions if c.is_empty)

        project_name = (
            d.get("name")
            or self.project_directory.name
            or self.draft_path.parent.name
        )

        return CapCutProjectInfo(
            project_id=str(d.get("id") or ""),
            project_name=str(project_name or "Untitled"),
            draft_path=self.draft_path,
            project_directory=self.project_directory,
            version=int(d.get("version") or 0),
            new_version=str(d.get("new_version")) if d.get("new_version") is not None else None,
            width=int(canvas.get("width") or 0),
            height=int(canvas.get("height") or 0),
            fps=float(d.get("fps") or 30.0),
            duration_us=int(d.get("duration") or 0),
            video_track_count=len(video_tracks),
            audio_track_count=len(audio_tracks),
            text_track_count=len(text_tracks),
            video_segment_count=seg_count(video_tracks),
            audio_segment_count=seg_count(audio_tracks),
            text_segment_count=seg_count(text_tracks),
            caption_count=len(captions),
            caption_with_tts_count=with_tts,
            empty_caption_count=empty,
        )

    # ------------------------------------------------------------------
    # Captions
    # ------------------------------------------------------------------
    @staticmethod
    def _parse_caption_text(material: dict[str, Any]) -> tuple[str, bool]:
        """Return (text, content_ok)."""
        raw = material.get("content")
        if raw is None:
            return "", True
        if isinstance(raw, dict):
            return str(raw.get("text") or ""), True
        if not isinstance(raw, str):
            return "", False
        if not raw.strip():
            return "", True
        try:
            content = json.loads(raw)
        except json.JSONDecodeError:
            # some drafts store plain text
            return raw, True
        if isinstance(content, dict):
            return str(content.get("text") or ""), True
        return str(content), True

    def extract_captions(self) -> list[CaptionRow]:
        if self.draft is None:
            raise RuntimeError("No draft loaded")
        materials = self.draft.get("materials") or {}
        texts = materials.get("texts") or []
        text_by_id = {t.get("id"): t for t in texts if isinstance(t, dict) and t.get("id")}

        # segment id set for validating text_to_audio_ids
        all_segment_ids: set[str] = set()
        for track in self.draft.get("tracks") or []:
            for seg in track.get("segments") or []:
                sid = seg.get("id")
                if sid:
                    all_segment_ids.add(sid)

        rows: list[CaptionRow] = []
        index = 0
        for track in self.draft.get("tracks") or []:
            if track.get("type") != "text":
                continue
            track_id = str(track.get("id") or "")
            for seg in track.get("segments") or []:
                mat_id = seg.get("material_id")
                material = text_by_id.get(mat_id)
                if not material:
                    continue
                if material.get("type") != "subtitle":
                    continue
                index += 1
                text, _ok = self._parse_caption_text(material)
                tr = seg.get("target_timerange") or {}
                start_us = int(tr.get("start") or 0)
                duration_us = int(tr.get("duration") or 0)
                tta = material.get("text_to_audio_ids") or []
                if not isinstance(tta, list):
                    tta = []
                existing = [str(x) for x in tta if x]
                rows.append(
                    CaptionRow(
                        index=index,
                        text_track_id=track_id,
                        text_segment_id=str(seg.get("id") or ""),
                        text_material_id=str(material.get("id") or mat_id),
                        start_us=start_us,
                        duration_us=duration_us,
                        text=text,
                        existing_tts_segment_ids=existing,
                    )
                )

        rows.sort(key=lambda r: (r.start_us, r.index))
        # re-index after sort to keep stable timeline order
        for i, row in enumerate(rows, start=1):
            row.index = i
        self._captions = rows
        return list(rows)

    def get_captions(self) -> list[CaptionRow]:
        if not self._captions and self.draft is not None:
            return self.extract_captions()
        return list(self._captions)

    # ------------------------------------------------------------------
    # Templates
    # ------------------------------------------------------------------
    def find_tts_templates(self) -> dict[str, Any] | None:
        """Return deep-copyable templates from draft if TTS already exists."""
        if self.draft is None:
            return None
        materials = self.draft.get("materials") or {}
        audios = materials.get("audios") or []
        tts_mats = [a for a in audios if isinstance(a, dict) and a.get("type") == "text_to_audio"]
        if not tts_mats:
            return None

        mat = tts_mats[0]
        mat_id = mat.get("id")
        segment = None
        track = None
        for t in self.draft.get("tracks") or []:
            if t.get("type") != "audio":
                continue
            for s in t.get("segments") or []:
                if s.get("material_id") == mat_id:
                    segment = s
                    track = t
                    break
            if segment:
                break
        if not segment:
            return None

        extras: dict[str, Any] = {}
        for ref in segment.get("extra_material_refs") or []:
            for bucket, items in materials.items():
                if not isinstance(items, list):
                    continue
                for it in items:
                    if isinstance(it, dict) and it.get("id") == ref:
                        extras[bucket] = copy.deepcopy(it)
                        break

        return {
            "audio_material": copy.deepcopy(mat),
            "audio_segment": copy.deepcopy(segment),
            "audio_track": {k: copy.deepcopy(v) for k, v in (track or {}).items() if k != "segments"},
            "extras": extras,
        }

    def find_speed_templates(self) -> dict[str, Any] | None:
        if self.draft is None:
            return None
        speeds = (self.draft.get("materials") or {}).get("speeds") or []
        if not speeds:
            return None
        # Prefer a speed material already used by TTS if present
        tpl = self.find_tts_templates()
        if tpl and "speeds" in tpl.get("extras", {}):
            return copy.deepcopy(tpl["extras"]["speeds"])
        return copy.deepcopy(speeds[0])

    def builtin_tts_templates(self) -> dict[str, Any]:
        """Fallback templates derived from verified CapCut sample (no draft TTS)."""
        return {
            "audio_material": {
                "id": "",
                "unique_id": "",
                "type": "text_to_audio",
                "name": "",
                "duration": 0,
                "path": "",
                "category_name": "",
                "wave_points": [],
                "music_id": "",
                "app_id": 0,
                "text_id": "",
                "tone_type": "",
                "source_platform": 0,
                "video_id": "",
                "effect_id": "",
                "resource_id": "",
                "third_resource_id": "",
                "category_id": "",
                "intensifies_path": "",
                "formula_id": "",
                "check_flag": 1,
                "team_id": "",
                "local_material_id": "",
                "tone_speaker": "",
                "mock_tone_speaker": "",
                "tone_effect_id": "",
                "tone_effect_name": "",
                "tone_platform": "sami",
                "cloned_model_type": "",
                "tone_category_id": "",
                "tone_category_name": "",
                "tone_second_category_id": "",
                "tone_second_category_name": "",
                "tone_emotion_name_key": "",
                "tone_emotion_style": "",
                "tone_emotion_role": "",
                "tone_emotion_selection": "",
                "tone_emotion_scale": 0.0,
                "moyin_emotion": "",
                "request_id": "",
                "query": "",
                "search_id": "",
                "sound_separate_type": "",
                "is_text_edit_overdub": False,
                "is_ugc": False,
                "is_ai_clone_tone": False,
                "is_ai_clone_tone_post": False,
                "source_from": "",
                "copyright_limit_type": "none",
                "aigc_history_id": "",
                "aigc_item_id": "",
                "music_source": "",
                "pgc_id": "",
                "pgc_name": "",
                "similiar_music_info": {
                    "original_song_id": "",
                    "original_song_name": "",
                },
                "ai_music_type": 0,
                "ai_music_enter_from": "",
                "lyric_type": 0,
                "tts_task_id": "",
                "tts_generate_scene": "audio_panel",
                "ai_music_generate_scene": 0,
                "tts_benefit_info": {
                    "benefit_type": "none",
                    "benefit_log_id": "",
                    "benefit_log_extra": "",
                    "benefit_amount": -1,
                },
            },
            "audio_segment": {
                "id": "",
                "source_timerange": {"start": 0, "duration": 0},
                "target_timerange": {"start": 0, "duration": 0},
                "render_timerange": {"start": 0, "duration": 0},
                "desc": "",
                "state": 0,
                "speed": 1.0,
                "is_loop": False,
                "is_tone_modify": False,
                "reverse": False,
                "intensifies_audio": False,
                "cartoon": False,
                "volume": 1.0,
                "last_nonzero_volume": 1.0,
                "clip": None,
                "uniform_scale": None,
                "material_id": "",
                "extra_material_refs": [],
                "render_index": 0,
                "keyframe_refs": [],
                "enable_lut": False,
                "enable_adjust": False,
                "enable_hsl": False,
                "visible": True,
                "group_id": "",
                "enable_color_curves": True,
                "enable_hsl_curves": True,
                "track_render_index": 0,
                "hdr_settings": None,
                "enable_color_wheels": True,
                "track_attribute": 0,
                "is_placeholder": False,
                "template_id": "",
                "enable_smart_color_adjust": False,
                "template_scene": "default",
                "common_keyframes": [],
                "caption_info": None,
                "responsive_layout": {
                    "enable": False,
                    "target_follow": "",
                    "size_layout": 0,
                    "horizontal_pos_layout": 0,
                    "vertical_pos_layout": 0,
                },
                "enable_color_match_adjust": False,
                "enable_color_correct_adjust": False,
                "enable_adjust_mask": False,
                "raw_segment_id": "",
                "lyric_keyframes": None,
                "enable_video_mask": True,
                "digital_human_template_group_id": "",
                "color_correct_alg_result": "",
                "source": "segmentsourcenormal",
                "enable_mask_stroke": False,
                "enable_mask_shadow": False,
                "enable_color_adjust_pro": False,
            },
            "audio_track": {
                "id": "",
                "type": "audio",
                "flag": 0,
                "attribute": 0,
                "name": "",
                "is_default_name": True,
                "segments": [],
            },
            "extras": {
                "speeds": {
                    "id": "",
                    "type": "speed",
                    "mode": 0,
                    "speed": 1.0,
                    "curve_speed": None,
                },
                "placeholder_infos": {
                    "id": "",
                    "type": "placeholder_info",
                    "meta_type": "none",
                    "res_path": "",
                    "res_text": "",
                    "error_path": "",
                    "error_text": "",
                },
                "beats": {
                    "id": "",
                    "type": "beats",
                    "enable_ai_beats": False,
                    "gear": 404,
                    "gear_count": 0,
                    "mode": 404,
                    "user_beats": [],
                    "user_delete_ai_beats": None,
                    "ai_beats": {
                        "melody_url": "",
                        "melody_path": "",
                        "beats_url": "",
                        "beats_path": "",
                        "melody_percents": [0.0],
                        "beat_speed_infos": [],
                    },
                },
                "sound_channel_mappings": {
                    "id": "",
                    "type": "",
                    "audio_channel_mapping": 0,
                    "is_config_open": False,
                },
                "vocal_separations": {
                    "id": "",
                    "type": "vocal_separation",
                    "choice": 0,
                    "removed_sounds": [],
                    "time_range": None,
                    "production_path": "",
                    "final_algorithm": "",
                    "enter_from": "",
                },
            },
        }

    def get_tts_templates(self) -> dict[str, Any]:
        found = self.find_tts_templates()
        if found:
            return found
        return self.builtin_tts_templates()

    # ------------------------------------------------------------------
    # Inspection
    # ------------------------------------------------------------------
    def inspect_project(self, selected_ids: list[str] | None = None) -> ProjectInspectionResult:
        info = self.get_project_info()
        captions = self.get_captions()
        warnings: list[str] = []
        invalid_content = 0
        orphan_segments = 0

        materials = (self.draft or {}).get("materials") or {}
        texts = materials.get("texts") or []
        text_by_id = {t.get("id"): t for t in texts if isinstance(t, dict)}

        # orphan text segments (material missing)
        for track in (self.draft or {}).get("tracks") or []:
            if track.get("type") != "text":
                continue
            for seg in track.get("segments") or []:
                if seg.get("material_id") not in text_by_id:
                    orphan_segments += 1

        # invalid content + dangling text_to_audio_ids
        all_seg_ids: set[str] = set()
        for track in (self.draft or {}).get("tracks") or []:
            for seg in track.get("segments") or []:
                if seg.get("id"):
                    all_seg_ids.add(seg["id"])

        for t in texts:
            if t.get("type") != "subtitle":
                continue
            _, ok = self._parse_caption_text(t)
            if not ok:
                invalid_content += 1
            for sid in t.get("text_to_audio_ids") or []:
                if sid not in all_seg_ids:
                    warnings.append(f"text_to_audio_ids orphan segment: {sid}")

        # subtitle materials without segments
        linked_mat_ids = {c.text_material_id for c in captions}
        for t in texts:
            if t.get("type") == "subtitle" and t.get("id") not in linked_mat_ids:
                warnings.append(f"subtitle material without segment: {t.get('id')}")

        # orphan TTS audio materials
        for a in materials.get("audios") or []:
            if a.get("type") == "text_to_audio":
                tid = a.get("text_id")
                if tid and tid not in text_by_id:
                    warnings.append(f"orphan TTS audio material text_id={tid} id={a.get('id')}")

        # duplicate UUIDs (sample)
        seen: set[str] = set()
        dups = 0
        for track in (self.draft or {}).get("tracks") or []:
            for seg in track.get("segments") or []:
                sid = seg.get("id")
                if not sid:
                    continue
                if sid in seen:
                    dups += 1
                seen.add(sid)
        if dups:
            warnings.append(f"duplicate segment UUIDs: {dups}")

        has_tpl = self.find_tts_templates() is not None
        if not has_tpl:
            warnings.append("No existing TTS template in project; using built-in CapCut-verified template")

        selected_set = set(selected_ids or [])
        if selected_ids is None:
            selected_count = sum(1 for c in captions if not c.is_empty)
        else:
            selected_count = sum(
                1
                for c in captions
                if c.text_segment_id in selected_set or str(c.index) in selected_set
            )

        return ProjectInspectionResult(
            project_info=info,
            valid_caption_count=sum(1 for c in captions if not c.is_empty),
            selected_caption_count=selected_count,
            skipped_empty_count=sum(1 for c in captions if c.is_empty),
            existing_tts_count=sum(1 for c in captions if c.has_existing_tts),
            orphan_text_segment_count=orphan_segments,
            invalid_content_count=invalid_content,
            warnings=warnings,
            has_tts_template=has_tpl,
        )
