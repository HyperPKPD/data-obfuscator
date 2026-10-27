//! `reverse` and keymap_columns.txt: mask a dataset, optionally shuffle it, reverse it, compare with the source.

use data_obfuscator::config::{apply_map, parse_config_text, RunOptions};
use data_obfuscator::dataset::{read_dataset, write_dataset, Dataset};
use data_obfuscator::obfuscate::run;
use data_obfuscator::reverse::{reverse, ReverseOptions, Step};
use data_obfuscator::trace::TraceLog;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "do-rev-{}-{}-{}",
        std::process::id(),
        tag,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

fn sample() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/sample.csv")
}

fn load(p: &Path) -> Dataset {
    read_dataset(p, None, false, &[], "#;$").unwrap()
}

/// Masks `input` with the config text; returns the output folder.
fn mask(tag: &str, input: &Path, cfg: &str) -> PathBuf {
    let dir = tmp(tag);
    let mut opts = RunOptions::default();
    apply_map(&mut opts, &parse_config_text(cfg).unwrap()).unwrap();
    opts.input = input.to_path_buf();
    opts.output_dir = dir.join("masked");
    fs::create_dir_all(&opts.output_dir).unwrap();
    opts.validate().unwrap();
    let mut log = TraceLog::open(&opts.output_dir.join("obfuscator.log"), false).unwrap();
    run(&opts, &mut log).unwrap();
    dir.join("masked")
}

fn rev_opts(input: &Path, keymap: &Path, out: &Path) -> ReverseOptions {
    ReverseOptions { input: input.to_path_buf(), keymap: Some(keymap.to_path_buf()), output_dir: out.to_path_buf(), ..Default::default() }
}

fn do_reverse(o: &ReverseOptions) -> anyhow::Result<Dataset> {
    fs::create_dir_all(&o.output_dir).unwrap();
    let mut log = TraceLog::open(&o.output_dir.join("reverse.log"), false).unwrap();
    reverse(o, &mut log).map(|a| a.dataset)
}

fn norm(x: &str) -> String {
    let t = x.trim();
    match t.parse::<f64>() {
        Ok(v) => format!("{:.9e}", v),
        Err(_) => t.to_string(),
    }
}

/// Free text is redacted by the masking, so it can never come back.
const NOT_BACK: [&str; 3] = ["COMMENT", "EMAIL", "SUBJINIT"];

fn rows_as_sets(d: &Dataset, cols: &[String]) -> Vec<Vec<String>> {
    let idx: Vec<usize> = cols.iter().map(|c| d.col_index(c).unwrap_or_else(|| panic!("column {c} missing"))).collect();
    let mut v: Vec<Vec<String>> = d.rows.iter().map(|r| idx.iter().map(|&i| norm(&r[i])).collect()).collect();
    v.sort();
    v
}

fn common_cols(a: &Dataset, b: &Dataset) -> Vec<String> {
    a.headers.iter().filter(|h| b.col_index(h).is_some() && !NOT_BACK.contains(&h.as_str())).cloned().collect()
}

/// Same rows (in any order) and same cell values in every column both have, except redacted text.
fn assert_same_rows_any_order(orig: &Dataset, got: &Dataset) -> Vec<String> {
    assert_eq!(orig.n_rows(), got.n_rows());
    let cols = common_cols(orig, got);
    assert!(cols.len() >= 15, "only {} common columns", cols.len());
    assert_eq!(rows_as_sets(orig, &cols), rows_as_sets(got, &cols), "restored rows differ from the source");
    cols
}

/// Same cells, row by row.
fn assert_same_in_order(orig: &Dataset, got: &Dataset) {
    assert_eq!(orig.n_rows(), got.n_rows());
    for c in common_cols(orig, got) {
        let (a, b) = (orig.col_index(&c).unwrap(), got.col_index(&c).unwrap());
        for (i, (ra, rb)) in orig.rows.iter().zip(got.rows.iter()).enumerate() {
            assert_eq!(norm(&ra[a]), norm(&rb[b]), "column {c}, row {}", i + 1);
        }
    }
}

/// Reverse the row order, rotate the columns and add a column the key does not know (like a PRED from NONMEM).
fn shuffle(masked: &Path, to: &Path) {
    let mut d = load(masked);
    d.rows.reverse();
    let k = 5 % d.headers.len();
    d.headers.rotate_left(k);
    for r in d.rows.iter_mut() {
        r.rotate_left(k);
    }
    d.headers.push("PRED".into());
    for (i, r) in d.rows.iter_mut().enumerate() {
        r.push(format!("{}", i as f64 * 0.37));
    }
    write_dataset(to, &d).unwrap();
}

