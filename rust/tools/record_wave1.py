"""Record the original Python behavior for wave-1 units into rust/golden/*.jsonl.

Run from the repo root:  python rust/tools/record_wave1.py
Every line is {"fn": name, "args": [...], "ok": result} or {"fn", "args", "err": "Type: msg"}.
The Rust tests in rust/crates/capdraft-core/tests/golden.rs replay every line.
"""

from __future__ import annotations

import json
import math
import random
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from core.capcut_project import models, srt_exporter  # noqa: E402
from core import i18n, preferences  # noqa: E402

OUT = ROOT / "rust" / "golden"
rng = random.Random(20261120)


def call(fn, *args, **kwargs):
    try:
        return {"ok": fn(*args, **kwargs)}
    except Exception as exc:  # noqa: BLE001
        return {"err": f"{type(exc).__name__}: {exc}"}


def write(name: str, rows: list[dict]) -> None:
    path = OUT / f"{name}.jsonl"
    with path.open("w", encoding="utf-8", newline="\n") as f:
        for row in rows:
            f.write(json.dumps(row, ensure_ascii=False, allow_nan=False) + "\n")
    print(f"{path.relative_to(ROOT)}: {len(rows)} cases")


# ---------------------------------------------------------------- models
INT_TRAPS = [0, 1, -1, 499, 500, 501, 1499, 1500, 1501, 2500, 3500, 999_999, 1_000_000,
             3_599_999_499, 3_599_999_500, 3_600_000_000, 86_399_999_999, -500, -1500,
             123_456_789, 2**40, 2**53 + 1]
FLOAT_TRAPS = [0.0, 0.5, 1.5, 2.5, 0.0000005, 0.0000015, 0.0000025, 1.0, 1.3, 0.75, 2.0,
               -1.0, 1e-9, 123.4567895, 3599.9995, 1e6]
SPEEDS = [0.0, -1.0, 1.0, 1.0 + 1e-10, 1.0 + 1e-8, 0.5, 0.75, 1.25, 1.3, 1.5, 2.0, 3.0, 0.1]
FPS = [30.0, 25.0, 24.0, 29.97, 60.0, 0.0, -1.0, 23.976, 50.0]

rows = []
for v in INT_TRAPS + [rng.randint(0, 10**10) for _ in range(200)]:
    rows.append({"fn": "format_duration_us", "args": [v], **call(models.format_duration_us, v)})
for v in FLOAT_TRAPS + [rng.uniform(0, 10_000) for _ in range(200)]:
    rows.append({"fn": "seconds_to_us", "args": [v], **call(models.seconds_to_us, v)})
for v in INT_TRAPS + [rng.randint(0, 10**10) for _ in range(50)]:
    if abs(v) < 2**53:
        rows.append({"fn": "us_to_seconds", "args": [v], **call(models.us_to_seconds, v)})
src = [0, 1, 999, 1000, 33_333, 500_000, 1_000_000, 1_234_567, 10_000_000, 59_999_999]
for s in src + [rng.randint(0, 30_000_000) for _ in range(60)]:
    for sp in SPEEDS:
        for fps in FPS:
            rows.append({"fn": "compute_target_duration_us", "args": [s, sp, fps],
                         **call(models.compute_target_duration_us, s, sp, fps=fps)})
for m in ["follow_speed", "preserve_pitch", "FOLLOW_SPEED", " Preserve_Pitch ", "", "x",
          "followspeed", "follow speed", "PRESERVE_PITCH\n", " follow_speed"]:
    rows.append({"fn": "map_tone_mode_to_capcut_flag", "args": [m],
                 **call(models.map_tone_mode_to_capcut_flag, m)})
write("models", rows)

# ---------------------------------------------------------------- whitespace traps
# Python str.strip()/isspace() vs Rust char::is_whitespace: record the exact set.
space = [cp for cp in range(0x110000) if not (0xD800 <= cp <= 0xDFFF) and chr(cp).isspace()]
write("isspace", [{"fn": "isspace_set", "args": [], "ok": space}])

# ---------------------------------------------------------------- srt
TEXTS = ["Hello", "  padded  ", "", "   ", "line1\r\nline2", "a\rb", " nbsp ",
         "\x1fctrl\x1c", "Tiếng Việt có dấu", "中文字幕", "emoji 😀👍🏽", "​zero-width",
         "　ideographic　", "tab\tinside", "\n\nlead newlines", "trail\r\n"]


def rand_caption(i: int) -> dict:
    return {
        "index": rng.randint(0, 50),
        "start_us": rng.choice([0, -5, 500, 1500, 2500, rng.randint(-1_000, 10**10)]),
        "duration_us": rng.choice([0, -1, 499, 500, rng.randint(-1_000, 10**8)]),
        "text": rng.choice(TEXTS),
    }


def to_rows(caps: list[dict]) -> list[models.CaptionRow]:
    return [models.CaptionRow(index=c["index"], text_track_id="t", text_segment_id="s",
                              text_material_id="m", start_us=c["start_us"],
                              duration_us=c["duration_us"], text=c["text"]) for c in caps]


rows = []
for _ in range(300):
    caps = [rand_caption(i) for i in range(rng.randint(0, 8))]
    rows.append({"fn": "render_captions_srt", "args": [caps],
                 **call(srt_exporter.render_captions_srt, to_rows(caps))})
    rows.append({"fn": "count_srt_captions", "args": [caps],
                 "ok": sum(1 for c in to_rows(caps) if c.text.strip())})
