//! `run2` (several datasets masked together) and the reversal of a combined dataset through `reverse`.

use data_obfuscator::dataset::{read_dataset, Dataset};
use data_obfuscator::reverse::{reverse, ReverseOptions};
use data_obfuscator::run2::{resolve_run2, run2, Run2Args};
use data_obfuscator::trace::TraceLog;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "do-run2-{}-{}-{}",
        std::process::id(),
        tag,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

const ADSL: &str = "STUDYID,USUBJID,SITEID,ARM,AGE,SEX,WEIGHTBL,TRTSDT,COMMENT\n\
ABC-1,ABC-1-0142-1001,0142,Placebo,34,M,70.5,2024-03-11,called at home\n\
ABC-1,ABC-1-0142-1002,0142,Drug 50 mg,51,F,64.25,2024-03-12,lives at 5 Elm St\n\
ABC-1,ABC-1-0267-1003,0267,Drug 100 mg,45,F,88,2024-03-14,.\n";
const ADEX: &str = "STUDYID,USUBJID,EXDOSE,EXSTDTC\n\
ABC-1,ABC-1-0142-1001,0,2024-03-11T08:00:00\n\
ABC-1,ABC-1-0142-1002,50,2024-03-12T09:30:00\n\
ABC-1,ABC-1-0267-1003,100,2024-03-14T07:45:00\n";
const ADPC: &str = "STUDYID,USUBJID,PARAMCD,ADTM,PCSTRESN\n\
ABC-1,ABC-1-0142-1001,CONC,2024-03-11T09:00:00,0\n\
ABC-1,ABC-1-0142-1002,CONC,2024-03-12T10:30:00,12.375\n\
ABC-1,ABC-1-0142-1002,CONC,2024-03-12T13:30:00,7.5\n\
ABC-1,ABC-1-0267-1003,CONC,2024-03-14T09:45:00,31.0625\n";
const ADLB: &str = "STUDYID,USUBJID,PARAMCD,ADT,LBSTRESN\n\
ABC-1,ABC-1-0142-1001,ALT,2024-03-10,22.4\n\
ABC-1,ABC-1-0142-1002,ALT,2024-03-11,35\n\
ABC-1,ABC-1-0267-1003,ALT,2024-03-13,18.7\n";

fn write_inputs(dir: &Path) -> Vec<String> {
    let mut v = Vec::new();
    for (n, t) in [("adsl", ADSL), ("adex", ADEX), ("adpc", ADPC), ("adlb", ADLB)] {
        let p = dir.join(format!("{n}.csv"));
        fs::write(&p, t).unwrap();
        v.push(p.display().to_string());
    }
    v
}

fn load(p: &Path) -> Dataset {
    read_dataset(p, None, false, &[], "#;$").unwrap()
}

fn mask(tag: &str, level: &str, scale: &str, shift: Option<&str>) -> (PathBuf, PathBuf) {
    let dir = tmp(tag);
    let ins = write_inputs(&dir);
    let out = dir.join("masked");
    let args = Run2Args {
        input: ins,
        output: Some(out.clone()),
        level: Some(level.into()),
        seed: Some(5),
        scale_columns: (!scale.is_empty()).then(|| scale.to_string()),
        date_shift_days: shift.map(String::from),
        ..Default::default()
    };
    let opts = resolve_run2(&args).unwrap();
    fs::create_dir_all(&out).unwrap();
    let mut log = TraceLog::open(&out.join("run2.log"), false).unwrap();
    run2(&opts, &mut log).unwrap();
    (dir, out)
}

fn rev(input: &Path, key: &Path, out: &Path, map: &[&str], strict: bool) -> anyhow::Result<Dataset> {
    fs::create_dir_all(out).unwrap();
    let mut log = TraceLog::open(&out.join("reverse.log"), false).unwrap();
    let o = ReverseOptions {
        input: input.to_path_buf(),
        keymap: Some(key.to_path_buf()),
        output_dir: out.to_path_buf(),
        map: map.iter().map(|s| s.to_string()).collect(),
        strict,
        ..Default::default()
    };
    reverse(&o, &mut log).map(|a| a.dataset)
}

fn norm(x: &str) -> String {
    let t = x.trim();
    match t.parse::<f64>() {
        Ok(v) => format!("{:.9e}", v),
        Err(_) => t.to_string(),
    }
}

fn col(d: &Dataset, name: &str) -> Vec<String> {
    let j = d.col_index(name).unwrap_or_else(|| panic!("no column {name} in {:?}", d.headers));
    d.rows.iter().map(|r| r[j].clone()).collect()
}

