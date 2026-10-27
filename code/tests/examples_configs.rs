//! Every shipped example config must carry a header and run cleanly.

use data_obfuscator::config::{apply_map, parse_config_file, RunOptions};
use data_obfuscator::obfuscate::run;
use data_obfuscator::trace::TraceLog;
use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn example_configs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(root().join("examples/configs"))
        .expect("examples/configs exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("txt"))
        .collect();
    v.push(root().join("examples/config.txt"));
    v.sort();
    v
}

/// Which command a config is for, read from its "Run it" header line (`data-obfuscator run2 --config ...`).
/// Configs of `run2` are checked in tests/adam_examples.rs; this file runs the `run` ones.
fn is_run2_config(f: &PathBuf) -> bool {
    fs::read_to_string(f)
        .unwrap()
        .lines()
        .take(40)
        .filter(|l| l.starts_with('#'))
        .any(|l| l.contains("data-obfuscator run2 "))
}

#[test]
fn every_example_config_has_a_header_the_parser_ignores() {
    let files = example_configs();
    assert!(files.len() >= 21, "expected the full set of example configs, found {}", files.len());
    for f in files {
        let text = fs::read_to_string(&f).unwrap();
        let first_lines: Vec<&str> = text.lines().take(12).collect();
        assert!(first_lines.iter().all(|l| l.starts_with('#')), "{} header must be comments", f.display());
        for needle in ["Program", "Version", "Author", "Obinna Obianom", "Date", "Purpose"] {
            assert!(text.contains(needle), "{} header lacks '{needle}'", f.display());
        }
        // Parsing must not trip over the header and must still find the real options.
        let map = parse_config_file(&f).unwrap_or_else(|e| panic!("{}: {e:#}", f.display()));
        assert!(map.contains_key("input") && map.contains_key("output"), "{} lost its options", f.display());
        if is_run2_config(&f) {
            continue; // run2 options are validated by running them (tests/adam_examples.rs)
        }
        let mut opts = RunOptions::default();
        apply_map(&mut opts, &map).unwrap_or_else(|e| panic!("{}: {e:#}", f.display()));
    }
}

#[test]
fn every_example_config_runs_on_the_sample_data() {
    for f in example_configs() {
        if is_run2_config(&f) {
            continue;
        }
        // examples/config.txt expects to be run from examples/ and has its own relative paths.
        let relative_to_examples = f.file_name().and_then(|s| s.to_str()) == Some("config.txt");
        let base = if relative_to_examples { root().join("examples") } else { root() };
        let map = parse_config_file(&f).unwrap();
        let mut opts = RunOptions::default();
        apply_map(&mut opts, &map).unwrap();
        if opts.input.is_relative() {
            opts.input = base.join(&opts.input);
        }
        let out = std::env::temp_dir().join(format!(
            "do-examples-{}-{}",
            std::process::id(),
            f.file_stem().and_then(|s| s.to_str()).unwrap_or("x")
        ));
        opts.output_dir = out.clone();
        fs::create_dir_all(&out).unwrap();
        let mut log = TraceLog::open(&out.join("obfuscator.log"), false).unwrap();
        let art = run(&opts, &mut log).unwrap_or_else(|e| panic!("{}: {e:#}", f.display()));
        assert!(art.output_dataset.exists(), "{}", f.display());
        assert!(art.llm_path.exists(), "{}", f.display());
    }
}
