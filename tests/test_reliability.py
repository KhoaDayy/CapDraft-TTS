"""TDD tests for cancel/timeout, multi-target commit, temp cleanup, closeEvent, partial failure."""

from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from core.cache import TtsCache
from core.capcut_tts import CapCutTtsWrapper, CancelledError, run_cancellable_subprocess
from core.capcut_project.models import GenerationResult, ToneModifyMode
from core.capcut_project.project_exporter import ProjectExporter
from core.capcut_project.tts_project_service import CapCutProjectTtsService


class TestCancellableSubprocess(unittest.TestCase):
    def test_timeout_terminates_process(self):
        # long-running python sleep
        with self.assertRaises(TimeoutError):
            run_cancellable_subprocess(
                [sys.executable, "-c", "import time; time.sleep(30)"],
                timeout_sec=0.3,
            )

    def test_cancel_callback_terminates_process(self):
        cancel_at = time.perf_counter() + 0.2
        with self.assertRaises(CancelledError):
            run_cancellable_subprocess(
                [sys.executable, "-c", "import time; time.sleep(30)"],
                timeout_sec=10,
                is_cancelled_callback=lambda: time.perf_counter() >= cancel_at,
            )

    def test_success_returns_completed(self):
        res = run_cancellable_subprocess(
            [sys.executable, "-c", "import sys; print('200'); print('ok')"],
            timeout_sec=10,
        )
        self.assertEqual(res.returncode, 0)
        self.assertIn("200", res.stdout)

    def test_large_stdout_does_not_deadlock(self):
        """Regression: poll()+PIPE without drain hangs when stdout fills OS buffer."""
        # ~256KB stdout — larger than typical pipe buffer
        code = (
            "import sys\n"
            "sys.stdout.write('X' * 262144)\n"
            "sys.stdout.write('\\nEND\\n')\n"
            "sys.stdout.flush()\n"
        )
        res = run_cancellable_subprocess(
            [sys.executable, "-c", code],
            timeout_sec=10,
        )
        self.assertEqual(res.returncode, 0)
        self.assertGreaterEqual(len(res.stdout), 262144)
        self.assertIn("END", res.stdout)

    def test_large_stderr_does_not_deadlock(self):
        code = (
            "import sys\n"
            "sys.stderr.write('E' * 262144)\n"
            "sys.stderr.write('\\nERR\\n')\n"
            "sys.stderr.flush()\n"
        )
        res = run_cancellable_subprocess(
            [sys.executable, "-c", code],
            timeout_sec=10,
        )
        self.assertEqual(res.returncode, 0)
        self.assertGreaterEqual(len(res.stderr), 262144)
        self.assertIn("ERR", res.stderr)

    def test_large_stdout_and_stderr_together(self):
        code = (
            "import sys\n"
            "sys.stdout.write('O' * 200000)\n"
            "sys.stdout.flush()\n"
            "sys.stderr.write('E' * 200000)\n"
            "sys.stderr.flush()\n"
            "print('DONE')\n"
        )
        res = run_cancellable_subprocess(
            [sys.executable, "-c", code],
            timeout_sec=15,
        )
        self.assertEqual(res.returncode, 0)
        self.assertGreaterEqual(len(res.stdout), 200000)
        self.assertGreaterEqual(len(res.stderr), 200000)
        self.assertIn("DONE", res.stdout)

    def test_sleep_then_exit_returns_output(self):
        res = run_cancellable_subprocess(
            [sys.executable, "-c", "import time; time.sleep(0.4); print('200'); print('ok')"],
            timeout_sec=10,
        )
        self.assertEqual(res.returncode, 0)
        self.assertIn("200", res.stdout)
        self.assertIn("ok", res.stdout)

    def test_cancel_large_output_child(self):
        # child writes forever; cancel must terminate without hang
        code = (
            "import sys, time\n"
            "while True:\n"
            "    sys.stdout.write('Z' * 8192)\n"
            "    sys.stdout.flush()\n"
            "    time.sleep(0.01)\n"
        )
        cancel_at = time.perf_counter() + 0.3
        with self.assertRaises(CancelledError):
            run_cancellable_subprocess(
                [sys.executable, "-c", code],
                timeout_sec=15,
                is_cancelled_callback=lambda: time.perf_counter() >= cancel_at,
            )


