//! Numeric scaling (multiply or divide by a hidden factor), keymap defaults, and the
//! output-folder overwrite guard.

use data_obfuscator::config::{apply_map, parse_config_text, RunOptions};
use data_obfuscator::obfuscate::run;
use data_obfuscator::trace::TraceLog;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "do-scale-{}-{}-{}",
        std::process::id(),
        tag,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

const DATA: &str = "\
ID,USUBJID,DATE,TIME,AMT,DV,EVID,MDV,CMT,WT,AGE,HGB,SEX,NOTE
1,S-001,20240325,0,100,.,1,1,1,70.5,40,13.2,1,a
1,S-001,20240326,1,.,5.5,0,0,2,70.5,40,13.2,1,b
1,S-001,20240327,4,.,3.1,0,0,2,70.5,40,13.2,1,c
2,S-002,20240401,0,200,.,1,1,1,85,55,.,0,d
2,S-002,20240402,1,.,9.9,0,0,2,85,55,.,0,e
2,S-002,20240403,4,.,6.2,0,0,2,85,55,.,0,f
3,S-003,20240410,0,100,.,1,1,1,0,61,11.0,1,g
3,S-003,20240411,2,.,4.4,0,0,2,0,61,11.0,1,h
";

struct Run {
    out: String,
    key: String,
    dir: PathBuf,
}

fn do_run(tag: &str, cfg: &str, tweak: impl FnOnce(&mut RunOptions)) -> Run {
    let dir = tmp(tag);
    let input = dir.join("in.csv");
    fs::write(&input, DATA).unwrap();
    let mut opts = RunOptions::default();
    apply_map(&mut opts, &parse_config_text(cfg).unwrap()).unwrap();
    opts.input = input;
    opts.output_dir = dir.join("out");
    fs::create_dir_all(&opts.output_dir).unwrap();
    tweak(&mut opts);
    opts.validate().unwrap();
    let mut log = TraceLog::open(&opts.output_dir.join("obfuscator.log"), false).unwrap();
    let art = run(&opts, &mut log).unwrap();
    Run {
        out: fs::read_to_string(&art.output_dataset).unwrap(),
        key: art.keymap_path.map(|p| fs::read_to_string(p).unwrap()).unwrap_or_default(),
        dir,
    }
}

fn col(csv: &str, name: &str) -> Vec<String> {
    let mut l = csv.lines();
    let h: Vec<&str> = l.next().unwrap().split(',').collect();
    let i = h.iter().position(|x| *x == name).unwrap();
    l.map(|r| r.split(',').nth(i).unwrap().to_string()).collect()
}

/// (operation, factor) of a column as written in keymap.txt.
fn key_entry(key: &str, column: &str) -> Option<(String, f64)> {
    key.lines().filter(|l| !l.starts_with('#')).find_map(|l| {
        let f: Vec<&str> = l.split('\t').collect();
        (f.len() >= 3 && f[0] == column && (f[1] == "multiply" || f[1] == "divide"))
            .then(|| (f[1].to_string(), f[2].parse::<f64>().unwrap()))
    })
}

