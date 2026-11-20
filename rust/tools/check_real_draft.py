"""Compare the Rust port with the Python original on a real draft_content.json.

    python rust/tools/check_real_draft.py <draft_content.json> [work_dir]

Runs every ported function on the draft in both implementations (Rust through
`cargo run --release --example real_draft`), then compares the outputs byte for
byte and prints the timings side by side. Nothing from the draft is written into
the repository: outputs go to work_dir (default: a temporary folder).
"""

from __future__ import annotations

import json
import logging
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from core.capcut_project import project_split as ps  # noqa: E402
from core.capcut_project.draft_patcher import DraftPatcher  # noqa: E402
from core.capcut_project.draft_reader import DraftReader  # noqa: E402
from core.capcut_project.models import CaptionTtsResult, ToneModifyMode  # noqa: E402
from core.capcut_project.native_audio_alignment import NativeAudioAlignmentSettings  # noqa: E402
from core.capcut_project.srt_exporter import render_captions_srt  # noqa: E402
from core.capcut_project.validator import DraftValidator  # noqa: E402

logging.disable(logging.CRITICAL)
CLOCK = 1_700_000_000.25
META = '{"draft_id":"SRCMETA","draft_name":"Proj","duration":1,"draft_cover":"","tm_duration":1}'
ROOT_META = ('{"all_draft_store":[{"draft_fold_path":"<ROOT>/Proj","draft_name":"Proj","draft_id":"SRCMETA",'
             '"tm_duration":1,"tm_draft_create":5}],"draft_ids":1,"root_path":"<ROOT>"}')
_counter = [0]
_real_uuid4, _real_time = uuid.uuid4, time.time


def deterministic(on: bool) -> None:
    if on:
        _counter[0] = 0

        def fake():
            _counter[0] += 1
            return uuid.UUID(int=_counter[0])
        uuid.uuid4, time.time = fake, (lambda: CLOCK)
    else:
        uuid.uuid4, time.time = _real_uuid4, _real_time


def compact(obj) -> str:
    return json.dumps(obj, ensure_ascii=False, separators=(",", ":"))


def fnv1a(data: bytes) -> str:
    h = 0xCBF29CE484222325
    for b in data:
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}"


