//! Column renaming (rename_columns / rename_except), its record in keymap.txt, and the
//! descriptive keymap header.

use data_obfuscator::config::{apply_map, parse_config_text, RunOptions};
use data_obfuscator::obfuscate::run;
use data_obfuscator::trace::TraceLog;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "do-rename-{}-{}-{}",
        std::process::id(),
        tag,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&p).unwrap();
    p
}

const DATA: &str = "\
ID,USUBJID,SITE_NAME,DATE,TIME,AMT,DV,EVID,MDV,CMT,WT,AGE,SEX,NOTE
1,S-001,Harbor Clinic,20240325,0,100,.,1,1,1,70.5,40,1,a
1,S-001,Harbor Clinic,20240326,1,.,5.5,0,0,2,70.5,40,1,b
2,S-002,Lake Hospital,20240401,0,200,.,1,1,1,85,55,0,c
2,S-002,Lake Hospital,20240402,1,.,9.9,0,0,2,85,55,0,d
";

struct Run {
    out: String,
    key: String,
    dir: PathBuf,
    sha: String,
}

fn do_run(tag: &str, cfg: &str) -> Run {
    let dir = tmp(tag);
    let input = dir.join("in.csv");
    fs::write(&input, DATA).unwrap();
    let mut opts = RunOptions::default();
    apply_map(&mut opts, &parse_config_text(cfg).unwrap()).unwrap();
    opts.input = input;
    opts.output_dir = dir.join("out");
    opts.validate().unwrap();
    let mut log = TraceLog::open(&{
        fs::create_dir_all(&opts.output_dir).unwrap();
        opts.output_dir.join("obfuscator.log")
    }, false)
    .unwrap();
    let art = run(&opts, &mut log).unwrap();
    Run {
        out: fs::read_to_string(&art.output_dataset).unwrap(),
        key: art.keymap_path.map(|p| fs::read_to_string(p).unwrap()).unwrap_or_default(),
        sha: art.summary.output_sha256.clone(),
        dir,
    }
}

fn header(csv: &str) -> Vec<String> {
    csv.lines().next().unwrap().split(',').map(|s| s.to_string()).collect()
}
fn col(csv: &str, name: &str) -> Vec<String> {
    let mut l = csv.lines();
    let h: Vec<&str> = l.next().unwrap().split(',').collect();
    let i = h.iter().position(|x| *x == name).unwrap_or_else(|| panic!("no column {name} in {h:?}"));
    l.map(|r| r.split(',').nth(i).unwrap().to_string()).collect()
}
fn renamed_lines(key: &str) -> Vec<(String, String)> {
    key.lines()
        .filter(|l| l.starts_with("renamed_column\t"))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f[1].to_string(), f[2].to_string())
        })
        .collect()
}

#[test]
fn nothing_is_renamed_by_default() {
    let r = do_run("none", "level = 2\n");
    assert_eq!(header(&r.out), header(DATA));
    assert!(renamed_lines(&r.key).is_empty());
    assert!(r.key.contains("no columns were renamed"));
}

#[test]
fn listed_columns_get_generic_names_and_every_rename_is_in_the_keymap() {
    let r = do_run("list", "level = 2\nrename_columns = SITE_NAME, NOTE, wt\n");
    let h = header(&r.out);
    assert_eq!(h.len(), 14, "same number of columns");
    assert_eq!(h[2], "COL03");
    assert_eq!(h[13], "COL14");
    assert_eq!(h[10], "COL11", "matched case-insensitively");
    assert_eq!(h[0], "ID");
    assert_eq!(h[10 + 1], "AGE");
    assert_eq!(
        renamed_lines(&r.key),
        vec![
            ("SITE_NAME".to_string(), "COL03".to_string()),
            ("WT".to_string(), "COL11".to_string()),
            ("NOTE".to_string(), "COL14".to_string()),
        ]
    );
    // only the header changed: protected columns and untouched values are as before
    for c in ["TIME", "AMT", "DV", "EVID", "MDV", "CMT"] {
        let orig: Vec<String> = col(DATA, c);
        assert_eq!(col(&r.out, c), orig, "{c}");
    }
    assert_eq!(col(&r.out, "COL11"), col(DATA, "WT"), "WT values unchanged at level 2");
}

