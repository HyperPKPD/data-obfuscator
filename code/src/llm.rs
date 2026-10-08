//! `llm.txt` - ready-to-paste prompts tailored to the masked dataset.
//!
//! The dataset is profiled (subjects, record types, compartments, dose levels,
//! covariates, special NONMEM columns ...) and those facts are written into three
//! single-paragraph prompts: analyse the data, build a NONMEM control stream, and
//! perform quality-control checks.

use crate::dataset::{Dataset, Delimiter};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// What the prompt pack needs to know about the run besides the dataset itself.
pub struct LlmContext<'a> {
    pub file_name: &'a str,
    pub level: u8,
    pub level_name: &'a str,
    /// Columns whose values were multiplied or divided by a hidden constant (absolute values are not real).
    pub scaled: Vec<String>,
    /// New names of columns that were renamed to generic labels (their meaning is withheld).
    pub renamed: Vec<String>,
    /// (model label, estimated tokens of the masked dataset)
    pub token_estimates: Vec<(String, u64)>,
}

const ID_NAMES: &[&str] = &["ID", "NMID", "I"];
const TIME_NAMES: &[&str] = &["TIME"];
const DV_NAMES: &[&str] = &["DV", "CONC", "LNDV"];
const AMT_NAMES: &[&str] = &["AMT"];
const EVID_NAMES: &[&str] = &["EVID"];
const MDV_NAMES: &[&str] = &["MDV"];
const CMT_NAMES: &[&str] = &["CMT"];
const ARM_NAMES: &[&str] = &[
    "ARM", "ARMCD", "ACTARM", "TRT", "TRTA", "TRTP", "TREAT", "TREATMENT", "GROUP", "COHORT",
    "DOSEGRP", "TRTGRP",
];
const COV_CONT: &[&str] = &[
    "WT", "WGT", "WEIGHT", "AGE", "HT", "HEIGHT", "BMI", "CRCL", "EGFR", "SCR", "ALT", "AST",
    "BILI", "ALB", "BSA", "LBWT",
];
const COV_CAT: &[&str] = &["SEX", "GENDER", "SEXN", "RACE", "RACEN", "ETHNIC", "ETHNICITY", "COUNTRY"];
const CORE: &[&str] = &[
    "ID", "NMID", "I", "TIME", "TAD", "DV", "LNDV", "CONC", "AMT", "RATE", "EVID", "MDV", "CMT",
    "ADDL", "II", "SS", "DVID", "CENS", "LIMIT", "BLQ", "LLOQ", "OCC", "PC", "PCYC", "L2",
];

#[derive(Debug, Default)]
pub struct Profile {
    pub n_rows: usize,
    pub headers: Vec<String>,
    pub id_col: Option<String>,
    pub n_subjects: usize,
    pub time_col: Option<String>,
    pub time_range: Option<(f64, f64)>,
    pub dv_col: Option<String>,
    pub n_obs_records: usize,
    pub n_obs: usize,
    /// Observation records with no numeric DV, and how many of those still have MDV != 1.
    pub n_obs_no_dv: usize,
    pub n_obs_no_dv_mdv0: usize,
    pub dv_range: Option<(f64, f64)>,
    pub n_dv_nonpositive: usize,
    pub n_dose: usize,
    pub dose_levels: Vec<f64>,
    pub evid_values: Vec<String>,
    pub cmt_col: Option<String>,
    /// cmt value -> (observation records, dose records)
    pub cmt_counts: BTreeMap<String, (usize, usize)>,
    pub obs_per_subject: Option<(usize, f64, usize)>,
    pub has: BTreeSet<String>,
    pub infusion: bool,
    pub addl: bool,
    pub steady_state: bool,
    pub arm_col: Option<String>,
    pub arm_values: Vec<String>,
    pub cov_cont: Vec<String>,
    pub cov_cat: Vec<String>,
    pub cov_other: Vec<String>,
    pub non_numeric: Vec<String>,
    pub blq_cols: Vec<String>,
    /// DVID value -> observation records (only when the file has a DVID column).
    pub dvid: Vec<(String, usize)>,
    /// Columns rescaled by a hidden factor during masking (set by `render`).
    pub scaled: Vec<String>,
    /// Columns renamed to generic labels during masking (set by `render`).
    pub renamed: Vec<String>,
    /// Calendar date / clock-time columns that can be used to re-derive TIME.
    pub datetime_cols: Vec<String>,
    /// Planned dose column (DOSE, DOSELEVEL, ...) and nominal time column (NTIM, ...).
    pub dose_col: Option<String>,
    pub ntim_col: Option<String>,
}

fn find(ds: &Dataset, names: &[&str]) -> Option<usize> {
    names.iter().find_map(|n| ds.col_index(n))
}

