"""Time the whole generate-and-attach pipeline, Python vs Rust, on a real draft.

    python rust/tools/check_pipeline.py <draft_content.json> [work_dir]

Both run against FakeCapCut (instant answers), so the times are the tool's own work:
reading, TTS bookkeeping, audio copies, patching, validation, backup and writes.
Run 1 is sequential and deterministic: the project folders must end up identical.
Run 2 uses the default parallel settings, for timing only.
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
from core.capcut_project.models import ToneModifyMode  # noqa: E402

logging.disable(logging.CRITICAL)
state = {"uuid": 0, "rand": 0, "bytes": 0}
REAL = {"uuid4": uuid.uuid4, "time": time.time, "sleep": time.sleep, "randint": random.randint,
        "token_bytes": secrets.token_bytes, "pe_dt": pe.datetime, "tps_dt": tps.datetime}


class FixedDatetime(dt.datetime):
    @classmethod
    def now(cls, tz=None):
        return dt.datetime(2023, 11, 14, 22, 13, 20)


def deterministic(on: bool):
    state.update(uuid=0, rand=0, bytes=0)
    if on:
        def u():
            state["uuid"] += 1
            return uuid.UUID(int=state["uuid"])

        def r(a, b):
            state["rand"] += 1
            return a + (state["rand"] * 7) % (b - a + 1)

        def tb(n):
            c = state["bytes"]
            state["bytes"] += n
            return bytes(((c + i) * 73) % 256 for i in range(n))
        uuid.uuid4, time.time, random.randint, secrets.token_bytes = u, (lambda: 1_700_000_000.25), r, tb
        pe.datetime = tps.datetime = FixedDatetime
    else:
        uuid.uuid4, time.time, random.randint, secrets.token_bytes = (REAL["uuid4"], REAL["time"], REAL["randint"],
                                                                       REAL["token_bytes"])
        pe.datetime, tps.datetime = REAL["pe_dt"], REAL["tps_dt"]
    time.sleep = lambda s: None


def make_root(base: Path, draft_text: str, parallel: bool) -> Path:
    draft = json.loads(draft_text)
    proj = base / "Proj"
    (proj / "Timelines" / str(draft["id"])).mkdir(parents=True)
    (proj / "draft_content.json").write_text(draft_text, encoding="utf-8", newline="")
    (proj / "Timelines" / str(draft["id"]) / "draft_content.json").write_text(draft_text, encoding="utf-8", newline="")
    (proj / "draft_meta_info.json").write_text('{"draft_id":"META","draft_name":"Proj"}', encoding="utf-8")
    cfg = {"tts_chunk_size": 25, "tts_parallel_chunks": 4 if parallel else 1, "tts_download_workers": 8 if parallel else 1,
           "tts_poll_interval_sec": 1.0, "capcut_tts_path": str(ROOT / "external" / "capcut-tts-api"),
           "device_json_path": str(ROOT / "external" / "capcut-tts-api" / "device.json"),
           "tts_device_profile": "legacy"}
    (base / "config.json").write_text(json.dumps(cfg), encoding="utf-8")
    return base


def python_run(root: Path, det: bool) -> float:
    deterministic(det)
    fake = FakeCapCut()
    requests.post, requests.get = fake.post, fake.get
    config_mod.APP_ROOT = root
    config_mod.AppConfig._instance = None
    t = time.perf_counter()
    svc = tps.CapCutProjectTtsService()
    svc.load_project(root / "Proj")
    ids = [c.text_segment_id for c in svc.get_captions()]
    deterministic(det)
    res = svc.generate_and_attach(ids, "BV421_vivn_streaming", "7252594014782755330", "Nhỏ Ngọt Ngào", 1.0, 1.3,
                                  ToneModifyMode.PRESERVE_PITCH, existing_tts_mode="replace_existing")
    took = time.perf_counter() - t
    print(f"  python: {took:.3f}s success={res.success} attached={res.attached} failed={res.failed}")
    return took


def rust_run(root: Path, det: bool) -> float:
    exe = ROOT / "rust" / "target" / "release" / "examples" / "pipeline_bench"
    out = subprocess.run([str(exe), str(root), "1" if det else "0"], capture_output=True, text=True, check=True).stdout.strip().splitlines()[-1]
    print(f"  rust:   {out.strip().split(' ', 1)[0]}s {out.strip().split(' ', 1)[1]}")
    return float(out.split()[0])


def tree(base: Path) -> dict:
    return {p.relative_to(base).as_posix(): p.read_bytes().replace(str(base).encode(), b"<ROOT>")
            for p in sorted(base.rglob("*")) if p.is_file() and p.name != "config.json"}


def main() -> int:
    draft_text = Path(sys.argv[1]).read_bytes().decode("utf-8").lstrip("﻿")
    work = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(tempfile.mkdtemp(prefix="capdraft-pipe-"))
    subprocess.run(["cargo", "build", "--release", "--example", "pipeline_bench", "-q"], cwd=ROOT / "rust", check=True)
    shutil.rmtree(work, ignore_errors=True)
    print("run 1: sequential, deterministic")
    py_root = make_root(work / "py1", draft_text, parallel=False)
    rs_root = make_root(work / "rs1", draft_text, parallel=False)
    tp1, tr1 = python_run(py_root, True), rust_run(rs_root, True)
    a, b = tree(py_root), tree(rs_root)
    same = a == b
    print(f"  project folders identical: {same} ({len(a)} files, {sum(len(v) for v in a.values()):,} bytes)")
    if not same:
        for k in sorted(set(a) | set(b)):
            if a.get(k) != b.get(k):
                print("   differs:", k)
    print("run 2: default parallel settings")
    tp2 = python_run(make_root(work / "py2", draft_text, parallel=True), False)
    tr2 = rust_run(make_root(work / "rs2", draft_text, parallel=True), False)
    print(f"\nsequential: {tp1 / tr1:.1f}x faster   parallel: {tp2 / tr2:.1f}x faster")
    return 0 if same else 1


if __name__ == "__main__":
    raise SystemExit(main())
