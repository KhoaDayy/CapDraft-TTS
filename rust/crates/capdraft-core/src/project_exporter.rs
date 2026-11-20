//! Commit a patched draft into the CapCut project: copy audio, back up, write
//! every target atomically, roll back on failure
//! (port of `core/capcut_project/project_exporter.py`).

use crate::ids::local_now;
use crate::json::{Value, dumps_compact, dumps_indent};
use crate::paths::{
    capcut_text_reading_path, ensure_draftpath_placeholder, list_draft_write_targets, resolve,
};
use crate::project_split::{copy_file, io_err};
use crate::pyjson::text_mode;
use crate::pyval::{PyResult, str_of, truthy};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// How a TTS material's `path` is written into the draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStyle {
    /// `C:/.../textReading/file.mp3` (most reliable).
    Absolute,
    /// `##_draftpath_placeholder_…_##/textReading/file.mp3`.
    Placeholder,
    /// `textReading/file.mp3`.
    Relative,
}

impl PathStyle {
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s {
            "placeholder" => Self::Placeholder,
            "relative" => Self::Relative,
            _ => Self::Absolute,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProjectExporter {
    pub max_backups: usize,
}

impl Default for ProjectExporter {
    fn default() -> Self {
        Self { max_backups: 10 }
    }
}

fn os_err(e: &std::io::Error, p: &Path) -> String {
    io_err(e, p)
}

/// `str(target.relative_to(project_directory))` with separators as `__`, or `fallback`.
fn backup_name(target: &Path, project_directory: &Path, fallback: &str) -> String {
    match target.strip_prefix(project_directory) {
        Ok(rel) => rel.to_string_lossy().replace(['\\', '/'], "__"),
        Err(_) => fallback.to_owned(),
    }
}

/// Write `payload` to `path + ".tmp"`, fsynced; returns the temp path.
fn prepare_bytes_temp(path: &Path, payload: &[u8]) -> std::io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = sibling(path, ".tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(payload)?;
    f.flush()?;
    f.sync_all()?;
    Ok(tmp)
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

fn write_bytes_atomic(path: &Path, payload: &[u8]) -> std::io::Result<()> {
    let tmp = prepare_bytes_temp(path, payload)?;
    fs::rename(tmp, path)
}

fn restore_file_atomic(source: &Path, dest: &Path) -> std::io::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = sibling(dest, ".rollback.tmp");
    let data = fs::read(source)?;
    let mut f = fs::File::create(&tmp)?;
    f.write_all(&data)?;
    f.sync_all()?;
    fs::rename(tmp, dest)
}

impl ProjectExporter {
    #[must_use]
    pub const fn new(max_backups: usize) -> Self {
        Self { max_backups }
    }

    /// `<project>/textReading`, created if needed.
    ///
    /// # Errors
    /// When the folder cannot be created.
    pub fn text_reading_dir(&self, project_directory: &Path) -> PyResult<PathBuf> {
        let d = project_directory.join("textReading");
        fs::create_dir_all(&d).map_err(|e| os_err(&e, &d))?;
        Ok(d)
    }

    fn format_audio_path(
        &self,
        dest: &Path,
        dest_name: &str,
        draft: Option<&Value>,
        placeholder: Option<&str>,
        style: PathStyle,
    ) -> String {
        match style {
            PathStyle::Relative => format!("textReading/{dest_name}"),
            PathStyle::Placeholder => {
                let ph = placeholder.map_or_else(
                    || {
                        draft.map_or_else(
                            || "##_draftpath_placeholder_PLACEHOLDER_##".to_owned(),
                            ensure_draftpath_placeholder,
                        )
                    },
                    str::to_owned,
                );
                capcut_text_reading_path(&ph, dest_name)
            }
            PathStyle::Absolute => resolve(dest).to_string_lossy().replace('\\', "/"),
        }
    }

    fn copy_one(
        &self,
        src: &Path,
        dest_dir: &Path,
        dest_name: &str,
        draft: Option<&Value>,
        placeholder: Option<&str>,
        style: PathStyle,
    ) -> PyResult<String> {
        if !src.exists() {
            return Err(format!("Audio not found: {}", src.display()));
        }
        let dest = dest_dir.join(dest_name);
        if resolve(src) != resolve(&dest) {
            copy_file(src, &dest).map_err(|e| os_err(&e, src))?;
        }
        Ok(self.format_audio_path(&dest, dest_name, draft, placeholder, style))
    }

    /// Copy one audio file into `textReading/`; returns the path CapCut resolves.
    ///
    /// # Errors
    /// `FileNotFoundError: Audio not found: …`, or the copy's error.
    pub fn copy_audio_into_project(
        &self,
        source: &Path,
        project_directory: &Path,
        dest_name: &str,
        draft: Option<&Value>,
        placeholder: Option<&str>,
        style: PathStyle,
    ) -> PyResult<String> {
        if !source.exists() {
            return Err(format!(
                "FileNotFoundError: Audio not found: {}",
                source.display()
            ));
        }
        let dir = self.text_reading_dir(project_directory)?;
        self.copy_one(source, &dir, dest_name, draft, placeholder, style)
    }

    /// Copy many audio files into `textReading/` on all cores.
    /// One `(path, error)` per item, in order.
    #[must_use]
    pub fn copy_audio_batch(
        &self,
        items: &[(PathBuf, String)],
        project_directory: &Path,
        draft: Option<&Value>,
        placeholder: Option<&str>,
        style: PathStyle,
        workers: usize,
    ) -> Vec<(String, Option<String>)> {
        let dir = match self.text_reading_dir(project_directory) {
            Ok(d) => d,
            Err(e) => {
                return items
                    .iter()
                    .map(|_| (String::new(), Some(e.clone())))
                    .collect();
            }
        };
        let one = |(src, name): &(PathBuf, String)| match self.copy_one(
            src,
            &dir,
            name,
            draft,
            placeholder,
            style,
        ) {
            Ok(rel) => (rel, None),
            Err(e) => (String::new(), Some(e)),
        };
        // Sequential on purpose: thousands of small files into one folder copy faster
        // from one thread than from many (they contend on the folder), and a missing
        // placeholder then draws its ids in order. `workers` is kept for the API.
        let _ = workers;
        items.iter().map(one).collect()
    }

    /// Copy every existing write target to `backups/<timestamp>/` with the
    /// manifest, then keep only the newest `max_backups` backups.
    ///
    /// # Errors
    /// When a target cannot be copied or the manifest written.
    pub fn create_backup(
        &self,
        draft_path: &Path,
        project_directory: &Path,
        manifest: &Value,
        project_id: &str,
    ) -> PyResult<PathBuf> {
        let ts = local_now().format("%Y%m%d_%H%M%S").to_string();
        let backup_dir = project_directory.join("backups").join(ts);
        fs::create_dir_all(&backup_dir).map_err(|e| os_err(&e, &backup_dir))?;
        for (i, target) in list_draft_write_targets(draft_path, project_directory, project_id)
            .iter()
            .enumerate()
        {
            if target.exists() {
                let fallback = if i == 0 {
                    "draft_content.json".to_owned()
                } else {
                    format!("draft_content_{i}.json")
                };
                let dest = backup_dir.join(backup_name(target, project_directory, &fallback));
                copy_file(target, &dest).map_err(|e| os_err(&e, target))?;
            }
        }
        let mpath = backup_dir.join("generation_manifest.json");
        fs::write(&mpath, text_mode(&dumps_indent(manifest, 2))).map_err(|e| os_err(&e, &mpath))?;
        self.prune_backups(&project_directory.join("backups"));
        Ok(backup_dir)
    }

    fn prune_backups(&self, root: &Path) {
        let Ok(rd) = fs::read_dir(root) else { return };
        let mut dirs: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        for old in dirs.into_iter().skip(self.max_backups) {
            let _ = fs::remove_dir_all(old);
        }
    }

    /// Write `draft` to every CapCut write target: all temps prepared and
    /// fsynced first (in parallel), pre-images snapshotted, then replaced; a
    /// failure restores what was already replaced. Companion `.bak` and
    /// `template-2.tmp` files follow, best effort.
    ///
    /// # Errors
    /// The first I/O error, after rolling back.
    pub fn atomic_write_draft(
        &self,
        draft_path: &Path,
        draft: &Value,
        project_directory: Option<&Path>,
        project_id: &str,
    ) -> PyResult<Vec<PathBuf>> {
        use rayon::prelude::*;
        let parent = draft_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let project_directory = project_directory.unwrap_or(&parent);
        let pid = if project_id.is_empty() {
            let v = &draft["id"];
            if truthy(v) { str_of(v) } else { String::new() }
        } else {
            project_id.to_owned()
        };
        let targets = list_draft_write_targets(draft_path, project_directory, &pid);
        let payload = dumps_compact(draft).into_bytes();

        let mut prepared: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut preimages: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
        let mut written: Vec<PathBuf> = Vec::new();
        let attempt = (|| -> Result<(), (std::io::Error, PathBuf)> {
            let temps: Vec<Result<PathBuf, (std::io::Error, PathBuf)>> = targets
                .par_iter()
                .map(|t| prepare_bytes_temp(t, &payload).map_err(|e| (e, t.clone())))
                .collect();
            for (t, r) in targets.iter().zip(temps) {
                match r {
                    Ok(tmp) => prepared.push((t.clone(), tmp)),
                    Err(e) => {
                        // keep the temps that were written so they are cleaned up
                        return Err(e);
                    }
                }
            }
            for (t, _) in &prepared {
                if t.exists() {
                    let snap = sibling(t, ".precommit");
                    preimages.push((t.clone(), Some(snap.clone())));
                    copy_file(t, &snap).map_err(|e| (e, t.clone()))?;
                } else {
                    preimages.push((t.clone(), None));
                }
            }
            for (t, tmp) in &prepared {
                fs::rename(tmp, t).map_err(|e| (e, t.clone()))?;
                written.push(t.clone());
            }
            Ok(())
        })();
        if let Err((e, at)) = attempt {
            for t in written.iter().rev() {
                match preimages
                    .iter()
                    .find(|(p, _)| p == t)
                    .and_then(|(_, s)| s.clone())
                {
                    Some(snap) if snap.exists() => {
                        let _ = restore_file_atomic(&snap, t);
                    }
                    None if t.exists() => {
                        let _ = fs::remove_file(t);
                    }
                    _ => {}
                }
            }
            for t in &targets {
                let tmp = sibling(t, ".tmp");
                if tmp.is_file() {
                    let _ = fs::remove_file(tmp);
                }
            }
            for (_, snap) in &preimages {
                if let Some(s) = snap
                    && s.exists()
                {
                    let _ = fs::remove_file(s);
                }
            }
            return Err(os_err(&e, &at));
        }
        for (_, snap) in &preimages {
            if let Some(s) = snap
                && s.exists()
            {
                let _ = fs::remove_file(s);
            }
        }
        let bases: BTreeSet<PathBuf> = written
            .iter()
            .filter_map(|t| t.parent().map(Path::to_path_buf))
            .collect();
        let companions: Vec<PathBuf> = bases
            .iter()
            .flat_map(|b| [b.join("draft_content.json.bak"), b.join("template-2.tmp")])
            .collect();
        companions.par_iter().for_each(|c| {
            let _ = write_bytes_atomic(c, &payload);
        });
        Ok(written)
    }

    /// Restore the drafts from `backup_dir` and delete the audio files this run created.
    /// Never deletes the backup or other project files.
    pub fn rollback(
        &self,
        draft_path: &Path,
        backup_dir: Option<&Path>,
        new_audio_paths: &[PathBuf],
        project_directory: Option<&Path>,
        project_id: &str,
    ) {
        if let Some(b) = backup_dir.filter(|b| b.is_dir()) {
            if let Some(pd) = project_directory {
                for target in list_draft_write_targets(draft_path, pd, project_id) {
                    let fallback = target
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let mut cand = b.join(backup_name(&target, pd, &fallback));
                    if !cand.exists()
                        && target
                            .file_name()
                            .is_some_and(|n| n == "draft_content.json")
                    {
                        let alt = b.join("draft_content.json");
                        if alt.exists() {
                            cand = alt;
                        }
                    }
                    if cand.exists() {
                        let _ = restore_file_atomic(&cand, &target);
                    }
                }
            } else {
                let cand = b.join("draft_content.json");
                if cand.exists() {
                    let _ = restore_file_atomic(&cand, draft_path);
                }
            }
        }
        for p in new_audio_paths {
            if p.exists() {
                let _ = fs::remove_file(p);
            }
        }
    }

    /// `tts_generation_manifest.json` (indent 2).
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn write_manifest(&self, project_directory: &Path, manifest: &Value) -> PyResult<PathBuf> {
        let path = project_directory.join("tts_generation_manifest.json");
        fs::write(&path, text_mode(&dumps_indent(manifest, 2))).map_err(|e| os_err(&e, &path))?;
        Ok(path)
    }

    /// `logs/generation_<timestamp>.log`.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn write_log_file(&self, project_directory: &Path, lines: &[String]) -> PyResult<PathBuf> {
        let dir = project_directory.join("logs");
        fs::create_dir_all(&dir).map_err(|e| os_err(&e, &dir))?;
        let path = dir.join(format!(
            "generation_{}.log",
            local_now().format("%Y%m%d_%H%M%S")
        ));
        let mut text = lines.join("\n");
        if !lines.is_empty() {
            text.push('\n');
        }
        fs::write(&path, text_mode(&text)).map_err(|e| os_err(&e, &path))?;
        Ok(path)
    }
}
