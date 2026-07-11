"""CapCut media path helpers (draftpath placeholder)."""

from __future__ import annotations

import re
import uuid
from pathlib import Path
from typing import Any

_PLACEHOLDER_RE = re.compile(r"##_draftpath_placeholder_([A-Fa-f0-9-]+)_##")


def find_draftpath_placeholder(draft: dict[str, Any]) -> str | None:
    """Reuse existing CapCut draftpath placeholder if present in materials."""
    materials = draft.get("materials") or {}
    for bucket, items in materials.items():
        if not isinstance(items, list):
            continue
        for it in items:
            if not isinstance(it, dict):
                continue
            for key in ("path", "media_path", "reverse_path", "intensifies_path"):
                val = it.get(key)
                if isinstance(val, str):
                    m = _PLACEHOLDER_RE.search(val)
                    if m:
                        return f"##_draftpath_placeholder_{m.group(1)}_##"
    # top-level path field
    p = draft.get("path")
    if isinstance(p, str):
        m = _PLACEHOLDER_RE.search(p)
        if m:
            return f"##_draftpath_placeholder_{m.group(1)}_##"
    return None


def ensure_draftpath_placeholder(draft: dict[str, Any]) -> str:
    existing = find_draftpath_placeholder(draft)
    if existing:
        return existing
    return f"##_draftpath_placeholder_{str(uuid.uuid4()).upper()}_##"


def capcut_text_reading_path(placeholder: str, dest_name: str) -> str:
    """Path CapCut resolves relative to project folder."""
    name = Path(dest_name).name
    return f"{placeholder}/textReading/{name}"


def list_draft_write_targets(draft_path: Path, project_directory: Path, project_id: str) -> list[Path]:
    """Root + matching Timelines/<id> copies CapCut may load."""
    targets: list[Path] = []
    seen: set[Path] = set()

    def add(p: Path):
        try:
            rp = p.resolve()
        except Exception:
            rp = p
        if rp in seen:
            return
        seen.add(rp)
        targets.append(p)

    add(draft_path)
    root_draft = project_directory / "draft_content.json"
    add(root_draft)

    pid = (project_id or "").strip()
    timelines = project_directory / "Timelines"
    if pid:
        add(timelines / pid / "draft_content.json")
    # If the opened file is already under Timelines, keep it (already added)
    # Also mirror into any single-timeline folder when project_id mismatches case
    if timelines.is_dir() and pid:
        for child in timelines.iterdir():
            if child.is_dir() and child.name.upper() == pid.upper():
                add(child / "draft_content.json")

    return targets
