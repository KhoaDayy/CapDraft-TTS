"""Record wave-5 units (the whole generate-and-attach pipeline, video merge) into rust/golden/.

Run from the repo root:  python rust/tools/record_wave5.py
The TTS server is FakeCapCut (fakecapcut.py); ids, clocks and random sources are
deterministic (ADR-012). Merges run the real ffmpeg through a logging wrapper script.
"""

from __future__ import annotations

import datetime as dt
import json
import logging
import random
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

import requests

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from fakecapcut import FakeCapCut  # noqa: E402
from core import config as config_mod  # noqa: E402
from core.capcut_project import project_exporter as pe  # noqa: E402
from core.capcut_project import tts_project_service as tps  # noqa: E402
from core.capcut_project import video_merge as vm  # noqa: E402
from core.capcut_project.models import ToneModifyMode  # noqa: E402
from core.capcut_project.native_audio_alignment import NativeAudioAlignmentSettings  # noqa: E402

logging.disable(logging.CRITICAL)
OUT = ROOT / "rust" / "golden"
CLOCK = 1_700_000_000.25
state = {"uuid": 0, "rand": 0, "bytes": 0}


def reset():
    state.update(uuid=0, rand=0, bytes=0)


def fake_uuid4():
    state["uuid"] += 1
    return uuid.UUID(int=state["uuid"])


def fake_randint(a, b):
    state["rand"] += 1
    return a + (state["rand"] * 7) % (b - a + 1)


def fake_token_bytes(n):
    c = state["bytes"]
    state["bytes"] += n
    return bytes(((c + i) * 73) % 256 for i in range(n))


class FixedDatetime(dt.datetime):
    @classmethod
    def now(cls, tz=None):
        return dt.datetime(2023, 11, 14, 22, 13, 20)


uuid.uuid4 = fake_uuid4
time.time = lambda: CLOCK
time.sleep = lambda s: None
random.randint = fake_randint
secrets.token_bytes = fake_token_bytes
pe.datetime = FixedDatetime
tps.datetime = FixedDatetime


def write(name: str, rows: list[dict]) -> None:
    path = OUT / f"{name}.jsonl"
    with path.open("w", encoding="utf-8", newline="\n") as f:
        for row in rows:
            f.write(json.dumps(row, ensure_ascii=False, allow_nan=False) + "\n")
    print(f"{path.relative_to(ROOT)}: {len(rows)} cases")


def snapshot(base: Path) -> dict:
    out = {}
    for p in sorted(base.rglob("*")):
        if p.is_file() and p.name != "config.json":
            data = p.read_bytes()
            try:
                text = data.decode("utf-8")
            except UnicodeDecodeError:
                text = "hex:" + data.hex()
            out[p.relative_to(base).as_posix()] = text.replace(str(base), "<ROOT>")
    return out


def compact(o):
    return json.dumps(o, ensure_ascii=False, separators=(",", ":"))


PH = "##_draftpath_placeholder_0A1B2C3D-AAAA-BBBB-1234-56789ABCDEF0_##"


