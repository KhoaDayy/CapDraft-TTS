"""Voice catalog loaded from Voice.json — no hardcoded voice list."""

from __future__ import annotations

import json
from pathlib import Path

from core.logger import logger
from .models import VoiceOption


class VoiceCatalog:
    def __init__(self, path: Path | str | None = None):
        self.path = Path(path) if path else None
        self._voices: list[VoiceOption] = []

    def load(self, path: Path | str | None = None) -> list[VoiceOption]:
        if path is not None:
            self.path = Path(path)
        if self.path is None:
            raise ValueError("Voice catalog path is not set")
        if not self.path.exists():
            raise FileNotFoundError(f"Voice catalog not found: {self.path}")

        with open(self.path, "r", encoding="utf-8-sig") as f:
            raw = json.load(f)
        if not isinstance(raw, list):
            raise ValueError(f"Voice catalog must be a JSON array: {self.path}")

        voices: list[VoiceOption] = []
        for i, item in enumerate(raw):
            if not isinstance(item, dict):
                logger.warning("Skipping non-object voice entry at index %s", i)
                continue
            voice_type = str(item.get("voice_type") or "").strip()
            resource_id = str(item.get("resource_id") or "").strip()
            display_name = str(item.get("display_name") or voice_type or f"Voice {i + 1}").strip()
            if not voice_type or not resource_id:
                logger.warning("Skipping incomplete voice entry at index %s: %r", i, item)
                continue
            voices.append(
                VoiceOption(
                    language_code=str(item.get("lan") or "").strip(),
                    locale=str(item.get("lang") or "").strip(),
                    voice_type=voice_type,
                    display_name=display_name,
                    resource_id=resource_id,
                )
            )

        # Keep duplicate resource_id entries — CapCut may expose multiple labels.
        self._voices = voices
        logger.info("Loaded %s voices from %s", len(voices), self.path)
        return list(self._voices)

    @property
    def voices(self) -> list[VoiceOption]:
        return list(self._voices)

    def languages(self) -> list[tuple[str, str]]:
        """Unique (language_code, locale) pairs preserving order."""
        seen: set[str] = set()
        out: list[tuple[str, str]] = []
        for v in self._voices:
            key = v.language_code or v.locale
            if key in seen:
                continue
            seen.add(key)
            out.append((v.language_code, v.locale))
        return out

    def filter(
        self,
        *,
        language_code: str | None = None,
        locale: str | None = None,
        query: str | None = None,
    ) -> list[VoiceOption]:
        q = (query or "").strip().lower()
        results: list[VoiceOption] = []
        for v in self._voices:
            if language_code and v.language_code != language_code and v.locale != language_code:
                # also allow filtering by locale prefix
                if not (v.locale.startswith(language_code) if language_code else True):
                    continue
            if locale and v.locale != locale and v.language_code != locale:
                continue
            if q:
                hay = f"{v.display_name} {v.voice_type} {v.resource_id} {v.locale}".lower()
                if q not in hay:
                    continue
            results.append(v)
        return results

    def find(self, voice_type: str, resource_id: str | None = None) -> VoiceOption | None:
        for v in self._voices:
            if v.voice_type == voice_type and (resource_id is None or v.resource_id == resource_id):
                return v
        return None
