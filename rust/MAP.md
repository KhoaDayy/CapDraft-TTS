# Port map: CapDraft TTS → Rust

Goal: a standalone Rust desktop app (Windows x64) that does what the Python app does;
the Python code stays as reference until cutover. Decisions: [DECISIONS.md](DECISIONS.md)
(ADR-001 … ADR-019). Terms: [GLOSSARY.md](GLOSSARY.md).

How a unit is verified: [ADR-006](DECISIONS.md#adr-006-verify-against-recordings-of-the-python-original).

```sh
python rust/tools/record_wave1.py          # re-record (from the repo root)
python rust/tools/record_wave2.py
python rust/tools/record_wave3.py
python rust/tools/record_wave4.py
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
| 2 | JSON value type, parser, writer (ADR-013, ADR-014) | `json.rs` | done | all golden files are read by it; parallel = sequential tests |
| 3 | `draft_patcher.py` (materials, segments, speeds, fades, replace/skip existing, track assignment) | `draft_patcher.rs` | done | 164/164 |
| 3 | `project_exporter.py` (audio copy, backup + prune, atomic multi-target write, rollback, manifest, log) | `project_exporter.rs` | done | 24 projects, steps + whole tree |
| 3 | `core/config.py` | `config.rs` | done | 13 configs, values + saved file |
| 3 | `core/cache.py` | `cache.rs` | done | 192 hashes (`.2f` rounding traps) + 6 cache folders |
| 3 | `voice_catalog.py` (parse, filter, languages, find) | `voice_catalog.rs` | done | 7 catalogs; download: wave 4 (ADR-016) |
| 3 | `voice_catalog_updater.py` | folded into `voice_catalog.rs` | done | shim only |
| 3 | `core/logger.py` | `logger.rs` | done | same line layout, not replayed (ADR-016) |
| 4 | `external/capcut-tts-api/capcut_common_task_client.py`: `tts-new`, `tts-query` (query, headers, SSML, MD5 sign, RSA PKCS#1 v1.5) | `capcut_client.rs` | done | 122/122 requests byte for byte |
| 4 | `core/capcut_tts.py` (cache hits, chunks, parallel chunks, polling with backoff, parallel downloads, cancel) | `tts.rs` | done | 22 runs against a fake CapCut: results, callbacks, cancel checks, requests, waits, files |
| 4 | Device profiles and protocol handling from upstream `K07VN/capcut-tts-api` (ADR-018, ADR-019) | `capcut_client.rs`, `tts.rs` | done | 2 tests |
| 4 | Voice catalog download (ADR-016) | `voice_catalog.rs` | done | unit test |
| 4 | STT, VOD upload (not used by the app) | | out of scope (ADR-017) | |
| 5 | `tts_project_service.py`, `video_merge.py`, `capcut_process.py` | | open | |
| 6 | `ui/*` (PySide6) → Rust GUI | | open | Slint (ADR-008) |

## Out of scope

- STT and VOD upload in the CapCut client (ADR-017).

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
TTS yet: all 10 outputs identical (including both 32 MB patched drafts), re-serialized draft identical to the original file. Timings on
the 4-core build container (one run each; more cores make the parallel steps faster):

| Step | Python | Rust | Speed-up |
|---|---:|---:|---:|
| load (read + parse) | 0.148 s | 0.049 s | 3.0x |
| dumps_compact | 0.106 s | 0.023 s | 4.7x |
| extract_captions | 0.047 s | 0.009 s | 5.5x |
| inspect_project | 0.015 s | 0.005 s | 3.0x |
| validate | 0.104 s | 0.005 s | 19x |
| slice part 1 / part 2 | 0.369 / 0.411 s | 0.002 / 0.005 s | 205x / 88x |
| split_project (copy + slice + write both parts) | 1.608 s | 0.071 s | 22.5x |
| patch: TTS for all 3,585 captions | 0.262 s | 0.044 s | 6.0x |
| patch again, replacing that TTS | 0.311 s | 0.052 s | 6.0x |
| validate the patched draft (32 MB) | 0.185 / 0.338 s | 0.024 s | 7.6x / 14x |

`cargo run --release --example bench_json -- <draft>` times parse, clone and dump alone
(best of 7: parse 0.037 s, dump 0.010 s, clone O(1)).

## Known limits of the recordings

- Mixed-type segment ids with equal time ranges: Python's sort raises `TypeError` with the
  operand types of the first failing comparison; the Rust sort may compare another pair first.
- `json.loads` accepts `NaN`/`Infinity` and lone surrogates, `serde_json` does not (ADR-002).
- Unassigned code points count as printable in `pyval::repr_str` (Python escapes them).
- Two mutations survive as equivalent: the dead `else` in `_slice_segment_part2`'s
  `remain_old`, and `draft_ids` set by the purge, which `register_root_meta` always overwrites.

## Open questions

- First live run against CapCut: `cargo run --release --example tts_live` on the user's machine.
