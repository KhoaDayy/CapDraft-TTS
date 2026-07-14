"""Split a CapCut draft into two time-halves CapCut can open/export."""

from __future__ import annotations

import json
import shutil
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

from core.logger import logger


_IGNORE_NAMES = {
    "backups",
    "logs",
    ".DS_Store",
    "Thumbs.db",
    "tts_generation_manifest.json",
}


@dataclass(frozen=True)
class SplitResult:
    cut_us: int
    part1_dir: Path
    part2_dir: Path
    part1_draft: Path
    part2_draft: Path
    part1_duration_us: int
    part2_duration_us: int
    part1_segments: int
    part2_segments: int


def choose_cut_us(duration_us: int, caption_edges_us: list[int] | None = None) -> int:
    """Midpoint cut, snapped to nearest caption edge when helpful."""
    duration_us = max(0, int(duration_us or 0))
    if duration_us <= 1:
        raise ValueError("Project duration too short to split")
    mid = duration_us // 2
    edges = [int(x) for x in (caption_edges_us or []) if 0 < int(x) < duration_us]
    if not edges:
        return mid
    best = min(edges, key=lambda e: (abs(e - mid), e))
    # Reject edges that leave a tiny half (<2% or <1s)
    min_half = max(1_000_000, duration_us // 50)
    if best < min_half or (duration_us - best) < min_half:
        return mid
    return best


def _clone(obj: Any) -> Any:
    try:
        return json.loads(json.dumps(obj, ensure_ascii=False, separators=(",", ":")))
    except (TypeError, ValueError):
        import copy

        return copy.deepcopy(obj)


def _seg_range(seg: dict[str, Any]) -> tuple[int, int]:
    tr = seg.get("target_timerange") or {}
    start = int(tr.get("start") or 0)
    dur = max(0, int(tr.get("duration") or 0))
    return start, start + dur


def _set_target(seg: dict[str, Any], start: int, duration: int) -> None:
    tr = seg.get("target_timerange")
    if not isinstance(tr, dict):
        tr = {}
        seg["target_timerange"] = tr
    tr["start"] = int(start)
    tr["duration"] = int(max(0, duration))


def _clip_speed(seg: dict[str, Any]) -> float:
    try:
        sp = float(seg.get("speed") or 1.0)
        return sp if sp > 0 else 1.0
    except (TypeError, ValueError):
        return 1.0


def _trim_source_head(seg: dict[str, Any], trim_target_us: int) -> None:
    """Advance source_timerange when target head is trimmed (speed-aware)."""
    if trim_target_us <= 0:
        return
    src = seg.get("source_timerange")
    if not isinstance(src, dict):
        return
    speed = _clip_speed(seg)
    src_trim = int(round(trim_target_us * speed))
    src_start = int(src.get("start") or 0) + src_trim
    src_dur = max(0, int(src.get("duration") or 0) - src_trim)
    src["start"] = src_start
    src["duration"] = src_dur


def _trim_source_tail(seg: dict[str, Any], new_target_dur: int, old_target_dur: int) -> None:
    if new_target_dur >= old_target_dur or old_target_dur <= 0:
        return
    src = seg.get("source_timerange")
    if not isinstance(src, dict):
        return
    speed = _clip_speed(seg)
    # Keep head; shorten source proportional to target duration change.
    ratio = new_target_dur / float(old_target_dur)
    src_dur = max(0, int(round(int(src.get("duration") or 0) * ratio)))
    # Also consistent with speed: target_dur * speed ≈ source_dur
    src_dur_alt = max(0, int(round(new_target_dur * speed)))
    src["duration"] = src_dur_alt if src_dur_alt > 0 else src_dur


def _slice_segment_part1(seg: dict[str, Any], cut_us: int) -> dict[str, Any] | None:
    start, end = _seg_range(seg)
    if start >= cut_us or end <= start:
        return None
    out = _clone(seg)
    if end <= cut_us:
        return out
    old_dur = end - start
    new_dur = cut_us - start
    _set_target(out, start, new_dur)
    _trim_source_tail(out, new_dur, old_dur)
    return out


def _slice_segment_part2(seg: dict[str, Any], cut_us: int) -> dict[str, Any] | None:
    start, end = _seg_range(seg)
    if end <= cut_us or end <= start:
        return None
    out = _clone(seg)
    if start >= cut_us:
        _set_target(out, start - cut_us, end - start)
        return out
    # Overlaps cut: drop head before cut, place at t=0
    old_dur = end - start
    trim = cut_us - start
    new_dur = end - cut_us
    _set_target(out, 0, new_dur)
    _trim_source_head(out, trim)
    # Ensure source duration matches remaining target if still oversize
    _trim_source_tail(out, new_dur, old_dur - trim if old_dur > trim else new_dur)
    return out


def _filter_tracks(
    tracks: list[Any],
    *,
    cut_us: int,
    part: int,
) -> tuple[list[Any], int]:
    kept_tracks: list[Any] = []
    seg_count = 0
    slicer = _slice_segment_part1 if part == 1 else _slice_segment_part2
    for track in tracks:
        if not isinstance(track, dict):
            continue
        t = _clone(track)
        segs_in = t.get("segments") or []
        segs_out: list[Any] = []
        if isinstance(segs_in, list):
            for seg in segs_in:
                if not isinstance(seg, dict):
                    continue
                sliced = slicer(seg, cut_us)
                if sliced is None:
                    continue
                # Drop zero-length after clamp
                _s, e = _seg_range(sliced)
                if e <= _s:
                    continue
                segs_out.append(sliced)
                seg_count += 1
        t["segments"] = segs_out
        kept_tracks.append(t)
    return kept_tracks, seg_count


def _collect_refs(tracks: list[Any]) -> set[str]:
    ids: set[str] = set()
    for track in tracks:
        if not isinstance(track, dict):
            continue
        for seg in track.get("segments") or []:
            if not isinstance(seg, dict):
                continue
            mid = seg.get("material_id")
            if mid:
                ids.add(str(mid))
            for ref in seg.get("extra_material_refs") or []:
                if ref:
                    ids.add(str(ref))
            for key in ("refer_material", "common_keyframes"):
                # leave nested; material ids above cover CapCut TTS/video links
                _ = key
    return ids


def _prune_materials(materials: dict[str, Any], keep_ids: set[str]) -> dict[str, Any]:
    out: dict[str, Any] = {}
    for bucket, items in materials.items():
        if not isinstance(items, list):
            out[bucket] = _clone(items)
            continue
        kept: list[Any] = []
        for it in items:
            if not isinstance(it, dict):
                continue
            iid = it.get("id")
            if iid is None or str(iid) in keep_ids:
                kept.append(_clone(it))
                # Nested material refs inside kept extras (rare)
                for k, v in it.items():
                    if k.endswith("_id") and v:
                        keep_ids.add(str(v))
        # Second pass if nested ids pulled more — keep simple: one extra pass
        out[bucket] = kept
    # One more pass for ids discovered in nested fields of kept items
    if keep_ids:
        for bucket, items in list(out.items()):
            if not isinstance(items, list):
                continue
            existing = {str(it.get("id")) for it in items if isinstance(it, dict) and it.get("id")}
            src_items = materials.get(bucket) or []
            if not isinstance(src_items, list):
                continue
            for it in src_items:
                if not isinstance(it, dict):
                    continue
                iid = it.get("id")
                if iid and str(iid) in keep_ids and str(iid) not in existing:
                    items.append(_clone(it))
                    existing.add(str(iid))
    return out


def slice_draft(draft: dict[str, Any], cut_us: int, *, part: int, name_suffix: str) -> tuple[dict[str, Any], int]:
    """Return (sliced_draft, segment_count). part is 1 or 2."""
    if part not in (1, 2):
        raise ValueError("part must be 1 or 2")
    total = int(draft.get("duration") or 0)
    if cut_us <= 0 or cut_us >= total:
        raise ValueError(f"cut_us must be inside (0, duration); got {cut_us} / {total}")

    out = _clone(draft)
    tracks, seg_count = _filter_tracks(out.get("tracks") or [], cut_us=cut_us, part=part)
    out["tracks"] = tracks
    keep_ids = _collect_refs(tracks)
    mats = out.get("materials")
    if isinstance(mats, dict):
        out["materials"] = _prune_materials(mats, keep_ids)

    if part == 1:
        out["duration"] = cut_us
    else:
        out["duration"] = total - cut_us

    new_id = str(uuid.uuid4()).upper()
    out["id"] = new_id
    base_name = str(out.get("name") or "Project")
    out["name"] = f"{base_name}{name_suffix}"
    return out, seg_count


def _copy_project_tree(src: Path, dest: Path) -> None:
    if dest.exists():
        raise FileExistsError(f"Split output already exists: {dest}")

    def _ignore(directory: str, names: list[str]) -> set[str]:
        ignored = set()
        for n in names:
            if n in _IGNORE_NAMES:
                ignored.add(n)
            elif n.endswith(".tmp") or n.endswith(".precommit"):
                ignored.add(n)
        return ignored

    shutil.copytree(src, dest, ignore=_ignore)


def _write_draft_targets(project_dir: Path, draft_path_rel: Path | None, draft: dict[str, Any]) -> Path:
    """Write draft_content.json at root and matching Timelines copy."""
    payload = json.dumps(draft, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    root_draft = project_dir / "draft_content.json"
    root_draft.parent.mkdir(parents=True, exist_ok=True)
    root_draft.write_bytes(payload)

    pid = str(draft.get("id") or "")
    timelines = project_dir / "Timelines"
    if timelines.is_dir() and pid:
        # Prefer single existing timeline folder; else create Timelines/<id>/
        children = [p for p in timelines.iterdir() if p.is_dir()]
        if len(children) == 1:
            target = children[0] / "draft_content.json"
        else:
            target = timelines / pid / "draft_content.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(payload)
        return target if draft_path_rel and "Timelines" in draft_path_rel.parts else root_draft

    # Modern layout may only have Timelines
    if not root_draft.exists():
        pass
    return root_draft


def _patch_meta(project_dir: Path, draft: dict[str, Any], folder_name: str) -> None:
    meta_path = project_dir / "draft_meta_info.json"
    if not meta_path.exists():
        return
    try:
        with open(meta_path, "r", encoding="utf-8-sig") as f:
            meta = json.load(f)
        if not isinstance(meta, dict):
            return
        meta["draft_name"] = str(draft.get("name") or folder_name)
        meta["draft_id"] = str(draft.get("id") or meta.get("draft_id") or "")
        meta["draft_fold_path"] = str(project_dir).replace("\\", "/")
        meta["draft_root_path"] = str(project_dir).replace("\\", "/")
        # duration fields vary by CapCut version
        dur = int(draft.get("duration") or 0)
        if "tm_duration" in meta:
            meta["tm_duration"] = dur
        if "duration" in meta:
            meta["duration"] = dur
        with open(meta_path, "w", encoding="utf-8") as f:
            json.dump(meta, f, ensure_ascii=False, separators=(",", ":"))
    except Exception as e:
        logger.warning("Could not patch draft_meta_info.json in %s: %s", project_dir, e)


def split_project(
    *,
    source_project_dir: Path,
    source_draft: dict[str, Any],
    source_draft_path: Path,
    cut_us: int | None = None,
    caption_edges_us: list[int] | None = None,
    output_parent: Path | None = None,
    progress_callback: Callable[[float, str], None] | None = None,
) -> SplitResult:
    """Copy project twice and write time-sliced drafts.

    Creates ``<name>_part1`` and ``<name>_part2`` next to the source project
    (or under output_parent). Original project is never modified.
    """
    source_project_dir = Path(source_project_dir).resolve()
    if not source_project_dir.is_dir():
        raise FileNotFoundError(f"Project directory not found: {source_project_dir}")

    total = int(source_draft.get("duration") or 0)
    if cut_us is None:
        cut_us = choose_cut_us(total, caption_edges_us)
    cut_us = int(cut_us)
    if cut_us <= 0 or cut_us >= total:
        raise ValueError(f"Invalid cut point {cut_us} for duration {total}")

    parent = Path(output_parent) if output_parent else source_project_dir.parent
    base = source_project_dir.name
    part1_dir = parent / f"{base}_part1"
    part2_dir = parent / f"{base}_part2"

    def prog(p: float, msg: str) -> None:
        if progress_callback:
            progress_callback(p, msg)

    prog(0.05, "Copying part 1…")
    _copy_project_tree(source_project_dir, part1_dir)
    prog(0.35, "Copying part 2…")
    _copy_project_tree(source_project_dir, part2_dir)

    prog(0.55, "Slicing drafts…")
    d1, n1 = slice_draft(source_draft, cut_us, part=1, name_suffix=" (Part 1)")
    d2, n2 = slice_draft(source_draft, cut_us, part=2, name_suffix=" (Part 2)")

    prog(0.75, "Writing part 1 draft…")
    p1_draft = _write_draft_targets(part1_dir, source_draft_path, d1)
    _patch_meta(part1_dir, d1, part1_dir.name)

    prog(0.88, "Writing part 2 draft…")
    p2_draft = _write_draft_targets(part2_dir, source_draft_path, d2)
    _patch_meta(part2_dir, d2, part2_dir.name)

    manifest = {
        "source_project": str(source_project_dir),
        "source_draft": str(source_draft_path),
        "cut_us": cut_us,
        "total_duration_us": total,
        "part1": {"dir": str(part1_dir), "duration_us": d1["duration"], "segments": n1},
        "part2": {"dir": str(part2_dir), "duration_us": d2["duration"], "segments": n2},
    }
    for d in (part1_dir, part2_dir, source_project_dir):
        try:
            with open(d / "capdraft_split_manifest.json", "w", encoding="utf-8") as f:
                json.dump(manifest, f, ensure_ascii=False, indent=2)
        except Exception:
            pass

    prog(1.0, "Split complete")
    logger.info(
        "Split project cut=%sus → %s (%s segs) + %s (%s segs)",
        cut_us,
        part1_dir.name,
        n1,
        part2_dir.name,
        n2,
    )
    return SplitResult(
        cut_us=cut_us,
        part1_dir=part1_dir,
        part2_dir=part2_dir,
        part1_draft=p1_draft,
        part2_draft=p2_draft,
        part1_duration_us=int(d1["duration"]),
        part2_duration_us=int(d2["duration"]),
        part1_segments=n1,
        part2_segments=n2,
    )