def project_draft(texts, existing=(), fps=30.0):
    """A CapCut-like project: one text track, a video track, optional TTS on some captions."""
    mats = {"texts": [], "videos": [{"id": "V1", "type": "video", "duration": 0, "path": f"{PH}/media/v.mp4"}],
            "audios": [], "speeds": [], "audio_fades": [], "placeholder_infos": [], "beats": [],
            "sound_channel_mappings": [], "vocal_separations": [], "canvases": [{"id": "C1", "type": "canvas_color"}]}
    text_segs, audio_segs = [], []
    for i, t in enumerate(texts):
        mats["texts"].append({"id": f"M{i:03d}", "type": "subtitle", "content": compact({"text": t, "styles": []}),
                              "text_to_audio_ids": []})
        text_segs.append({"id": f"S{i:03d}", "material_id": f"M{i:03d}",
                          "target_timerange": {"start": i * 1_500_000, "duration": 1_200_000},
                          "extra_material_refs": []})
        if i in existing:
            mats["audios"].append({"id": f"A{i:03d}", "type": "text_to_audio", "text_id": f"M{i:03d}",
                                   "duration": 1_000_000, "path": f"{PH}/textReading/old_{i}.mp3",
                                   "resource_id": "OLD", "tone_speaker": "old"})
            mats["speeds"].append({"id": f"SP{i:03d}", "type": "speed", "speed": 1.0, "mode": 0, "curve_speed": None})
            audio_segs.append({"id": f"AS{i:03d}", "material_id": f"A{i:03d}", "extra_material_refs": [f"SP{i:03d}"],
                               "source_timerange": {"start": 0, "duration": 1_000_000},
                               "target_timerange": {"start": i * 1_500_000, "duration": 1_000_000},
                               "speed": 1.0, "is_tone_modify": False, "volume": 1.0})
            mats["texts"][-1]["text_to_audio_ids"] = [f"AS{i:03d}"]
    total = max(1, len(texts)) * 1_500_000
    mats["videos"][0]["duration"] = total
    tracks = [{"id": "TV", "type": "video", "segments": [
        {"id": "VS", "material_id": "V1", "source_timerange": {"start": 0, "duration": total},
         "target_timerange": {"start": 0, "duration": total}, "speed": 1.0, "extra_material_refs": []}]},
        {"id": "TT", "type": "text", "segments": text_segs}]
    if audio_segs:
        tracks.append({"id": "TA", "type": "audio", "segments": audio_segs})
    return {"id": "PROJ-0001", "name": "", "duration": total, "fps": fps, "version": 360000, "new_version": "110.0.0",
            "canvas_config": {"width": 1920, "height": 1080, "ratio": "original"}, "materials": mats, "tracks": tracks}


T5 = ["Xin chào", "Câu thứ hai", "Third line", "Bốn 4", "năm"]
SCENARIOS = [
    dict(name="fresh", texts=T5),
    dict(name="replace", texts=T5, existing=[0, 2, 3]),
    dict(name="skip_existing", texts=T5, existing=[0, 2], mode="skip_existing"),
    dict(name="selected_only", texts=T5, existing=[1], mode="selected_only", selection=["S001", "3", "M004"]),
    dict(name="partial", texts=["ok one", "#DLFAIL two", "three", "#NOURL four", "five"]),
    dict(name="no_partial", texts=["ok one", "#DLFAIL two", "three"], allow_partial=False),
    dict(name="all_fail", texts=["#HTTP500 a", "#HTTP500 b"]),
    dict(name="empty_rows", texts=["", "   ", "real text", ""], selection=["S000", "S001", "S002", "S003"]),
    dict(name="cancel_early", texts=T5, cancel_after=0),
    dict(name="cancel_in_tts", texts=T5, cancel_after=6),
    dict(name="cancel_after_tts", texts=["a", "b"], cancel_after=19),
    dict(name="no_meta", texts=T5[:2], meta=False, timelines=False),
    dict(name="cached", texts=T5, pre_cache=["Xin chào", "năm"]),
    dict(name="no_cache", texts=T5[:3], pre_cache=["Xin chào"], use_cache=False),
    dict(name="speed_tone", texts=T5, clip_speed=1.3, tone="follow_speed", rate=1.25,
         alignment={"enabled": False, "leading_trim_frames": 3.0, "fade_in_ms": 8.0}),
    dict(name="align_custom", texts=T5, fps=25.0,
         alignment={"enabled": True, "leading_trim_frames": 1.5, "fade_in_ms": 30.0}),
    dict(name="nothing_selected", texts=T5, selection=["nope"]),
    dict(name="audio_vanishes", texts=T5, pre_cache=["năm"], delete_audio_of=[2, 5]),
    dict(name="many", texts=[f"Dòng {i}" for i in range(45)] + ["#DLFAIL x"]),
]

