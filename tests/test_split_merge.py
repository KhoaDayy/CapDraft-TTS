"""Runnable checks for project split + video merge helpers."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from core.capcut_project.project_split import (  # noqa: E402
    choose_cut_us,
    slice_draft,
    split_project,
)
from core.capcut_project.video_merge import resolve_ffmpeg  # noqa: E402


def _mini_draft(duration_us: int = 10_000_000) -> dict:
    mid = duration_us // 2
    return {
        "id": "SRC-ID",
        "name": "Demo",
        "duration": duration_us,
        "fps": 30.0,
        "canvas_config": {"width": 1080, "height": 1920},
        "tracks": [
            {
                "id": "v1",
                "type": "video",
                "segments": [
                    {
                        "id": "vs1",
                        "material_id": "vm1",
                        "target_timerange": {"start": 0, "duration": duration_us},
                        "source_timerange": {"start": 0, "duration": duration_us},
                        "speed": 1.0,
                        "extra_material_refs": [],
                    }
                ],
            },
            {
                "id": "t1",
                "type": "text",
                "segments": [
                    {
                        "id": "ts1",
                        "material_id": "tm1",
                        "target_timerange": {"start": 1_000_000, "duration": 2_000_000},
                        "source_timerange": {"start": 0, "duration": 2_000_000},
                        "extra_material_refs": [],
                    },
                    {
                        "id": "ts2",
                        "material_id": "tm2",
                        "target_timerange": {"start": mid + 500_000, "duration": 1_500_000},
                        "source_timerange": {"start": 0, "duration": 1_500_000},
                        "extra_material_refs": [],
                    },
                ],
            },
            {
                "id": "a1",
                "type": "audio",
                "segments": [
                    {
                        "id": "as1",
                        "material_id": "am1",
                        "target_timerange": {"start": mid - 1_000_000, "duration": 2_000_000},
                        "source_timerange": {"start": 0, "duration": 2_000_000},
                        "speed": 1.0,
                        "extra_material_refs": ["sp1"],
                    }
                ],
            },
        ],
        "materials": {
            "videos": [{"id": "vm1", "path": "C:/x.mp4", "type": "video"}],
            "texts": [
                {"id": "tm1", "type": "subtitle", "content": '{"text":"a"}'},
                {"id": "tm2", "type": "subtitle", "content": '{"text":"b"}'},
            ],
            "audios": [{"id": "am1", "type": "text_to_audio", "path": "C:/a.mp3"}],
            "speeds": [{"id": "sp1", "type": "speed", "speed": 1.0}],
            "beats": [{"id": "orphan", "type": "beats"}],
        },
    }


class TestChooseCut(unittest.TestCase):
    def test_midpoint_default(self):
        self.assertEqual(choose_cut_us(10_000_000), 5_000_000)

    def test_snap_to_caption_edge(self):
        cut = choose_cut_us(10_000_000, [4_800_000, 7_000_000])
        self.assertEqual(cut, 4_800_000)


class TestSliceDraft(unittest.TestCase):
    def test_part1_and_part2_ranges(self):
        draft = _mini_draft(10_000_000)
        cut = 5_000_000
        p1, n1 = slice_draft(draft, cut, part=1, name_suffix=" (Part 1)")
        p2, n2 = slice_draft(draft, cut, part=2, name_suffix=" (Part 2)")
        self.assertEqual(p1["duration"], cut)
        self.assertEqual(p2["duration"], 5_000_000)
        self.assertNotEqual(p1["id"], draft["id"])
        self.assertNotEqual(p2["id"], p1["id"])
        self.assertNotEqual(p1["id"], p2["id"])
        self.assertGreater(n1, 0)
        self.assertGreater(n2, 0)

        # part1 text: only early caption
        text_segs_p1 = [
            s
            for t in p1["tracks"]
            if t.get("type") == "text"
            for s in (t.get("segments") or [])
        ]
        self.assertEqual(len(text_segs_p1), 1)
        self.assertEqual(text_segs_p1[0]["material_id"], "tm1")

        # part2 text: late caption shifted
        text_segs_p2 = [
            s
            for t in p2["tracks"]
            if t.get("type") == "text"
            for s in (t.get("segments") or [])
        ]
        self.assertEqual(len(text_segs_p2), 1)
        self.assertEqual(text_segs_p2[0]["material_id"], "tm2")
        self.assertEqual(text_segs_p2[0]["target_timerange"]["start"], 500_000)

        # overlapping audio split across both parts
        audio_p1 = [
            s
            for t in p1["tracks"]
            if t.get("type") == "audio"
            for s in (t.get("segments") or [])
        ]
        audio_p2 = [
            s
            for t in p2["tracks"]
            if t.get("type") == "audio"
            for s in (t.get("segments") or [])
        ]
        self.assertEqual(len(audio_p1), 1)
        self.assertEqual(len(audio_p2), 1)
        self.assertEqual(audio_p1[0]["target_timerange"]["duration"], 1_000_000)
        self.assertEqual(audio_p2[0]["target_timerange"]["start"], 0)
        self.assertEqual(audio_p2[0]["target_timerange"]["duration"], 1_000_000)
        # source advanced for part2 head trim
        self.assertEqual(audio_p2[0]["source_timerange"]["start"], 1_000_000)

        # orphan beats pruned; speed kept via extra ref
        p1_ids = {x["id"] for x in p1["materials"]["speeds"]}
        self.assertIn("sp1", p1_ids)
        self.assertEqual(p1["materials"].get("beats"), [])

    def test_nested_combination_sliced(self):
        draft = _mini_draft(10_000_000)
        # Outer wrapper segment references a combination draft (AutoVideo style)
        combo_id = "combo-1"
        draft["tracks"] = [
            {
                "id": "v1",
                "type": "video",
                "segments": [
                    {
                        "id": "vs1",
                        "material_id": "vm1",
                        "target_timerange": {"start": 0, "duration": 10_000_000},
                        "source_timerange": {"start": 0, "duration": 10_000_000},
                        "speed": 1.0,
                        "extra_material_refs": [combo_id, "sp1"],
                    }
                ],
            }
        ]
        draft["materials"]["drafts"] = [
            {
                "id": combo_id,
                "type": "combination",
                "draft": {
                    "id": "nested-id",
                    "name": "Clip",
                    "duration": 10_000_000,
                    "tracks": [
                        {
                            "id": "nt",
                            "type": "text",
                            "segments": [
                                {
                                    "id": "nts1",
                                    "material_id": "tm1",
                                    "target_timerange": {"start": 1_000_000, "duration": 1_000_000},
                                    "source_timerange": {"start": 0, "duration": 1_000_000},
                                    "extra_material_refs": [],
                                },
                                {
                                    "id": "nts2",
                                    "material_id": "tm2",
                                    "target_timerange": {"start": 6_000_000, "duration": 1_000_000},
                                    "source_timerange": {"start": 0, "duration": 1_000_000},
                                    "extra_material_refs": [],
                                },
                            ],
                        }
                    ],
                    "materials": {
                        "texts": [
                            {"id": "tm1", "type": "subtitle"},
                            {"id": "tm2", "type": "subtitle"},
                        ]
                    },
                },
            }
        ]
        cut = 5_000_000
        p1, _ = slice_draft(draft, cut, part=1, name_suffix="")
        p2, _ = slice_draft(draft, cut, part=2, name_suffix="")
        n1 = p1["materials"]["drafts"][0]["draft"]
        n2 = p2["materials"]["drafts"][0]["draft"]
        self.assertEqual(n1["duration"], cut)
        self.assertEqual(n2["duration"], cut)
        self.assertEqual(n1["id"], "nested-id")  # subdraft folder id preserved
        segs1 = n1["tracks"][0]["segments"]
        segs2 = n2["tracks"][0]["segments"]
        self.assertEqual(len(segs1), 1)
        self.assertEqual(segs1[0]["material_id"], "tm1")
        self.assertEqual(len(segs2), 1)
        self.assertEqual(segs2[0]["material_id"], "tm2")
        self.assertEqual(segs2[0]["target_timerange"]["start"], 1_000_000)
        # Outer source reset after nested rewrite
        outer2 = p2["tracks"][0]["segments"][0]
        self.assertEqual(outer2["source_timerange"]["start"], 0)
        self.assertEqual(outer2["source_timerange"]["duration"], cut)

    def test_split_project_writes_dirs(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            src = root / "MyProj"
            src.mkdir()
            (src / "draft_content.json").write_text("{}", encoding="utf-8")
            (src / "textReading").mkdir()
            (src / "textReading" / "a.mp3").write_bytes(b"ID3")
            draft = _mini_draft(8_000_000)
            (src / "draft_content.json").write_text(
                json.dumps(draft), encoding="utf-8"
            )
            res = split_project(
                source_project_dir=src,
                source_draft=draft,
                source_draft_path=src / "draft_content.json",
                cut_us=4_000_000,
            )
            self.assertTrue(res.part1_dir.is_dir())
            self.assertTrue(res.part2_dir.is_dir())
            self.assertTrue((res.part1_dir / "draft_content.json").is_file())
            self.assertTrue((res.part2_dir / "textReading" / "a.mp3").is_file())
            d1 = json.loads((res.part1_dir / "draft_content.json").read_text(encoding="utf-8"))
            d2 = json.loads((res.part2_dir / "draft_content.json").read_text(encoding="utf-8"))
            self.assertEqual(d1["duration"], 4_000_000)
            self.assertEqual(d2["duration"], 4_000_000)
            # original untouched
            orig = json.loads((src / "draft_content.json").read_text(encoding="utf-8"))
            self.assertEqual(orig["duration"], 8_000_000)


class TestResolveFfmpeg(unittest.TestCase):
    def test_missing_raises(self):
        with patch("core.capcut_project.video_merge.shutil.which", return_value=None):
            with patch("core.capcut_project.video_merge.Path.is_file", return_value=False):
                with self.assertRaises(FileNotFoundError):
                    resolve_ffmpeg("not-a-real-ffmpeg-binary-xyz")


if __name__ == "__main__":
    unittest.main()