def python_side(draft_path: Path, out: Path) -> dict:
    out.mkdir(parents=True, exist_ok=True)
    timings = {}

    def timed(name, fn):
        t = time.perf_counter()
        res = fn()
        timings[name] = round(time.perf_counter() - t, 4)
        return res

    text = draft_path.read_bytes().decode("utf-8")
    draft = timed("load", lambda: json.loads(text.lstrip("﻿")))
    (out / "roundtrip.json").write_text(timed("dumps_compact", lambda: compact(draft)), encoding="utf-8", newline="")

    reader = DraftReader()
    reader.draft = draft
    reader.draft_path = Path("/p/Proj/draft_content.json")
    reader.project_directory = reader.draft_path.parent
    captions = timed("extract_captions", reader.extract_captions)
    (out / "captions.json").write_text(compact([dict(c.__dict__) for c in captions]), encoding="utf-8", newline="")
    (out / "captions.srt").write_text(render_captions_srt(captions), encoding="utf-8", newline="")
    info = reader.get_project_info()
    insp = timed("inspect_project", lambda: reader.inspect_project(None))
    errors = timed("validate", lambda: DraftValidator().validate(draft, project_directory=None, require_audio_files=False))
    edges = ps._caption_edges_from_draft(draft)
    cut = ps.choose_cut_us(info.duration_us, edges)
    i = info.__dict__
    summary = {
        "project_info": {k: i[k] for k in [
            "project_id", "project_name", "version", "new_version", "width", "height", "fps", "duration_us"]}
        | {"duration_display": info.duration_display, "resolution": info.resolution}
        | {k: i[k] for k in [
            "video_track_count", "audio_track_count", "text_track_count", "video_segment_count",
            "audio_segment_count", "text_segment_count", "caption_count", "caption_with_tts_count",
            "empty_caption_count"]},
        "inspect": {k: insp.__dict__[k] for k in [
            "valid_caption_count", "selected_caption_count", "skipped_empty_count", "existing_tts_count",
            "orphan_text_segment_count", "invalid_content_count", "warnings", "has_tts_template"]},
        "find_tts_templates": reader.find_tts_templates(),
        "find_speed_templates": reader.find_speed_templates(),
        "get_tts_templates": reader.get_tts_templates(),
        "validate": errors,
        "caption_edges_count": len(edges),
        "cut_us": cut,
    }
    (out / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=1), encoding="utf-8", newline="")

    for part in (1, 2):
        deterministic(True)
        d, n = timed(f"slice_part{part}", lambda: ps.slice_draft(draft, cut, part=part))
        deterministic(False)
        (out / f"part{part}.json").write_text(compact([d, n]), encoding="utf-8", newline="")

    # Attach TTS to every caption, then regenerate it (replace_existing on a draft with TTS).
    patched = draft
    for round_no in (1, 2):
        r2 = DraftReader()
        r2.draft = patched
        r2.draft_path = Path("/p/Proj/draft_content.json")
        r2.project_directory = Path("/p/Proj")
        caps = r2.extract_captions()
        templates = r2.get_tts_templates()
        results = [CaptionTtsResult(caption_index=c.index, text_material_id=c.text_material_id,
                                    text_segment_id=c.text_segment_id, start_us=c.start_us, audio_path="a.mp3",
                                    source_duration_us=300_000 + 55_000 * len(c.text) + round_no, target_duration_us=0,
                                    status="generated") for c in caps if c.text.strip()]
        rel = {c.index: f"C:/Proj/textReading/tts_{c.index:06d}.mp3" for c in caps}
        work_copy = json.loads(json.dumps(patched))

        def do_patch(work_copy=work_copy, caps=caps, templates=templates, results=results, rel=rel):
            p = DraftPatcher(templates, fps=info.fps, alignment=NativeAudioAlignmentSettings())
            return p.patch(work_copy, captions=caps, results=results, voice_type="BV421_vivn_streaming",
                           resource_id="7252594014782755330", voice_display_name="Nhỏ Ngọt Ngào",
                           capcut_clip_speed=1.3, tone_modify_mode=ToneModifyMode.PRESERVE_PITCH,
                           existing_tts_mode="replace_existing", audio_rel_paths=rel,
                           project_directory=Path("/p/Proj"))
        deterministic(True)
        patched, stats = timed(f"patch_round{round_no}", do_patch)
        deterministic(False)
        perrs = timed(f"validate_patched{round_no}", lambda: DraftValidator().validate(
            patched, project_directory=None, require_audio_files=False))
        (out / f"patched{round_no}.json").write_text(compact([patched, stats.__dict__, perrs]), encoding="utf-8", newline="")

    with tempfile.TemporaryDirectory() as tmp:
        parent = Path(tmp).resolve()
        proj = parent / "Proj"
        (proj / "Timelines" / info.project_id).mkdir(parents=True)
        (proj / "draft_content.json").write_text(text, encoding="utf-8", newline="")
        (proj / "Timelines" / info.project_id / "draft_content.json").write_text(text, encoding="utf-8", newline="")
        (proj / "draft_meta_info.json").write_text(META, encoding="utf-8", newline="")
        (parent / "root_meta_info.json").write_text(ROOT_META.replace("<ROOT>", str(parent)), encoding="utf-8", newline="")
        deterministic(True)
        timed("split_project", lambda: ps.split_project(
            source_project_dir=proj, source_draft=draft, source_draft_path=proj / "draft_content.json"))
        deterministic(False)
        tree = {}
        for p in sorted(parent.rglob("*")):
            if p.is_file():
                data = p.read_bytes()
                try:
                    data = data.decode("utf-8").replace(str(parent), "<ROOT>").encode("utf-8")
                except UnicodeDecodeError:
                    pass
                tree[p.relative_to(parent).as_posix()] = [len(data), fnv1a(data)]
    (out / "tree.json").write_text(json.dumps(tree, ensure_ascii=False, indent=1), encoding="utf-8", newline="")
    return timings


def main() -> int:
    draft_path = Path(sys.argv[1]).resolve()
    work = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(tempfile.mkdtemp(prefix="capdraft-check-"))
    py_out, rs_out = work / "python", work / "rust"
    subprocess.run(["cargo", "build", "--release", "--example", "real_draft", "-q"],
                   cwd=ROOT / "rust", check=True)
    subprocess.run([str(ROOT / "rust" / "target" / "release" / "examples" / "real_draft"), str(draft_path), str(rs_out)],
                   check=True)
    py_t = python_side(draft_path, py_out)

    ok = True
    original = draft_path.read_bytes().lstrip(b"\xef\xbb\xbf")
    same_as_file = (rs_out / "roundtrip.json").read_bytes() == original
    print(f"{'roundtrip == original file':28} {'OK' if same_as_file else 'DIFF'}")
    ok &= same_as_file
    for name in ["roundtrip.json", "captions.json", "captions.srt", "summary.json", "part1.json", "part2.json",
                 "patched1.json", "patched2.json", "tree.json"]:
        a, b = (py_out / name).read_bytes(), (rs_out / name).read_bytes()
        same = a == b
        ok &= same
        print(f"{name:28} {'OK' if same else 'DIFF'}  ({len(a):,} bytes)")
        if not same:
            at = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
            print(f"    python …{a[max(0, at - 80):at + 80]!r}\n    rust   …{b[max(0, at - 80):at + 80]!r}")
    rs_t = json.loads((rs_out / "timings.json").read_text())
    print(f"\n{'step':20} {'Python s':>10} {'Rust s':>10} {'speed-up':>9}")
    for k, v in py_t.items():
        r = rs_t.get(k, 0)
        print(f"{k:20} {v:10.3f} {r:10.3f} {v / r if r else float('inf'):8.1f}x")
    print(f"\noutputs: {work}")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
