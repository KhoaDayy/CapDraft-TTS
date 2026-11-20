//! App log: console from INFO up, plus an optional UTF-8 file with every level
//! (port of `core/logger.py`; same line layout).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warning,
    Error,
}

impl Level {
    const fn name(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warning => "WARNING",
            Self::Error => "ERROR",
        }
    }
}

static FILE: Mutex<Option<File>> = Mutex::new(None);

/// Send every later line also to `path` (`append` or truncate).
///
/// # Errors
/// When the file cannot be opened.
pub fn setup_file_logger(path: &Path, append: bool) -> std::io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let f = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)?;
    if let Ok(mut g) = FILE.lock() {
        *g = Some(f);
    }
    log(
        Level::Info,
        file!(),
        line!(),
        &format!("File logger redirected to: {}", path.display()),
    );
    Ok(path.to_path_buf())
}

/// `[time] [LEVEL] [file:line] - message`.
pub fn log(level: Level, file: &str, line: u32, msg: &str) {
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S,%3f");
    let name = Path::new(file)
        .file_name()
        .map_or_else(|| file.into(), |n| n.to_string_lossy());
    let text = format!("[{ts}] [{}] [{name}:{line}] - {msg}\n", level.name());
    if level >= Level::Info {
        let _ = std::io::stdout().write_all(text.as_bytes());
    }
    if let Ok(mut g) = FILE.lock()
        && let Some(f) = g.as_mut()
    {
        let _ = f.write_all(text.as_bytes());
    }
}

#[macro_export]
macro_rules! log_info { ($($a:tt)*) => { $crate::logger::log($crate::logger::Level::Info, file!(), line!(), &format!($($a)*)) } }
#[macro_export]
macro_rules! log_warn { ($($a:tt)*) => { $crate::logger::log($crate::logger::Level::Warning, file!(), line!(), &format!($($a)*)) } }
#[macro_export]
macro_rules! log_error { ($($a:tt)*) => { $crate::logger::log($crate::logger::Level::Error, file!(), line!(), &format!($($a)*)) } }
