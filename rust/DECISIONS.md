# Decisions (ADR)

Numbered in order and never renumbered. A changed decision gets a new ADR that supersedes the old one.

## ADR-001: Rewrite the whole app as a pure Rust desktop app
Status: accepted (2026-11-20)
Context: CapDraft TTS is a PySide6 desktop app (`ui/`, ~2,000 lines) over a Python core
(`core/`, `external/`, ~6,000 lines). The user chose a full rewrite (Q1: b) over keeping PySide6.
Decision: The final product is one Rust binary: a GUI crate on top of `capdraft-core`.
The Python app stays in the repo as the reference until cutover.
Consequences: rust-anything only verifies CLIs, servers and libraries, so the GUI is
rewritten by hand and checked by hand. The core is ported first and verified against the
Python original case by case (ADR-006).

## ADR-002: Byte-identical JSON output
Status: accepted (2026-11-20)
Context: The user was unsure (Q2). CapCut only needs valid JSON, but comparing the Rust
output with Python's byte for byte is the cheapest and strictest check for every later wave.
Decision: `capdraft_core::pyjson` writes JSON exactly like `json.dumps(ensure_ascii=False)`:
key order kept, integers verbatim, floats in Python `repr` form, the same escapes.
Consequences: Inputs Python accepts but `serde_json` rejects (`NaN`/`Infinity` literals,
lone surrogates `\ud800`) fail to load in Rust. CapCut does not write them.

## ADR-003: Drop the legacy root `capcut_tts.py`
Status: accepted (2026-11-20)
Context: `capcut_tts.py` at the repo root is an older copy of `core/capcut_tts.py`; nothing imports it.
Decision: Not ported.

## ADR-004: Windows x64 only
Status: accepted (2026-11-20)
Decision: Target `x86_64-pc-windows-msvc`. Code stays portable where it costs nothing
(tests run on Linux in CI), but Windows-only behavior (CapCut paths, `Path.name` splitting
on `\`, process detection) follows Windows.

## ADR-005: Stop for review after each wave
Status: accepted (2026-11-20)
Decision: After each wave in `MAP.md`: report golden replay counts, clippy and tests, then
wait for the user's go-ahead.

## ADR-006: Verify against recordings of the Python original
Status: accepted (2026-11-20)
Context: Installing the rust-anything CLI was not permitted in the session that started the port.
Decision: `rust/tools/record_<wave>.py` runs the original functions on edge cases and random
inputs and writes `rust/golden/<unit>.jsonl`; `crates/capdraft-core/tests/golden.rs` replays
every line. A unit is done when every case matches, `cargo clippy -D warnings` and
`cargo test` pass, and no `unwrap`/`panic` sits where the original raises.
Consequences: Same contract as `rust-anything record`/`diff`/`gate`; switching to the CLI later
only changes the tooling. A mismatch is fixed in the port, never in the recording.

## ADR-007: `capdraft-core` has no GUI dependencies
Status: accepted (2026-11-20)
Decision: All logic lives in `crates/capdraft-core`; the GUI crate only renders state and
calls the core. Long jobs report progress through callbacks and support cancellation.
Consequences: The core can also ship a CLI later at no extra cost.

## ADR-008: Slint for the GUI
Status: accepted (2026-11-20)
Context: The current UI uses PySide6-Fluent-Widgets; the caption table must scroll ~5,000 rows.
Decision: Wave 6 builds the GUI with Slint and its `fluent` style.
Consequences: Slint's royalty-free license for desktop apps requires attribution
("Made with Slint", e.g. in the About dialog). Virtualized `ListView` covers the caption table.

## ADR-009: Keep every on-disk format
Status: accepted (2026-11-20)
Decision: `config.json`, the TTS cache layout, `Voice.json`, `device.json`, logs, manifests and
backups keep their names, locations and formats, so the Rust build reads what the Python build wrote.

## ADR-010: Ship the Rust build beside the Python release first
Status: accepted (2026-11-20)
Decision: Release the Rust build as a beta next to the Python release; replace the Python
build once the beta has run on real projects without regressions.

## ADR-011: Reproduce the original's failures on malformed drafts
Status: accepted (2026-11-20)
Context: The Python core reads drafts loosely (`int(x or 0)`, `.get` on whatever is there) and
raises `TypeError`/`ValueError`/`AttributeError` on unexpected types.
Decision: `capdraft_core::pyval` implements those Python semantics (truthiness, `int()`,
`float()`, `str()`/`repr()`, iteration, `==` across int/float/bool) and the port returns
`Err("<PythonType>: <message>")` exactly where the original raises. Recordings mutate draft
values to reach those paths.
Consequences: The UI can show the same error text; a future ADR may choose to be more lenient
than the original, as a recorded known difference.

## ADR-012: Deterministic ids and clock in recordings
Status: accepted (2026-11-20)
Decision: `capdraft_core::ids` makes UUIDs (`uuid4().upper()`) and the clock overridable per
thread. Recorders patch `uuid.uuid4` to `UUID(int=1, 2, …)` and `time.time` to a constant; the
replay tests set the same sequence, so new ids and whole split projects compare byte for byte.
