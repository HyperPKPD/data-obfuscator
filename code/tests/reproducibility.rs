//! The same input, options and seed must give byte-identical results.

use data_obfuscator::config::{IdStyle, RunOptions};
use data_obfuscator::obfuscate::run;
use data_obfuscator::trace::TraceLog;
use std::fs;
use std::path::PathBuf;

fn run_once(tag: &str, level: u8, style: IdStyle, seed: u64) -> (Vec<u8>, Vec<u8>, String) {
    let dir = std::env::temp_dir().join(format!("do-repro-{}-{tag}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let mut opts = RunOptions::default();
    opts.input = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/sample.csv");
    opts.output_dir = dir.clone();
    opts.level = level;
    opts.id_style = style;
    opts.seed = seed;
    opts.write_keymap = true;
    let mut log = TraceLog::open(&dir.join("obfuscator.log"), false).unwrap();
    let art = run(&opts, &mut log).unwrap();
    let sha = fs::read_to_string(&art.summary_path)
        .unwrap()
        .lines()
        .find(|l| l.trim_start().starts_with("SHA-256:"))
        .expect("summary has a SHA-256 line")
        .to_string();
    (
        fs::read(&art.output_dataset).unwrap(),
        fs::read(art.keymap_path.unwrap()).unwrap(),
        sha,
    )
}

#[test]
fn same_command_same_seed_gives_identical_output() {
    for level in 1..=4u8 {
        for style in [IdStyle::Sequential, IdStyle::Padded, IdStyle::Hashed] {
            let a = run_once(&format!("a{level}{}", style.as_str()), level, style, 42);
            let b = run_once(&format!("b{level}{}", style.as_str()), level, style, 42);
            assert_eq!(a.0, b.0, "dataset differs at level {level} / {}", style.as_str());
            assert_eq!(a.1, b.1, "keymap differs at level {level} / {}", style.as_str());
            assert_eq!(a.2, b.2, "SHA-256 differs at level {level} / {}", style.as_str());
        }
    }
}

#[test]
fn a_different_seed_changes_the_output() {
    let a = run_once("s42", 3, IdStyle::Sequential, 42);
    let b = run_once("s43", 3, IdStyle::Sequential, 43);
    assert_ne!(a.0, b.0);
    assert_ne!(a.2, b.2);
}