class TestParallelBatchCompletion(unittest.TestCase):
    def test_two_chunks_both_complete_with_cancellable_client(self):
        """Integration-style: both chunks finish; protected by suite timeout."""
        wrapper = CapCutTtsWrapper(project_name="")
        call_order = []
        lock = threading.Lock()

        def fake_chunk(pending, *args, **kwargs):
            with lock:
                call_order.append(tuple(p["index"] for p in pending))
            # simulate short client work
            time.sleep(0.05)
            out = {}
            for p in pending:
                out[int(p["index"])] = {
                    "audio_path": "",
                    "duration": 0.5,
                    "status": "Downloaded",
                }
            return out

        def config_get(key, default=None):
            overrides = {
                "tts_chunk_size": 2,
                "tts_parallel_chunks": 2,
                "tts_poll_interval_sec": 0.1,
                "tts_download_workers": 2,
                "tts_subprocess_timeout_sec": 30,
            }
            if key in overrides:
                return overrides[key]
            return wrapper.config._data.get(key, default)

        items = [{"index": i, "text": f"t{i}"} for i in range(1, 5)]  # 2 chunks of 2
        t0 = time.perf_counter()
        with patch.object(wrapper.config, "get", side_effect=config_get):
            with patch.object(wrapper, "_get_client_path", return_value=str(ROOT / "tests" / "test_reliability.py")):
                with patch.object(wrapper, "_process_single_chunk", side_effect=fake_chunk):
                    with patch.object(wrapper.cache, "get_cached_file", return_value=(None, None)):
                        results = wrapper.generate_tts_batch(
                            items,
                            voice="v",
                            resource_id="r",
                            rate=1.0,
                            use_cache=False,
                        )
        elapsed = time.perf_counter() - t0
        self.assertLess(elapsed, 5.0)
        self.assertEqual(len(results), 4)
        self.assertEqual(len(call_order), 2)


class TestParallelBatchCancel(unittest.TestCase):
    def test_cancel_stops_pending_chunks_without_infinite_wait(self):
        wrapper = CapCutTtsWrapper(project_name="")
        cancel = {"flag": False}
        call_count = {"n": 0}
        lock = threading.Lock()

        def slow_chunk(*args, **kwargs):
            with lock:
                call_count["n"] += 1
                n = call_count["n"]
            if n == 1:
                time.sleep(0.15)
                cancel["flag"] = True
                time.sleep(0.05)
            else:
                time.sleep(2.0)  # would hang if not cancelled
            return {}

        def config_get(key, default=None):
            overrides = {
                "tts_chunk_size": 1,
                "tts_parallel_chunks": 4,
                "tts_poll_interval_sec": 0.1,
                "tts_download_workers": 2,
                "tts_subprocess_timeout_sec": 30,
            }
            if key in overrides:
                return overrides[key]
            return wrapper.config._data.get(key, default)

        items = [{"index": i, "text": f"t{i}"} for i in range(1, 7)]
        t0 = time.perf_counter()
        with patch.object(wrapper.config, "get", side_effect=config_get):
            with patch.object(wrapper, "_get_client_path", return_value=str(ROOT / "tests" / "test_reliability.py")):
                with patch.object(wrapper, "_process_single_chunk", side_effect=slow_chunk):
                    with patch.object(wrapper.cache, "get_cached_file", return_value=(None, None)):
                        results = wrapper.generate_tts_batch(
                            items,
                            voice="v",
                            resource_id="r",
                            rate=1.0,
                            use_cache=False,
                            is_cancelled_callback=lambda: cancel["flag"],
                        )
        elapsed = time.perf_counter() - t0
        self.assertLess(elapsed, 1.5, f"cancel waited too long: {elapsed:.2f}s")
        self.assertIsInstance(results, dict)


