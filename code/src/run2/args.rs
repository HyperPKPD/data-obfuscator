//! Command-line arguments and options of `run2`. Options are those of `run` where they make sense for
//! several datasets; the shared parsing (config file, role columns, ranges) is `run`'s own.

use crate::config::{
    add_cols, apply_map, parse_bool, parse_config_text, parse_level, split_list, IdStyle, RunOptions,
};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(clap::Args, Debug, Clone, Default)]
pub struct Run2Args {
    /// A dataset to mask (CSV, TSV, semicolon or whitespace delimited). Repeat the flag, give a comma-separated
    /// list, or give a folder (every .csv .tsv .txt .dat .tab file in it)
    #[arg(short, long, value_name = "FILE", action = clap::ArgAction::Append)]
    pub input: Vec<String>,

    /// Output folder (created if missing): the masked datasets, keymap2.txt, llm2.txt, summary2.txt, run2.log
    #[arg(short, long, value_name = "DIR")]
    pub output: Option<PathBuf>,

    /// Options file (key = value; `input` / `inputs` may be repeated). CLI flags override the file
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Obfuscation level: 1 light (identifiers), 2 clinical (default: also labels, dates, free text), 3 covariate
    /// (also scales covariates). Level 4 is not available in run2
    #[arg(short, long, value_name = "LEVEL")]
    pub level: Option<String>,

    /// Columns to drop from every dataset, comma-separated
    #[arg(long, value_name = "COLS")]
    pub drop: Option<String>,

    /// Subject ID column(s) (NONMEM ID). Several allowed
    #[arg(long, value_name = "COLS", action = clap::ArgAction::Append)]
    pub id_column: Vec<String>,

    /// Unique subject identifier column(s) (USUBJID / SUBJID). Several allowed
    #[arg(long, value_name = "COLS", action = clap::ArgAction::Append)]
    pub subject_column: Vec<String>,

    /// Study column(s), e.g. STUDYID. Several allowed
    #[arg(long, value_name = "COLS", action = clap::ArgAction::Append)]
    pub study_column: Vec<String>,

    /// Site / centre column(s). Several allowed
    #[arg(long, value_name = "COLS", action = clap::ArgAction::Append)]
    pub site_column: Vec<String>,

    /// Treatment / ARM column(s). Several allowed
    #[arg(long, value_name = "COLS", action = clap::ArgAction::Append)]
    pub treatment_column: Vec<String>,

    /// User / investigator column(s). Several allowed
    #[arg(long, value_name = "COLS", action = clap::ArgAction::Append)]
    pub user_column: Vec<String>,

    /// Date columns, comma-separated (ADaM names ending in DT, DTM, DTC are found automatically)
    #[arg(long, value_name = "COLS")]
    pub date_columns: Option<String>,

    /// Extra identifier columns to recode
    #[arg(long, value_name = "COLS")]
    pub sensitive: Option<String>,

    /// Columns that must never be altered
    #[arg(long, value_name = "COLS")]
    pub protect: Option<String>,

    /// Numeric columns to scale (multiply or divide by a hidden factor, the same factor in every dataset) at level 3
    #[arg(long, value_name = "COLS")]
    pub scale_columns: Option<String>,

    /// Free-text columns to redact at level >= 2
    #[arg(long, value_name = "COLS")]
    pub freetext_columns: Option<String>,

    /// RNG seed (default 42). Same inputs + options + seed = identical output every run
    #[arg(long)]
    pub seed: Option<u64>,

    /// Field delimiter of the inputs: auto, comma, tab, space, semicolon
    #[arg(long)]
    pub delimiter: Option<String>,

    /// ID recoding style: sequential (default), padded, hashed
    #[arg(long, value_name = "STYLE")]
    pub id_style: Option<String>,

    /// Range of the random factor of scaled columns (level 3), default 1-20
    #[arg(long, value_name = "MIN-MAX")]
    pub scale_range: Option<String>,

    /// Date shift in days: a MIN-MAX window to draw from (default 1-100) or one fixed number. Same for all datasets
    #[arg(long, alias = "date-shift", value_name = "MIN-MAX|DAYS", allow_hyphen_values = true)]
    pub date_shift_days: Option<String>,

