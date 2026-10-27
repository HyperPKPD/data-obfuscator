//! Recode identifiers, shift dates, scale covariates — never touch PK core.

use crate::config::{IdStyle, RunOptions};
use crate::dataset::{write_dataset, Dataset};
use crate::detect::{output_headers, plan_columns, ColumnPlan, Role};
use crate::summary::{self, Summary};
use crate::llm::{self, LlmContext};
use crate::tokens::{self, TokenReport, MODELS, TOKENIZER_NAME};
use crate::trace::TraceLog;
use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct RunArtifacts {
    pub output_dataset: PathBuf,
    pub summary_path: PathBuf,
    pub tokens_path: PathBuf,
    pub log_path: PathBuf,
    pub config_used_path: PathBuf,
    pub column_report_path: PathBuf,
    pub llm_path: PathBuf,
    pub warnings: usize,
    pub keymap_path: Option<PathBuf>,
    /// Level 4 only: keymap_columns.txt, the per-value factors needed to restore scaled columns exactly.
    pub keymap_columns_path: Option<PathBuf>,
    pub dataset: Dataset,
    pub summary: Summary,
    pub token_report: TokenReport,
}

/// Level 4: every numeric cell of a scaled column gets its own factor, the column's base factor
/// plus or minus this fraction (uniform), so a value no longer shares one exact factor with its column.
pub const LEVEL4_SPREAD: f64 = 0.15;

pub type Masked = (Dataset, Vec<KeyRow>, Vec<ColumnPlan>, f64, Vec<ScaleRow>);

/// Level 4: the factor every scaled cell of every row received, recorded so that `reverse` can undo
/// the per-value variation exactly. Written to keymap_columns.txt.
#[derive(Clone, Debug)]
pub struct PerValue {
    /// Indices (in the INPUT dataset) of the columns that identify a row: IDs, subject IDs, study columns, TIME, DVID, EVID, CMT.
    pub key_cols: Vec<usize>,
    /// Indices (in the input dataset) of the scaled columns that are in the masked dataset, with their `scales` position.
    pub value_cols: Vec<(usize, usize)>,
    /// One ready-to-write TAB-separated line per data row of the source:
    /// `source row, key values..., then masked value and factor for every value column`
    /// (the factor is `.` when the cell was blank or text and therefore not scaled).
    pub lines: Vec<String>,
}

/// Everything `obfuscate_detailed` produces.
pub struct Detailed {
    pub out: Dataset,
    pub key_rows: Vec<KeyRow>,
    pub plan: Vec<ColumnPlan>,
    pub date_shift: f64,
    pub scales: Vec<ScaleRow>,
    pub per_value: Option<PerValue>,
}

pub fn obfuscate(ds: &Dataset, opts: &RunOptions, log: &mut TraceLog) -> Result<Masked> {
    let d = obfuscate_detailed(ds, opts, log)?;
    Ok((d.out, d.key_rows, d.plan, d.date_shift, d.scales))
}