#[test]
fn every_level_reverses_to_the_source_when_nothing_is_shuffled() {
    let src = load(&sample());
    for level in 1..=4 {
        let m = mask(&format!("rt{level}"), &sample(), &format!("level = {level}\ndate_columns = DATE\nseed = 17\n"));
        let o = rev_opts(&m.join("obfuscated.csv"), &m.join("keymap.txt"), &m.parent().unwrap().join("back"));
        let back = do_reverse(&o).unwrap_or_else(|e| panic!("level {level}: {e:#}"));
        assert_same_in_order(&src, &back);
        // original column order, too (free text is gone from level 4 on)
        let order: Vec<&String> = back.headers.iter().collect();
        let expect: Vec<&String> = src.headers.iter().filter(|h| back.col_index(h).is_some()).collect();
        assert_eq!(order, expect, "level {level}: column order");
        assert!(m.parent().unwrap().join("back/reverse_report.txt").exists());
        assert!(m.parent().unwrap().join("back/restored.csv").exists());
    }
}

#[test]
fn shuffled_rows_and_columns_and_an_extra_column_still_reverse() {
    let src = load(&sample());
    for level in [2, 3, 4] {
        let m = mask(&format!("sh{level}"), &sample(), &format!("level = {level}\ndate_columns = DATE\nseed = 5\n"));
        let shuffled = m.join("shuffled.csv");
        shuffle(&m.join("obfuscated.csv"), &shuffled);
        let o = rev_opts(&shuffled, &m.join("keymap.txt"), &m.parent().unwrap().join("back"));
        let back = do_reverse(&o).unwrap_or_else(|e| panic!("level {level}: {e:#}"));
        assert_same_rows_any_order(&src, &back);
        assert!(back.col_index("PRED").is_some(), "extra columns are kept");
        if level == 4 {
            // keymap_columns.txt knows the source row numbers, so even the row order comes back
            assert_same_in_order(&src, &back);
            assert_eq!(back.headers.last().map(|s| s.as_str()), Some("PRED"));
        }
    }
}

#[test]
fn level4_writes_keymap_columns_with_keys_and_every_factor() {
    let m = mask("kc", &sample(), "level = 4\nseed = 3\n");
    let text = fs::read_to_string(m.join("keymap_columns.txt")).expect("level 4 writes keymap_columns.txt");
    let key = fs::read_to_string(m.join("keymap.txt")).unwrap();
    assert!(key.contains("Per-value file  : keymap_columns.txt"), "keymap.txt must name the file");
    let line = |p: &str| text.lines().find(|l| l.starts_with(p)).unwrap_or_else(|| panic!("no {p} line")).to_string();
    let keys: Vec<String> = line("@key_columns\t").split('\t').skip(1).map(String::from).collect();
    assert!(keys.contains(&"ID".to_string()) && keys.contains(&"TIME".to_string()), "key columns: {keys:?}");
    let values: Vec<String> = line("@value_columns\t").split('\t').skip(1).map(String::from).collect();
    assert_eq!(values, vec!["WT", "AGE"]);
    let bases: Vec<f64> = line("@base_factors\t").split('\t').skip(1).map(|x| x.parse().unwrap()).collect();
    let data: Vec<Vec<&str>> = text.lines().filter(|l| !l.starts_with('#') && !l.starts_with('@') && !l.is_empty()).map(|l| l.split('\t').collect()).collect();
    assert_eq!(data.len(), 180, "one line per data row");
    let mut seen = std::collections::HashSet::new();
    for r in &data {
        assert_eq!(r.len(), 1 + keys.len() + 2 * values.len());
        assert!(seen.insert(r[0]), "source row numbers are unique");
        for (i, base) in bases.iter().enumerate() {
            let f: f64 = r[1 + keys.len() + 2 * i + 1].parse().expect("every WT and AGE value in the sample is numeric");
            assert!(f >= base * 0.85 * (1.0 - 1e-9) && f <= base * 1.15 * (1.0 + 1e-9), "factor {f} outside {base} +/- 15 %");
        }
    }
    // at level 3 nothing varies, so there is no such file
    let m3 = mask("kc3", &sample(), "level = 3\nseed = 3\n");
    assert!(!m3.join("keymap_columns.txt").exists());
    assert!(fs::read_to_string(m3.join("keymap.txt")).unwrap().contains("Per-value file  : none"));
    // and it follows write_keymap
    let off = mask("kc-off", &sample(), "level = 4\nwrite_keymap = false\n");
    assert!(!off.join("keymap_columns.txt").exists() && !off.join("keymap.txt").exists());
}

