//! CapDraft TTS core, ported from the Python `core/` package.
//!
//! Every module matches the original's behavior; `rust/golden/*.jsonl` holds the
//! cases recorded from the Python original and `tests/golden.rs` replays them.

pub mod i18n;
pub mod models;
pub mod paths;
pub mod preferences;
pub mod pyfmt;
pub mod pyjson;
pub mod srt;
