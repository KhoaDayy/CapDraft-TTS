"""Facade: load CapCut project → generate TTS → patch → validate → commit."""

from __future__ import annotations

import time
import traceback
import uuid
from datetime import datetime
from pathlib import Path
from typing import Any, Callable

from core.capcut_tts import CapCutTtsWrapper
from core.config import AppConfig
from core.logger import logger, setup_file_logger
from .capcut_process import is_capcut_running
from .draft_patcher import DraftPatcher
from .draft_reader import DraftReader
from .models import (
    CaptionRow,
    CaptionTtsResult,
    CapCutProjectInfo,
    GenerationLogEvent,
    GenerationResult,
    ProjectInspectionResult,
    ToneModifyMode,
    coerce_tone_modify_mode,
    compute_target_duration_us,
    format_duration_us,
    map_tone_mode_to_capcut_flag,
    seconds_to_us,
)
from .native_audio_alignment import NativeAudioAlignmentSettings
from .paths import ensure_draftpath_placeholder
from .project_exporter import ProjectExporter
from .validator import DraftValidator
from .voice_catalog import VoiceCatalog
from .voice_catalog_updater import VoiceCatalogUpdateResult, update_voice_catalog_from_url


LogCallback = Callable[[GenerationLogEvent], None]
ProgressCallback = Callable[[float, str], None]
ItemCallback = Callable[[int, dict[str, Any]], None]
CancelCallback = Callable[[], bool]