class TestMultiTargetCommitRollback(unittest.TestCase):
    def test_second_target_failure_rolls_back_first(self):
        exporter = ProjectExporter(max_backups=5)
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            draft_path = td / "draft_content.json"
            original = {"id": "P1", "materials": {"audios": []}, "tracks": [], "version": 1}
            draft_path.write_text(json.dumps(original), encoding="utf-8")
            # second target under Timelines
            t2 = td / "Timelines" / "P1" / "draft_content.json"
            t2.parent.mkdir(parents=True)
            t2.write_text(json.dumps(original), encoding="utf-8")

            backup = exporter.create_backup(draft_path, td, {"ok": True}, project_id="P1")
            patched = {"id": "P1", "materials": {"audios": [{"id": "a"}]}, "tracks": [], "version": 2}

            real_replace = os.replace
            commit_replaces = {"n": 0}

            def flaky_replace(src, dst):
                # Fail only on the second *commit* replace (*.tmp -> draft_content.json).
                # Allow rollback (*.rollback.tmp) and other files.
                src_s, dst_s = str(src), str(dst)
                if (
                    Path(dst_s).name == "draft_content.json"
                    and src_s.endswith(".tmp")
                    and "rollback" not in src_s
                ):
                    commit_replaces["n"] += 1
                    if commit_replaces["n"] >= 2:
                        raise OSError("simulated disk full on second target")
                return real_replace(src, dst)

            with patch("core.capcut_project.project_exporter.os.replace", side_effect=flaky_replace):
                with self.assertRaises(OSError):
                    exporter.atomic_write_draft(
                        draft_path, patched, project_directory=td, project_id="P1"
                    )

            # After failed multi-commit, either nothing committed or rollback restores originals.
            # The safer API stages then commits; on failure both should match original.
            data_root = json.loads(draft_path.read_text(encoding="utf-8"))
            data_t2 = json.loads(t2.read_text(encoding="utf-8"))
            self.assertEqual(data_root.get("version"), 1, "root should be rolled back / uncommitted")
            self.assertEqual(data_t2.get("version"), 1, "timeline should be rolled back / uncommitted")
            self.assertTrue(backup.is_dir())
            # backup must still exist (never deleted)
            self.assertTrue(any(backup.iterdir()))


class TestPartFileCleanup(unittest.TestCase):
    def test_download_uses_part_and_cleans_on_failure(self):
        wrapper = CapCutTtsWrapper(project_name="")
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            dest = td / "out.mp3"
            part = Path(str(dest) + ".part")

            def bad_get(*a, **k):
                # simulate partial write path via raising mid-download after creating part
                class FakeResp:
                    def raise_for_status(self):
                        pass

                    def iter_content(self, chunk_size=8192):
                        yield b"partial"
                        raise ConnectionError("network dropped")

                    def close(self):
                        pass

                    def __enter__(self):
                        return self

                    def __exit__(self, *a):
                        return False

                return FakeResp()

            with patch("core.capcut_tts.requests.get", side_effect=bad_get):
                with self.assertRaises(ConnectionError):
                    wrapper._download_file("http://example/a.mp3", dest)
            self.assertFalse(part.exists(), "stale .part must be cleaned")
            self.assertFalse(dest.exists(), "final dest must not exist after failed download")

    def test_download_and_cache_cleans_temp_on_cache_failure(self):
        wrapper = CapCutTtsWrapper(project_name="")
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            # point temp dir into our sandbox
            with patch.object(wrapper.config, "project_file", return_value=td):
                with patch.object(wrapper, "_download_file", side_effect=lambda url, p: p.write_bytes(b"x" * 10)):
                    with patch.object(wrapper.cache, "cache_audio", side_effect=RuntimeError("cache boom")):
                        with self.assertRaises(RuntimeError):
                            wrapper._download_and_cache(
                                "http://x", "tid1", "hello", "v", 1.0, "r", 1.0, 1
                            )
                leftovers = list(td.glob("*.part")) + list(td.glob("temp_*.mp3"))
                self.assertEqual(leftovers, [], f"temp leftovers: {leftovers}")

    def test_cache_rejects_empty_audio(self):
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            with patch("core.cache.AppConfig") as cfg:
                cfg.return_value.cache_path = str(td)
                cache = TtsCache()
                empty = td / "empty.mp3"
                empty.write_bytes(b"")
                with self.assertRaises(ValueError):
                    cache.cache_audio(empty, "t", "v", 1.0, "r", duration=1.0)
                self.assertEqual(list(td.glob("*.mp3")), [empty])  # no cache entry


