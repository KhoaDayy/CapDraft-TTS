"""CapCut project read/patch/export for TTS attachment."""

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
from .draft_reader import DraftReader
from .tts_project_service import CapCutProjectTtsService

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
