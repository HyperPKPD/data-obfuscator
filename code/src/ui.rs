//! Terminal presentation: colours, the program banner and the `--version` text.
//!
//! Colour scheme: success = green, warning = orange, failure/error = red.
//! Colour is only used on interactive terminals (never in log files or pipes) and is
//! disabled by `--no-color` or the `NO_COLOR` environment variable.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{APP_NAME, APP_RELEASED, APP_VERSION};

pub const RESET: &str = "\x1b[0m";
pub const GREEN: &str = "\x1b[1;32m";
pub const ORANGE: &str = "\x1b[38;5;208m";
pub const RED: &str = "\x1b[1;31m";
pub const DIM: &str = "\x1b[2m";
const FRAME: &str = "\x1b[38;5;39m";
const TITLE: &str = "\x1b[1;38;5;81m";
const ACCENT: &str = "\x1b[38;5;220m";

pub const TAGLINE: &str = "Mask NONMEM-style datasets - keep the PK/PD structure intact";
pub const AUDIENCE: &str = "For Clinical Pharmacology, Pharmacometrics & Data Science";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
}

static DISABLED: AtomicBool = AtomicBool::new(false);
static OUT_VT: AtomicBool = AtomicBool::new(true);
static ERR_VT: AtomicBool = AtomicBool::new(true);

/// Call once at start-up. Enables ANSI processing on Windows consoles.
pub fn init(no_color_flag: bool) {
    if no_color_flag || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        DISABLED.store(true, Ordering::Relaxed);
    }
    if std::env::var("TERM").is_ok_and(|t| t == "dumb") {
        DISABLED.store(true, Ordering::Relaxed);
    }
    OUT_VT.store(enable_vt(Stream::Out), Ordering::Relaxed);
    ERR_VT.store(enable_vt(Stream::Err), Ordering::Relaxed);
}

pub fn use_color(stream: Stream) -> bool {
    if DISABLED.load(Ordering::Relaxed) {
        return false;
    }
    match stream {
        Stream::Out => std::io::stdout().is_terminal() && OUT_VT.load(Ordering::Relaxed),
        Stream::Err => std::io::stderr().is_terminal() && ERR_VT.load(Ordering::Relaxed),
    }
}

pub fn paint(stream: Stream, code: &str, text: &str) -> String {
    if use_color(stream) {
        format!("{code}{text}{RESET}")
    } else {
        text.to_string()
    }
}

pub fn green(stream: Stream, text: &str) -> String {
    paint(stream, GREEN, text)
}
pub fn orange(stream: Stream, text: &str) -> String {
    paint(stream, ORANGE, text)
}
pub fn red(stream: Stream, text: &str) -> String {
    paint(stream, RED, text)
}

#[cfg(windows)]
fn enable_vt(stream: Stream) -> bool {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(n: u32) -> *mut c_void;
        fn GetConsoleMode(h: *mut c_void, mode: *mut u32) -> i32;
        fn SetConsoleMode(h: *mut c_void, mode: u32) -> i32;
    }
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    let id = match stream {
        Stream::Out => STD_OUTPUT_HANDLE,
        Stream::Err => STD_ERROR_HANDLE,
    };
    // SAFETY: plain Win32 console calls on the process's own standard handles.
    unsafe {
        let h = GetStdHandle(id);
        if h.is_null() || h as isize == -1 {
            return false;
        }
        let mut mode = 0u32;
        if GetConsoleMode(h, &mut mode) == 0 {
            // Not a console (redirected): colour is off anyway via is_terminal().
            return true;
        }
        if mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0 {
            return true;
        }
        SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

#[cfg(not(windows))]
fn enable_vt(_stream: Stream) -> bool {
    true
}

/// "2026-10-07 23:54:12 -04:00" in the user's local time zone.
pub fn local_now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S %:z").to_string()
}

const INNER: usize = 68;

/// Text padded to the banner's inner width (two spaces of margin on each side).
fn cell(text: &str) -> String {
    let n = text.chars().count();
    format!("  {text}{}  ", " ".repeat(INNER.saturating_sub(n)))
}

