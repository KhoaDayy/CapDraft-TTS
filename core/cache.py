"""TTS audio cache."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import tempfile
from pathlib import Path
from typing import Optional

from core.config import AppConfig
from core.logger import logger


class TtsCache:
    def __init__(self):
        self.cache_dir = Path(AppConfig().cache_path)
        self.cache_dir.mkdir(parents=True, exist_ok=True)

    def generate_hash(self, text: str, voice: str, rate: float, resource_id: str) -> str:
        raw_key = f"{text}|{voice}|{rate:.2f}|{resource_id}"
        return hashlib.sha256(raw_key.encode("utf-8")).hexdigest()

    def _meta_path(self, file_hash: str) -> Path:
        return self.cache_dir / f"{file_hash}.meta.json"

    def get_cached_file(
        self, text: str, voice: str, rate: float, resource_id: str
    ) -> tuple[Optional[Path], Optional[float]]:
        file_hash = self.generate_hash(text, voice, rate, resource_id)
        cached_path = self.cache_dir / f"{file_hash}.mp3"
        if cached_path.exists() and cached_path.stat().st_size > 0:
            return cached_path, self._read_duration(file_hash)
        if cached_path.exists():
            logger.warning("Ignoring empty/corrupted TTS cache file: %s", cached_path)
        return None, None

    def _read_duration(self, file_hash: str) -> Optional[float]:
        meta = self._meta_path(file_hash)
        if not meta.exists():
            return None
        try:
            with open(meta, "r", encoding="utf-8") as f:
                data = json.load(f)
            return float(data.get("duration", 0.0))
        except Exception:
            return None

    def _unique_part_path(self, final_path: Path) -> Path:
        """Create an empty temp file next to ``final_path`` that only this writer owns.

        Several threads can cache the same key at once (duplicate caption
        texts in parallel chunks), so a shared ``<name>.part`` would be
        clobbered or replaced out from under another writer.
        """
        fd, name = tempfile.mkstemp(dir=self.cache_dir, prefix=final_path.name + ".", suffix=".part")
        os.close(fd)
        return Path(name)

    @staticmethod
    def _discard(path: Optional[Path]):
        if path is None:
            return
        try:
            path.unlink(missing_ok=True)
        except Exception:
            pass

    def _write_duration(self, file_hash: str, duration: float):
        meta = self._meta_path(file_hash)
        part_path: Optional[Path] = None
        try:
            part_path = self._unique_part_path(meta)
            with open(part_path, "w", encoding="utf-8") as f:
                json.dump({"duration": duration}, f)
            os.replace(part_path, meta)
        except Exception as e:
            self._discard(part_path)
            logger.warning("Failed to write cache metadata %s: %s", meta, e)

    def cache_audio(
        self,
        src_file_path: Path,
        text: str,
        voice: str,
        rate: float,
        resource_id: str,
        duration: float = 0.0,
    ) -> Path:
        src = Path(src_file_path)
        if not src.exists() or src.stat().st_size <= 0:
            raise ValueError(f"Refuse to cache empty/missing audio: {src}")
        file_hash = self.generate_hash(text, voice, rate, resource_id)
        dest_path = self.cache_dir / f"{file_hash}.mp3"
        part_path: Optional[Path] = None
        try:
            part_path = self._unique_part_path(dest_path)
            shutil.copy2(src, part_path)
            if part_path.stat().st_size <= 0:
                raise ValueError(f"Cache copy produced empty file: {part_path}")
            try:
                os.replace(part_path, dest_path)
            except OSError:
                # On Windows, replacing a file another writer is replacing (or
                # a reader holds open) can fail. Content for the same key is
                # equivalent, so a complete entry already in place is a success.
                if not (dest_path.exists() and dest_path.stat().st_size > 0):
                    raise
                self._discard(part_path)
        except Exception as e:
            self._discard(part_path)
            logger.error("Failed to copy file %s to cache: %s", src, e)
            raise
        self._write_duration(file_hash, duration)
        return dest_path
