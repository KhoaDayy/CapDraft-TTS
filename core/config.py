"""App configuration (standalone CapDraft TTS)."""

from __future__ import annotations

import copy
import json
import os
import sys
from pathlib import Path

from core.capcut_project.voice_catalog import DEFAULT_VOICE_CATALOG_URL

APP_ROOT = (
    Path(sys.executable).resolve().parent
    if getattr(sys, "frozen", False)
    else Path(__file__).resolve().parents[1]
)

DEFAULT_CONFIG = {
    "ffprobe_path": "ffprobe",
    "ffmpeg_path": "ffmpeg",
    "capcut_tts_path": "external/capcut-tts-api",
    "device_json_path": "external/capcut-tts-api/device.json",
    "voice_catalog_url": DEFAULT_VOICE_CATALOG_URL,
    "default_voice": "BV074_streaming",
    "default_resource_id": "7102355709945188865",
    "default_rate": 1.0,
    "default_clip_speed": 1.0,
    "tts_chunk_size": 25,
    "tts_parallel_chunks": 4,
    "tts_poll_interval_sec": 1.0,
    "tts_download_workers": 8,
    "cache_path": "cache",
    "project_output_path": "projects",
    "max_backups": 10,
}


class AppConfig:
    _instance = None

    def __new__(cls, *args, **kwargs):
        if not cls._instance:
            cls._instance = super().__new__(cls)
            cls._instance._load()
        return cls._instance

    def _load(self):
        self.app_root = APP_ROOT
        self.config_path = self.resolve_app_path("config.json")
        self._data = copy.deepcopy(DEFAULT_CONFIG)
        if self.config_path.exists():
            try:
                with open(self.config_path, "r", encoding="utf-8-sig") as f:
                    loaded = json.load(f)
                self._recursive_merge(self._data, loaded)
            except Exception as e:
                print(f"Error loading config.json: {e}")
        self._normalize_legacy_keys()
        if not self.config_path.exists():
            self.save()

    def _normalize_legacy_keys(self):
        # Prefer explicit URL; fall back to old update-url key; drop local path.
        legacy_url = self._data.pop("voice_catalog_update_url", None)
        self._data.pop("voice_catalog_path", None)
        if not str(self._data.get("voice_catalog_url") or "").strip() and legacy_url:
            self._data["voice_catalog_url"] = legacy_url
        if not str(self._data.get("voice_catalog_url") or "").strip():
            self._data["voice_catalog_url"] = DEFAULT_VOICE_CATALOG_URL

    @staticmethod
    def _recursive_merge(base: dict, override: dict):
        for key, value in override.items():
            if key in base and isinstance(base[key], dict) and isinstance(value, dict):
                AppConfig._recursive_merge(base[key], value)
            else:
                base[key] = value

    def save(self):
        try:
            with open(self.config_path, "w", encoding="utf-8") as f:
                json.dump(self._data, f, indent=4, ensure_ascii=False)
        except Exception as e:
            print(f"Error saving config.json: {e}")

    def get(self, key, default=None):
        return self._data.get(key, default)

    def set(self, key, value):
        self._data[key] = value
        self.save()

    def resolve_app_path(self, path_value: str | os.PathLike) -> Path:
        path = Path(path_value)
        if path.is_absolute():
            return path
        return self.app_root / path

    @property
    def cache_path(self) -> str:
        return str(self.resolve_app_path(self.get("cache_path")))

    @property
    def cache_dir(self) -> Path:
        return Path(self.cache_path)

    @property
    def projects_dir(self) -> Path:
        return self.resolve_app_path(self.get("project_output_path", "projects"))

    def project_dir(self, project_name: str) -> Path:
        return self.projects_dir / project_name

    def project_file(self, project_name: str, *parts: str) -> Path:
        return self.project_dir(project_name).joinpath(*parts)

    @property
    def ffprobe_path(self) -> str:
        return self.get("ffprobe_path")

    @property
    def capcut_tts_path(self) -> str:
        return str(self.resolve_app_path(self.get("capcut_tts_path")))

    @property
    def device_json_path(self) -> str:
        return str(self.resolve_app_path(self.get("device_json_path")))

    @property
    def default_voice(self) -> str:
        return self.get("default_voice")

    @property
    def default_resource_id(self) -> str:
        return self.get("default_resource_id")

    @property
    def default_rate(self) -> float:
        return float(self.get("default_rate") or 1.0)

    @property
    def voice_catalog_url(self) -> str:
        return str(self.get("voice_catalog_url") or DEFAULT_VOICE_CATALOG_URL).strip()