class CapCutProjectTtsService:
    def __init__(
        self,
        *,
        log_callback: LogCallback | None = None,
        voice_catalog_url: str | None = None,
    ):
        self.config = AppConfig()
        self.reader = DraftReader()
        self.catalog = VoiceCatalog(voice_catalog_url or self.config.voice_catalog_url)
        self.exporter = ProjectExporter(max_backups=int(self.config.get("max_backups", 10) or 10))
        self.validator = DraftValidator()
        self._log_callback = log_callback
        self._log_lines: list[str] = []
        self._info: CapCutProjectInfo | None = None

        try:
            self.catalog.load()
        except Exception as e:
            logger.warning("Voice catalog not loaded at init: %s", e)

    # ------------------------------------------------------------------
    # Logging
    # ------------------------------------------------------------------
    def set_log_callback(self, cb: LogCallback | None) -> None:
        self._log_callback = cb

    def emit_log(
        self,
        level: str,
        message: str,
        *,
        stage: str | None = None,
        caption_index: int | None = None,
        progress: float | None = None,
        log_callback: LogCallback | None = None,
    ) -> None:
        level_u = (level or "INFO").upper()
        prefix = f"[{stage}] " if stage else ""
        line = f"{prefix}{message}"
        self._log_lines.append(
            f"{datetime.now().strftime('%H:%M:%S')} {level_u:<7} {line}"
        )
        # cap memory
        if len(self._log_lines) > 5000:
            self._log_lines = self._log_lines[-4000:]

        log_fn = {
            "DEBUG": logger.debug,
            "INFO": logger.info,
            "WARNING": logger.warning,
            "ERROR": logger.error,
            "SUCCESS": logger.info,
            "CACHED": logger.info,
        }.get(level_u, logger.info)
        log_fn("%s", line)

        cb = log_callback or self._log_callback
        if cb:
            try:
                cb(
                    GenerationLogEvent(
                        timestamp=datetime.now(),
                        level=level_u,
                        message=message,
                        stage=stage,
                        caption_index=caption_index,
                        progress=progress,
                    )
                )
            except Exception as e:
                logger.debug("log_callback error ignored: %s", e)

    # ------------------------------------------------------------------
    # Load / inspect
    # ------------------------------------------------------------------
    def load_project(self, draft_path: Path | str) -> CapCutProjectInfo:
        self._log_lines.clear()
        self.emit_log("INFO", "Loading CapCut project...", stage="Project")
        self.reader.load_draft(draft_path)
        info = self.reader.get_project_info()
        self._info = info
        self.emit_log("INFO", f"Loaded {info.draft_path.name}", stage="Project")
        self.emit_log("INFO", f"ID: {info.project_id}", stage="Project")
        self.emit_log(
            "INFO",
            f"Version: {info.version} / {info.new_version or '-'}",
            stage="Project",
        )
        self.emit_log(
            "INFO",
            f"Canvas: {info.width}×{info.height} @ {info.fps:g} FPS",
            stage="Project",
        )
        self.emit_log(
            "INFO",
            f"Duration: {format_duration_us(info.duration_us)}",
            stage="Project",
        )
        self.emit_log(
            "INFO",
            f"Tracks: video={info.video_track_count}, audio={info.audio_track_count}, text={info.text_track_count}",
            stage="Project",
        )
        self.emit_log(
            "INFO",
            f"Captions: total={info.caption_count}, existing_tts={info.caption_with_tts_count}, empty={info.empty_caption_count}",
            stage="Project",
        )
        return info

    def get_captions(self) -> list[CaptionRow]:
        return self.reader.get_captions()

    def get_voices(self) -> list:
        return self.get_voice_catalog().voices

    def get_voice_catalog(self, *, reload: bool = False) -> VoiceCatalog:
        url = self.config.voice_catalog_url
        if reload or not self.catalog.voices or self.catalog.url != url:
            self.catalog.load(url)
        return self.catalog

    def update_voice_catalog(self, url: str | None = None) -> VoiceCatalogUpdateResult:
        return update_voice_catalog_from_url(
            url=url or self.config.voice_catalog_url,
            catalog=self.catalog,
        )

    def inspect_project(
        self, selected_caption_ids: list[str] | None = None
    ) -> ProjectInspectionResult:
        result = self.reader.inspect_project(selected_caption_ids)
        for w in result.warnings:
            self.emit_log("WARNING", w, stage="Project")
        self.emit_log(
            "INFO",
            f"Inspection: valid={result.valid_caption_count}, empty={result.skipped_empty_count}, "
            f"existing_tts={result.existing_tts_count}, orphan_segments={result.orphan_text_segment_count}",
            stage="Project",
        )
        return result

    def is_capcut_running(self) -> bool:
        return is_capcut_running()

    # ------------------------------------------------------------------
    # Generate + attach (single workflow)
    # ------------------------------------------------------------------
    def generate_and_attach(
        self,
        selected_caption_ids: list[str],
        voice_type: str,
        resource_id: str,
        voice_display_name: str,
        tts_rate: float,
        capcut_clip_speed: float,
        tone_modify_mode: ToneModifyMode,
        existing_tts_mode: str = "replace_existing",
        allow_partial_attach: bool = True,
        progress_callback: ProgressCallback | None = None,
        item_completed_callback: ItemCallback | None = None,
        log_callback: LogCallback | None = None,
        is_cancelled_callback: CancelCallback | None = None,
        use_cache: bool = True,
        alignment_settings: NativeAudioAlignmentSettings | None = None,
    ) -> GenerationResult:
        started = time.perf_counter()
        cb = log_callback or self._log_callback
        # QComboBox / IPC may pass str instead of enum
        tone_modify_mode = coerce_tone_modify_mode(tone_modify_mode)
        alignment_settings = alignment_settings or NativeAudioAlignmentSettings()

        def prog(p: float, msg: str = ""):
            if progress_callback:
                try:
                    progress_callback(max(0.0, min(1.0, p)), msg)
                except Exception:
                    pass

        def cancelled() -> bool:
            return bool(is_cancelled_callback and is_cancelled_callback())

        if self.reader.draft_path is None:
            raise RuntimeError("No project loaded. Call load_project() first.")

        draft_path = self.reader.draft_path
        project_dir = self.reader.project_directory
        assert project_dir is not None

        # Reload from disk to avoid stale state
        self.emit_log("INFO", "Reloading project from disk before generate", stage="preparing", log_callback=cb)
        prog(0.01, "reload")
        self.reader.load_draft(draft_path)
        info = self.reader.get_project_info()
        self._info = info
        captions = self.reader.get_captions()

        selected_set = set(selected_caption_ids)
        selected_caps = [
            c
            for c in captions
            if c.text_segment_id in selected_set
            or str(c.index) in selected_set
            or c.text_material_id in selected_set
        ]

        inspection = self.inspect_project([c.text_segment_id for c in selected_caps])
        prog(0.04, "inspect")

        self.emit_log("INFO", f"Voice: {voice_display_name}", stage="Settings", log_callback=cb)
        self.emit_log("INFO", f"Voice type: {voice_type}", stage="Settings", log_callback=cb)
        self.emit_log("INFO", f"Resource ID: {resource_id}", stage="Settings", log_callback=cb)
        self.emit_log("INFO", f"TTS generation rate: {tts_rate:.2f}x", stage="Settings", log_callback=cb)
        self.emit_log(
            "INFO",
            f"CapCut clip speed: {capcut_clip_speed:.2f}x",
            stage="Settings",
            log_callback=cb,
        )
        from .models import TONE_MODIFY_MAPPING_VERIFIED

        self.emit_log(
            "INFO",
            f"Tone mode: {tone_modify_mode.value} "
            f"(is_tone_modify={map_tone_mode_to_capcut_flag(tone_modify_mode)}, "
            f"mapping_verified={TONE_MODIFY_MAPPING_VERIFIED})",
            stage="Settings",
            log_callback=cb,
        )
        self.emit_log(
            "INFO",
            f"Existing TTS mode: {existing_tts_mode}",
            stage="Settings",
            log_callback=cb,
        )
        self.emit_log(
            "INFO",
            f"Selected captions: {len(selected_caps)}/{info.caption_count}",
            stage="Settings",
            log_callback=cb,
        )
        self.emit_log("INFO", f"Cache: {'enabled' if use_cache else 'disabled'}", stage="Settings", log_callback=cb)
        self.emit_log("INFO", f"Project: {project_dir}", stage="Settings", log_callback=cb)

        # Filter empty
        to_generate: list[CaptionRow] = []
        item_results: list[CaptionTtsResult] = []
        skipped = 0
        for cap in selected_caps:
            if cap.is_empty:
                skipped += 1
                item_results.append(
                    CaptionTtsResult(
                        caption_index=cap.index,
                        text_material_id=cap.text_material_id,
                        text_segment_id=cap.text_segment_id,
                        start_us=cap.start_us,
                        audio_path="",
                        source_duration_us=0,
                        target_duration_us=0,
                        status="skipped",
                        error="empty text",
                    )
                )
                self.emit_log(
                    "WARNING",
                    f"Caption #{cap.index} skipped · empty text",
                    stage="TTS",
                    caption_index=cap.index,
                    log_callback=cb,
                )
                continue
            if existing_tts_mode == "skip_existing" and cap.has_existing_tts:
                skipped += 1
                item_results.append(
                    CaptionTtsResult(
                        caption_index=cap.index,
                        text_material_id=cap.text_material_id,
                        text_segment_id=cap.text_segment_id,
                        start_us=cap.start_us,
                        audio_path="",
                        source_duration_us=0,
                        target_duration_us=0,
                        status="skipped",
                        error="existing TTS",
                    )
                )
                continue
            to_generate.append(cap)

        if cancelled():
            return self._cancel_result(project_dir, started)

        self.emit_log(
            "INFO",
            f"Starting TTS for {len(to_generate)} captions",
            stage="Generate",
            log_callback=cb,
        )
        prog(0.05, "tts")

        # Generate via CapCutTtsWrapper — do NOT use project_name copy path;
        # we manage textReading ourselves.
        tts = CapCutTtsWrapper(project_name="")
        items = [{"index": c.index, "text": c.text} for c in to_generate]
        tts_results: dict[int, dict[str, Any]] = {}

        def on_item(idx: int, res: dict[str, Any]):
            tts_results[int(idx)] = res
            status = res.get("status", "")
            dur = float(res.get("duration") or 0.0)
            if status == "Cached":
                self.emit_log(
                    "CACHED",
                    f"Caption #{idx} loaded from cache · duration={dur:.2f}s",
                    stage="TTS",
                    caption_index=idx,
                    log_callback=cb,
                )
            elif status == "Failed":
                self.emit_log(
                    "ERROR",
                    f"Caption #{idx} failed · {res.get('error', 'unknown')}",
                    stage="TTS",
                    caption_index=idx,
                    log_callback=cb,
                )
            else:
                self.emit_log(
                    "SUCCESS",
                    f"Caption #{idx} generated · duration={dur:.2f}s",
                    stage="TTS",
                    caption_index=idx,
                    log_callback=cb,
                )
            if item_completed_callback:
                try:
                    item_completed_callback(int(idx), res)
                except Exception:
                    pass
            # progress 5–75%
            done = len(tts_results)
            total = max(len(to_generate), 1)
            prog(0.05 + 0.70 * (done / total), "tts")

        if to_generate:
            try:
                batch = tts.generate_tts_batch(
                    items=items,
                    voice=voice_type,
                    resource_id=resource_id,
                    rate=float(tts_rate),
                    item_completed_callback=on_item,
                    is_cancelled_callback=is_cancelled_callback,
                    use_cache=use_cache,
                )
                tts_results.update(batch)
            except Exception as e:
                self.emit_log(
                    "ERROR",
                    f"TTS batch failed: {type(e).__name__}: {e}",
                    stage="generating",
                    log_callback=cb,
                )
                logger.error(traceback.format_exc())

        if cancelled():
            return self._cancel_result(project_dir, started)

        # Build CaptionTtsResult list
        generated = cached = failed = 0
        for cap in to_generate:
            res = tts_results.get(cap.index) or {}
            status_raw = res.get("status", "Failed")
            audio_path = res.get("audio_path") or ""
            dur_s = float(res.get("duration") or 0.0)
            source_us = seconds_to_us(dur_s)
            target_us = compute_target_duration_us(
                source_us, capcut_clip_speed, fps=info.fps
            )
            if status_raw == "Cached" and audio_path:
                st = "cached"
                cached += 1
                from_cache = True
            elif status_raw in {"Downloaded", "Generated", "OK"} or (
                audio_path and Path(audio_path).exists()
            ):
                st = "generated"
                generated += 1
                from_cache = False
            else:
                st = "failed"
                failed += 1
                from_cache = False
            item_results.append(
                CaptionTtsResult(
                    caption_index=cap.index,
                    text_material_id=cap.text_material_id,
                    text_segment_id=cap.text_segment_id,
                    start_us=cap.start_us,
                    audio_path=audio_path,
                    source_duration_us=source_us,
                    target_duration_us=target_us,
                    status=st,
                    error=str(res.get("error") or ""),
                    from_cache=from_cache,
                )
            )

        success_items = [r for r in item_results if r.status in {"generated", "cached"}]
        if not success_items:
            self.emit_log("ERROR", "No captions generated successfully", stage="Generate", log_callback=cb)
            return GenerationResult(
                success=False,
                project_path=project_dir,
                backup_path=None,
                selected=len(selected_caps),
                generated=generated,
                cached=cached,
                skipped=skipped,
                failed=failed,
                processing_seconds=time.perf_counter() - started,
                errors=["No captions generated successfully"],
                items=item_results,
            )

        # Always surface pre-commit stats clearly (even when continuing with partial attach)
        self.emit_log(
            "INFO",
            f"Pre-commit stats · generated={generated} · cached={cached} · "
            f"failed={failed} · skipped={skipped} · ok={len(success_items)}",
            stage="Generate",
            log_callback=cb,
        )
        if failed > 0:
            self.emit_log(
                "WARNING",
                f"{failed} caption(s) failed before attach"
                + ("" if allow_partial_attach else " · partial attach disabled"),
                stage="Generate",
                log_callback=cb,
            )

        if not allow_partial_attach and failed > 0:
            self.emit_log(
                "ERROR",
                "Partial attach disabled and some captions failed — not committing",
                stage="Generate",
                log_callback=cb,
            )
            return GenerationResult(
                success=False,
                project_path=project_dir,
                backup_path=None,
                selected=len(selected_caps),
                generated=generated,
                cached=cached,
                skipped=skipped,
                failed=failed,
                processing_seconds=time.perf_counter() - started,
                errors=[
                    f"Partial attach disabled: generated={generated}, cached={cached}, "
                    f"failed={failed}, skipped={skipped}"
                ],
                warnings=[f"{failed} caption(s) failed"],
                items=item_results,
            )

        prog(0.75, "copy_audio")
        self.emit_log("INFO", "Copying audio into project textReading/", stage="copying_audio", log_callback=cb)

        # Deep-copy first so we can reuse/create CapCut draftpath placeholder
        draft = self.reader.get_draft_copy()
        placeholder = ensure_draftpath_placeholder(draft)
        self.emit_log(
            "INFO",
            f"Audio path placeholder: {placeholder}",
            stage="copying_audio",
            log_callback=cb,
        )
        if alignment_settings.enabled:
            trim_ms = int(round(alignment_settings.leading_trim_frames / max(info.fps, 1e-6) * 1000))
            self.emit_log(
                "INFO",
                f"Native alignment on · FPS={info.fps:g} · trim={alignment_settings.leading_trim_frames:g} frames ≈ {trim_ms}ms · fade={alignment_settings.fade_in_ms:g}ms",
                stage="Alignment",
                log_callback=cb,
            )

        audio_rel: dict[int, str] = {}
        new_audio_paths: list[Path] = []
        for r in success_items:
            src = Path(r.audio_path)
            if not src.exists():
                r.status = "failed"
                r.error = "audio file missing after generate"
                failed += 1
                if r.from_cache:
                    cached = max(0, cached - 1)
                else:
                    generated = max(0, generated - 1)
                continue
            short = uuid.uuid4().hex[:8]
            dest_name = f"tts_{r.caption_index:06d}_{short}{src.suffix or '.mp3'}"
            try:
                rel = self.exporter.copy_audio_into_project(
                    src,
                    project_dir,
                    dest_name,
                    draft=draft,
                    placeholder=placeholder,
                    path_style="absolute",
                )
                audio_rel[r.caption_index] = rel
                new_audio_paths.append(project_dir / "textReading" / dest_name)
                r.audio_path = str(project_dir / "textReading" / dest_name)
            except Exception as e:
                r.status = "failed"
                r.error = str(e)
                failed += 1
                self.emit_log(
                    "ERROR",
                    f"Copy failed caption #{r.caption_index}: {e}",
                    stage="copying_audio",
                    caption_index=r.caption_index,
                    log_callback=cb,
                )

        success_items = [r for r in item_results if r.status in {"generated", "cached"} and r.caption_index in audio_rel]
        if not success_items:
            return GenerationResult(
                success=False,
                project_path=project_dir,
                backup_path=None,
                selected=len(selected_caps),
                generated=generated,
                cached=cached,
                skipped=skipped,
                failed=failed,
                processing_seconds=time.perf_counter() - started,
                errors=["No audio files copied"],
                items=item_results,
            )

        # Patch in memory
        prog(0.78, "patch")
        self.emit_log("INFO", "Creating audio materials", stage="Patch", log_callback=cb)
        templates = self.reader.get_tts_templates()
        patcher = DraftPatcher(templates, fps=info.fps, alignment=alignment_settings)
        try:
            patched, stats = patcher.patch(
                draft,
                captions=captions,
                results=item_results,
                voice_type=voice_type,
                resource_id=resource_id,
                voice_display_name=voice_display_name,
                capcut_clip_speed=float(capcut_clip_speed),
                tone_modify_mode=tone_modify_mode,
                existing_tts_mode=existing_tts_mode,
                audio_rel_paths=audio_rel,
                project_directory=project_dir,
            )
        except Exception as e:
            self.emit_log("ERROR", f"Patch failed: {e}", stage="patching", log_callback=cb)
            logger.error(traceback.format_exc())
            for p in new_audio_paths:
                try:
                    p.unlink(missing_ok=True)
                except Exception:
                    pass
            return GenerationResult(
                success=False,
                project_path=project_dir,
                backup_path=None,
                selected=len(selected_caps),
                generated=generated,
                cached=cached,
                skipped=skipped,
                failed=failed,
                processing_seconds=time.perf_counter() - started,
                errors=[f"Patch failed: {e}"],
                items=item_results,
            )

        prog(0.88, "validate")
        self.emit_log("INFO", "Validating patched project", stage="Validate", log_callback=cb)
        missing_new_audio = [
            str(path)
            for path in new_audio_paths
            if not path.exists() or path.stat().st_size <= 0
        ]
        if missing_new_audio:
            for err in missing_new_audio[:20]:
                self.emit_log("ERROR", f"New audio file missing: {err}", stage="Validate", log_callback=cb)
            return GenerationResult(
                success=False,
                project_path=project_dir,
                backup_path=None,
                selected=len(selected_caps),
                generated=generated,
                cached=cached,
                skipped=skipped,
                failed=failed,
                validation_passed=False,
                processing_seconds=time.perf_counter() - started,
                errors=[f"New audio file missing: {p}" for p in missing_new_audio],
                items=item_results,
            )
        errors = self.validator.validate(
            patched,
            project_directory=project_dir,
            require_audio_files=False,
        )
        if errors:
            for err in errors[:20]:
                self.emit_log("ERROR", err, stage="Validate", log_callback=cb)
            for p in new_audio_paths:
                try:
                    p.unlink(missing_ok=True)
                except Exception:
                    pass
            return GenerationResult(
                success=False,
                project_path=project_dir,
                backup_path=None,
                selected=len(selected_caps),
                generated=generated,
                cached=cached,
                skipped=skipped,
                failed=failed,
                validation_passed=False,
                processing_seconds=time.perf_counter() - started,
                errors=errors,
                items=item_results,
            )

        self.emit_log("INFO", "Project integrity passed", stage="Validate", log_callback=cb)
        prog(0.95, "backup")

        manifest = {
            "project_info": {
                "project_id": info.project_id,
                "resolution": f"{info.width}x{info.height}",
                "fps": info.fps,
                "duration_us": info.duration_us,
                "caption_count": info.caption_count,
            },
            "settings": {
                "voice_type": voice_type,
                "resource_id": resource_id,
                "voice_display_name": voice_display_name,
                "tts_rate": tts_rate,
                "capcut_clip_speed": capcut_clip_speed,
                "tone_modify_mode": tone_modify_mode.value,
                "existing_tts_mode": existing_tts_mode,
            },
            "summary": {
                "selected": len(selected_caps),
                "generated": generated,
                "cached": cached,
                "skipped": skipped,
                "failed": failed,
                "attached": len(success_items),
                "audio_tracks": stats.tracks_used,
                "tracks_created": stats.tracks_created,
                "replaced": stats.replaced,
                "alignment_applied": stats.alignment_applied,
                "fades_created": stats.fades_created,
            },
            "alignment": {
                "enabled": alignment_settings.enabled,
                "leading_trim_frames": alignment_settings.leading_trim_frames,
                "fade_in_ms": alignment_settings.fade_in_ms,
            },
            "audio_path_placeholder": placeholder,
            "created_ids_count": len(stats.created_ids),
            "timestamp": datetime.now().isoformat(timespec="seconds"),
        }

        backup_path = None
        written_paths: list[Path] = []
        try:
            backup_path = self.exporter.create_backup(
                draft_path,
                project_dir,
                manifest,
                project_id=info.project_id,
            )
            written_paths = self.exporter.atomic_write_draft(
                draft_path,
                patched,
                project_directory=project_dir,
                project_id=info.project_id,
            )
            for wp in written_paths:
                self.emit_log("INFO", f"Wrote {wp}", stage="Commit", log_callback=cb)
        except Exception as e:
            self.emit_log("ERROR", f"Commit failed: {e}", stage="Commit", log_callback=cb)
            self.exporter.rollback(
                draft_path=draft_path,
                backup_dir=backup_path,
                new_audio_paths=new_audio_paths,
                project_directory=project_dir,
                project_id=info.project_id,
            )
            return GenerationResult(
                success=False,
                project_path=project_dir,
                backup_path=backup_path,
                selected=len(selected_caps),
                generated=generated,
                cached=cached,
                skipped=skipped,
                failed=failed,
                validation_passed=True,
                processing_seconds=time.perf_counter() - started,
                errors=[f"Commit failed and rolled back: {e}"],
                items=item_results,
            )

        # Final reload check
        try:
            self.reader.load_draft(draft_path)
        except Exception as e:
            self.emit_log("WARNING", f"Post-commit reload warning: {e}", stage="Commit", log_callback=cb)

        log_file = self.exporter.write_log_file(project_dir, self._log_lines)
        manifest["log_file"] = str(log_file.relative_to(project_dir)) if log_file.is_relative_to(project_dir) else str(log_file)
        manifest_path = self.exporter.write_manifest(project_dir, manifest)

        elapsed = time.perf_counter() - started
        warn_msgs: list[str] = []
        if failed:
            warn_msgs.append(
                f"{failed} caption(s) failed · attached {len(success_items)}/"
                f"{len(selected_caps)} (generated={generated}, cached={cached}, skipped={skipped})"
            )
        result = GenerationResult(
            success=True,
            project_path=project_dir,
            backup_path=backup_path,
            selected=len(selected_caps),
            generated=generated,
            cached=cached,
            skipped=skipped,
            failed=failed,
            replaced=stats.replaced,
            attached=len(success_items),
            audio_tracks_used=stats.tracks_used,
            audio_tracks_created=stats.tracks_created,
            processing_seconds=elapsed,
            validation_passed=True,
            log_file=log_file,
            manifest_path=manifest_path,
            warnings=warn_msgs,
            items=item_results,
        )

        mins, secs = divmod(int(elapsed), 60)
        self.emit_log(
            "SUCCESS" if failed == 0 else "WARNING",
            "TTS attached successfully" if failed == 0 else "Completed with warnings",
            stage="Complete",
            log_callback=cb,
        )
        self.emit_log("INFO", f"Selected: {result.selected}", stage="Complete", log_callback=cb)
        self.emit_log("INFO", f"Generated: {result.generated}", stage="Complete", log_callback=cb)
        self.emit_log("INFO", f"Cache hits: {result.cached}", stage="Complete", log_callback=cb)
        self.emit_log("INFO", f"Skipped: {result.skipped}", stage="Complete", log_callback=cb)
        self.emit_log("INFO", f"Failed: {result.failed}", stage="Complete", log_callback=cb)
        self.emit_log("INFO", f"Attached: {result.attached}", stage="Complete", log_callback=cb)
        self.emit_log("INFO", f"Audio tracks: {result.audio_tracks_used}", stage="Complete", log_callback=cb)
        self.emit_log(
            "INFO",
            f"Native trim applied: {stats.alignment_applied} · fades: {stats.fades_created} · short-trim reduced: {stats.alignment_trim_reduced}",
            stage="Alignment",
            log_callback=cb,
        )
        self.emit_log("INFO", f"Processing time: {mins:02d}:{secs:02d}", stage="Complete", log_callback=cb)
        self.emit_log("INFO", f"Backup: {backup_path}", stage="Complete", log_callback=cb)
        self.emit_log("INFO", f"Project: {project_dir}", stage="Complete", log_callback=cb)
        if not (
            (project_dir / "draft_meta_info.json").exists()
            or (project_dir / "Timelines").is_dir()
        ):
            self.emit_log(
                "WARNING",
                "Project folder does not look like a CapCut draft root "
                "(missing draft_meta_info.json / Timelines). "
                "Open the CapCut project folder under "
                "AppData\\Local\\CapCut\\User Data\\Projects\\com.lveditor.draft\\… "
                "and generate again, otherwise CapCut will not show the audio.",
                stage="Complete",
                log_callback=cb,
            )
        prog(1.0, "done")
        return result

    def _cancel_result(
        self,
        project_dir: Path,
        started: float,
        *,
        selected: int = 0,
        generated: int = 0,
        cached: int = 0,
        skipped: int = 0,
        failed: int = 0,
        items: list | None = None,
    ) -> GenerationResult:
        self.emit_log("WARNING", "Generation cancelled", stage="cancelled")
        return GenerationResult(
            success=False,
            project_path=project_dir,
            backup_path=None,
            selected=selected,
            generated=generated,
            cached=cached,
            skipped=skipped,
            failed=failed,
            processing_seconds=time.perf_counter() - started,
            errors=["Cancelled by user"],
            items=list(items or []),
            extra={"cancelled": True},
        )

    # Alias required by earlier API sketch
    def generate_project(self, *args, output_directory: Path | None = None, **kwargs) -> GenerationResult:
        """Backward-compatible name — attaches in-place; output_directory ignored."""
        if output_directory is not None:
            self.emit_log(
                "WARNING",
                "output_directory is ignored; TTS attaches into the selected project",
                stage="Settings",
            )
        return self.generate_and_attach(*args, **kwargs)
