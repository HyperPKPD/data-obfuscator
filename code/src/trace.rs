//! Crash-safe, line-flushed trace log written in real time.

use anyhow::{Context, Result};
use chrono::Utc;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::ui::{self, Stream};

static PANIC_LOG: OnceLock<Mutex<File>> = OnceLock::new();

pub struct TraceLog {
    verbose: bool,
    path: PathBuf,
    file: Mutex<File>,
    warnings: AtomicUsize,
}

impl TraceLog {
    /// Open (or create) the log immediately. Every line is flushed to disk so a
    /// panic or kill still leaves a complete trace up to the last event.
    pub fn open(path: &Path, verbose: bool) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating log directory {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("opening log {}", path.display()))?;
        let panic_clone = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("opening panic log {}", path.display()))?;
        let _ = PANIC_LOG.set(Mutex::new(panic_clone));
        install_panic_hook();

        let log = Self {
            verbose,
            path: path.to_path_buf(),
            file: Mutex::new(file),
            warnings: AtomicUsize::new(0),
        };
        // Every run's section of the log starts with the program banner (plain text).
        if let Ok(mut f) = log.file.lock() {
            let header = format!("\n{}\n", ui::banner_plain(&ui::local_now()));
            let _ = f.write_all(header.as_bytes());
            let _ = f.flush();
        }
        log.raw(
            "INFO",
            &format!(
                "log opened path={} pid={} version={} released={}",
                path.display(),
                std::process::id(),
                crate::APP_VERSION,
                crate::APP_RELEASED
            ),
        );
        Ok(log)
    }

    /// Number of warnings emitted so far (orange lines).
    pub fn warn_count(&self) -> usize {
        self.warnings.load(Ordering::Relaxed)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn info(&self, msg: impl AsRef<str>) {
        self.raw("INFO", msg.as_ref());
    }
    /// A step that completed successfully (green on the terminal).
    pub fn success(&self, msg: impl AsRef<str>) {
        self.raw("OK", msg.as_ref());
    }
    pub fn warn(&self, msg: impl AsRef<str>) {
        self.warnings.fetch_add(1, Ordering::Relaxed);
        self.raw("WARN", msg.as_ref());
    }
    pub fn error(&self, msg: impl AsRef<str>) {
        self.raw("ERROR", msg.as_ref());
    }
    pub fn debug(&self, msg: impl AsRef<str>) {
        if self.verbose {
            self.raw("DEBUG", msg.as_ref());
        }
    }

    fn raw(&self, level: &str, msg: &str) {
        let line = format!("{} [{}] {}\n", stamp(), level, msg);
        eprint!("{}", colourise(level, &line));
        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
            let _ = f.sync_data();
        }
    }
}

/// Terminal colour for a log level: OK green, WARN orange, ERROR/FATAL red.
fn colourise(level: &str, line: &str) -> String {
    let code = match level {
        "OK" => ui::GREEN,
        "WARN" => ui::ORANGE,
        "ERROR" | "FATAL" => ui::RED,
        "DEBUG" => ui::DIM,
        _ => return line.to_string(),
    };
    match line.strip_suffix('\n') {
        Some(body) => ui::paint(Stream::Err, code, body) + "\n",
        None => ui::paint(Stream::Err, code, line),
    }
}

fn stamp() -> String {
    Utc::now().format("%Y-%m-%d %H:%M:%S%.3f UTC").to_string()
}

fn write_panic_line(level: &str, msg: &str) {
    let line = format!("{} [{}] {}\n", stamp(), level, msg);
    eprint!("{}", colourise(level, &line));
    if let Some(lock) = PANIC_LOG.get() {
        if let Ok(mut f) = lock.lock() {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
            let _ = f.sync_data();
        }
    }
}

fn install_panic_hook() {
    static HOOK: OnceLock<()> = OnceLock::new();
    HOOK.get_or_init(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let loc = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_else(|| "unknown".into());
            let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "Box<dyn Any>".into()
            };
            write_panic_line(
                "FATAL",
                &format!("panic at {loc}: {payload} — trace is complete up to this line"),
            );
            default(info);
        }));
    });
}
