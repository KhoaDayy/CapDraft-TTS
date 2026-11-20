//! Concatenate exported videos with ffmpeg, stream copy first
//! (port of `core/capcut_project/video_merge.py`).

use crate::pyval::PyResult;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResult {
    pub output_path: PathBuf,
    pub used_reencode: bool,
    pub ffmpeg_path: String,
}

/// `shutil.which`: an existing file, or a name looked up on `PATH` (with `PATHEXT` on Windows).
#[must_use]
pub fn which(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.BAT;.CMD".into())
            .split(';')
            .map(str::to_lowercase)
            .collect()
    } else {
        vec![String::new()]
    };
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let cand = if ext.is_empty() || name.to_lowercase().ends_with(ext) {
                dir.join(name)
            } else {
                dir.join(format!("{name}{ext}"))
            };
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// The ffmpeg to run: `explicit`, else `PATH`, else next to `ffprobe`.
///
/// # Errors
/// `FileNotFoundError` with the original's hint.
pub fn resolve_ffmpeg(explicit: Option<&str>, ffprobe: Option<&str>) -> PyResult<String> {
    let mut cands: Vec<String> = Vec::new();
    if let Some(e) = explicit.map(str::trim).filter(|e| !e.is_empty()) {
        cands.push(e.to_owned());
    }
    if let Some(w) = which("ffmpeg") {
        cands.push(w.to_string_lossy().into_owned());
    }
    if let Some(probe) = ffprobe.filter(|p| !p.is_empty()) {
        let p = Path::new(probe);
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.to_lowercase().starts_with("ffprobe") {
            cands.push(
                p.with_file_name(
                    name.replace("ffprobe", "ffmpeg")
                        .replace("FFPROBE", "ffmpeg"),
                )
                .to_string_lossy()
                .into_owned(),
            );
        }
        let parent = if p.is_dir() {
            p.to_path_buf()
        } else {
            p.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        cands.push(parent.join("ffmpeg.exe").to_string_lossy().into_owned());
        cands.push(parent.join("ffmpeg").to_string_lossy().into_owned());
    }
    for c in cands.iter().filter(|c| !c.is_empty()) {
        let path = Path::new(c);
        if path.is_file() {
            return Ok(crate::paths::resolve(path).to_string_lossy().into_owned());
        }
        if let Some(w) = which(c) {
            return Ok(w.to_string_lossy().into_owned());
        }
    }
    Err("FileNotFoundError: ffmpeg not found. Install ffmpeg and ensure it is on PATH, or set ffmpeg_path in config.json.".into())
}

fn run(ff: &str, args: &[String]) -> std::io::Result<std::process::Output> {
    let mut cmd = Command::new(ff);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd.output()
}

/// Concatenate `inputs` in order with the concat demuxer: `-c copy` first, and
/// on failure a libx264 + aac re-encode (or only that, with `reencode`).
///
/// # Errors
/// `ValueError` for fewer than two inputs, `FileNotFoundError` for a missing
/// input or ffmpeg, `RuntimeError` with ffmpeg's last output when it fails.
pub fn merge_videos(
    inputs: &[PathBuf],
    output: &Path,
    ffmpeg_path: Option<&str>,
    ffprobe_path: Option<&str>,
    reencode: bool,
    progress: &mut dyn FnMut(f64, &str),
) -> PyResult<MergeResult> {
    if inputs.len() < 2 {
        return Err("ValueError: Need at least 2 videos to merge".into());
    }
    for p in inputs {
        if !p.is_file() {
            return Err(format!(
                "FileNotFoundError: Video not found: {}",
                p.display()
            ));
        }
    }
    let ext = output
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let out = if matches!(ext.as_str(), "mp4" | "mov" | "mkv" | "m4v") {
        output.to_path_buf()
    } else {
        output.with_extension("mp4")
    };
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| crate::project_split::io_err(&e, parent))?;
    }
    let ff = resolve_ffmpeg(ffmpeg_path, ffprobe_path)?;

    progress(0.1, "Preparing concat list…");
    let list = std::env::temp_dir().join(format!("capdraft_concat_{}.txt", crate::ids::uuid_hex()));
    let result = (|| -> PyResult<MergeResult> {
        let lines: Vec<String> = inputs
            .iter()
            .map(|p| {
                let ap = crate::paths::resolve(p)
                    .to_string_lossy()
                    .replace('\\', "/")
                    .replace('\'', r"'\''");
                format!("file '{ap}'")
            })
            .collect();
        std::fs::write(&list, lines.join("\n") + "\n")
            .map_err(|e| crate::project_split::io_err(&e, &list))?;
        let concat = |copy: bool| {
            let mut args: Vec<String> = ["-y", "-f", "concat", "-safe", "0", "-i"]
                .iter()
                .map(|s| (*s).to_owned())
                .collect();
            args.push(list.to_string_lossy().into_owned());
            let tail: &[&str] = if copy {
                &["-c", "copy"]
            } else {
                &[
                    "-c:v",
                    "libx264",
                    "-preset",
                    "veryfast",
                    "-crf",
                    "18",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "192k",
                    "-movflags",
                    "+faststart",
                ]
            };
            args.extend(tail.iter().map(|s| (*s).to_owned()));
            args.push(out.to_string_lossy().into_owned());
            run(&ff, &args)
        };
        let ok = |o: &std::process::Output, min: u64| {
            o.status.success()
                && std::fs::metadata(&out).is_ok_and(|m| m.is_file() && m.len() >= min)
        };
        let mut used_reencode = reencode;
        let mut res = if reencode {
            progress(0.3, "Merging (re-encode)…");
            concat(false)
        } else {
            progress(0.3, "Merging (stream copy)…");
            let r = concat(true);
            if r.as_ref().map_or(true, |o| !ok(o, 32)) {
                progress(0.5, "Stream copy failed — re-encoding…");
                let _ = std::fs::remove_file(&out);
                used_reencode = true;
                concat(false)
            } else {
                r
            }
        };
        let out_ok = res.as_ref().is_ok_and(|o| ok(o, 0));
        if !out_ok {
            let text = match res.as_mut() {
                // `stderr or stdout or "ffmpeg failed"`, then strip
                Ok(o) if !o.stderr.is_empty() => String::from_utf8_lossy(&o.stderr).into_owned(),
                Ok(o) if !o.stdout.is_empty() => String::from_utf8_lossy(&o.stdout).into_owned(),
                Ok(_) => "ffmpeg failed".to_owned(),
                Err(e) => e.to_string(),
            };
            let t = text.trim();
            let chars: Vec<char> = t.chars().collect();
            let tail: String = chars[chars.len().saturating_sub(2000)..].iter().collect();
            return Err(format!("RuntimeError: {tail}"));
        }
        progress(1.0, "Merge complete");
        Ok(MergeResult {
            output_path: out.clone(),
            used_reencode,
            ffmpeg_path: ff.clone(),
        })
    })();
    let _ = std::fs::remove_file(&list);
    result
}