fn num(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() || t == "." {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn upper(s: &str) -> String {
    s.trim().to_ascii_uppercase()
}

pub fn profile(ds: &Dataset) -> Profile {
    let mut p = Profile {
        n_rows: ds.n_rows(),
        headers: ds.headers.clone(),
        ..Default::default()
    };
    for h in &ds.headers {
        p.has.insert(upper(h));
    }

    let id_i = find(ds, ID_NAMES);
    let time_i = find(ds, TIME_NAMES);
    let dv_i = find(ds, DV_NAMES);
    let amt_i = find(ds, AMT_NAMES);
    let evid_i = find(ds, EVID_NAMES);
    let mdv_i = find(ds, MDV_NAMES);
    let cmt_i = find(ds, CMT_NAMES);
    let rate_i = ds.col_index("RATE");
    let addl_i = ds.col_index("ADDL");
    let ss_i = ds.col_index("SS");
    let arm_i = find(ds, ARM_NAMES);
    let dvid_i = ds.col_index("DVID");
    let mut dvid_counts: BTreeMap<String, usize> = BTreeMap::new();

    p.id_col = id_i.map(|i| ds.headers[i].clone());
    p.time_col = time_i.map(|i| ds.headers[i].clone());
    p.dv_col = dv_i.map(|i| ds.headers[i].clone());
    p.cmt_col = cmt_i.map(|i| ds.headers[i].clone());
    p.arm_col = arm_i.map(|i| ds.headers[i].clone());

    let cell = |row: &Vec<String>, i: Option<usize>| -> Option<String> {
        i.and_then(|i| row.get(i)).map(|s| s.trim().to_string())
    };

    let mut subjects: HashMap<String, usize> = HashMap::new();
    let mut evids: BTreeSet<String> = BTreeSet::new();
    let mut doses: Vec<f64> = Vec::new();
    let mut arms: BTreeSet<String> = BTreeSet::new();
    let (mut tmin, mut tmax) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut dmin, mut dmax) = (f64::INFINITY, f64::NEG_INFINITY);

    for row in &ds.rows {
        let id = cell(row, id_i);
        if let Some(id) = &id {
            if !id.is_empty() && id != "." {
                subjects.entry(id.clone()).or_insert(0);
            }
        }
        if let Some(t) = cell(row, time_i).as_deref().and_then(num) {
            tmin = tmin.min(t);
            tmax = tmax.max(t);
        }
        let evid = cell(row, evid_i).as_deref().and_then(num).map(|v| v.round() as i64);
        if let Some(e) = evid {
            evids.insert(e.to_string());
        }
        let amt = cell(row, amt_i).as_deref().and_then(num);
        let dv = cell(row, dv_i).as_deref().and_then(num);
        let mdv = cell(row, mdv_i).as_deref().and_then(num).map(|v| v.round() as i64);

        let is_dose = match evid {
            Some(1) | Some(4) => true,
            Some(_) => false,
            None => amt.is_some_and(|a| a > 0.0),
        };
        let is_obs_record = match evid {
            Some(0) => true,
            Some(_) => false,
            None => !is_dose,
        };
        let cmt = cell(row, cmt_i).filter(|c| !c.is_empty() && c != ".");

        if is_dose {
            p.n_dose += 1;
            if let Some(a) = amt.filter(|a| *a > 0.0) {
                doses.push(a);
            }
            if let Some(c) = &cmt {
                p.cmt_counts.entry(c.clone()).or_default().1 += 1;
            }
        }
        if is_obs_record {
            p.n_obs_records += 1;
            if let Some(v) = cell(row, dvid_i).filter(|v| !v.is_empty() && v != ".") {
                *dvid_counts.entry(v).or_insert(0) += 1;
            }
            if let Some(c) = &cmt {
                p.cmt_counts.entry(c.clone()).or_default().0 += 1;
            }
            if dv.is_none() {
                p.n_obs_no_dv += 1;
                if mdv != Some(1) {
                    p.n_obs_no_dv_mdv0 += 1;
                }
            }
            if dv.is_some() && mdv != Some(1) {
                p.n_obs += 1;
                if let Some(v) = dv {
                    dmin = dmin.min(v);
                    dmax = dmax.max(v);
                    if v <= 0.0 {
                        p.n_dv_nonpositive += 1;
                    }
                }
                if let Some(id) = id.as_ref().filter(|i| !i.is_empty() && *i != ".") {
                    *subjects.entry(id.clone()).or_insert(0) += 1;
                }
            }
        }
        if rate_i.is_some() && cell(row, rate_i).as_deref().and_then(num).is_some_and(|r| r > 0.0) {
            p.infusion = true;
        }
        if addl_i.is_some() && cell(row, addl_i).as_deref().and_then(num).is_some_and(|r| r > 0.0) {
            p.addl = true;
        }
        if ss_i.is_some() && cell(row, ss_i).as_deref().and_then(num).is_some_and(|r| r > 0.0) {
            p.steady_state = true;
        }
        if let Some(a) = cell(row, arm_i).filter(|a| !a.is_empty() && a != ".") {
            arms.insert(a);
        }
    }

    p.n_subjects = subjects.len();
    if tmin.is_finite() {
        p.time_range = Some((tmin, tmax));
    }
    if dmin.is_finite() {
        p.dv_range = Some((dmin, dmax));
    }
    p.dvid = dvid_counts.into_iter().collect();
    p.dose_col = ["DOSE", "DOSEMG", "DOSELVL", "DOSELEVEL", "PLANDOSE"]
        .iter()
        .find_map(|n| ds.col_index(n))
        .map(|i| ds.headers[i].clone());
    p.ntim_col = ["NTIM", "NTIME", "NOMTIME", "NOMTM"]
        .iter()
        .find_map(|n| ds.col_index(n))
        .map(|i| ds.headers[i].clone());
    p.datetime_cols = ds
        .headers
        .iter()
        .filter(|h| {
            let u = upper(h);
            u == "DATE" || u == "DATETIME" || u == "CLOCK" || u == "TIMEOFDAY" || u.ends_with("DTC")
        })
        .cloned()
        .collect();
    p.evid_values = evids.into_iter().collect();
    p.arm_values = arms.into_iter().collect();

    doses.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    doses.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    p.dose_levels = doses;

    if !subjects.is_empty() && id_i.is_some() {
        let mut counts: Vec<usize> = subjects.values().copied().collect();
        counts.sort_unstable();
        let median = if counts.len() % 2 == 1 {
            counts[counts.len() / 2] as f64
        } else {
            (counts[counts.len() / 2 - 1] + counts[counts.len() / 2]) as f64 / 2.0
        };
        p.obs_per_subject = Some((counts[0], median, counts[counts.len() - 1]));
    }

    // Column classification.
    for (i, h) in ds.headers.iter().enumerate() {
        let u = upper(h);
        let mut all_numeric = true;
        let mut any_value = false;
        for row in &ds.rows {
            let v = row.get(i).map(|s| s.trim()).unwrap_or("");
            if v.is_empty() || v == "." {
                continue;
            }
            any_value = true;
            if v.parse::<f64>().is_err() {
                all_numeric = false;
                break;
            }
        }
        if any_value && !all_numeric {
            p.non_numeric.push(h.clone());
        }
        if ["BLQ", "CENS", "LIMIT", "LLOQ"].contains(&u.as_str()) {
            p.blq_cols.push(h.clone());
        }
        if COV_CONT.contains(&u.as_str()) {
            p.cov_cont.push(h.clone());
        } else if COV_CAT.contains(&u.as_str()) {
            p.cov_cat.push(h.clone());
        } else if !CORE.contains(&u.as_str())
            && !ARM_NAMES.contains(&u.as_str())
            && any_value
            && all_numeric
            && !u.contains("DATE")
            && !u.ends_with("DT")
        {
            p.cov_other.push(h.clone());
        }
    }
    p
}

fn fmt_num(v: f64) -> String {
    let s = if v.abs() >= 1000.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.4}")
    };
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

