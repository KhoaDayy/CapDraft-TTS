"""Unit / integration tests for CapCut Project TTS (no real TTS API)."""

from __future__ import annotations

import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from core.capcut_project.models import (
    CaptionRow,
    CaptionTtsResult,
    ToneModifyMode,
    compute_target_duration_us,
    map_tone_mode_to_capcut_flag,
    seconds_to_us,
)
from core.capcut_project.voice_catalog import VoiceCatalog
from core.capcut_project.draft_reader import DraftReader
from core.capcut_project.draft_patcher import DraftPatcher
from core.capcut_project.native_audio_alignment import (
    NativeAudioAlignmentSettings,
    apply_native_audio_alignment,
    frames_to_us,
)
from core.capcut_project.paths import capcut_text_reading_path, ensure_draftpath_placeholder
from core.capcut_project.validator import DraftValidator
from core.capcut_project.tts_project_service import CapCutProjectTtsService


# Prefer a clean CapCut draft (no TTS). Workspace / CapCut project drafts may already be patched.
def _pick_clean_draft() -> Path:
    candidates = [
        ROOT / "tests" / "fixtures" / "draft_content.json",
        ROOT / "draft_content.json",
    ]
    for p in candidates:
        if not p.exists():
            continue
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
            tts = sum(
                1
                for a in (data.get("materials") or {}).get("audios") or []
                if isinstance(a, dict) and a.get("type") == "text_to_audio"
            )
            if tts == 0:
                return p
        except Exception:
            continue
    # last resort: first existing candidate
    for p in candidates:
        if p.exists():
            return p
    return ROOT / "draft_content.json"


SAMPLE_DRAFT = _pick_clean_draft()
VOICE_JSON = ROOT / "Voice.json"
SAMPLE_VOICE_TYPE = "BV074_streaming"
SAMPLE_RESOURCE_ID = "7102355709945188865"
SAMPLE_VOICE_DISPLAY_NAME = "Cô Gái Hoạt Ngôn"


def _tts_count(draft: dict) -> int:
    return sum(
        1
        for a in (draft.get("materials") or {}).get("audios") or []
        if isinstance(a, dict) and a.get("type") == "text_to_audio"
    )


class TestVoiceCatalog(unittest.TestCase):
    def test_load_preserves_fields_and_duplicates(self):
        cat = VoiceCatalog(VOICE_JSON)
        voices = cat.load()
        self.assertGreater(len(voices), 10)
        # known entry
        match = [v for v in voices if v.voice_type == "BV074_streaming"]
        self.assertTrue(match)
        self.assertEqual(match[0].display_name, "Cô Gái Hoạt Ngôn")
        self.assertEqual(match[0].resource_id, "7102355709945188865")
        # duplicate resource_ids must not be dropped
        from collections import Counter

        c = Counter(v.resource_id for v in voices)
        # just ensure count equals raw file length for valid entries
        raw = json.loads(VOICE_JSON.read_text(encoding="utf-8"))
        valid = [x for x in raw if x.get("voice_type") and x.get("resource_id")]
        self.assertEqual(len(voices), len(valid))

    def test_filter_language(self):
        cat = VoiceCatalog(VOICE_JSON)
        cat.load()
        vi = cat.filter(language_code="vi")
        self.assertTrue(vi)
        self.assertTrue(all(v.language_code == "vi" or v.locale.startswith("vi") for v in vi))


class TestToneAndSpeed(unittest.TestCase):
    def test_tone_adapter_both_values(self):
        self.assertIs(map_tone_mode_to_capcut_flag(ToneModifyMode.FOLLOW_SPEED), True)
        self.assertIs(map_tone_mode_to_capcut_flag(ToneModifyMode.PRESERVE_PITCH), False)
        self.assertNotEqual(
            map_tone_mode_to_capcut_flag(ToneModifyMode.FOLLOW_SPEED),
            map_tone_mode_to_capcut_flag(ToneModifyMode.PRESERVE_PITCH),
        )
        # QComboBox / IPC may pass str
        self.assertEqual(
            map_tone_mode_to_capcut_flag("preserve_pitch"),
            map_tone_mode_to_capcut_flag(ToneModifyMode.PRESERVE_PITCH),
        )
        self.assertEqual(
            map_tone_mode_to_capcut_flag("follow_speed"),
            map_tone_mode_to_capcut_flag(ToneModifyMode.FOLLOW_SPEED),
        )

    def test_target_duration_frame_snap(self):
        # sample: src=2233333 speed=1.3 -> frame-snapped 1733333 @30fps
        tgt = compute_target_duration_us(2233333, 1.3, fps=30.0)
        self.assertEqual(tgt, 1733333)
        self.assertEqual(compute_target_duration_us(1_000_000, 1.0, fps=30.0), 1_000_000)

    def test_tts_rate_not_in_clip_formula(self):
        # clip speed only affects target; tts_rate is API-only
        a = compute_target_duration_us(2_000_000, 1.0)
        b = compute_target_duration_us(2_000_000, 2.0)
        self.assertGreater(a, b)