#[test]
fn without_keymap_columns_level4_is_only_approximate_and_strict_refuses() {
    let src = load(&sample());
    let m = mask("noapprox", &sample(), "level = 4\nseed = 8\ndate_columns = DATE\n");
    // copy only the dataset and keymap.txt to a folder that has no keymap_columns.txt
    let lone = tmp("lone");
    fs::copy(m.join("obfuscated.csv"), lone.join("obfuscated.csv")).unwrap();
    fs::copy(m.join("keymap.txt"), lone.join("keymap.txt")).unwrap();
    let mut o = rev_opts(&lone.join("obfuscated.csv"), &lone.join("keymap.txt"), &lone.join("back"));
    let back = do_reverse(&o).unwrap();
    // rows are not in source order here, so pair them by ID, TIME, EVID and CMT
    let key = |d: &Dataset, r: &Vec<String>| -> String { ["ID", "TIME", "EVID", "CMT"].iter().map(|c| norm(&r[d.col_index(c).unwrap()])).collect::<Vec<_>>().join("|") };
    let by_key: HashMap<String, &Vec<String>> = back.rows.iter().map(|r| (key(&back, r), r)).collect();
    let wt_s = src.col_index("WT").unwrap();
    let wt_b = back.col_index("WT").unwrap();
    for r in &src.rows {
        let b = by_key.get(&key(&src, r)).expect("every source row is there");
        let (x, y): (f64, f64) = (r[wt_s].parse().unwrap(), b[wt_b].parse().unwrap());
        assert!(((x - y) / x).abs() <= 0.15 / 0.85 + 1e-6, "WT {x} restored as {y}: more than the 15 % the base factor allows");
    }
    // exact everywhere else
    assert_eq!(norm(&back.rows[0][back.col_index("USUBJID").unwrap()]).starts_with("ABC-PK"), true);
    // --strict refuses to approximate
    o.strict = true;
    o.output_dir = lone.join("back2");
    let err = do_reverse(&o).unwrap_err().to_string();
    assert!(err.contains("keymap_columns.txt") && err.contains("--strict"), "{err}");
}

#[test]
fn a_keymap_columns_file_from_another_run_is_refused() {
    let a = mask("mixa", &sample(), "level = 4\nseed = 1\n");
    let b = mask("mixb", &sample(), "level = 4\nseed = 2\n");
    let mut o = rev_opts(&a.join("obfuscated.csv"), &a.join("keymap.txt"), &a.parent().unwrap().join("back"));
    o.keymap_columns = Some(b.join("keymap_columns.txt"));
    let err = do_reverse(&o).unwrap_err().to_string();
    assert!(err.contains("different runs"), "{err}");
}

#[test]
fn renamed_columns_and_hashed_ids_reverse_at_level_4() {
    let src = load(&sample());
    let m = mask("ren", &sample(), "level = 4\nseed = 9\nid_style = hashed\ndate_columns = DATE\nrename_columns = ID, TIME, WT, AGE=YEARS\n");
    let masked = load(&m.join("obfuscated.csv"));
    assert!(masked.col_index("TIME").is_none() && masked.col_index("YEARS").is_some(), "columns were renamed");
    let shuffled = m.join("shuffled.csv");
    shuffle(&m.join("obfuscated.csv"), &shuffled);
    let o = rev_opts(&shuffled, &m.join("keymap.txt"), &m.parent().unwrap().join("back"));
    let back = do_reverse(&o).unwrap();
    assert!(back.col_index("TIME").is_some() && back.col_index("AGE").is_some() && back.col_index("YEARS").is_none());
    assert_same_in_order(&src, &back);
}