fn join_and(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} and {}", items[0], items[1]),
        _ => format!("{}, and {}", items[..items.len() - 1].join(", "), items[items.len() - 1]),
    }
}

fn limited(items: &[String], max: usize) -> Vec<String> {
    if items.len() <= max {
        items.to_vec()
    } else {
        let mut v: Vec<String> = items[..max].to_vec();
        v.push(format!("... ({} more)", items.len() - max));
        v
    }
}

impl Profile {
    fn dv_name(&self) -> String {
        self.dv_col.clone().unwrap_or_else(|| "DV".into())
    }
    fn id_name(&self) -> String {
        self.id_col.clone().unwrap_or_else(|| "ID".into())
    }
    fn subjects_phrase(&self) -> String {
        if self.id_col.is_some() {
            format!("{} subjects", self.n_subjects)
        } else {
            "an unknown number of subjects (no ID column was detected)".into()
        }
    }
    fn obs_phrase(&self) -> String {
        match (&self.dv_col, self.dv_range) {
            (Some(dv), Some((lo, hi))) => format!(
                "{} analysable observations in {dv} (observed range {} to {}, units not recorded in the file)",
                self.n_obs,
                fmt_num(lo),
                fmt_num(hi)
            ),
            (Some(dv), None) => format!("no numeric observations in {dv}"),
            _ => "no DV column".into(),
        }
    }
    fn dose_phrase(&self) -> String {
        let mut s = format!("{} dosing records", self.n_dose);
        if !self.dose_levels.is_empty() {
            let lv: Vec<String> = self.dose_levels.iter().map(|d| fmt_num(*d)).collect();
            s.push_str(&format!(
                " across {} distinct AMT value{} ({})",
                lv.len(),
                if lv.len() == 1 { "" } else { "s" },
                join_and(&limited(&lv, 8))
            ));
        }
        let mut feats = Vec::new();
        if self.infusion {
            feats.push("zero-order infusions (RATE > 0)".to_string());
        }
        if self.addl {
            feats.push("repeated-dose records (ADDL/II)".to_string());
        }
        if self.steady_state {
            feats.push("steady-state flags (SS)".to_string());
        }
        if !feats.is_empty() {
            s.push_str(&format!(", including {}", join_and(&feats)));
        }
        s
    }
    fn cmt_phrase(&self) -> Option<String> {
        if self.cmt_counts.is_empty() {
            return None;
        }
        let parts: Vec<String> = self
            .cmt_counts
            .iter()
            .map(|(c, (o, d))| format!("CMT {c} ({d} dose / {o} observation records)"))
            .collect();
        Some(join_and(&parts))
    }
    /// "DVID 1 (96 records) and DVID 2 (108 records)" when the file carries several endpoints.
    fn dvid_phrase(&self) -> Option<String> {
        if self.dvid.len() < 2 {
            return None;
        }
        let parts: Vec<String> = self
            .dvid
            .iter()
            .map(|(v, n)| format!("DVID {v} ({n} observation records)"))
            .collect();
        Some(join_and(&parts))
    }
    fn scaled_present(&self) -> Vec<String> {
        self.scaled
            .iter()
            .filter(|c| self.headers.iter().any(|h| h.eq_ignore_ascii_case(c)))
            .cloned()
            .collect()
    }
    fn is_scaled(&self, col: &str) -> bool {
        self.scaled.iter().any(|c| c.eq_ignore_ascii_case(col))
    }
    fn arm_phrase(&self) -> Option<String> {
        let col = self.arm_col.as_ref()?;
        if self.arm_values.is_empty() {
            return None;
        }
        Some(format!(
            "{} treatment group{} in {col} ({}; labels are masked and arbitrary)",
            self.arm_values.len(),
            if self.arm_values.len() == 1 { "" } else { "s" },
            join_and(&limited(&self.arm_values, 6))
        ))
    }
    fn covariates(&self) -> Vec<String> {
        let mut v = self.cov_cont.clone();
        v.extend(self.cov_cat.clone());
        v.extend(self.cov_other.clone());
        // The planned dose and the nominal time are design variables, not covariates.
        v.retain(|c| {
            ![&self.dose_col, &self.ntim_col]
                .iter()
                .any(|d| d.as_ref().is_some_and(|d| d.eq_ignore_ascii_case(c)))
        });
        v
    }
    fn dose_cmts(&self) -> Vec<&String> {
        self.cmt_counts.iter().filter(|(_, (_, d))| *d > 0).map(|(c, _)| c).collect()
    }
    fn obs_cmts(&self) -> Vec<&String> {
        self.cmt_counts.iter().filter(|(_, (o, _))| *o > 0).map(|(c, _)| c).collect()
    }
    fn structure_hint(&self) -> String {
        let d = self.dose_cmts();
        let o = self.obs_cmts();
        if d.is_empty() || o.is_empty() {
            return "the compartment layout is not evident from the CMT column, so propose one and say why".into();
        }
        if o.len() > 1 {
            return format!(
                "observations sit in several compartments ({}), which may indicate more than one analyte or a PK/PD or parent/metabolite structure - confirm before modelling",
                join_and(&o.iter().map(|c| format!("CMT {c}")).collect::<Vec<_>>())
            );
        }
        if d.len() == 1 && d[0] != o[0] {
            return format!(
                "doses go into CMT {} and observations are in CMT {}, which is typical of a depot (first-order absorption) to central-compartment structure, but confirm the route",
                d[0], o[0]
            );
        }
        if d.len() == 1 {
            return format!(
                "doses and observations share CMT {}, which is typical of an intravenous bolus or infusion into the central compartment, but confirm the route",
                d[0]
            );
        }
        "dosing occurs in more than one compartment, so state what you infer about the route(s)".into()
    }
}

