"""Record wave-3 units (draft patcher, exporter, config, cache, voice catalog) into rust/golden/.

Run from the repo root:  python rust/tools/record_wave3.py
uuid4, time.time and datetime.now are deterministic (ADR-012); file-system cases are
fixture specs ({"files": {relpath: text}}) that the Rust replay recreates.
"""

from __future__ import annotations

import copy
import datetime as dt
import json
import logging
import random
import sys
import tempfile
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from draftgen import Gen  # noqa: E402
from core import config as config_mod  # noqa: E402
from core import cache as cache_mod  # noqa: E402
from core.capcut_project import project_exporter as pe  # noqa: E402
from core.capcut_project.draft_patcher import DraftPatcher  # noqa: E402
from core.capcut_project.draft_reader import DraftReader  # noqa: E402
from core.capcut_project.models import CaptionTtsResult, ToneModifyMode  # noqa: E402
from core.capcut_project.native_audio_alignment import NativeAudioAlignmentSettings  # noqa: E402
from core.capcut_project.voice_catalog import VoiceCatalog  # noqa: E402

logging.disable(logging.CRITICAL)
OUT = ROOT / "rust" / "golden"
rng = random.Random(20261122)
CLOCK = 1_700_000_000.25
_counter = [0]


def _fake_uuid4():
    _counter[0] += 1
    return uuid.UUID(int=_counter[0])


class _FixedDatetime(dt.datetime):
    @classmethod
    def now(cls, tz=None):
        return dt.datetime(2023, 11, 14, 22, 13, 20)


uuid.uuid4 = _fake_uuid4
time.time = lambda: CLOCK
pe.datetime = _FixedDatetime


def call(fn, *args, **kwargs):
    _counter[0] = 0
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


def make_fixture(spec: dict, base: Path) -> None:
    for rel, content in spec.get("files", {}).items():
        p = base / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(content.replace("<ROOT>", str(base)), encoding="utf-8", newline="")
    for rel in spec.get("dirs", []):
        (base / rel).mkdir(parents=True, exist_ok=True)


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
        elif p.is_dir() and not any(p.iterdir()):
            out[p.relative_to(base).as_posix() + "/"] = "<dir>"
    return out


def norm(obj, base: Path):
    return json.loads(json.dumps(obj, ensure_ascii=False, default=str).replace(str(base), "<ROOT>"))


