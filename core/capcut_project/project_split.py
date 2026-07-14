"""Split a CapCut draft into two time-halves CapCut can open/export.

Handles flat drafts and combination/nested ``materials.drafts`` (AutoVideo style).
"""

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
    "capdraft_split_manifest.json",
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
    min_half = max(1_000_000, duration_us // 50)
    if best < min_half or (duration_us - best) < min_half:
        return mid
    return best


def _new_id() -> str:
    return str(uuid.uuid4()).upper()


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
    if trim_target_us <= 0:
        return
    src = seg.get("source_timerange")
    if not isinstance(src, dict):
        return
    speed = _clip_speed(seg)
    src_trim = int(round(trim_target_us * speed))
    src["start"] = int(src.get("start") or 0) + src_trim
    src["duration"] = max(0, int(src.get("duration") or 0) - src_trim)


def _trim_source_tail(seg: dict[str, Any], new_target_dur: int, old_target_dur: int) -> None:
    if new_target_dur >= old_target_dur or old_target_dur <= 0:
        return
    src = seg.get("source_timerange")
    if not isinstance(src, dict):
        return
    speed = _clip_speed(seg)
    src_dur_alt = max(0, int(round(new_target_dur * speed)))
    if src_dur_alt > 0:
        src["duration"] = src_dur_alt
    else:
        ratio = new_target_dur / float(old_target_dur)
        src["duration"] = max(0, int(round(int(src.get("duration") or 0) * ratio)))


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
    old_dur = end - start
    trim = cut_us - start
    new_dur = end - cut_us
    _set_target(out, 0, new_dur)
    _trim_source_head(out, trim)
    remain_old = old_dur - trim if old_dur > trim else new_dur
    _trim_source_tail(out, new_dur, remain_old)
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
        segs_out: list[Any] = []
        for seg in t.get("segments") or []:
            if not isinstance(seg, dict):
                continue
            sliced = slicer(seg, cut_us)
            if sliced is None:
                continue
            s0, e0 = _seg_range(sliced)
            if e0 <= s0:
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
    return ids


def _prune_materials(materials: dict[str, Any], keep_ids: set[str]) -> dict[str, Any]:
    """Drop unreferenced list items; always keep combination drafts still linked."""
    out: dict[str, Any] = {}
    # Expand keep set via nested draft materials that remain referenced
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
                kept.append(it)  # already a clone tree from parent slice
        out[bucket] = kept
    return out


def _material_duration_us(item: dict[str, Any], new_duration: int) -> None:
    if "duration" in item and item.get("duration") is not None:
        try:
            # Only shrink long timeline-spanning materials (video/audio full clips)
            old = int(item.get("duration") or 0)
            if old > 0:
                item["duration"] = min(old, int(new_duration)) if old >= new_duration else old
        except (TypeError, ValueError):
            pass


def slice_draft(
    draft: dict[str, Any],
    cut_us: int,
    *,
    part: int,
    name_suffix: str = "",
    new_content_id: str | None = None,
    rename_content: bool = True,
) -> tuple[dict[str, Any], int]:
    """Return (sliced_draft, segment_count). part is 1 or 2.

    Recursively slices nested combination drafts under materials.drafts[].draft.
    """
    if part not in (1, 2):
        raise ValueError("part must be 1 or 2")
    total = int(draft.get("duration") or 0)
    if cut_us <= 0 or (total > 0 and cut_us >= total):
        # Nested drafts sometimes slightly longer/shorter — clamp
        if total > 1 and 0 < cut_us < total:
            pass
        elif total > 1:
            raise ValueError(f"cut_us must be inside (0, duration); got {cut_us} / {total}")
        else:
            raise ValueError("Project duration too short to split")

    out = _clone(draft)
    tracks, seg_count = _filter_tracks(out.get("tracks") or [], cut_us=cut_us, part=part)
    out["tracks"] = tracks
    keep_ids = _collect_refs(tracks)

    new_dur = cut_us if part == 1 else max(0, total - cut_us)
    mats = out.get("materials")
    sliced_combo_ids: set[str] = set()
    if isinstance(mats, dict):
        # Slice nested combination drafts first (before prune)
        nested_drafts = mats.get("drafts")
        if isinstance(nested_drafts, list):
            new_list: list[Any] = []
            for item in nested_drafts:
                if not isinstance(item, dict):
                    continue
                iid = item.get("id")
                nested = item.get("draft")
                if isinstance(nested, dict) and (iid is None or str(iid) in keep_ids or not keep_ids):
                    nd, nseg = slice_draft(
                        nested,
                        cut_us,
                        part=part,
                        name_suffix=name_suffix,
                        new_content_id=None,
                        rename_content=False,  # keep nested id = subdraft folder
                    )
                    item = _clone(item)
                    item["draft"] = nd
                    seg_count += nseg
                    new_list.append(item)
                    if iid:
                        keep_ids.add(str(iid))
                        sliced_combo_ids.add(str(iid))
                elif iid is None or str(iid) in keep_ids:
                    new_list.append(_clone(item))
            mats["drafts"] = new_list

        pruned = _prune_materials(mats, keep_ids)
        out["materials"] = pruned
        mats = pruned

        # Align full-span video/audio material duration with new timeline
        for bucket in ("videos", "audios"):
            for it in mats.get(bucket) or []:
                if isinstance(it, dict):
                    _material_duration_us(it, new_dur)

    # Nested combo timelines were rewritten to start at 0 — outer segments that
    # reference them must not keep a source offset into the old full draft.
    if sliced_combo_ids:
        for track in out.get("tracks") or []:
            if not isinstance(track, dict):
                continue
            for seg in track.get("segments") or []:
                if not isinstance(seg, dict):
                    continue
                refs = {str(x) for x in (seg.get("extra_material_refs") or []) if x}
                mid = str(seg.get("material_id") or "")
                if not (refs & sliced_combo_ids or mid in sliced_combo_ids):
                    continue
                src = seg.get("source_timerange")
                if not isinstance(src, dict):
                    src = {}
                    seg["source_timerange"] = src
                tgt = seg.get("target_timerange") or {}
                tdur = int(tgt.get("duration") or new_dur)
                src["start"] = 0
                src["duration"] = tdur
                # Placeholder video material that wraps the combination
                if isinstance(mats, dict):
                    for it in mats.get("videos") or []:
                        if isinstance(it, dict) and str(it.get("id") or "") == mid:
                            it["duration"] = tdur

    if part == 1:
        out["duration"] = cut_us
    else:
        out["duration"] = max(0, total - cut_us)

    if rename_content:
        out["id"] = new_content_id or _new_id()
        base_name = str(out.get("name") or "").strip()
        if name_suffix:
            out["name"] = f"{base_name}{name_suffix}" if base_name else name_suffix.strip()
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


def _write_json(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    path.write_bytes(payload)


def _write_nested_subdrafts(project_dir: Path, draft: dict[str, Any]) -> None:
    """Sync materials.drafts[].draft into subdraft/<id>/draft_content.json when present."""
    mats = draft.get("materials") or {}
    for item in mats.get("drafts") or []:
        if not isinstance(item, dict):
            continue
        nested = item.get("draft")
        if not isinstance(nested, dict):
            continue
        nid = str(nested.get("id") or "").strip()
        if not nid:
            continue
        sub = project_dir / "subdraft" / nid / "draft_content.json"
        if sub.parent.is_dir() or (project_dir / "subdraft").is_dir():
            _write_json(sub, nested)
            # companion bak CapCut sometimes reads
            try:
                _write_json(sub.with_name("draft_content.json.bak"), nested)
            except Exception:
                pass


def _rename_timeline_folder(project_dir: Path, old_content_id: str, new_content_id: str) -> Path | None:
    timelines = project_dir / "Timelines"
    if not timelines.is_dir():
        return None
    new_dir = timelines / new_content_id
    # Prefer exact old id folder
    candidates: list[Path] = []
    if old_content_id:
        p = timelines / old_content_id
        if p.is_dir():
            candidates.append(p)
    candidates.extend(
        sorted([p for p in timelines.iterdir() if p.is_dir() and p.name != new_content_id], key=lambda x: x.name)
    )
    src = candidates[0] if candidates else None
    if src is None:
        new_dir.mkdir(parents=True, exist_ok=True)
        return new_dir
    if src.resolve() == new_dir.resolve():
        return new_dir
    if new_dir.exists():
        # Already present — remove empty conflict only if different
        if src != new_dir:
            shutil.rmtree(new_dir, ignore_errors=True)
    src.rename(new_dir)
    # Drop other leftover timeline dirs (single-timeline projects)
    for p in list(timelines.iterdir()):
        if p.is_dir() and p.name != new_content_id:
            try:
                shutil.rmtree(p)
            except Exception as e:
                logger.warning("Could not remove old timeline %s: %s", p, e)
    return new_dir


def _write_draft_targets(
    project_dir: Path,
    draft: dict[str, Any],
    *,
    old_content_id: str,
) -> Path:
    """Write root + Timelines/<content_id> drafts; rename timeline folder to new id."""
    content_id = str(draft.get("id") or "")
    tl_dir = _rename_timeline_folder(project_dir, old_content_id, content_id)
    root_draft = project_dir / "draft_content.json"
    _write_json(root_draft, draft)
    # companions
    try:
        _write_json(project_dir / "draft_content.json.bak", draft)
    except Exception:
        pass
    if tl_dir is not None:
        tl_draft = tl_dir / "draft_content.json"
        _write_json(tl_draft, draft)
        try:
            _write_json(tl_dir / "draft_content.json.bak", draft)
        except Exception:
            pass
    _write_nested_subdrafts(project_dir, draft)
    return root_draft


def _patch_meta(
    project_dir: Path,
    *,
    content_draft: dict[str, Any],
    folder_name: str,
    source_meta_id: str | None,
) -> str:
    """Patch draft_meta_info.json. Returns meta draft_id used.

    CapCut keeps meta.draft_id separate from draft_content.json id (timeline id).
    root_meta_info.draft_id must match this meta.draft_id.
    """
    meta_path = project_dir / "draft_meta_info.json"
    meta_id = _new_id()
    if not meta_path.exists():
        logger.warning("draft_meta_info.json missing in %s — CapCut may not open this project", project_dir)
        return meta_id
    try:
        with open(meta_path, "r", encoding="utf-8-sig") as f:
            meta = json.load(f)
        if not isinstance(meta, dict):
            return meta_id
        # Always new meta id so CapCut treats this as a distinct project
        meta["draft_id"] = meta_id
        meta["draft_name"] = folder_name
        fold = _norm_fold(project_dir.resolve())
        root = _norm_fold(project_dir.resolve().parent)
        meta["draft_fold_path"] = fold
        meta["draft_root_path"] = root
        dur = int(content_draft.get("duration") or 0)
        meta["tm_duration"] = dur
        if "duration" in meta:
            meta["duration"] = dur
        meta["draft_is_invisible"] = False
        # Relative cover name (CapCut resolves under fold path)
        if not meta.get("draft_cover"):
            meta["draft_cover"] = "draft_cover.jpg"
        _write_json(meta_path, meta)
        # Sanity: meta file must exist and parse after write
        if not meta_path.is_file() or meta_path.stat().st_size < 10:
            raise RuntimeError(f"draft_meta_info.json write failed: {meta_path}")
    except Exception as e:
        logger.warning("Could not patch draft_meta_info.json in %s: %s", project_dir, e)
    return meta_id


def _capcut_join(fold_fwd: str, filename: str) -> str:
    """CapCut root_meta uses mixed separators: 'C:/.../folder\\\\file'."""
    return f"{fold_fwd.rstrip('/')}\\{filename}"


def _norm_fold(path: str | Path) -> str:
    return str(path).replace("\\", "/").rstrip("/")


def _purge_stale_root_meta(root_meta: Path) -> int:
    """Drop all_draft_store rows whose draft_fold_path folder is missing (ghosts)."""
    if not root_meta.is_file():
        return 0
    try:
        data = json.loads(root_meta.read_text(encoding="utf-8-sig"))
        if not isinstance(data, dict):
            return 0
        store = data.get("all_draft_store")
        if not isinstance(store, list):
            return 0
        kept: list[Any] = []
        removed = 0
        for item in store:
            if not isinstance(item, dict):
                continue
            fold = str(item.get("draft_fold_path") or "")
            if fold and Path(fold).is_dir() and (Path(fold) / "draft_content.json").is_file():
                kept.append(item)
            else:
                removed += 1
        if removed:
            data["all_draft_store"] = kept
            if isinstance(data.get("draft_ids"), int):
                data["draft_ids"] = len(kept)
            _write_json(root_meta, data)
            logger.info("Purged %s stale CapCut root_meta entries", removed)
        return removed
    except Exception as e:
        logger.warning("Could not purge stale root_meta: %s", e)
        return 0


def _register_root_meta(
    project_dir: Path,
    *,
    meta_id: str,
    folder_name: str,
    duration_us: int,
    source_project_dir: Path,
) -> None:
    """Add/update entry in parent root_meta_info.json so CapCut lists the project."""
    root_meta = project_dir.parent / "root_meta_info.json"
    if not root_meta.is_file():
        return
    try:
        # Drop ghost rows (missing folders) so CapCut doesn't show unclickable cards
        _purge_stale_root_meta(root_meta)
        data = json.loads(root_meta.read_text(encoding="utf-8-sig"))
        if not isinstance(data, dict):
            return
        store = data.get("all_draft_store")
        if not isinstance(store, list):
            store = []
            data["all_draft_store"] = store

        fold = _norm_fold(project_dir.resolve())
        root = _norm_fold(project_dir.resolve().parent)
        src_fold = _norm_fold(source_project_dir.resolve())

        # Template from source entry when possible (preserves CapCut field set)
        template: dict[str, Any] | None = None
        for item in store:
            if isinstance(item, dict) and _norm_fold(item.get("draft_fold_path") or "") == src_fold:
                template = _clone(item)
                break
        if template is None:
            for item in store:
                if isinstance(item, dict):
                    template = _clone(item)
                    break
        if template is None:
            template = {}

        import time

        now_us = int(time.time() * 1_000_000)
        template["draft_id"] = meta_id
        template["draft_name"] = folder_name
        template["draft_fold_path"] = fold
        template["draft_root_path"] = root
        template["tm_duration"] = int(duration_us)
        if "duration" in template:
            template["duration"] = int(duration_us)
        # Match CapCut path style used by working projects
        template["draft_json_file"] = _capcut_join(fold, "draft_content.json")
        template["draft_cover"] = _capcut_join(fold, "draft_cover.jpg")
        template["draft_is_invisible"] = False
        template["streaming_edit_draft_ready"] = True
        template["tm_draft_removed"] = 0
        template["tm_draft_modified"] = now_us
        if not template.get("tm_draft_create"):
            template["tm_draft_create"] = now_us

        # Replace existing same fold / same name or append
        new_store: list[Any] = []
        replaced = False
        for item in store:
            if not isinstance(item, dict):
                continue
            item_fold = _norm_fold(item.get("draft_fold_path") or "")
            item_name = str(item.get("draft_name") or "")
            if item_fold == fold or item_name == folder_name:
                if not replaced:
                    new_store.append(template)
                    replaced = True
                # skip duplicates
                continue
            new_store.append(item)
        if not replaced:
            new_store.insert(0, template)
        data["all_draft_store"] = new_store

        # CapCut uses draft_ids as an int counter (not a list) on this install
        ids = data.get("draft_ids")
        if isinstance(ids, list):
            if meta_id not in ids:
                ids.insert(0, meta_id)
            data["draft_ids"] = ids
        else:
            data["draft_ids"] = len(new_store)

        _write_json(root_meta, data)
        logger.info("Registered %s in root_meta_info.json", folder_name)
    except Exception as e:
        logger.warning("Could not update root_meta_info.json: %s", e)


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
    """Copy project twice and write time-sliced drafts (nested combinations included)."""
    source_project_dir = Path(source_project_dir).resolve()
    if not source_project_dir.is_dir():
        raise FileNotFoundError(f"Project directory not found: {source_project_dir}")

    total = int(source_draft.get("duration") or 0)
    # Caption edges from nested combination if top-level has none
    edges = list(caption_edges_us or [])
    if not edges:
        edges = _caption_edges_from_draft(source_draft)
    if cut_us is None:
        cut_us = choose_cut_us(total, edges)
    cut_us = int(cut_us)
    if cut_us <= 0 or cut_us >= total:
        raise ValueError(f"Invalid cut point {cut_us} for duration {total}")

    parent = Path(output_parent) if output_parent else source_project_dir.parent
    base = source_project_dir.name
    part1_dir = parent / f"{base}_part1"
    part2_dir = parent / f"{base}_part2"

    old_content_id = str(source_draft.get("id") or "")
    source_meta_id = None
    meta_path = source_project_dir / "draft_meta_info.json"
    if meta_path.is_file():
        try:
            sm = json.loads(meta_path.read_text(encoding="utf-8-sig"))
            if isinstance(sm, dict):
                source_meta_id = str(sm.get("draft_id") or "") or None
        except Exception:
            pass

    def prog(p: float, msg: str) -> None:
        if progress_callback:
            progress_callback(p, msg)

    # Remove previous broken parts + ghost CapCut library entries first
    root_meta = parent / "root_meta_info.json"
    _purge_stale_root_meta(root_meta)
    for d in (part1_dir, part2_dir):
        if d.exists():
            shutil.rmtree(d)
            logger.info("Removed previous split output: %s", d)

    prog(0.05, "Copying part 1…")
    _copy_project_tree(source_project_dir, part1_dir)
    prog(0.30, "Copying part 2…")
    _copy_project_tree(source_project_dir, part2_dir)

    prog(0.50, "Slicing drafts (incl. nested)…")
    d1, n1 = slice_draft(
        source_draft,
        cut_us,
        part=1,
        name_suffix="",
        new_content_id=_new_id(),
        rename_content=True,
    )
    d1["name"] = f"{base}_part1"
    d2, n2 = slice_draft(
        source_draft,
        cut_us,
        part=2,
        name_suffix="",
        new_content_id=_new_id(),
        rename_content=True,
    )
    d2["name"] = f"{base}_part2"

    prog(0.70, "Writing part 1…")
    p1_draft = _write_draft_targets(part1_dir, d1, old_content_id=old_content_id)
    meta1 = _patch_meta(
        part1_dir, content_draft=d1, folder_name=part1_dir.name, source_meta_id=source_meta_id
    )
    _register_root_meta(
        part1_dir,
        meta_id=meta1,
        folder_name=part1_dir.name,
        duration_us=int(d1["duration"]),
        source_project_dir=source_project_dir,
    )

    prog(0.88, "Writing part 2…")
    p2_draft = _write_draft_targets(part2_dir, d2, old_content_id=old_content_id)
    meta2 = _patch_meta(
        part2_dir, content_draft=d2, folder_name=part2_dir.name, source_meta_id=source_meta_id
    )
    _register_root_meta(
        part2_dir,
        meta_id=meta2,
        folder_name=part2_dir.name,
        duration_us=int(d2["duration"]),
        source_project_dir=source_project_dir,
    )

    manifest = {
        "source_project": str(source_project_dir),
        "source_draft": str(source_draft_path),
        "cut_us": cut_us,
        "total_duration_us": total,
        "part1": {
            "dir": str(part1_dir),
            "duration_us": d1["duration"],
            "segments": n1,
            "content_id": d1.get("id"),
            "meta_id": meta1,
        },
        "part2": {
            "dir": str(part2_dir),
            "duration_us": d2["duration"],
            "segments": n2,
            "content_id": d2.get("id"),
            "meta_id": meta2,
        },
    }
    for d in (part1_dir, part2_dir):
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


def _caption_edges_from_draft(draft: dict[str, Any]) -> list[int]:
    """Collect subtitle start times from top-level and nested combination drafts."""
    edges: list[int] = []

    def walk(d: dict[str, Any]) -> None:
        mats = d.get("materials") or {}
        texts = {
            t.get("id"): t
            for t in (mats.get("texts") or [])
            if isinstance(t, dict) and t.get("id")
        }
        for track in d.get("tracks") or []:
            if not isinstance(track, dict) or track.get("type") != "text":
                continue
            for seg in track.get("segments") or []:
                if not isinstance(seg, dict):
                    continue
                mat = texts.get(seg.get("material_id"))
                if mat is not None and mat.get("type") not in (None, "subtitle", "text"):
                    continue
                tr = seg.get("target_timerange") or {}
                start = int(tr.get("start") or 0)
                if start > 0:
                    edges.append(start)
        for item in mats.get("drafts") or []:
            if isinstance(item, dict) and isinstance(item.get("draft"), dict):
                walk(item["draft"])

    walk(draft)
    return edges