pub fn render(ds: &Dataset, ctx: &LlmContext) -> String {
    let mut p = profile(ds);
    p.scaled = ctx.scaled.clone();
    p.renamed = ctx.renamed.clone();
    let file = ctx.file_name;
    let mut o = String::new();

    o.push_str("DATA OBFUSCATOR - LLM PROMPT PACK\n");
    o.push_str("=================================\n");
    o.push_str(&format!(
        "Generated by {} {} (released {}) on {}\n",
        crate::APP_NAME,
        crate::APP_VERSION,
        crate::APP_RELEASED,
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
    ));
    o.push_str(&format!(
        "Masked dataset: {file}   |   masking level {} ({})\n\n",
        ctx.level, ctx.level_name
    ));

    o.push_str("HOW TO USE\n");
    o.push_str(&format!(
        "  1. Attach '{file}' to your LLM chat (or paste its contents) and then paste ONE of\n\
         \x20    the three prompts below.\n\
         \x20 2. Share only the masked dataset and this prompt text. Never share keymap.txt or the\n\
         \x20    original input file.\n\
         \x20 3. The prompts say units are not recorded in the file. Add them yourself - for\n\
         \x20    example \"time in hours, DV in ng/mL, AMT in mg\" - plus the drug class or\n\
         \x20    indication if you are comfortable disclosing it.\n\
         \x20 4. The facts in the prompts were computed from the masked file; check them if\n\
         \x20    something looks off.\n\n"
    ));

    // ---- facts
    o.push_str("DATASET FACTS (computed from the masked file)\n");
    o.push_str(&format!("  Records / columns   : {} / {}\n", p.n_rows, p.headers.len()));
    o.push_str(&format!("  Columns             : {}\n", p.headers.join(", ")));
    o.push_str(&format!(
        "  Subjects            : {}{}\n",
        if p.id_col.is_some() { p.n_subjects.to_string() } else { "unknown".into() },
        p.id_col.as_ref().map(|c| format!(" (column {c})")).unwrap_or_default()
    ));
    o.push_str(&format!(
        "  Observations        : {} analysable of {} observation records{}\n",
        p.n_obs,
        p.n_obs_records,
        p.dv_col.as_ref().map(|c| format!(" (DV column {c})")).unwrap_or_default()
    ));
    o.push_str(&format!("  Dosing              : {}\n", p.dose_phrase()));
    if let Some((lo, hi)) = p.time_range {
        o.push_str(&format!(
            "  TIME range          : {} to {} (units not recorded)\n",
            fmt_num(lo),
            fmt_num(hi)
        ));
    }
    if let Some((mn, med, mx)) = p.obs_per_subject {
        o.push_str(&format!(
            "  Obs per subject     : min {mn}, median {}, max {mx}\n",
            fmt_num(med)
        ));
    }
    if let Some(c) = p.cmt_phrase() {
        o.push_str(&format!("  Compartments        : {c}\n"));
    }
    if !p.scaled_present().is_empty() {
        o.push_str(&format!(
            "  Rescaled columns    : {} (each multiplied or divided by a hidden constant; values are not physiological)\n",
            p.scaled_present().join(", ")
        ));
    }
    if !p.renamed.is_empty() {
        o.push_str(&format!(
            "  Renamed columns     : {} (original names withheld; their meaning is unknown)\n",
            p.renamed.join(", ")
        ));
    }
    if let Some(d) = p.dvid_phrase() {
        o.push_str(&format!("  Endpoints (DVID)    : {d}\n"));
    }
    if !p.evid_values.is_empty() {
        o.push_str(&format!("  EVID values         : {}\n", p.evid_values.join(", ")));
    }
    if let Some(a) = p.arm_phrase() {
        o.push_str(&format!("  Treatment groups    : {a}\n"));
    }
    let covs = p.covariates();
    if !covs.is_empty() {
        o.push_str(&format!("  Covariates          : {}\n", covs.join(", ")));
    }
    if !p.non_numeric.is_empty() {
        o.push_str(&format!(
            "  Non-numeric columns : {} (need DROP in a NONMEM $INPUT)\n",
            p.non_numeric.join(", ")
        ));
    }
    if p.n_obs_no_dv > 0 {
        o.push_str(&format!(
            "  Missing DV          : {} observation record(s) have no DV value ({} of them with MDV not equal to 1)\n",
            p.n_obs_no_dv, p.n_obs_no_dv_mdv0
        ));
    }
    if p.n_dv_nonpositive > 0 {
        o.push_str(&format!(
            "  DV <= 0             : {} observation(s) - check for BLQ coding\n",
            p.n_dv_nonpositive
        ));
    }
    if !p.token_free_note().is_empty() {
        o.push_str(&p.token_free_note());
    }
    if !ctx.token_estimates.is_empty() {
        let parts: Vec<String> = ctx
            .token_estimates
            .iter()
            .map(|(l, n)| format!("{l}: ~{n}"))
            .collect();
        o.push_str("\n  Approximate size of the masked dataset in tokens (estimates; see tokens.txt):\n");
        for part in parts {
            o.push_str(&format!("    - {part}\n"));
        }
    }
    o.push('\n');

    // ---- prompts
    o.push_str("1. ANALYZE THE DATA\n");
    o.push_str("-------------------\n");
    o.push_str(&analyze_prompt(&p, file));
    o.push_str("\n\n");

    o.push_str("2. CREATE A NONMEM CONTROL STREAM\n");
    o.push_str("---------------------------------\n");
    o.push_str(&nonmem_prompt(&p, file, ds.delimiter));
    o.push_str("\n\n");

    o.push_str("3. PERFORM QUALITY CONTROL CHECKS\n");
    o.push_str("---------------------------------\n");
    o.push_str(&qc_prompt(&p, file));
    o.push('\n');
    o
}

