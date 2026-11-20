//! Is CapCut running? (port of `core/capcut_project/capcut_process.py`, best effort).

use std::process::Command;

#[cfg(windows)]
fn quiet(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn quiet(cmd: &mut Command) -> &mut Command {
    cmd
}

/// `tasklist` on Windows, `ps -A` elsewhere; `false` when the list cannot be read.
#[must_use]
pub fn is_capcut_running() -> bool {
    let (mut cmd, names): (Command, &[&str]) = if cfg!(windows) {
        let mut c = Command::new("tasklist");
        c.args(["/FO", "CSV", "/NH"]);
        (c, &["capcut.exe", "jianyingpro.exe", "videoeditor.exe"])
    } else {
        let mut c = Command::new("ps");
        c.arg("-A");
        (
            c,
            &[
                "capcut.exe",
                "jianyingpro.exe",
                "videoeditor.exe",
                "capcut",
                "jianyingpro",
            ],
        )
    };
    match quiet(&mut cmd).output() {
        Ok(out) if out.status.success() => {
            let lower = String::from_utf8_lossy(&out.stdout).to_lowercase();
            names.iter().any(|n| lower.contains(n))
        }
        _ => false,
    }
}