class TestPartialBatchFailure(unittest.TestCase):
    def test_partial_failure_stats_and_warning_not_silent_success(self):
        # minimal draft with 2 captions
        draft = {
            "id": "PID",
            "canvas_config": {"width": 1920, "height": 1080, "ratio": "16:9"},
            "fps": 30.0,
            "duration": 5_000_000,
            "materials": {
                "texts": [
                    {"id": "T1", "content": '{"text":"one"}', "type": "subtitle"},
                    {"id": "T2", "content": '{"text":"two"}', "type": "subtitle"},
                ],
                "audios": [],
                "speeds": [],
                "sound_channel_mappings": [],
                "beats": [],
                "audio_fades": [],
            },
            "tracks": [
                {
                    "id": "TR_TEXT",
                    "type": "text",
                    "segments": [
                        {
                            "id": "S1",
                            "material_id": "T1",
                            "target_timerange": {"start": 0, "duration": 1_000_000},
                            "source_timerange": {"start": 0, "duration": 1_000_000},
                        },
                        {
                            "id": "S2",
                            "material_id": "T2",
                            "target_timerange": {"start": 1_000_000, "duration": 1_000_000},
                            "source_timerange": {"start": 0, "duration": 1_000_000},
                        },
                    ],
                }
            ],
        }
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            draft_path = td / "draft_content.json"
            draft_path.write_text(json.dumps(draft), encoding="utf-8")
            (td / "draft_meta_info.json").write_text("{}", encoding="utf-8")

            svc = CapCutProjectTtsService()
            svc.load_project(draft_path)
            caps = svc.get_captions()
            ids = [c.text_segment_id for c in caps]
            self.assertEqual(len(ids), 2)

            fake_audio = td / "ok.mp3"
            fake_audio.write_bytes(b"ID3" + b"\x00" * 50)

            def half_fail(self, items, voice=None, resource_id=None, rate=None, **kwargs):
                out = {}
                cb = kwargs.get("item_completed_callback")
                for i, it in enumerate(items):
                    idx = int(it["index"])
                    if i == 0:
                        res = {"audio_path": str(fake_audio), "duration": 1.0, "status": "Downloaded"}
                    else:
                        res = {"audio_path": "", "duration": 0.0, "status": "Failed", "error": "boom"}
                    out[idx] = res
                    if cb:
                        cb(idx, res)
                return out

            with patch(
                "core.capcut_project.tts_project_service.CapCutTtsWrapper.generate_tts_batch",
                half_fail,
            ):
                result = svc.generate_and_attach(
                    selected_caption_ids=ids,
                    voice_type="BV074_streaming",
                    resource_id="1",
                    voice_display_name="V",
                    tts_rate=1.0,
                    capcut_clip_speed=1.0,
                    tone_modify_mode=ToneModifyMode.PRESERVE_PITCH,
                    existing_tts_mode="replace_existing",
                    allow_partial_attach=True,
                    use_cache=False,
                    alignment_settings=None,
                )

            self.assertEqual(result.generated + result.cached, 1)
            self.assertEqual(result.failed, 1)
            self.assertTrue(result.success)  # partial attach allowed
            self.assertTrue(result.completed_with_warnings)
            self.assertTrue(result.warnings)
            joined = (" ".join(result.warnings) + " " + " ".join(result.errors)).lower()
            self.assertIn("fail", joined)

    def test_disallow_partial_attach_returns_failure_with_stats(self):
        draft = {
            "id": "PID2",
            "canvas_config": {"width": 1920, "height": 1080, "ratio": "16:9"},
            "fps": 30.0,
            "duration": 5_000_000,
            "materials": {
                "texts": [
                    {"id": "T1", "content": '{"text":"one"}', "type": "subtitle"},
                    {"id": "T2", "content": '{"text":"two"}', "type": "subtitle"},
                ],
                "audios": [],
                "speeds": [],
                "sound_channel_mappings": [],
                "beats": [],
                "audio_fades": [],
            },
            "tracks": [
                {
                    "id": "TR_TEXT",
                    "type": "text",
                    "segments": [
                        {
                            "id": "S1",
                            "material_id": "T1",
                            "target_timerange": {"start": 0, "duration": 1_000_000},
                            "source_timerange": {"start": 0, "duration": 1_000_000},
                        },
                        {
                            "id": "S2",
                            "material_id": "T2",
                            "target_timerange": {"start": 1_000_000, "duration": 1_000_000},
                            "source_timerange": {"start": 0, "duration": 1_000_000},
                        },
                    ],
                }
            ],
        }
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            draft_path = td / "draft_content.json"
            draft_path.write_text(json.dumps(draft), encoding="utf-8")
            (td / "draft_meta_info.json").write_text("{}", encoding="utf-8")
            svc = CapCutProjectTtsService()
            svc.load_project(draft_path)
            ids = [c.text_segment_id for c in svc.get_captions()]
            fake_audio = td / "ok.mp3"
            fake_audio.write_bytes(b"ID3" + b"\x00" * 50)

            def half_fail(self, items, voice=None, resource_id=None, rate=None, **kwargs):
                out = {}
                cb = kwargs.get("item_completed_callback")
                for i, it in enumerate(items):
                    idx = int(it["index"])
                    if i == 0:
                        res = {"audio_path": str(fake_audio), "duration": 1.0, "status": "Downloaded"}
                    else:
                        res = {"audio_path": "", "duration": 0.0, "status": "Failed", "error": "boom"}
                    out[idx] = res
                    if cb:
                        cb(idx, res)
                return out

            with patch(
                "core.capcut_project.tts_project_service.CapCutTtsWrapper.generate_tts_batch",
                half_fail,
            ):
                result = svc.generate_and_attach(
                    selected_caption_ids=ids,
                    voice_type="BV074_streaming",
                    resource_id="1",
                    voice_display_name="V",
                    tts_rate=1.0,
                    capcut_clip_speed=1.0,
                    tone_modify_mode=ToneModifyMode.PRESERVE_PITCH,
                    existing_tts_mode="replace_existing",
                    allow_partial_attach=False,
                    use_cache=False,
                )
            self.assertFalse(result.success)
            self.assertEqual(result.failed, 1)
            self.assertEqual(result.generated, 1)