impl Profile {
    fn token_free_note(&self) -> String {
        let mut s = String::new();
        if !self.blq_cols.is_empty() {
            s.push_str(&format!(
                "  Censoring columns   : {} (BLQ/LLOQ handling present)\n",
                self.blq_cols.join(", ")
            ));
        }
        s
    }
}

fn analyze_prompt(p: &Profile, file: &str) -> String {
    let dv = p.dv_name();
    let mut s = format!(
        "Act as a senior pharmacometrician and clinical pharmacologist. The attached file '{file}' is a masked NONMEM-style dataset with {} records and {} columns ({}) from {}, containing {} and {}",
        p.n_rows,
        p.headers.len(),
        p.headers.join(", "),
        p.subjects_phrase(),
        p.obs_phrase(),
        p.dose_phrase()
    );
    if let Some(c) = p.cmt_phrase() {
        s.push_str(&format!("; compartments are used as follows: {c}"));
    }
    if let Some(d) = p.dvid_phrase() {
        s.push_str(&format!(
            "; DVID separates several endpoints ({d}), so summarise, plot and analyse each endpoint on its own and never pool them"
        ));
    }
    if let Some(a) = p.arm_phrase() {
        s.push_str(&format!("; there are {a}"));
    }
    s.push_str(
        ". Subject, study and site identifiers, categorical labels and dates have been recoded or shifted, so treat every label as arbitrary and do not try to re-identify anything. ",
    );
    if !p.scaled_present().is_empty() {
        s.push_str(&format!(
            "The column{} {} {} each multiplied or divided by a hidden constant factor, so absolute values and units are not physiological: use {} only for ratios, rank order and within-subject constancy, never judge {} against real-world ranges, and do not report them as if they were true weights, ages or lab values. ",
            if p.scaled_present().len() == 1 { "" } else { "s" },
            join_and(&p.scaled_present()),
            if p.scaled_present().len() == 1 { "was" } else { "were" },
            if p.scaled_present().len() == 1 { "it" } else { "them" },
            if p.scaled_present().len() == 1 { "it" } else { "them" },
        ));
    }
    if !p.renamed.is_empty() {
        s.push_str(&format!(
            "The column{} {} {} renamed to generic labels and the original name{} withheld, so do not guess what {} mean: ask me before using {} in any analysis. ",
            if p.renamed.len() == 1 { "" } else { "s" },
            join_and(&p.renamed),
            if p.renamed.len() == 1 { "was" } else { "were" },
            if p.renamed.len() == 1 { " was" } else { "s were" },
            if p.renamed.len() == 1 { "it" } else { "they" },
            if p.renamed.len() == 1 { "it" } else { "them" },
        ));
    }
    s.push_str(
        "I will state the units for time, dose and the dependent variable; if I have not, tell me what you assume. First, confirm how you read the structure (what each column means, the record types implied by EVID, MDV and CMT, and the time, dose and concentration units you infer) and list anything ambiguous for me to confirm instead of guessing. ",
    );
    let mut design = String::from(
        "Next, describe the study design and data completeness (dosing and sampling schedule, observations per subject, missing or censored values",
    );
    if p.arm_col.is_some() {
        design.push_str(", subjects per treatment group");
    }
    if !p.dose_levels.is_empty() && p.dose_levels.len() > 1 {
        design.push_str(", subjects per dose level");
    }
    design.push(')');
    let covs = p.covariates();
    if !covs.is_empty() {
        design.push_str(&format!(
            " and summarise the distributions of {}",
            join_and(&covs)
        ));
    }
    s.push_str(&design);
    s.push_str(&format!(
        ". Then explore the {dv}-time profiles on linear and semi-logarithmic scales (individual profiles, and geometric mean or median by nominal time"
    ));
    if p.dose_levels.len() > 1 || p.arm_col.is_some() {
        s.push_str(", dose-normalised and split by dose or group");
    }
    s.push_str("), and carry out a non-compartmental analysis (Cmax, Tmax, AUC, terminal half-life, apparent clearance and volume where the data allow");
    if p.dose_levels.len() > 1 {
        s.push_str(", plus a dose-proportionality assessment across the dose levels");
    }
    if p.addl || p.steady_state {
        s.push_str(", and accumulation or steady-state exposure for the multiple-dose records");
    }
    s.push(')');
    if !covs.is_empty() {
        s.push_str(&format!(
            "; describe between-subject variability and any visible relationship between exposure and {}",
            join_and(&covs)
        ));
    }
    s.push_str(
        ". Finish with a recommended structural, statistical and covariate strategy for population modelling, with your reasoning, the main risks in this dataset and the next analyses you would run. Quote exact numbers from the file, give reproducible R or Python code for every calculation, and clearly separate what the data show from what you are assuming.",
    );
    s
}

