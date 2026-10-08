//! CLI / config-file options. File values are the base; flags override.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdStyle {
    Sequential,
    Padded,
    Hashed,
}

impl IdStyle {
    pub fn parse(s: &str) -> Result<Self> {
        match norm_key(s).as_str() {
            "sequential" | "seq" | "numeric" => Ok(Self::Sequential),
            "padded" | "prefix" | "labelled" | "labeled" => Ok(Self::Padded),
            "hashed" | "hash" | "sha" => Ok(Self::Hashed),
            other => bail!("unknown id_style '{other}' (sequential|padded|hashed)"),
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Padded => "padded",
            Self::Hashed => "hashed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub level: u8,
    pub keep: Option<Vec<String>>,
    pub drop: Vec<String>,
    pub id_column: Vec<String>,
    pub subject_column: Vec<String>,
    pub study_column: Vec<String>,
    pub site_column: Vec<String>,
    pub treatment_column: Vec<String>,
    pub user_column: Vec<String>,
    pub date_columns: Vec<String>,
    pub sensitive: Vec<String>,
    pub protect: Vec<String>,
    /// Extra numeric columns to multiply/divide by a hidden factor (level 3+).
    pub scale_columns: Vec<String>,
    pub freetext_columns: Vec<String>,
    pub seed: u64,
    pub seed_was_generated: bool,
    pub delimiter: Option<char>,
    pub no_header: bool,
    pub columns: Vec<String>,
    pub id_style: IdStyle,
    /// Range of the random scaling factor (default 1 to 20).
    pub scale_min: f64,
    pub scale_max: f64,
    pub date_shift_min: f64,
    pub date_shift_max: f64,
    /// Columns to rename in the masked file: `NAME` (automatic new name) or `NAME=NEW`.
    pub rename_columns: Vec<String>,
    /// Alternative to `rename_columns`: rename every output column EXCEPT these.
    pub rename_except: Vec<String>,
    /// Prefix of automatic new names (`COL` gives COL01, COL02, ... by column position).
    pub rename_prefix: String,
    pub write_keymap: bool,
    /// Allow writing into an output folder that already contains files.
    pub overwrite: bool,
    pub output_name: Option<String>,
    pub comment_chars: String,
    pub drop_freetext: bool,
    pub verbose: bool,
    pub config_path: Option<PathBuf>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            input: PathBuf::new(),
            output_dir: PathBuf::new(),
            level: 2,
            keep: None,
            drop: Vec::new(),
            id_column: Vec::new(),
            subject_column: Vec::new(),
            study_column: Vec::new(),
            site_column: Vec::new(),
            treatment_column: Vec::new(),
            user_column: Vec::new(),
            date_columns: Vec::new(),
            sensitive: Vec::new(),
            protect: default_protect(),
            scale_columns: Vec::new(),
            freetext_columns: Vec::new(),
            seed: 42,
            seed_was_generated: false,
            delimiter: None,
            no_header: false,
            columns: Vec::new(),
            id_style: IdStyle::Sequential,
            scale_min: 1.0,
            scale_max: 20.0,
            date_shift_min: 1.0,
            date_shift_max: 100.0,
            rename_columns: Vec::new(),
            rename_except: Vec::new(),
            rename_prefix: "COL".into(),
            write_keymap: true,
            overwrite: false,
            output_name: None,
            comment_chars: "#;$".into(),
            drop_freetext: false,
            verbose: false,
            config_path: None,
        }
    }
}

impl RunOptions {
    /// Checks values that cannot be validated while parsing one key at a time.
    pub fn validate(&self) -> Result<()> {
        if !(1..=4).contains(&self.level) {
            bail!("level must be 1-4");
        }
        if !(self.scale_min.is_finite() && self.scale_max.is_finite())
            || self.scale_min <= 0.0
            || self.scale_max < self.scale_min
        {
            bail!(
                "scale_range must be MIN-MAX with 0 < MIN <= MAX (got {}-{})",
                self.scale_min,
                self.scale_max
            );
        }
        if !(self.date_shift_min.is_finite() && self.date_shift_max.is_finite()) {
            bail!("date_shift_days must be a number or MIN-MAX");
        }
        crate::rename::validate_options(self)?;
        Ok(())
    }

