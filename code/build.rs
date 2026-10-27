//! Build script: automatic 4-part versioning.
//!
//! * `VERSION` holds the current version as `MAJOR.MINOR.PATCH.REVISION`
//!   (it starts at `0.0.0.1`).
//! * `.version-state` remembers a fingerprint of everything that counts as "the
//!   program" (src/, tests/, documentations/, build.rs, Cargo.toml, README.md) and the release date.
//! * Whenever a build sees that fingerprint has changed, REVISION is incremented,
//!   `VERSION` and `.version-state` are rewritten, and the new version is baked into
//!   the binary. Build again without edits and the version stays put.
//!
//! Set `DATA_OBFUSCATOR_NO_BUMP=1` to build without bumping (scratch iterations).
//! If `.version-state` is missing it is created from the current files without a bump.
//!
//! Cargo itself insists on 3-part semver in Cargo.toml, so that field is only a
//! placeholder; the real version users see comes from here.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_VERSION: &str = "0.0.0.1";

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    for p in [
        "src",
        "tests",
        "documentations",
        "build.rs",
        "Cargo.toml",
        "README.md",
        "VERSION",
        ".version-state",
    ] {
        println!("cargo:rerun-if-changed={p}");
    }
    println!("cargo:rerun-if-env-changed=DATA_OBFUSCATOR_NO_BUMP");

    let version_path = root.join("VERSION");
    let state_path = root.join(".version-state");

    let mut version = fs::read_to_string(&version_path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| valid_version(s))
        .unwrap_or_else(|| DEFAULT_VERSION.to_string());

    let hash = fingerprint(&root);
    let today = today_utc();
    let no_bump = env::var("DATA_OBFUSCATOR_NO_BUMP").is_ok_and(|v| !v.is_empty() && v != "0");

    let (stored_hash, stored_date) = read_state(&state_path);
    let mut released = stored_date.unwrap_or_else(|| today.clone());

    match stored_hash {
        None => {
            // First build (or state deleted): adopt the current files as this version.
            write_quiet(&version_path, &format!("{version}\n"));
            write_quiet(&state_path, &render_state(&hash, &released));
        }
        Some(h) if h != hash && !no_bump => {
            version = bump(&version);
            released = today;
            write_quiet(&version_path, &format!("{version}\n"));
            write_quiet(&state_path, &render_state(&hash, &released));
            println!("cargo:warning=data-obfuscator version bumped to {version}");
        }
        Some(_) => {}
    }

    println!("cargo:rustc-env=DATA_OBFUSCATOR_VERSION={version}");
    println!("cargo:rustc-env=DATA_OBFUSCATOR_RELEASED={released}");
}

fn valid_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn bump(v: &str) -> String {
    let mut parts: Vec<u64> = v.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    if let Some(last) = parts.last_mut() {
        *last += 1;
    }
    parts
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(".")
}

fn read_state(path: &Path) -> (Option<String>, Option<String>) {
    let Ok(text) = fs::read_to_string(path) else {
        return (None, None);
    };
    let mut hash = None;
    let mut date = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("fingerprint=") {
            hash = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("released=") {
            date = Some(v.trim().to_string());
        }
    }
    (hash, date)
}

fn render_state(hash: &str, released: &str) -> String {
    format!(
        "# Managed by build.rs - commit this file together with VERSION.\n\
         # A changed fingerprint on the next build bumps the revision in VERSION.\n\
         fingerprint={hash}\nreleased={released}\n"
    )
}

fn write_quiet(path: &Path, body: &str) {
    if let Err(e) = fs::write(path, body) {
        println!("cargo:warning=could not update {}: {e}", path.display());
    }
}

/// FNV-1a over relative path + content (CRLF normalised so Windows/Git agree).
fn fingerprint(root: &Path) -> String {
    let mut files: Vec<PathBuf> = Vec::new();
    for rel in ["build.rs", "Cargo.toml", "README.md"] {
        let p = root.join(rel);
        if p.is_file() {
            files.push(p);
        }
    }
    for dir in ["src", "tests", "documentations"] {
        walk(&root.join(dir), &mut files);
    }
    let mut keyed: Vec<(String, PathBuf)> = files
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            (rel, p)
        })
        .collect();
    keyed.sort();

    let mut h: u64 = 0xcbf29ce484222325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    for (rel, p) in keyed {
        feed(rel.as_bytes());
        feed(&[0]);
        if let Ok(content) = fs::read(&p) {
            let normalised: Vec<u8> = content.into_iter().filter(|&b| b != b'\r').collect();
            feed(&normalised);
        }
        feed(&[0]);
    }
    format!("{h:016x}")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

fn today_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}
