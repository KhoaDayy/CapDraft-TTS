"""Online updater for the Voice.json catalog."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime
import json
from pathlib import Path
import tempfile

import requests

from core.logger import logger
from .voice_catalog import VoiceCatalog


@dataclass(frozen=True)
class VoiceCatalogUpdateResult:
    path: Path
    url: str
    voice_count: int
    backup_path: Path | None


class VoiceCatalogUpdateError(RuntimeError):
    """Raised when a downloaded catalog is unavailable or invalid."""


def update_voice_catalog_from_url(
    *,
    url: str,
    destination: Path | str,
    timeout: int = 20,
) -> VoiceCatalogUpdateResult:
    """Download, validate, and atomically replace a local Voice.json file."""

    url = (url or "").strip()
    if not url:
        raise VoiceCatalogUpdateError("Voice catalog update URL is empty")

    dest = Path(destination)
    dest.parent.mkdir(parents=True, exist_ok=True)

    try:
        response = requests.get(
            url,
            timeout=timeout,
            headers={"Accept": "application/json,text/plain,*/*"},
        )
        response.raise_for_status()
    except requests.RequestException as exc:
        raise VoiceCatalogUpdateError(f"Cannot download voice catalog: {exc}") from exc

    text = response.text
    try:
        raw = json.loads(text)
    except json.JSONDecodeError as exc:
        raise VoiceCatalogUpdateError(f"Downloaded voice catalog is not valid JSON: {exc}") from exc

    if not isinstance(raw, list):
        raise VoiceCatalogUpdateError("Downloaded voice catalog must be a JSON array")

    with tempfile.NamedTemporaryFile(
        "w",
        encoding="utf-8",
        suffix=".json",
        delete=False,
        dir=str(dest.parent),
    ) as tmp:
        json.dump(raw, tmp, ensure_ascii=False, indent=2)
        tmp.write("\n")
        tmp_path = Path(tmp.name)

    backup_path: Path | None = None
    try:
        voices = VoiceCatalog(tmp_path).load()
        if not voices:
            raise VoiceCatalogUpdateError("Downloaded voice catalog has no usable voices")

        if dest.exists():
            stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
            backup_path = dest.with_name(f"{dest.stem}.{stamp}.bak{dest.suffix}")
            dest.replace(backup_path)

        tmp_path.replace(dest)
        logger.info("Updated voice catalog from %s with %s voices", url, len(voices))
        return VoiceCatalogUpdateResult(
            path=dest,
            url=url,
            voice_count=len(voices),
            backup_path=backup_path,
        )
    except Exception:
        if tmp_path.exists():
            tmp_path.unlink(missing_ok=True)
        if backup_path and backup_path.exists() and not dest.exists():
            backup_path.replace(dest)
        raise
