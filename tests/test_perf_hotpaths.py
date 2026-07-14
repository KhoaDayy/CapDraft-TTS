"""Runnable checks for 5k-caption hot paths (no Qt display needed for model)."""

from __future__ import annotations

import sys
import time
import unittest
from pathlib import Path
from unittest.mock import MagicMock

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from core.capcut_project.models import CaptionRow, CaptionTtsResult
from core.capcut_project.draft_patcher import DraftPatcher
from core.capcut_project.project_exporter import ProjectExporter


def _fake_caption(i: int) -> CaptionRow:
    return CaptionRow(
        index=i,
        text_track_id="t1",
        text_segment_id=f"seg-{i}",
        text_material_id=f"mat-{i}",
        start_us=i * 1_000_000,
        duration_us=800_000,
        text=f"Caption line {i}",
        existing_tts_segment_ids=[],
    )


class TestCaptionModelPerf(unittest.TestCase):
    def test_model_loads_5k_quickly(self):
        # Import Qt only when available (headless CI may still have PySide6)
        from PySide6.QtWidgets import QApplication
        from ui.caption_table_model import CaptionFilterProxy, CaptionTableModel

        app = QApplication.instance() or QApplication([])
        caps = [_fake_caption(i) for i in range(1, 5001)]
        model = CaptionTableModel()
        t0 = time.perf_counter()
        model.set_captions(caps)
        elapsed = time.perf_counter() - t0
        self.assertEqual(model.rowCount(), 5000)
        self.assertLess(elapsed, 2.0, f"set_captions 5k took {elapsed:.3f}s")

        proxy = CaptionFilterProxy()
        proxy.setSourceModel(model)
        t1 = time.perf_counter()
        proxy.set_filters(query="line 42", hide_empty=True)
        f_elapsed = time.perf_counter() - t1
        self.assertGreater(proxy.rowCount(), 0)
        self.assertLess(f_elapsed, 0.5, f"filter 5k took {f_elapsed:.3f}s")

        # Batch status updates
        updates = {i: {"status": "Cached", "duration": 1.2} for i in range(1, 501)}
        t2 = time.perf_counter()
        model.apply_item_results(updates)
        u_elapsed = time.perf_counter() - t2
        self.assertLess(u_elapsed, 0.5, f"apply 500 updates took {u_elapsed:.3f}s")
        _ = app  # keep ref


class TestPatcherClone(unittest.TestCase):
    def test_clone_and_normalize_skip_clean(self):
        templates = {
            "audio_material": {
                "id": "",
                "type": "text_to_audio",
                "path": "",
                "name": "",
                "duration": 0,
                "wave_points": [0.1] * 20,
                "nested": {"a": 1, "b": [1, 2, 3]},
            },
            "audio_segment": {
                "id": "",
                "material_id": "",
                "source_timerange": {"start": 0, "duration": 0},
                "target_timerange": {"start": 0, "duration": 0},
                "extra_material_refs": [],
                "speed": 1.0,
                "volume": 1.0,
            },
            "audio_track": {
                "id": "",
                "type": "audio",
                "segments": [],
                "is_default_name": True,
                "name": "",
            },
            "extras": {
                "speeds": {"id": "", "type": "speed", "mode": 0, "speed": 1.0, "curve_speed": None},
                "placeholder_infos": {"id": "", "type": "placeholder_info"},
                "beats": {"id": "", "type": "beats"},
            },
        }
        patcher = DraftPatcher(templates, fps=30.0)
        a = patcher._clone(templates["audio_material"])
        b = patcher._clone(templates["audio_material"])
        self.assertIsNot(a, b)
        self.assertEqual(a["nested"]["b"], [1, 2, 3])
        a["nested"]["b"].append(9)
        self.assertEqual(b["nested"]["b"], [1, 2, 3])

        # already-safe material skipped
        from core.capcut_project.draft_patcher import SAFE_TTS_RESOURCE_ID, SAFE_TTS_VOICE_TYPE
        from core.capcut_project.models import CaptionRow, CaptionTtsResult

        mats = {
            "audios": [
                {
                    "type": "text_to_audio",
                    "resource_id": SAFE_TTS_RESOURCE_ID,
                    "tone_speaker": SAFE_TTS_VOICE_TYPE,
                    "mock_tone_speaker": SAFE_TTS_VOICE_TYPE,
                    "copyright_limit_type": "none",
                },
                {
                    "type": "text_to_audio",
                    "resource_id": "other",
                    "tone_speaker": "x",
                    "mock_tone_speaker": "x",
                    "copyright_limit_type": "paid",
                },
            ]
        }
        n = patcher._normalize_tts_voice_materials(mats)
        self.assertEqual(n, 1)
        self.assertEqual(mats["audios"][1]["resource_id"], SAFE_TTS_RESOURCE_ID)

        # Slim extras: only speeds (+ optional fades later), never placeholder/beats per caption
        cap = CaptionRow(
            index=1,
            text_track_id="t",
            text_segment_id="s",
            text_material_id="m",
            start_us=0,
            duration_us=1_000_000,
            text="hi",
        )
        res = CaptionTtsResult(
            caption_index=1,
            text_material_id="m",
            text_segment_id="s",
            start_us=0,
            audio_path="x.mp3",
            source_duration_us=1_200_000,
            target_duration_us=1_200_000,
            status="generated",
        )
        mat, seg, extras, _ = patcher._build_tts_objects(
            caption=cap,
            result=res,
            rel_path="textReading/x.mp3",
            voice_type="v",
            resource_id="r",
            voice_display_name="n",
            clip_speed=1.0,
            is_tone_modify=False,
        )
        buckets = {b for b, _o in extras}
        self.assertIn("speeds", buckets)
        self.assertNotIn("placeholder_infos", buckets)
        self.assertNotIn("beats", buckets)
        self.assertEqual(mat.get("wave_points"), [])


class TestExporterBatchCopy(unittest.TestCase):
    def test_batch_copy_parallel(self):
        import tempfile

        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            src_dir = root / "src"
            proj = root / "proj"
            src_dir.mkdir()
            proj.mkdir()
            items = []
            for i in range(20):
                p = src_dir / f"a{i}.mp3"
                p.write_bytes(b"ID3fake" + bytes([i % 256]) * 64)
                items.append((p, f"tts_{i:06d}.mp3"))
            exp = ProjectExporter()
            out = exp.copy_audio_batch(items, proj, workers=4)
            self.assertEqual(len(out), 20)
            for rel, err in out:
                self.assertIsNone(err)
                self.assertTrue(rel)
            self.assertEqual(len(list((proj / "textReading").glob("*.mp3"))), 20)


if __name__ == "__main__":
    unittest.main()