#[test]
fn rename_except_renames_all_other_columns() {
    let r = do_run("except", "level = 2\nrename_except = ID, TIME, AMT, DV, EVID, MDV, CMT\n");
    let h = header(&r.out);
    for keep in ["ID", "TIME", "AMT", "DV", "EVID", "MDV", "CMT"] {
        assert!(h.contains(&keep.to_string()), "{keep} kept");
    }
    let pairs = renamed_lines(&r.key);
    assert_eq!(pairs.len(), 14 - 7);
    for (old, new) in &pairs {
        assert!(!["ID", "TIME", "AMT", "DV", "EVID", "MDV", "CMT"].contains(&old.as_str()));
        assert!(new.starts_with("COL"), "{new}");
        assert!(h.contains(new));
        assert!(!h.contains(old));
    }
    assert!(r.key.contains("7 column(s) renamed"));
}

#[test]
fn user_chosen_names_prefix_and_scaling_follow_the_new_name() {
    let r = do_run("named", "level = 3\nseed = 3\nrename_columns = WT=BODYSIZE, SITE_NAME\nrename_prefix = VAR\n");
    let h = header(&r.out);
    assert!(h.contains(&"BODYSIZE".to_string()));
    assert!(h.contains(&"VAR03".to_string()));
    assert!(!h.contains(&"WT".to_string()));
    let pairs = renamed_lines(&r.key);
    assert!(pairs.contains(&("WT".to_string(), "BODYSIZE".to_string())));
    assert!(pairs.contains(&("SITE_NAME".to_string(), "VAR03".to_string())));
    // scaling is recorded under the original name and applied to the renamed column
    let line = r.key.lines().find(|l| l.starts_with("WT\t")).expect("WT scaling line");
    let f: Vec<&str> = line.split('\t').collect();
    let factor: f64 = f[2].parse().unwrap();
    let got: f64 = col(&r.out, "BODYSIZE")[0].parse().unwrap();
    let expect = if f[1] == "multiply" { 70.5 * factor } else { 70.5 / factor };
    assert!((got - expect).abs() < expect * 1e-6);
    // llm.txt: scaled column under its new name; only the automatic name is called unknown
    let llm = fs::read_to_string(r.dir.join("out/llm.txt")).unwrap();
    assert!(llm.contains("Rescaled columns    : BODYSIZE") || llm.contains("BODYSIZE"), "{llm}");
    assert!(llm.contains("Renamed columns     : VAR03 "), "{llm}");
    assert!(!llm.contains("Renamed columns     : VAR03, BODYSIZE"));
    assert!(!llm.contains("SITE_NAME"), "original name must not leak into llm.txt");
}

#[test]
fn keymap_has_a_descriptive_header() {
    let r = do_run("hdr", "level = 3\nseed = 11\nrename_columns = NOTE\n");
    let top: String = r.key.lines().take(40).collect::<Vec<_>>().join("\n");
    for needle in [
        "DATA OBFUSCATOR",
        "DE-IDENTIFICATION KEY",
        "CONFIDENTIAL",
        "Source dataset  : in.csv",
        "Masked dataset  : obfuscated.csv",
        "Masking level   : 3",
        "Random seed     : 11",
        "WHAT IS IN THIS FILE",
        "1. COLUMN RENAMING",
        "2. NUMERIC SCALING",
        "3. DATE SHIFT",
        "4. IDENTIFIER AND LABEL RECODING",
        "HOW TO READ IT",
    ] {
        assert!(top.contains(needle), "header lacks '{needle}':\n{top}");
    }
    assert!(r.key.contains(&format!("Masked SHA-256  : {}", r.sha)), "header carries the dataset fingerprint");
    // every line before the data is a comment or a data line with TABs; no stray text
    for l in r.key.lines() {
        assert!(l.starts_with('#') || l.contains('\t'), "unexpected line: {l:?}");
    }
}

