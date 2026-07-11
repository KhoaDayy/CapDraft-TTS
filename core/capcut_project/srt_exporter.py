"""Convert CapCut caption rows to SubRip subtitle files."""

from __future__ import annotations

from pathlib import Path
from typing import Iterable

from .models import CaptionRow


def _srt_timestamp(microseconds: int) -> str:
    total_ms = max(0, int(round(microseconds / 1000.0)))
    hours, remainder = divmod(total_ms, 3_600_000)
    minutes, remainder = divmod(remainder, 60_000)
    seconds, milliseconds = divmod(remainder, 1000)
    return f"{hours:02d}:{minutes:02d}:{seconds:02d},{milliseconds:03d}"


def render_captions_srt(captions: Iterable[CaptionRow]) -> str:
    blocks: list[str] = []
    ordered = sorted(captions, key=lambda caption: (caption.start_us, caption.index))
    for caption in ordered:
        text = caption.text.replace("\r\n", "\n").replace("\r", "\n").strip()
        if not text:
            continue
        start_us = max(0, caption.start_us)
        end_us = max(start_us, start_us + max(0, caption.duration_us))
        blocks.append(
            f"{len(blocks) + 1}\n"
            f"{_srt_timestamp(start_us)} --> {_srt_timestamp(end_us)}\n"
            f"{text}"
        )
    return "\n\n".join(blocks) + ("\n" if blocks else "")


def export_captions_srt(captions: Iterable[CaptionRow], output_path: Path | str) -> int:
    caption_list = list(captions)
    rendered = render_captions_srt(caption_list)
    path = Path(output_path)
    path.write_text(rendered, encoding="utf-8-sig", newline="\n")
    return sum(1 for caption in caption_list if caption.text.strip())
