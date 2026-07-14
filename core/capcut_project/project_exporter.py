"""Atomic commit of patched draft into the selected CapCut project."""

from __future__ import annotations

import json
import os
import shutil
from datetime import datetime
from pathlib import Path
from typing import Any

from core.logger import logger
from .paths import ensure_draftpath_placeholder, capcut_text_reading_path, list_draft_write_targets


class ProjectExporter:
    def __init__(self, *, max_backups: int = 10):
        self.max_backups = max_backups

    def text_reading_dir(self, project_directory: Path) -> Path:
        d = project_directory / "textReading"
        d.mkdir(parents=True, exist_ok=True)
        return d

    def copy_audio_into_project(
        self,
        source_path: Path | str,
        project_directory: Path,
        dest_name: str,
        *,
        draft: dict[str, Any] | None = None,
        placeholder: str | None = None,
        path_style: str = "absolute",
    ) -> str:
        """Copy audio into project/textReading; return path CapCut can resolve.

        path_style:
          - absolute: C:/.../textReading/file.mp3 (most reliable; matches video materials)
          - placeholder: ##_draftpath_placeholder_...##/textReading/file.mp3
          - relative: textReading/file.mp3
        """
        src = Path(source_path)
        if not src.exists():
            raise FileNotFoundError(f"Audio not found: {src}")
        dest_dir = self.text_reading_dir(project_directory)
        dest = dest_dir / dest_name
        if src.resolve() != dest.resolve():
            shutil.copy2(src, dest)

        return self._format_audio_path(
            dest, dest_name, draft=draft, placeholder=placeholder, path_style=path_style
        )

    def copy_audio_batch(
        self,
        items: list[tuple[Path | str, str]],
        project_directory: Path,
        *,
        draft: dict[str, Any] | None = None,
        placeholder: str | None = None,
        path_style: str = "absolute",
        workers: int = 8,
    ) -> list[tuple[str, str | None]]:
        """Copy many audio files into textReading/ in parallel.

        items: list of (source_path, dest_name)
        returns: list of (rel_path_or_empty, error_or_None) aligned with items
        """
        from concurrent.futures import ThreadPoolExecutor, as_completed

        dest_dir = self.text_reading_dir(project_directory)
        results: list[tuple[str, str | None]] = [("", "not started")] * len(items)
        if not items:
            return results

        def _one(i: int, source_path: Path | str, dest_name: str) -> tuple[int, str, str | None]:
            try:
                src = Path(source_path)
                if not src.exists():
                    return i, "", f"Audio not found: {src}"
                dest = dest_dir / dest_name
                if src.resolve() != dest.resolve():
                    shutil.copy2(src, dest)
                rel = self._format_audio_path(
                    dest,
                    dest_name,
                    draft=draft,
                    placeholder=placeholder,
                    path_style=path_style,
                )
                return i, rel, None
            except Exception as e:
                return i, "", str(e)

        max_workers = max(1, min(int(workers or 1), len(items), 32))
        if max_workers == 1 or len(items) == 1:
            for i, (src, name) in enumerate(items):
                idx, rel, err = _one(i, src, name)
                results[idx] = (rel, err)
            return results

        with ThreadPoolExecutor(max_workers=max_workers) as pool:
            futs = [
                pool.submit(_one, i, src, name) for i, (src, name) in enumerate(items)
            ]
            for fut in as_completed(futs):
                idx, rel, err = fut.result()
                results[idx] = (rel, err)
        return results

    def _format_audio_path(
        self,
        dest: Path,
        dest_name: str,
        *,
        draft: dict[str, Any] | None,
        placeholder: str | None,
        path_style: str,
    ) -> str:
        # CapCut on Windows prefers forward slashes in material paths
        abs_path = str(dest.resolve()).replace("\\", "/")
        if path_style == "relative":
            return f"textReading/{dest_name}"
        if path_style == "placeholder":
            if placeholder is None:
                placeholder = (
                    ensure_draftpath_placeholder(draft)
                    if draft is not None
                    else "##_draftpath_placeholder_PLACEHOLDER_##"
                )
            return capcut_text_reading_path(placeholder, dest_name)
        # default absolute — silent clips usually mean CapCut failed to resolve placeholder/relative
        return abs_path

    def create_backup(
        self,
        draft_path: Path,
        project_directory: Path,
        manifest: dict[str, Any],
        *,
        project_id: str = "",
    ) -> Path:
        ts = datetime.now().strftime("%Y%m%d_%H%M%S")
        backup_dir = project_directory / "backups" / ts
        backup_dir.mkdir(parents=True, exist_ok=True)
        # backup every write target that exists
        for i, target in enumerate(list_draft_write_targets(draft_path, project_directory, project_id)):
            if target.exists():
                name = "draft_content.json" if i == 0 else f"draft_content_{i}.json"
                # keep path-relative name structure
                rel = target
                try:
                    rel_name = str(target.relative_to(project_directory)).replace("\\", "__").replace("/", "__")
                except ValueError:
                    rel_name = name
                shutil.copy2(target, backup_dir / rel_name)
        with open(backup_dir / "generation_manifest.json", "w", encoding="utf-8") as f:
            json.dump(manifest, f, ensure_ascii=False, indent=2)
        logger.info("[Backup] Backup created: %s", backup_dir)
        self._prune_backups(project_directory / "backups")
        return backup_dir

    def _prune_backups(self, backups_root: Path):
        if not backups_root.is_dir():
            return
        dirs = sorted(
            [p for p in backups_root.iterdir() if p.is_dir()],
            key=lambda p: p.name,
            reverse=True,
        )
        for old in dirs[self.max_backups :]:
            try:
                shutil.rmtree(old)
            except Exception as e:
                logger.warning("Could not prune backup %s: %s", old, e)

    def _prepare_json_temp(self, path: Path, draft: dict[str, Any]) -> Path:
        """Write fsynced temp sibling; caller replaces after all temps ready."""
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary_path = path.with_name(path.name + ".tmp")
        with temporary_path.open("w", encoding="utf-8") as file:
            json.dump(draft, file, ensure_ascii=False, separators=(",", ":"))
            file.flush()
            os.fsync(file.fileno())
        return temporary_path

    def atomic_write_json(self, path: Path, draft: dict[str, Any]) -> None:
        temporary_path = self._prepare_json_temp(path, draft)
        os.replace(temporary_path, path)

    def atomic_write_draft(
        self,
        draft_path: Path,
        draft: dict[str, Any],
        *,
        project_directory: Path | None = None,
        project_id: str = "",
    ) -> list[Path]:
        """Prepare+fsync all temps, then replace. On mid-commit failure, restore already-replaced targets.

        Serializes the draft JSON once, then fans the bytes out to every CapCut
        write target (root + Timelines copies) — 5k-caption drafts are multi-MB.
        """
        project_directory = project_directory or draft_path.parent
        targets = list_draft_write_targets(
            draft_path, project_directory, project_id or str(draft.get("id") or "")
        )
        payload = json.dumps(draft, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        prepared: list[tuple[Path, Path]] = []  # (target, temp)
        preimages: dict[Path, Path | None] = {}
        written: list[Path] = []
        try:
            for target in targets:
                temp = self._prepare_bytes_temp(target, payload)
                prepared.append((target, temp))
            # Snapshot existing content for exact rollback of replaced targets
            for target, _temp in prepared:
                if target.exists():
                    snap = target.with_name(target.name + ".precommit")
                    shutil.copy2(target, snap)
                    preimages[target] = snap
                else:
                    preimages[target] = None
            for target, temp in prepared:
                os.replace(temp, target)
                written.append(target)
                logger.info("[Commit] Updated CapCut draft: %s", target)
        except Exception:
            # Rollback already-replaced targets from preimages (temp+os.replace)
            for target in reversed(written):
                snap = preimages.get(target)
                try:
                    if snap is not None and snap.exists():
                        self._restore_file_atomic(snap, target)
                        logger.info("[Rollback] Restored %s from precommit snapshot", target)
                    elif target.exists() and preimages.get(target) is None:
                        target.unlink()
                except Exception as e:
                    logger.warning("[Rollback] Failed restore %s: %s", target, e)
            # Drop any leftover temps / preimages
            for _target, temp in prepared:
                try:
                    if temp.exists():
                        temp.unlink()
                except Exception:
                    pass
            for snap in preimages.values():
                if snap is None:
                    continue
                try:
                    if snap.exists():
                        snap.unlink()
                except Exception:
                    pass
            raise
        else:
            for snap in preimages.values():
                if snap is None:
                    continue
                try:
                    if snap.exists():
                        snap.unlink()
                except Exception:
                    pass

        # Companion files CapCut sometimes reloads (best-effort; not part of multi-target atomic set)
        # Write once per unique parent dir.
        for base in {t.parent for t in written}:
            for companion in ("draft_content.json.bak", "template-2.tmp"):
                cpath = base / companion
                try:
                    self._write_bytes_atomic(cpath, payload)
                except Exception as e:
                    logger.debug("Companion write skipped %s: %s", cpath, e)
        return written

    def _prepare_bytes_temp(self, path: Path, payload: bytes) -> Path:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary_path = path.with_name(path.name + ".tmp")
        with temporary_path.open("wb") as file:
            file.write(payload)
            file.flush()
            os.fsync(file.fileno())
        return temporary_path

    def _write_bytes_atomic(self, path: Path, payload: bytes) -> None:
        temporary_path = self._prepare_bytes_temp(path, payload)
        os.replace(temporary_path, path)

    def _restore_file_atomic(self, source: Path, dest: Path) -> None:
        dest.parent.mkdir(parents=True, exist_ok=True)
        temporary_path = dest.with_name(dest.name + ".rollback.tmp")
        # Copy via write handle so fsync works on Windows (rb + fsync → EBADF)
        with open(source, "rb") as src, open(temporary_path, "wb") as dst:
            shutil.copyfileobj(src, dst)
            dst.flush()
            os.fsync(dst.fileno())
        os.replace(temporary_path, dest)

    def rollback(
        self,
        *,
        draft_path: Path,
        backup_dir: Path | None,
        new_audio_paths: list[Path],
        project_directory: Path | None = None,
        project_id: str = "",
        written_paths: list[Path] | None = None,
    ) -> None:
        """Restore drafts from backup via temp+os.replace. Never deletes backup/project."""
        if backup_dir and backup_dir.is_dir():
            if project_directory:
                for target in list_draft_write_targets(draft_path, project_directory, project_id):
                    try:
                        rel_name = str(target.relative_to(project_directory)).replace("\\", "__").replace("/", "__")
                    except ValueError:
                        rel_name = target.name
                    cand = backup_dir / rel_name
                    if not cand.exists() and target.name == "draft_content.json":
                        # legacy backup name for primary
                        alt = backup_dir / "draft_content.json"
                        if alt.exists():
                            cand = alt
                    if cand.exists():
                        try:
                            self._restore_file_atomic(cand, target)
                            logger.info("[Rollback] Restored %s", target)
                        except Exception as e:
                            logger.warning("[Rollback] Failed %s: %s", target, e)
            elif draft_path:
                cand = backup_dir / "draft_content.json"
                if cand.exists():
                    try:
                        self._restore_file_atomic(cand, draft_path)
                        logger.info("[Rollback] Restored %s", draft_path)
                    except Exception as e:
                        logger.warning("[Rollback] Failed %s: %s", draft_path, e)
        # Only remove audio we just created — never touch backups or user project assets
        for p in new_audio_paths:
            try:
                if p.exists():
                    p.unlink()
            except Exception as e:
                logger.warning("[Rollback] Could not remove %s: %s", p, e)

    def write_manifest(self, project_directory: Path, manifest: dict[str, Any]) -> Path:
        path = project_directory / "tts_generation_manifest.json"
        with open(path, "w", encoding="utf-8") as f:
            json.dump(manifest, f, ensure_ascii=False, indent=2)
        return path

    def write_log_file(self, project_directory: Path, lines: list[str]) -> Path:
        log_dir = project_directory / "logs"
        log_dir.mkdir(parents=True, exist_ok=True)
        ts = datetime.now().strftime("%Y%m%d_%H%M%S")
        path = log_dir / f"generation_{ts}.log"
        with open(path, "w", encoding="utf-8") as f:
            f.write("\n".join(lines) + ("\n" if lines else ""))
        return path