rows = []
for sc in SCENARIOS:
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp).resolve()
        draft = project_draft(sc["texts"], sc.get("existing", ()), sc.get("fps", 30.0))
        files = {"Proj/draft_content.json": compact(draft)}
        if sc.get("timelines", True):
            files["Proj/Timelines/PROJ-0001/draft_content.json"] = compact(draft)
        if sc.get("meta", True):
            files["Proj/draft_meta_info.json"] = compact({"draft_id": "META", "draft_name": "Proj"})
        cfg = {"tts_chunk_size": 3, "tts_parallel_chunks": 1, "tts_download_workers": 1, "tts_poll_interval_sec": 1.0,
               "capcut_tts_path": str(ROOT / "external" / "capcut-tts-api"),
               "device_json_path": str(ROOT / "external" / "capcut-tts-api" / "device.json"),
               "tts_device_profile": "legacy", "max_backups": 10}
        files["config.json"] = json.dumps(cfg)
        for rel, text in files.items():
            p = base / rel
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(text, encoding="utf-8")
        config_mod.APP_ROOT = base
        config_mod.AppConfig._instance = None
        fake = FakeCapCut()
        requests.post = fake.post
        requests.get = fake.get
        events, progress, items = [], [], []
        svc = tps.CapCutProjectTtsService(log_callback=lambda e: events.append(
            [e.level, e.message, e.stage, e.caption_index, e.progress]))
        for t in sc.get("pre_cache", []):
            src = base / "pre.mp3"
            src.write_bytes(f"CACHED:{t}".encode())
            from core.cache import TtsCache
            TtsCache().cache_audio(src, t, "BV421_vivn_streaming", sc.get("rate", 1.0), "7252594014782755330",
                                   duration=1.75)
            src.unlink()
        info = svc.load_project(base / "Proj")
        selection = sc.get("selection") or [c.text_segment_id for c in svc.get_captions()]
        checks = [0]
        limit = sc.get("cancel_after")

        def cancelled():
            checks[0] += 1
            return limit is not None and checks[0] > limit
        al = sc.get("alignment", {"enabled": True, "leading_trim_frames": 3.0, "fade_in_ms": 8.0})
        reset()
        try:
            res = svc.generate_and_attach(
                selection, "BV421_vivn_streaming", "7252594014782755330", "Nhỏ Ngọt Ngào", sc.get("rate", 1.0),
                sc.get("clip_speed", 1.0), ToneModifyMode(sc.get("tone", "preserve_pitch")),
                existing_tts_mode=sc.get("mode", "replace_existing"), allow_partial_attach=sc.get("allow_partial", True),
                progress_callback=lambda p, m: progress.append([p, m]),
                item_completed_callback=lambda i, r: (items.append([i, dict(r)]),
                                                      Path(r["audio_path"]).unlink()
                                                      if i in sc.get("delete_audio_of", []) else None),
                is_cancelled_callback=cancelled, use_cache=sc.get("use_cache", True),
                alignment_settings=NativeAudioAlignmentSettings(**al))
            out = {k: (str(v) if isinstance(v, Path) else v) for k, v in res.__dict__.items() if k != "processing_seconds"}
            out["items"] = [dict(r.__dict__) for r in res.items]
            outcome = {"ok": out}
        except Exception as exc:  # noqa: BLE001
            outcome = {"err": f"{type(exc).__name__}: {exc}"}
        for e in events:
            if e[1].startswith("Processing time:"):
                e[1] = "Processing time: <t>"
        record = {"result": outcome, "events": events, "progress": progress, "items": items,
                  "cancel_checks": checks[0], "requests": fake.log}
        record = json.loads(json.dumps(record, ensure_ascii=False).replace(str(base), "<ROOT>"))
        rows.append({"fn": "generate_and_attach", "args": [sc, files], "ok": record, "tree": snapshot(base)})