class TestDraftReader(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not SAMPLE_DRAFT.exists():
            raise unittest.SkipTest("sample draft missing")

    def test_project_info_and_captions(self):
        r = DraftReader()
        r.load_draft(SAMPLE_DRAFT)
        info = r.get_project_info()
        self.assertEqual(info.width, 1920)
        self.assertEqual(info.height, 1080)
        self.assertEqual(info.fps, 30.0)
        self.assertGreater(info.caption_count, 100)
        caps = r.get_captions()
        self.assertEqual(len(caps), info.caption_count)
        self.assertTrue(all(isinstance(c, CaptionRow) for c in caps))
        # sorted by timeline
        starts = [c.start_us for c in caps]
        self.assertEqual(starts, sorted(starts))

    def test_inspection(self):
        r = DraftReader()
        r.load_draft(SAMPLE_DRAFT)
        insp = r.inspect_project()
        self.assertGreater(insp.valid_caption_count, 0)
        self.assertIsInstance(insp.warnings, list)


class TestPatcherIntegration(unittest.TestCase):
    def test_patch_references_and_speed(self):
        if not SAMPLE_DRAFT.exists():
            self.skipTest("no sample")
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            # minimal project: copy draft + fake audio
            draft_path = td / "draft_content.json"
            shutil.copy2(SAMPLE_DRAFT, draft_path)
            audio_dir = td / "textReading"
            audio_dir.mkdir()
            audio_file = audio_dir / "tts_000001_test.mp3"
            audio_file.write_bytes(b"ID3fakeaudio")

            reader = DraftReader()
            reader.load_draft(draft_path)
            caps = reader.get_captions()[:5]
            # pick non-empty
            caps = [c for c in caps if not c.is_empty][:3]
            self.assertTrue(caps)

            results = []
            rels = {}
            for c in caps:
                src_us = 1_200_000
                results.append(
                    CaptionTtsResult(
                        caption_index=c.index,
                        text_material_id=c.text_material_id,
                        text_segment_id=c.text_segment_id,
                        start_us=c.start_us,
                        audio_path=str(audio_file),
                        source_duration_us=src_us,
                        target_duration_us=compute_target_duration_us(src_us, 1.3, fps=30),
                        status="generated",
                    )
                )
                rels[c.index] = "textReading/tts_000001_test.mp3"

            draft = reader.get_draft_copy()
            existing_bad_tts = next(
                (
                    a
                    for a in draft["materials"]["audios"]
                    if isinstance(a, dict) and a.get("type") == "text_to_audio"
                ),
                None,
            )
            if existing_bad_tts is not None:
                existing_bad_tts["tone_speaker"] = "pNInz6obpgDQGcFmaJgB"
                existing_bad_tts["resource_id"] = ""
                existing_bad_tts["tone_effect_name"] = "Pro Voice"

            patcher = DraftPatcher(reader.get_tts_templates(), fps=30.0)
            patched, stats = patcher.patch(
                draft,
                captions=reader.get_captions(),
                results=results,
                voice_type=SAMPLE_VOICE_TYPE,
                resource_id=SAMPLE_RESOURCE_ID,
                voice_display_name=SAMPLE_VOICE_DISPLAY_NAME,
                capcut_clip_speed=1.3,
                tone_modify_mode=ToneModifyMode.PRESERVE_PITCH,
                existing_tts_mode="replace_existing",
                audio_rel_paths=rels,
                project_directory=td,
            )
            self.assertEqual(stats.materials_created, len(caps))
            self.assertEqual(stats.segments_created, len(caps))
            self.assertGreaterEqual(stats.tracks_used, 1)

            # validate CapCut sees every TTS as the known export-safe free voice
            materials = patched["materials"]
            tts = [
                a
                for a in materials["audios"]
                if "tts_000001_test.mp3" in str(a.get("path") or "")
            ]
            self.assertEqual(len(tts), len(caps))
            text_by_id = {t["id"]: t for t in materials["texts"]}
            for a in tts:
                self.assertEqual(a.get("type"), "text_to_audio")
                self.assertTrue(a.get("text_id"))
            for c in caps:
                self.assertTrue(text_by_id[c.text_material_id].get("text_to_audio_ids"))
            all_tts = [
                a
                for a in materials["audios"]
                if isinstance(a, dict) and a.get("type") == "text_to_audio"
            ]
            self.assertTrue(all_tts)
            for a in all_tts:
                self.assertEqual(a.get("resource_id"), SAMPLE_RESOURCE_ID)
                self.assertEqual(a.get("tone_speaker"), SAMPLE_VOICE_TYPE)
                self.assertEqual(a.get("mock_tone_speaker"), SAMPLE_VOICE_TYPE)
                self.assertEqual(a.get("tone_type"), SAMPLE_VOICE_DISPLAY_NAME)
                self.assertEqual(a.get("tone_effect_name"), SAMPLE_VOICE_DISPLAY_NAME)
                self.assertEqual(a.get("tone_platform"), "sami")
                self.assertEqual(a.get("copyright_limit_type"), "none")
                self.assertEqual((a.get("tts_benefit_info") or {}).get("benefit_type"), "none")

            # segment speed + tone + native trim (3 frames @30fps = 100_000)
            mat_ids = {a["id"] for a in tts}
            audio_tracks = [t for t in patched["tracks"] if t.get("type") == "audio"]
            segs = [s for t in audio_tracks for s in t.get("segments") or []]
            tts_segs = [s for s in segs if s.get("material_id") in mat_ids]
            self.assertEqual(len(tts_segs), len(caps))
            trim_us = 100_000
            src_after = 1_200_000 - trim_us
            for s in tts_segs:
                self.assertEqual(s["is_tone_modify"], map_tone_mode_to_capcut_flag(ToneModifyMode.PRESERVE_PITCH))
                self.assertEqual(s["source_timerange"]["start"], trim_us)
                self.assertEqual(s["source_timerange"]["duration"], src_after)
                self.assertEqual(
                    s["target_timerange"]["duration"],
                    compute_target_duration_us(src_after, 1.3, fps=30),
                )
                # speed material
                speed_ids = set(s["extra_material_refs"])
                speeds = [x for x in materials["speeds"] if x["id"] in speed_ids]
                self.assertEqual(len(speeds), 1)
                self.assertAlmostEqual(float(speeds[0]["speed"]), 1.3)
                # fade material
                fades = [
                    x
                    for x in (materials.get("audio_fades") or [])
                    if x["id"] in s["extra_material_refs"]
                ]
                self.assertTrue(fades, "expected audio_fade in extra_material_refs")

            # material keeps RAW duration
            for a in tts:
                self.assertEqual(a["duration"], 1_200_000)
                self.assertEqual(a.get("local_material_id"), a["id"])
                self.assertEqual(a.get("music_id"), a["id"])
                self.assertEqual(a.get("resource_id"), SAMPLE_RESOURCE_ID)
                self.assertEqual(a.get("tone_speaker"), SAMPLE_VOICE_TYPE)
                self.assertEqual(a.get("mock_tone_speaker"), SAMPLE_VOICE_TYPE)
                self.assertEqual(a.get("tone_type"), SAMPLE_VOICE_DISPLAY_NAME)
                self.assertEqual(a.get("tone_effect_name"), SAMPLE_VOICE_DISPLAY_NAME)
                self.assertEqual(a.get("copyright_limit_type"), "none")
                self.assertEqual((a.get("tts_benefit_info") or {}).get("benefit_type"), "none")

            # no audio_pitch_shifts required
            self.assertEqual(len(materials.get("audio_pitch_shifts") or []), 0)

            errors = DraftValidator().validate(
                patched, project_directory=td, require_audio_files=False
            )
            self.assertEqual(errors, [], msg=errors[:5])

    def test_both_tone_modes(self):
        if not SAMPLE_DRAFT.exists():
            self.skipTest("no sample")
        reader = DraftReader()
        reader.load_draft(SAMPLE_DRAFT)
        caps = [c for c in reader.get_captions() if not c.is_empty][:1]
        for mode in ToneModifyMode:
            draft = reader.get_draft_copy()
            with tempfile.TemporaryDirectory() as td:
                td = Path(td)
                (td / "textReading").mkdir()
                audio = td / "textReading" / "a.mp3"
                audio.write_bytes(b"x")
                results = [
                    CaptionTtsResult(
                        caption_index=caps[0].index,
                        text_material_id=caps[0].text_material_id,
                        text_segment_id=caps[0].text_segment_id,
                        start_us=caps[0].start_us,
                        audio_path=str(audio),
                        source_duration_us=500_000,
                        target_duration_us=500_000,
                        status="generated",
                    )
                ]
                patched, _ = DraftPatcher(reader.get_tts_templates(), fps=30).patch(
                    draft,
                    captions=reader.get_captions(),
                    results=results,
                    voice_type=SAMPLE_VOICE_TYPE,
                    resource_id="1",
                    voice_display_name="x",
                    capcut_clip_speed=1.0,
                    tone_modify_mode=mode,
                    existing_tts_mode="replace_existing",
                    audio_rel_paths={caps[0].index: "textReading/a.mp3"},
                    project_directory=td,
                )
                segs = [
                    s
                    for t in patched["tracks"]
                    if t.get("type") == "audio"
                    for s in t.get("segments") or []
                ]
                # find our segment via material path
                tts_mats = {
                    a["id"]
                    for a in patched["materials"]["audios"]
                    if a.get("type") == "text_to_audio" and "textReading/a.mp3" in str(a.get("path") or "")
                }
                ours = [s for s in segs if s.get("material_id") in tts_mats]
                self.assertTrue(ours)
                self.assertEqual(ours[0]["is_tone_modify"], map_tone_mode_to_capcut_flag(mode))


class TestLoggingCallback(unittest.TestCase):
    def test_log_callback_exception_does_not_break(self):
        svc = CapCutProjectTtsService()

        def bad_cb(ev):
            raise RuntimeError("ui boom")

        svc.set_log_callback(bad_cb)
        # should not raise
        svc.emit_log("INFO", "hello", stage="test")

    def test_load_project_logs_summary_not_full_captions(self):
        if not SAMPLE_DRAFT.exists():
            self.skipTest("no sample")
        events = []
        svc = CapCutProjectTtsService(log_callback=lambda e: events.append(e))
        info = svc.load_project(SAMPLE_DRAFT)
        self.assertTrue(events)
        joined = " ".join(e.message for e in events)
        self.assertIn(str(info.caption_count), joined)
        # should not dump thousands of caption texts
        self.assertLess(len(joined), 5000)


class TestNativeAlignment(unittest.TestCase):
    def test_trim_by_fps(self):
        self.assertEqual(frames_to_us(3, 30), 100_000)
        self.assertEqual(frames_to_us(3, 60), 50_000)
        self.assertEqual(frames_to_us(3, 25), 120_000)

    def test_apply_alignment_keeps_material_raw_and_caption_start(self):
        mat = {"duration": 0, "path": "x.mp3"}
        seg = {"extra_material_refs": ["SPEED1"], "speed": 1.0}
        fades: list = []
        caption_start = 266_666
        raw = 2_400_000
        res = apply_native_audio_alignment(
            audio_segment=seg,
            audio_material=mat,
            audio_fade_materials=fades,
            caption_start_us=caption_start,
            raw_duration_us=raw,
            project_fps=30.0,
            clip_speed=1.2,
            settings=NativeAudioAlignmentSettings(enabled=True, leading_trim_frames=3, fade_in_ms=8),
        )
        self.assertEqual(mat["duration"], raw)
        self.assertEqual(seg["source_timerange"]["start"], 100_000)
        self.assertEqual(seg["source_timerange"]["duration"], raw - 100_000)
        self.assertEqual(seg["target_timerange"]["start"], caption_start)
        self.assertEqual(
            seg["target_timerange"]["duration"],
            compute_target_duration_us(raw - 100_000, 1.2, fps=30),
        )
        self.assertEqual(res.fade_in_us, 8_000)
        self.assertEqual(len(fades), 1)
        self.assertEqual(fades[0]["type"], "audio_fade")
        self.assertIn(fades[0]["id"], seg["extra_material_refs"])
        self.assertEqual(mat["path"], "x.mp3")

    def test_short_audio_clamp(self):
        mat = {"duration": 0}
        seg = {"extra_material_refs": []}
        fades: list = []
        res = apply_native_audio_alignment(
            audio_segment=seg,
            audio_material=mat,
            audio_fade_materials=fades,
            caption_start_us=0,
            raw_duration_us=80_000,
            project_fps=30.0,
            clip_speed=1.0,
            settings=NativeAudioAlignmentSettings(enabled=True, leading_trim_frames=3, fade_in_ms=8),
        )
        self.assertTrue(res.trim_reduced)
        self.assertLess(res.applied_trim_us, 100_000)
        self.assertGreaterEqual(res.source_duration_us, 50_000)
        self.assertEqual(mat["duration"], 80_000)

    def test_placeholder_path_format(self):
        draft = {"materials": {"videos": [{"path": "##_draftpath_placeholder_ABC_##/x.mp4"}]}}
        ph = ensure_draftpath_placeholder(draft)
        self.assertEqual(ph, "##_draftpath_placeholder_ABC_##")
        p = capcut_text_reading_path(ph, "tts_1.mp3")
        self.assertEqual(p, "##_draftpath_placeholder_ABC_##/textReading/tts_1.mp3")


class TestServiceMockGenerate(unittest.TestCase):
    def test_generate_and_attach_mocked(self):
        if not SAMPLE_DRAFT.exists():
            self.skipTest("no sample")
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            # isolate project copy
            draft_path = td / "draft_content.json"
            shutil.copy2(SAMPLE_DRAFT, draft_path)

            svc = CapCutProjectTtsService()
            svc.load_project(draft_path)
            caps = [c for c in svc.get_captions() if not c.is_empty][:2]
            ids = [c.text_segment_id for c in caps]

            fake_audio = td / "fake.mp3"
            fake_audio.write_bytes(b"ID3" + b"\x00" * 100)

            def fake_batch(self, items, voice=None, resource_id=None, rate=None, **kwargs):
                out = {}
                cb = kwargs.get("item_completed_callback")
                for it in items:
                    res = {
                        "audio_path": str(fake_audio),
                        "duration": 1.2,
                        "status": "Downloaded",
                    }
                    out[int(it["index"])] = res
                    if cb:
                        cb(int(it["index"]), res)
                return out

            with patch("core.capcut_project.tts_project_service.CapCutTtsWrapper.generate_tts_batch", fake_batch):
                with patch(
                    "core.capcut_project.tts_project_service.is_capcut_running",
                    return_value=False,
                ):
                    result = svc.generate_and_attach(
                        selected_caption_ids=ids,
                        voice_type=SAMPLE_VOICE_TYPE,
                        resource_id=SAMPLE_RESOURCE_ID,
                        voice_display_name=SAMPLE_VOICE_DISPLAY_NAME,
                        tts_rate=1.0,
                        capcut_clip_speed=1.15,
                        tone_modify_mode=ToneModifyMode.PRESERVE_PITCH,
                        existing_tts_mode="replace_existing",
                        use_cache=False,
                    )
            self.assertTrue(result.success, result.errors)
            self.assertEqual(result.attached, 2)
            self.assertTrue(result.validation_passed)
            self.assertTrue((td / "textReading").exists())
            # reload
            data = json.loads(draft_path.read_text(encoding="utf-8"))
            temp_root = str(td).replace("\\", "/")
            tts = [
                a
                for a in data["materials"]["audios"]
                if a.get("type") == "text_to_audio"
                and temp_root in str(a.get("path") or "").replace("\\", "/")
            ]
            self.assertEqual(len(tts), 2)
            for a in tts:
                self.assertTrue(a.get("text_id"))
                self.assertEqual(a.get("resource_id"), SAMPLE_RESOURCE_ID)
                self.assertEqual(a.get("tone_speaker"), SAMPLE_VOICE_TYPE)
                self.assertEqual(a.get("mock_tone_speaker"), SAMPLE_VOICE_TYPE)
                self.assertEqual(a.get("tone_type"), SAMPLE_VOICE_DISPLAY_NAME)
                self.assertEqual(a.get("tone_effect_name"), SAMPLE_VOICE_DISPLAY_NAME)
                self.assertEqual((a.get("tts_benefit_info") or {}).get("benefit_type"), "none")
                path = str(a.get("path") or "")
                # absolute path is preferred; relative/placeholder still accepted
                self.assertTrue(
                    "textReading" in path.replace("\\", "/"),
                    path,
                )
                if path.startswith("C:") or path.startswith("/"):
                    self.assertTrue(Path(path).exists() or (td / "textReading").exists())
                # native trim default 3 frames @30fps
                # find segment
            segs = [
                s
                for t in data["tracks"]
                if t.get("type") == "audio"
                for s in t.get("segments") or []
                if s.get("material_id") in {a["id"] for a in tts}
            ]
            self.assertEqual(len(segs), 2)
            for s in segs:
                self.assertEqual(s["source_timerange"]["start"], 100_000)


if __name__ == "__main__":
    unittest.main()
