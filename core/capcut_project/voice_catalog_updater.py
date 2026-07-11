"""Compatibility shim — catalog is loaded live from URL, not written to disk."""

from __future__ import annotations

from dataclasses import dataclass

from .voice_catalog import VoiceCatalog, VoiceCatalogError

# Keep old name so existing imports/UI don't break.
VoiceCatalogUpdateError = VoiceCatalogError


@dataclass(frozen=True)
class VoiceCatalogUpdateResult:
    url: str
    voice_count: int


def update_voice_catalog_from_url(
    *,
    url: str,
    destination=None,  # ignored — no local file
    timeout: int = 20,
    catalog: VoiceCatalog | None = None,
) -> VoiceCatalogUpdateResult:
    """Fetch and validate a remote catalog into memory."""
    cat = catalog or VoiceCatalog(url)
    voices = cat.load(url, timeout=timeout)
    return VoiceCatalogUpdateResult(url=cat.url, voice_count=len(voices))
