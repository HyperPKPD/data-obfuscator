//! `reverse`: undo the masking of a dataset with keymap.txt (and keymap_columns.txt for level 4).
//!
//! The masked dataset may have been re-sorted, may have its columns in another order, and may carry
//! extra columns (for example a NONMEM table with PRED and IPRED added). Everything is therefore found
//! by column NAME and, for the per-value factors of level 4, by a row key (ID, TIME, ...) plus the masked
//! numbers themselves - never by position.
//!
//! What can be undone: renamed columns, numeric scaling (exactly at level 3, exactly at level 4 when
//! keymap_columns.txt is present, to within the recorded +/-15 % otherwise), the date shift and the recoding
//! of identifiers and labels. What cannot: redacted free text and columns that were dropped.

use crate::dataset::{read_dataset, write_dataset, Dataset};
use crate::obfuscate::{fmt_sig, unshift_date_cell};
use crate::trace::TraceLog;
use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

/// One part of the masking that `reverse` can undo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Rename,
    Scaling,
    Dates,
    Recoding,
}

impl Step {
    pub const ALL: [Step; 4] = [Step::Rename, Step::Scaling, Step::Dates, Step::Recoding];

    pub fn name(self) -> &'static str {
        match self {
            Step::Rename => "rename",
            Step::Scaling => "scaling",
            Step::Dates => "dates",
            Step::Recoding => "recoding",
        }
    }

    pub fn parse(s: &str) -> Result<Step> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "rename" | "renaming" | "names" | "columns" => Step::Rename,
            "scaling" | "scale" | "numbers" | "numeric" | "covariates" => Step::Scaling,
            "dates" | "date" | "shift" => Step::Dates,
            "recoding" | "recode" | "ids" | "id" | "labels" | "identifiers" => Step::Recoding,
            other => bail!("unknown step '{other}': use rename, scaling, dates or recoding"),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ReverseOptions {
    /// The masked dataset (rows and columns may be in any order).
    pub input: PathBuf,
    /// keymap.txt (default: keymap.txt next to the input).
    pub keymap: Option<PathBuf>,
    /// keymap_columns.txt (default: keymap_columns.txt next to keymap.txt, when it exists).
    pub keymap_columns: Option<PathBuf>,
    pub output_dir: PathBuf,
    pub output_name: Option<String>,
    pub overwrite: bool,
    /// Parts of the masking to undo (default: all).
    pub steps: Vec<Step>,
    /// Stop with an error instead of approximating, leaving values unchanged or skipping missing columns.
    pub strict: bool,
    /// Put columns (and, at level 4, rows) back in their original order.
    pub restore_order: bool,
    pub delimiter: Option<char>,
    pub no_header: bool,
    pub columns: Vec<String>,
    pub comment_chars: String,
    pub verbose: bool,
    /// run2 keys only: `COMBINED=SOURCE` or `COMBINED=SOURCE:WHENCOL=VALUE` rules that tell `reverse` which
    /// source column of the masked ADaM datasets a column of the combined dataset came from.
    pub map: Vec<String>,
    /// run2 keys only: extra columns of the combined dataset that hold shifted dates.
    pub date_columns: Vec<String>,
}

