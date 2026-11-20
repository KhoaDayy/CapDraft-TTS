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

## ADR-013: Own JSON value type instead of `serde_json::Value`
Status: accepted (2026-11-20)
Context: The user needs the Rust build to be clearly faster than Python on large projects, not on
par. On a real 19.6 MB draft, building a `serde_json::Value` (with `preserve_order` and
`arbitrary_precision`) took 0.12 to 0.14 s whatever the parser, slower than Python's `json`.
`sonic-rs` parses it in 0.015 s but loses `-0.0`'s sign, turns big ints into floats, keeps
duplicate keys and reorders keys on insert, which breaks ADR-002.
Decision: `capdraft_core::json` is the draft type: `Arc` copy-on-write arrays and objects
(clone O(1), only touched paths are copied), inline strings up to 24 bytes, ordered objects with
`dict` semantics (insert replaces in place, duplicate keys keep first position and last value),
Python numbers (`int` of any size, `float` with `-0.0` and infinity). A hand-written parser
(SWAR string scanning, shared scratch stacks) and writer (`itoa`/`ryu`, run-length escaping).
Consequences: `serde_json` stays only for `jv!` literals. `pyjson` and `pyval` wrap the new type.

## ADR-014: Use every core
Status: accepted (2026-11-20)
Decision: Large arrays (≥ 64 elements spanning ≥ 256 KB, the bulk of a CapCut draft) are split by
a bracket-matching scan and parsed on all cores; large arrays are also written in parallel chunks.
Caption texts are parsed in parallel. A split copies both parts, slices both halves and serializes
both drafts in parallel, and serializes each draft once for its six identical files. Results are
identical to the sequential path: `json::parse_sequential` exists for the tests that check it, and
a malformed large array falls back to the sequential parser so errors are reported the same way.
The app binary should use `mimalloc` as its global allocator.

## ADR-015: Speed is part of done
Status: accepted (2026-11-20)
Decision: Each wave runs `rust/tools/check_real_draft.py` on a large real project and reports
Python and Rust times per step. A unit is done only when its outputs are identical and it is
clearly faster than the original; a slowdown blocks the wave.

## ADR-016: What wave 3 leaves out
Status: accepted (2026-11-20)
Decision: The voice catalog download (`VoiceCatalog.load`, HTTP GET of `Voice.json`) moves to
wave 4 with the TTS client, so one HTTP stack is chosen for both; wave 3 ports parsing, filtering
and lookup. The log keeps the original's line layout (`[time] [LEVEL] [file:line] - message`)
but file and line name Rust sources, so logs are not replayed. `AppConfig` is a value the app
owns, not a process-wide singleton.

## ADR-017: What wave 4 ports from the CapCut client
Status: accepted (2026-11-20)
Context: `capcut_common_task_client.py` also has STT (`stt-new`, `stt-query`, `stt-file`) and VOD
upload with AWS4 signing; the app only calls `tts-new` and `tts-query`. The Python wrapper imported
the client from `capcut_tts_path` at run time.
Decision: Port `tts-new` and `tts-query` (query, headers, SSML, MD5 sign header, RSA PKCS#1 v1.5
payload sign) and the batch wrapper; leave STT and upload out. The client is compiled in, so
`capcut_tts_path` is no longer read. HTTP goes through `ureq` behind a `Transport` trait.
Behavior kept on purpose: a chunk that fails before its downloads leaves its captions without a
result; on cancel, downloads already queued still finish into the cache (results say "Cancelled by
user"); download errors are worded as `requests` words them.
Consequences: Wave 4 was verified against a fake CapCut server; the live API was not reachable from
the build container (the session's permission check refused calling it), so the first live run is
`cargo run --release --example tts_live` on the user's machine.

## ADR-018: Device profiles; the upstream device by default
Status: accepted (2026-11-20)
Context: The TTS client was forked from `K07VN/capcut-tts-api`. Upstream (2026-10-19) now sends
`device_type`/`device_brand` `MacBookPro17,4` and one random 20-digit id for `device_id`, `iid`
and `tdid`; its issue tracker advises changing those fields when CapCut blocks a device. The
user asked for that update.
Decision: `DeviceProfile::Legacy` reproduces the Python app (the golden files use it);
`DeviceProfile::Upstream` (default in the Rust app, `config.json` key `tts_device_profile`,
`legacy` to opt out) uses the upstream model and id layout, a fresh device per chunk, and polls
with the chunk's own device. `device.json` can still override `device_type`/`device_brand`.
Consequences: With the default profile the Rust build's requests differ from the Python build's in
those device fields only. Other upstream changes (XML-escaped voice and resource id, polling with
the returned `bind_id`, more success status names, a missing `ret` not being an error) are not
adopted yet: open question in `MAP.md`.

## ADR-019: The upstream profile also follows upstream's protocol handling
Status: accepted (2026-11-20), extends ADR-018
Context: The user accepted the remaining upstream `K07VN/capcut-tts-api` changes (Q11).
Decision: With `DeviceProfile::Upstream` the client XML-escapes the voice and resource id in the
SSML, polls with the `bind_id` the server returned (task, then data), accepts `success`, `succeed`,
`succeeded`, `completed` and `done` (any case), treats a missing, empty or numeric `0` `ret` as
success, and downloads from `speech_url`, else `audio_url`, else `url`. `prosody rate` keeps the
user's speed (upstream hard-codes `1.0`). `DeviceProfile::Legacy` is unchanged and remains what
the golden files check.