write("pipeline", rows)

# ======================================================================== video merge
rows = []
FFMPEG = shutil.which("ffmpeg")
WRAPPERS = {
    "ok": "",
    "copy_fails": 'case "$*" in *"-c copy"*) echo "copy refused" >&2; exit 1;; esac\n',
    "all_fail": 'echo "codec explodes: $1" >&2\necho "second line" >&2\nexit 3\n',
    "no_output": 'exit 0\n',
}


def wrapper_script(kind: str) -> str:
    return ("#!/bin/sh\n"
            'log="$(dirname "$0")/ffmpeg.log"\n'
            'printf "ARGS:" >> "$log"; for a in "$@"; do printf " [%s]" "$a" >> "$log"; done; echo >> "$log"\n'
            'prev=""; for a in "$@"; do if [ "$prev" = "-i" ]; then echo "LIST:" >> "$log"; cat "$a" >> "$log"; fi; '
            'prev="$a"; done\n'
            + WRAPPERS[kind]
            + f'exec {FFMPEG} -hide_banner -loglevel error "$@"\n')


MERGES = [("ok", ["a.mp4", "b.mp4"], "out.mp4", False), ("ok", ["a.mp4", "b.mp4", "a.mp4"], "joined.txt", False),
          ("copy_fails", ["a.mp4", "b.mp4"], "out.mp4", False), ("ok", ["a.mp4", "b.mp4"], "sub/out.mkv", True),
          ("all_fail", ["a.mp4", "b.mp4"], "out.mp4", False), ("no_output", ["a.mp4", "b.mp4"], "out.mp4", True),
          ("ok", ["a.mp4"], "out.mp4", False), ("ok", ["a.mp4", "missing.mp4"], "out.mp4", False),
          ("ok", ["it's a.mp4", "b.mp4"], "out.mov", False)]
for kind, inputs, output, reencode in MERGES:
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp).resolve()
        for name, freq in (("a.mp4", 440), ("b.mp4", 660), ("it's a.mp4", 550)):
            subprocess.run([FFMPEG, "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
                            "testsrc=duration=0.5:size=32x32:rate=10", "-f", "lavfi", "-i",
                            f"sine=frequency={freq}:duration=0.5", "-shortest", "-c:v", "libx264", "-c:a", "aac",
                            str(base / name)], check=True)
        tool = base / "tools" / "ffmpeg"
        tool.parent.mkdir()
        tool.write_text(wrapper_script(kind))
        tool.chmod(0o755)
        progress = []
        try:
            r = vm.merge_videos([base / i for i in inputs], base / output, ffmpeg_path=str(tool), reencode=reencode,
                                progress_callback=lambda p, m: progress.append([p, m]))
            outcome = {"ok": {"output_path": str(r.output_path), "used_reencode": r.used_reencode,
                              "ffmpeg_path": r.ffmpeg_path, "output_size_positive": r.output_path.stat().st_size > 0}}
        except Exception as exc:  # noqa: BLE001
            outcome = {"err": f"{type(exc).__name__}: {exc}"}
        log = (base / "tools" / "ffmpeg.log").read_text() if (base / "tools" / "ffmpeg.log").exists() else ""
        rec = json.loads(json.dumps({"result": outcome, "progress": progress, "ffmpeg_log": log},
                                    ensure_ascii=False).replace(str(base), "<ROOT>"))
        rec["ffmpeg_log"] = "\n".join(
            line if not line.startswith("ARGS:") else " ".join(
                "[<LIST>]" if "capdraft_concat_" in tok else tok for tok in line.split(" "))
            for line in rec["ffmpeg_log"].split("\n"))
        rows.append({"fn": "merge_videos", "args": [kind, inputs, output, reencode, wrapper_script(kind)], "ok": rec})
write("video_merge", rows)
