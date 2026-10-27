//! A role (study, site, subject, ...) can be spread over several columns, e.g. STU / STUDYID /
//! STUDY. Every listed column must be recoded, and columns that describe the same thing must
//! end up with the same masked label.

use data_obfuscator::config::{apply_map, parse_config_text, RunOptions};
use data_obfuscator::obfuscate::run;
use data_obfuscator::trace::TraceLog;
use std::fs;
use std::path::PathBuf;

fn tmp() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "data-obfuscator-multi-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

const DATA: &str = "\
ID,NMID,STU,STUDYID,STUDY,SITE_A,SITE_B,TIME,AMT,DV,EVID,MDV,CMT
1,1001,1,ABC-101,Alpha Trial,S10,Boston,0,100,.,1,1,1
1,1001,1,ABC-101,Alpha Trial,S10,Boston,1,.,5.5,0,0,2
2,1002,1,ABC-101,Alpha Trial,S11,Denver,0,100,.,1,1,1
2,1002,1,ABC-101,Alpha Trial,S11,Denver,1,.,6.1,0,0,2
3,1003,2,XYZ-202,Beta Trial,S10,Boston,0,200,.,1,1,1
3,1003,2,XYZ-202,Beta Trial,S10,Boston,1,.,9.9,0,0,2
";

fn column(csv: &str, name: &str) -> Vec<String> {
    let mut lines = csv.lines();
    let head: Vec<&str> = lines.next().unwrap().split(',').collect();
    let i = head.iter().position(|h| h.eq_ignore_ascii_case(name)).unwrap();
    lines.map(|l| l.split(',').nth(i).unwrap().to_string()).collect()
}

fn masked(opts_text: &str) -> (String, String) {
    let dir = tmp();
    let input = dir.join("in.csv");
    fs::write(&input, DATA).unwrap();
    let mut opts = RunOptions::default();
    let map = parse_config_text(opts_text).unwrap();
    apply_map(&mut opts, &map).unwrap();
    opts.input = input;
    opts.output_dir = dir.join("out");
    opts.level = 2;
    opts.write_keymap = true;
    fs::create_dir_all(&opts.output_dir).unwrap();
    let mut log = TraceLog::open(&opts.output_dir.join("obfuscator.log"), false).unwrap();
    let art = run(&opts, &mut log).unwrap();
    (
        fs::read_to_string(&art.output_dataset).unwrap(),
        fs::read_to_string(art.keymap_path.unwrap()).unwrap(),
    )
}

#[test]
fn every_listed_study_column_is_recoded_and_aligned() {
    let (out, _) = masked(
        "study_column = STU, STUDYID, STUDY\nid_column = ID, NMID\nsite_column = SITE_A, SITE_B\n",
    );
    for col in ["STU", "STUDYID", "STUDY", "SITE_A", "SITE_B", "NMID"] {
        let vals = column(&out, col);
        assert!(
            !DATA.contains(&vals[0]) || vals[0].chars().all(|c| !c.is_ascii_alphabetic()),
            "{col} still looks unmasked: {vals:?}"
        );
    }
    // STU (1/1/1/2...), STUDYID and STUDY pair one-to-one, so they share a label per study.
    assert_eq!(column(&out, "STU"), column(&out, "STUDYID"));
    assert_eq!(column(&out, "STU"), column(&out, "STUDY"));
    // Boston is always S10 and Denver always S11, so the two site columns agree as well.
    assert_eq!(column(&out, "SITE_A"), column(&out, "SITE_B"));
    // Two studies stay two labels; three subjects stay three.
    let mut stu = column(&out, "STU");
    stu.dedup();
    assert_eq!(stu.len(), 2, "{stu:?}");
    // ID and NMID pair one-to-one.
    assert_eq!(column(&out, "ID"), column(&out, "NMID"));
    // PK core untouched.
    assert_eq!(column(&out, "DV"), column(DATA, "DV"));
    assert_eq!(column(&out, "AMT"), column(DATA, "AMT"));
    assert!(!out.contains("Alpha") && !out.contains("ABC-101") && !out.contains("Boston"));
}

#[test]
fn columns_that_do_not_pair_up_get_their_own_labels() {
    // SITE_A has two values for the S10 centre when the second column is coarser: map TIME-like
    // coarse column (STU) onto SITE roles to force a many-to-one relation.
    let (out, _) = masked("site_column = SITE_A, STU\n");
    let a = column(&out, "SITE_A");
    let s = column(&out, "STU");
    // S10 spans both studies, so STU cannot share SITE_A labels: no value is reused across them.
    for v in &s {
        assert!(!a.contains(v), "label {v} reused across non-aligned columns: {a:?} / {s:?}");
    }
}

#[test]
fn unlisted_columns_are_still_auto_detected_and_old_single_value_syntax_works() {
    let (out, _) = masked("study_column = STUDYID\n");
    // STUDYID forced; STU / STUDY are still caught by name auto-detection.
    assert!(!out.contains("ABC-101"));
    assert!(!out.contains("Alpha Trial"));
    let stu = column(&out, "STU");
    assert!(stu.iter().all(|v| v.starts_with("STU")), "{stu:?}");
}

#[test]
fn keymap_lists_every_listed_column() {
    let (_, key) = masked("study_column = STU, STUDYID, STUDY\n");
    for col in ["STU", "STUDYID", "STUDY"] {
        assert!(key.contains(col), "keymap.txt should mention {col}:\n{key}");
    }
}

#[test]
fn missing_role_column_is_reported_but_not_fatal() {
    let (out, _) = masked("study_column = STUDYID, NOT_THERE\n");
    assert!(!out.contains("ABC-101"));
}

#[test]
fn repeated_cli_style_values_merge() {
    let mut o = RunOptions::default();
    let m = parse_config_text("study_column = STU\nstudy_column = STUDYID\n").unwrap();
    apply_map(&mut o, &m).unwrap();
    assert_eq!(o.study_column, ["STU", "STUDYID"]);
}

#[test]
fn cli_flags_take_comma_lists_and_can_be_repeated() {
    use clap::Parser;
    use data_obfuscator::cli::{resolve_run, Cli, Commands};
    let cli = Cli::try_parse_from([
        "data-obfuscator", "run", "-i", "in.csv", "-o", "out",
        "--study-column", "STU,STUDYID", "--study-column", "STUDY",
        "--subject-column", "USUBJID|SUBJID", "--id-column", "ID",
    ])
    .unwrap();
    let Some(Commands::Run(args)) = cli.command else { panic!("run expected") };
    let o = resolve_run(&args).unwrap();
    assert_eq!(o.study_column, ["STU", "STUDYID", "STUDY"]);
    assert_eq!(o.subject_column, ["USUBJID", "SUBJID"]);
    assert_eq!(o.id_column, ["ID"]);
}

#[test]
fn cli_role_flags_replace_the_config_file_value() {
    use clap::Parser;
    use data_obfuscator::cli::{resolve_run, Cli, Commands};
    let dir = tmp();
    let cfg = dir.join("c.txt");
    fs::write(&cfg, "input = a.csv\noutput = o\nstudy_column = STU, STUDYID\n").unwrap();
    let cli = Cli::try_parse_from([
        "data-obfuscator", "run", "--config", cfg.to_str().unwrap(), "--study-column", "STUDY",
    ])
    .unwrap();
    let Some(Commands::Run(args)) = cli.command else { panic!("run expected") };
    assert_eq!(resolve_run(&args).unwrap().study_column, ["STUDY"]);
}