#[test]
fn steps_can_be_chosen_with_only_and_skip() {
    let src = load(&sample());
    let m = mask("steps", &sample(), "level = 3\nseed = 4\ndate_columns = DATE\n");
    let out = m.parent().unwrap().to_path_buf();
    let masked_file = m.join("obfuscated.csv");
    let masked = load(&masked_file);

    let mut o = rev_opts(&masked_file, &m.join("keymap.txt"), &out.join("only-ids"));
    o.steps = vec![Step::Recoding];
    let back = do_reverse(&o).unwrap();
    let id = back.col_index("ID").unwrap();
    assert_eq!(back.rows[0][id], "101", "IDs restored");
    let wt = back.col_index("WT").unwrap();
    assert_eq!(back.rows[0][wt], masked.rows[0][masked.col_index("WT").unwrap()], "scaling left alone");
    let date = back.col_index("DATE").unwrap();
    assert_eq!(back.rows[0][date], masked.rows[0][masked.col_index("DATE").unwrap()], "dates left alone");

    let mut o = rev_opts(&masked_file, &m.join("keymap.txt"), &out.join("no-dates"));
    o.steps = Step::ALL.iter().copied().filter(|s| *s != Step::Dates).collect();
    let back = do_reverse(&o).unwrap();
    assert_eq!(back.rows[0][back.col_index("DATE").unwrap()], masked.rows[0][masked.col_index("DATE").unwrap()]);
    assert_eq!(norm(&back.rows[0][back.col_index("WT").unwrap()]), norm(&src.rows[0][src.col_index("WT").unwrap()]));

    // keeping the input order
    let shuffled = m.join("shuffled.csv");
    shuffle(&masked_file, &shuffled);
    let mut o = rev_opts(&shuffled, &m.join("keymap.txt"), &out.join("keep-order"));
    o.restore_order = false;
    let back = do_reverse(&o).unwrap();
    assert_eq!(back.headers.last().map(|s| s.as_str()), Some("PRED"));
    assert_ne!(back.headers[0], "ID", "columns stay where the input had them");
}

#[test]
fn keymap_lists_column_order_and_what_cannot_be_restored() {
    let m = mask("sec5", &sample(), "level = 2\n");
    let key = fs::read_to_string(m.join("keymap.txt")).unwrap();
    assert!(key.contains("# 5. COLUMN ORDER, AND WHAT CANNOT BE RESTORED"));
    assert!(key.contains("column_order\t1\tID\tID"));
    assert!(key.contains("column_order\t6\tTIME\tTIME"));
    for c in NOT_BACK {
        assert!(key.contains(&format!("not_restorable\t{c}\tredacted")), "{c} is redacted at level 2");
    }
    let m4 = mask("sec5b", &sample(), "level = 4\n");
    let key4 = fs::read_to_string(m4.join("keymap.txt")).unwrap();
    assert!(key4.contains("not_restorable\tCOMMENT\tnot in the masked dataset"), "dropped at level 4");
}

#[test]
fn study_and_dvid_columns_are_part_of_the_row_key() {
    // pooled studies: STU and STDY (two study columns), two endpoints told apart by DVID, same ID/TIME in both studies
    let dir = tmp("studykey");
    let input = dir.join("in.csv");
    fs::write(
        &input,
        "ID,STU,STDY,TIME,DVID,EVID,CMT,AMT,DV,WT\n\
         1,AB-101,AB-101,0,0,1,1,100,.,70.5\n\
         1,AB-101,AB-101,1,1,0,2,.,5.5,70.5\n\
         1,AB-101,AB-101,1,2,0,3,.,1.2,70.5\n\
         1,AB-202,AB-202,0,0,1,1,100,.,81.2\n\
         1,AB-202,AB-202,1,1,0,2,.,6.1,81.2\n\
         1,AB-202,AB-202,1,2,0,3,.,1.4,81.2\n\
         2,AB-101,AB-101,0,0,1,1,200,.,64.0\n\
         2,AB-101,AB-101,1,1,0,2,.,9.9,64.0\n",
    )
    .unwrap();
    let m = mask("studykey-m", &input, "level = 4\nseed = 5\n");
    let text = fs::read_to_string(m.join("keymap_columns.txt")).unwrap();
    let keys: Vec<String> = text.lines().find(|l| l.starts_with("@key_columns\t")).unwrap().split('\t').skip(1).map(String::from).collect();
    for want in ["ID", "STU", "STDY", "TIME", "DVID", "EVID", "CMT"] {
        assert!(keys.contains(&want.to_string()), "{want} must be a key column: {keys:?}");
    }
    let shuffled = m.join("shuffled.csv");
    shuffle(&m.join("obfuscated.csv"), &shuffled);
    let back = do_reverse(&rev_opts(&shuffled, &m.join("keymap.txt"), &m.parent().unwrap().join("back"))).unwrap();
    let src = load(&input);
    assert_eq!(back.n_rows(), 8);
    for c in ["ID", "STU", "STDY", "TIME", "DVID", "EVID", "CMT", "AMT", "DV", "WT"] {
        let (a, b) = (src.col_index(c).unwrap(), back.col_index(c).unwrap());
        for (i, (ra, rb)) in src.rows.iter().zip(back.rows.iter()).enumerate() {
            assert_eq!(norm(&ra[a]), norm(&rb[b]), "{c} row {}", i + 1);
        }
    }
}

