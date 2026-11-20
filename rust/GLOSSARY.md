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
| combination / nested draft | `materials.drafts[].draft`, a sub-timeline (AutoVideo style) | `project_split::slice_draft` recursion |
| split part | `<project>_part1` / `_part2` folders written by "Split in 2" | `project_split::split_project`, `SplitResult` |
| `root_meta_info.json` | CapCut's project library index (`all_draft_store`) next to the projects | `register_root_meta`, `purge_stale_root_meta` |
| `draft_meta_info.json` | Per-project card metadata; `draft_id` differs from the content id | `patch_meta` |
| native alignment | Head trim via `source_timerange.start` + `audio_fade`, no re-encode | `alignment::apply_native_audio_alignment` |
| TTS templates | Material/segment/track/extras JSON copied from an existing TTS clip, or built in | `draft_reader::find_tts_templates`, `DraftReader::builtin_tts_templates` |
| text mode write | Python `open(..., "w")`: `\n` → `\r\n` on Windows | `pyjson::text_mode` |