#[test]
fn datasets_share_codes_date_shift_and_factors() {
    let (dir, out) = mask("share", "3", "PCSTRESN,LBSTRESN,WEIGHTBL", None);
    for f in ["adsl_masked.csv", "adex_masked.csv", "adpc_masked.csv", "adlb_masked.csv", "keymap2.txt", "llm2.txt", "summary2.txt", "config.used.txt", "run2.log"] {
        assert!(out.join(f).is_file(), "{f} must be written");
    }
    assert!(!out.join("keymap.txt").exists(), "run2 writes keymap2.txt, not keymap.txt");
    let (sl, ex, pc, lb) = (load(&out.join("adsl_masked.csv")), load(&out.join("adex_masked.csv")), load(&out.join("adpc_masked.csv")), load(&out.join("adlb_masked.csv")));
    // the same real subject has the same code everywhere
    assert_eq!(col(&sl, "USUBJID"), col(&ex, "USUBJID"));
    assert_eq!(col(&pc, "USUBJID"), ["SUBJ001", "SUBJ002", "SUBJ002", "SUBJ003"]);
    assert_eq!(col(&lb, "USUBJID"), col(&sl, "USUBJID"));
    assert!(col(&sl, "STUDYID").iter().chain(col(&pc, "STUDYID").iter()).all(|c| c == "STU001"));
    assert!(!fs::read_to_string(out.join("adsl_masked.csv")).unwrap().contains("ABC-1"));
    // free text is gone
    assert!(col(&sl, "COMMENT").iter().all(|c| c == "."));
    // the original names are what is in the folder of originals, untouched
    assert!(fs::read_to_string(dir.join("adsl.csv")).unwrap().contains("ABC-1-0142-1001"));
    // ARM codes are shared with the other datasets that have ARM; here only ADSL
    let km = fs::read_to_string(out.join("keymap2.txt")).unwrap();
    assert!(km.contains("scale\tPCSTRESN") && km.contains("scale\tLBSTRESN") && km.contains("scale\tWEIGHTBL"));
    // dates: a date-only column (ADLB) and a date-time column (ADEX) moved by the same whole number of days
    let shift: f64 = km.lines().find(|l| l.starts_with("date_shift_days")).unwrap().split('\t').nth(1).unwrap().parse().unwrap();
    assert_eq!(shift.fract(), 0.0, "run2 draws whole days so date-only and date-time columns stay consistent");
    let day = |s: &str| chrono::NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d").unwrap();
    let (ex_d, lb_d) = (day(&col(&ex, "EXSTDTC")[0]), day(&col(&lb, "ADT")[0]));
    assert_eq!((ex_d - lb_d).num_days(), 1, "ADEX 2024-03-11 vs ADLB 2024-03-10: the interval of one day is kept");
}

#[test]
fn same_inputs_options_and_seed_give_identical_files() {
    let (_, a) = mask("rep-a", "3", "PCSTRESN", None);
    let (_, b) = mask("rep-b", "3", "PCSTRESN", None);
    for f in ["adsl_masked.csv", "adpc_masked.csv", "keymap2.txt"] {
        assert_eq!(fs::read(a.join(f)).unwrap(), fs::read(b.join(f)).unwrap(), "{f}");
    }
}

/// What an LLM would do: stack dosing, PK and lab records into one dataset with DV, DVID and its own column names.
fn build_master(dir: &Path, masked: bool, out: &Path) {
    let name = |n: &str| if masked { format!("{n}_masked.csv") } else { format!("{n}.csv") };
    let (ex, pc, lb, sl) = (load(&dir.join(name("adex"))), load(&dir.join(name("adpc"))), load(&dir.join(name("adlb"))), load(&dir.join(name("adsl"))));
    let mut lines = vec!["ID,USUBJID,DATE,EVID,AMT,DV,DVID,WT,ARM".to_string()];
    let get = |d: &Dataset, r: &Vec<String>, c: &str| r[d.col_index(c).unwrap()].clone();
    let sub = |u: &str| sl.rows.iter().find(|r| r[sl.col_index("USUBJID").unwrap()] == u).unwrap().clone();
    let id_of = |u: &str| -> String {
        if masked {
            let digits: String = u.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
            digits.trim_start_matches('0').to_string()
        } else {
            u.to_string() // the restored ID column holds the real USUBJID
        }
    };
    let push = |lines: &mut Vec<String>, u: &str, date: &str, evid: &str, amt: &str, dv: &str, dvid: &str| {
        let s = sub(u);
        lines.push(format!("{},{},{},{},{},{},{},{},{}", id_of(u), u, date, evid, amt, dv, dvid, get(&sl, &s, "WEIGHTBL"), get(&sl, &s, "ARM")));
    };
    for r in &ex.rows {
        push(&mut lines, &get(&ex, r, "USUBJID"), &get(&ex, r, "EXSTDTC"), "1", &get(&ex, r, "EXDOSE"), ".", "0");
    }
    for r in &pc.rows {
        push(&mut lines, &get(&pc, r, "USUBJID"), &get(&pc, r, "ADTM"), "0", ".", &get(&pc, r, "PCSTRESN"), "1");
    }
    for r in &lb.rows {
        push(&mut lines, &get(&lb, r, "USUBJID"), &get(&lb, r, "ADT"), "0", ".", &get(&lb, r, "LBSTRESN"), "2");
    }
    fs::write(out, lines.join("\n") + "\n").unwrap();
}