    pub fn level_name(&self) -> &'static str {
        match self.level {
            1 => "light (identifiers)",
            2 => "clinical labels",
            3 => "covariate masking",
            4 => "maximum",
            _ => "custom",
        }
    }

    pub fn dump(&self) -> String {
        let keep = self
            .keep
            .as_ref()
            .map(|v| v.join(","))
            .unwrap_or_else(|| "(all)".into());
        format!(
            "# resolved data-obfuscator options (CLI overrides config file)\n\
             input = {}\n\
             output = {}\n\
             level = {}\n\
             keep = {}\n\
             drop = {}\n\
             id_column = {}\n\
             subject_column = {}\n\
             study_column = {}\n\
             site_column = {}\n\
             treatment_column = {}\n\
             user_column = {}\n\
             date_columns = {}\n\
             sensitive = {}\n\
             protect = {}\n\
             scale_columns = {}\n\
             freetext_columns = {}\n\
             seed = {}\n\
             delimiter = {}\n\
             no_header = {}\n\
             columns = {}\n\
             id_style = {}\n\
             scale_range = {}-{}\n\
             date_shift_min = {}\n\
             date_shift_max = {}\n\
             rename_columns = {}\n\
             rename_except = {}\n\
             rename_prefix = {}\n\
             write_keymap = {}\n\
             overwrite = {}\n\
             output_name = {}\n\
             comment_chars = {}\n\
             drop_freetext = {}\n\
             verbose = {}\n\
             config = {}\n",
            self.input.display(),
            self.output_dir.display(),
            self.level,
            keep,
            self.drop.join(","),
            disp(&self.id_column),
            disp(&self.subject_column),
            disp(&self.study_column),
            disp(&self.site_column),
            disp(&self.treatment_column),
            disp(&self.user_column),
            self.date_columns.join(","),
            self.sensitive.join(","),
            self.protect.join(","),
            self.scale_columns.join(","),
            self.freetext_columns.join(","),
            self.seed,
            self.delimiter
                .map(|c| format!("{c}"))
                .unwrap_or_else(|| "auto".into()),
            self.no_header,
            self.columns.join(","),
            self.id_style.as_str(),
            self.scale_min,
            self.scale_max,
            self.date_shift_min,
            self.date_shift_max,
            self.rename_columns.join(","),
            self.rename_except.join(","),
            self.rename_prefix,
            self.write_keymap,
            self.overwrite,
            self.output_name.clone().unwrap_or_else(|| "(auto)".into()),
            self.comment_chars,
            self.drop_freetext || self.level >= 4,
            self.verbose,
            self.config_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(none)".into()),
        )
    }
}

fn disp(v: &[String]) -> String {
    if v.is_empty() {
        "(auto)".into()
    } else {
        v.join(",")
    }
}

pub fn default_protect() -> Vec<String> {
    [
        "TIME", "TAD", "DV", "LNDV", "AMT", "RATE", "EVID", "MDV", "CMT", "ADDL", "II", "SS", "DVID",
        "CENS", "LIMIT", "BLQ", "L2", "OCC", "ADDL", "PC", "PCYC",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect()
}

#[derive(Clone, Debug, Default)]
pub struct TokensOptions {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub exclude: Vec<String>,
}

pub fn parse_config_file(path: &Path) -> Result<HashMap<String, String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading config {}", path.display()))?;
    parse_config_text(&text)
}

/// Parse a `.txt` options file: `key = value`, `key: value`, or `--key value`.
/// `#` and `//` start comments. Keys are case-insensitive; `-` and `_` equivalent.
pub fn parse_config_text(text: &str) -> Result<HashMap<String, String>> {
    // Editors such as Windows Notepad may add a UTF-8 byte-order mark; skip it so a
    // commented header at the top of the file is still recognised as comments.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut map: HashMap<String, String> = HashMap::new();
    for (lineno, raw) in text.lines().enumerate() {
        let mut line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        if let Some(i) = line.find('#') {
            // keep # inside quotes
            if !line[..i].contains('"') && !line[..i].contains('\'') {
                line = line[..i].trim();
            }
        }
        if line.is_empty() {
            continue;
        }
        let (k, v) = if let Some(rest) = line.strip_prefix("--") {
            let rest = rest.trim();
            if let Some((a, b)) = rest.split_once('=') {
                (a.trim(), b.trim())
            } else if let Some((a, b)) = rest.split_once(char::is_whitespace) {
                (a.trim(), b.trim())
            } else {
                (rest, "true")
            }
        } else if let Some((a, b)) = line.split_once('=') {
            (a.trim(), b.trim())
        } else if let Some((a, b)) = line.split_once(':') {
            (a.trim(), b.trim())
        } else {
            bail!("config line {}: expected key = value, got: {raw}", lineno + 1);
        };
        let key = norm_key(k);
        let mut val = v.trim().to_string();
        if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
            || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
        {
            val = val[1..val.len() - 1].to_string();
        }
        if is_role_key(&key) {
            if let Some(prev) = map.get_mut(&key) {
                // `study_column = STU` followed by `study_column = STUDYID` means both.
                prev.push(',');
                prev.push_str(&val);
                continue;
            }
        }
        map.insert(key, val);
    }
    Ok(map)
}

pub fn norm_key(s: &str) -> String {
    s.trim()
        .trim_start_matches('-')
        .to_ascii_lowercase()
        .replace('-', "_")
}