#[test]
fn level3_multiplies_or_divides_every_number_of_a_column_by_one_factor_from_1_to_20() {
    let r = do_run("l3", "level = 3\nseed = 7\nscale_columns = HGB\n", |_| {});
    for c in ["WT", "AGE", "HGB"] {
        let (op, factor) = key_entry(&r.key, c).unwrap_or_else(|| panic!("keymap lacks {c}:\n{}", r.key));
        assert!((1.0..=20.0).contains(&factor), "{c}: factor {factor} outside 1-20");
        let before = col(DATA, c);
        let after = col(&r.out, c);
        let mut seen = 0;
        for (b, a) in before.iter().zip(after.iter()) {
            match b.parse::<f64>() {
                Ok(bv) => {
                    let av: f64 = a.parse().unwrap();
                    let expect = if op == "multiply" { bv * factor } else { bv / factor };
                    assert!((av - expect).abs() <= expect.abs() * 1e-6 + 1e-9, "{c}: {b} -> {a}, expected {expect} ({op} {factor})");
                    seen += 1;
                }
                Err(_) => assert_eq!(b, a, "{c}: non-numbers stay as they are"),
            }
        }
        assert!(seen > 0);
    }
    // one factor per column: constant within a subject, and WT=0 stays 0
    let wt = col(&r.out, "WT");
    assert_eq!(wt[0], wt[1]);
    assert_eq!(wt[0], wt[2]);
    assert_eq!(wt[6], "0");
    // missing HGB stays missing
    assert_eq!(col(&r.out, "HGB")[3], ".");
    // everything protected is untouched
    for c in ["TIME", "AMT", "DV", "EVID", "MDV", "CMT"] {
        assert_eq!(col(&r.out, c), col(DATA, c), "{c} must be untouched");
    }
    assert!(r.key.contains("NUMERIC SCALING"));
    assert!(r.dir.join("out/llm.txt").exists());
    let llm = fs::read_to_string(r.dir.join("out/llm.txt")).unwrap();
    assert!(llm.contains("hidden constant"), "llm.txt must say the covariates were rescaled");
    assert!(llm.contains("Rescaled columns"));
}

#[test]
fn factor_range_is_configurable_and_scaling_is_off_below_level_3() {
    let r = do_run("range", "level = 3\nscale_range = 3-3\n", |_| {});
    for c in ["WT", "AGE"] {
        let (_, f) = key_entry(&r.key, c).unwrap();
        assert!((f - 3.0).abs() < 1e-9);
    }
    let l2 = do_run("l2", "level = 2\n", |_| {});
    assert_eq!(col(&l2.out, "WT"), col(DATA, "WT"));
    assert!(key_entry(&l2.key, "WT").is_none());
    assert!(l2.key.contains("no columns were scaled"));
}

#[test]
fn scaling_is_reproducible_for_a_seed_and_changes_with_another() {
    let a = do_run("seed-a", "level = 3\nseed = 5\n", |_| {});
    let b = do_run("seed-b", "level = 3\nseed = 5\n", |_| {});
    let c = do_run("seed-c", "level = 3\nseed = 6\n", |_| {});
    assert_eq!(a.out, b.out);
    assert_eq!(key_entry(&a.key, "WT"), key_entry(&b.key, "WT"));
    assert_ne!(key_entry(&a.key, "WT"), key_entry(&c.key, "WT"));
}

/// (operation, base factor, variation) of a column as written in keymap.txt.
fn key_variation(key: &str, column: &str) -> Option<String> {
    key.lines().filter(|l| !l.starts_with('#')).find_map(|l| {
        let f: Vec<&str> = l.split('\t').collect();
        (f.len() >= 5 && f[0] == column && (f[1] == "multiply" || f[1] == "divide")).then(|| f[4].to_string())
    })
}

#[test]
fn level4_gives_every_value_its_own_factor_within_15_percent_of_the_column_base() {
    let r = do_run("l4", "level = 4\nseed = 11\nscale_columns = HGB\n", |_| {});
    for c in ["WT", "AGE", "HGB"] {
        let (op, base) = key_entry(&r.key, c).unwrap_or_else(|| panic!("keymap lacks {c}:\n{}", r.key));
        assert!((1.0..=20.0).contains(&base), "{c}: base {base} outside 1-20");
        assert_eq!(key_variation(&r.key, c).as_deref(), Some("+/-15%"), "{c}");
        let before = col(DATA, c);
        let after = col(&r.out, c);
        let mut factors: Vec<f64> = Vec::new();
        for (b, a) in before.iter().zip(after.iter()) {
            match b.parse::<f64>() {
                Ok(bv) if bv != 0.0 => {
                    let av: f64 = a.parse().unwrap();
                    let f = if op == "multiply" { av / bv } else { bv / av };
                    assert!(
                        f >= base * 0.85 * (1.0 - 1e-6) && f <= base * 1.15 * (1.0 + 1e-6),
                        "{c}: {b} -> {a} used factor {f}, outside base {base} +/- 15 %"
                    );
                    factors.push(f);
                }
                Ok(_) => assert_eq!(a, "0", "{c}: 0 stays 0"),
                Err(_) => assert_eq!(b, a, "{c}: blanks, '.' and text stay as they are"),
            }
        }
        assert!(factors.len() > 3);
        factors.sort_by(|x, y| x.partial_cmp(y).unwrap());
        factors.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
        assert!(factors.len() > 1, "{c}: the factor must differ from value to value");
    }
    // the same real value in one subject no longer gives the same masked value
    let wt = col(&r.out, "WT");
    assert!(wt[0] != wt[1] || wt[1] != wt[2], "WT repeats of one subject should differ: {wt:?}");
    // protected columns are untouched
    for c in ["TIME", "AMT", "DV", "EVID", "MDV", "CMT"] {
        assert_eq!(col(&r.out, c), col(DATA, c), "{c} must be untouched");
    }
    let llm = fs::read_to_string(r.dir.join("out/llm.txt")).unwrap();
    assert!(llm.contains("differs slightly from value to value"), "llm.txt must explain the per-value factor");
    assert!(llm.contains("NOT treat that as a data error"));
}

