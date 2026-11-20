//! CapCut media path helpers (port of `core/capcut_project/paths.py`).

use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static PLACEHOLDER_RE: LazyLock<Regex> = LazyLock::new(|| {
    // INFALLIBLE: a fixed, valid pattern.
    Regex::new(r"##_draftpath_placeholder_([A-Fa-f0-9-]+)_##").expect("valid regex")
});

const MEDIA_KEYS: [&str; 4] = ["path", "media_path", "reverse_path", "intensifies_path"];

fn placeholder_in(val: Option<&Value>) -> Option<String> {
    let s = val?.as_str()?;
    let caps = PLACEHOLDER_RE.captures(s)?;
    Some(format!("##_draftpath_placeholder_{}_##", &caps[1]))
}

/// The draftpath placeholder already used by a material, else by the
/// top-level `path`.
#[must_use]
pub fn find_draftpath_placeholder(draft: &Value) -> Option<String> {
    if let Some(materials) = draft.get("materials").and_then(Value::as_object) {
        for items in materials.values() {
            let Some(items) = items.as_array() else {
                continue;
            };
            for it in items.iter().filter_map(Value::as_object) {
                for key in MEDIA_KEYS {
                    if let Some(p) = placeholder_in(it.get(key)) {
                        return Some(p);
                    }
                }
            }
        }
    }
    placeholder_in(draft.get("path"))
}

/// The existing placeholder, or a new one with an upper-case UUID.
#[must_use]
pub fn ensure_draftpath_placeholder(draft: &Value) -> String {
    find_draftpath_placeholder(draft).unwrap_or_else(|| {
        format!(
            "##_draftpath_placeholder_{}_##",
            uuid::Uuid::new_v4().to_string().to_uppercase()
        )
    })
}

/// Last component of a path, splitting on `/` and `\` like a Windows `Path.name`.
#[must_use]
pub fn path_name(p: &str) -> &str {
    let trimmed = p.trim_end_matches(['/', '\\']);
    let name = trimmed.rsplit(['/', '\\']).next().unwrap_or("");
    if name == ".." || name == "." {
        ""
    } else {
        name
    }
}

/// Path CapCut resolves relative to the project folder.
#[must_use]
pub fn capcut_text_reading_path(placeholder: &str, dest_name: &str) -> String {
    format!("{placeholder}/textReading/{}", path_name(dest_name))
}

fn resolve_key(p: &Path) -> PathBuf {
    std::fs::canonicalize(p)
        .or_else(|_| std::path::absolute(p))
        .unwrap_or_else(|_| p.to_path_buf())
}

/// The root draft plus each `Timelines/<id>` copy CapCut may load, without
/// duplicates (compared after resolving).
#[must_use]
pub fn list_draft_write_targets(
    draft_path: &Path,
    project_directory: &Path,
    project_id: &str,
) -> Vec<PathBuf> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |p: PathBuf| {
        if seen.insert(resolve_key(&p)) {
            targets.push(p);
        }
    };
    add(draft_path.to_path_buf());
    add(project_directory.join("draft_content.json"));

    let pid = crate::pyfmt::py_strip(project_id);
    let timelines = project_directory.join("Timelines");
    if !pid.is_empty() {
        add(timelines.join(pid).join("draft_content.json"));
        if let Ok(entries) = std::fs::read_dir(&timelines) {
            let want = pid.to_uppercase();
            let mut children: Vec<PathBuf> =
                entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
            children.sort();
            for child in children {
                let matches = child
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.to_uppercase() == want);
                if matches && child.is_dir() {
                    add(child.join("draft_content.json"));
                }
            }
        }
    }
    targets
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn name_handles_windows_separators() {
        assert_eq!(path_name(r"C:\a\b.wav"), "b.wav");
        assert_eq!(path_name("x/"), "x");
        assert_eq!(path_name("../up.wav"), "up.wav");
        assert_eq!(path_name(""), "");
    }

    #[test]
    fn write_targets_dedupe_and_match_case_insensitively() {
        let dir = std::env::temp_dir().join(format!("capdraft-paths-{}", uuid::Uuid::new_v4()));
        let tl = dir.join("Timelines").join("abc-DEF");
        std::fs::create_dir_all(&tl).unwrap();
        let root = dir.join("draft_content.json");
        let got = list_draft_write_targets(&root, &dir, " ABC-def ");
        assert_eq!(
            got,
            vec![
                root.clone(),
                dir.join("Timelines")
                    .join("ABC-def")
                    .join("draft_content.json"),
                tl.join("draft_content.json"),
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
