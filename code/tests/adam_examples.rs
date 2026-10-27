//! The ADaM examples (examples/datasets/adam and the configs 40 to 42) must keep working: the `run2` config runs, the
//! reverse command script restores the ready-made master dataset to the real values, and the Windows script matches the
//! Linux one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "do-adam-{}-{}-{}",
        std::process::id(),
        tag,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            fs::copy(e.path(), &dst).unwrap();
        }
    }
}

/// The `data-obfuscator ...` lines of a command script, in order.
fn script_commands(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| l.starts_with("data-obfuscator "))
        .collect()
}

/// Runs one script line with the freshly built binary, in `cwd` (a temporary copy of the examples folder).
fn run_line(line: &str, cwd: &Path) -> std::process::Output {
    assert!(!line.contains('"') && !line.contains('\''), "script lines must not need shell quoting: {line}");
    let args: Vec<&str> = line.split_whitespace().skip(1).collect();
    Command::new(env!("CARGO_BIN_EXE_data-obfuscator"))
        .args(&args)
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn read_csv(p: &Path) -> Vec<Vec<String>> {
    let text = fs::read_to_string(p).unwrap();
    text.lines().map(|l| l.split(',').map(|x| x.to_string()).collect()).collect()
}

/// A copy of examples/ (the configs use paths like examples/datasets/adam/adsl.csv) in a temporary folder.
fn sandbox(tag: &str) -> PathBuf {
    let dir = tmp(tag);
    copy_dir(&root().join("examples"), &dir.join("examples"));
    dir
}

#[test]
fn the_adam_datasets_are_present_and_share_subjects() {
    let d = root().join("examples/datasets/adam");
    let adsl = read_csv(&d.join("adsl.csv"));
    let subjects: Vec<&String> = adsl.iter().skip(1).map(|r| &r[1]).collect();
    assert_eq!(subjects.len(), 18, "ADSL has 18 subjects");
    for name in ["adex", "adpc", "adlb", "advs", "adae"] {
        let rows = read_csv(&d.join(format!("{name}.csv")));
        assert!(rows.len() > 1, "{name}.csv has records");
        assert_eq!(rows[0][0], "STUDYID");
        assert_eq!(rows[0][1], "USUBJID");
        for r in rows.iter().skip(1) {
            assert!(subjects.contains(&&r[1]), "{name}: unknown subject {}", r[1]);
        }
    }
}

#[test]
fn the_run2_example_config_masks_all_six_datasets() {
    let dir = sandbox("run2");
    let out = Command::new(env!("CARGO_BIN_EXE_data-obfuscator"))
        .args(["run2", "--config", "examples/configs/41-adam-run2-all.txt"])
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    let o = dir.join("masked/adam-run2");
    for f in ["adsl", "adex", "adpc", "adlb", "advs", "adae"] {
        assert!(o.join(format!("{f}_masked.csv")).exists(), "{f}_masked.csv");
    }
    for f in ["keymap2.txt", "llm2.txt", "summary2.txt"] {
        assert!(o.join(f).exists(), "{f}");
    }
    // the same subject has the same code in every masked dataset; real codes are gone
    let adsl = read_csv(&o.join("adsl_masked.csv"));
    let adpc = read_csv(&o.join("adpc_masked.csv"));
    assert_eq!(adsl[1][1], "SUBJ001");
    assert!(adpc.iter().skip(1).all(|r| r[1].starts_with("SUBJ")));
    assert!(!fs::read_to_string(o.join("adsl_masked.csv")).unwrap().contains("XYZ-201"));
    // concentrations are not scaled
    let real = read_csv(&root().join("examples/datasets/adam/adpc.csv"));
    assert_eq!(real[1][7], adpc[1][4 + 3], "AVAL stays exact");
}

#[test]
fn the_reverse_script_restores_the_master_dataset() {
    let dir = sandbox("reverse");
    let sh = root().join("examples/configs/42-adam-reverse.sh");
    let cmds = script_commands(&sh);
    assert_eq!(cmds.len(), 2, "run2 step and reverse step");
    for c in &cmds {
        let out = run_line(c, &dir);
        assert!(out.status.success(), "{c}\n{}", String::from_utf8_lossy(&out.stdout));
    }
    let restored = read_csv(&dir.join("restored/adam-master/restored.csv"));
    let master = read_csv(&dir.join("examples/datasets/adam/master_from_llm.csv"));
    assert_eq!(restored.len(), master.len());
    let head = &restored[0];
    let col = |n: &str| head.iter().position(|h| h == n).unwrap();
    let adsl = read_csv(&root().join("examples/datasets/adam/adsl.csv"));
    let num = |s: &str| s.parse::<f64>().unwrap();
    for r in restored.iter().skip(1) {
        let real = adsl.iter().skip(1).find(|a| a[1] == r[col("USUBJID")]).expect("real USUBJID back");
        assert_eq!(r[col("ID")], r[col("USUBJID")]);
        assert_eq!(r[col("ARM")], real[5]);
        assert_eq!(r[col("SITEID")], real[3]);
        assert_eq!(r[col("SEX")], real[9]);
        assert!((num(&r[col("WT")]) - num(&real[11])).abs() < 1e-9, "WT");
        assert!((num(&r[col("CRCL")]) - num(&real[12])).abs() < 1e-9, "CRCL");
        assert!((num(&r[col("AGE")]) - num(&real[8])).abs() < 1e-9, "AGE");
    }
    // observation records come back with their real clock times and exact concentrations
    let pc = read_csv(&root().join("examples/datasets/adam/adpc.csv"));
    let mut real: Vec<(String, String, f64)> = pc.iter().skip(1).map(|p| (p[1].clone(), p[4].clone(), num(&p[7]))).collect();
    let mut got: Vec<(String, String, f64)> = restored
        .iter()
        .skip(1)
        .filter(|r| r[col("EVID")] == "0")
        .map(|r| (r[col("USUBJID")].clone(), r[col("DATETIME")].clone(), num(&r[col("DV")])))
        .collect();
    real.sort_by(|a, b| a.partial_cmp(b).unwrap());
    got.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(real, got);
}

#[test]
fn the_windows_script_has_the_same_commands_as_the_linux_script() {
    let d = root().join("examples/configs");
    assert_eq!(script_commands(&d.join("42-adam-reverse.sh")), script_commands(&d.join("42-adam-reverse.cmd")));
}