#[test]
fn level4_is_reproducible_and_level3_keeps_one_exact_factor() {
    let a = do_run("l4-a", "level = 4\nseed = 5\n", |_| {});
    let b = do_run("l4-b", "level = 4\nseed = 5\n", |_| {});
    let c = do_run("l4-c", "level = 4\nseed = 6\n", |_| {});
    assert_eq!(a.out, b.out, "same seed, same masked dataset");
    assert_eq!(a.key, b.key);
    assert_ne!(a.out, c.out);
    let l3 = do_run("l3-var", "level = 3\nseed = 5\n", |_| {});
    assert_eq!(key_variation(&l3.key, "WT").as_deref(), Some("none"));
    let wt = col(&l3.out, "WT");
    assert_eq!(wt[0], wt[1], "level 3 keeps one exact factor per column");
}

#[test]
fn old_jitter_options_fail_with_a_clear_message() {
    let m = parse_config_text("jitter = 0.05\n").unwrap();
    let e = apply_map(&mut RunOptions::default(), &m).unwrap_err().to_string();
    assert!(e.contains("scale_range"), "{e}");
    let mut o = RunOptions::default();
    o.scale_min = 5.0;
    o.scale_max = 2.0;
    assert!(o.validate().is_err());
}

#[test]
fn compact_yyyymmdd_dates_shift_as_dates_and_keep_their_intervals() {
    let r = do_run("dates", "level = 2\ndate_columns = DATE\ndate_shift_days = 100\n", |_| {});
    let d = col(&r.out, "DATE");
    assert_eq!(d[0], "20240703", "20240325 + 100 days");
    assert_eq!(d[1], "20240704");
    assert_eq!(d[3], "20240710", "20240401 + 100 days");
    assert!(d.iter().all(|x| x.len() == 8), "stay compact: {d:?}");
    assert!(r.key.contains("date_shift_days\t100\tDATE"));
}

#[test]
fn keymap_is_written_by_default_and_can_be_switched_off() {
    assert!(RunOptions::default().write_keymap);
    let on = do_run("km-on", "level = 2\n", |_| {});
    assert!(on.key.contains("DE-IDENTIFICATION KEY"));
    let off = do_run("km-off", "write_keymap = 0\n", |_| {});
    assert!(off.key.is_empty());
    assert!(!off.dir.join("out/keymap.txt").exists());
    let off2 = do_run("km-off2", "keymap = false\n", |_| {});
    assert!(!off2.dir.join("out/keymap.txt").exists());
}

// ---------------------------------------------------------------- command-line behaviour
fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_data-obfuscator"));
    c.arg("--no-color");
    c
}

fn input_file(dir: &Path) -> PathBuf {
    let p = dir.join("in.csv");
    fs::write(&p, DATA).unwrap();
    p
}