write("srt", rows)

# ---------------------------------------------------------------- python json
DOCS = [
    {}, [], {"a": []}, {"a": {}}, [1, 2.0, -0.0, 1e16, 1e-5, 1.5e-7, 123456789.123, 1e22, 5e-324,
    1.7976931348623157e308, 0.1, 1 / 3, 100.0, 1e15, 9999999999999998.0, 0.0001, 0.00001],
    {"text": "quote\" back\\ nl\n cr\r tab\t bs\b ff\f ctl\x01\x1f del\x7f", "vi": "Tiếng Việt"},
    {"emoji": "😀", "nested": {"k": [True, False, None, {"z": 1, "a": 2}]}},
    {"big": 2**62, "neg": -(2**62), "u": 2**63 - 1},
]
for _ in range(150):
    def rv(d=0):
        t = rng.randint(0, 7 if d < 3 else 4)
        if t == 0:
            return rng.randint(-10**12, 10**12)
        if t == 1:
            return rng.choice([rng.uniform(-1e6, 1e6), rng.random() * 10 ** rng.randint(-12, 25),
                               float(rng.randint(-10**6, 10**6)), rng.randint(1, 4) / 3])
        if t == 2:
            return "".join(rng.choice("abcxyz é中😀\n\"\\/\t\x00") for _ in range(rng.randint(0, 6)))
        if t == 3:
            return rng.choice([True, False, None])
        if t == 4:
            return rng.random()
        if t in (5, 6):
            return {f"k{rng.randint(0, 99)}": rv(d + 1) for _ in range(rng.randint(0, 4))}
        return [rv(d + 1) for _ in range(rng.randint(0, 4))]
    DOCS.append(rv())

rows = []
for doc in DOCS:
    text = json.dumps(doc, ensure_ascii=False)
    rows.append({"fn": "dumps_compact", "args": [text],
                 "ok": json.dumps(json.loads(text), ensure_ascii=False, separators=(",", ":"))})
    rows.append({"fn": "dumps_indent2", "args": [text],
                 "ok": json.dumps(json.loads(text), ensure_ascii=False, indent=2)})
    rows.append({"fn": "dumps_indent4", "args": [text],
                 "ok": json.dumps(json.loads(text), ensure_ascii=False, indent=4)})
# Input-side traps: number spellings Python normalizes on load.
for text in ['1E5', '1e+2', '-0', '-0.0', '0.10', '1.000', '[1.0e0, 2E-3]', '{"a":1,"a":2,"b":3}',
             '12345678901234567890', '"\\u00e9\\ud83d\\ude00"', '1.5e300', '3.0e-310']:
    rows.append({"fn": "dumps_compact", "args": [text],
                 "ok": json.dumps(json.loads(text), ensure_ascii=False, separators=(",", ":"))})
write("pyjson", rows)

# ---------------------------------------------------------------- i18n / preferences
rows = []
for v in [None, "", "vi", "en", "EN", "en_US", "en-GB", "zh-Hans", "ja_JP", "fr", " ja ", 0, "VI-vn"]:
    rows.append({"fn": "normalize_language", "args": [v], **call(i18n.normalize_language, v)})
for v in [None, "", "auto", "light", "dark", "LIGHT", " Dark ", "system", 1]:
    rows.append({"fn": "normalize_theme_mode", "args": [v],
                 **call(preferences.normalize_theme_mode, v)})
for lang in ["vi", "en", "zh", "ja"]:
    for key in list(i18n._VI) + list(i18n._EN) + ["missing_key"]:
        rows.append({"fn": "translate", "args": [key, lang], **call(i18n.translate, key, language=lang)})
    rows.append({"fn": "translate", "args": ["selection", lang, {"selected": 3, "total": 10, "visible": 7}],
                 **call(i18n.translate, "selection", language=lang, selected=3, total=10, visible=7)})
write("i18n", rows)

# ---------------------------------------------------------------- paths
from core.capcut_project import paths  # noqa: E402

rows = []
PH = "##_draftpath_placeholder_0A1B2C3D-AAAA-bbbb-1234-56789ABCDEF0_##"
for _ in range(200):
    def mat_item():
        it = {}
        for key in rng.sample(["path", "media_path", "reverse_path", "intensifies_path", "other", "id"], 3):
            it[key] = rng.choice(["", "C:/x/y.mp4", f"{PH}/textReading/a.wav", 5, None,
                                  "##_draftpath_placeholder_deadBEEF_##/v.mp4",
                                  "##_draftpath_placeholder_xyz_##/bad", "prefix ##_draftpath_placeholder_12-34_##"])
        return it
    mats = {}
    for b in rng.sample(["videos", "audios", "texts", "speeds", "stickers"], rng.randint(0, 4)):
        mats[b] = rng.choice([[mat_item() for _ in range(rng.randint(0, 3))], {"not": "list"}, ["str", 3]])
    draft = {"materials": rng.choice([mats, mats, None, {}])}
    if rng.random() < 0.3:
        draft["path"] = rng.choice([PH + "/x", "plain", 1])
    rows.append({"fn": "find_draftpath_placeholder", "args": [draft],
                 **call(paths.find_draftpath_placeholder, draft)})
for name in ["a.wav", "dir/a.wav", "/abs/dir/b.mp3", "", "x/", "../up.wav", "tiếng.wav"]:
    rows.append({"fn": "capcut_text_reading_path", "args": [PH, name],
                 **call(paths.capcut_text_reading_path, PH, name)})
write("paths", rows)