impl Default for ReverseOptions {
    fn default() -> Self {
        Self {
            input: PathBuf::new(),
            keymap: None,
            keymap_columns: None,
            output_dir: PathBuf::from("restored"),
            output_name: None,
            overwrite: false,
            steps: Step::ALL.to_vec(),
            strict: false,
            restore_order: true,
            delimiter: None,
            no_header: false,
            columns: Vec::new(),
            comment_chars: "#;$".into(),
            verbose: false,
            map: Vec::new(),
            date_columns: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------------- keymap.txt

#[derive(Clone, Debug)]
pub struct ScaleEntry {
    /// Original column name.
    pub column: String,
    pub divide: bool,
    pub factor: f64,
    /// Level 4: `factor` is only the base of per-value factors.
    pub varies: bool,
}

#[derive(Clone, Debug, Default)]
pub struct KeyMap {
    pub program: Option<String>,
    pub source: Option<String>,
    pub masked_name: Option<String>,
    pub masked_sha256: Option<String>,
    pub level: Option<u8>,
    /// The header says a keymap_columns.txt was written with this key.
    pub expects_per_value_file: bool,
    pub renames: Vec<(String, String)>,
    pub scales: Vec<ScaleEntry>,
    pub date_shift: Option<f64>,
    pub date_shift_whole: Option<i64>,
    pub date_columns: Vec<String>,
    /// (original column name, masked label -> original value(s)).
    pub recodes: Vec<(String, HashMap<String, Vec<String>>)>,
    /// (original name, name in the masked dataset), in the order the columns were written.
    pub column_order: Vec<(String, String)>,
    /// (original name, why its values are not stored).
    pub not_restorable: Vec<(String, String)>,
}

pub fn parse_keymap(text: &str) -> Result<KeyMap> {
    let mut km = KeyMap::default();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        if let Some(c) = line.strip_prefix('#') {
            if let Some((k, v)) = c.split_once(':') {
                let (k, v) = (k.trim(), v.trim());
                match k {
                    "Program" => km.program = Some(v.to_string()),
                    "Source dataset" => km.source = Some(v.to_string()),
                    "Masked dataset" => km.masked_name = Some(v.to_string()),
                    "Masked SHA-256" => km.masked_sha256 = Some(v.to_ascii_lowercase()),
                    "Masking level" => km.level = v.split_whitespace().next().and_then(|t| t.parse().ok()),
                    "Per-value file" => km.expects_per_value_file = v.starts_with("keymap_columns.txt"),
                    _ => {}
                }
            }
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        match f[0] {
            "renamed_column" if f.len() >= 3 => km.renames.push((f[1].to_string(), f[2].to_string())),
            "date_shift_days" if f.len() >= 2 => {
                km.date_shift = Some(f[1].trim().parse().with_context(|| format!("keymap.txt line {}: date shift '{}'", n + 1, f[1]))?);
                km.date_columns = f.get(2).map(|c| split_columns(c)).unwrap_or_default();
            }
            "date_shift_whole_days" if f.len() >= 2 => {
                km.date_shift_whole = Some(
                    f[1].trim()
                        .parse::<f64>()
                        .with_context(|| format!("keymap.txt line {}: whole-day shift '{}'", n + 1, f[1]))?
                        .round() as i64,
                );
            }
            "column_order" if f.len() >= 4 => km.column_order.push((f[2].to_string(), f[3].to_string())),
            "not_restorable" if f.len() >= 3 => km.not_restorable.push((f[1].to_string(), f[2].to_string())),
            _ => {
                if f.len() >= 4 && (f[1] == "multiply" || f[1] == "divide") {
                    if let Ok(factor) = f[2].trim().parse::<f64>() {
                        km.scales.push(ScaleEntry {
                            column: f[0].to_string(),
                            divide: f[1] == "divide",
                            factor,
                            varies: f.get(4).map(|v| v.trim() != "none").unwrap_or(false),
                        });
                        continue;
                    }
                }
                if f.len() == 3 {
                    if f[0] == "column" && f[1] == "original" && f[2] == "masked" {
                        continue; // the header line of section 4
                    }
                    let map = match km.recodes.iter_mut().find(|(c, _)| c == f[0]) {
                        Some((_, m)) => m,
                        None => {
                            km.recodes.push((f[0].to_string(), HashMap::new()));
                            &mut km.recodes.last_mut().unwrap().1
                        }
                    };
                    map.entry(f[2].to_string()).or_default().push(f[1].to_string());
                }
            }
        }
    }
    if km.renames.is_empty()
        && km.scales.is_empty()
        && km.date_shift.is_none()
        && km.recodes.is_empty()
        && km.column_order.is_empty()
    {
        bail!("this does not look like a keymap.txt written by data-obfuscator: no renamed columns, scaling factors, date shift or recoded values were found");
    }
    Ok(km)
}

fn split_columns(s: &str) -> Vec<String> {
    let t = s.trim();
    if t.is_empty() || t == "(none)" {
        return Vec::new();
    }
    t.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect()
}

// ------------------------------------------------------------------------ keymap_columns.txt

#[derive(Clone, Debug)]
pub struct PvRowRecord {
    pub orig_row: usize,
    pub key: Vec<String>,
    pub cells: Vec<(String, Option<f64>)>,
}

#[derive(Clone, Debug, Default)]
pub struct PerValueFile {
    pub masked_sha256: Option<String>,
    pub key_cols: Vec<String>,
    pub key_cols_original: Vec<String>,
    pub value_cols: Vec<String>,
    pub value_cols_original: Vec<String>,
    pub rows: Vec<PvRowRecord>,
}

pub fn parse_keymap_columns(text: &str) -> Result<PerValueFile> {
    let mut pv = PerValueFile::default();
    let mut seen_layout = false;
    let tail = |f: &[&str]| -> Vec<String> { f[1..].iter().map(|s| s.to_string()).collect() };
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if let Some(d) = f[0].strip_prefix('@') {
            match d {
                "masked_sha256" => pv.masked_sha256 = f.get(1).map(|s| s.trim().to_ascii_lowercase()),
                "key_columns" => {
                    pv.key_cols = if f.len() > 1 { tail(&f) } else { Vec::new() };
                    seen_layout = true;
                }
                "key_columns_original" => pv.key_cols_original = if f.len() > 1 { tail(&f) } else { Vec::new() },
                "value_columns" => {
                    pv.value_cols = tail(&f);
                    seen_layout = true;
                }
                "value_columns_original" => pv.value_cols_original = tail(&f),
                _ => {}
            }
            continue;
        }
        if !seen_layout || pv.value_cols.is_empty() {
            bail!("keymap_columns.txt line {}: data before the @key_columns / @value_columns lines", n + 1);
        }
        let expect = 1 + pv.key_cols.len() + 2 * pv.value_cols.len();
        if f.len() != expect {
            bail!(
                "keymap_columns.txt line {}: {} fields, expected {} (1 row number + {} key + 2 x {} value columns)",
                n + 1,
                f.len(),
                expect,
                pv.key_cols.len(),
                pv.value_cols.len()
            );
        }
        let orig_row: usize = f[0].trim().parse().with_context(|| format!("keymap_columns.txt line {}: row number '{}'", n + 1, f[0]))?;
        let k = pv.key_cols.len();
        let key = f[1..1 + k].iter().map(|s| s.to_string()).collect();
        let cells = (0..pv.value_cols.len())
            .map(|i| {
                let text = f[1 + k + 2 * i].to_string();
                let fac = f[2 + k + 2 * i].trim();
                let factor = if fac == "." { None } else { fac.parse::<f64>().ok() };
                (text, factor)
            })
            .collect();
        pv.rows.push(PvRowRecord { orig_row, key, cells });
    }
    if pv.value_cols.is_empty() {
        bail!("this does not look like a keymap_columns.txt written by data-obfuscator: no @value_columns line");
    }
    if pv.value_cols_original.len() != pv.value_cols.len() {
        pv.value_cols_original = pv.value_cols.clone();
    }
    if pv.key_cols_original.len() != pv.key_cols.len() {
        pv.key_cols_original = pv.key_cols.clone();
    }
    Ok(pv)
}

// ------------------------------------------------------------------------------------ helpers

/// Number text in one canonical form, so "7.50", "7.5" and "7.5000000" compare equal.
fn canon(text: &str) -> String {
    let t = text.trim();
    match t.parse::<f64>() {
        Ok(v) if v.is_finite() => fmt_sig(v, 8),
        _ => t.to_string(),
    }
}

fn pos_ci(headers: &[String], name: &str) -> Option<usize> {
    let n = name.trim();
    headers.iter().position(|h| h.trim().eq_ignore_ascii_case(n))
}

/// Index of a column given its ORIGINAL name: under its new name when it was renamed, else under the original.
fn locate(headers: &[String], renames: &[(String, String)], orig: &str) -> Option<usize> {
    if let Some((_, new)) = renames.iter().find(|(o, _)| o.eq_ignore_ascii_case(orig)) {
        if let Some(i) = pos_ci(headers, new) {
            return Some(i);
        }
    }
    pos_ci(headers, orig)
}

fn round_sig(x: f64, k: usize) -> f64 {
    format!("{:.*e}", k.saturating_sub(1), x).parse().unwrap_or(x)
}

/// Undo one scaled number. Finds the shortest decimal that the forward pass turns back into exactly this
/// masked text, which is the original value as long as it has at most 15 significant digits.
pub fn restore_number(masked: f64, divide: bool, factor: f64) -> String {
    let cand = if divide { masked * factor } else { masked / factor };
    if cand == 0.0 || !cand.is_finite() {
        return "0".into();
    }
    let target = fmt_sig(masked, 8);
    for k in 1..=15usize {
        let r = round_sig(cand, k);
        let back = if divide { r / factor } else { r * factor };
        if fmt_sig(back, 8) == target {
            return fmt_sig(r, k as i32);
        }
    }
    fmt_sig(cand, 10)
}

/// Approximate restore with a base factor (level 4 without per-value factors).
fn restore_approx(masked: f64, divide: bool, factor: f64) -> String {
    let cand = if divide { masked * factor } else { masked / factor };
    fmt_sig(cand, 6)
}

/// Looks rows of the masked dataset up in keymap_columns.txt by key and masked numbers.
struct Matcher {
    full: HashMap<String, VecDeque<usize>>,
    by_key: HashMap<String, VecDeque<usize>>,
    used: Vec<bool>,
}

fn take(map: &mut HashMap<String, VecDeque<usize>>, key: &str, used: &mut [bool]) -> Option<usize> {
    let q = map.get_mut(key)?;
    while let Some(i) = q.pop_front() {
        if !used[i] {
            used[i] = true;
            return Some(i);
        }
    }
    None
}

impl Matcher {
    fn new(pv: &PerValueFile) -> Self {
        let mut full: HashMap<String, VecDeque<usize>> = HashMap::new();
        let mut by_key: HashMap<String, VecDeque<usize>> = HashMap::new();
        for (i, r) in pv.rows.iter().enumerate() {
            let key = r.key.iter().map(|k| canon(k)).collect::<Vec<_>>().join("\u{1}");
            let all = r.cells.iter().map(|c| canon(&c.0)).collect::<Vec<_>>().join("\u{1}");
            full.entry(format!("{key}\u{2}{all}")).or_default().push_back(i);
            by_key.entry(key).or_default().push_back(i);
        }
        Self { full, by_key, used: vec![false; pv.rows.len()] }
    }
}

pub fn check_output_dir(dir: &Path, overwrite: bool) -> Result<usize> {
    if !dir.exists() {
        return Ok(0);
    }
    if !dir.is_dir() {
        bail!("the output path {} exists and is not a folder", dir.display());
    }
    let existing = fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("cannot read output folder {}: {e}", dir.display()))?
        .count();
    if existing > 0 && !overwrite {
        bail!(
            "output folder {} already contains {} file(s) or folder(s). Nothing was written. Choose another --output folder, or add --overwrite to replace the files this program writes there",
            dir.display(),
            existing
        );
    }
    Ok(existing)
}

// ----------------------------------------------------------------------------------- reverse

#[derive(Clone, Debug)]
pub struct ReverseArtifacts {
    pub dataset_path: PathBuf,
    pub report_path: PathBuf,
    pub log_path: PathBuf,
    pub dataset: Dataset,
    pub warnings: usize,
    pub restored_sha256: String,
}

struct Notes<'a> {
    log: &'a TraceLog,
    strict: bool,
    lines: Vec<String>,
    warnings: Vec<String>,
}

