"""TTS audio cache."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
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

    def _write_duration(self, file_hash: str, duration: float):
        meta = self._meta_path(file_hash)
        try:
            with open(meta, "w", encoding="utf-8") as f:
                json.dump({"duration": duration}, f)
        except Exception as e:
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
        part_path = Path(str(dest_path) + ".part")
        try:
            shutil.copy2(src, part_path)
            if part_path.stat().st_size <= 0:
                raise ValueError(f"Cache copy produced empty file: {part_path}")
            os.replace(part_path, dest_path)
        except Exception as e:
            try:
                if part_path.exists():
                    part_path.unlink()
            except Exception:
                pass
            logger.error("Failed to copy file %s to cache: %s", src, e)
            raise
        self._write_duration(file_hash, duration)
        return dest_path