# ======================================================================== draft patcher
def add_existing_tts(g: Gen, d: dict) -> None:
    """Some captions already carry TTS, as CapCut leaves it (shared extras, empty tracks)."""
    mats = d["materials"]
    texts = [t for t in mats.get("texts", []) if isinstance(t, dict)]
    if not texts:
        return
    track = {"id": g.uid(), "type": "audio", "segments": []}
    shared_speed = {"id": "SHARED-SPEED", "type": "speed", "speed": 1.0}
    mats.setdefault("speeds", []).append(shared_speed)
    for t in rng.sample(texts, k=max(1, len(texts) // 2)):
        aid, sid, spid = g.uid(), g.uid(), g.uid()
        mats.setdefault("audios", []).append({"id": aid, "type": "text_to_audio", "text_id": t["id"], "duration": 900_000,
                                              "resource_id": rng.choice(["7102355709945188865", "OTHER"]),
                                              "tone_speaker": "BV074_streaming", "mock_tone_speaker": "BV074_streaming",
                                              "copyright_limit_type": rng.choice(["none", "limited"])})
        mats["speeds"].append({"id": spid, "type": "speed", "speed": 1.0})
        refs = [spid] + (["SHARED-SPEED"] if rng.random() < 0.3 else [])
        track["segments"].append({"id": sid, "material_id": aid, "extra_material_refs": refs,
                                  "target_timerange": {"start": 0, "duration": 900_000}})
        t["text_to_audio_ids"] = [sid]
    d["tracks"].append(track)
    if rng.random() < 0.3:
        d["tracks"].append({"id": g.uid(), "type": "audio", "segments": []})
    if rng.random() < 0.3:
        # an unrelated audio track that stays (music)
        mats.setdefault("audios", []).append({"id": "MUSIC", "type": "music", "duration": 5_000_000})
        d["tracks"].append({"id": g.uid(), "type": "audio", "segments": [
            {"id": "MUSIC-SEG", "material_id": "MUSIC", "extra_material_refs": ["SHARED-SPEED"],
             "target_timerange": {"start": 0, "duration": 5_000_000}}]})


def caption_dict(c):
    return dict(c.__dict__)


rows = []
for i in range(160):
    g = Gen(rng, mutate=0.0)
    d = g.draft()
    if rng.random() < 0.5:
        add_existing_tts(g, d)
    d = json.loads(json.dumps(d))
    reader = DraftReader()
    reader.draft = d
    reader.draft_path = Path("/p/Proj/draft_content.json")
    reader.project_directory = Path("/p/Proj")
    captions = reader.extract_captions()
    templates = reader.get_tts_templates()
    results = []
    rel_paths = {}
    for c in captions:
        if rng.random() < 0.15:
            continue
        status = rng.choice(["generated"] * 5 + ["cached"] * 2 + ["failed", "skipped"])
        src = rng.choice([0, -5, 1, 999, 50_000, 120_000, 1_234_567, rng.randint(200_000, 4_000_000)])
        results.append(CaptionTtsResult(caption_index=c.index, text_material_id=c.text_material_id,
                                        text_segment_id=c.text_segment_id, start_us=c.start_us, audio_path="a.mp3",
                                        source_duration_us=src, target_duration_us=0, status=status))
        if rng.random() < 0.9:
            rel_paths[c.index] = f"C:/proj/textReading/tts_{c.index:06d}.mp3"
    if results and rng.random() < 0.1:
        results.append(CaptionTtsResult(caption_index=9999, text_material_id="x", text_segment_id="y", start_us=0,
                                        audio_path="", source_duration_us=1, target_duration_us=0, status="generated"))
    settings = {
        "voice_type": "BV421_vivn_streaming", "resource_id": "7252594014782755330",
        "voice_display_name": rng.choice(["Nhỏ Ngọt Ngào", "", "A very long display name for a voice"]),
        "capcut_clip_speed": rng.choice([1.0, 1.25, 1.3, 0.0, -1.0, 2.0]),
        "tone_modify_mode": rng.choice(["follow_speed", "preserve_pitch"]),
        "existing_tts_mode": rng.choice(["replace_existing", "skip_existing", "selected_only"]),
        "fps": rng.choice([30.0, 25.0, 0.0, 60.0, 29.97]),
        "alignment": {"enabled": rng.random() < 0.8, "leading_trim_frames": rng.choice([3.0, 0.0, 1.5]),
                      "fade_in_ms": rng.choice([8.0, 0.0, 30.0]), "minimum_source_duration_us": 50_000},
    }
    args = {"draft": copy.deepcopy(d), "captions": [caption_dict(c) for c in captions], "templates": templates,
            "results": [dict(r.__dict__) for r in results], "audio_rel_paths": {str(k): v for k, v in rel_paths.items()},
            "settings": settings}

    def run(d=d, captions=captions, templates=templates, results=results, rel_paths=rel_paths, st=settings):
        patcher = DraftPatcher(templates, fps=st["fps"], alignment=NativeAudioAlignmentSettings(**st["alignment"]))
        patched, stats = patcher.patch(
            d, captions=captions, results=results, voice_type=st["voice_type"], resource_id=st["resource_id"],
            voice_display_name=st["voice_display_name"], capcut_clip_speed=st["capcut_clip_speed"],
            tone_modify_mode=ToneModifyMode(st["tone_modify_mode"]), existing_tts_mode=st["existing_tts_mode"],
            audio_rel_paths=rel_paths, project_directory=Path("/p/Proj"))
        return {"draft": patched, "stats": dict(stats.__dict__), "results": [dict(r.__dict__) for r in results],
                "captions": [caption_dict(c) for c in captions]}
    rows.append({"fn": "patch", "args": [args], **call(run)})
# Hand-written: back-to-back captions (a lane frees exactly at the next start), names over
# 40 characters, a custom template (scene, string speed mode, missing segment keys).
def handmade(n_caps: int, gap: int, text_len: int, custom: bool) -> dict:
    texts, segs = [], []
    t = 0
    for k in range(n_caps):
        mid, sid = f"T{k:03d}", f"S{k:03d}"
        txt = ("Tiếng Việt 😀 " * 10)[: text_len + k]
        texts.append({"id": mid, "type": "subtitle", "content": json.dumps({"text": txt}, ensure_ascii=False)})
        segs.append({"id": sid, "material_id": mid, "target_timerange": {"start": t, "duration": 1_000_000}})
        t += 1_000_000 + gap
    return {"id": "D", "duration": t, "materials": {"texts": texts}, "tracks": [{"id": "TT", "type": "text", "segments": segs}]}


for n_caps, gap, text_len, custom in [(6, 0, 45, True), (8, 0, 39, False), (5, -200_000, 60, True), (7, 1, 41, False)]:
    d = handmade(n_caps, gap, text_len, custom)
    reader = DraftReader()
    reader.draft = d
    reader.draft_path = Path("/p/Proj/draft_content.json")
    reader.project_directory = Path("/p/Proj")
    captions = reader.extract_captions()
    templates = copy.deepcopy(reader.builtin_tts_templates())
    if custom:
        templates["audio_material"]["tts_generate_scene"] = "custom_scene"
        templates["extras"]["speeds"]["mode"] = "2"
        for k in ("hdr_settings", "volume", "track_render_index"):
            templates["audio_segment"].pop(k, None)
    results = [CaptionTtsResult(caption_index=c.index, text_material_id=c.text_material_id,
                                text_segment_id=c.text_segment_id, start_us=c.start_us, audio_path="a.mp3",
                                source_duration_us=1_000_000, target_duration_us=0, status="generated")
               for c in captions]
    rel_paths = {c.index: f"C:/proj/textReading/tts_{c.index:06d}.mp3" for c in captions}
    settings = {"voice_type": "v", "resource_id": "r", "voice_display_name": "", "capcut_clip_speed": 1.0,
                "tone_modify_mode": "preserve_pitch", "existing_tts_mode": "replace_existing", "fps": 30.0,
                "alignment": {"enabled": False, "leading_trim_frames": 3.0, "fade_in_ms": 8.0,
                              "minimum_source_duration_us": 50_000}}
    args = {"draft": copy.deepcopy(d), "captions": [caption_dict(c) for c in captions], "templates": templates,
            "results": [dict(r.__dict__) for r in results], "audio_rel_paths": {str(k): v for k, v in rel_paths.items()},
            "settings": settings}

    def run(d=d, captions=captions, templates=templates, results=results, rel_paths=rel_paths, st=settings):
        patcher = DraftPatcher(templates, fps=st["fps"], alignment=NativeAudioAlignmentSettings(**st["alignment"]))
        patched, stats = patcher.patch(
            d, captions=captions, results=results, voice_type=st["voice_type"], resource_id=st["resource_id"],
            voice_display_name=st["voice_display_name"], capcut_clip_speed=st["capcut_clip_speed"],
            tone_modify_mode=ToneModifyMode(st["tone_modify_mode"]), existing_tts_mode=st["existing_tts_mode"],
            audio_rel_paths=rel_paths, project_directory=Path("/p/Proj"))
        return {"draft": patched, "stats": dict(stats.__dict__), "results": [dict(r.__dict__) for r in results],
                "captions": [caption_dict(c) for c in captions]}
    rows.append({"fn": "patch", "args": [args], **call(run)})
write("patcher", rows)

# ======================================================================== exporter
rows = []
DRAFT_A = json.dumps({"id": "PID", "tracks": [], "materials": {"texts": []}, "name": "A"})
DRAFT_B = {"id": "PID", "tracks": [{"id": "t", "type": "audio", "segments": []}], "materials": {"texts": [], "f": 1.5},
           "name": "B — Tiếng Việt"}
for i in range(24):
    files = {"Proj/draft_content.json": DRAFT_A, "audio/src1.mp3": "ID3-1", "audio/src2.mp3": "ID3-2"}
    if i % 2 == 0:
        files["Proj/Timelines/PID/draft_content.json"] = DRAFT_A
    if i % 3 == 0:
        files["Proj/Timelines/pid/draft_content.json"] = DRAFT_A
    old_backups = rng.choice([0, 3, 12])
    for k in range(old_backups):
        files[f"Proj/backups/2023010{k % 10}_00000{k % 10}{k}/draft_content.json"] = "{}"
    dirs = []
    if i % 8 == 4:
        # a write target that is a directory: backup and commit must fail and roll back
        files.pop("Proj/Timelines/PID/draft_content.json", None)
        dirs.append("Proj/Timelines/PID/draft_content.json")
    spec = {"files": files, "dirs": dirs}
    max_backups = rng.choice([10, 2, 1])
    path_style = rng.choice(["absolute", "placeholder", "relative"])
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp).resolve()
        make_fixture(spec, base)
        proj = base / "Proj"
        draft_path = proj / ("Timelines/PID/draft_content.json" if i % 4 == 2 and i % 2 == 0 else "draft_content.json")
        steps = []
        ex = pe.ProjectExporter(max_backups=max_backups)

        def step(name, fn):
            res = call(fn)
            if "err" in res:
                res["err"] = res["err"].split(":", 1)[0]  # OS error text differs between languages
            steps.append({"step": name, **norm(res, base)})
            return res.get("ok")

        backup = step("create_backup", lambda: str(ex.create_backup(draft_path, proj, {"m": 1, "é": [1.5]},
                                                                    project_id="PID")))
        items = [(base / "audio/src1.mp3", "tts_000001_aaaa.mp3"), (base / "audio/missing.mp3", "tts_000002_bbbb.mp3"),
                 (base / "audio/src2.mp3", "tts_000003_cccc.mp3")]
        placeholder = rng.choice([None, "##_draftpath_placeholder_REC_##"])
        # A missing placeholder draws a new uuid per file: keep one worker so the order is fixed.
        workers = 1 if placeholder is None and path_style == "placeholder" else rng.choice([1, 4])
        step("copy_audio_batch", lambda: ex.copy_audio_batch(items, proj, draft=copy.deepcopy(DRAFT_B),
                                                            placeholder=placeholder, path_style=path_style,
                                                            workers=workers))
        step("atomic_write_draft", lambda: [str(p) for p in ex.atomic_write_draft(draft_path, copy.deepcopy(DRAFT_B),
                                                                                  project_directory=proj,
                                                                                  project_id="PID")])
        step("write_manifest", lambda: str(ex.write_manifest(proj, {"a": [1, {"b": None}], "t": "ố"})))
        step("write_log_file", lambda: str(ex.write_log_file(proj, rng.choice([[], ["line 1", "dòng 2"]]))))
        if backup and i % 3 == 1:
            step("rollback", lambda: ex.rollback(draft_path=draft_path, backup_dir=Path(backup),
                                                 new_audio_paths=[proj / "textReading/tts_000001_aaaa.mp3"],
                                                 project_directory=proj, project_id="PID"))
        rows.append({"fn": "exporter", "args": [spec, str(draft_path.relative_to(base)), max_backups, path_style,
                                                placeholder, workers],
                     "ok": steps, "tree": snapshot(base)})
write("exporter", rows)

# ======================================================================== config
rows = []
CONFIGS = [None, "{}", '{"language": "en", "theme_mode": "DARK"}', "\ufeff{\"default_rate\": 1.5}",
           '{"voice_catalog_update_url": "http://legacy", "voice_catalog_url": "", "voice_catalog_path": "x"}',
           '{"voice_catalog_update_url": "http://legacy"}', '{"ffmpeg_path": "  ", "extra": {"a": 1}}',
           '{"nested": {"x": 1}}', "{bad json", "[1, 2]", '{"cache_path": "/abs/cache", "capcut_projects_path": "<ROOT>"}',
           '{"default_rate": 0, "language": "fr", "max_backups": 3, "tts_chunk_size": 50}',
           '{"default_rate": "2.5", "voice_catalog_url": "  http://x  "}']
for text in CONFIGS:
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp).resolve()
        if text is not None:
            (base / "config.json").write_text(text.replace("<ROOT>", str(base)), encoding="utf-8")
        config_mod.APP_ROOT = base
        config_mod.AppConfig._instance = None

        def load(base=base):
            c = config_mod.AppConfig()
            return {"data": c._data, "cache_path": c.cache_path, "projects_dir": str(c.projects_dir),
                    "capcut_projects_path": str(c.capcut_projects_path) if c.capcut_projects_path else None,
                    "ffmpeg_path": c.ffmpeg_path, "default_rate": c.default_rate,
                    "voice_catalog_url": c.voice_catalog_url, "language": c.language, "theme_mode": c.theme_mode,
                    "device_json_path": c.device_json_path, "capcut_tts_path": c.capcut_tts_path}
        res = norm(call(load), base)
        c = config_mod.AppConfig()
        c.set("language", "ja")
        tree = snapshot(base)
        rows.append({"fn": "config", "args": [text], **res, "tree": tree})