#[test]
fn renaming_is_reproducible_and_keymap_stays_identical() {
    let cfg = "level = 3\nseed = 5\nrename_except = ID, TIME, DV, AMT, EVID, MDV, CMT\n";
    let a = do_run("rep-a", cfg);
    let b = do_run("rep-b", cfg);
    assert_eq!(a.out, b.out);
    assert_eq!(a.key, b.key);
}

#[test]
fn unknown_or_dropped_names_only_warn_and_both_modes_are_refused() {
    let r = do_run("warn", "rename_columns = NOPE, NOTE, WT\ndrop = NOTE\n");
    let pairs = renamed_lines(&r.key);
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!(pairs[0].0, "WT");
    let log = fs::read_to_string(r.dir.join("out/obfuscator.log")).unwrap();
    assert!(log.contains("'NOPE'") && log.contains("'NOTE'"), "{log}");
    assert!(log.contains("[WARN]"));

    let m = parse_config_text("rename_columns = WT\nrename_except = ID\n").unwrap();
    let mut o = RunOptions::default();
    apply_map(&mut o, &m).unwrap();
    assert!(o.validate().is_err());
}

#[test]
fn config_file_syntax_accepts_old_equals_new_and_aliases() {
    let m = parse_config_text("# header\nrename = WT=BODYSIZE, AGE\nno_rename: ID, TIME\nrename_prefix = V\n").unwrap();
    let mut o = RunOptions::default();
    apply_map(&mut o, &m).unwrap();
    assert_eq!(o.rename_columns, vec!["WT=BODYSIZE", "AGE"]);
    assert_eq!(o.rename_except, vec!["ID", "TIME"]);
    assert_eq!(o.rename_prefix, "V");
    assert!(o.dump().contains("rename_columns = WT=BODYSIZE,AGE"));
}

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_data-obfuscator"));
    c.arg("--no-color");
    c
}

#[test]
fn command_line_flags_rename_columns() {
    let dir = tmp("cli");
    let input = dir.join("in.csv");
    fs::write(&input, DATA).unwrap();

    let out = dir.join("a");
    let r = bin()
        .args(["run", "-i"]).arg(&input).arg("-o").arg(&out)
        .args(["--rename-columns", "SITE_NAME,WT=BODYSIZE", "--rename-prefix", "X"])
        .output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let o = fs::read_to_string(out.join("obfuscated.csv")).unwrap();
    assert_eq!(header(&o)[2], "X03");
    assert!(header(&o).contains(&"BODYSIZE".to_string()));
    let k = fs::read_to_string(out.join("keymap.txt")).unwrap();
    assert_eq!(renamed_lines(&k).len(), 2);

    let out = dir.join("b");
    let r = bin()
        .args(["run", "-i"]).arg(&input).arg("-o").arg(&out)
        .args(["--no-rename", "ID,TIME,DV,AMT,EVID,MDV,CMT"])
        .output().unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let o = fs::read_to_string(out.join("obfuscated.csv")).unwrap();
    assert_eq!(&header(&o)[..2], &["ID".to_string(), "COL02".to_string()]);

    // both modes on the command line, and a bad new name: errors, nothing written
    for (tag, args) in [
        ("c", vec!["--rename-columns", "WT", "--rename-except", "ID"]),
        ("d", vec!["--rename-columns", "WT=A B"]),
    ] {
        let out = dir.join(tag);
        let r = bin().args(["run", "-i"]).arg(&input).arg("-o").arg(&out).args(&args).output().unwrap();
        assert!(!r.status.success(), "{tag}");
        assert!(!out.exists(), "{tag}: nothing is created when the options are invalid");
    }
}
