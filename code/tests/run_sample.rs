use data_obfuscator::config::RunOptions;
use data_obfuscator::obfuscate::{run, run_tokens};
use data_obfuscator::trace::TraceLog;
use std::fs;
use std::path::PathBuf;

fn tmp() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "data-obfuscator-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn run_writes_required_outputs_and_preserves_dv() {
    let dir = tmp();
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/sample.csv");
    assert!(input.exists(), "examples/sample.csv must exist");
    let original = fs::read_to_string(&input).unwrap();
    let mut opts = RunOptions::default();
    opts.input = input;
    opts.output_dir = dir.clone();
    opts.level = 2;
    opts.seed = 42;
    opts.keep = Some(
        ["ID", "TIME", "AMT", "DV", "EVID", "ARM", "STUDYID"]
            .into_iter()
            .map(|s| s.to_string())
            .collect(),
    );
    let log_path = dir.join("obfuscator.log");
    let mut log = TraceLog::open(&log_path, true).unwrap();
    let art = run(&opts, &mut log).unwrap();
    assert!(art.output_dataset.exists());
    assert!(art.summary_path.exists());
    assert!(art.tokens_path.exists());
    assert!(dir.join("column_report.txt").exists());
    assert!(art.llm_path.exists(), "llm.txt must be written");
    let llm = fs::read_to_string(&art.llm_path).unwrap();
    for section in [
        "1. ANALYZE THE DATA",
        "2. CREATE A NONMEM CONTROL STREAM",
        "3. PERFORM QUALITY CONTROL CHECKS",
    ] {
        assert!(llm.contains(section), "llm.txt missing section {section}");
    }
    assert!(llm.contains("20 subjects"), "prompts must carry dataset facts");
    assert!(!llm.contains("ABC-PK-2024"), "llm.txt must not leak source identifiers");
    assert!(dir.join("config.used.txt").exists());
    let out = fs::read_to_string(&art.output_dataset).unwrap();
    assert!(out.lines().next().unwrap().contains("ID"));
    assert!(!out.contains("ABC-PK-2024"), "study name must be recoded");
    assert!(!out.contains("Placebo"), "arm labels must be recoded at level 2");
    // DV values from the original that are numeric should still appear
    let orig_dv: Vec<&str> = original
        .lines()
        .skip(1)
        .filter_map(|l| l.split(',').nth(8))
        .filter(|v| *v != "." && !v.is_empty())
        .take(5)
        .collect();
    for v in orig_dv {
        assert!(out.contains(v), "protected DV {v} missing from output");
    }
    let tokens = fs::read_to_string(&art.tokens_path).unwrap();
    assert!(tokens.contains("TOTAL INPUT"));
    assert!(tokens.contains("MULTI-MODEL ESTIMATES"));
    assert!(tokens.contains("Claude") && tokens.contains("Gemini") && tokens.contains("Llama 3"));
    assert!(tokens.contains("obfuscator.log") || tokens.contains("EXCLUDED"));
    let log_body = fs::read_to_string(&log_path).unwrap();
    assert!(log_body.contains("SUCCESS"));
    assert!(log_body.contains("DATA OBFUSCATOR"), "log must start with the banner");
    assert!(log_body.contains(&format!("v{}", data_obfuscator::APP_VERSION)));
    assert!(log_body.contains("[OK]"), "success lines are tagged OK");
}

#[test]
fn tokens_subcommand_excludes_log() {
    let dir = tmp();
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/sample.csv");
    fs::write(dir.join("obfuscator.log"), "this is a log and should be excluded\n").unwrap();
    fs::write(dir.join("notes.txt"), "hello world from notes\n").unwrap();
    let log_path = dir.join("trace.log");
    let mut log = TraceLog::open(&log_path, false).unwrap();
    let path = run_tokens(&input, &dir, &[], &mut log).unwrap();
    let body = fs::read_to_string(path).unwrap();
    assert!(body.contains("notes.txt"));
    assert!(body.contains("EXCLUDED"));
}

#[test]
fn warnings_are_counted_and_tagged() {
    let dir = tmp();
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/sample.csv");
    let mut opts = RunOptions::default();
    opts.input = input;
    opts.output_dir = dir.clone();
    opts.level = 1; // dates are not shifted at level 1
    opts.keep = Some(
        ["ID", "TIME", "DV", "DATE", "NOT_A_COLUMN"]
            .into_iter()
            .map(|s| s.to_string())
            .collect(),
    );
    let log_path = dir.join("obfuscator.log");
    let mut log = TraceLog::open(&log_path, false).unwrap();
    let art = run(&opts, &mut log).unwrap();
    assert!(art.warnings >= 2, "expected keep + date warnings, got {}", art.warnings);
    let body = fs::read_to_string(&log_path).unwrap();
    assert!(body.contains("[WARN]"));
    assert!(body.contains("NOT_A_COLUMN"));
    assert!(body.contains("NOT shifted"));
}