fn title_text() -> String {
    let left = "DATA OBFUSCATOR";
    let ver = format!("v{APP_VERSION}");
    let gap = INNER.saturating_sub(left.chars().count() + ver.chars().count());
    format!("{left}{}{ver}", " ".repeat(gap))
}

fn released_text(run_stamp: &str) -> String {
    format!("Released {APP_RELEASED}   |   Run {run_stamp}")
}

/// Plain-text banner (used for the log file and when colour is off).
pub fn banner_plain(run_stamp: &str) -> String {
    let bar = "═".repeat(INNER + 4);
    let mut s = String::new();
    s.push_str(&format!("╔{bar}╗\n"));
    for t in [title_text(), TAGLINE.to_string(), AUDIENCE.to_string(), released_text(run_stamp)] {
        s.push_str(&format!("║{}║\n", cell(&t)));
    }
    s.push_str(&format!("╚{bar}╝\n"));
    s
}

/// Coloured banner when the stream supports it, otherwise the plain one.
pub fn banner(stream: Stream, run_stamp: &str) -> String {
    if !use_color(stream) {
        return banner_plain(run_stamp);
    }
    let bar = "═".repeat(INNER + 4);
    let edge = format!("{FRAME}║{RESET}");
    let mut s = String::new();
    s.push_str(&format!("{FRAME}╔{bar}╗{RESET}\n"));
    let rows: [(&str, String); 4] = [
        (TITLE, title_text()),
        ("", TAGLINE.to_string()),
        (ACCENT, AUDIENCE.to_string()),
        (DIM, released_text(run_stamp)),
    ];
    for (code, text) in rows {
        let body = cell(&text);
        if code.is_empty() {
            s.push_str(&format!("{edge}{body}{edge}\n"));
        } else {
            s.push_str(&format!("{edge}{code}{body}{RESET}{edge}\n"));
        }
    }
    s.push_str(&format!("{FRAME}╚{bar}╝{RESET}\n"));
    s
}

/// Everything printed by `--version`.
pub fn version_text(stream: Stream) -> String {
    let mut s = banner(stream, &local_now());
    let h = |t: &str| paint(stream, TITLE, t);
    s.push('\n');
    s.push_str(&format!("{}\n", h("WHAT IT DOES")));
    s.push_str(
        "  Recodes a NONMEM-style pharmacometric dataset so it can be shown to an LLM or a\n\
         \x20 collaborator WITHOUT exposing source identities (subjects, studies, sites,\n\
         \x20 treatment labels, dates, free text), while keeping the analysis structure\n\
         \x20 intact: TIME, TAD, DV, AMT, RATE, EVID, MDV, CMT, ADDL, II and SS are never\n\
         \x20 altered and within-subject time order is preserved.\n\n",
    );
    s.push_str(&format!("{}\n", h("WHO IT IS FOR")));
    s.push_str(
        "  Clinical pharmacologists, pharmacometricians and data scientists who want an\n\
         \x20 LLM's help with exploratory analysis, NONMEM control streams and data QC on\n\
         \x20 clinical PK/PD datasets without handing over the real identifiers.\n\n",
    );
    s.push_str(&format!("{}\n", h("ALSO WRITES")));
    s.push_str(
        "  llm.txt     ready-to-paste prompts (analysis, NONMEM control stream, QC)\n\
         \x20             tailored to the masked dataset\n\
         \x20 tokens.txt  estimated token counts for GPT, Claude, Gemini, Llama and others\n\n",
    );
    s.push_str(&format!("{}\n", h("BUILD")));
    s.push_str(&format!("  {APP_NAME} {APP_VERSION}  (released {APP_RELEASED})\n"));
    s.push_str("  Run `data-obfuscator --help` for commands and options.\n");
    s.push_str(&format!(
        "  {}\n",
        paint(
            stream,
            DIM,
            "Masking reduces re-identification risk; it is not a formal anonymisation guarantee."
        )
    ));
    s
}
