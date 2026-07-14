"""Merge exported CapCut videos with ffmpeg (stream-copy when possible)."""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Callable


@dataclass(frozen=True)
class MergeResult:
    output_path: Path
    used_reencode: bool
    ffmpeg_path: str


def resolve_ffmpeg(explicit: str | None = None) -> str:
    """Find ffmpeg executable. Prefer explicit path, then PATH, then next to ffprobe."""
    candidates: list[str] = []
    if explicit and str(explicit).strip():
        candidates.append(str(explicit).strip())
    which = shutil.which("ffmpeg")
    if which:
        candidates.append(which)
    # Common Windows winget / chocolatey locations are already on PATH via which.
    # If only ffprobe is configured, try sibling ffmpeg.
    try:
        from core.config import AppConfig

        probe = AppConfig().ffprobe_path
        if probe:
            p = Path(probe)
            if p.name.lower().startswith("ffprobe"):
                sibling = p.with_name(p.name.replace("ffprobe", "ffmpeg").replace("FFPROBE", "ffmpeg"))
                candidates.append(str(sibling))
            parent = p if p.is_dir() else p.parent
            candidates.append(str(parent / "ffmpeg.exe"))
            candidates.append(str(parent / "ffmpeg"))
    except Exception:
        pass

    for c in candidates:
        if not c:
            continue
        path = Path(c)
        if path.is_file():
            return str(path.resolve())
        w = shutil.which(c)
        if w:
            return w
    raise FileNotFoundError(
        "ffmpeg not found. Install ffmpeg and ensure it is on PATH, "
        "or set ffmpeg_path in config.json."
    )


def _run(cmd: list[str], *, timeout: int = 3600) -> subprocess.CompletedProcess[str]:
    creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    return subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        errors="replace",
        timeout=timeout,
        creationflags=creationflags,
    )


def merge_videos(
    inputs: list[Path | str],
    output: Path | str,
    *,
    ffmpeg_path: str | None = None,
    reencode: bool = False,
    progress_callback: Callable[[float, str], None] | None = None,
) -> MergeResult:
    """Concatenate videos in order via ffmpeg concat demuxer.

    Tries ``-c copy`` first (fast, no quality loss). On failure, re-encodes
    with libx264 + aac unless reencode was already requested.
    """
    paths = [Path(p) for p in inputs]
    if len(paths) < 2:
        raise ValueError("Need at least 2 videos to merge")
    for p in paths:
        if not p.is_file():
            raise FileNotFoundError(f"Video not found: {p}")

    out = Path(output)
    if out.suffix.lower() not in {".mp4", ".mov", ".mkv", ".m4v"}:
        out = out.with_suffix(".mp4")
    out.parent.mkdir(parents=True, exist_ok=True)

    ff = resolve_ffmpeg(ffmpeg_path)

    def prog(p: float, msg: str) -> None:
        if progress_callback:
            progress_callback(p, msg)

    # concat demuxer list — paths escaped for ffmpeg
    list_file = None
    try:
        prog(0.1, "Preparing concat list…")
        fd, list_path = tempfile.mkstemp(prefix="capdraft_concat_", suffix=".txt")
        os.close(fd)
        list_file = Path(list_path)
        lines = []
        for p in paths:
            # ffmpeg concat: escape single quotes
            ap = str(p.resolve()).replace("\\", "/").replace("'", r"'\''")
            lines.append(f"file '{ap}'")
        list_file.write_text("\n".join(lines) + "\n", encoding="utf-8")

        def _concat(copy: bool) -> subprocess.CompletedProcess[str]:
            cmd = [
                ff,
                "-y",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                str(list_file),
            ]
            if copy:
                cmd += ["-c", "copy"]
            else:
                cmd += [
                    "-c:v",
                    "libx264",
                    "-preset",
                    "veryfast",
                    "-crf",
                    "18",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "192k",
                    "-movflags",
                    "+faststart",
                ]
            cmd.append(str(out))
            return _run(cmd)

        used_reencode = bool(reencode)
        if not reencode:
            prog(0.3, "Merging (stream copy)…")
            result = _concat(copy=True)
            if result.returncode != 0 or not out.is_file() or out.stat().st_size < 32:
                prog(0.5, "Stream copy failed — re-encoding…")
                if out.exists():
                    try:
                        out.unlink()
                    except Exception:
                        pass
                result = _concat(copy=False)
                used_reencode = True
        else:
            prog(0.3, "Merging (re-encode)…")
            result = _concat(copy=False)

        if result.returncode != 0 or not out.is_file():
            err = (result.stderr or result.stdout or "ffmpeg failed").strip()
            raise RuntimeError(err[-2000:] if len(err) > 2000 else err)

        prog(1.0, "Merge complete")
        return MergeResult(output_path=out, used_reencode=used_reencode, ffmpeg_path=ff)
    finally:
        if list_file is not None:
            try:
                list_file.unlink()
            except Exception:
                pass
