# Port map: CapDraft TTS → Rust

Goal: a standalone Rust desktop app (Windows x64) that does what the Python app does;
the Python code stays as reference until cutover. Decisions: [DECISIONS.md](DECISIONS.md)
(ADR-001 … ADR-012). Terms: [GLOSSARY.md](GLOSSARY.md).

How a unit is verified: [ADR-006](DECISIONS.md#adr-006-verify-against-recordings-of-the-python-original).

```sh
python rust/tools/record_wave1.py          # re-record (from the repo root)
python rust/tools/record_wave2.py
cargo test --manifest-path rust/Cargo.toml # replays rust/golden/*.jsonl
```

## Units

| Wave | Python original | Rust | State | Golden |
|---|---|---|---|---|
| 1 | `core/capcut_project/models.py` (time math, tone mode, data types) | `models.rs` | done | 8709/8709 |
| 1 | `core/capcut_project/srt_exporter.py` | `srt.rs` | done | 600/600 |
| 1 | `core/capcut_project/paths.py` | `paths.rs` | done | 207/207 + unit tests |
| 1 | `core/i18n.py` | `i18n.rs` + `i18n.json` | done | 646/646 |
| 1 | `core/preferences.py` | `preferences.rs` | done | in i18n.jsonl |
| 1 | `json.dumps`/`json.loads` semantics (ADR-002) | `pyjson.rs`, `pyfmt.rs` | done | 486/486 + `isspace` set |
| 2 | `native_audio_alignment.py` | `alignment.rs` | done | 681/681 |
| 2 | `project_split.py`: `choose_cut_us`, `slice_draft`, caption edges | `project_split.rs` | done | 1332/1332 |
| 2 | `project_split.py`: `split_project` (copy, Timelines rename, meta + root_meta) | `project_split.rs` | done | 40 projects, result + whole file tree byte for byte |
| 2 | `validator.py` | `validator.rs` | done | 417/417 |
| 2 | `draft_reader.py` (captions, info, templates, inspect, load) | `draft_reader.rs` | done | 1525/1525 + 12 load fixtures |
| 2 | Python value semantics (ADR-011), ids/clock (ADR-012) | `pyval.rs`, `ids.rs` | done | via the units above |
| 3 | `draft_patcher.py` | | open | |
| 3 | `project_exporter.py` (backup, atomic write, rollback) | | open | |
| 3 | `core/cache.py`, `core/config.py`, `core/logger.py` | | open | |
| 3 | `voice_catalog.py`, `voice_catalog_updater.py` | | open | |
| 4 | `external/capcut-tts-api/capcut_common_task_client.py` (RSA, AWS4 signing, upload) | | open | record signing/payloads; mock HTTP |
| 4 | `core/capcut_tts.py` (parallel batches, cancel) | | open | |
| 5 | `tts_project_service.py`, `video_merge.py`, `capcut_process.py` | | open | |
| 6 | `ui/*` (PySide6) → Rust GUI | | open | Slint (ADR-008) |

## Out of scope

- Root `capcut_tts.py` (ADR-003).
- Platforms other than Windows x64 (ADR-004).

## Check on a real project

```sh
python rust/tools/check_real_draft.py path/to/draft_content.json
```

Runs every ported function on the draft in Python and in Rust and compares the outputs byte for
byte (re-serialized draft, captions, SRT, project info, templates, inspection, validation, both
split halves, and the whole split project tree), then prints timings. The draft stays outside the
repository.

Last run (2026-11-20) on a real CapCut 191 project, 19.6 MB, 3,585 captions, 2 h 28 min, no
TTS yet: all 8 outputs identical, re-serialized draft identical to the original file.

## Known limits of the recordings

- Mixed-type segment ids with equal time ranges: Python's sort raises `TypeError` with the
  operand types of the first failing comparison; the Rust sort may compare another pair first.
- `json.loads` accepts `NaN`/`Infinity` and lone surrogates, `serde_json` does not (ADR-002).
- Unassigned code points count as printable in `pyval::repr_str` (Python escapes them).
- Two mutations survive as equivalent: the dead `else` in `_slice_segment_part2`'s
  `remain_old`, and `draft_ids` set by the purge, which `register_root_meta` always overwrites.

## Open questions

None.
