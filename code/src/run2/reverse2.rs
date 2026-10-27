//! Puts real values back into the NONMEM-style master dataset that was built from `run2`-masked datasets.
//!
//! The master dataset is made by an LLM (or by hand): rows are new, in any order, columns may be new, renamed
//! or merged. So nothing here depends on row position or on row keys. Every column is restored on its own,
//! value by value:
//!
//! * identifier columns (subject, study, site, treatment, ...): code -> real value, with the shared tables;
//! * date columns: the shift is taken off;
//! * scaled columns: the exact original number is recovered (the shortest decimal the forward pass maps to the
//!   masked text);
//! * which source column a master column came from is found by NAME, by `--map COMBINED=SOURCE` rules, and for
//!   identifiers also by looking at the values.
//!
//! Values the key cannot explain (derived, re-coded or transformed by the LLM) are left unchanged and reported.

use super::keymap2::{self, KeyMap2};
use crate::config::default_protect;
use crate::dataset::{read_dataset, write_dataset, Dataset, Delimiter};
use crate::obfuscate::{parse_date_prefix, unshift_date_cell};
use crate::reverse::{restore_number, ReverseArtifacts, ReverseOptions, Step};
use crate::trace::TraceLog;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

/// `--map COMBINED=SOURCE` or `--map COMBINED=SOURCE:WHENCOL=VALUE`.
#[derive(Clone, Debug, PartialEq)]
pub struct MapRule {
    pub combined: String,
    pub source: String,
    pub when: Option<(String, String)>,
}

pub fn parse_map_rules(items: &[String]) -> Result<Vec<MapRule>> {
    let mut v = Vec::new();
    for item in items {
        for part in item.split(',').map(str::trim).filter(|x| !x.is_empty()) {
            let Some((c, rest)) = part.split_once('=') else {
                bail!("--map '{part}': expected COMBINED=SOURCE (for example DV=AVAL) or COMBINED=SOURCE:WHENCOL=VALUE (DV=PCSTRESN:DVID=1)");
            };
            let (src, when) = match rest.split_once(':') {
                Some((s, w)) => {
                    let Some((wc, wv)) = w.split_once('=') else {
                        bail!("--map '{part}': the condition must look like WHENCOL=VALUE");
                    };
                    (s, Some((wc.trim().to_ascii_uppercase(), wv.trim().to_string())))
                }
                None => (rest, None),
            };
            let (c, src) = (c.trim().to_ascii_uppercase(), src.trim().to_ascii_uppercase());
            if c.is_empty() || src.is_empty() {
                bail!("--map '{part}': both names are needed");
            }
            v.push(MapRule { combined: c, source: src, when });
        }
    }
    Ok(v)
}

fn same_value(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    a == b || matches!((a.parse::<f64>(), b.parse::<f64>()), (Ok(x), Ok(y)) if x == y)
}

/// Digits at the end of a code with leading zeros removed: `SUBJ007` -> `7`.
fn suffix_number(code: &str) -> Option<String> {
    let digits: String = code.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
    if digits.is_empty() {
        return None;
    }
    let t = digits.trim_start_matches('0');
    Some(if t.is_empty() { "0".into() } else { t.to_string() })
}

struct DomainTables {
    name: String,
    /// masked -> original
    back: HashMap<String, String>,
    /// numeric suffix of the masked code -> original, only where the suffix is unique
    by_number: HashMap<String, String>,
    /// the masked codes contain letters, so they cannot be mistaken for ordinary numbers
    texty: bool,
    columns: Vec<String>,
}

fn tables(km: &KeyMap2) -> Vec<DomainTables> {
    km.domains
        .iter()
        .map(|d| {
            let back: HashMap<String, String> = d.pairs.iter().map(|(o, m)| (m.clone(), o.clone())).collect();
            let mut count: HashMap<String, usize> = HashMap::new();
            for (_, m) in &d.pairs {
                if let Some(n) = suffix_number(m) {
                    *count.entry(n).or_default() += 1;
                }
            }
            let by_number: HashMap<String, String> = d
                .pairs
                .iter()
                .filter_map(|(o, m)| suffix_number(m).filter(|n| count[n] == 1).map(|n| (n, o.clone())))
                .collect();
            let texty = d.pairs.iter().any(|(_, m)| m.chars().any(|c| c.is_alphabetic()));
            DomainTables { name: d.name.clone(), back, by_number, texty, columns: d.columns.clone() }
        })
        .collect()
}

