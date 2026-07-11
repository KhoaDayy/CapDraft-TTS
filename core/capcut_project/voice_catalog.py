"""Voice catalog loaded from a remote JSON URL — no local Voice.json."""

from __future__ import annotations

import json

import requests

from core.logger import logger
from .models import VoiceOption

DEFAULT_VOICE_CATALOG_URL = (
    "https://raw.githubusercontent.com/KhoaDayy/CapDraft-TTS/refs/heads/main/Voice.json"
)


class VoiceCatalogError(RuntimeError):
    """Raised when the remote voice catalog is unavailable or invalid."""


class VoiceCatalog:
    def __init__(self, url: str | None = None):
        self.url = (url or DEFAULT_VOICE_CATALOG_URL).strip()
        self._voices: list[VoiceOption] = []

    def load(self, url: str | None = None, *, timeout: int = 20) -> list[VoiceOption]:
        if url is not None:
            self.url = (url or "").strip()
        if not self.url:
            raise VoiceCatalogError("Voice catalog URL is empty")

        try:
            response = requests.get(
                self.url,
                timeout=timeout,
                headers={"Accept": "application/json,text/plain,*/*"},
            )
            response.raise_for_status()
        except requests.RequestException as exc:
            raise VoiceCatalogError(f"Cannot download voice catalog: {exc}") from exc

        try:
            raw = json.loads(response.text)
        except json.JSONDecodeError as exc:
            raise VoiceCatalogError(f"Voice catalog is not valid JSON: {exc}") from exc

        return self.load_data(raw, source=self.url)

    def load_data(self, raw, *, source: str = "memory") -> list[VoiceOption]:
        if not isinstance(raw, list):
            raise VoiceCatalogError(f"Voice catalog must be a JSON array: {source}")

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
        if not voices:
            raise VoiceCatalogError(f"Voice catalog has no usable voices: {source}")
        self._voices = voices
        logger.info("Loaded %s voices from %s", len(voices), source)
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
