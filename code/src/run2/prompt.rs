//! llm2.txt (what to paste to an LLM) and summary2.txt (private run record) for `run2`.

use super::args::Run2Options;
use super::keymap2::{FileEntry, KeyMap2};
use crate::dataset::Dataset;
use crate::detect::{ColumnPlan, Role};

/// One line per kept column of a masked file: how it was treated.
fn column_notes(plan: &[ColumnPlan], out: &Dataset, key: &KeyMap2) -> Vec<String> {
    let mut v = Vec::new();
    for h in &out.headers {
        let Some(p) = plan.iter().find(|p| p.name.eq_ignore_ascii_case(h)) else { continue };
        let up = h.trim().to_ascii_uppercase();
        let note = if p.redact {
            Some("free text removed (every value is '.')".to_string())
        } else if p.role == Role::Date && p.recode {
            Some("dates moved by one constant number of days (the same in every dataset; intervals and order are real)".into())
        } else if let Some(d) = key.domains.iter().find(|d| d.columns.contains(&up)) {
            Some(format!("{} codes - the same real value has the same code in every dataset", d.name))
        } else if key.scales.iter().any(|s| s.column == up) {
            Some("numbers multiplied or divided by a hidden constant (not in real units; ratios within the column are real)".into())
        } else {
            None
        };
        if let Some(n) = note {
            v.push(format!("      - {h}: {n}"));
        }
    }
    v
}