    /// Recognise CDISC ADaM column names (ADT, ASTDT, TRT01P, ...) as dates and treatments. On by default
    #[arg(long, value_name = "BOOL", num_args = 0..=1, default_missing_value = "true")]
    pub adam_hints: Option<String>,

    /// Write keymap2.txt. On by default; without it the masking can never be reversed
    #[arg(long, alias = "keymap", value_name = "BOOL", num_args = 0..=1, default_missing_value = "true")]
    pub write_keymap: Option<String>,

    /// Allow writing into an output folder that already contains files (they may be replaced)
    #[arg(long, default_value_t = false)]
    pub overwrite: bool,

    /// Leading comment characters of the inputs (default #;$)
    #[arg(long)]
    pub comment_chars: Option<String>,

    /// Drop free-text columns instead of redacting them
    #[arg(long, default_value_t = false)]
    pub drop_freetext: bool,

    /// Verbose debug lines in the log
    #[arg(short, long, default_value_t = false)]
    pub verbose: bool,
}

#[derive(Clone, Debug)]
pub struct Run2Options {
    /// Input files in the order they were given (folders already expanded).
    pub inputs: Vec<PathBuf>,
    /// Everything `run` knows: levels, roles, seed, ranges, delimiter ... (its `input` field is unused).
    pub base: RunOptions,
    pub adam_hints: bool,
}

const INPUT_KEYS: [&str; 7] = ["input", "inputs", "infile", "file", "files", "data", "datasets"];

/// Splits the input lines out of an options file: the rest goes to the normal parser.
fn split_inputs(text: &str) -> (Vec<String>, String) {
    let mut inputs = Vec::new();
    let mut rest = String::new();
    for raw in text.strip_prefix('\u{feff}').unwrap_or(text).lines() {
        let line = raw.trim();
        let is_comment = line.is_empty() || line.starts_with('#') || line.starts_with("//");
        let kv = if is_comment {
            None
        } else if let Some(r) = line.strip_prefix("--") {
            let r = r.trim();
            r.split_once('=').or_else(|| r.split_once(char::is_whitespace))
        } else {
            line.split_once('=').or_else(|| line.split_once(':'))
        };
        if let Some((k, v)) = kv {
            let key = crate::config::norm_key(k);
            if INPUT_KEYS.contains(&key.as_str()) {
                let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
                inputs.extend(split_list(v));
                continue;
            }
        }
        rest.push_str(raw);
        rest.push('\n');
    }
    (inputs, rest)
}

/// Files for one `--input` entry: the file itself, or the data files inside a folder.
fn expand(entry: &str) -> Result<Vec<PathBuf>> {
    let p = PathBuf::from(entry.trim());
    if p.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&p)
            .with_context(|| format!("reading folder {}", p.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|f| {
                f.is_file()
                    && f.extension()
                        .map(|x| matches!(x.to_string_lossy().to_ascii_lowercase().as_str(), "csv" | "tsv" | "txt" | "dat" | "tab"))
                        .unwrap_or(false)
            })
            .collect();
        v.sort();
        if v.is_empty() {
            bail!("the folder {} holds no .csv, .tsv, .txt, .dat or .tab files", p.display());
        }
        return Ok(v);
    }
    Ok(vec![p])
}

