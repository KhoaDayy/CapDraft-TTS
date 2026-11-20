"""Record wave-2 units (alignment, draft reader, validator, split) into rust/golden/.

Run from the repo root:  python rust/tools/record_wave2.py
uuid4 and time.time are made deterministic (ADR-012); file-system cases are described
as a fixture spec ({"files": {relpath: text}}) that the Rust replay recreates.
"""

from __future__ import annotations

import copy
import json
import logging
import random
import shutil
import sys
import tempfile
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from draftgen import Gen  # noqa: E402
from core.capcut_project import native_audio_alignment as naa  # noqa: E402
from core.capcut_project import project_split as ps  # noqa: E402
from core.capcut_project.draft_reader import DraftReader  # noqa: E402
from core.capcut_project.validator import DraftValidator  # noqa: E402

logging.disable(logging.CRITICAL)
OUT = ROOT / "rust" / "golden"
rng = random.Random(20261121)

_counter = [0]


def _fake_uuid4():
    _counter[0] += 1
    return uuid.UUID(int=_counter[0])


uuid.uuid4 = _fake_uuid4
time.time = lambda: 1_700_000_000.25


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


def drafts(n: int):
    """Half clean, half with mutated values."""
    for i in range(n):
        yield Gen(rng, mutate=0.0 if i % 2 == 0 else rng.choice([0.02, 0.05, 0.1])).draft()


def make_fixture(spec: dict, base: Path) -> None:
    for rel, content in spec["files"].items():
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
    return out


def norm(obj, base: Path):
    return json.loads(json.dumps(obj, ensure_ascii=False, default=str).replace(str(base), "<ROOT>"))


# ======================================================================== alignment
rows = []
for frames in [0, 1, 3, 3.0, 2.5, 0.5, 10, 1e-9]:
    for fps in [30.0, 25, 24.0, 29.97, 60, 0, -1, None, 23.976]:
        rows.append({"fn": "frames_to_us", "args": [frames, fps], **call(naa.frames_to_us, frames, fps)})
for ms in [0, 8, 8.0, 0.0005, 0.0015, 2.5, 1.5, 100, -3]:
    rows.append({"fn": "ms_to_us", "args": [ms], **call(naa.ms_to_us, ms)})
for _ in range(600):
    seg = {"id": "SEG", "speed": 1.0, "extra_material_refs": rng.choice([[], ["SPD"], None, ["SPD", "X"]])}
    if rng.random() < 0.3:
        del seg["extra_material_refs"]
    mat = {"id": "MAT", "type": "text_to_audio", "duration": 0}
    fades = rng.choice([[], [{"id": "OLD"}]])
    st = {"enabled": rng.random() < 0.85, "leading_trim_frames": rng.choice([3.0, 0.0, 1.0, 10.0, 2.5, -1.0]),
          "fade_in_ms": rng.choice([8.0, 0.0, 30.0, 1000.0, 0.0015]),
          "minimum_source_duration_us": rng.choice([50_000, 0, 1, 500_000])}
    kw = {"caption_start_us": rng.choice([0, 1_000_000, 123_457]),
          "raw_duration_us": rng.choice([0, 1, 999, 50_000, 100_000, 120_000, 1_234_567, rng.randint(0, 10**7)]),
          "project_fps": rng.choice([30.0, 25.0, 24.0, 60.0, 29.97, 0, None, -5.0]),
          "clip_speed": rng.choice([1.0, 1.25, 1.3, 2.0, 0.5, 0, None, -1.0])}
    args = {"segment": copy.deepcopy(seg), "material": copy.deepcopy(mat), "fades": copy.deepcopy(fades),
            "settings": st, **kw}

    def run(seg=seg, mat=mat, fades=fades, st=st, kw=kw):
        res = naa.apply_native_audio_alignment(audio_segment=seg, audio_material=mat, audio_fade_materials=fades,
                                               settings=naa.NativeAudioAlignmentSettings(**st), **kw)
        return {"segment": seg, "material": mat, "fades": fades, "result": res.__dict__}
    rows.append({"fn": "apply_native_audio_alignment", "args": [args], **call(run)})
write("alignment", rows)