write("config", rows)

# ======================================================================== cache
rows = []
for text in ["Xin chào", "", "a|b", "😀"]:
    for voice in ["BV074_streaming", ""]:
        for rate in [1.0, 0.125, 0.375, 1.005, 2.675, 1.115, 0.5, 1.255, 10.0, -0.005, 1e-9, 123.456]:
            for rid in ["7102355709945188865", ""]:
                h = cache_mod.TtsCache.generate_hash(None, text, voice, rate, rid)
                rows.append({"fn": "generate_hash", "args": [text, voice, rate, rid], "ok": h})
for i in range(6):
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp).resolve()
        (base / "src.mp3").write_bytes(b"ID3" * (i + 1))
        (base / "empty.mp3").write_bytes(b"")
        c = object.__new__(cache_mod.TtsCache)
        c.cache_dir = base / "cache"
        c.cache_dir.mkdir()
        steps = []
        dur = [0.0, 1.5, 2.0, 1e-7, 3.25, 100.0][i]
        steps.append(norm(call(lambda: c.get_cached_file("t", "v", 1.0, "r")), base))
        steps.append(norm(call(lambda: str(c.cache_audio(base / "src.mp3", "t", "v", 1.0, "r", duration=dur))), base))
        steps.append(norm(call(lambda: [str(x) if isinstance(x, Path) else x for x in c.get_cached_file("t", "v", 1.0, "r")]), base))
        res = call(lambda: str(c.cache_audio(base / "empty.mp3", "t2", "v", 1.0, "r")))
        steps.append(norm({"err": res["err"].split(":", 1)[0]} if "err" in res else res, base))
        h = c.generate_hash("t3", "v", 1.0, "r")
        (c.cache_dir / f"{h}.mp3").write_bytes(b"" if i % 2 else b"X")
        (c.cache_dir / f"{h}.meta.json").write_text(["{bad", '{"duration": "2"}', "[]", '{"x": 1}', "", '{"duration": 4}'][i])
        steps.append(norm(call(lambda: [str(x) if isinstance(x, Path) else x for x in c.get_cached_file("t3", "v", 1.0, "r")]), base))
        rows.append({"fn": "cache", "args": [i], "ok": steps, "tree": snapshot(base)})
