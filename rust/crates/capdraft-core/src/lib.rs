//! CapDraft TTS core, ported from the Python `core/` package.
//!
//! Every module matches the original's behavior; `rust/golden/*.jsonl` holds the
//! cases recorded from the Python original and `tests/golden*.rs` replays them.

pub mod alignment;
pub mod cache;
pub mod capcut_client;
pub mod capcut_process;
pub mod config;
pub mod draft_patcher;
pub mod draft_reader;
pub mod i18n;
pub mod ids;
pub mod json;
pub mod logger;
pub mod models;
pub mod paths;
pub mod preferences;
pub mod project_exporter;
pub mod project_split;
pub mod pyfmt;
pub mod pyjson;
pub mod pyval;
pub mod service;
pub mod srt;
pub mod tts;
pub mod validator;
pub mod video_merge;
pub mod voice_catalog;