#[test]
fn output_folder_with_files_is_refused_unless_overwrite_is_given() {
    let dir = tmp("ow");
    let input = input_file(&dir);
    let out = dir.join("results");

    let first = bin().args(["run", "-i"]).arg(&input).arg("-o").arg(&out).output().unwrap();
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    assert!(out.join("keymap.txt").exists(), "keymap.txt is on by default");
    let before = fs::read(out.join("obfuscated.csv")).unwrap();
    fs::write(out.join("my-notes.txt"), "keep me").unwrap();

    // second run into the same folder: error, nothing touched
    let second = bin().args(["run", "-i"]).arg(&input).arg("-o").arg(&out).args(["--level", "1"]).output().unwrap();
    assert!(!second.status.success(), "must fail");
    let err = String::from_utf8_lossy(&second.stderr);
    assert!(err.contains("--overwrite") && err.contains("already contains"), "{err}");
    assert_eq!(fs::read(out.join("obfuscated.csv")).unwrap(), before, "existing results untouched");
    assert_eq!(fs::read_to_string(out.join("my-notes.txt")).unwrap(), "keep me");

    // with --overwrite it goes ahead (and warns)
    let third = bin().args(["run", "-i"]).arg(&input).arg("-o").arg(&out).args(["--level", "1", "--overwrite"]).output().unwrap();
    assert!(third.status.success(), "{}", String::from_utf8_lossy(&third.stderr));
    assert!(String::from_utf8_lossy(&third.stderr).contains("--overwrite:"));
    assert_ne!(fs::read(out.join("obfuscated.csv")).unwrap(), before);
    assert_eq!(fs::read_to_string(out.join("my-notes.txt")).unwrap(), "keep me", "unrelated files are not deleted");
}

#[test]
fn missing_or_empty_output_folders_are_fine() {
    let dir = tmp("empty");
    let input = input_file(&dir);
    let empty = dir.join("empty");
    fs::create_dir_all(&empty).unwrap();
    for out in [empty, dir.join("new").join("deeper")] {
        let r = bin().args(["run", "-i"]).arg(&input).arg("-o").arg(&out).output().unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    }
}

#[test]
fn write_keymap_flag_accepts_false_and_zero_and_a_bare_flag_means_true() {
    let dir = tmp("flag");
    let input = input_file(&dir);
    for (tag, extra, expect) in [
        ("a", vec!["--write-keymap", "false"], false),
        ("b", vec!["--write-keymap", "0"], false),
        ("c", vec!["--write-keymap"], true),
        ("d", vec!["--write-keymap", "true"], true),
        ("e", vec![], true),
    ] {
        let out = dir.join(tag);
        let r = bin().args(["run", "-i"]).arg(&input).arg("-o").arg(&out).args(&extra).output().unwrap();
        assert!(r.status.success(), "{tag}: {}", String::from_utf8_lossy(&r.stderr));
        assert_eq!(out.join("keymap.txt").exists(), expect, "{tag} {extra:?}");
    }
}

#[test]
fn removed_jitter_flag_gives_a_clear_error() {
    let dir = tmp("jf");
    let input = input_file(&dir);
    let r = bin().args(["run", "-i"]).arg(&input).arg("-o").arg(dir.join("o")).args(["--jitter", "0.05"]).output().unwrap();
    assert!(!r.status.success());
    assert!(String::from_utf8_lossy(&r.stderr).contains("--scale-range"));
    assert!(!dir.join("o").exists(), "nothing is created when the options are invalid");
}

// ---------------------------------------------------------------- decimal date shift (0.0.0.5)
fn shift_line(key: &str, name: &str) -> Option<String> {
    key.lines().filter(|l| !l.starts_with('#')).find_map(|l| {
        let f: Vec<&str> = l.split('\t').collect();
        (f.len() >= 2 && f[0] == name).then(|| f[1].to_string())
    })
}