# ======================================================================== draft reader
rows = []
for content in [None, "", "  ", "plain", '{"text": "hi"}', '{"text": null}', '{"text": 5}', '{"x": 1}',
                "[1, 2]", '"str"', "5", "{bad", {"text": "d"}, {"text": None}, {}, 5, 1.5, True, [], ["a"],
                '{"text": ["a"]}', "null", "true", "1e400"]:
    m = {"content": content}
    rows.append({"fn": "parse_caption_text", "args": [m], **call(lambda m=m: list(DraftReader._parse_caption_text(m)))})


def reader_for(draft: dict, draft_path: str) -> DraftReader:
    r = DraftReader()
    r.draft = draft
    r.draft_path = Path(draft_path)
    r.project_directory = r.draft_path.parent
    if r.draft_path.parent.parent.name == "Timelines":
        r.project_directory = r.draft_path.parent.parent.parent
    return r


def caption_dict(c):
    return dict(c.__dict__)


def info_dict(i):
    d = dict(i.__dict__)
    d["draft_path"] = str(d["draft_path"])
    d["project_directory"] = str(d["project_directory"])
    d["duration_display"] = i.duration_display
    d["resolution"] = i.resolution
    return d


DRAFT_PATHS = ["/p/My Proj/draft_content.json", "/p/Proj2/Timelines/ABC/draft_content.json", "/draft_content.json"]
for d in drafts(300):
    dp = rng.choice(DRAFT_PATHS)
    sel = rng.choice([None, [], ["1", "2"], ["x"]])

    def inspect(d=d, dp=dp, sel=sel):
        r = reader_for(d, dp)
        if sel and sel != ["x"]:
            caps = r.get_captions()
            sel = [c.text_segment_id for c in caps[: len(caps) // 2]] + ["3"]
        res = r.inspect_project(sel)
        out = dict(res.__dict__)
        out["project_info"] = info_dict(res.project_info)
        return out
    rows.append({"fn": "reader", "args": [d, dp, sel], "ok": {
        "extract_captions": call(lambda d=d: [caption_dict(c) for c in reader_for(d, dp).extract_captions()]),
        "project_info": call(lambda d=d, dp=dp: info_dict(reader_for(d, dp).get_project_info())),
        "find_tts_templates": call(lambda d=d: reader_for(d, dp).find_tts_templates()),
        "find_speed_templates": call(lambda d=d: reader_for(d, dp).find_speed_templates()),
        "inspect_project": call(inspect),
    }})
rows.append({"fn": "builtin_tts_templates", "args": [], **call(lambda: DraftReader().builtin_tts_templates())})
write("reader", rows)

# load_draft on fixture folders
rows = []
good = json.dumps({"tracks": [], "materials": {"texts": []}, "name": "x"})
LOAD_SPECS = [
    ({"files": {"P/draft_content.json": good}}, "P"),
    ({"files": {"P/draft_content.json": good}}, "P/draft_content.json"),
    ({"files": {"P/Timelines/ID1/draft_content.json": good}}, "P"),
    ({"files": {"P/Timelines/ID1/draft_content.json": good}}, "P/Timelines/ID1/draft_content.json"),
    ({"files": {"P/other.txt": "x"}}, "P"),
    ({"files": {"P/other.txt": "x"}}, "P/missing.json"),
    ({"files": {"P/draft_content.json": "﻿" + good}}, "P"),
    ({"files": {"P/draft_content.json": "[1]"}}, "P"),
    ({"files": {"P/draft_content.json": '{"tracks": []}'}}, "P"),
    ({"files": {"P/draft_content.json": '{"tracks": [], "materials": {}}'}}, "P"),
    ({"files": {"P/draft_content.json": '{"tracks": [], "materials": null}'}}, "P"),
    ({"files": {"P/draft_content.json": "{bad"}}, "P"),
]
for spec, target in LOAD_SPECS:
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)
        make_fixture(spec, base)

        def load(base=base, target=target):
            r = DraftReader()
            r.load_draft(base / target)
            return {"draft_path": str(r.draft_path), "project_directory": str(r.project_directory),
                    "captions": len(r.get_captions())}
        res = call(load)
        if "err" in res and res["err"].startswith("JSONDecodeError"):
            res["err"] = "JSONDecodeError"  # serde_json words parse errors differently
        rows.append({"fn": "load_draft", "args": [spec, target], **norm(res, base)})
write("reader_load", rows)

# ======================================================================== validator
rows = []
for d in drafts(400):
    fs = None
    if rng.random() < 0.4:
        files = {}
        mats = d.get("materials") if isinstance(d.get("materials"), dict) else {}
        auds = mats.get("audios") if isinstance(mats.get("audios"), list) else []
        for a in auds:
            if not isinstance(a, dict) or not isinstance(a.get("path"), str):
                continue
            name = a["path"].rsplit("/", 1)[-1]
            choice = rng.random()
            if choice < 0.5 and name:
                files[f"textReading/{name}"] = "RIFF"
            elif choice < 0.6 and name:
                files[f"textReading/{name}"] = ""
            elif choice < 0.7 and name:
                files[name] = "RIFF"
        fs = {"files": files}
    req = rng.random() < 0.8
    if fs is None:
        res = call(lambda d=d, req=req: DraftValidator().validate(d, project_directory=None, require_audio_files=req))
    else:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            make_fixture(fs, base)
            res = norm(call(lambda d=d, req=req, base=base: DraftValidator().validate(
                d, project_directory=base, require_audio_files=req)), base)
    rows.append({"fn": "validate", "args": [d, fs, req], **res})
# Hand-written boundary drafts the generator rarely reaches.
def tts_draft(src_dur=500_000, tgt_dur=500_000, fade=None, mat_dur=600_000, src_start=100_000, ids=None):
    ids = ids or {}
    seg = {"id": ids.get("seg", "S1"), "material_id": "A1",
           "source_timerange": {"start": src_start, "duration": src_dur},
           "target_timerange": {"start": 0, "duration": tgt_dur}, "speed": 1.0, "is_tone_modify": False,
           "extra_material_refs": ["SP1"] + (["F1"] if fade is not None else [])}
    mats = {"texts": [{"id": "T1", "type": "subtitle", "text_to_audio_ids": ["S1"]}],
            "audios": [{"id": "A1", "type": "text_to_audio", "text_id": "T1", "duration": mat_dur}],
            "speeds": [{"id": "SP1", "type": "speed", "speed": 1.0}],
            "audio_fades": [{"id": "F1", "type": "audio_fade", "fade_in_duration": fade}] if fade is not None else []}
    for extra in ids.get("dups", []):
        mats["speeds"].append({"id": extra, "type": "speed", "speed": 1.0})
    return {"materials": mats, "tracks": [{"id": "TR", "type": "audio", "segments": [seg]}]}


BOUNDARY = [tts_draft(src_dur=d) for d in (999, 1000, 1001)] + [tts_draft(tgt_dur=d) for d in (999, 1000, 1001)]
BOUNDARY += [tts_draft(fade=f, tgt_dur=1000) for f in (999, 1000, 1001, 0, -1)]
BOUNDARY += [tts_draft(mat_dur=m, src_start=100_000, src_dur=500_000) for m in (599_998, 599_999, 600_000, 600_001)]
BOUNDARY += [tts_draft(ids={"dups": ["zz", "zz", "aa", "aa", "mm", "mm"]}),
             tts_draft(ids={"dups": ["b", "b", "B", "B", "é", "é", "_", "_"]})]
for d in BOUNDARY:
    rows.append({"fn": "validate", "args": [d, None, False],
                 **call(lambda d=d: DraftValidator().validate(d, project_directory=None, require_audio_files=False))})
write("validator", rows)

# ======================================================================== split (pure)
rows = []
for dur in [0, 1, 2, 3, 10, 1_000_000, 2_000_000, 10_000_000, 100_000_000, None, -5]:
    for edges in [None, [], [1], [5_000_000], [999_999, 1_000_001], [4_999_999, 5_000_001],
                  [3_000_000, 7_000_000], [5_000_001, 4_999_999], [7_000_000, 3_000_000], [0, 10_000_000, 5_000_000], [5_000_000, 5_000_000], ["12", 3.7]]:
        rows.append({"fn": "choose_cut_us", "args": [dur, edges], **call(ps.choose_cut_us, dur, edges)})
for d in drafts(300):
    total = d.get("duration") if isinstance(d.get("duration"), int) else 10
    slices = []
    for cut in [rng.randint(-2, max(2, total + 2)), max(1, total // 2), rng.choice([0, total, -1])]:
        part = rng.choice([1, 2, 2, 1, 3]) if cut != max(1, total // 2) else rng.choice([1, 2])
        suffix = rng.choice(["", "_part1", " (2)"])
        cid = rng.choice([None, "NEWID"])
        ren = rng.random() < 0.8
        slices.append({"args": [cut, part, suffix, cid, ren],
                       **call(lambda d=d, cut=cut, part=part, suffix=suffix, cid=cid, ren=ren: list(
                           ps.slice_draft(d, cut, part=part, name_suffix=suffix, new_content_id=cid,
                                          rename_content=ren)))})
    rows.append({"fn": "split", "args": [d], "ok": {"caption_edges": call(ps._caption_edges_from_draft, d),
                                                    "slices": slices}})
write("split", rows)

# ======================================================================== split_project end to end
rows = []
for i in range(40):
    d = Gen(rng, mutate=0.0 if i % 4 else 0.02).draft()
    if not isinstance(d.get("duration"), int) or d["duration"] < 4:
        d["duration"] = 10_000_000
    cid = str(d.get("id") or "")
    files = {"root_meta_info.json": json.dumps({
        "all_draft_store": [
            {"draft_fold_path": "<ROOT>/Proj", "draft_name": "Proj", "draft_id": "SRC", "tm_duration": 1,
             "duration": 1, "tm_draft_create": 5},
            {"draft_fold_path": "<ROOT>/Ghost", "draft_name": "Ghost"},
            {"draft_fold_path": "<ROOT>/Other", "draft_name": "Other"},
            {"draft_fold_path": "<ROOT>/Proj_part1", "draft_name": "Proj_part1"},
            "not a dict",
        ],
        "draft_ids": rng.choice([3, ["SRC"], None, 3.0, True]), "root_path": "<ROOT>"}, ensure_ascii=False),
        "Other/draft_content.json": "{}",
        "Proj/draft_content.json": json.dumps(d, ensure_ascii=False),
        "Proj/draft_cover.jpg": "JPEG",
        "Proj/media/a.mp4": "MP4",
        "Proj/backups/old.json": "{}",
        "Proj/x.tmp": "tmp",
        "Proj/y.precommit": "p",
        "Proj/tts_generation_manifest.json": "{}",
        "Proj/subdraft/s1/draft_content.json": '{"keep": 1}',
    }
    r = rng.random()
    if r < 0.8:
        files["Proj/draft_meta_info.json"] = json.dumps(
            {"draft_id": "SRCMETA", "draft_name": "Proj", "duration": 1, "draft_cover": rng.choice(["", "c.jpg"]),
             "draft_fold_path": "x"})
    if rng.random() < 0.7:
        files[f"Proj/Timelines/{cid or 'X'}/draft_content.json"] = json.dumps(d, ensure_ascii=False)
        if rng.random() < 0.5:
            files[f"Proj/Timelines/{cid or 'X'}/template.tmp"] = json.dumps({"canvas_config": {}, "pad": "x" * 60})
        files["Proj/Timelines/project.json"] = json.dumps(
            {"id": "PJ", "main_timeline_id": cid, "timelines": [{"id": cid, "name": "t"}], "n": 1})
        if rng.random() < 0.5:
            files["Proj/Timelines/OTHER/draft_content.json"] = "{}"
        if rng.random() < 0.5:
            files["Proj/timeline_layout.json"] = json.dumps({"dockItems": [{"timelineIds": [cid]}]})
    if rng.random() < 0.2:
        files["Proj_part1/stale.txt"] = "stale"
    spec = {"files": files}
    cut = rng.choice([None, None, d["duration"] // 3, 0])
    with tempfile.TemporaryDirectory() as tmp:
        base = Path(tmp)
        make_fixture(spec, base)

        def run(base=base, d=d, cut=cut):
            res = ps.split_project(source_project_dir=base / "Proj", source_draft=d,
                                   source_draft_path=base / "Proj" / "draft_content.json", cut_us=cut)
            return {"result": {k: (str(v) if isinstance(v, Path) else v) for k, v in res.__dict__.items()}}
        res = call(run)
        res = norm(res, base)
        rows.append({"fn": "split_project", "args": [spec, cut], **res, "tree": snapshot(base)})
write("split_project", rows)
