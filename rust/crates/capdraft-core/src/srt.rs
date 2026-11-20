//! Captions to SubRip (port of `core/capcut_project/srt_exporter.py`).

use crate::models::CaptionRow;
use crate::pyfmt::{py_strip, round_half_even};
use std::io;
use std::path::Path;

fn srt_timestamp(microseconds: i64) -> String {
    let total_ms = round_half_even(microseconds as f64 / 1000.0).max(0);
    let (hours, rem) = (total_ms / 3_600_000, total_ms % 3_600_000);
    let (minutes, rem) = (rem / 60_000, rem % 60_000);
    let (seconds, ms) = (rem / 1000, rem % 1000);
    format!("{hours:02}:{minutes:02}:{seconds:02},{ms:03}")
}

/// Ordered by `(start_us, index)`, empty captions dropped, numbered from 1.
#[must_use]
pub fn render_captions_srt(captions: &[CaptionRow]) -> String {
    let mut ordered: Vec<&CaptionRow> = captions.iter().collect();
    ordered.sort_by_key(|c| (c.start_us, c.index));
    let mut blocks: Vec<String> = Vec::new();
    for caption in ordered {
        let normalized = caption.text.replace("\r\n", "\n").replace('\r', "\n");
        let text = py_strip(&normalized);
        if text.is_empty() {
            continue;
        }
        let start_us = caption.start_us.max(0);
        let end_us = start_us.max(start_us + caption.duration_us.max(0));
        blocks.push(format!(
            "{}\n{} --> {}\n{}",
            blocks.len() + 1,
            srt_timestamp(start_us),
            srt_timestamp(end_us),
            text
        ));
    }
    let mut out = blocks.join("\n\n");
    if !blocks.is_empty() {
        out.push('\n');
    }
    out
}

/// Number of captions with text, the count `export_captions_srt` returns.
#[must_use]
pub fn count_srt_captions(captions: &[CaptionRow]) -> usize {
    captions.iter().filter(|c| !c.is_empty()).count()
}

/// Write UTF-8 with BOM and `\n` line endings, as the original does.
///
/// # Errors
/// Any I/O error writing `output_path`.
pub fn export_captions_srt(captions: &[CaptionRow], output_path: &Path) -> io::Result<usize> {
    let rendered = render_captions_srt(captions);
    let mut bytes = Vec::with_capacity(rendered.len() + 3);
    bytes.extend_from_slice("\u{feff}".as_bytes());
    bytes.extend_from_slice(rendered.as_bytes());
    std::fs::write(output_path, bytes)?;
    Ok(count_srt_captions(captions))
}
