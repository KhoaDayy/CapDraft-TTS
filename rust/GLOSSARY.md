# Glossary

| Term (original) | Meaning | Rust name |
|---|---|---|
| draft / `draft_content.json` | CapCut project timeline JSON | `serde_json::Value` via `pyjson::loads` |
| caption / `CaptionRow` | One text segment on a text track | `models::CaptionRow` |
| `_us` | Microseconds, CapCut's time unit | `i64` fields with `_us` suffix |
| clip speed | CapCut segment speed; timeline duration = frame-snapped `source / speed` | `models::compute_target_duration_us` |
| tone modify mode | Whether pitch follows clip speed (`is_tone_modify`) | `models::ToneModifyMode` |
| draftpath placeholder | `##_draftpath_placeholder_<UUID>_##`, CapCut's project-relative path prefix | `paths::find_draftpath_placeholder` |
| `textReading/` | Project subfolder where TTS audio files live | `paths::capcut_text_reading_path` |
| `Timelines/<id>/` | Modern CapCut layout: per-timeline copies of the draft | `paths::list_draft_write_targets` |
