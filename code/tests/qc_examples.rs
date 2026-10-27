//! The shipped example datasets must behave as examples/README.md says they do.

use data_obfuscator::config::{apply_map, parse_config_file, RunOptions};
use data_obfuscator::obfuscate::run;
use data_obfuscator::trace::TraceLog;
use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("do-qcex-{}-{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct Out {
    dataset: String,
    log: String,
    llm: String,
    warnings: usize,
    dir: PathBuf,
}

fn run_example(tag: &str, config: Option<&str>, input: &str, tweak: impl FnOnce(&mut RunOptions)) -> Out {
    let dir = tmp(tag);
    let mut opts = RunOptions::default();
    if let Some(c) = config {
        let map = parse_config_file(&root().join("examples/configs").join(c)).unwrap();
        apply_map(&mut opts, &map).unwrap();
    }
    opts.input = root().join("examples/datasets").join(input);
    opts.output_dir = dir.clone();
    opts.write_keymap = false;
    tweak(&mut opts);
    let mut log = TraceLog::open(&dir.join("obfuscator.log"), false).unwrap();
    let art = run(&opts, &mut log).unwrap_or_else(|e| panic!("{tag}: {e:#}"));
    Out {
        dataset: fs::read_to_string(&art.output_dataset).unwrap(),
        log: fs::read_to_string(dir.join("obfuscator.log")).unwrap(),
        llm: fs::read_to_string(&art.llm_path).unwrap(),
        warnings: art.warnings,
        dir,
    }
}

fn col(csv: &str, name: &str, sep: char) -> Vec<String> {
    let mut lines = csv.lines();
    let head: Vec<&str> = lines.next().unwrap().split(sep).collect();
    let i = head.iter().position(|h| *h == name).unwrap();
    lines.map(|l| l.split(sep).nth(i).unwrap_or("").to_string()).collect()
}

#[test]
fn all_five_datasets_and_the_answer_key_exist() {
    for f in [
        "qc_nearly_clean.csv", "qc_many_errors.csv", "pk_iv_infusion_addl_ss.csv",
        "pk_oral_pooled_multi_id.csv", "pkpd_dvid_endpoints.tsv", "qc_answer_key.md", "make_datasets.py",
    ] {
        assert!(root().join("examples/datasets").join(f).exists(), "missing examples/datasets/{f}");
    }
    assert!(root().join("examples/README.md").exists());
}

#[test]
fn nearly_clean_dataset_masks_without_warnings() {
    let o = run_example("clean", Some("30-qc-nearly-clean.txt"), "qc_nearly_clean.csv", |_| {});
    assert_eq!(o.warnings, 0, "{}", o.log);
    assert!(o.llm.contains("12 subjects"));
    assert!(o.llm.contains("Planned versus administered dose"), "DOSE check expected in the QC prompt");
    assert!(o.llm.contains("Time translation"), "DATE/CLOCK check expected in the QC prompt");
    assert!(!o.dataset.contains("ABC-101"));
}

#[test]
fn many_errors_dataset_keeps_its_defects_visible_after_masking() {
    let o = run_example("errors", Some("31-qc-many-errors.txt"), "qc_many_errors.csv", |_| {});
    assert!(o.log.contains("UNSHIFTED"), "mixed-format dates must be reported:\n{}", o.log);
    assert!(o.log.contains("do not have 24 fields"), "ragged rows must be reported:\n{}", o.log);
    let lines: Vec<&str> = o.dataset.lines().collect();
    assert_eq!(lines[39].split(',').count(), 20, "truncated record must stay truncated (line 40)");
    assert_eq!(lines[69].split(',').count(), 25, "record with a stray field must stay that way (line 70)");
    assert_eq!(lines.len(), 161, "row count and order are unchanged, so answer-key line numbers still apply");
    // text defects in protected columns are untouched
    assert!(o.dataset.contains(",BQL,") && o.dataset.contains(",NA,") && o.dataset.contains(",<0.5,"));
    assert!(o.dataset.contains(",1h,"));
    // the blank ID record is not counted as an extra subject
    assert!(o.llm.contains("10 subjects"), "blank ID must not count as a subject");
}

#[test]
fn iv_infusion_dataset_keeps_rate_addl_ii_ss() {
    let o = run_example("iv", Some("32-iv-infusion-nonmem.txt"), "pk_iv_infusion_addl_ss.csv", |_| {});
    assert_eq!(o.warnings, 0, "{}", o.log);
    assert!(o.llm.contains("zero-order infusions"));
    assert!(o.llm.contains("steady-state"));
    assert!(o.llm.contains("ADDL/II"));
    let orig = fs::read_to_string(root().join("examples/datasets/pk_iv_infusion_addl_ss.csv")).unwrap();
    for c in ["RATE", "AMT", "ADDL", "II", "SS", "DV", "TIME"] {
        assert_eq!(col(&o.dataset, c, ','), col(&orig, c, ','), "{c} must be untouched");
    }
}

#[test]
fn pooled_dataset_with_multi_column_roles_is_fully_masked_and_aligned() {
    let o = run_example("pooled", Some("33-pooled-multi-column-roles.txt"), "pk_oral_pooled_multi_id.csv", |_| {});
    assert_eq!(o.warnings, 0, "{}", o.log);
    for leak in ["Harbor", "Eastvale", "Lakeshore", "Phase 1", "Phase 2", "XR-10", "XR-20", "Okafor", "ascending"] {
        assert!(!o.dataset.contains(leak), "{leak} leaked");
    }
    let stu = col(&o.dataset, "STU", ',');
    for other in ["STUDYID", "STUDY", "PROTNAME"] {
        assert_eq!(stu, col(&o.dataset, other, ','), "STU and {other} must share labels");
    }
    assert_eq!(col(&o.dataset, "USUBJID", ','), col(&o.dataset, "SUBJID", ','));
    assert_eq!(col(&o.dataset, "SITEID", ','), col(&o.dataset, "SITENAME", ','));
    assert_eq!(col(&o.dataset, "ARM", ','), col(&o.dataset, "ARMCD", ','));
    let mut studies = stu.clone();
    studies.sort();
    studies.dedup();
    assert_eq!(studies.len(), 3);
}

#[test]
fn pooled_dataset_without_role_options_warns_about_unrecognised_text_columns() {
    let o = run_example("pooled-bare", None, "pk_oral_pooled_multi_id.csv", |_| {});
    assert!(o.warnings >= 1);
    assert!(o.log.contains("PROTNAME") && o.log.contains("SITENAME"), "{}", o.log);
    assert!(o.log.contains("passed through UNCHANGED"));
}

#[test]
fn tsv_pkpd_dataset_round_trips_as_tsv_with_dvid_endpoints() {
    let o = run_example("pkpd", Some("34-pkpd-tab-delimited.txt"), "pkpd_dvid_endpoints.tsv", |_| {});
    assert_eq!(o.warnings, 0, "{}", o.log);
    assert!(o.dir.join("obfuscated.tsv").exists());
    assert!(!o.dataset.contains("PDX"));
    assert!(o.dataset.lines().next().unwrap().contains('\t'));
    assert!(o.llm.contains("DVID separates several endpoints"));
    assert!(o.llm.contains("separate residual-error model per DVID"));
}