write("cache", rows)

# ======================================================================== voice catalog
rows = []
real = json.loads((ROOT / "Voice.json").read_text(encoding="utf-8"))
BAD = [5, None, {"voice_type": "x"}, {"resource_id": "1"}, {"voice_type": " a ", "resource_id": " 1 ", "lan": None},
       {"voice_type": "b", "resource_id": 7, "display_name": "", "lang": "en-US"}, "str",
       {"voice_type": 3, "resource_id": 4, "lan": 5, "lang": 6, "display_name": 7}]
DATAS = [real, real[:5] + BAD, BAD, [], {"not": "list"}, "x", [{"voice_type": "", "resource_id": ""}]]
for raw in DATAS:
    cat = VoiceCatalog("u")

    def run(raw=raw, cat=cat):
        voices = cat.load_data(raw, source="test")
        out = {"voices": [dict(v.__dict__) for v in voices], "languages": [list(x) for x in cat.languages()],
               "filters": [], "find": []}
        for lc in [None, "", "vi", "en", "en-US", "zh", "x"]:
            for loc in [None, "vi-VN", "en-US"]:
                for q in [None, "", "  NHỎ ", "bv", "7252", "zzz"]:
                    out["filters"].append([lc, loc, q, [v.voice_type for v in cat.filter(language_code=lc, locale=loc, query=q)]])
        for vt, rid in [("BV421_vivn_streaming", None), ("BV421_vivn_streaming", "7252594014782755330"),
                        ("BV421_vivn_streaming", "x"), ("nope", None), ("b", "7")]:
            f = cat.find(vt, rid)
            out["find"].append(dict(f.__dict__) if f else None)
        return out
    rows.append({"fn": "voice_catalog", "args": [raw], **call(run)})
write("voice_catalog", rows)