class TestCloseEventWorker(unittest.TestCase):
    def test_close_event_cancels_and_waits(self):
        try:
            from PySide6.QtWidgets import QApplication
        except ImportError:
            self.skipTest("PySide6 not available")

        app = QApplication.instance() or QApplication([])
        from ui.main_window import MainWindow, GenerateWorker

        win = MainWindow()
        # fake a running worker without real TTS
        worker = GenerateWorker(
            service=win.service,
            selected_ids=["x"],
            voice_type="v",
            resource_id="r",
            voice_display_name="n",
            tts_rate=1.0,
            clip_speed=1.0,
            tone_mode=ToneModifyMode.PRESERVE_PITCH,
            existing_mode="replace_existing",
            use_cache=False,
        )
        # prevent real run work: override run to wait until cancel
        def slow_run():
            for _ in range(200):
                if worker._cancel:
                    return
                time.sleep(0.02)

        worker.run = slow_run  # type: ignore
        win.worker = worker
        worker.start()
        self.assertTrue(worker.isRunning())

        # closeEvent should request cancel and wait
        from PySide6.QtGui import QCloseEvent

        ev = QCloseEvent()
        win.closeEvent(ev)
        self.assertTrue(ev.isAccepted() or not worker.isRunning())
        # thread must not still be running after close wait
        self.assertFalse(worker.isRunning(), "QThread still running after closeEvent")
        win.deleteLater()


