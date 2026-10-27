//! The masking engine of `run2`: several datasets, one shared set of codes, one date shift, one factor per
//! scaled column name. Uses the building blocks of `run` (column planning, code formats, date shifting,
//! number formatting) so a value is masked exactly as `run` would mask it.

use super::args::Run2Options;
use super::keymap2::{clean, Domain, KeyMap2, Scale2};
use crate::config::RunOptions;
use crate::dataset::Dataset;
use crate::detect::{detect_role, output_headers, plan_columns, ColumnPlan, Role};
use crate::obfuscate::{distinct_values, recode_value, scale_cell, shift_date_cell};
use crate::trace::TraceLog;
use anyhow::{bail, Result};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use std::collections::{HashMap, HashSet};

pub struct Input {
    /// File name as given (used in keymap2.txt and messages).
    pub name: String,
    pub ds: Dataset,
}

pub struct Masked2 {
    pub outputs: Vec<Dataset>,
    pub plans: Vec<Vec<ColumnPlan>>,
    /// Everything of keymap2.txt except the file table (the runner fills that in after writing the files).
    pub key: KeyMap2,
    /// Columns recognised by the ADaM hints, per input: (column, what it was taken for).
    pub hints: Vec<Vec<(String, &'static str)>>,
}

/// The identifier kinds that get a shared table.
fn domain_name(role: Role) -> Option<&'static str> {
    Some(match role {
        Role::Id => "id",
        Role::Subject => "subject",
        Role::Study => "study",
        Role::Site => "site",
        Role::Treatment => "treatment",
        Role::User => "user",
        Role::Sex => "sex",
        Role::Race => "race",
        Role::Country => "country",
        _ => return None,
    })
}

fn explicit(o: &RunOptions, name: &str) -> bool {
    let has = |v: &Vec<String>| v.iter().any(|x| x.trim().eq_ignore_ascii_case(name.trim()));
    has(&o.id_column)
        || has(&o.subject_column)
        || has(&o.study_column)
        || has(&o.site_column)
        || has(&o.treatment_column)
        || has(&o.user_column)
        || has(&o.date_columns)
        || has(&o.sensitive)
        || has(&o.protect)
        || has(&o.scale_columns)
        || has(&o.freetext_columns)
}

/// CDISC ADaM / SDTM naming: --DT --DTM --DTC columns are dates, TRT.. columns are treatments, INVID is a user.
/// Only columns that `run`'s own detection leaves as "other" and that the user did not name are taken.
fn adam_hints(headers: &[String], o: &mut RunOptions) -> Vec<(String, &'static str)> {
    let mut found = Vec::new();
    for h in headers {
        if explicit(o, h) || detect_role(h) != Role::Other {
            continue;
        }
        let n = h.trim().to_ascii_uppercase();
        let is_date = ["DT", "DTM", "DTC", "DTTM", "DTMC", "DTS"].iter().any(|suf| n.ends_with(suf) && n.len() > suf.len());
        if is_date {
            o.date_columns.push(h.clone());
            found.push((h.clone(), "date"));
        } else if n.starts_with("TRT") && !n.ends_with("DY") && !n.ends_with("DUR") {
            o.treatment_column.push(h.clone());
            found.push((h.clone(), "treatment"));
        } else if matches!(n.as_str(), "INVID" | "INVNAM") {
            o.user_column.push(h.clone());
            found.push((h.clone(), "user"));
        }
    }
    found
}

fn unique_code(code: String, used: &HashSet<String>, n: usize) -> String {
    if !used.contains(&code) {
        return code;
    }
    let mut k = n;
    loop {
        let c = format!("{code}_{k}");
        if !used.contains(&c) {
            return c;
        }
        k += 1;
    }
}