pub fn render(files: &[FileEntry], outputs: &[Dataset], plans: &[Vec<ColumnPlan>], _hints: &[Vec<(String, &'static str)>], key: &KeyMap2, level: u8) -> String {
    let mut s = String::new();
    s.push_str(&format!("# data-obfuscator {} - run2 prompts\n", crate::APP_VERSION));
    s.push_str("# Paste ONE prompt below into your LLM together with the *_masked files. Never share keymap2.txt or summary2.txt.\n\n");

    s.push_str("## What was masked\n\n");
    s.push_str(&format!("{} dataset(s) were masked together, level {level}. Masking is consistent across the datasets:\n", files.len()));
    s.push_str("- identifiers (subject, study, site, ...) have ONE code table: the same real subject has the same code in every file, so the files can be joined on those columns;\n");
    if key.date_shift != 0.0 {
        s.push_str("- every date was moved by the SAME number of days in every file, so differences between dates (time since first dose, visit windows) and their order are real;\n");
    }
    if !key.scales.is_empty() {
        s.push_str(&format!(
            "- the numeric column(s) {} were multiplied or divided by a hidden constant (one constant per column name, the same in every file);\n",
            key.scales.iter().map(|s| s.column.clone()).collect::<Vec<_>>().join(", ")
        ));
    }
    s.push_str("- free text was removed; nothing in the files can identify a person.\n\n");

    s.push_str("## The datasets\n\n");
    for (i, f) in files.iter().enumerate() {
        let out = &outputs[i];
        s.push_str(&format!("{}. {} - {} rows, {} columns\n", i + 1, f.masked, out.n_rows(), out.n_cols()));
        s.push_str(&format!("   columns: {}\n", out.headers.join(", ")));
        let notes = column_notes(&plans[i], out, key);
        if !notes.is_empty() {
            s.push_str("   masked columns:\n");
            for n in notes {
                s.push_str(&n);
                s.push('\n');
            }
        }
    }
    s.push('\n');

    s.push_str("## Prompt 1 - build the NONMEM-style master dataset\n\n```\n");
    s.push_str("You are an experienced pharmacometrician. I attach several masked ADaM-style datasets from one clinical\n");
    s.push_str("pharmacology study (listed above). Combine them into ONE NONMEM-ready master dataset: one row per dosing or\n");
    s.push_str("observation record, with ID, TIME, TAD, AMT, RATE (if infused), DV, EVID, MDV, CMT (and DVID for several\n");
    s.push_str("endpoints), plus the covariates I will need. Write the code (R or Python) that does it and run it on the files.\n\n");
    s.push_str("The data are masked. Please follow these rules, because I will later put the real values back into your\n");
    s.push_str("result with a private key, and that only works if the masked values stay as they are:\n\n");
    s.push_str("1. Treat every identifier (subject, study, site, treatment, ...) as an opaque code. Join datasets on the\n");
    s.push_str("   columns they share (for example STUDYID + USUBJID). Do not change, re-format or re-order the codes.\n");
    s.push_str("2. Keep the masked identifier columns in the master dataset under their ORIGINAL column names (USUBJID, STUDYID,\n");
    s.push_str("   SITEID, ARM, ...) next to the numeric ID you create. Derive the numeric NONMEM ID from the digits of the\n");
    s.push_str("   masked USUBJID (for example SUBJ007 gives ID 7) instead of renumbering subjects 1..N.\n");
    if key.date_shift != 0.0 {
        s.push_str("3. Dates are shifted by a constant, which does not matter for TIME: compute TIME, TAD and the study day from\n");
        s.push_str("   DIFFERENCES between dates and times. If you keep a calendar date column in the master dataset keep its\n");
        s.push_str("   original column name and its ISO format (YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS).\n");
    } else {
        s.push_str("3. If you keep a calendar date column in the master dataset keep its original column name and format.\n");
    }
    if !key.scales.is_empty() {
        s.push_str(&format!(
            "4. The values of {} are not in real units. Do not convert units, apply reference ranges or transform them in place.\n   Put any derived value (log, normalised, categorised) in a NEW column and keep the masked column as it is. If one\n   master column (for example DV) takes values from several of these columns, add a column (DVID or SRC) that says\n   which source column each row came from.\n",
            key.scales.iter().map(|s| s.column.clone()).collect::<Vec<_>>().join(", ")
        ));
    } else {
        s.push_str("4. If one master column (for example DV) takes values from several source columns, add a column (DVID or SRC)\n   that says which source column each row came from.\n");
    }
    s.push_str("5. Do not guess real identities, dates or units. If something is missing or ambiguous, say so instead of inventing it.\n\n");
    s.push_str("Return: (a) the master dataset as CSV, (b) the code, (c) a table of every master column and the source column and\n");
    s.push_str("rule it came from, (d) a list of assumptions and open questions, (e) record counts per source dataset and per\n");
    s.push_str("EVID in the master dataset, so I can reconcile them with the sources.\n```\n\n");

    s.push_str("## Prompt 2 - check the master dataset against the sources\n\n```\n");
    s.push_str("Here are the masked source datasets and the master dataset you built. Check the master dataset: every subject\n");
    s.push_str("in the sources appears; record counts reconcile; TIME is non-negative within a subject and non-decreasing;\n");
    s.push_str("each subject's first record is a dose when there are observations after it; EVID/MDV/CMT/DVID combinations are\n");
    s.push_str("valid; no DV for dosing records; AMT only on dosing records; no duplicated ID+TIME+EVID+CMT records unless\n");
    s.push_str("intended; covariates are constant within subject unless time-varying by design. List every problem with the row\n");
    s.push_str("numbers and the source records involved. Do not change the masked values.\n```\n\n");

    s.push_str("## Getting the real values back\n\n");
    s.push_str("Save the master dataset the LLM returns, then on your computer, with the private key:\n\n");
    s.push_str("    data-obfuscator reverse --input master.csv --keymap keymap2.txt --output ./restored\n\n");
    s.push_str("Columns that kept their original names are restored automatically. If the LLM renamed or merged columns, tell\n");
    s.push_str("`reverse` where they came from with --map, for example --map ID=USUBJID (a numeric ID built from the digits of\n");
    s.push_str("the masked USUBJID) or --map DV=AVAL.\n");
    s
}

pub fn summary(files: &[FileEntry], outputs: &[Dataset], plans: &[Vec<ColumnPlan>], key: &KeyMap2, opts: &Run2Options) -> String {
    let mut s = String::new();
    s.push_str(&format!("data-obfuscator {} - run2 summary (PRIVATE: shows the seed and the date shift)\n", crate::APP_VERSION));
    s.push_str(&"=".repeat(70));
    s.push('\n');
    s.push_str(&format!("Level           : {} ({})\n", opts.base.level, opts.base.level_name()));
    s.push_str(&format!("Seed            : {}\n", opts.base.seed));
    s.push_str(&format!("Date shift      : {} day(s), whole-day part {}\n", key.date_shift, key.date_whole));
    s.push_str(&format!("ADaM hints      : {}\n", if opts.adam_hints { "on" } else { "off" }));
    s.push_str(&format!("Datasets        : {}\n\n", files.len()));
    for (i, f) in files.iter().enumerate() {
        s.push_str(&format!("{}. {} -> {}\n   {} rows x {} columns; SHA-256 {}\n", f.index, f.original, f.masked, f.rows, outputs[i].n_cols(), f.sha256));
        let kept: Vec<&str> = plans[i].iter().filter(|p| !p.drop).map(|p| p.name.as_str()).collect();
        let dropped: Vec<&str> = plans[i].iter().filter(|p| p.drop).map(|p| p.name.as_str()).collect();
        s.push_str(&format!("   kept: {}\n", kept.join(", ")));
        if !dropped.is_empty() {
            s.push_str(&format!("   dropped: {}\n", dropped.join(", ")));
        }
    }
    s.push_str("\nShared code tables\n");
    for d in &key.domains {
        s.push_str(&format!("  {:<10} {} value(s); columns {}\n", d.name, d.pairs.len(), d.columns.join(", ")));
    }
    if !key.date_columns.is_empty() {
        s.push_str(&format!("Date columns    : {}\n", key.date_columns.join(", ")));
    }
    for sc in &key.scales {
        s.push_str(&format!("Scaled          : {} {} {}\n", sc.column, if sc.divide { "divided by" } else { "multiplied by" }, sc.factor));
    }
    s.push_str("\nShare: the *_masked files and llm2.txt. Keep private: keymap2.txt, summary2.txt, config.used.txt, run2.log.\n");
    s
}