#[test]
fn default_date_shift_is_a_random_decimal_of_1_to_100_days_and_is_in_the_keymap() {
    let mut seen_fraction = false;
    for seed in 1..=12u32 {
        let r = do_run(&format!("ds{seed}"), &format!("level = 2\ndate_columns = DATE\nseed = {seed}\n"), |_| {});
        let v: f64 = shift_line(&r.key, "date_shift_days").expect("date_shift_days line").parse().unwrap();
        assert!((1.0..=100.0).contains(&v), "shift {v} outside 1-100");
        assert!(((v * 10.0).round() - v * 10.0).abs() < 1e-6, "one decimal: {v}");
        seen_fraction |= v.fract().abs() > 1e-9;
        let whole: f64 = shift_line(&r.key, "date_shift_whole_days").unwrap().parse().unwrap();
        assert_eq!(whole, v.floor());
        // the masked date column moved by exactly the whole days; intervals are unchanged
        let d = col(&r.out, "DATE");
        let base = chrono::NaiveDate::from_ymd_opt(2024, 3, 25).unwrap();
        let got = chrono::NaiveDate::parse_from_str(&d[0], "%Y%m%d").unwrap();
        assert_eq!((got - base).num_days(), whole as i64, "seed {seed}");
        assert_eq!(d[0].len(), 8);
    }
    assert!(seen_fraction, "12 draws should include non-integer shifts");
    assert!(do_run("ds-l1", "level = 1\n", |_| {}).key.contains("date_shift_days\t0\t"));
}

#[test]
fn date_shift_can_be_fixed_to_a_decimal_in_the_config_file_or_on_the_command_line() {
    for (tag, cfg) in [("a", "date_shift_days = 23.6\n"), ("b", "date_shift = 23.6\n"), ("c", "date_shift_days = 23.6-23.6\n")] {
        let r = do_run(&format!("fx-{tag}"), &format!("level = 2\ndate_columns = DATE\n{cfg}"), |_| {});
        assert_eq!(shift_line(&r.key, "date_shift_days").as_deref(), Some("23.6"), "{cfg}");
        assert_eq!(shift_line(&r.key, "date_shift_whole_days").as_deref(), Some("23"));
        assert_eq!(col(&r.out, "DATE")[0], "20240417");
    }
    // a MIN-MAX window with decimals and negative numbers
    let (lo, hi) = data_obfuscator::config::parse_f64_range("-30.5--5").unwrap();
    assert_eq!((lo, hi), (-30.5, -5.0));
    assert_eq!(data_obfuscator::config::parse_f64_range("1-100").unwrap(), (1.0, 100.0));
    assert_eq!(data_obfuscator::config::parse_f64_range("7.25").unwrap(), (7.25, 7.25));
    assert!(data_obfuscator::config::parse_f64_range("abc").is_err());

    // command line: flag value, alias, and a date-time column that moves by the exact fraction
    let dir = tmp("fxcli");
    let input = dir.join("dt.csv");
    fs::write(&input, "ID,DTM,DV\n1,2024-03-25 08:00,5\n1,2024-03-26 09:30,6\n2,2024-04-01,7\n").unwrap();
    for (tag, flag) in [("a", "--date-shift-days"), ("b", "--date-shift")] {
        let out = dir.join(tag);
        let r = bin().args(["run", "-i"]).arg(&input).arg("-o").arg(&out).args([flag, "23.6", "--date-columns", "DTM"]).output().unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        let o = fs::read_to_string(out.join("obfuscated.csv")).unwrap();
        let d = col(&o, "DTM");
        assert_eq!(d[0], "2024-04-17 22:24", "08:00 + 23.6 days");
        assert_eq!(d[1], "2024-04-18 23:54", "09:30 + 23.6 days");
        assert_eq!(d[2], "2024-04-24", "date-only moves by 23 whole days");
        let k = fs::read_to_string(out.join("keymap.txt")).unwrap();
        assert_eq!(shift_line(&k, "date_shift_days").as_deref(), Some("23.6"));
    }
    // a bad value is an error and nothing is written
    let bad = bin().args(["run", "-i"]).arg(&input).arg("-o").arg(dir.join("bad")).args(["--date-shift-days", "soon"]).output().unwrap();
    assert!(!bad.status.success());
    assert!(!dir.join("bad").exists());
}

#[test]
fn date_shift_is_reproducible_for_a_seed() {
    let a = do_run("rs-a", "level = 2\ndate_columns = DATE\nseed = 9\n", |_| {});
    let b = do_run("rs-b", "level = 2\ndate_columns = DATE\nseed = 9\n", |_| {});
    assert_eq!(shift_line(&a.key, "date_shift_days"), shift_line(&b.key, "date_shift_days"));
    assert_eq!(a.out, b.out);
}