pub fn obfuscate_detailed(ds: &Dataset, opts: &RunOptions, log: &mut TraceLog) -> Result<Detailed> {
    log.info(format!(
        "planning columns n={} level={} seed={}",
        ds.n_cols(),
        opts.level,
        opts.seed
    ));
    let plan = plan_columns(ds, opts);
    for p in &plan {
        log.debug(format!(
            "column {} role={} recode={} scale={} redact={} drop={} protect={}",
            p.name,
            p.role.name(),
            p.recode,
            p.scale,
            p.redact,
            p.drop,
            p.protect
        ));
    }

    let mut rng = ChaCha20Rng::seed_from_u64(opts.seed);
    // One dataset-wide shift, a decimal number of days (default 1-100, e.g. 23.6). A random draw is
    // rounded to 1 decimal (0.1 day = 2 h 24 min, so every drawn value is a whole number of minutes);
    // a fixed value given by the user is used exactly as written.
    let date_shift = if opts.level >= 2 {
        let lo = opts.date_shift_min.min(opts.date_shift_max);
        let hi = opts.date_shift_min.max(opts.date_shift_max);
        if lo == hi {
            lo
        } else {
            ((rng.gen_range(lo..=hi) * 10.0).round() / 10.0).clamp(lo, hi)
        }
    } else {
        0.0
    };
    if date_shift != 0.0 {
        log.info(format!(
            "date shift (dataset-wide, intervals preserved): {} days",
            fmt_days(date_shift)
        ));
    }

    // Numeric scaling (level 3+): every valid number of a scaled column is multiplied OR divided
    // by a random factor drawn for that column. Level 3 uses that one factor for the whole column
    // (keeps within-subject constancy, ratios and rank order). Level 4 uses it as a base: every
    // number gets its own factor, base +/- 15 % (uniform), so even ratios and constancy are blurred.
    // The base factor is recorded in keymap.txt.
    let spread = if opts.level >= 4 { LEVEL4_SPREAD } else { 0.0 };
    let mut scales: Vec<ScaleRow> = Vec::new();
    for p in plan.iter().filter(|p| p.scale && !p.drop && !p.protect) {
        let factor = if opts.scale_max > opts.scale_min {
            rng.gen_range(opts.scale_min..=opts.scale_max)
        } else {
            opts.scale_min
        };
        let divide = rng.gen_bool(0.5);
        scales.push(ScaleRow {
            column: p.name.clone(),
            index: p.index,
            divide,
            factor,
            spread,
            values_changed: 0,
        });
    }
    for sc in &scales {
        if sc.spread > 0.0 {
            log.info(format!(
                "scaling column {}: every valid number will be {} its own random factor, base {:.10} +/- {:.0} %",
                sc.column,
                if sc.divide { "divided by" } else { "multiplied by" },
                sc.factor,
                sc.spread * 100.0
            ));
        } else {
            log.info(format!(
                "scaling column {}: every valid number will be {} {:.10}",
                sc.column,
                if sc.divide { "divided by" } else { "multiplied by" },
                sc.factor
            ));
        }
    }

    // Own random stream for the per-value factors, so they never disturb the other draws above.
    let mut value_rng = ChaCha20Rng::seed_from_u64(opts.seed ^ 0x5CA1_E5CA_1E5C_A1E5);

    let mut maps: HashMap<usize, HashMap<String, String>> = HashMap::new();
    let mut counters: HashMap<Role, usize> = HashMap::new();
    let mut key_rows: Vec<KeyRow> = Vec::new();

    // Several columns can share a role (STU / STUDYID / STUDY are all "study"). Columns are
    // processed in file order; a later column whose values pair one-to-one with an earlier
    // column of the same role (row by row) reuses that column's masked labels, so the same
    // study, site or subject never ends up with two different labels.
    let mut done_by_role: HashMap<Role, Vec<usize>> = HashMap::new();
    for p in plan.iter().filter(|p| p.recode && p.role != Role::Date && !p.drop) {
        let distinct = distinct_values(ds, p.index);
        let earlier: Vec<usize> = done_by_role.get(&p.role).cloned().unwrap_or_default();
        let mut aligned_with: HashMap<String, (usize, String)> = HashMap::new();
        for &q in &earlier {
            for (v, w) in one_to_one_pairs(ds, p.index, q) {
                if let Some(masked) = maps.get(&q).and_then(|m| m.get(&w)) {
                    aligned_with.entry(v).or_insert((q, masked.clone()));
                }
            }
        }
        let mut map = HashMap::new();
        let mut n_aligned = 0usize;
        for raw in &distinct {
            if let Some((q, masked)) = aligned_with.get(raw) {
                log.debug(format!(
                    "column {}: value '{}' shares its masked label with column {}",
                    p.name,
                    raw,
                    plan[*q].name
                ));
                map.insert(raw.clone(), masked.clone());
                n_aligned += 1;
                continue;
            }
            let idx = counters.entry(p.role).or_insert(0);
            *idx += 1;
            let n = *idx;
            map.insert(raw.clone(), recode_value(p.role, raw, n, opts.id_style, opts.seed));
        }
        if n_aligned > 0 {
            log.success(format!(
                "recoded {} unique values in column {} ({}); {} of them reuse the labels of the other {} column(s)",
                map.len(),
                p.name,
                p.role.name(),
                n_aligned,
                p.role.name()
            ));
        } else {
            log.success(format!(
                "recoded {} unique values in column {} ({})",
                map.len(),
                p.name,
                p.role.name()
            ));
        }
        if n_aligned > 0 && n_aligned < map.len() {
            log.info(format!(
                "column {} ({}): {} value(s) do not pair one-to-one with the earlier {} column(s) and got their own labels",
                p.name,
                p.role.name(),
                map.len() - n_aligned,
                p.role.name()
            ));
        }
        if opts.write_keymap {
            for (orig, masked) in &map {
                key_rows.push(KeyRow {
                    column: p.name.clone(),
                    original: orig.clone(),
                    masked: masked.clone(),
                });
            }
        }
        maps.insert(p.index, map);
        done_by_role.entry(p.role).or_default().push(p.index);
    }

    // Level 4 (per-value factors): which columns identify a row and which carry a varying factor.
    // Both lists only contain columns that are in the masked dataset.
    let headers_out_pre = output_headers(&plan, &opts.keep, &ds.headers);
    let in_output = |name: &str| headers_out_pre.iter().any(|h| h.eq_ignore_ascii_case(name));
    let pv_value_cols: Vec<(usize, usize)> = if opts.write_keymap {
        scales
            .iter()
            .enumerate()
            .filter(|(_, sc)| sc.spread > 0.0 && in_output(&sc.column))
            .map(|(si, sc)| (sc.index, si))
            .collect()
    } else {
        Vec::new()
    };
    let pv_key_cols: Vec<usize> = {
        let mut v: Vec<usize> = Vec::new();
        let mut add = |i: usize| {
            if !v.contains(&i) {
                v.push(i);
            }
        };
        // Subject identifiers first, then study columns (STU, STDY, STUDY, STUDYID, ...), then the row keys.
        for role in [Role::Id, Role::Subject, Role::Study] {
            for p in plan.iter().filter(|p| p.role == role && !p.drop && in_output(&p.name)) {
                add(p.index);
            }
        }
        for want in ["TIME", "DVID", "EVID", "CMT"] {
            if let Some(p) = plan.iter().find(|p| p.name.eq_ignore_ascii_case(want) && !p.drop && in_output(&p.name)) {
                add(p.index);
            }
        }
        v
    };
    let mut per_value: Option<PerValue> = if pv_value_cols.is_empty() {
        None
    } else {
        Some(PerValue { key_cols: pv_key_cols, value_cols: pv_value_cols, lines: Vec::with_capacity(ds.rows.len()) })
    };
    let mut cell_factors: Vec<(usize, f64)> = Vec::new();

    // Date cells the shifter could not read (mixed formats such as DD/MM/YYYY): column -> (count, first data row).
    let mut unshifted: Vec<(String, usize, usize)> = Vec::new();
    let mut out_rows: Vec<Vec<String>> = Vec::with_capacity(ds.rows.len());
    for (row_no, row) in ds.rows.iter().enumerate() {
        let mut new_row = row.clone();
        cell_factors.clear();
        for p in &plan {
            if p.drop {
                continue;
            }
            let cell = new_row.get_mut(p.index);
            let Some(cell) = cell else { continue };
            if p.protect {
                continue;
            }
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
            if let Some(map) = maps.get(&p.index) {
                let key = cell.trim();
                if let Some(v) = map.get(key) {
                    *cell = v.clone();
                }
                continue;
            }
            if p.scale {
                if let Some(sc) = scales.iter_mut().find(|sc| sc.index == p.index) {
                    let scaled = if sc.spread > 0.0 {
                        scale_cell_varying(cell, sc.divide, sc.factor, sc.spread, &mut value_rng)
                            .map(|(v, f)| {
                                cell_factors.push((p.index, f));
                                v
                            })
                    } else {
                        scale_cell(cell, sc.divide, sc.factor)
                    };
                    if let Some(v) = scaled {
                        *cell = v;
                        sc.values_changed += 1;
                    }
                }
            }
        }
        if let Some(pv) = per_value.as_mut() {
            let text = |i: usize| new_row.get(i).map(|s| s.trim().replace(['\t', '\n', '\r'], " ")).unwrap_or_default();
            let mut line = (row_no + 1).to_string();
            for &i in &pv.key_cols {
                line.push('\t');
                line.push_str(&text(i));
            }
            for &(i, _) in &pv.value_cols {
                line.push('\t');
                line.push_str(&text(i));
                line.push('\t');
                match cell_factors.iter().find(|(c, _)| *c == i) {
                    Some((_, f)) => line.push_str(&format!("{f}")),
                    None => line.push('.'),
                }
            }
            pv.lines.push(line);
        }
        out_rows.push(new_row);
    }

    for (name, n, first) in &unshifted {
        log.warn(format!(
            "date column {name}: {n} value(s) are not in a recognised date format and were left UNSHIFTED (first at data row {}) - they still show the real date; fix or drop them before sharing",
            first + 1
        ));
    }

    // Level 4: reorder subjects (keep within-ID chronology).
    if opts.level >= 4 {
        if let Some(id_plan) = plan.iter().find(|p| p.role == Role::Id && !p.drop) {
            log.info("level 4: reordering subjects, preserving within-ID TIME order");
            let time_idx = plan.iter().find(|p| p.name.eq_ignore_ascii_case("TIME")).map(|p| p.index);
            let mut order: Vec<usize> = (0..out_rows.len()).collect();
            order.sort_by(|&a, &b| {
                let ia = out_rows[a].get(id_plan.index).map(|s| s.as_str()).unwrap_or("");
                let ib = out_rows[b].get(id_plan.index).map(|s| s.as_str()).unwrap_or("");
                ia.cmp(ib).then_with(|| {
                    let ta = time_idx
                        .and_then(|i| out_rows[a].get(i))
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    let tb = time_idx
                        .and_then(|i| out_rows[b].get(i))
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    ta.partial_cmp(&tb).unwrap_or(std::cmp::Ordering::Equal)
                })
            });
            out_rows = order.into_iter().map(|i| out_rows[i].clone()).collect();
        }
    }

    let headers_out = output_headers(&plan, &opts.keep, &ds.headers);
    if headers_out.is_empty() {
        bail!("no columns remain after keep/drop — refusing to write an empty dataset");
    }
    log.info(format!(
        "projecting {} of {} columns",
        headers_out.len(),
        ds.n_cols()
    ));
    let idx_out: Vec<usize> = headers_out
        .iter()
        .map(|h| {
            ds.col_index(h)
                .with_context(|| format!("missing column {h}"))
        })
        .collect::<Result<Vec<_>>>()?;

    // When every column is kept in its original position, rows with a wrong number of fields
    // keep that shape in the masked file so a QC pass still sees them; otherwise rows are
    // projected onto the kept columns.
    let identity = idx_out.len() == ds.n_cols() && idx_out.iter().enumerate().all(|(a, &b)| a == b);
    let mut projected = Vec::with_capacity(out_rows.len());
    for row in out_rows {
        if identity {
            projected.push(row);
        } else {
            projected.push(idx_out.iter().map(|&i| row.get(i).cloned().unwrap_or_default()).collect());
        }
    }

    let out = Dataset {
        headers: headers_out,
        rows: projected,
        delimiter: ds.delimiter,
        comments: Vec::new(),
        source_name: "obfuscated".into(),
        ragged_rows: if identity { ds.ragged_rows } else { 0 },
    };
    // HashMap iteration order is random per process; sort so keymap.txt is identical every run.
    key_rows.sort_by(|a, b| a.column.cmp(&b.column).then_with(|| a.masked.cmp(&b.masked)));
    Ok(Detailed { out, key_rows, plan, date_shift, scales, per_value })
}

/// Distinct non-empty values of a column in order of first appearance.
pub(crate) fn distinct_values(ds: &Dataset, col: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for row in &ds.rows {
        let raw = row.get(col).map(|s| s.as_str()).unwrap_or("").trim();
        if raw.is_empty() || raw == "." {
            continue;
        }
        if seen.insert(raw.to_string()) {
            out.push(raw.to_string());
        }
    }
    out
}