fn nonmem_prompt(p: &Profile, file: &str, delim: Delimiter) -> String {
    let dv = p.dv_name();
    let id = p.id_name();
    let input: Vec<String> = p
        .headers
        .iter()
        .map(|h| {
            if p.non_numeric.iter().any(|n| n.eq_ignore_ascii_case(h)) {
                format!("{h}=DROP")
            } else {
                h.clone()
            }
        })
        .collect();
    let mut s = format!(
        "Act as an expert NONMEM modeller. Using the attached masked dataset '{file}' ({}, {}, and {}) write a complete, runnable NONMEM 7.5 control stream for a population pharmacokinetic model, followed by a short explanation of each block. ",
        p.subjects_phrase(),
        p.obs_phrase(),
        p.dose_phrase()
    );
    s.push_str(&format!(
        "$INPUT must follow the file's exact column order: {}",
        input.join(" ")
    ));
    if p.non_numeric.is_empty() {
        s.push_str(" (all columns are numeric, so none need DROP)");
    } else {
        s.push_str(&format!(
            " (the non-numeric columns {} are marked =DROP because NONMEM cannot read text)",
            join_and(&p.non_numeric)
        ));
    }
    s.push_str(&format!(
        ", and $DATA must read '{file}' with IGNORE=@ to skip the header row"
    ));
    match delim {
        Delimiter::Semicolon => s.push_str(
            " (the file is semicolon-delimited, which NONMEM cannot read, so tell me to convert it to commas or tabs first)",
        ),
        Delimiter::Whitespace => s.push_str(" (the file is whitespace-delimited)"),
        Delimiter::Tab => s.push_str(" (the file is tab-delimited)"),
        Delimiter::Comma => s.push_str(" (the file is comma-delimited)"),
    }
    s.push_str(&format!(
        ". Handle the record structure actually present: {}",
        p.structure_hint()
    ));
    let mut feats = Vec::new();
    if p.infusion {
        feats.push("use the RATE column for the zero-order infusions".to_string());
    }
    if p.addl {
        feats.push("use ADDL and II for the repeated doses".to_string());
    }
    if p.steady_state {
        feats.push("honour the SS flags for steady-state dosing".to_string());
    }
    if p.has.contains("MDV") {
        feats.push("respect MDV for records that must not enter the likelihood".to_string());
    }
    if let Some(d) = p.dvid_phrase() {
        feats.push(format!(
            "model the endpoints in DVID ({d}) jointly with a separate residual-error model per DVID (an indicator-variable $ERROR block) and say which compartment or effect model each endpoint needs"
        ));
    }
    if !p.blq_cols.is_empty() || p.n_dv_nonpositive > 0 {
        feats.push(format!(
            "handle censored or below-the-limit-of-quantification observations explicitly (for example M3 or M4 using {}), and say which method you chose",
            if p.blq_cols.is_empty() { "the DV <= 0 records".to_string() } else { p.blq_cols.join("/") }
        ));
    }
    if !feats.is_empty() {
        s.push_str(&format!("; in addition {}", join_and(&feats)));
    }
    s.push_str(&format!(
        ". Select ADVAN and TRANS to match, justify the number of compartments and say when you would test a more complex model; parameterise in clearance and volume (plus absorption parameters if the data call for them) with log-normal inter-individual variability on {id}-level random effects, a combined or proportional residual error model on {dv}, and initial estimates that you derive from the observed data (show the arithmetic)"
    ));
    let weight = p
        .cov_cont
        .iter()
        .find(|c| ["WT", "WGT", "WEIGHT", "LBWT"].contains(&upper(c).as_str()));
    if let Some(w) = weight {
        if p.is_scaled(w) {
            s.push_str(&format!(
                "; apply allometric scaling with {w} (exponents 0.75 on clearances and 1 on volumes) centred on the MEDIAN of {w} in this file, because {w} was multiplied or divided by a hidden constant and a 70 kg reference would be wrong"
            ));
        } else {
            s.push_str(&format!(
                "; apply allometric scaling with {w} (exponents 0.75 on clearances and 1 on volumes, centred on a sensible reference weight)"
            ));
        }
    }
    let is_text = |c: &String| p.non_numeric.iter().any(|n| n.eq_ignore_ascii_case(c));
    let other_covs: Vec<String> = p
        .covariates()
        .into_iter()
        .filter(|c| Some(c) != weight && !is_text(c))
        .collect();
    if !other_covs.is_empty() {
        s.push_str(&format!(
            "; include clearly labelled, switchable covariate relationships to test for {}",
            join_and(&other_covs)
        ));
    }
    let text_covs: Vec<String> = p.covariates().into_iter().filter(|c| is_text(c)).collect();
    if !text_covs.is_empty() {
        s.push_str(&format!(
            "; {} {} stored as text and therefore dropped from $INPUT, so say how I should recode {} to numbers if I want to test {}",
            join_and(&text_covs),
            if text_covs.len() == 1 { "is" } else { "are" },
            if text_covs.len() == 1 { "it" } else { "them" },
            if text_covs.len() == 1 { "it" } else { "them" }
        ));
    }
    let other_scaled: Vec<String> = p
        .scaled_present()
        .into_iter()
        .filter(|c| Some(c) != weight)
        .collect();
    if !other_scaled.is_empty() {
        s.push_str(&format!(
            "; {} {} rescaled by hidden constants, so centre every covariate effect on the median of the column and note that the coefficients refer to the rescaled values",
            join_and(&other_scaled),
            if other_scaled.len() == 1 { "was" } else { "were" }
        ));
    }
    s.push_str(&format!(
        ". Use $ESTIMATION METHOD=1 INTERACTION (or SAEM followed by importance sampling if you judge it more robust and tell me why), add $COVARIANCE, and a $TABLE that outputs {id} TIME {dv} PRED IPRED CWRES and the other items needed for goodness-of-fit plots. List every assumption I must confirm before running (compartment numbering, units, LLOQ and handling of censored data, route of administration), do not invent data that are not in the file, and point out anything in the dataset that would make this control stream fail or mislead."
    ));
    s
}