#[test]
fn duplicate_keys_and_blank_cells_are_matched_correctly() {
    // two records with the same ID, TIME, EVID and CMT; blank and text cells in a scaled column
    let dir = tmp("dup");
    let input = dir.join("in.csv");
    fs::write(
        &input,
        "ID,TIME,EVID,CMT,AMT,DV,WT,AGE,DATE\n\
         1,0,1,1,100,.,70.5,40,2024-03-25\n\
         1,0,0,2,.,5.5,70.5,40,2024-03-25\n\
         1,0,0,2,.,5.6,71.1,40,2024-03-25\n\
         1,1,0,2,.,3.1,.,41,2024-03-26\n\
         2,0,1,1,200,.,85,55,2024-04-01\n\
         2,1,0,2,.,9.9,n/a,55,2024-04-02\n\
         3,0,1,1,100,.,0,61,2024-04-10\n",
    )
    .unwrap();
    let m = mask("dup-m", &input, "level = 4\nseed = 12\ndate_columns = DATE\n");
    let shuffled = m.join("shuffled.csv");
    shuffle(&m.join("obfuscated.csv"), &shuffled);
    let back = do_reverse(&rev_opts(&shuffled, &m.join("keymap.txt"), &m.parent().unwrap().join("back"))).unwrap();
    let src = load(&input);
    assert_eq!(back.n_rows(), 7);
    for c in ["ID", "TIME", "EVID", "CMT", "AMT", "DV", "WT", "AGE", "DATE"] {
        let (a, b) = (src.col_index(c).unwrap(), back.col_index(c).unwrap());
        for (i, (ra, rb)) in src.rows.iter().zip(back.rows.iter()).enumerate() {
            assert_eq!(norm(&ra[a]), norm(&rb[b]), "{c} row {}", i + 1);
        }
    }
}

#[test]
fn every_date_format_comes_back_after_a_fractional_shift() {
    let dir = tmp("dates");
    let input = dir.join("in.csv");
    fs::write(
        &input,
        "ID,TIME,DV,DATE\n\
         1,0,.,2024-03-25 08:00\n\
         1,1,5,2024-03-25T20:00:30\n\
         1,2,6,20240325\n\
         1,3,7,202403250830\n\
         1,4,8,2024-03-25\n\
         1,5,9,45300\n\
         1,6,.,.\n\
         1,7,3,25/03/2024\n\
         1,8,4,25-MAR-2024\n",
    )
    .unwrap();
    for shift in ["23.6", "-30.5", "0.4", "100"] {
        let m = mask(&format!("dates-{shift}"), &input, &format!("level = 2\ndate_columns = DATE\ndate_shift_days = {shift}\n"));
        let back = do_reverse(&rev_opts(&m.join("obfuscated.csv"), &m.join("keymap.txt"), &m.parent().unwrap().join("back"))).unwrap();
        let date = back.col_index("DATE").unwrap();
        let got: Vec<&str> = back.rows.iter().map(|r| r[date].as_str()).collect();
        assert_eq!(
            got,
            [
                "2024-03-25 08:00",
                "2024-03-25T20:00:30",
                "20240325",
                "202403250830",
                "2024-03-25",
                "45300",
                ".",
                "25/03/2024",
                "2024-03-25", // DD-MMM-YYYY is written as YYYY-MM-DD by the masking and stays so
            ],
            "shift {shift}"
        );
    }
}

