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
| safe TTS voice | The one voice verified to export from CapCut (`BV074_streaming`); every TTS material is pointed at it | `draft_patcher::SAFE_TTS_*` |
| lane | One audio track during greedy interval partitioning of the new TTS segments | `DraftPatcher::assign_tracks` |
| write targets | Root `draft_content.json` plus each `Timelines/<id>` copy, written all-or-nothing | `ProjectExporter::atomic_write_draft` |
| precommit snapshot | `<target>.precommit`, the pre-image restored if a later target fails | `atomic_write_draft` |
| TTS cache key | sha256 of `text|voice|rate:.2f|resource_id` | `TtsCache::generate_hash` |
| common_task | CapCut's async job API: `tts-new` creates a task, `tts-query` polls it | `capcut_client::Request` |
| chunk | Up to `tts_chunk_size` captions sent as one TTS task, with its own random device | `CapCutTts::process_chunk` |
| device profile | The device fields presented to CapCut (`legacy` = Python app, `upstream` = K07VN 2026-10) | `capcut_client::DeviceProfile` |
| payload sign | RSA PKCS#1 v1.5 of `appid:…&did:…&creditDisable:false&ssml:<md5>&extraInfo:…` | `capcut_client::make_tts_payload_sign` |
| sign header | md5 of `9e2c|<last 7 path chars>|3|appvr|device-time|tdid|11ac` | `capcut_client::make_sign_header` |