pub fn resolve_run2(args: &Run2Args) -> Result<Run2Options> {
    let mut base = RunOptions::default();
    let mut raw_inputs: Vec<String> = Vec::new();
    if let Some(cfg) = &args.config {
        let text = std::fs::read_to_string(cfg).with_context(|| format!("reading config {}", cfg.display()))?;
        let (ins, rest) = split_inputs(&text);
        raw_inputs.extend(ins);
        let map = parse_config_text(&rest)?;
        apply_map(&mut base, &map)?;
        base.config_path = Some(cfg.clone());
    }
    if !args.input.is_empty() {
        // inputs on the command line replace the ones in the options file
        raw_inputs.clear();
        for a in &args.input {
            raw_inputs.extend(split_list(a));
        }
    }
    if let Some(p) = &args.output {
        base.output_dir = p.clone();
    }
    if let Some(l) = &args.level {
        base.level = parse_level(l)?;
    }
    if let Some(d) = &args.drop {
        base.drop = split_list(d);
    }
    let mut roles: [(&Vec<String>, &mut Vec<String>); 6] = [
        (&args.id_column, &mut base.id_column),
        (&args.subject_column, &mut base.subject_column),
        (&args.study_column, &mut base.study_column),
        (&args.site_column, &mut base.site_column),
        (&args.treatment_column, &mut base.treatment_column),
        (&args.user_column, &mut base.user_column),
    ];
    for (cli, target) in roles.iter_mut() {
        if !cli.is_empty() {
            target.clear();
            for v in cli.iter() {
                add_cols(target, v);
            }
        }
    }
    if let Some(v) = &args.date_columns {
        base.date_columns = split_list(v);
    }
    if let Some(v) = &args.sensitive {
        base.sensitive = split_list(v);
    }
    if let Some(v) = &args.protect {
        base.protect = split_list(v);
    }
    if let Some(v) = &args.scale_columns {
        base.scale_columns = split_list(v);
    }
    if let Some(v) = &args.freetext_columns {
        base.freetext_columns = split_list(v);
    }
    if let Some(s) = args.seed {
        base.seed = s;
        base.seed_was_generated = false;
    }
    let mut m: HashMap<String, String> = HashMap::new();
    if let Some(d) = &args.delimiter {
        m.insert("delimiter".into(), d.clone());
    }
    if let Some(r) = &args.scale_range {
        m.insert("scale_range".into(), r.clone());
    }
    if let Some(w) = &args.date_shift_days {
        m.insert("date_shift_days".into(), w.clone());
    }
    if !m.is_empty() {
        apply_map(&mut base, &m)?;
    }
    if let Some(s) = &args.id_style {
        base.id_style = IdStyle::parse(s)?;
    }
    if let Some(v) = &args.write_keymap {
        base.write_keymap = parse_bool(v)?;
    }
    if args.overwrite {
        base.overwrite = true;
    }
    if let Some(c) = &args.comment_chars {
        base.comment_chars = c.clone();
    }
    if args.drop_freetext {
        base.drop_freetext = true;
    }
    if args.verbose {
        base.verbose = true;
    }
    let adam_hints = match &args.adam_hints {
        Some(v) => parse_bool(v)?,
        None => true,
    };

    if raw_inputs.is_empty() {
        bail!("--input is required: give each dataset with --input FILE (repeat the flag), a folder, or inputs = ... in the options file");
    }
    if base.output_dir.as_os_str().is_empty() {
        bail!("--output is required (or set output = ... in the options file)");
    }
    if base.level == 4 {
        bail!("level 4 is not available in run2: it re-sorts rows and gives every value its own random factor, which could not be traced back through a dataset that an LLM has merged and re-ordered. Use level 1-3");
    }
    if base.keep.is_some() {
        bail!("keep is not available in run2 (the datasets have different columns): use drop = COLS for the columns to leave out");
    }
    if !base.rename_columns.is_empty() || !base.rename_except.is_empty() {
        bail!("column renaming is not available in run2: the LLM must be able to see the ADaM column names");
    }
    base.validate()?;

    let mut inputs: Vec<PathBuf> = Vec::new();
    for e in &raw_inputs {
        for p in expand(e)? {
            if !p.is_file() {
                bail!("input file {} not found", p.display());
            }
            let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
            if inputs.iter().any(|q| q.canonicalize().unwrap_or_else(|_| q.clone()) == canon) {
                bail!("{} is listed twice", p.display());
            }
            inputs.push(p);
        }
    }
    Ok(Run2Options { inputs, base, adam_hints })
}

impl Run2Options {
    /// The resolved options as an options file (config.used.txt).
    pub fn dump(&self) -> String {
        let mut s = String::from("# resolved data-obfuscator run2 options (CLI overrides config file)\n");
        for p in &self.inputs {
            s.push_str(&format!("input = {}\n", p.display()));
        }
        s.push_str(&format!("adam_hints = {}\n", self.adam_hints));
        for l in self.base.dump().lines() {
            if !l.starts_with("input =") && !l.starts_with("keep =") && !l.starts_with("rename_") && !l.starts_with("output_name") && !l.starts_with('#') {
                s.push_str(l);
                s.push('\n');
            }
        }
        s
    }
}