#[test]
fn columns_that_were_never_kept_do_not_trouble_a_strict_reverse() {
    let src = load(&sample());
    let m = mask("keep", &sample(), "level = 4\nseed = 2\nkeep = ID, TIME, EVID, CMT, DV, WT\n");
    let mut o = rev_opts(&m.join("obfuscated.csv"), &m.join("keymap.txt"), &m.parent().unwrap().join("back"));
    o.strict = true; // AGE and the dates were scaled or shifted but are not in the masked file: nothing to undo
    let back = do_reverse(&o).unwrap();
    assert_eq!(back.headers, ["ID", "TIME", "EVID", "CMT", "DV", "WT"]);
    for c in ["ID", "TIME", "EVID", "CMT", "DV", "WT"] {
        let (a, b) = (src.col_index(c).unwrap(), back.col_index(c).unwrap());
        for (ra, rb) in src.rows.iter().zip(back.rows.iter()) {
            assert_eq!(norm(&ra[a]), norm(&rb[b]), "{c}");
        }
    }
}

// ---------------------------------------------------------------- command line
fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_data-obfuscator"));
    c.arg("--no-color");
    c
}

#[test]
fn reverse_command_line_end_to_end() {
    let dir = tmp("cli");
    let masked = dir.join("masked");
    let run = bin()
        .args(["run", "-i"])
        .arg(sample())
        .arg("-o")
        .arg(&masked)
        .args(["--level", "4", "--seed", "21", "--date-columns", "DATE"])
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(String::from_utf8_lossy(&run.stderr).contains("keymap_columns.txt"), "the run lists the new file");

    // keymap.txt and keymap_columns.txt are found next to the masked dataset
    let out = dir.join("restored");
    let rev = bin().args(["reverse", "-i"]).arg(masked.join("obfuscated.csv")).arg("-o").arg(&out).output().unwrap();
    assert!(rev.status.success(), "{}", String::from_utf8_lossy(&rev.stderr));
    for f in ["restored.csv", "reverse_report.txt", "reverse.log"] {
        assert!(out.join(f).exists(), "{f}");
    }
    assert_same_in_order(&load(&sample()), &load(&out.join("restored.csv")));
    let report = fs::read_to_string(out.join("reverse_report.txt")).unwrap();
    assert!(report.contains("matched 180 of 180") && report.contains("Restored SHA-256"), "{report}");

    // the same folder again is refused, --overwrite allows it
    let again = bin().args(["reverse", "-i"]).arg(masked.join("obfuscated.csv")).arg("-o").arg(&out).output().unwrap();
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("--overwrite"));
    let again = bin().args(["reverse", "-i"]).arg(masked.join("obfuscated.csv")).arg("-o").arg(&out).arg("--overwrite").output().unwrap();
    assert!(again.status.success());

    // no key, wrong steps, both --only and --skip
    let lonely = dir.join("lonely.csv");
    fs::copy(masked.join("obfuscated.csv"), &lonely).unwrap();
    let r = bin().args(["reverse", "-i"]).arg(&lonely).arg("-o").arg(dir.join("o1")).output().unwrap();
    assert!(!r.status.success() && String::from_utf8_lossy(&r.stderr).contains("keymap.txt not found"));
    let r = bin().args(["reverse", "-i"]).arg(masked.join("obfuscated.csv")).arg("-o").arg(dir.join("o2")).args(["--only", "ids,dates"]).output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let r = bin().args(["reverse", "-i"]).arg(masked.join("obfuscated.csv")).arg("-o").arg(dir.join("o3")).args(["--only", "ids", "--skip", "dates"]).output().unwrap();
    assert!(!r.status.success());
    let r = bin().args(["reverse", "-i"]).arg(masked.join("obfuscated.csv")).arg("-o").arg(dir.join("o4")).args(["--skip", "bogus"]).output().unwrap();
    assert!(!r.status.success());
}

#[test]
fn a_leftover_keymap_columns_file_triggers_a_warning_on_a_later_run() {
    let dir = tmp("stale");
    let out = dir.join("masked");
    let first = bin().args(["run", "-i"]).arg(sample()).arg("-o").arg(&out).args(["--level", "4"]).output().unwrap();
    assert!(first.status.success());
    assert!(out.join("keymap_columns.txt").exists());
    let second = bin().args(["run", "-i"]).arg(sample()).arg("-o").arg(&out).args(["--level", "3", "--overwrite"]).output().unwrap();
    assert!(second.status.success());
    let log = fs::read_to_string(out.join("obfuscator.log")).unwrap();
    assert!(log.contains("keymap_columns.txt from an EARLIER run"), "{log}");
}