fn qc_prompt(p: &Profile, file: &str) -> String {
    let dv = p.dv_name();
    let id = p.id_name();
    let mut s = format!(
        "Act as a pharmacometric data programmer carrying out an independent quality-control review of the attached masked NONMEM-style dataset '{file}' ({} records, {} columns, {}; {}; {}). Check each of the following and report the count and the exact {id}/TIME of every offending record. (1) Structure: consistent number of fields per row, a single header row, the missing-value convention, and non-numeric entries in columns that should be numeric",
        p.n_rows,
        p.headers.len(),
        p.subjects_phrase(),
        p.obs_phrase(),
        p.dose_phrase()
    );
    if !p.non_numeric.is_empty() {
        s.push_str(&format!(" (currently text in {})", join_and(&p.non_numeric)));
    }
    s.push_str(&format!(
        ". (2) Uniqueness and ordering: duplicate {id}/TIME/EVID/CMT records, TIME decreasing within a subject, and {id} values that are not contiguous"
    ));
    s.push_str(". (3) Event logic: EVID, MDV, AMT and ");
    s.push_str(&dv);
    s.push_str(" must agree (dosing records need a positive AMT and no DV; observation records need EVID=0 with a DV and MDV=0; MDV=1 wherever DV is missing)");
    if p.n_obs_no_dv > 0 {
        s.push_str(&format!(
            "; my own quick scan already found {} observation record(s) with no {dv} value, {} of which have MDV not equal to 1, so please quantify and explain that first",
            p.n_obs_no_dv, p.n_obs_no_dv_mdv0
        ));
    }
    let mut ev = Vec::new();
    if p.has.contains("RATE") {
        ev.push("RATE");
    }
    if p.has.contains("ADDL") || p.has.contains("II") {
        ev.push("ADDL/II");
    }
    if p.has.contains("SS") {
        ev.push("SS");
    }
    if !ev.is_empty() {
        s.push_str(&format!(
            ", and the {} fields must be consistent with the dosing records",
            ev.join(", ")
        ));
    }
    s.push_str(
        ". (4) Subject-level completeness: subjects with no dose, no observations, or observations before the first dose, and each subject's observation count against the median of the group",
    );
    if let Some(c) = p.cmt_phrase() {
        s.push_str(&format!(
            ". (5) Compartments: that CMT is valid and used consistently for dosing versus observation records ({c})"
        ));
    } else {
        s.push_str(". (5) Compartments: there is no CMT column, so state what that implies for model coding");
    }
    if p.has.contains("TAD") {
        s.push_str(". (6) TAD: recompute time-after-dose from TIME and the most recent dose and compare it with the TAD column");
    } else {
        s.push_str(". (6) Time: plausibility of the TIME values and of the sampling schedule against the dosing");
    }
    let range = p
        .dv_range
        .map(|(lo, hi)| format!(" (observed {} to {})", fmt_num(lo), fmt_num(hi)))
        .unwrap_or_default();
    s.push_str(&format!(
        ". (7) Dependent variable: {dv} values that are zero, negative, implausibly large or inconsistent with neighbouring samples{range}, plus how below-limit-of-quantification values are coded"
    ));
    if !p.dose_levels.is_empty() {
        let lv: Vec<String> = p.dose_levels.iter().map(|d| fmt_num(*d)).collect();
        s.push_str(&format!(
            ". (8) Doses: whether the AMT values ({}) are consistent with the treatment groups and with the dosing history",
            join_and(&limited(&lv, 8))
        ));
    }
    let covs = p.covariates();
    if !covs.is_empty() {
        let scaled: Vec<String> = covs.iter().filter(|c| p.is_scaled(c)).cloned().collect();
        let real: Vec<String> = covs.iter().filter(|c| !p.is_scaled(c)).cloned().collect();
        let mut t = String::from(". (9) Covariates: ");
        if !real.is_empty() {
            t.push_str(&format!(
                "{} - check physiological plausibility, unexpected missing values, whether each is constant within a subject unless it is meant to vary with time, and the coding of categorical values",
                join_and(&real)
            ));
        }
        if !scaled.is_empty() {
            if !real.is_empty() {
                t.push_str("; ");
            }
            t.push_str(&format!(
                "{} {} multiplied or divided by a hidden constant during masking, so do NOT judge {} against physiological ranges - check only missing values, constancy within a subject and internal consistency",
                join_and(&scaled),
                if scaled.len() == 1 { "was" } else { "were" },
                if scaled.len() == 1 { "it" } else { "them" }
            ));
        }
        s.push_str(&t);
    }
    let mut n = 10;
    if let Some(d) = &p.dose_col {
        s.push_str(&format!(
            ". ({n}) Planned versus administered dose: on every dosing record AMT must equal {d} (flag any difference, including ratios that look like unit slips such as 1000x or 0.001x, and treat a documented partial dose as a query rather than an error), and {d} must stay constant within a subject unless a dose change is recorded"
        ));
        n += 1;
    }
    if !p.datetime_cols.is_empty() {
        s.push_str(&format!(
            ". ({n}) Time translation: rebuild the elapsed time since each subject's first dose from {} and compare it with TIME (allow rounding); report wrong units such as days instead of hours, dates out of order, mixed date formats, a missing day rollover after midnight, and any record whose date and TIME disagree",
            join_and(&p.datetime_cols)
        ));
        n += 1;
    }
    if let Some(c) = &p.ntim_col {
        s.push_str(&format!(
            ". ({n}) Nominal versus actual time: compare TIME with {c} and flag deviations beyond a tolerance you propose and justify"
        ));
        n += 1;
    }
    if let Some(d) = p.dvid_phrase() {
        s.push_str(&format!(
            ". ({n}) Endpoints: each DVID ({d}) must map to one CMT and one DV meaning, and no record may mix endpoints"
        ));
    }
    s.push_str(". Return a prioritised findings table (severity, check, number of records, example rows, suggested fix), then a PASS or FAIL checklist for every numbered check, then reproducible R or Python code that performs the checks. Do not change, impute or delete data silently, and ask me about anything ambiguous instead of assuming.");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::parse_lines;

    fn ds(text: &str) -> Dataset {
        let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        parse_lines(&lines, None, false, &[], "#;$", "t.csv".into()).unwrap()
    }

    const SAMPLE: &str = "ID,TIME,AMT,DV,EVID,MDV,CMT,WT,SEX,ARM\n\
        1,0,100,.,1,1,1,70,0,ARM_A\n\
        1,1,.,2.5,0,0,2,70,0,ARM_A\n\
        1,4,.,1.2,0,0,2,70,0,ARM_A\n\
        2,0,200,.,1,1,1,80,1,ARM_B\n\
        2,1,.,4.0,0,0,2,80,1,ARM_B\n";

    #[test]
    fn profiles_basic_structure() {
        let p = profile(&ds(SAMPLE));
        assert_eq!(p.n_subjects, 2);
        assert_eq!(p.n_dose, 2);
        assert_eq!(p.n_obs, 3);
        assert_eq!(p.dose_levels, vec![100.0, 200.0]);
        assert_eq!(p.cmt_counts.get("1"), Some(&(0, 2)));
        assert_eq!(p.cmt_counts.get("2"), Some(&(3, 0)));
        assert!(p.structure_hint().contains("depot"));
        assert_eq!(p.non_numeric, vec!["ARM".to_string()]);
    }

    #[test]
    fn prompts_are_single_paragraphs_with_dataset_facts() {
        let d = ds(SAMPLE);
        let txt = render(
            &d,
            &LlmContext {
                file_name: "obfuscated.csv",
                level: 2,
                level_name: "clinical labels",
                scaled: Vec::new(),
                renamed: Vec::new(),
                token_estimates: vec![("GPT-4".into(), 123)],
            },
        );
        for title in ["1. ANALYZE THE DATA", "2. CREATE A NONMEM CONTROL STREAM", "3. PERFORM QUALITY CONTROL CHECKS"] {
            let i = txt.find(title).expect(title);
            let body = txt[i..].lines().nth(2).unwrap();
            assert!(body.len() > 400, "{title} paragraph too short");
        }
        assert!(txt.contains("ARM=DROP"));
        assert!(txt.contains("2 subjects"));
        assert!(txt.contains("obfuscated.csv"));
    }
}