#[derive(Default)]
struct ColStat {
    treatment: String,
    restored: usize,
    unknown: usize,
    unknown_samples: Vec<String>,
}

struct ScaleUse {
    divide: bool,
    factor: f64,
    /// condition: (index of the column to look at, value)
    when: Option<(usize, String)>,
}

struct ColPlan {
    domain: Option<usize>,
    by_suffix: bool,
    date: Option<bool>, // Some(true): named date column (numbers too); Some(false): auto-detected text dates only
    scales: Vec<ScaleUse>,
    stat: ColStat,
}

fn distinct_nonempty(ds: &Dataset, j: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut v = Vec::new();
    for r in &ds.rows {
        let c = r.get(j).map(|s| s.trim()).unwrap_or("");
        if c.is_empty() || c == "." {
            continue;
        }
        if seen.insert(c.to_string()) {
            v.push(c.to_string());
        }
    }
    v
}

pub fn reverse2(opts: &ReverseOptions, log: &TraceLog, keymap_path: &Path, km_text: &str) -> Result<ReverseArtifacts> {
    let on = |s: Step| opts.steps.contains(&s);
    let km = keymap2::parse(km_text).with_context(|| format!("reading {}", keymap_path.display()))?;
    log.info(format!("reverse start (run2 key) input={} output={}", opts.input.display(), opts.output_dir.display()));
    log.success(format!(
        "read key {} (program {}, level {}, {} dataset(s), {} identifier domain(s), {} scaled column(s), {} date column(s))",
        keymap_path.display(),
        km.program.as_deref().unwrap_or("unknown"),
        km.level.map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
        km.files.len(),
        km.domains.len(),
        km.scales.len(),
        km.date_columns.len()
    ));
    if opts.keymap_columns.is_some() {
        log.warn("--keymap-columns belongs to `run` level 4 and is ignored for a run2 key");
    }
    fs::create_dir_all(&opts.output_dir).with_context(|| format!("creating output dir {}", opts.output_dir.display()))?;

    let ds = read_dataset(&opts.input, opts.delimiter, opts.no_header, &opts.columns, &opts.comment_chars)
        .with_context(|| format!("reading {}", opts.input.display()))?;
    log.success(format!("read {} ({} rows x {} columns)", opts.input.display(), ds.n_rows(), ds.n_cols()));

    let rules = parse_map_rules(&opts.map)?;
    let tabs = tables(&km);
    let protect: Vec<String> = default_protect().into_iter().map(|s| s.to_ascii_uppercase()).collect();
    let extra_dates: Vec<String> = opts.date_columns.iter().map(|s| s.trim().to_ascii_uppercase()).collect();

    let mut warnings: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let warn = |m: String, log: &TraceLog, w: &mut Vec<String>| {
        log.warn(&m);
        w.push(m);
    };

    // ---- decide, for every column of the master dataset, what to undo
    let mut plans: Vec<ColPlan> = Vec::new();
    for (j, h) in ds.headers.iter().enumerate() {
        let up = h.trim().to_ascii_uppercase();
        let mine: Vec<&MapRule> = rules.iter().filter(|r| r.combined == up).collect();
        let alias = mine.iter().rev().find(|r| r.when.is_none()).map(|r| r.source.clone());
        let source = alias.clone().unwrap_or_else(|| up.clone());
        let mut plan = ColPlan { domain: None, by_suffix: false, date: None, scales: Vec::new(), stat: ColStat::default() };

        // identifier domain: by name (or alias), else by what the values are
        let by_name: Vec<usize> = tabs.iter().enumerate().filter(|(_, t)| t.columns.contains(&source)).map(|(i, _)| i).collect();
        let values = distinct_nonempty(&ds, j);
        let overlap = |t: &DomainTables| values.iter().filter(|v| t.back.contains_key(*v)).count();
        if !by_name.is_empty() {
            plan.domain = by_name.iter().copied().max_by_key(|&i| overlap(&tabs[i]));
            plan.by_suffix = alias.is_some();
            plan.stat.treatment = if alias.is_some() { format!("identifier ({}) via --map", tabs[plan.domain.unwrap()].name) } else { format!("identifier ({})", tabs[plan.domain.unwrap()].name) };
        } else if alias.is_none() && !protect.contains(&up) && !values.is_empty() {
            let best = tabs.iter().enumerate().filter(|(_, t)| t.texty).max_by_key(|(_, t)| overlap(t));
            if let Some((i, t)) = best {
                let n = overlap(t);
                if n > 0 && n * 10 >= values.len() * 8 {
                    plan.domain = Some(i);
                    plan.stat.treatment = format!("identifier ({}) found by its values", t.name);
                }
            }
        }
        // dates
        if plan.domain.is_none() {
            if km.date_columns.contains(&source) || extra_dates.contains(&up) || extra_dates.contains(&source) {
                plan.date = Some(true);
                plan.stat.treatment = "date".into();
            }
        }
        // scaling
        if plan.domain.is_none() && plan.date.is_none() {
            if let Some(s) = km.scales.iter().find(|s| s.column == source) {
                plan.scales.push(ScaleUse { divide: s.divide, factor: s.factor, when: None });
                plan.stat.treatment = if alias.is_some() { format!("scaled ({}) via --map", s.column) } else { "scaled".into() };
            }
        }
        for r in mine.iter().filter(|r| r.when.is_some()) {
            let Some(s) = km.scales.iter().find(|s| s.column == r.source) else {
                warn(format!("--map {}={}: {} is not a scaled column in the key, so there is nothing to undo", r.combined, r.source, r.source), log, &mut warnings);
                continue;
            };
            let (wc, wv) = r.when.clone().unwrap();
            let Some(wj) = ds.col_index(&wc) else {
                warn(format!("--map {}={}:{}={}: the dataset has no column {wc}", r.combined, r.source, wc, wv), log, &mut warnings);
                continue;
            };
            plan.scales.push(ScaleUse { divide: s.divide, factor: s.factor, when: Some((wj, wv)) });
            if plan.stat.treatment.is_empty() {
                plan.stat.treatment = "scaled (by condition)".into();
            } else {
                plan.stat.treatment.push_str(" + conditional");
            }
        }
        // text dates the key does not name (a date column the LLM created or renamed)
        if plan.domain.is_none() && plan.date.is_none() && plan.scales.is_empty() && !protect.contains(&up) && !values.is_empty() {
            let looks = values.iter().filter(|v| v.parse::<f64>().is_err() && parse_date_prefix(v).is_some()).count();
            if looks * 10 >= values.len() * 9 {
                plan.date = Some(false);
                plan.stat.treatment = "date found by its values".into();
            }
        }
        if plan.stat.treatment.is_empty() {
            plan.stat.treatment = "unchanged".into();
        }
        plans.push(plan);
    }

    // numbers that look like the digits of a masked code but were not mapped
    let mut hints: Vec<String> = Vec::new();
    for (j, h) in ds.headers.iter().enumerate() {
        if plans[j].stat.treatment != "unchanged" || protect.contains(&h.trim().to_ascii_uppercase()) {
            continue;
        }
        let values = distinct_nonempty(&ds, j);
        if values.len() < 2 || !values.iter().all(|v| v.parse::<i64>().is_ok()) {
            continue;
        }
        for t in tabs.iter().filter(|t| t.texty && !t.by_number.is_empty()) {
            let n = values.iter().filter(|v| t.by_number.contains_key(v.trim_start_matches('0')) || (v.as_str() == "0" && t.by_number.contains_key("0"))).count();
            if n * 10 >= values.len() * 9 {
                let col = t.columns.first().cloned().unwrap_or_else(|| t.name.to_ascii_uppercase());
                hints.push(format!(
                    "column {h} holds whole numbers that match the digits of the masked {} codes ({} of {} values). If it is a subject number built from {col}, restore it with --map {}={col}",
                    t.name,
                    n,
                    values.len(),
                    h.trim().to_ascii_uppercase()
                ));
                break;
            }
        }
    }
    for h in &hints {
        warn(h.clone(), log, &mut warnings);
    }

    // ---- restore, cell by cell
    let mut out_rows: Vec<Vec<String>> = Vec::with_capacity(ds.rows.len());
    let mut bad_dates: HashMap<usize, (usize, String)> = HashMap::new();
    for row in &ds.rows {
        let mut new_row = row.clone();
        for (j, plan) in plans.iter_mut().enumerate() {
            let Some(cell) = new_row.get(j).cloned() else { continue };
            let t = cell.trim();
            if t.is_empty() || t == "." {
                continue;
            }
            if let (Some(di), true) = (plan.domain, on(Step::Recoding)) {
                let tab = &tabs[di];
                let found = tab.back.get(t).cloned().or_else(|| {
                    if plan.by_suffix {
                        t.parse::<i64>().ok().and_then(|_| tab.by_number.get(t.trim_start_matches('0').to_string().as_str()).cloned().or_else(|| (t == "0").then(|| tab.by_number.get("0").cloned()).flatten()))
                    } else {
                        None
                    }
                });
                match found {
                    Some(o) => {
                        new_row[j] = o;
                        plan.stat.restored += 1;
                    }
                    None => {
                        plan.stat.unknown += 1;
                        if plan.stat.unknown_samples.len() < 5 && !plan.stat.unknown_samples.contains(&t.to_string()) {
                            plan.stat.unknown_samples.push(t.to_string());
                        }
                    }
                }
                continue;
            }
            if let (Some(named), true) = (plan.date, on(Step::Dates)) {
                if km.date_shift == 0.0 {
                    continue;
                }
                if !named && (t.parse::<f64>().is_ok() || parse_date_prefix(t).is_none()) {
                    continue;
                }
                let (r, ok) = unshift_date_cell(&cell, km.date_shift, km.date_whole);
                if ok {
                    new_row[j] = r;
                    plan.stat.restored += 1;
                } else {
                    let e = bad_dates.entry(j).or_insert((0, t.to_string()));
                    e.0 += 1;
                    plan.stat.unknown += 1;
                }
                continue;
            }
            if !plan.scales.is_empty() && on(Step::Scaling) {
                let Ok(x) = t.parse::<f64>() else { continue };
                if !x.is_finite() {
                    continue;
                }
                let pick = plan.scales.iter().find(|s| match &s.when {
                    None => true,
                    Some((wj, wv)) => row.get(*wj).map(|c| same_value(c, wv)).unwrap_or(false),
                });
                if let Some(s) = pick {
                    new_row[j] = restore_number(x, s.divide, s.factor);
                    plan.stat.restored += 1;
                }
            }
        }
        out_rows.push(new_row);
    }

    // ---- problems
    let mut problems: Vec<String> = Vec::new();
    for (j, plan) in plans.iter().enumerate() {
        let h = &ds.headers[j];
        if plan.domain.is_some() && plan.stat.unknown > 0 {
            problems.push(format!(
                "column {h}: {} value(s) are not codes of the masked datasets (for example {}) and were left unchanged - the LLM changed or invented them",
                plan.stat.unknown,
                plan.stat.unknown_samples.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", ")
            ));
        }
        if let Some((n, first)) = bad_dates.get(&j) {
            problems.push(format!("column {h}: {n} date value(s) could not be read (for example '{first}') and were left unchanged"));
        }
    }
    if opts.strict && !problems.is_empty() {
        bail!("{} (--strict: stopping instead of leaving values unchanged)", problems[0]);
    }
    for p in problems {
        warn(p, log, &mut warnings);
    }

    // ---- columns of the key that the master dataset does not have
    let have = |name: &str| ds.headers.iter().any(|h| h.trim().eq_ignore_ascii_case(name)) || rules.iter().any(|r| r.source == name.to_ascii_uppercase());
    let mut missing: Vec<String> = Vec::new();
    for d in &km.domains {
        for c in &d.columns {
            if !have(c) {
                missing.push(format!("{c} ({} codes)", d.name));
            }
        }
    }
    for c in &km.date_columns {
        if !have(c) {
            missing.push(format!("{c} (dates)"));
        }
    }
    for s in &km.scales {
        if !have(&s.column) {
            missing.push(format!("{} (scaled)", s.column));
        }
    }
    if !missing.is_empty() {
        let m = format!("columns of the key that are not in the master dataset (nothing to restore, or they were renamed - use --map): {}", missing.join(", "));
        notes.push(m.clone());
        log.info(m);
    }

    // ---- write
    let out = Dataset {
        headers: ds.headers.clone(),
        rows: out_rows,
        delimiter: ds.delimiter,
        comments: Vec::new(),
        source_name: "restored".into(),
        ragged_rows: ds.ragged_rows,
    };
    let out_name = opts.output_name.clone().unwrap_or_else(|| match ds.delimiter {
        Delimiter::Tab => "restored.tsv".into(),
        Delimiter::Whitespace => "restored.dat".into(),
        _ => "restored.csv".into(),
    });
    let dataset_path = opts.output_dir.join(&out_name);
    write_dataset(&dataset_path, &out)?;
    let restored_sha256 = hex::encode(Sha256::digest(&fs::read(&dataset_path)?));
    log.success(format!("wrote restored dataset {} ({} rows x {} cols), SHA-256 {restored_sha256}", dataset_path.display(), out.n_rows(), out.n_cols()));

    let mut rep = String::new();
    rep.push_str("data-obfuscator reverse (run2 key) - report. PRIVATE: the restored dataset holds real data again.\n");
    rep.push_str(&"=".repeat(70));
    rep.push('\n');
    rep.push_str(&format!("Input           : {}\n", opts.input.display()));
    rep.push_str(&format!("Key             : {} (program {}, level {})\n", keymap_path.display(), km.program.as_deref().unwrap_or("unknown"), km.level.map(|l| l.to_string()).unwrap_or_else(|| "?".into())));
    rep.push_str(&format!("Steps           : {}\n", opts.steps.iter().filter(|s| **s != Step::Rename).map(|s| s.name()).collect::<Vec<_>>().join(", ")));
    rep.push_str(&format!("Restored file   : {}\nRestored SHA-256: {restored_sha256}\n\n", dataset_path.display()));
    rep.push_str("Columns of the dataset\n");
    for (j, h) in ds.headers.iter().enumerate() {
        let st = &plans[j].stat;
        let extra = if st.restored > 0 || st.unknown > 0 { format!("  restored {} value(s){}", st.restored, if st.unknown > 0 { format!(", {} not recognised", st.unknown) } else { String::new() }) } else { String::new() };
        rep.push_str(&format!("  {:<16} {}{}\n", h, st.treatment, extra));
    }
    if !km.not_restorable.is_empty() {
        rep.push_str("\nCannot be restored (removed when masking)\n");
        for (f, c, why) in &km.not_restorable {
            rep.push_str(&format!("  {f}: {c} - {why}\n"));
        }
    }
    for n in &notes {
        rep.push_str(&format!("\nNote: {n}\n"));
    }
    if !warnings.is_empty() {
        rep.push_str("\nWarnings\n");
        for w in &warnings {
            rep.push_str(&format!("  - {w}\n"));
        }
    }
    rep.push_str("\nWhat this cannot do: values the LLM derived or transformed (a log of DV, a unit conversion, a re-coded SEX) are not\n");
    rep.push_str("recognised and stay as they are; free text and dropped columns were removed when masking; a master column that\n");
    rep.push_str("mixes several scaled source columns needs --map COMBINED=SOURCE:WHENCOL=VALUE rules to be restored.\n");
    let report_path = opts.output_dir.join("reverse_report.txt");
    fs::write(&report_path, rep).with_context(|| format!("writing {}", report_path.display()))?;
    log.success(format!("wrote {}", report_path.display()));

    Ok(ReverseArtifacts {
        dataset_path,
        report_path,
        log_path: log.path().to_path_buf(),
        dataset: out,
        warnings: log.warn_count(),
        restored_sha256,
    })
}