/// (value in `col`, value in `other`) pairs that map one-to-one over every row where both
/// columns are filled. Values that co-occur with more than one partner are left out.
fn one_to_one_pairs(ds: &Dataset, col: usize, other: usize) -> Vec<(String, String)> {
    let mut fwd: HashMap<String, HashSet<String>> = HashMap::new();
    let mut back: HashMap<String, HashSet<String>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for row in &ds.rows {
        let v = row.get(col).map(|s| s.trim()).unwrap_or("");
        let w = row.get(other).map(|s| s.trim()).unwrap_or("");
        if v.is_empty() || v == "." || w.is_empty() || w == "." {
            continue;
        }
        if !fwd.contains_key(v) {
            order.push(v.to_string());
        }
        fwd.entry(v.to_string()).or_default().insert(w.to_string());
        back.entry(w.to_string()).or_default().insert(v.to_string());
    }
    order
        .into_iter()
        .filter_map(|v| {
            let ws = &fwd[&v];
            if ws.len() != 1 {
                return None;
            }
            let w = ws.iter().next().unwrap().clone();
            (back[&w].len() == 1).then_some((v, w))
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct KeyRow {
    pub column: String,
    pub original: String,
    pub masked: String,
}

pub(crate) fn recode_value(role: Role, original: &str, n: usize, style: IdStyle, seed: u64) -> String {
    if style == IdStyle::Hashed {
        let mut h = Sha256::new();
        h.update(seed.to_le_bytes());
        h.update(role.name().as_bytes());
        h.update(original.as_bytes());
        let hex = hex::encode(h.finalize());
        return match role {
            Role::Id => {
                let num = u64::from_str_radix(&hex[..8], 16).unwrap_or(n as u64);
                format!("{}", 10000 + (num % 89999))
            }
            _ => format!("{}{}", prefix(role), &hex[..8].to_ascii_uppercase()),
        };
    }
    let padded = style == IdStyle::Padded || role != Role::Id;
    match role {
        Role::Id if !padded => format!("{n}"),
        Role::Id => format!("{n:04}"),
        Role::Treatment => {
            let letter = ((n - 1) % 26) as u8 + b'A';
            format!("ARM_{}", letter as char)
        }
        Role::Sex => match n {
            1 => "0".into(),
            2 => "1".into(),
            _ => format!("{n}"),
        },
        _ => format!("{}{:03}", prefix(role), n),
    }
}

fn prefix(role: Role) -> &'static str {
    match role {
        Role::Id => "ID",
        Role::Subject => "SUBJ",
        Role::Study => "STU",
        Role::Site => "SITE",
        Role::Treatment => "ARM_",
        Role::User => "USER",
        Role::Race => "CAT",
        Role::Country => "C",
        Role::Sex => "SEX",
        _ => "X",
    }
}

/// One scaled column, as recorded in keymap.txt.
#[derive(Clone, Debug)]
pub struct ScaleRow {
    pub column: String,
    pub index: usize,
    /// true: values were divided by `factor`; false: multiplied.
    pub divide: bool,
    /// Base factor: the factor itself at level 3; the centre of the per-value factors at level 4.
    pub factor: f64,
    /// 0 = one factor for the whole column (level 3); 0.15 = each value gets base +/- 15 % (level 4).
    pub spread: f64,
    pub values_changed: usize,
}

impl ScaleRow {
    /// Text for keymap.txt: how much the factor varies from value to value.
    pub fn variation(&self) -> String {
        if self.spread > 0.0 {
            format!("+/-{:.0}%", self.spread * 100.0)
        } else {
            "none".into()
        }
    }

    pub fn operation(&self) -> &'static str {
        if self.divide {
            "divide"
        } else {
            "multiply"
        }
    }
}

/// Level 4: like `scale_cell`, but this value gets its own factor, `factor` x (1 +/- `spread`), uniform.
/// Returns the new text and the factor this value received.
fn scale_cell_varying(cell: &str, divide: bool, factor: f64, spread: f64, rng: &mut ChaCha20Rng) -> Option<(String, f64)> {
    let v: f64 = cell.trim().parse().ok().filter(|v: &f64| v.is_finite())?;
    let f = factor * rng.gen_range((1.0 - spread)..=(1.0 + spread));
    let nv = if divide { v / f } else { v * f };
    Some((fmt_sig(nv, 8), f))
}

/// Multiplies or divides one valid number; blanks, "." and text are left alone (None).
pub(crate) fn scale_cell(cell: &str, divide: bool, factor: f64) -> Option<String> {
    let v: f64 = cell.trim().parse().ok().filter(|v: &f64| v.is_finite())?;
    let nv = if divide { v / factor } else { v * factor };
    Some(fmt_sig(nv, 8))
}

/// `x` with `sig` significant digits, no exponent, trailing zeros removed.
pub(crate) fn fmt_sig(x: f64, sig: i32) -> String {
    if x == 0.0 || !x.is_finite() {
        return "0".into();
    }
    let mag = x.abs().log10().floor() as i32;
    let decimals = (sig - 1 - mag).clamp(0, 15) as usize;
    let s = format!("{x:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// Days as text: up to 6 decimals, trailing zeros removed (`23.6`, `100`, `-0.5`).
pub fn fmt_days(x: f64) -> String {
    let s = format!("{:.6}", x);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" {
        "0".to_string()
    } else {
        s.to_string()
    }
}

/// Returns the shifted cell and whether it could be shifted (empty cells and `.` count as fine).
///
/// `days` may be fractional. Date-times (a date followed by a time of day) and numeric day counts
/// move by exactly `days`; date-only text dates move by `floor(days)` whole days, which keeps the
/// intervals between date-only values exact.
pub(crate) fn shift_date_cell(cell: &str, days: f64) -> (String, bool) {
    shift_date_cell_whole(cell, days, days.floor() as i64)
}

/// Undo `shift_date_cell` for a shift of `days` (whole-day part `whole` = floor(days) as written in keymap.txt).
pub(crate) fn unshift_date_cell(cell: &str, days: f64, whole: i64) -> (String, bool) {
    shift_date_cell_whole(cell, -days, -whole)
}

/// `days` moves date-times and day counts; `whole` moves date-only text dates.
fn shift_date_cell_whole(cell: &str, days: f64, whole: i64) -> (String, bool) {
    let t = cell.trim();
    if t.is_empty() || t == "." || days == 0.0 {
        return (cell.to_string(), true);
    }
    // Compact dates (20240325, 202403250830, 20240325T0830) are shifted as real dates and keep
    // their compact form; any other number is treated as a day count.
    if let Some(r) = shift_compact_date(t, days, whole) {
        return (r, true);
    }
    if let Ok(x) = t.parse::<f64>() {
        if x.is_finite() {
            return (fmt_days(x + days), true);
        }
    }
    if let Some((d, rest)) = parse_date_prefix(t) {
        if let Some(r) = shift_date_and_time(d, &rest, days, false, whole) {
            return (r, true);
        }
    }
    (cell.to_string(), false)
}

/// `YYYYMMDD`, optionally followed by `HHMM`/`HHMMSS` digits or a non-digit separator and a time.
fn shift_compact_date(t: &str, days: f64, whole: i64) -> Option<String> {
    if t.len() < 8 || !t.is_char_boundary(8) || !t[..8].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let rest = &t[8..];
    let rest_ok = rest.is_empty() || !rest.starts_with(|c: char| c.is_ascii_digit());
    let digits_ok = rest.chars().all(|c| c.is_ascii_digit()) && matches!(rest.len(), 4 | 6);
    if !(rest_ok || digits_ok) {
        return None;
    }
    let y: i32 = t[0..4].parse().ok()?;
    let m: u32 = t[4..6].parse().ok()?;
    let d: u32 = t[6..8].parse().ok()?;
    if !(1800..=2200).contains(&y) {
        return None;
    }
    let date = NaiveDate::from_ymd_opt(y, m, d)?;
    shift_date_and_time(date, rest, days, true, whole)
}

/// A time of day found after a date: separator, `HH:MM[:SS]` or `HHMM[SS]`, then an opaque suffix.
struct TimeOfDay {
    sep: String,
    h: u32,
    mi: u32,
    s: Option<u32>,
    colon: bool,
    suffix: String,
}

fn parse_time_of_day(rest: &str) -> Option<TimeOfDay> {
    let mut r = rest;
    let mut sep = String::new();
    if let Some(c) = r.chars().next() {
        if !c.is_ascii_digit() {
            sep.push(c);
            r = &r[c.len_utf8()..];
        }
    }
    let b = r.as_bytes();
    let dig = |i: usize| b.get(i).map_or(false, |c| c.is_ascii_digit());
    let num = |i: usize| -> u32 { r[i..i + 2].parse().unwrap_or(99) };
    let (h, mi, s, colon, used);
    if dig(0) && dig(1) && b.get(2) == Some(&b':') && dig(3) && dig(4) {
        h = num(0);
        mi = num(3);
        colon = true;
        if b.get(5) == Some(&b':') && dig(6) && dig(7) {
            s = Some(num(6));
            used = 8;
        } else {
            s = None;
            used = 5;
        }
    } else if dig(0) && dig(1) && dig(2) && dig(3) {
        h = num(0);
        mi = num(2);
        colon = false;
        if dig(4) && dig(5) {
            s = Some(num(4));
            used = 6;
        } else {
            s = None;
            used = 4;
        }
    } else {
        return None;
    }
    // the time must not be followed by more digits (that would be something else)
    if dig(used) || h > 23 || mi > 59 || s.map_or(false, |x| x > 59) {
        return None;
    }
    Some(TimeOfDay { sep, h, mi, s, colon, suffix: r[used..].to_string() })
}

/// Shift a parsed date (plus whatever text followed it) by `days`.
/// With a recognisable time of day the whole date-time moves by exactly `days` (to the minute, or
/// to the second when the value has seconds); without one the date moves by `floor(days)`.
fn shift_date_and_time(date: NaiveDate, rest: &str, days: f64, compact: bool, whole: i64) -> Option<String> {
    let dfmt = if compact { "%Y%m%d" } else { "%Y-%m-%d" };
    if let Some(tp) = parse_time_of_day(rest) {
        let secs = if tp.s.is_some() {
            (days * 86400.0).round() as i64
        } else {
            (days * 1440.0).round() as i64 * 60
        };
        let dt = date.and_hms_opt(tp.h, tp.mi, tp.s.unwrap_or(0))?;
        let nd = dt.checked_add_signed(chrono::Duration::seconds(secs))?;
        let time = match (tp.colon, tp.s.is_some()) {
            (true, true) => nd.format("%H:%M:%S"),
            (true, false) => nd.format("%H:%M"),
            (false, true) => nd.format("%H%M%S"),
            (false, false) => nd.format("%H%M"),
        };
        return Some(format!("{}{}{}{}", nd.format(dfmt), tp.sep, time, tp.suffix));
    }
    let nd = date.checked_add_signed(chrono::Duration::days(whole))?;
    Some(format!("{}{}", nd.format(dfmt), rest))
}

pub(crate) fn parse_date_prefix(s: &str) -> Option<(NaiveDate, String)> {
    const MONTHS: [&str; 12] = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    let t = s.trim();
    // YYYY-MM-DD or YYYY/MM/DD optionally followed by time
    if t.len() >= 10 {
        let head = &t[..10];
        let sep = if head.chars().nth(4) == Some('-') {
            '-'
        } else if head.chars().nth(4) == Some('/') {
            '/'
        } else {
            '\0'
        };
        if sep != '\0' {
            let y: i32 = head[0..4].parse().ok()?;
            let m: u32 = head[5..7].parse().ok()?;
            let d: u32 = head[8..10].parse().ok()?;
            let date = NaiveDate::from_ymd_opt(y, m, d)?;
            let rest = t[10..].to_string();
            return Some((date, rest));
        }
    }
    if t.len() >= 8 && t[..8].chars().all(|c| c.is_ascii_digit()) {
        let y: i32 = t[0..4].parse().ok()?;
        let m: u32 = t[4..6].parse().ok()?;
        let d: u32 = t[6..8].parse().ok()?;
        let date = NaiveDate::from_ymd_opt(y, m, d)?;
        return Some((date, t[8..].to_string()));
    }
    // DD-MMM-YYYY
    let up = t.to_ascii_uppercase();
    for (i, mon) in MONTHS.iter().enumerate() {
        if let Some(pos) = up.find(mon) {
            if pos >= 1 {
                let day: u32 = t[..pos].trim_matches(|c: char| !c.is_ascii_digit()).parse().ok()?;
                let after = &t[pos + 3..];
                let year_s: String = after.chars().filter(|c| c.is_ascii_digit()).take(4).collect();
                if year_s.len() == 4 {
                    let y: i32 = year_s.parse().ok()?;
                    let date = NaiveDate::from_ymd_opt(y, (i as u32) + 1, day)?;
                    return Some((date, String::new()));
                }
            }
        }
    }
    None
}

pub fn unique_count(ds: &Dataset, col: &str) -> usize {
    let Some(i) = ds.col_index(col) else {
        return 0;
    };
    let mut set = HashSet::new();
    for row in &ds.rows {
        if let Some(v) = row.get(i) {
            let t = v.trim();
            if !t.is_empty() && t != "." {
                set.insert(t.to_string());
            }
        }
    }
    set.len()
}

pub fn run(opts: &RunOptions, log: &mut TraceLog) -> Result<RunArtifacts> {
    log.info(format!(
        "run start input={} output={} level={}",
        opts.input.display(),
        opts.output_dir.display(),
        opts.level
    ));
    fs::create_dir_all(&opts.output_dir)
        .with_context(|| format!("creating output dir {}", opts.output_dir.display()))?;

    let ds = crate::dataset::read_dataset(
        &opts.input,
        opts.delimiter,
        opts.no_header,
        &opts.columns,
        &opts.comment_chars,
    )?;
    log.success(format!(
        "read {} rows × {} cols ({}) from {}",
        ds.n_rows(),
        ds.n_cols(),
        ds.delimiter.name(),
        ds.source_name
    ));

    let Detailed { mut out, key_rows, plan, date_shift, scales, per_value } = obfuscate_detailed(&ds, opts, log)?;
    emit_warnings(&ds, &plan, &out, opts, log);
    // Column names of the masked dataset before any renaming (keymap.txt section 5 lists them in order).
    let out_headers_original = out.headers.clone();

    // Subject counts use the original ID column name, so count before any renaming.
    let id_name = plan
        .iter()
        .find(|p| p.role == Role::Id)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "ID".into());
    let output_subjects = unique_count(&out, &id_name);

    // Optional column renaming (rename_columns / rename_except). Done last, so every detection
    // rule above worked on the original names; the mapping goes to keymap.txt.
    let rename_plan = crate::rename::plan(&out.headers, opts)?;
    for w in &rename_plan.warnings {
        log.warn(w.clone());
    }
    let renames = rename_plan.pairs;
    let auto_renamed = rename_plan.auto_names;
    if !renames.is_empty() {
        crate::rename::apply(&mut out, &renames);
        for (old, new) in &renames {
            log.info(format!("renamed column {old} -> {new}"));
        }
        log.success(format!(
            "renamed {} column(s) in the masked dataset ({})",
            renames.len(),
            if opts.write_keymap { "old -> new names are in keymap.txt" } else { "write_keymap is off: the mapping is only in column_report.txt, summary.txt and the log" }
        ));
        if !opts.write_keymap {
            log.warn("columns were renamed but keymap.txt is switched off: keep column_report.txt (or the log) to translate the new names back, and never share them");
        }
    }
    let new_name_of = |orig: &str| -> String {
        renames
            .iter()
            .find(|(o, _)| o.eq_ignore_ascii_case(orig))
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| orig.to_string())
    };

    let out_name = opts.output_name.clone().unwrap_or_else(|| match ds.delimiter {
        crate::dataset::Delimiter::Tab => "obfuscated.tsv".into(),
        crate::dataset::Delimiter::Whitespace => "obfuscated.dat".into(),
        _ => "obfuscated.csv".into(),
    });
    let output_dataset = opts.output_dir.join(&out_name);
    let out_name_pre = out_name.clone();
    write_dataset(&output_dataset, &out)?;
    log.success(format!(
        "wrote dataset {} ({} rows × {} cols)",
        output_dataset.display(),
        out.n_rows(),
        out.n_cols()
    ));

    let output_sha256 = {
        let bytes = fs::read(&output_dataset)
            .with_context(|| format!("re-reading {} to fingerprint it", output_dataset.display()))?;
        hex::encode(Sha256::digest(&bytes))
    };
    log.success(format!(
        "dataset SHA-256 {output_sha256} (identical on every re-run with the same input, options and seed {})",
        opts.seed
    ));

    let config_used_path = opts.output_dir.join("config.used.txt");
    fs::write(&config_used_path, opts.dump())?;
    log.success(format!("wrote {}", config_used_path.display()));

    let column_report_path = opts.output_dir.join("column_report.txt");
    fs::write(&column_report_path, render_column_report(&plan, &ds, &out, &renames))?;
    log.success(format!("wrote {}", column_report_path.display()));

    // keymap_columns.txt (level 4): written first so keymap.txt can name it; it holds the factor of every scaled cell.
    let keymap_columns_path = match (&per_value, opts.write_keymap) {
        (Some(pv), true) => {
            let p = opts.output_dir.join("keymap_columns.txt");
            let names = |idx: &[usize]| -> Vec<(String, String)> {
                idx.iter().map(|&i| (ds.headers[i].clone(), new_name_of(&ds.headers[i]))).collect()
            };
            let key_names = names(&pv.key_cols);
            let value_idx: Vec<usize> = pv.value_cols.iter().map(|v| v.0).collect();
            let value_names = names(&value_idx);
            let mut body = String::new();
            let rule = "# ==============================================================================\n";
            body.push_str(rule);
            body.push_str("#  DATA OBFUSCATOR  -  PER-VALUE FACTORS  (keymap_columns.txt)\n");
            body.push_str("#  Level 4 gave every number in the columns below its OWN random factor. This file records\n");
            body.push_str("#  that factor for every value, so the `reverse` command can restore the numbers exactly.\n");
            body.push_str(rule);
            body.push_str("#\n");
            body.push_str("#  !! CONFIDENTIAL - KEEP OFFLINE !!  Same rules as keymap.txt: never share it.\n");
            body.push_str("#\n");
            body.push_str(&format!("#  Program         : data-obfuscator {} (released {})\n", crate::APP_VERSION, crate::APP_RELEASED));
            body.push_str(&format!("#  Masked dataset  : {out_name_pre}\n"));
            body.push_str(&format!("#  Masked SHA-256  : {output_sha256}\n"));
            body.push_str(&format!("#  Rows described  : {} (every data row of the source, in source order)\n", pv.lines.len()));
            body.push_str("#\n");
            body.push_str("#  HOW TO READ IT\n");
            body.push_str("#    - Lines starting with '#' are comments. Lines starting with '@' describe the layout.\n");
            body.push_str("#      Every other line is one data row of the source, TAB-separated:\n");
            body.push_str("#        source_row <TAB> key values ... <TAB> masked value, factor (for each value column)\n");
            body.push_str("#    - source_row is the position of the row in the SOURCE dataset (1 = first data row), so the\n");
            body.push_str("#      original row order can be restored even though level 4 sorts rows by masked ID.\n");
            body.push_str("#    - The key values (@key_columns) are what the masked dataset shows for the identifier columns,\n");
            body.push_str("#      such as ID and TIME. A row is found by those values plus the masked numbers, so the masked\n");
            body.push_str("#      dataset may be re-sorted, and its columns re-ordered, before it is reversed.\n");
            body.push_str("#    - factor is what the ORIGINAL value was multiplied by (@operations says multiply) or divided by\n");
            body.push_str("#      (divide) to give the masked value. '.' means the cell was blank or text and not scaled.\n");
            body.push_str("#    - Names in '@...' lines are the names in the MASKED dataset (after any renaming);\n");
            body.push_str("#      '@..._original' lines give the original names.\n");
            body.push_str(rule);
            let tabs = |v: Vec<String>| v.join("\t");
            body.push_str("@format\t1\n");
            body.push_str(&format!("@masked_sha256\t{output_sha256}\n"));
            body.push_str(&format!("@key_columns\t{}\n", tabs(key_names.iter().map(|k| k.1.clone()).collect())));
            body.push_str(&format!("@key_columns_original\t{}\n", tabs(key_names.iter().map(|k| k.0.clone()).collect())));
            body.push_str(&format!("@value_columns\t{}\n", tabs(value_names.iter().map(|k| k.1.clone()).collect())));
            body.push_str(&format!("@value_columns_original\t{}\n", tabs(value_names.iter().map(|k| k.0.clone()).collect())));
            body.push_str(&format!(
                "@operations\t{}\n",
                tabs(pv.value_cols.iter().map(|&(_, si)| scales[si].operation().to_string()).collect())
            ));
            body.push_str(&format!(
                "@base_factors\t{}\n",
                tabs(pv.value_cols.iter().map(|&(_, si)| format!("{}", scales[si].factor)).collect())
            ));
            body.push_str(&format!("@variation\t+/-{:.0}%\n", LEVEL4_SPREAD * 100.0));
            for l in &pv.lines {
                body.push_str(l);
                body.push('\n');
            }
            fs::write(&p, body)?;
            log.success(format!(
                "wrote {} ({} rows: the factor of every scaled value, needed to reverse level 4 exactly) - keep it offline with keymap.txt",
                p.display(),
                pv.lines.len()
            ));
            Some(p)
        }
        _ => None,
    };
    if keymap_columns_path.is_none() && opts.output_dir.join("keymap_columns.txt").exists() {
        log.warn("keymap_columns.txt from an EARLIER run is still in the output folder (this run did not write one); it does not belong to this dataset and `reverse` would refuse it - delete it or keep it private");
    }

    let keymap_path = if opts.write_keymap {
        let p = opts.output_dir.join("keymap.txt");
        let date_cols: Vec<&str> = plan
            .iter()
            .filter(|p| p.role == Role::Date && p.recode && !p.drop)
            .map(|p| p.name.as_str())
            .collect();
        let cols_txt = if date_cols.is_empty() { "(none)".to_string() } else { date_cols.join(",") };
        let recoded_cols = {
            let mut c: Vec<&str> = key_rows.iter().map(|k| k.column.as_str()).collect();
            c.sort();
            c.dedup();
            c.len()
        };
        // Columns whose original values are not stored anywhere: redacted free text, and columns that are
        // not in the masked dataset at all (dropped, or not listed in keep).
        let not_restorable: Vec<(String, String)> = {
            let mut v: Vec<(String, String)> = Vec::new();
            for p in plan.iter().filter(|p| p.redact && !p.drop) {
                if out_headers_original.iter().any(|h| h.eq_ignore_ascii_case(&p.name)) {
                    v.push((p.name.clone(), "redacted: every cell was replaced by '.', the text is not stored".into()));
                }
            }
            for h in &ds.headers {
                if !out_headers_original.iter().any(|o| o.eq_ignore_ascii_case(h)) {
                    v.push((h.clone(), "not in the masked dataset (dropped, or not listed in keep): its values are not stored".into()));
                }
            }
            v
        };
        let rule = "# ==============================================================================\n";
        let thin = "# ------------------------------------------------------------------------------\n";
        let mut body = String::new();
        body.push_str(rule);
        body.push_str("#  DATA OBFUSCATOR  -  DE-IDENTIFICATION KEY  (keymap.txt)\n");
        body.push_str("#  The secret that turns the masked dataset back into the original.\n");
        body.push_str(rule);
        body.push_str("#\n");
        body.push_str("#  !! CONFIDENTIAL - KEEP OFFLINE !!\n");
        body.push_str("#  Anyone who holds this file together with the masked dataset can undo the masking.\n");
        body.push_str("#  Never send it to an LLM, a vendor, a CRO or a collaborator, and do not paste it\n");
        body.push_str("#  into a chat. Share only the masked dataset (and llm.txt), never this file.\n");
        body.push_str("#\n");
        body.push_str(&format!("#  Program         : data-obfuscator {} (released {})\n", crate::APP_VERSION, crate::APP_RELEASED));
        body.push_str(&format!("#  Source dataset  : {}\n", ds.source_name));
        body.push_str(&format!("#  Masked dataset  : {}\n", out_name));
        body.push_str(&format!("#  Masked SHA-256  : {}\n", output_sha256));
        body.push_str(&format!("#  Masking level   : {} ({})\n", opts.level, opts.level_name()));
        body.push_str(&format!("#  Random seed     : {}\n", opts.seed));
        body.push_str(&format!("#  ID style        : {}\n", opts.id_style.as_str()));
        body.push_str(&match &keymap_columns_path {
            Some(_) => format!(
                "#  Per-value file  : keymap_columns.txt ({} rows) - needed to reverse level 4 exactly; keep it with this file\n",
                per_value.as_ref().map(|p| p.lines.len()).unwrap_or(0)
            ),
            None => "#  Per-value file  : none (no value was given its own factor)\n".to_string(),
        });
        body.push_str("#\n");
        body.push_str("#  To undo the masking run:  data-obfuscator reverse --input <masked dataset> --keymap keymap.txt\n");
        body.push_str("#\n");
        body.push_str("#  WHAT IS IN THIS FILE\n");
        body.push_str(&format!(
            "#    1. COLUMN RENAMING                  {}\n",
            if renames.is_empty() { "none (no column was renamed)".to_string() } else { format!("{} column(s) renamed", renames.len()) }
        ));
        body.push_str(&format!(
            "#    2. NUMERIC SCALING                  {}\n",
            if scales.is_empty() { "none (scaling starts at level 3)".to_string() } else { format!("{} column(s) multiplied or divided", scales.len()) }
        ));
        body.push_str(&format!(
            "#    3. DATE SHIFT                       {} day(s) added to: {}\n",
            fmt_days(date_shift),
            cols_txt
        ));
        body.push_str(&format!(
            "#    4. IDENTIFIER AND LABEL RECODING    {} value(s) in {} column(s)\n",
            key_rows.len(),
            recoded_cols
        ));
        body.push_str(&format!(
            "#    5. COLUMN ORDER, NOT RESTORABLE     {} column(s) in order; {} column(s) whose values are not stored\n",
            out_headers_original.len(),
            not_restorable.len()
        ));
        body.push_str("#\n");
        body.push_str("#  HOW TO READ IT\n");
        body.push_str("#    - Lines that start with '#' are comments. Every other line is TAB-separated.\n");
        body.push_str("#    - Column names in sections 2-4 are the ORIGINAL names; section 1 shows what each\n");
        body.push_str("#      renamed column is called in the masked dataset.\n");
        body.push_str("#    - Using the same input, options and seed always gives this same key and the same\n");
        body.push_str("#      masked dataset (compare the SHA-256 above).\n");
        body.push_str(rule);
        body.push_str("#\n");
        body.push_str(thin);
        body.push_str("# 1. COLUMN RENAMING - column names that were changed in the masked dataset.\n");
        body.push_str("# Format: renamed_column<TAB>original name<TAB>new name\n");
        body.push_str(thin);
        if renames.is_empty() {
            body.push_str("# (no columns were renamed: see rename_columns / rename_except)\n");
        }
        for (old, new) in &renames {
            body.push_str(&format!("renamed_column\t{old}\t{new}\n"));
        }
        body.push_str("#\n");
        body.push_str(thin);
        body.push_str("# 2. NUMERIC SCALING - every valid number in the column was multiplied or divided by the factor.\n");
        body.push_str("# To restore the original: masked value divided by factor (multiply) or times factor (divide).\n");
        body.push_str("# Variation 'none' (level 3): that one factor was used for every value, so the restore is exact.\n");
        body.push_str("# Variation '+/-15%' (level 4): the factor is only the BASE; each value was multiplied or divided by\n");
        body.push_str("# its own random factor within +/-15 % of the base. Those per-value factors are in keymap_columns.txt\n");
        body.push_str("# (the 'reverse' command uses it); without that file a restore is only approximate (within about 15 %).\n");
        body.push_str("# Blank, '.' and text cells are unchanged.\n");
        body.push_str("# Format: column<TAB>operation<TAB>base factor<TAB>values changed<TAB>variation per value\n");
        body.push_str("# (the column is listed under its ORIGINAL name; see section 1 for its new name)\n");
        body.push_str(thin);
        if scales.is_empty() {
            body.push_str("# (no columns were scaled: scaling applies from level 3)\n");
        }
        for sc in &scales {
            body.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\n",
                sc.column,
                sc.operation(),
                sc.factor,
                sc.values_changed,
                sc.variation()
            ));
        }
        body.push_str("#\n");
        body.push_str(thin);
        body.push_str("# 3. DATE SHIFT - one random number of days (a decimal, e.g. 23.6) was ADDED to every date in the\n");
        body.push_str("# date columns. It is the same for the whole dataset, so intervals between dates are preserved.\n");
        body.push_str("# Date-times (date plus a time of day) and numeric day counts moved by the exact value; date-only\n");
        body.push_str("# text dates moved by the whole days only (the fraction is dropped). To restore the originals,\n");
        body.push_str("# subtract the same amount. Format: name<TAB>days<TAB>columns\n");
        body.push_str(thin);
        body.push_str(&format!(
            "date_shift_days\t{}\t{}\ndate_shift_whole_days\t{}\t{}\n",
            fmt_days(date_shift),
            cols_txt,
            fmt_days(date_shift.floor()),
            cols_txt
        ));
        body.push_str("#\n");
        body.push_str(thin);
        body.push_str("# 4. IDENTIFIER AND LABEL RECODING - original value and what it became.\n");
        body.push_str("# Format: column<TAB>original<TAB>masked\n");
        body.push_str(thin);
        body.push_str("column\toriginal\tmasked\n");
        for k in &key_rows {
            body.push_str(&format!("{}\t{}\t{}\n", k.column, k.original, k.masked));
        }
        body.push_str("#\n");
        body.push_str(thin);
        body.push_str("# 5. COLUMN ORDER, AND WHAT CANNOT BE RESTORED\n");
        body.push_str("# column_order lists the columns in the order they were written to the masked dataset.\n");
        body.push_str("#   Format: column_order<TAB>position<TAB>original name<TAB>name in the masked dataset\n");
        body.push_str("# not_restorable lists columns whose original values are not stored in any file.\n");
        body.push_str("#   Format: not_restorable<TAB>original name<TAB>why\n");
        body.push_str(thin);
        for (i, h) in out_headers_original.iter().enumerate() {
            body.push_str(&format!("column_order\t{}\t{}\t{}\n", i + 1, h, new_name_of(h)));
        }
        if not_restorable.is_empty() {
            body.push_str("# (every column of the source is in the masked dataset and none was redacted)\n");
        }
        for (name, why) in &not_restorable {
            body.push_str(&format!("not_restorable\t{name}\t{why}\n"));
        }
        fs::write(&p, body)?;
        log.success(format!(
            "wrote key map {} (original values, scaling factors, date shift) - keep it offline and never share it",
            p.display()
        ));
        Some(p)
    } else {
        None
    };

    let recoded: Vec<String> = plan
        .iter()
        .filter(|p| p.recode && !p.drop)
        .map(|p| p.name.clone())
        .collect();
    let dropped: Vec<String> = plan.iter().filter(|p| p.drop).map(|p| p.name.clone()).collect();
    let protected: Vec<String> = plan
        .iter()
        .filter(|p| p.protect && !p.drop)
        .map(|p| p.name.clone())
        .collect();

    let summary = Summary {
        generated: chrono::Utc::now()
            .format("%Y-%m-%d %H:%M:%S UTC")
            .to_string(),
        program: format!("data-obfuscator {} (released {})", crate::APP_VERSION, crate::APP_RELEASED),
        command: "run".into(),
        input_file: ds.source_name.clone(),
        input_format: ds.delimiter.name().into(),
        input_rows: ds.n_rows(),
        input_cols: ds.n_cols(),
        input_subjects: unique_count(&ds, &id_name),
        input_bytes: fs::metadata(&opts.input).map(|m| m.len()).unwrap_or(0),
        output_file: out_name,
        output_rows: out.n_rows(),
        output_cols: out.n_cols(),
        output_subjects,
        output_sha256: output_sha256.clone(),
        level: opts.level,
        level_name: opts.level_name().into(),
        seed: opts.seed,
        id_style: opts.id_style.as_str().into(),
        recoded,
        dropped,
        kept: out.headers.clone(),
        protected,
        date_shift,
        renamed: renames.clone(),
        scaled_columns: scales.iter().map(|s| s.column.clone()).collect(),
        scale_range: (opts.scale_min, opts.scale_max),
        keymap: keymap_path.is_some(),
        keymap_columns: keymap_columns_path.is_some(),
        status: "SUCCESS".into(),
    };
    let summary_path = opts.output_dir.join("summary.txt");
    fs::write(&summary_path, summary::render(&summary))?;
    log.success(format!("wrote {}", summary_path.display()));

    // llm.txt is written before tokens.txt so the token report counts it too.
    let dataset_text = fs::read_to_string(&output_dataset)
        .with_context(|| format!("re-reading {} for token estimates", output_dataset.display()))?;
    let est = tokens::count_all_models(&dataset_text);
    let llm_path = opts.output_dir.join("llm.txt");
    let llm_body = llm::render(
        &out,
        &LlmContext {
            file_name: &summary.output_file,
            level: opts.level,
            level_name: opts.level_name(),
            scale_varies: scales.iter().any(|s| s.spread > 0.0),
            scaled: scales
                .iter()
                .map(|s| new_name_of(&s.column))
                .filter(|n| out.col_index(n).is_some())
                .collect(),
            renamed: auto_renamed.clone(),
            token_estimates: MODELS
                .iter()
                .zip(est.iter())
                .map(|(m, n)| (m.label.to_string(), *n))
                .collect(),
        },
    );
    fs::write(&llm_path, llm_body)?;
    log.success(format!(
        "wrote {} (analysis, NONMEM control-stream and QC prompts tailored to the dataset)",
        llm_path.display()
    ));

    let mut report = TokenReport {
        tokenizer: TOKENIZER_NAME.into(),
        input: Some(tokens::file_row("input", &opts.input)?),
        output_dataset: Some(tokens::file_row("output dataset", &output_dataset)?),
        other: Vec::new(),
        excluded: Vec::new(),
    };
    let log_path = log.path().to_path_buf();
    collect_other_files(
        &opts.output_dir,
        &output_dataset,
        &log_path,
        &mut report,
    )?;
    let tokens_path = opts.output_dir.join("tokens.txt");
    // tokens.txt is included in the listing: write a placeholder, recount others, then rewrite.
    tokens::write_token_report(&tokens_path, &report)?;
    report.other.retain(|r| r.path != "tokens.txt");
    if let Ok(row) = tokens::file_row("tokens.txt", &tokens_path) {
        report.other.push(row);
        report.other.sort_by(|a, b| a.path.cmp(&b.path));
    }
    tokens::write_token_report(&tokens_path, &report)?;
    log.success(format!(
        "wrote {} (input {} tokens, output {} tokens; GPT-4 estimate, other models in the file)",
        tokens_path.display(),
        report.total_input_tokens(),
        report.total_output_dataset_tokens()
    ));

    log.success("run complete SUCCESS");
    Ok(RunArtifacts {
        output_dataset,
        summary_path,
        tokens_path,
        log_path,
        config_used_path,
        column_report_path,
        llm_path,
        warnings: log.warn_count(),
        keymap_path,
        keymap_columns_path,
        dataset: out,
        summary,
        token_report: report,
    })
}