pub fn parse_level(s: &str) -> Result<u8> {
    match norm_key(s).as_str() {
        "1" | "light" | "identifiers" => Ok(1),
        "2" | "clinical" | "medium" | "default" => Ok(2),
        "3" | "covariate" | "covariates" | "heavy" => Ok(3),
        "4" | "maximum" | "max" | "full" => Ok(4),
        other => bail!("unknown level '{other}' (1-4 or light|clinical|covariate|maximum)"),
    }
}

pub fn split_list(s: &str) -> Vec<String> {
    s.split(|c| c == ',' || c == ';' || c == '|')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

/// Append the names in `v` to `list`, skipping duplicates (case-insensitive), keeping order.
pub fn add_cols(list: &mut Vec<String>, v: &str) {
    for name in split_list(v) {
        if !list.iter().any(|x| x.eq_ignore_ascii_case(&name)) {
            list.push(name);
        }
    }
}

/// Config keys (canonical or alias) that name a role column; repeated lines add up.
pub fn is_role_key(key: &str) -> bool {
    matches!(
        key,
        "id_column" | "id" | "subject_column" | "usubjid" | "subjid" | "study_column" | "study"
            | "site_column" | "site" | "treatment_column" | "arm" | "trt" | "user_column"
            | "userid" | "investigator"
    )
}

/// A finite decimal number of days (negative allowed).
fn parse_days(v: &str, key: &str) -> Result<f64> {
    let x: f64 = v.trim().parse().with_context(|| format!("{key} '{v}' is not a number"))?;
    if !x.is_finite() {
        bail!("{key} '{v}' is not a finite number");
    }
    Ok(x)
}

/// `MIN-MAX` or a single number, decimals and negative numbers allowed (`1-100`, `23.6`, `-30--5`).
pub fn parse_f64_range(v: &str) -> Result<(f64, f64)> {
    let t = v.trim();
    if let Ok(x) = t.parse::<f64>() {
        if x.is_finite() {
            return Ok((x, x));
        }
    }
    let b = t.as_bytes();
    for i in 1..b.len() {
        if b[i] == b'-' && !matches!(b[i - 1], b'e' | b'E') {
            if let (Ok(lo), Ok(hi)) = (t[..i].trim().parse::<f64>(), t[i + 1..].trim().parse::<f64>()) {
                if lo.is_finite() && hi.is_finite() {
                    return Ok((lo, hi));
                }
            }
        }
    }
    bail!("expected a number or MIN-MAX (for example 23.6 or 1-100)")
}

pub fn parse_bool(s: &str) -> Result<bool> {
    match norm_key(s).as_str() {
        "1" | "true" | "yes" | "on" | "y" => Ok(true),
        "0" | "false" | "no" | "off" | "n" => Ok(false),
        other => bail!("expected boolean, got '{other}'"),
    }
}

pub fn apply_map(opts: &mut RunOptions, map: &HashMap<String, String>) -> Result<()> {
    for (k, v) in map {
        match k.as_str() {
            "input" | "infile" | "data" | "file" => opts.input = PathBuf::from(v),
            "output" | "outdir" | "output_dir" | "output_folder" => {
                opts.output_dir = PathBuf::from(v)
            }
            "level" | "obfuscation" | "obfuscation_level" => opts.level = parse_level(v)?,
            "keep" | "keep_columns" | "columns_keep" => opts.keep = Some(split_list(v)),
            "drop" | "drop_columns" => opts.drop = split_list(v),
            // Role columns take one OR SEVERAL names (e.g. study_column = STU, STUDYID, STUDY).
            // Aliases and repeated keys add to the list instead of replacing it.
            "id_column" | "id" => add_cols(&mut opts.id_column, v),
            "subject_column" | "usubjid" | "subjid" => add_cols(&mut opts.subject_column, v),
            "study_column" | "study" => add_cols(&mut opts.study_column, v),
            "site_column" | "site" => add_cols(&mut opts.site_column, v),
            "treatment_column" | "arm" | "trt" => add_cols(&mut opts.treatment_column, v),
            "user_column" | "userid" | "investigator" => add_cols(&mut opts.user_column, v),
            "date_columns" | "date_column" | "dates" => opts.date_columns = split_list(v),
            "sensitive" | "sensitive_columns" => opts.sensitive = split_list(v),
            "protect" | "protect_columns" => opts.protect = split_list(v),
            "scale_columns" | "jitter_columns" => opts.scale_columns = split_list(v),
            "freetext_columns" | "freetext" => opts.freetext_columns = split_list(v),
            "seed" => {
                if norm_key(v) == "auto" {
                    opts.seed = generate_seed();
                    opts.seed_was_generated = true;
                } else {
                    opts.seed = v.parse().with_context(|| format!("seed '{v}'"))?;
                    opts.seed_was_generated = false;
                }
            }
            "delimiter" => {
                opts.delimiter = match norm_key(v).as_str() {
                    "auto" | "" => None,
                    "comma" | "csv" | "," => Some(','),
                    "tab" | "tsv" | "\\t" => Some('\t'),
                    "space" | "whitespace" | "nm" | "nonmem" => Some(' '),
                    "semicolon" | ";" => Some(';'),
                    other if other.chars().count() == 1 => other.chars().next(),
                    other => bail!("unknown delimiter '{other}'"),
                }
            }
            "no_header" => opts.no_header = parse_bool(v)?,
            "columns" | "header" => opts.columns = split_list(v),
            "id_style" => opts.id_style = IdStyle::parse(v)?,
            "jitter" | "jitter_pct" | "jitter_percent" => bail!(
                "'{k}' was replaced in 0.0.0.4: covariates are now multiplied or divided by a random factor per column. Use scale_range = MIN-MAX (default 1-20) instead"
            ),
            "scale_range" | "scale_factor" => {
                let (a, b) = parse_f64_range(v).with_context(|| format!("scale_range '{v}'"))?;
                opts.scale_min = a;
                opts.scale_max = b;
            }
            "date_shift_min" => opts.date_shift_min = parse_days(v, k)?,
            "date_shift_max" => opts.date_shift_max = parse_days(v, k)?,
            "date_shift_days" | "date_shift" => {
                let (a, b) = parse_f64_range(v).with_context(|| format!("{k} '{v}'"))?;
                opts.date_shift_min = a;
                opts.date_shift_max = b;
            }
            "rename_columns" | "rename" | "rename_column" => opts.rename_columns = split_list(v),
            "rename_except" | "rename_all_except" | "do_not_rename" | "no_rename" | "rename_not" => {
                opts.rename_except = split_list(v)
            }
            "rename_prefix" | "rename_prefix_name" => opts.rename_prefix = v.trim().to_string(),
            "write_keymap" | "keymap" => opts.write_keymap = parse_bool(v)?,
            "overwrite" => opts.overwrite = parse_bool(v)?,
            "output_name" | "outfile" => opts.output_name = Some(v.clone()),
            "comment_chars" | "comments" => opts.comment_chars = v.clone(),
            "drop_freetext" => opts.drop_freetext = parse_bool(v)?,
            "verbose" => opts.verbose = parse_bool(v)?,
            "config" => opts.config_path = Some(PathBuf::from(v)),
            other => bail!("unknown config key '{other}'"),
        }
    }
    Ok(())
}

pub fn generate_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA24B_AED4_96E9_C13F
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_block_and_bom_are_ignored() {
        let text = "\u{feff}# ====================\n#  DATA OBFUSCATOR - config\n#  Author  : Someone\n\
                    #  Purpose : key = value inside a comment must be ignored\n# ====================\n\
                    // another comment style\n\ninput = pk.csv\noutput = ./masked\n";
        let m = parse_config_text(text).unwrap();
        assert_eq!(m.len(), 2, "only real options should be parsed: {m:?}");
        assert_eq!(m.get("input").unwrap(), "pk.csv");
        assert_eq!(m.get("output").unwrap(), "./masked");
    }

    #[test]
    fn role_columns_accept_several_names() {
        let m = parse_config_text(
            "study_column = STU, STUDYID | STUDY\nstudy_column = PROTO\nsite = SITEID; CENTER\nid_column = ID\n",
        )
        .unwrap();
        let mut o = RunOptions::default();
        apply_map(&mut o, &m).unwrap();
        assert_eq!(o.study_column, ["STU", "STUDYID", "STUDY", "PROTO"]);
        assert_eq!(o.site_column, ["SITEID", "CENTER"]);
        assert_eq!(o.id_column, ["ID"]);
        assert!(o.subject_column.is_empty());
    }

    #[test]
    fn role_alias_and_canonical_keys_combine_without_duplicates() {
        let m = parse_config_text("study = STUDYID\nstudy_column = studyid, STU\n").unwrap();
        // both keys survive in the map; applying them adds to one list
        let mut o = RunOptions::default();
        apply_map(&mut o, &m).unwrap();
        let mut got = o.study_column.clone();
        got.sort();
        assert_eq!(got.len(), 2, "{got:?}");
    }

    #[test]
    fn parses_txt_options() {
        let m = parse_config_text(
            "# veil config\ninput = pk.csv\nlevel: 2\nkeep = ID, TIME, DV\n--seed 7\n",
        )
        .unwrap();
        assert_eq!(m.get("input").unwrap(), "pk.csv");
        assert_eq!(m.get("level").unwrap(), "2");
        assert_eq!(m.get("keep").unwrap(), "ID, TIME, DV");
        assert_eq!(m.get("seed").unwrap(), "7");
    }
}