#[test]
fn a_combined_dataset_is_restored_with_conditional_map_rules() {
    let (dir, out) = mask("master", "3", "PCSTRESN,LBSTRESN,WEIGHTBL", None);
    let (mm, mo) = (dir.join("master_masked.csv"), dir.join("master_orig.csv"));
    build_master(&out, true, &mm);
    build_master(&dir, false, &mo);
    assert!(!fs::read_to_string(&mm).unwrap().contains("ABC-1"));
    // DV mixes two differently scaled source columns: the DVID tells them apart
    let back = rev(&mm, &out.join("keymap2.txt"), &dir.join("back"), &["ID=USUBJID", "WT=WEIGHTBL", "DV=PCSTRESN:DVID=1", "DV=LBSTRESN:DVID=2"], true).unwrap();
    let want = load(&mo);
    assert_eq!(back.headers, want.headers);
    assert_eq!(back.n_rows(), want.n_rows());
    for c in &want.headers {
        let (a, b) = (col(&back, c), col(&want, c));
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            assert_eq!(norm(x), norm(y), "column {c}, row {}", i + 1);
        }
    }
    // the report names what was done
    let rep = fs::read_to_string(dir.join("back/reverse_report.txt")).unwrap();
    assert!(rep.contains("COMMENT") && rep.contains("free text redacted"), "report lists what cannot come back:\n{rep}");
}

#[test]
fn without_map_rules_renamed_columns_stay_and_the_id_hint_is_given() {
    let (dir, out) = mask("nomap", "3", "PCSTRESN,LBSTRESN,WEIGHTBL", None);
    let mm = dir.join("master_masked.csv");
    build_master(&out, true, &mm);
    let back = rev(&mm, &out.join("keymap2.txt"), &dir.join("back"), &[], false).unwrap();
    // USUBJID, ARM and DATE keep their names and are restored by themselves
    assert_eq!(col(&back, "USUBJID")[0], "ABC-1-0142-1001");
    assert_eq!(col(&back, "ARM")[0], "Placebo");
    assert!(col(&back, "DATE")[0].starts_with("2024-03-11"), "{:?}", col(&back, "DATE"));
    // DV and WT were renamed by the "LLM": left as they are, and the numeric ID is pointed out
    assert_eq!(col(&back, "DV"), col(&load(&mm), "DV"));
    let rep = fs::read_to_string(dir.join("back/reverse.log")).unwrap();
    assert!(rep.contains("--map ID=USUBJID"), "the log should suggest the map for ID:\n{rep}");
}

#[test]
fn level_1_and_2_need_no_scaling_and_level_4_is_refused() {
    let (dir, out) = mask("lvl1", "1", "", None);
    let km = fs::read_to_string(out.join("keymap2.txt")).unwrap();
    assert!(km.contains("date_shift_days\t0") && !km.contains("\nscale\t"));
    let mm = dir.join("m.csv");
    build_master(&out, true, &mm);
    let back = rev(&mm, &out.join("keymap2.txt"), &dir.join("back"), &["ID=USUBJID"], false).unwrap();
    assert_eq!(col(&back, "USUBJID")[0], "ABC-1-0142-1001");
    assert_eq!(col(&back, "ID")[0], "ABC-1-0142-1001");

    let args = Run2Args { input: write_inputs(&tmp("l4")), output: Some(tmp("l4o")), level: Some("4".into()), ..Default::default() };
    let e = resolve_run2(&args).unwrap_err().to_string();
    assert!(e.contains("level 4 is not available in run2"), "{e}");
}

#[test]
fn invented_codes_are_reported_and_strict_stops() {
    let (dir, out) = mask("bad", "2", "", None);
    let mm = dir.join("m.csv");
    fs::write(&mm, "USUBJID,DV\nSUBJ001,1\nSUBJ002,2\nSUBJ999,3\n").unwrap();
    let back = rev(&mm, &out.join("keymap2.txt"), &dir.join("back"), &[], false).unwrap();
    assert_eq!(col(&back, "USUBJID"), ["ABC-1-0142-1001", "ABC-1-0142-1002", "SUBJ999"]);
    let e = rev(&mm, &out.join("keymap2.txt"), &dir.join("back2"), &[], true).unwrap_err().to_string();
    assert!(e.contains("SUBJ999") && e.contains("--strict"), "{e}");
}

