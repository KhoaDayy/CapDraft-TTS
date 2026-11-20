"""Record wave-4 units (CapCut TTS client requests, TTS batch wrapper) into rust/golden/.

Run from the repo root:  python rust/tools/record_wave4.py
No network: requests.post/get are served by FakeCapCut (below), which the Rust replay
implements the same way. uuid4, time.time, time.sleep, random.randint and
secrets.token_bytes are deterministic (ADR-012).
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import logging
import random
import re
import secrets
import sys
import tempfile
import time
import uuid
from pathlib import Path

import requests

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from core import cache as cache_mod  # noqa: E402
from core import config as config_mod  # noqa: E402
from core import capcut_tts as tts_mod  # noqa: E402

logging.disable(logging.CRITICAL)
OUT = ROOT / "rust" / "golden"
CLOCK = 1_700_000_000.25
CLIENT_PY = ROOT / "external" / "capcut-tts-api" / "capcut_common_task_client.py"
spec = importlib.util.spec_from_file_location("capcut_common_task_client", str(CLIENT_PY))
client = importlib.util.module_from_spec(spec)
spec.loader.exec_module(client)

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


uuid.uuid4 = fake_uuid4
time.time = lambda: CLOCK
SLEEPS: list = []
time.sleep = lambda s: SLEEPS.append(round(s, 9))
random.randint = fake_randint
secrets.token_bytes = fake_token_bytes


def call(fn, *args, **kwargs):
    reset()
    try:
        return {"ok": fn(*args, **kwargs)}
    except Exception as exc:  # noqa: BLE001
        return {"err": f"{type(exc).__name__}: {exc}"}


def write(name: str, rows: list[dict]) -> None:
    path = OUT / f"{name}.jsonl"
    with path.open("w", encoding="utf-8", newline="\n") as f:
        for row in rows:
            f.write(json.dumps(row, ensure_ascii=False, allow_nan=False) + "\n")
    errs = sum(1 for r in rows if "err" in r)
    print(f"{path.relative_to(ROOT)}: {len(rows)} cases ({errs} raise)")


def snapshot(base: Path) -> dict:
    out = {}
    for p in sorted(base.rglob("*")):
        if p.is_file():
            data = p.read_bytes()
            try:
                text = data.decode("utf-8")
            except UnicodeDecodeError:
                text = "hex:" + data.hex()
            out[p.relative_to(base).as_posix()] = text.replace(str(base), "<ROOT>")
    return out


class Args:
    def __init__(self, **kw):
        self.text_file = None
        self.bind_id = ""
        self.__dict__.update(kw)


# ======================================================================== client
rows = []
for s in ["plain", "a & b", "<tag attr=\"x\">", "it's", "&amp;", "", "😀 Tiếng Việt", "\n\t", "<>&\"'"]:
    rows.append({"fn": "escape_xml", "args": [s], "ok": client.escape_xml(s)})
n, e = client.rsa_public_numbers_from_pem(client.TTS_SIGN_PUBLIC_KEY_PEM)
rows.append({"fn": "rsa_public_numbers", "args": [], "ok": [format(n, "x"), e]})
for msg in ["", "a", "appid:359289&did:1&creditDisable:false&ssml:abc", "é" * 50, "x" * 245, "x" * 246]:
    rows.append({"fn": "rsa_encrypt_pkcs1v15", "args": [msg], **call(client.rsa_encrypt_pkcs1v15, msg)})
for url in [client.BASE + "/lv/v1/common_task/new?x=1", client.BASE + "/lv/v1/common_task/query", "short", "abc?def"]:
    rows.append({"fn": "make_sign_header", "args": [url, "8.7.0", "1700000000", "123"],
                 "ok": client.make_sign_header(url, "8.7.0", "1700000000", "123")})

DEVICES = [None, {}, {"device_id": "1234567890123456789", "iid": "42", "tdid": "1234567890123456789", "loc": "US",
                      "lan": "en-US", "region": "US", "appvr": "9.0.0"},
           {"device_id": 77, "aid": 1, "loc": "Vn"}, {"extra": "kept", "app_name": "Cap Cut & co"},
           {"app_name": "Cap~Cut/é+%", "os_version": "a b"}]
TEXTS = [["Xin chào"], ["Hello", "world"], ["a & b <c>", "it's \"quoted\"", "😀 emoji", "dòng 1\ndòng 2"],
         ["x" * 300], ["Một", "Hai", "Ba", "Bốn", "Năm"]]
for di, dev in enumerate(DEVICES):
    with tempfile.TemporaryDirectory() as tmp:
        dpath = None
        if dev is not None:
            dpath = str(Path(tmp) / "device.json")
            Path(dpath).write_text(json.dumps(dev), encoding="utf-8")
        for ti, texts in enumerate(TEXTS):
            for voice, rid, rate in [("BV074_streaming", "7102355709945188865", "1.0"), ("vi_female", "99", "1.25"),
                                     ("v\"q", "r&1", "0.5")]:
                args = Args(mode="tts-new", device_json=dpath, text=list(texts), voice=voice, resource_id=rid, rate=rate)
                res = call(lambda a=args: list(client.build_request(a)))
                if "ok" in res:
                    url, headers, body = res["ok"]
                    res = {"ok": {"url": url, "headers": [[k, v] for k, v in headers.items()], "body": body}}
                rows.append({"fn": "build_request", "args": [dev, "tts-new", texts, voice, rid, rate], **res})
        for tid, tok, bind in [("T1", "K1", ""), ("7", "tok en/+", "B")]:
            args = Args(mode="tts-query", device_json=dpath, task_id=tid, token=tok, bind_id=bind)
            res = call(lambda a=args: list(client.build_request(a)))
            if "ok" in res:
                url, headers, body = res["ok"]
                res = {"ok": {"url": url, "headers": [[k, v] for k, v in headers.items()], "body": body}}
            rows.append({"fn": "build_request", "args": [dev, "tts-query", tid, tok, bind], **res})
write("capcut_client", rows)


from fakecapcut import FakeCapCut  # noqa: E402


# ======================================================================== wrapper
rows = []
rng = random.Random(20261123)
SCENARIOS = [
    # (texts, chunk_size, parallel, workers, use_cache, cancel_after, pre_cache, project)
    (["Xin chào", "Hello"], 25, 1, 1, True, None, [], ""),
    ([f"Câu {i}" for i in range(7)], 3, 1, 1, True, None, [], ""),
    (["ok", "#DLFAIL bad", "#EMPTY", "#NOURL x", "fine", "#DL500 x"], 10, 1, 1, True, None, [], ""),
    (["a", "b #SHORT", "c"], 10, 1, 1, True, None, [], ""),
    (["#HTTP500"], 5, 1, 1, True, None, [], ""),
    (["x", "#RET1"], 5, 1, 1, True, None, [], ""),
    (["#NOTASK"], 5, 1, 1, True, None, [], ""),
    (["#FAILTASK"], 5, 1, 1, True, None, [], ""),
    (["#QUERY500 then ok", "two"], 5, 1, 1, True, None, [], ""),
    (["a", "b", "c #QRET1"], 5, 1, 1, True, None, [], ""),
    (["cached one", "fresh", "", "   ", "cached two"], 5, 1, 1, True, None, ["cached one", "cached two"], ""),
    (["cached one", "fresh"], 5, 1, 1, False, None, ["cached one"], ""),
    ([f"t{i}" for i in range(6)], 2, 1, 1, True, 1, [], ""),
    ([f"t{i}" for i in range(6)], 2, 1, 1, True, 4, [], ""),
    ([f"t{i}" for i in range(6)], 2, 1, 1, True, 9, [], ""),
    ([f"t{i}" for i in range(6)], 2, 1, 1, True, 15, [], ""),
    ([f"u{i}" for i in range(4)], 10, 1, 1, True, 12, [], ""),
    ([f"u{i}" for i in range(4)], 10, 1, 1, True, 16, [], ""),
    ([f"u{i}" for i in range(4)], 10, 1, 1, True, 17, [], ""),
    (["proj a", "proj b"], 5, 1, 1, True, None, ["proj a"], "MyProj"),
    ([f"p{i}" for i in range(20)], 3, 4, 4, True, None, [], ""),
    ([f"q{i}" for i in range(9)] + ["#DLFAIL q", "#HTTP500"], 2, 3, 2, True, None, ["q1"], ""),
]
for si, (texts, chunk, par, workers, use_cache, cancel_after, pre, project) in enumerate(SCENARIOS):
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp).resolve()
        cfg = {"tts_chunk_size": chunk, "tts_parallel_chunks": par, "tts_download_workers": workers,
               "tts_poll_interval_sec": 1.0, "capcut_tts_path": str(CLIENT_PY.parent),
               "device_json_path": str(ROOT / "external" / "capcut-tts-api" / "device.json")}
        (base / "config.json").write_text(json.dumps(cfg), encoding="utf-8")
        config_mod.APP_ROOT = base
        config_mod.AppConfig._instance = None
        fake = FakeCapCut()
        requests.post = fake.post
        requests.get = fake.get
        client.requests = requests
        reset()
        w = tts_mod.CapCutTtsWrapper(project_name=project)
        for t in pre:
            src = base / "pre.mp3"
            src.write_bytes(f"CACHED:{t}".encode())
            w.cache.cache_audio(src, t, "BV074_streaming", 1.0, "7102355709945188865", duration=2.5)
            src.unlink()
        events = []
        checks = [0]
        SLEEPS.clear()

        def cancelled(limit=cancel_after):
            checks[0] += 1
            return limit is not None and checks[0] > limit
        items = [{"index": i + 1, "text": t} for i, t in enumerate(texts)]
        reset()
        res = call(lambda: w.generate_tts_batch(items, voice="BV074_streaming", resource_id="7102355709945188865",
                                                rate=1.0, is_cancelled_callback=cancelled,
                                                item_completed_callback=lambda i, r: events.append([i, dict(r)]),
                                                use_cache=use_cache))
        sequential = par == 1 or (len([t for t in texts if t.strip()]) + chunk - 1) // chunk <= 1
        out = {"results": [[k, v] for k, v in res["ok"].items()] if "ok" in res else res,
               "events": events if sequential else sorted(events, key=lambda e: e[0]),
               "cancel_checks": checks[0] if sequential else None,
               "requests": fake.log if sequential else None,
               "sleeps": list(SLEEPS) if sequential else None}
        if not sequential:
            out["results"] = sorted(out["results"], key=lambda e: e[0])
        out = json.loads(json.dumps(out, ensure_ascii=False).replace(str(base), "<ROOT>"))
        tree = {k: v for k, v in snapshot(base).items() if k != "config.json"}
        rows.append({"fn": "tts_batch", "args": [texts, chunk, par, workers, use_cache, cancel_after, pre, project],
                     "ok": out, "tree": tree})
write("tts_batch", rows)