/// "08:15" or "8:15:30" - a clock time carries no identity and is common in QC datasets.
fn looks_like_clock(t: &str) -> bool {
    let parts: Vec<&str> = t.split(':').collect();
    (parts.len() == 2 || parts.len() == 3)
        && parts.iter().all(|p| !p.is_empty() && p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit()))
}

/// Orange warnings: things that make the masked output less safe or less useful.
fn emit_warnings(ds: &Dataset, plan: &[ColumnPlan], out: &Dataset, opts: &RunOptions, log: &TraceLog) {
    if !plan.iter().any(|p| p.role == Role::Id && !p.drop) {
        log.warn("no subject ID column detected - subjects are not recoded and llm.txt cannot count them; set --id-column");
    }
    let role_lists: [(&str, &Vec<String>); 6] = [
        ("id_column", &opts.id_column),
        ("subject_column", &opts.subject_column),
        ("study_column", &opts.study_column),
        ("site_column", &opts.site_column),
        ("treatment_column", &opts.treatment_column),
        ("user_column", &opts.user_column),
    ];
    for (opt, names) in role_lists {
        for n in names {
            if ds.col_index(n).is_none() {
                log.warn(format!("{opt} lists '{n}' but the input has no such column - ignored"));
            }
        }
    }
    if ds.ragged_rows > 0 {
        log.warn(format!(
            "{} data row(s) do not have {} fields like the header - NONMEM would reject this file. {}",
            ds.ragged_rows,
            ds.n_cols(),
            if out.ragged_rows > 0 {
                "They keep their shape in the masked copy so the problem stays visible; fields beyond the header are copied UNMASKED."
            } else {
                "Because columns were kept/dropped, they were reshaped to the kept columns in the masked copy."
            }
        ));
    }
    if !opts.write_keymap && opts.output_dir.join("keymap.txt").exists() {
        log.warn("keymap.txt from an EARLIER run is still in the output folder (this run did not write one); it does not belong to this dataset - delete it or keep it private");
    }
    if let Some(keep) = &opts.keep {
        for k in keep {
            if ds.col_index(k).is_none() {
                log.warn(format!("--keep lists '{k}' but the input has no such column - ignored"));
            }
        }
    }
    let removed: Vec<&str> = plan
        .iter()
        .filter(|p| p.role == Role::Protect && p.drop)
        .map(|p| p.name.as_str())
        .collect();
    // Only core columns are worth a warning; trimming helpers such as TAD or MDV is routine.
    let essential: Vec<&str> = removed
        .iter()
        .copied()
        .filter(|n| ["TIME", "DV", "AMT", "EVID"].contains(&n.to_ascii_uppercase().as_str()))
        .collect();
    if !essential.is_empty() {
        log.warn(format!(
            "core PK/PD column(s) removed by keep/drop: {} - the dataset may no longer be analysable",
            essential.join(", ")
        ));
    } else if !removed.is_empty() {
        log.info(format!(
            "protected helper column(s) not kept: {} (core TIME/DV/AMT/EVID columns are intact)",
            removed.join(", ")
        ));
    }
    for name in ["TIME", "DV"] {
        if out.col_index(name).is_none() {
            log.warn(format!("output has no {name} column - NONMEM-style analysis and the llm.txt prompts will be limited"));
        }
    }
    // Unrecognised columns that hold text are passed through untouched - that is where an
    // un-named study title, site name or sample label would slip out.
    let text_passthrough: Vec<&str> = plan
        .iter()
        .filter(|p| p.role == Role::Other && !p.drop && !p.protect)
        .filter(|p| {
            ds.rows.iter().any(|r| {
                r.get(p.index).is_some_and(|v| {
                    let t = v.trim();
                    !t.is_empty() && t != "." && t.parse::<f64>().is_err() && !looks_like_clock(t)
                })
            })
        })
        .map(|p| p.name.as_str())
        .collect();
    if !text_passthrough.is_empty() {
        log.warn(format!(
            "text column(s) not recognised and passed through UNCHANGED: {} - if they identify a study, site, person or sample, name them in sensitive / freetext_columns / a role option (e.g. site_column) or drop them",
            text_passthrough.join(", ")
        ));
    }
    if opts.level < 2 {
        for p in plan.iter().filter(|p| p.role == Role::Date && !p.drop) {
            log.warn(format!(
                "date column {} is kept but NOT shifted at level {} - use level 2 or higher to shift dates",
                p.name, opts.level
            ));
        }
    }
    for (i, h) in out.headers.iter().enumerate() {
        let looks_like_email = out
            .rows
            .iter()
            .any(|r| r.get(i).is_some_and(|v| v.contains('@') && v.contains('.')));
        if looks_like_email {
            log.warn(format!(
                "column {h} appears to contain e-mail addresses and was not redacted - add it with --freetext-columns or --drop"
            ));
        }
    }
}