class TestWorkbenchUi(unittest.TestCase):
    """UI structure / collapse / selection — no real TTS."""

    @classmethod
    def setUpClass(cls):
        try:
            from PySide6.QtWidgets import QApplication
        except ImportError:
            raise unittest.SkipTest("PySide6 not available")
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        from ui.main_window import MainWindow

        self.win = MainWindow()

    def tearDown(self):
        self.win.close()
        self.win.deleteLater()

    def test_required_widgets_exist(self):
        for name in (
            "ed_project",
            "lbl_info",
            "cmb_lang",
            "cmb_voice",
            "ed_voice_search",
            "lbl_voice_adv",
            "sp_clip_speed",
            "cmb_tone",
            "cmb_existing",
            "chk_cache",
            "chk_align",
            "sp_trim_frames",
            "sp_fade_ms",
            "lbl_align_hint",
            "ed_search",
            "chk_hide_empty",
            "chk_only_no_tts",
            "chk_only_errors",
            "lbl_selection",
            "table",
            "log",
            "progress",
            "lbl_progress",
            "btn_cancel",
            "btn_generate",
            "btn_export_srt",
            "advanced_content",
            "btn_advanced",
            "main_split",
        ):
            self.assertTrue(hasattr(self.win, name), f"missing widget attr: {name}")

    def test_uses_windows_blue_accent(self):
        from qfluentwidgets import qconfig

        self.assertEqual(qconfig.themeColor.value.name().lower(), "#0078d4")

    def test_advanced_collapsed_by_default_frees_height(self):
        # isVisible() is False while window is hidden; isHidden tracks setVisible
        self.assertTrue(self.win.advanced_content.isHidden())
        self.assertFalse(self.win._advanced_open)
        self.win._toggle_advanced()
        self.assertFalse(self.win.advanced_content.isHidden())
        self.assertTrue(self.win._advanced_open)
        self.win._toggle_advanced()
        self.assertTrue(self.win.advanced_content.isHidden())
        self.assertFalse(self.win._advanced_open)

    def test_align_toggle_disables_trim_fade(self):
        from PySide6.QtCore import Qt

        self.win.chk_align.setChecked(True)
        self.win._on_align_toggled(Qt.Checked)
        self.assertTrue(self.win.sp_trim_frames.isEnabled())
        self.assertTrue(self.win.sp_fade_ms.isEnabled())
        self.win.chk_align.setChecked(False)
        self.win._on_align_toggled(Qt.Unchecked)
        self.assertFalse(self.win.sp_trim_frames.isEnabled())
        self.assertFalse(self.win.sp_fade_ms.isEnabled())

    def test_selection_label_format(self):
        # empty table
        self.win._update_selection_label()
        self.assertIn("Đã chọn 0/0", self.win.lbl_selection.text())
        self.assertIn("Hiển thị", self.win.lbl_selection.text())

    def test_table_header_noi_dung(self):
        headers = [
            self.win.table.horizontalHeaderItem(i).text()
            for i in range(self.win.table.columnCount())
        ]
        self.assertIn("Nội dung", headers)
        self.assertNotIn("Caption", headers)

    def test_splitter_prefers_caption_height(self):
        factors = getattr(self.win, "_split_stretch", (4, 1))
        self.assertGreater(factors[0], factors[1])
        self.assertGreaterEqual(self.win.table.minimumHeight(), 200)

    def test_cancel_sets_cancelling_label(self):
        from ui.main_window import GenerateWorker, UiState

        worker = GenerateWorker(
            service=self.win.service,
            selected_ids=["x"],
            voice_type="v",
            resource_id="r",
            voice_display_name="n",
            tts_rate=1.0,
            clip_speed=1.0,
            tone_mode=ToneModifyMode.PRESERVE_PITCH,
            existing_mode="replace_existing",
            use_cache=False,
        )

        def slow_run():
            for _ in range(100):
                if worker._cancel:
                    return
                time.sleep(0.02)

        worker.run = slow_run  # type: ignore
        self.win.worker = worker
        worker.start()
        self.win._apply_ui_state(UiState.GENERATING)
        self.win._cancel()
        self.assertEqual(self.win._ui_state, UiState.CANCELLING)
        self.assertIn("hủy", self.win.btn_generate.text().lower())
        worker.wait(3000)
        self.win.worker = None

    def test_info_label_no_hardcoded_html_color_span(self):
        # after init, info is plain text
        self.assertNotIn('style=', self.win.lbl_info.text())
        self.assertNotIn("#64748B", self.win.lbl_info.text())

    def test_no_project_disables_generate_and_reload(self):
        from ui.main_window import UiState

        self.win.ed_project.setText("")
        self.win._apply_ui_state(UiState.IDLE_NO_PROJECT)
        self.assertFalse(self.win.btn_generate.isEnabled())
        self.assertFalse(self.win.btn_reload.isEnabled())
        self.assertFalse(self.win.btn_export_srt.isEnabled())
        self.assertFalse(self.win.btn_cancel.isEnabled())

    def test_ready_enables_generate(self):
        from ui.main_window import UiState

        self.win.ed_project.setText("C:/fake/project")
        self.win._apply_ui_state(UiState.IDLE_READY)
        self.assertTrue(self.win.btn_generate.isEnabled())
        self.assertTrue(self.win.btn_reload.isEnabled())
        self.assertTrue(self.win.btn_export_srt.isEnabled())
        self.assertFalse(self.win.btn_cancel.isEnabled())

    def test_generating_disables_settings_and_selection(self):
        from ui.main_window import UiState
        from PySide6.QtCore import Qt
        from PySide6.QtWidgets import QTableWidgetItem

        # seed one selectable row
        self.win.table.setRowCount(1)
        chk = QTableWidgetItem()
        chk.setFlags(Qt.ItemIsUserCheckable | Qt.ItemIsEnabled)
        chk.setCheckState(Qt.Checked)
        self.win.table.setItem(0, 0, chk)
        for col in range(1, 9):
            self.win.table.setItem(0, col, QTableWidgetItem("x" if col != 6 else "Sẵn sàng"))

        self.win._apply_ui_state(UiState.GENERATING)
        self.assertFalse(self.win.btn_browse.isEnabled())
        self.assertFalse(self.win.btn_reload.isEnabled())
        self.assertFalse(self.win.btn_export_srt.isEnabled())
        self.assertFalse(self.win.cmb_voice.isEnabled())
        self.assertFalse(self.win.ed_search.isEnabled())
        self.assertFalse(self.win.btn_select_all.isEnabled())
        self.assertFalse(self.win.chk_only_errors.isEnabled())
        # checkbox flags frozen (not user-checkable)
        flags = self.win.table.item(0, 0).flags()
        self.assertFalse(bool(flags & Qt.ItemIsUserCheckable))

        self.win._apply_ui_state(UiState.IDLE_READY)
        self.assertTrue(self.win.cmb_voice.isEnabled())
        self.assertTrue(self.win.ed_search.isEnabled())
        self.assertTrue(bool(self.win.table.item(0, 0).flags() & Qt.ItemIsUserCheckable))

    def test_filter_only_errors(self):
        from PySide6.QtCore import Qt
        from PySide6.QtWidgets import QTableWidgetItem

        self.win.table.setRowCount(2)
        for row, status in enumerate(["Sẵn sàng", "Lỗi"]):
            chk = QTableWidgetItem()
            chk.setFlags(Qt.ItemIsUserCheckable | Qt.ItemIsEnabled)
            chk.setCheckState(Qt.Unchecked)
            self.win.table.setItem(row, 0, chk)
            self.win.table.setItem(row, 1, QTableWidgetItem(str(row + 1)))
            self.win.table.setItem(row, 2, QTableWidgetItem("0"))
            self.win.table.setItem(row, 3, QTableWidgetItem("1"))
            self.win.table.setItem(row, 4, QTableWidgetItem(f"text{row}"))
            self.win.table.setItem(row, 5, QTableWidgetItem("Không"))
            self.win.table.setItem(row, 6, QTableWidgetItem(status))
            self.win.table.setItem(row, 7, QTableWidgetItem(""))
            self.win.table.setItem(row, 8, QTableWidgetItem("boom" if status == "Lỗi" else ""))

        self.win.chk_hide_empty.setChecked(False)
        self.win.chk_only_no_tts.setChecked(False)
        self.win.chk_only_errors.setChecked(True)
        self.win._filter_table()
        self.assertTrue(self.win.table.isRowHidden(0))
        self.assertFalse(self.win.table.isRowHidden(1))

    def test_advanced_collapsed_sizehint_smaller(self):
        # sizeHint of content may stay constant; maximumHeight + isHidden free layout space
        self.assertEqual(self.win.advanced_content.maximumHeight(), 0)
        self.assertTrue(self.win.advanced_content.isHidden())
        self.win._toggle_advanced()  # open
        self.assertGreater(self.win.advanced_content.maximumHeight(), 0)
        self.assertFalse(self.win.advanced_content.isHidden())
        # Prefer geometry height after open when widget is shown
        self.win.show()
        self.app.processEvents()
        open_h = max(self.win.advanced_content.height(), self.win.advanced_content.sizeHint().height())
        self.win._toggle_advanced()  # close
        self.app.processEvents()
        self.assertEqual(self.win.advanced_content.maximumHeight(), 0)
        self.assertTrue(self.win.advanced_content.isHidden())
        self.assertGreater(open_h, 0)

    def test_footer_progress_label_min_width(self):
        self.assertGreaterEqual(self.win.lbl_progress.minimumWidth(), 120)

    def test_clip_speed_control_range_and_label(self):
        self.assertAlmostEqual(self.win.sp_clip_speed.minimum(), 0.1)
        self.assertAlmostEqual(self.win.sp_clip_speed.maximum(), 10.0)
        self.assertAlmostEqual(self.win.sp_clip_speed.singleStep(), 0.05)
        self.assertAlmostEqual(float(self.win.sp_clip_speed.value()), 1.0, places=2)
        self.assertFalse(hasattr(self.win, "sp_tts_rate"))

    def test_start_generate_uses_fixed_tts_rate_and_clip_speed(self):
        """API tts_rate is fixed at 1.0; UI only exposes clip_speed."""
        from ui.main_window import GenerateWorker, UiState
        from unittest.mock import patch
        from PySide6.QtCore import Qt
        from PySide6.QtWidgets import QTableWidgetItem

        self.win.ed_project.setText("C:/fake")
        self.win._apply_ui_state(UiState.IDLE_READY)

        self.win.table.setRowCount(1)
        chk = QTableWidgetItem()
        chk.setFlags(Qt.ItemIsUserCheckable | Qt.ItemIsEnabled)
        chk.setCheckState(Qt.Checked)
        self.win.table.setItem(0, 0, chk)
        idx_item = QTableWidgetItem("1")
        idx_item.setData(Qt.UserRole, "seg-1")
        self.win.table.setItem(0, 1, idx_item)
        for col in range(2, 9):
            self.win.table.setItem(0, col, QTableWidgetItem("x" if col != 6 else "Sẵn sàng"))

        class V:
            voice_type = "vt"
            resource_id = "rid"
            display_name = "Voice"

        self.win.cmb_voice.blockSignals(True)
        self.win.cmb_voice.clear()
        self.win.cmb_voice.addItem("Voice", userData=V())
        self.win.cmb_voice.setCurrentIndex(0)
        self.win.cmb_voice.blockSignals(False)

        self.win.sp_clip_speed.setValue(1.70)
        captured = {}

        class FakeWorker(GenerateWorker):
            def __init__(self, *args, **kwargs):
                captured["tts_rate"] = kwargs.get("tts_rate", args[5] if len(args) > 5 else None)
                captured["clip_speed"] = kwargs.get("clip_speed", args[6] if len(args) > 6 else None)
                super().__init__(*args, **kwargs)

            def start(self):
                return None

        with patch("ui.main_window.GenerateWorker", FakeWorker):
            with patch.object(self.win.service, "is_capcut_running", return_value=False):
                self.win._start_generate()

        self.assertAlmostEqual(float(captured["tts_rate"]), 1.0, places=2)
        self.assertAlmostEqual(float(captured["clip_speed"]), 1.70, places=2)

    def test_generating_disables_clip_speed(self):
        from ui.main_window import UiState

        self.win._apply_ui_state(UiState.IDLE_READY)
        self.assertTrue(self.win.sp_clip_speed.isEnabled())
        self.win._apply_ui_state(UiState.GENERATING)
        self.assertFalse(self.win.sp_clip_speed.isEnabled())
        self.win._apply_ui_state(UiState.CANCELLING)
        self.assertFalse(self.win.sp_clip_speed.isEnabled())
        self.win._apply_ui_state(UiState.IDLE_READY)
        self.assertTrue(self.win.sp_clip_speed.isEnabled())


class TestCancelResultUiContract(unittest.TestCase):
    def test_cancel_result_not_success(self):
        svc = CapCutProjectTtsService()
        with tempfile.TemporaryDirectory() as td:
            r = svc._cancel_result(Path(td), time.perf_counter())
            self.assertFalse(r.success)
            self.assertTrue(r.errors)
            self.assertIn("cancel", r.errors[0].lower())


if __name__ == "__main__":
    unittest.main()
