"""Validate patched CapCut draft integrity before commit."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from core.logger import logger


class ValidationError(Exception):
    def __init__(self, errors: list[str]):
        self.errors = errors
        super().__init__("; ".join(errors[:5]) + (f" (+{len(errors) - 5} more)" if len(errors) > 5 else ""))


class DraftValidator:
    def validate(
        self,
        draft: dict[str, Any],
        *,
        project_directory: Path | None = None,
        require_audio_files: bool = True,
    ) -> list[str]:
        """Return list of error strings (empty = OK)."""
        errors: list[str] = []
        materials = draft.get("materials") or {}
        tracks = draft.get("tracks") or []

        # Index materials by id across buckets
        by_id: dict[str, tuple[str, dict]] = {}
        duplicate_ids: list[str] = []
        for bucket, items in materials.items():
            if not isinstance(items, list):
                continue
            for it in items:
                if not isinstance(it, dict):
                    continue
                iid = it.get("id")
                if not iid:
                    continue
                if iid in by_id:
                    duplicate_ids.append(str(iid))
                by_id[str(iid)] = (bucket, it)

        # Segment index
        segments_by_id: dict[str, dict] = {}
        for track in tracks:
            for seg in track.get("segments") or []:
                sid = seg.get("id")
                if not sid:
                    continue
                if sid in segments_by_id:
                    duplicate_ids.append(str(sid))
                segments_by_id[str(sid)] = seg
                if sid in by_id:
                    duplicate_ids.append(str(sid))

        if duplicate_ids:
            # unique
            uniq = sorted(set(duplicate_ids))
            errors.append(f"Duplicate UUIDs ({len(uniq)}): e.g. {uniq[0]}")
            logger.info("[Validate] Duplicate UUIDs: %s", len(uniq))
        else:
            logger.info("[Validate] Duplicate UUIDs: none")

        texts = [t for t in (materials.get("texts") or []) if isinstance(t, dict)]
        audios = [a for a in (materials.get("audios") or []) if isinstance(a, dict)]
        tts_audios = [a for a in audios if a.get("type") == "text_to_audio"]
        text_ids = {t.get("id") for t in texts if t.get("id")}

        # Caption -> audio segment refs
        bad_tta = 0
        for t in texts:
            for sid in t.get("text_to_audio_ids") or []:
                if sid not in segments_by_id:
                    bad_tta += 1
                    errors.append(
                        f"Caption material {t.get('id')} text_to_audio_ids -> missing segment {sid}"
                    )
        logger.info("[Validate] Caption links: %s", "OK" if bad_tta == 0 else f"FAILED ({bad_tta})")

        # Audio segment -> material
        audio_tracks = [t for t in tracks if t.get("type") == "audio"]
        bad_mat = 0
        bad_speed = 0
        bad_extra = 0
        bad_tr = 0
        bad_tone = 0
        speed_mismatch = 0

        for track in audio_tracks:
            for seg in track.get("segments") or []:
                sid = seg.get("id")
                mid = seg.get("material_id")
                mat = None
                if mid not in by_id:
                    bad_mat += 1
                    errors.append(f"Audio segment {sid} references missing material {mid}")
                else:
                    bucket, mat = by_id[mid]
                    if bucket != "audios":
                        bad_mat += 1
                        errors.append(f"Audio segment {sid} material {mid} is in {bucket}")

                # timeranges
                src = seg.get("source_timerange") or {}
                tgt = seg.get("target_timerange") or {}
                if not isinstance(src, dict) or not isinstance(tgt, dict):
                    bad_tr += 1
                    errors.append(f"Audio segment {sid} missing timerange objects")
                else:
                    for key, tr in (("source", src), ("target", tgt)):
                        dur = tr.get("duration")
                        start = tr.get("start")
                        if not isinstance(dur, (int, float)) or dur <= 0:
                            bad_tr += 1
                            errors.append(f"Audio segment {sid} {key}_timerange.duration invalid: {dur}")
                        if not isinstance(start, (int, float)) or start < 0:
                            bad_tr += 1
                            errors.append(f"Audio segment {sid} {key}_timerange.start invalid: {start}")
                        # microseconds sanity: duration shouldn't be tiny float seconds
                        if isinstance(dur, (int, float)) and 0 < dur < 1000:
                            bad_tr += 1
                            errors.append(
                                f"Audio segment {sid} {key}_timerange.duration looks like seconds not us: {dur}"
                            )

                tone = seg.get("is_tone_modify")
                if not isinstance(tone, bool):
                    bad_tone += 1
                    errors.append(f"Audio segment {sid} is_tone_modify not bool: {tone!r}")

                # extras + speed
                speed_refs = []
                for ref in seg.get("extra_material_refs") or []:
                    if ref not in by_id:
                        bad_extra += 1
                        errors.append(f"Audio segment {sid} extra ref missing: {ref}")
                        continue
                    bucket, obj = by_id[ref]
                    if bucket == "speeds" or obj.get("type") == "speed":
                        speed_refs.append(obj)
                fade_refs = []
                for ref in seg.get("extra_material_refs") or []:
                    if ref not in by_id:
                        continue
                    bucket, obj = by_id[ref]
                    if bucket == "audio_fades" or obj.get("type") == "audio_fade":
                        fade_refs.append(obj)

                if mat and mat.get("type") == "text_to_audio":
                    if len(speed_refs) != 1:
                        bad_speed += 1
                        errors.append(
                            f"TTS segment {sid} must have exactly one speed material, got {len(speed_refs)}"
                        )
                    elif abs(float(speed_refs[0].get("speed", 1.0)) - float(seg.get("speed") or 1.0)) > 1e-6:
                        speed_mismatch += 1
                        errors.append(
                            f"TTS segment {sid} speed={seg.get('speed')} != speed_material={speed_refs[0].get('speed')}"
                        )
                    # native alignment: source within material duration
                    mat_dur = int(mat.get("duration") or 0)
                    src_start = int((seg.get("source_timerange") or {}).get("start") or 0)
                    src_dur = int((seg.get("source_timerange") or {}).get("duration") or 0)
                    if src_start < 0 or src_dur <= 0:
                        errors.append(f"TTS segment {sid} invalid source_timerange")
                    elif mat_dur > 0 and src_start + src_dur > mat_dur + 1:
                        errors.append(
                            f"TTS segment {sid} source range exceeds material duration "
                            f"({src_start}+{src_dur} > {mat_dur})"
                        )
                    for fade in fade_refs:
                        fi = int(fade.get("fade_in_duration") or 0)
                        tgt_dur = int((seg.get("target_timerange") or {}).get("duration") or 0)
                        if fi < 0 or (tgt_dur > 0 and fi > tgt_dur):
                            errors.append(
                                f"TTS segment {sid} fade_in_duration {fi} invalid for target {tgt_dur}"
                            )

        logger.info("[Validate] Audio materials: %s", "OK" if bad_mat == 0 else f"FAILED ({bad_mat})")
        logger.info("[Validate] Extra references: %s", "OK" if bad_extra == 0 else f"FAILED ({bad_extra})")
        logger.info(
            "[Validate] Speed material references: %s",
            "OK" if bad_speed == 0 and speed_mismatch == 0 else f"FAILED speed={bad_speed} mismatch={speed_mismatch}",
        )

        # TTS material text_id
        bad_text_id = 0
        for a in tts_audios:
            tid = a.get("text_id")
            if not tid or tid not in text_ids:
                bad_text_id += 1
                errors.append(f"TTS audio {a.get('id')} text_id invalid: {tid}")
            dur = a.get("duration")
            if not isinstance(dur, (int, float)) or dur <= 0:
                errors.append(f"TTS audio {a.get('id')} duration invalid: {dur}")

            if require_audio_files and project_directory is not None:
                path = str(a.get("path") or "")
                if not path:
                    errors.append(f"TTS audio {a.get('id')} missing path")
                else:
                    # resolve relative / placeholder paths
                    resolved = self._resolve_audio_path(path, project_directory)
                    if resolved is None or not resolved.exists() or resolved.stat().st_size <= 0:
                        errors.append(f"TTS audio file missing: {path}")

        # Orphan audio fades (not referenced by any segment)
        referenced_extras: set[str] = set()
        for track in tracks:
            for seg in track.get("segments") or []:
                for ref in seg.get("extra_material_refs") or []:
                    referenced_extras.add(str(ref))
        orphan_fades = 0
        for fade in materials.get("audio_fades") or []:
            if not isinstance(fade, dict):
                continue
            fid = fade.get("id")
            if fid and str(fid) not in referenced_extras:
                orphan_fades += 1
                errors.append(f"Orphan audio_fade material: {fid}")
        if orphan_fades:
            logger.info("[Validate] Orphan audio fades: %s", orphan_fades)

        # Track overlaps (same audio track)
        overlaps = 0
        for track in audio_tracks:
            intervals = []
            for seg in track.get("segments") or []:
                tgt = seg.get("target_timerange") or {}
                start = int(tgt.get("start") or 0)
                dur = int(tgt.get("duration") or 0)
                intervals.append((start, start + dur, seg.get("id")))
            intervals.sort()
            for i in range(1, len(intervals)):
                prev = intervals[i - 1]
                cur = intervals[i]
                if cur[0] < prev[1]:
                    overlaps += 1
                    errors.append(
                        f"Overlap on audio track {track.get('id')}: {prev[2]} and {cur[2]}"
                    )
        logger.info("[Validate] Track overlaps: %s", "none" if overlaps == 0 else overlaps)

        # JSON serialize
        try:
            import json

            json.dumps(draft, ensure_ascii=False, separators=(",", ":"))
            logger.info("[Validate] JSON serialization: OK")
        except Exception as e:
            errors.append(f"JSON serialization failed: {e}")
            logger.info("[Validate] JSON serialization: FAILED")

        if errors:
            logger.info("[Validate] Project integrity: FAILED (%s errors)", len(errors))
        else:
            logger.info("[Validate] Project integrity: PASSED")
        return errors

    @staticmethod
    def _resolve_audio_path(path: str, project_directory: Path) -> Path | None:
        p = Path(path)
        if p.is_file():
            return p
        # CapCut draftpath placeholder
        if "##_draftpath_placeholder_" in path or path.startswith("textReading/") or "/textReading/" in path:
            # take suffix after last placeholder or relative textReading
            m = path
            if "textReading/" in m:
                rel = m.split("textReading/", 1)[1]
                candidate = project_directory / "textReading" / rel
                if candidate.exists():
                    return candidate
            name = Path(path).name
            candidate = project_directory / "textReading" / name
            if candidate.exists():
                return candidate
        # relative to project
        candidate = project_directory / path
        if candidate.exists():
            return candidate
        candidate = project_directory / Path(path).name
        if candidate.exists():
            return candidate
        return None