fn collect_other_files(
    dir: &Path,
    dataset: &Path,
    log_path: &Path,
    report: &mut TokenReport,
) -> Result<()> {
    let ds_name = dataset.file_name();
    let log_name = log_path.file_name();
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    entries.sort();
    for p in entries {
        let name = p.file_name();
        if name == ds_name {
            continue;
        }
        let is_log = name == log_name
            || p.extension().and_then(|s| s.to_str()) == Some("log")
            || p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("obfuscator.log"));
        match tokens::file_row(&p.display().to_string(), &p) {
            Ok(mut row) => {
                if is_log {
                    row.excluded = true;
                    report.excluded.push(row);
                } else {
                    report.other.push(row);
                }
            }
            Err(e) => {
                // skip unreadable
                let _ = e;
            }
        }
    }
    Ok(())
}

fn render_column_report(
    plan: &[ColumnPlan],
    input: &Dataset,
    output: &Dataset,
    renames: &[(String, String)],
) -> String {
    let mut s = String::from("DATA OBFUSCATOR — COLUMN REPORT\n================================\n\n");
    s.push_str(&format!(
        "{:<16} {:<12} {:<8} {:<8} {:<8} {:<8} {}\n",
        "COLUMN", "ROLE", "RECODE", "SCALE", "REDACT", "DROP", "ACTION"
    ));
    s.push_str(&format!("{:-<80}\n", ""));
    for p in plan {
        let action = if p.drop {
            "dropped"
        } else if p.protect {
            "protected (unchanged)"
        } else if p.redact {
            "redacted"
        } else if p.recode && p.role == Role::Date {
            "date-shifted"
        } else if p.recode {
            "recoded"
        } else if p.scale {
            "scaled (x or / hidden factor; varies per value at level 4)"
        } else {
            "passed through"
        };
        s.push_str(&format!(
            "{:<16} {:<12} {:<8} {:<8} {:<8} {:<8} {}\n",
            p.name,
            p.role.name(),
            yn(p.recode),
            yn(p.scale),
            yn(p.redact),
            yn(p.drop),
            action
        ));
    }
    s.push_str(&format!(
        "\nInput columns:  {}\nOutput columns: {}\n",
        input.headers.join(", "),
        output.headers.join(", ")
    ));
    if !renames.is_empty() {
        s.push_str("\nRenamed columns (this report shows the ORIGINAL names - do not share it):\n");
        for (old, new) in renames {
            s.push_str(&format!("  {old} -> {new}\n"));
        }
    }
    s
}