pub fn mask_all(inputs: &[Input], opts: &Run2Options, log: &TraceLog) -> Result<Masked2> {
    let base = &opts.base;
    if inputs.is_empty() {
        bail!("no input datasets");
    }
    let mut hints: Vec<Vec<(String, &'static str)>> = Vec::new();
    let mut plans: Vec<Vec<ColumnPlan>> = Vec::new();
    for inp in inputs {
        let mut o = base.clone();
        let h = if opts.adam_hints { adam_hints(&inp.ds.headers, &mut o) } else { Vec::new() };
        for (c, what) in &h {
            log.info(format!("{}: column {c} recognised as {what} (ADaM naming)", inp.name));
        }
        hints.push(h);
        plans.push(plan_columns(&inp.ds, &o));
    }

    let mut rng = ChaCha20Rng::seed_from_u64(base.seed);
    let date_shift = if base.level >= 2 {
        let lo = base.date_shift_min.min(base.date_shift_max);
        let hi = base.date_shift_min.max(base.date_shift_max);
        if lo == hi {
            lo
        } else {
            // Whole days: date-only columns (ADT) and date-time columns (ADTM) then move by exactly the same
            // amount, so intervals between them stay exact when the datasets are combined.
            rng.gen_range(lo..=hi).round().clamp(lo.ceil(), hi.floor().max(lo.ceil()))
        }
    } else {
        0.0
    };
    if date_shift.fract() != 0.0 {
        log.warn(format!(
            "date shift {date_shift} is not a whole number of days: date-only columns move by {} day(s) and date-time columns by {date_shift}, so intervals between a date-only and a date-time column will be off by {:.2} day(s) when the datasets are combined. Use a whole number of days",
            date_shift.floor(),
            date_shift.fract()
        ));
    }
    let date_whole = date_shift.floor() as i64;
    if date_shift != 0.0 {
        log.info(format!("date shift (one value for all {} dataset(s), intervals preserved): {date_shift} days", inputs.len()));
    }

    // One factor per scaled column NAME, the same in every dataset that has the column.
    let mut scales: Vec<Scale2> = Vec::new();
    for plan in &plans {
        for p in plan.iter().filter(|p| p.scale && !p.drop && !p.protect) {
            let key = p.name.trim().to_ascii_uppercase();
            if scales.iter().any(|s| s.column == key) {
                continue;
            }
            let factor = if base.scale_max > base.scale_min { rng.gen_range(base.scale_min..=base.scale_max) } else { base.scale_min };
            let divide = rng.gen_bool(0.5);
            scales.push(Scale2 { column: key, divide, factor });
        }
    }
    for s in &scales {
        log.info(format!(
            "scaling column {}: every valid number in every dataset will be {} {}",
            s.column,
            if s.divide { "divided by" } else { "multiplied by" },
            s.factor
        ));
    }

    // Shared code tables, one per identifier domain, in order of first appearance across the datasets.
    struct Build {
        dom: Domain,
        map: HashMap<String, String>,
        used: HashSet<String>,
        role: Role,
    }
    let mut builds: Vec<Build> = Vec::new();
    for (fi, inp) in inputs.iter().enumerate() {
        for p in plans[fi].iter().filter(|p| p.recode && p.role != Role::Date && !p.drop) {
            let Some(dname) = domain_name(p.role) else { continue };
            let bi = match builds.iter().position(|b| b.dom.name == dname) {
                Some(i) => i,
                None => {
                    builds.push(Build {
                        dom: Domain { name: dname.to_string(), ..Default::default() },
                        map: HashMap::new(),
                        used: HashSet::new(),
                        role: p.role,
                    });
                    builds.len() - 1
                }
            };
            let b = &mut builds[bi];
            let col = p.name.trim().to_ascii_uppercase();
            if !b.dom.columns.contains(&col) {
                b.dom.columns.push(col);
            }
            for raw in distinct_values(&inp.ds, p.index) {
                if b.map.contains_key(&raw) {
                    continue;
                }
                let n = b.dom.pairs.len() + 1;
                let code = unique_code(recode_value(b.role, &raw, n, base.id_style, base.seed), &b.used, n);
                b.used.insert(code.clone());
                b.map.insert(raw.clone(), code.clone());
                b.dom.pairs.push((raw, code));
            }
        }
    }
    for b in &builds {
        log.success(format!(
            "domain {}: {} unique value(s) in column(s) {}, one code table shared by all datasets",
            b.dom.name,
            b.dom.pairs.len(),
            b.dom.columns.join(", ")
        ));
    }

    // Mask every dataset.
    let mut outputs = Vec::new();
    let mut date_columns: Vec<String> = Vec::new();
    let mut not_restorable: Vec<(String, String, String)> = Vec::new();
    for (fi, inp) in inputs.iter().enumerate() {
        let plan = &plans[fi];
        let ds = &inp.ds;
        let mut unshifted: Vec<(String, usize, usize)> = Vec::new();
        let mut rows = Vec::with_capacity(ds.rows.len());
        for (row_no, row) in ds.rows.iter().enumerate() {
            let mut new_row = row.clone();
            for p in plan {
                if p.drop || p.protect {
                    continue;
                }
                let Some(cell) = new_row.get_mut(p.index) else { continue };
                if p.redact {
                    *cell = ".".into();
                    continue;
                }
                if p.role == Role::Date && p.recode {
                    let (shifted, ok) = shift_date_cell(cell, date_shift);
                    if !ok {
                        match unshifted.iter_mut().find(|u| u.0 == p.name) {
                            Some(u) => u.1 += 1,
                            None => unshifted.push((p.name.clone(), 1, row_no)),
                        }
                    }
                    *cell = shifted;
                    continue;
                }
                if let Some(dname) = domain_name(p.role).filter(|_| p.recode) {
                    if let Some(b) = builds.iter().find(|b| b.dom.name == dname) {
                        if let Some(v) = b.map.get(cell.trim()) {
                            *cell = v.clone();
                        }
                    }
                    continue;
                }
                if p.scale {
                    let key = p.name.trim().to_ascii_uppercase();
                    if let Some(s) = scales.iter().find(|s| s.column == key) {
                        if let Some(v) = scale_cell(cell, s.divide, s.factor) {
                            *cell = v;
                        }
                    }
                }
            }
            rows.push(new_row);
        }
        for (name, n, first) in &unshifted {
            log.warn(format!(
                "{}: date column {name}: {n} value(s) are not in a recognised date format and were left UNSHIFTED (first at data row {}) - they still show the real date; fix or drop them before sharing",
                inp.name,
                first + 1
            ));
        }
        for p in plan {
            if p.role == Role::Date && p.recode && !p.drop {
                let c = p.name.trim().to_ascii_uppercase();
                if !date_columns.contains(&c) {
                    date_columns.push(c);
                }
            }
            if p.redact && !p.drop {
                not_restorable.push((inp.name.clone(), p.name.clone(), "free text redacted (.)".into()));
            }
            if p.drop {
                not_restorable.push((inp.name.clone(), p.name.clone(), "column dropped".into()));
            }
        }
        let headers = output_headers(plan, &None, &ds.headers);
        if headers.is_empty() {
            bail!("{}: no columns remain after drop - refusing to write an empty dataset", inp.name);
        }
        let idx: Vec<usize> = headers.iter().filter_map(|h| ds.col_index(h)).collect();
        let identity = idx.len() == ds.n_cols() && idx.iter().enumerate().all(|(a, &b)| a == b);
        let projected: Vec<Vec<String>> = rows
            .into_iter()
            .map(|r| if identity { r } else { idx.iter().map(|&i| r.get(i).cloned().unwrap_or_default()).collect() })
            .collect();
        outputs.push(Dataset {
            headers,
            rows: projected,
            delimiter: ds.delimiter,
            comments: Vec::new(),
            source_name: "masked".into(),
            ragged_rows: if identity { ds.ragged_rows } else { 0 },
        });
    }

    let key = KeyMap2 {
        program: Some(crate::APP_VERSION.to_string()),
        seed: Some(base.seed),
        level: Some(base.level),
        date_shift,
        date_whole,
        files: Vec::new(),
        domains: builds.into_iter().map(|b| b.dom).collect(),
        date_columns,
        scales,
        not_restorable: {
            let mut v = not_restorable;
            v.iter_mut().for_each(|t| t.1 = clean(&t.1));
            v
        },
    };
    Ok(Masked2 { outputs, plans, key, hints })
}