impl Notes<'_> {
    fn info(&mut self, m: impl Into<String>) {
        let m = m.into();
        self.log.info(&m);
        self.lines.push(m);
    }
    fn warn(&mut self, m: impl Into<String>) {
        let m = m.into();
        self.log.warn(&m);
        self.warnings.push(m);
    }
    /// A problem that `--strict` turns into an error.
    fn problem(&mut self, m: impl Into<String>) -> Result<()> {
        let m = m.into();
        if self.strict {
            bail!("{m} (--strict: stopping instead of approximating or skipping)");
        }
        self.warn(m);
        Ok(())
    }
}

pub fn reverse(opts: &ReverseOptions, log: &mut TraceLog) -> Result<ReverseArtifacts> {
    let log: &TraceLog = log;
    let mut nt = Notes { log, strict: opts.strict, lines: Vec::new(), warnings: Vec::new() };
    let on = |s: Step| opts.steps.contains(&s);

    log.info(format!("reverse start input={} output={}", opts.input.display(), opts.output_dir.display()));
    fs::create_dir_all(&opts.output_dir).with_context(|| format!("creating output dir {}", opts.output_dir.display()))?;

    // ---- the keys
    let keymap_path = match &opts.keymap {
        Some(p) => p.clone(),
        None => {
            let dir = opts.input.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
            let k1 = dir.join("keymap.txt");
            let k2 = dir.join("keymap2.txt");
            if !k1.is_file() && k2.is_file() { k2 } else { k1 }
        }
    };
    // A key written by `run2` (several ADaM datasets masked together) is handled by the run2 module.
    if keymap_path.is_file() {
        if let Ok(text) = fs::read_to_string(&keymap_path) {
            if crate::run2::keymap2::is_keymap2(&text) {
                return crate::run2::reverse2::reverse2(opts, log, &keymap_path, &text);
            }
        }
    }
    if !keymap_path.is_file() {
        bail!(
            "keymap.txt not found at {}. Give it with --keymap FILE (it is written next to the masked dataset by `run`; it is the file that must never be shared)",
            keymap_path.display()
        );
    }
    let km_text = fs::read_to_string(&keymap_path).with_context(|| format!("reading {}", keymap_path.display()))?;
    let km = parse_keymap(&km_text).with_context(|| format!("reading {}", keymap_path.display()))?;
    log.success(format!(
        "read key {} (program {}, level {}, {} recoded column(s), {} scaled column(s), {} renamed column(s))",
        keymap_path.display(),
        km.program.as_deref().unwrap_or("unknown"),
        km.level.map(|l| l.to_string()).unwrap_or_else(|| "?".into()),
        km.recodes.len(),
        km.scales.len(),
        km.renames.len()
    ));

    let pv_path = match &opts.keymap_columns {
        Some(p) => {
            if !p.is_file() {
                bail!("keymap_columns.txt not found at {}", p.display());
            }
            Some(p.clone())
        }
        None => {
            let p = keymap_path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new(".")).join("keymap_columns.txt");
            p.is_file().then_some(p)
        }
    };
    let pv: Option<PerValueFile> = match &pv_path {
        Some(p) if on(Step::Scaling) => {
            let text = fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
            let f = parse_keymap_columns(&text).with_context(|| format!("reading {}", p.display()))?;
            if let (Some(a), Some(b)) = (&km.masked_sha256, &f.masked_sha256) {
                if a != b {
                    bail!(
                        "{} and {} belong to different runs (their masked-dataset SHA-256 differ). Use the two files that were written together",
                        keymap_path.display(),
                        p.display()
                    );
                }
            }
            log.success(format!("read per-value factors {} ({} rows)", p.display(), f.rows.len()));
            Some(f)
        }
        _ => None,
    };
    let any_varies = km.scales.iter().any(|s| s.varies);
    if any_varies && on(Step::Scaling) && pv.is_none() {
        nt.problem(
            "this key was made at level 4 (every value has its own factor) but keymap_columns.txt was not found next to keymap.txt: scaled columns can only be restored APPROXIMATELY (within about 15 %). Give the file with --keymap-columns",
        )?;
    }

    // ---- the masked dataset
    let mut ds = read_dataset(&opts.input, opts.delimiter, opts.no_header, &opts.columns, &opts.comment_chars)?;
    let n_cols_in = ds.n_cols();
    log.success(format!("read {} rows x {} cols ({}) from {}", ds.n_rows(), ds.n_cols(), ds.delimiter.name(), ds.source_name));
    let input_sha = hex::encode(Sha256::digest(&fs::read(&opts.input).with_context(|| format!("reading {}", opts.input.display()))?));
    match &km.masked_sha256 {
        Some(k) if *k == input_sha => nt.info("the input is byte-identical to the masked dataset this key was written for"),
        Some(_) => nt.info("the input differs from the masked dataset this key was written for (re-sorted, re-ordered, edited or extended): rows and columns are found by name and key"),
        None => nt.info("keymap.txt does not record the masked dataset's SHA-256; the input could not be compared"),
    }
    if let Some(name) = &km.masked_name {
        if !ds.source_name.eq_ignore_ascii_case(name) {
            nt.info(format!("the input file is called {} but the key was written for {name}", ds.source_name));
        }
    }

    // ---- per-value matching, on the untouched input (the keys contain the MASKED ID and TIME)
    let n_rows = ds.n_rows();
    let mut matched: Vec<Option<usize>> = vec![None; n_rows];
    let mut value_idx_in_pv: HashMap<String, usize> = HashMap::new();
    if let Some(pvf) = &pv {
        for (i, o) in pvf.value_cols_original.iter().enumerate() {
            value_idx_in_pv.insert(o.to_ascii_uppercase(), i);
        }
        let find = |masked: &str, original: &str| pos_ci(&ds.headers, masked).or_else(|| pos_ci(&ds.headers, original));
        let key_idx: Vec<Option<usize>> =
            pvf.key_cols.iter().zip(&pvf.key_cols_original).map(|(m, o)| find(m, o)).collect();
        let value_idx: Vec<Option<usize>> =
            pvf.value_cols.iter().zip(&pvf.value_cols_original).map(|(m, o)| find(m, o)).collect();
        let missing_keys: Vec<&str> = key_idx.iter().zip(&pvf.key_cols).filter(|(i, _)| i.is_none()).map(|(_, n)| n.as_str()).collect();
        if !missing_keys.is_empty() {
            nt.problem(format!(
                "the key column(s) {} used by keymap_columns.txt are not in the input, so its rows cannot be matched and level 4 columns are restored only approximately",
                missing_keys.join(", ")
            ))?;
        } else {
            let all_values = value_idx.iter().all(|i| i.is_some());
            if !all_values {
                nt.warn("some scaled columns listed in keymap_columns.txt are not in the input: rows are matched by their key columns alone");
            }
            let mut m = Matcher::new(pvf);
            for (ri, row) in ds.rows.iter().enumerate() {
                let cell = |i: Option<usize>| i.and_then(|i| row.get(i)).map(|s| s.as_str()).unwrap_or("");
                let key = key_idx.iter().map(|&i| canon(cell(i))).collect::<Vec<_>>().join("\u{1}");
                let hit = if all_values {
                    let all = value_idx.iter().map(|&i| canon(cell(i))).collect::<Vec<_>>().join("\u{1}");
                    take(&mut m.full, &format!("{key}\u{2}{all}"), &mut m.used)
                } else {
                    None
                }
                .or_else(|| if all_values { None } else { take(&mut m.by_key, &key, &mut m.used) });
                matched[ri] = hit;
            }
            let n_hit = matched.iter().filter(|m| m.is_some()).count();
            nt.info(format!("matched {n_hit} of {n_rows} row(s) to keymap_columns.txt"));
            if n_hit < n_rows {
                nt.problem(format!(
                    "{} row(s) could not be found in keymap_columns.txt (new rows, or ID/TIME/scaled values that were edited): their level 4 columns are restored only approximately",
                    n_rows - n_hit
                ))?;
            }
        }
    }

    // Columns of the source that were never in the masked dataset (dropped, or not listed in keep): nothing to undo.
    let known_cols: HashSet<String> = km.column_order.iter().map(|(o, _)| o.to_ascii_uppercase()).collect();
    let never_there = |c: &str| !known_cols.is_empty() && !known_cols.contains(&c.to_ascii_uppercase());

    // ---- scaling
    let mut report_steps: Vec<String> = Vec::new();
    if on(Step::Scaling) && !km.scales.is_empty() {
        for sc in &km.scales {
            let Some(ci) = locate(&ds.headers, &km.renames, &sc.column) else {
                if never_there(&sc.column) {
                    log.info(format!("scaled column {} was not in the masked dataset (dropped or not kept): nothing to undo", sc.column));
                } else {
                    nt.problem(format!("scaled column {} is not in the input: skipped", sc.column))?;
                }
                continue;
            };
            let pvi = value_idx_in_pv.get(&sc.column.to_ascii_uppercase()).copied();
            let (mut exact, mut approx) = (0usize, 0usize);
            for (ri, row) in ds.rows.iter_mut().enumerate() {
                let Some(cell) = row.get_mut(ci) else { continue };
                let t = cell.trim();
                if t.is_empty() || t == "." {
                    continue;
                }
                let Ok(v) = t.parse::<f64>() else { continue };
                if !v.is_finite() {
                    continue;
                }
                if !sc.varies {
                    *cell = restore_number(v, sc.divide, sc.factor);
                    exact += 1;
                    continue;
                }
                let factor = match (&pv, pvi, matched[ri]) {
                    (Some(pvf), Some(vi), Some(m)) => pvf.rows[m].cells[vi].1,
                    _ => None,
                };
                match factor {
                    Some(f) => {
                        *cell = restore_number(v, sc.divide, f);
                        exact += 1;
                    }
                    None => {
                        *cell = restore_approx(v, sc.divide, sc.factor);
                        approx += 1;
                    }
                }
            }
            if approx > 0 {
                nt.problem(format!(
                    "column {}: {approx} value(s) were restored only APPROXIMATELY (the base factor was used; each is within about 15 % of the original)",
                    sc.column
                ))?;
            }
            let line = format!(
                "scaling: {} {} {} value(s) exactly{}",
                sc.column,
                if sc.varies { "(level 4, per-value factors)" } else { "(one factor)" },
                exact,
                if approx > 0 { format!(", {approx} approximately") } else { String::new() }
            );
            log.info(&line);
            report_steps.push(line);
        }
    } else if !on(Step::Scaling) {
        report_steps.push("scaling: skipped (--skip / --only)".into());
    }

    // ---- dates
    if on(Step::Dates) {
        if let Some(days) = km.date_shift.filter(|d| *d != 0.0) {
            let whole = km.date_shift_whole.unwrap_or(days.floor() as i64);
            for dc in &km.date_columns {
                let Some(ci) = locate(&ds.headers, &km.renames, dc) else {
                    log.info(format!("date column {dc} is not in the input: skipped"));
                    continue;
                };
                let (mut done, mut unreadable) = (0usize, 0usize);
                for row in ds.rows.iter_mut() {
                    let Some(cell) = row.get_mut(ci) else { continue };
                    let t = cell.trim();
                    if t.is_empty() || t == "." {
                        continue;
                    }
                    let (v, ok) = unshift_date_cell(cell, days, whole);
                    if ok {
                        *cell = v;
                        done += 1;
                    } else {
                        unreadable += 1;
                    }
                }
                let line = format!("dates: {dc} moved back by {} day(s): {done} value(s){}", crate::obfuscate::fmt_days(days), if unreadable > 0 { format!(", {unreadable} not in a recognised date format and left as they are (the run left them unshifted too)") } else { String::new() });
                log.info(&line);
                report_steps.push(line);
            }
            if km.date_columns.is_empty() {
                report_steps.push("dates: the key lists no date columns".into());
            }
        } else {
            report_steps.push("dates: nothing to undo (no date shift in the key)".into());
        }
    } else {
        report_steps.push("dates: skipped (--skip / --only)".into());
    }

    // ---- identifiers and labels
    if on(Step::Recoding) {
        for (col, map) in &km.recodes {
            let Some(ci) = locate(&ds.headers, &km.renames, col) else {
                if never_there(col) {
                    log.info(format!("recoded column {col} was not in the masked dataset (dropped or not kept): nothing to undo"));
                } else {
                    nt.problem(format!("recoded column {col} is not in the input: skipped"))?;
                }
                continue;
            };
            let ambiguous: HashSet<&str> = map.iter().filter(|(_, v)| v.len() > 1).map(|(k, _)| k.as_str()).collect();
            let (mut done, mut unmapped, mut amb) = (0usize, 0usize, 0usize);
            let mut examples: Vec<String> = Vec::new();
            for row in ds.rows.iter_mut() {
                let Some(cell) = row.get_mut(ci) else { continue };
                let t = cell.trim().to_string();
                if t.is_empty() || t == "." {
                    continue;
                }
                match map.get(&t) {
                    Some(v) if v.len() == 1 => {
                        *cell = v[0].clone();
                        done += 1;
                    }
                    Some(_) => amb += 1,
                    None => {
                        unmapped += 1;
                        if examples.len() < 3 && !examples.contains(&t) {
                            examples.push(t);
                        }
                    }
                }
            }
            if amb > 0 {
                nt.problem(format!(
                    "column {col}: {amb} value(s) carry a label that stands for more than one original value ({} such label(s), a hashed ID collision): left unchanged",
                    ambiguous.len()
                ))?;
            }
            if unmapped > 0 {
                nt.problem(format!(
                    "column {col}: {unmapped} value(s) are not in keymap.txt (for example {}): left unchanged",
                    examples.join(", ")
                ))?;
            }
            let line = format!("recoding: {col}: {done} value(s) mapped back to the original");
            log.info(&line);
            report_steps.push(line);
        }
    } else {
        report_steps.push("recoding: skipped (--skip / --only)".into());
    }

    // ---- column names
    if on(Step::Rename) {
        for (orig, new) in &km.renames {
            match pos_ci(&ds.headers, new) {
                Some(i) => {
                    ds.headers[i] = orig.clone();
                    let line = format!("rename: {new} -> {orig}");
                    log.info(&line);
                    report_steps.push(line);
                }
                None => nt.problem(format!("renamed column {new} (original name {orig}) is not in the input: skipped"))?,
            }
        }
    } else if !km.renames.is_empty() {
        report_steps.push("rename: skipped (--skip / --only); columns keep their masked names".into());
    }

    // ---- original column order
    if opts.restore_order && !km.column_order.is_empty() {
        let n = ds.headers.len();
        let mut used = vec![false; n];
        let mut order: Vec<usize> = Vec::with_capacity(n);
        for (orig, _) in &km.column_order {
            if let Some(i) = locate(&ds.headers, &km.renames, orig) {
                if !used[i] {
                    used[i] = true;
                    order.push(i);
                }
            }
        }
        for (i, u) in used.iter().enumerate() {
            if !u {
                order.push(i);
            }
        }
        if order.iter().enumerate().any(|(a, &b)| a != b) {
            ds.headers = order.iter().map(|&i| ds.headers[i].clone()).collect();
            for row in ds.rows.iter_mut() {
                *row = order.iter().map(|&i| row.get(i).cloned().unwrap_or_default()).collect();
            }
            report_steps.push("order: columns put back in the order the masked dataset was written in (columns the key does not know follow at the end)".into());
        }
    }

    // ---- original row order (level 4 sorts rows by masked ID)
    if opts.restore_order {
        if pv.is_some() && matched.iter().all(|m| m.is_some()) && n_rows > 0 {
            let pvf = pv.as_ref().unwrap();
            let mut idx: Vec<usize> = (0..n_rows).collect();
            idx.sort_by_key(|&i| pvf.rows[matched[i].unwrap()].orig_row);
            if idx.iter().enumerate().any(|(a, &b)| a != b) {
                let old = std::mem::take(&mut ds.rows);
                ds.rows = idx.into_iter().map(|i| old[i].clone()).collect();
                report_steps.push("order: rows put back in the order of the source dataset (from keymap_columns.txt)".into());
            }
        } else if km.level == Some(4) {
            nt.warn("level 4 sorts rows by masked ID and the source row order is not recorded without a complete keymap_columns.txt match: rows keep the order of the input");
        }
    }

    // ---- what cannot come back
    if !km.not_restorable.is_empty() {
        let list = km.not_restorable.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>().join(", ");
        nt.warn(format!(
            "these source columns cannot be restored because their values were never stored: {list} (free text is redacted to '.', and dropped columns are not in the masked file). See keymap.txt section 5"
        ));
    }

    // ---- write
    let out_name = opts.output_name.clone().unwrap_or_else(|| match ds.delimiter {
        crate::dataset::Delimiter::Tab => "restored.tsv".into(),
        crate::dataset::Delimiter::Whitespace => "restored.dat".into(),
        _ => "restored.csv".into(),
    });
    let dataset_path = opts.output_dir.join(&out_name);
    ds.comments = Vec::new();
    write_dataset(&dataset_path, &ds)?;
    let restored_sha256 = hex::encode(Sha256::digest(&fs::read(&dataset_path)?));
    log.success(format!("wrote restored dataset {} ({} rows x {} cols), SHA-256 {restored_sha256}", dataset_path.display(), ds.n_rows(), ds.n_cols()));

    let report_path = opts.output_dir.join("reverse_report.txt");
    let mut rep = String::new();
    rep.push_str("DATA OBFUSCATOR - REVERSE REPORT\n================================\n");
    rep.push_str(&format!("Generated:        {}\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));
    rep.push_str(&format!("Program:          data-obfuscator {} (released {})\n", crate::APP_VERSION, crate::APP_RELEASED));
    rep.push_str(&format!("Masked input:     {} ({} rows x {} cols)\n", opts.input.display(), n_rows, n_cols_in));
    rep.push_str(&format!("Key:              {}{}\n", keymap_path.display(), km.program.as_ref().map(|p| format!(" (written by {p})")).unwrap_or_default()));
    rep.push_str(&format!(
        "Per-value file:   {}\n",
        match (&pv_path, pv.is_some()) {
            (Some(p), true) => p.display().to_string(),
            (Some(p), false) => format!("{} (not used: scaling was skipped)", p.display()),
            (None, _) => "none".into(),
        }
    ));
    rep.push_str(&format!("Steps undone:     {}\n", opts.steps.iter().map(|s| s.name()).collect::<Vec<_>>().join(", ")));
    rep.push_str(&format!("Restored dataset: {} ({} rows x {} cols)\n", dataset_path.display(), ds.n_rows(), ds.n_cols()));
    rep.push_str(&format!("Restored SHA-256: {restored_sha256}\n\n"));
    rep.push_str("WHAT WAS DONE\n");
    for l in nt.lines.iter().chain(report_steps.iter()) {
        rep.push_str(&format!("  - {l}\n"));
    }
    rep.push_str("\nWARNINGS\n");
    if nt.warnings.is_empty() {
        rep.push_str("  none\n");
    }
    for w in &nt.warnings {
        rep.push_str(&format!("  - {w}\n"));
    }
    rep.push_str("\nNOTES\n");
    rep.push_str("  - Numbers are written in their shortest form (70.50 comes back as 70.5).\n");
    rep.push_str("  - Dates written as DD-MMM-YYYY or YYYY/MM/DD were turned into YYYY-MM-DD by the masking and stay in that form.\n");
    rep.push_str("  - Rows are in the order of the input file, except that level 4 row order is restored when keymap_columns.txt matches every row.\n");
    rep.push_str("  - This report and the restored dataset contain real data: do not share them.\n");
    fs::write(&report_path, rep)?;
    log.success(format!("wrote {}", report_path.display()));
    log.success("reverse complete SUCCESS");

    Ok(ReverseArtifacts {
        dataset_path,
        report_path,
        log_path: log.path().to_path_buf(),
        dataset: ds,
        warnings: log.warn_count(),
        restored_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_number_returns_the_shortest_original() {
        for (orig, divide, f) in [(65.4_f64, true, 8.7937773185_f64), (70.5, false, 3.3), (0.123456789, true, 12.5), (1234.5, false, 1.07)] {
            let nv = if divide { orig / f } else { orig * f };
            let masked: f64 = fmt_sig(nv, 8).parse().unwrap();
            let back = restore_number(masked, divide, f);
            assert_eq!(back.parse::<f64>().unwrap(), orig, "{orig} via {masked} -> {back}");
        }
        assert_eq!(restore_number(0.0, true, 3.0), "0");
    }

    #[test]
    fn step_names_parse() {
        assert_eq!(Step::parse("IDs").unwrap(), Step::Recoding);
        assert_eq!(Step::parse("scale").unwrap(), Step::Scaling);
        assert!(Step::parse("x").is_err());
    }

    #[test]
    fn keymap_columns_lines_are_checked() {
        let ok = "@key_columns\tID\tTIME\n@value_columns\tWT\n1\t1\t0\t7.4\t8.8\n";
        let pv = parse_keymap_columns(ok).unwrap();
        assert_eq!(pv.rows.len(), 1);
        assert_eq!(pv.rows[0].cells[0].1, Some(8.8));
        assert!(parse_keymap_columns("@key_columns\tID\n@value_columns\tWT\n1\t1\t7.4\n").is_err());
    }
}