fn yn(v: bool) -> &'static str {
    if v {
        "yes"
    } else {
        "no"
    }
}

pub fn run_tokens(input: &Path, output_dir: &Path, extra_exclude: &[String], log: &mut TraceLog) -> Result<PathBuf> {
    fs::create_dir_all(output_dir)?;
    log.info(format!(
        "tokens start input={} output={}",
        input.display(),
        output_dir.display()
    ));
    let mut report = TokenReport {
        tokenizer: TOKENIZER_NAME.into(),
        input: Some(tokens::file_row("input", input)?),
        output_dataset: None,
        other: Vec::new(),
        excluded: Vec::new(),
    };
    if output_dir.exists() {
        let mut entries: Vec<PathBuf> = fs::read_dir(output_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file())
            .collect();
        entries.sort();
        for p in entries {
            let fname = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if fname.eq_ignore_ascii_case("tokens.txt") {
                continue;
            }
            let is_log = fname.ends_with(".log")
                || fname.eq_ignore_ascii_case("obfuscator.log")
                || extra_exclude.iter().any(|e| e.eq_ignore_ascii_case(&fname));
            if let Ok(mut row) = tokens::file_row(&fname, &p) {
                if is_log {
                    row.excluded = true;
                    report.excluded.push(row);
                } else {
                    report.other.push(row);
                }
            }
        }
    }
    let tokens_path = output_dir.join("tokens.txt");
    tokens::write_token_report(&tokens_path, &report)?;
    if let Ok(row) = tokens::file_row("tokens.txt", &tokens_path) {
        report.other.retain(|r| r.path != "tokens.txt");
        report.other.push(row);
        report.other.sort_by(|a, b| a.path.cmp(&b.path));
        tokens::write_token_report(&tokens_path, &report)?;
    }
    log.success(format!(
        "tokens complete input_tokens={} counted_total={}",
        report.total_input_tokens(),
        report.total_counted_tokens()
    ));
    Ok(tokens_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_dates_are_shifted_as_dates_and_stay_compact() {
        assert_eq!(shift_date_cell("20240325", 10.0), ("20240404".into(), true));
        assert_eq!(shift_date_cell("20240331", 1.0), ("20240401".into(), true));
        assert_eq!(shift_date_cell("20231231", 1.0), ("20240101".into(), true));
        assert_eq!(shift_date_cell("20240301", -1.0), ("20240229".into(), true), "leap year");
        assert_eq!(shift_date_cell("202403250830", 10.0), ("202404040830".into(), true));
        assert_eq!(shift_date_cell("20240325083015", 10.0), ("20240404083015".into(), true));
        assert_eq!(shift_date_cell("20240325T0830", 10.0), ("20240404T0830".into(), true));
    }

    #[test]
    fn other_whole_numbers_in_date_columns_are_still_day_counts() {
        assert_eq!(shift_date_cell("45300", 10.0), ("45310".into(), true));
        // 8 digits but not a calendar date (month 56): treated as a number
        assert_eq!(shift_date_cell("12345678", 10.0), ("12345688".into(), true));
        assert_eq!(shift_date_cell("20241399", 1.0), ("20241400".into(), true));
    }

    #[test]
    fn iso_and_blank_dates_still_work() {
        assert_eq!(shift_date_cell("2024-03-25", 10.0), ("2024-04-04".into(), true));
        assert_eq!(shift_date_cell("", 10.0), ("".into(), true));
        assert_eq!(shift_date_cell(".", 10.0), (".".into(), true));
        assert!(!shift_date_cell("25/03/2024", 10.0).1);
    }

    #[test]
    fn fractional_shifts_move_date_times_exactly_and_date_only_values_by_whole_days() {
        // date-only: floor(23.6) = 23 days
        assert_eq!(shift_date_cell("2024-03-25", 23.6), ("2024-04-17".into(), true));
        assert_eq!(shift_date_cell("20240325", 23.6), ("20240417".into(), true));
        assert_eq!(shift_date_cell("25-MAR-2024", 23.6), ("2024-04-17".into(), true));
        assert_eq!(shift_date_cell("2024-03-25", -0.5), ("2024-03-24".into(), true));
        // date-times: the exact fraction, 0.6 day = 14 h 24 min
        assert_eq!(shift_date_cell("2024-03-25 08:00", 23.6), ("2024-04-17 22:24".into(), true));
        assert_eq!(shift_date_cell("2024-03-25T20:00:30", 0.5), ("2024-03-26T08:00:30".into(), true));
        assert_eq!(shift_date_cell("2024-03-25 08:00", -0.5), ("2024-03-24 20:00".into(), true));
        assert_eq!(shift_date_cell("202403252000", 23.6), ("202404181024".into(), true));
        assert_eq!(shift_date_cell("20240325T200030", 0.5), ("20240326T080030".into(), true));
        // intervals between date-times are preserved exactly
        let a = shift_date_cell("2024-03-25 08:00", 37.3).0;
        let b = shift_date_cell("2024-03-26 09:30", 37.3).0;
        let pa = chrono::NaiveDateTime::parse_from_str(&a, "%Y-%m-%d %H:%M").unwrap();
        let pb = chrono::NaiveDateTime::parse_from_str(&b, "%Y-%m-%d %H:%M").unwrap();
        assert_eq!((pb - pa).num_minutes(), 24 * 60 + 90);
        // numeric day counts: exact decimal
        assert_eq!(shift_date_cell("45300", 23.6), ("45323.6".into(), true));
        assert_eq!(shift_date_cell("45300.25", 23.6), ("45323.85".into(), true));
        assert_eq!(fmt_days(23.6), "23.6");
        assert_eq!(fmt_days(100.0), "100");
        assert_eq!(fmt_days(-0.5), "-0.5");
        assert_eq!(fmt_days(-0.0), "0");
    }

    #[test]
    fn scaling_formats_numbers_and_skips_non_numbers() {
        assert_eq!(scale_cell("70", false, 2.0).as_deref(), Some("140"));
        assert_eq!(scale_cell(" 70.5 ", true, 2.0).as_deref(), Some("35.25"));
        assert_eq!(scale_cell(".", false, 2.0), None);
        assert_eq!(scale_cell("", false, 2.0), None);
        assert_eq!(scale_cell("M", false, 2.0), None);
        assert_eq!(scale_cell("NaN", false, 2.0), None);
        assert_eq!(scale_cell("0", true, 3.0).as_deref(), Some("0"));
        assert_eq!(scale_cell("-4", false, 2.5).as_deref(), Some("-10"));
        assert_eq!(fmt_sig(0.00012345678912, 8), "0.00012345679");
        assert_eq!(fmt_sig(12345.678912345, 8), "12345.679");
    }
}
