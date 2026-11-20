# Port map: CapDraft TTS → Rust

Goal: a standalone Rust desktop app (Windows x64) that does what the Python app does;
the Python code stays as reference until cutover. Decisions: [DECISIONS.md](DECISIONS.md)
(ADR-001 … ADR-007). Terms: [GLOSSARY.md](GLOSSARY.md).

How a unit is verified: [ADR-006](DECISIONS.md#adr-006-verify-against-recordings-of-the-python-original).

```sh
python rust/tools/record_wave1.py          # re-record (from the repo root)
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
| 2 | `native_audio_alignment.py` | | open | |
| 2 | `project_split.py` (`slice_draft`, `choose_cut_us`, …) | | open | |
| 2 | `validator.py` | | open | |
| 2 | `draft_reader.py` | | open | |
| 3 | `draft_patcher.py` | | open | |
| 3 | `project_exporter.py` (backup, atomic write, rollback) | | open | |
| 3 | `core/cache.py`, `core/config.py`, `core/logger.py` | | open | |
| 3 | `voice_catalog.py`, `voice_catalog_updater.py` | | open | |
| 4 | `external/capcut-tts-api/capcut_common_task_client.py` (RSA, AWS4 signing, upload) | | open | record signing/payloads; mock HTTP |
| 4 | `core/capcut_tts.py` (parallel batches, cancel) | | open | |
| 5 | `tts_project_service.py`, `video_merge.py`, `capcut_process.py` | | open | |
| 6 | `ui/*` (PySide6) → Rust GUI | | open | framework: open question |

## Out of scope

- Root `capcut_tts.py` (ADR-003).
- Platforms other than Windows x64 (ADR-004).

## Open questions

- GUI framework for wave 6.
- Keep `config.json`, cache layout, `Voice.json`, `device.json` formats unchanged?
- Cutover: ship the Rust build beside the Python release first, or replace it at once?