#[test]
fn a_fractional_shift_is_allowed_but_warned_about() {
    let (dir, out) = mask("frac", "2", "", Some("23.6"));
    let log = fs::read_to_string(out.join("run2.log")).unwrap();
    assert!(log.contains("not a whole number of days"), "{log}");
    let mm = dir.join("m.csv");
    fs::write(&mm, "USUBJID,DATE\nSUBJ001,2024-04-03T09:00:00\n").unwrap();
    let _ = fs::read(&mm).unwrap();
    // adex 2024-03-11T08:00 + 23.6 d = 2024-04-03T22:24 ; unshift the datetime back
    let ex = load(&out.join("adex_masked.csv"));
    assert_eq!(col(&ex, "EXSTDTC")[0], "2024-04-03T22:24:00");
    let back = rev(&out.join("adex_masked.csv"), &out.join("keymap2.txt"), &dir.join("back"), &[], false).unwrap();
    assert_eq!(col(&back, "EXSTDTC")[0], "2024-03-11T08:00:00");
}

#[test]
fn folder_and_config_inputs_and_overwrite_protection() {
    let dir = tmp("cfg");
    fs::create_dir_all(dir.join("in")).unwrap();
    let ins = write_inputs(&dir.join("in"));
    let cfg = dir.join("study.txt");
    fs::write(
        &cfg,
        format!("input = {}\ninput = {}\ninput = {}\ninput = {}\noutput = {}\nlevel = 2\nseed = 3\n", ins[0], ins[1], ins[2], ins[3], dir.join("o1").display()),
    )
    .unwrap();
    let o = resolve_run2(&Run2Args { config: Some(cfg), ..Default::default() }).unwrap();
    assert_eq!(o.inputs.len(), 4);
    assert_eq!(o.base.seed, 3);
    // a folder gives every data file in it
    let o = resolve_run2(&Run2Args { input: vec![dir.join("in").display().to_string()], output: Some(dir.join("o2")), ..Default::default() }).unwrap();
    assert_eq!(o.inputs.len(), 4);
    // the same file twice is refused
    let e = resolve_run2(&Run2Args { input: vec![ins[0].clone(), ins[0].clone()], output: Some(dir.join("o3")), ..Default::default() }).unwrap_err().to_string();
    assert!(e.contains("listed twice"), "{e}");
    // an output folder with files in it is refused without --overwrite
    fs::create_dir_all(dir.join("o4")).unwrap();
    fs::write(dir.join("o4/x.txt"), "x").unwrap();
    let o = resolve_run2(&Run2Args { input: ins.clone(), output: Some(dir.join("o4")), ..Default::default() }).unwrap();
    assert!(data_obfuscator::run2::runner::check_output_dir(&o).is_err());
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_data-obfuscator"))
}

#[test]
fn run2_and_reverse_end_to_end_on_the_command_line() {
    let dir = tmp("cli");
    let ins = write_inputs(&dir);
    let out = dir.join("masked");
    let mut c = bin();
    c.arg("run2").arg("--no-color").args(["--level", "3", "--seed", "9", "--scale-columns", "PCSTRESN,LBSTRESN,WEIGHTBL", "--output"]).arg(&out);
    for i in &ins {
        c.arg("--input").arg(i);
    }
    let r = c.output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(String::from_utf8_lossy(&r.stderr).contains("run2 complete"));
    let mm = dir.join("m.csv");
    build_master(&out, true, &mm);
    let back = dir.join("back");
    let r = bin()
        .args(["reverse", "--no-color", "--strict", "--map", "ID=USUBJID", "--map", "WT=WEIGHTBL", "--map", "DV=PCSTRESN:DVID=1", "--map", "DV=LBSTRESN:DVID=2", "--input"])
        .arg(&mm)
        .arg("--keymap")
        .arg(out.join("keymap2.txt"))
        .arg("--output")
        .arg(&back)
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let restored = load(&back.join("restored.csv"));
    assert_eq!(col(&restored, "USUBJID")[4], "ABC-1-0142-1002");
    assert_eq!(norm(&col(&restored, "DV")[4]), norm("12.375"));
    assert_eq!(norm(&col(&restored, "WT")[0]), norm("70.5"));
    // an ordinary keymap.txt is still handled by the original code path (run, then reverse)
    let one = dir.join("one");
    let r = bin().args(["run", "--no-color", "--level", "2", "--input"]).arg(&ins[0]).arg("--output").arg(&one).output().unwrap();
    assert!(r.status.success());
    let r = bin().args(["reverse", "--no-color", "--input"]).arg(one.join("obfuscated.csv")).arg("--output").arg(dir.join("one_back")).output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(col(&load(&dir.join("one_back/restored.csv")), "USUBJID")[0], "ABC-1-0142-1001");
}
