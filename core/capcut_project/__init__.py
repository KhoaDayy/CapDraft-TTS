"""CapCut project read/patch/export for TTS attachment.

Keep this package init light: importing a leaf module (e.g. voice_catalog
from AppConfig) must not pull CapCutTtsWrapper and create an import cycle.
"""

from .models import (
    CaptionRow,
    CapCutProjectInfo,
    GenerationLogEvent,
    GenerationResult,
    ProjectInspectionResult,
    ToneModifyMode,
    TtsGenerationSettings,
    VoiceOption,
    map_tone_mode_to_capcut_flag,
)
from .voice_catalog import DEFAULT_VOICE_CATALOG_URL, VoiceCatalog, VoiceCatalogError
from .voice_catalog_updater import VoiceCatalogUpdateError, VoiceCatalogUpdateResult

__all__ = [
    "CaptionRow",
    "CapCutProjectInfo",
    "GenerationLogEvent",
    "GenerationResult",
    "ProjectInspectionResult",
    "ToneModifyMode",
    "TtsGenerationSettings",
    "VoiceOption",
    "map_tone_mode_to_capcut_flag",
    "DEFAULT_VOICE_CATALOG_URL",
    "VoiceCatalog",
    "VoiceCatalogError",
    "VoiceCatalogUpdateError",
    "VoiceCatalogUpdateResult",
    "DraftReader",
    "CapCutProjectTtsService",
]


def __getattr__(name: str):
    # Lazy heavy imports — avoid config ↔ tts_project_service cycle.
    if name == "DraftReader":
        from .draft_reader import DraftReader

        return DraftReader
    if name == "CapCutProjectTtsService":
        from .tts_project_service import CapCutProjectTtsService

        return CapCutProjectTtsService
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
