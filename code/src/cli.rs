use crate::config::{
    add_cols,
    apply_map, parse_bool, parse_config_file, parse_level, split_list, IdStyle, RunOptions,
    TokensOptions,
};
use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "data-obfuscator",
    disable_version_flag = true,
    about = "Mask NONMEM-style pharmacometric datasets for safe sharing - for Clinical Pharmacology, Pharmacometrics and Data Science.",
    long_about = "Recode study, subject, treatment, site, and user identifiers so a real \
NONMEM dataset can be handed to an LLM or collaborator while looking synthetic. \
PK/PD structure is preserved: within-subject TIME order, doses, DV, and event flags \
are never altered.\n\n\
Intended for Clinical Pharmacology, Pharmacometrics and Data Science work.\n\n\
Besides the masked dataset, `run` writes llm.txt (ready-to-paste analysis, NONMEM \
control-stream and QC prompts tailored to the dataset) and tokens.txt (token \
estimates for GPT, Claude, Gemini, Llama and other models).\n\n\
Examples:\n  \
data-obfuscator run --input pk.csv --output ./masked --level 2\n  \
data-obfuscator run --config study_options.txt\n  \
data-obfuscator tokens --input pk.csv --output ./masked"
)]
pub struct Cli {
    /// Print the version banner (what the program does and who it is for)
    #[arg(short = 'V', long = "version")]
    pub show_version: bool,

    /// Disable coloured output (also honoured: the NO_COLOR environment variable)
    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Recode a NONMEM-style dataset and write summary, tokens, and a live log
    Run(RunArgs),
    /// Estimate token counts (GPT, Claude, Gemini, Llama, ...) for a file and an output directory
    Tokens(TokensArgs),
}

#[derive(clap::Args, Debug, Clone)]
pub struct RunArgs {
    /// NONMEM-style dataset (CSV, TSV, semicolon, or whitespace-delimited)
    #[arg(short, long, value_name = "FILE")]
    pub input: Option<PathBuf>,

    /// Output folder (created if missing)
    #[arg(short, long, value_name = "DIR")]
    pub output: Option<PathBuf>,

    /// Obfuscation level: 1 light, 2 clinical (default), 3 covariate, 4 maximum
    #[arg(short, long, value_name = "LEVEL")]
    pub level: Option<String>,

    /// Options file (key = value). CLI flags override the file.
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Columns to keep, comma-separated. Default: all columns
    #[arg(long, value_name = "COLS")]
    pub keep: Option<String>,

    /// Columns to drop, comma-separated
    #[arg(long, value_name = "COLS")]
    pub drop: Option<String>,

    /// Subject ID column (NONMEM ID). Several allowed: comma-separated or repeat the flag
    #[arg(long, value_name = "COLS", action = clap::ArgAction::Append)]
    pub id_column: Vec<String>,

    /// Unique subject identifier column(s) (USUBJID / SUBJID). Several allowed
    #[arg(long, value_name = "COLS", action = clap::ArgAction::Append)]
    pub subject_column: Vec<String>,

    /// Study column(s), e.g. STU,STUDYID,STUDY. Several allowed
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

    /// Date columns, comma-separated
    #[arg(long, value_name = "COLS")]
    pub date_columns: Option<String>,

    /// Extra identifier columns to recode
    #[arg(long, value_name = "COLS")]
    pub sensitive: Option<String>,

    /// Columns that must never be altered (default: TIME, DV, AMT, EVID, …)
    #[arg(long, value_name = "COLS")]
    pub protect: Option<String>,

    /// Extra numeric columns to scale (multiply or divide by a hidden random factor) at level >= 3.
    /// Covariates such as WT and AGE are scaled automatically at level 3 and 4
    #[arg(long, alias = "jitter-columns", value_name = "COLS")]
    pub scale_columns: Option<String>,

    /// Rename these columns in the masked file (automatic names like COL07, or WT=BODYSIZE to choose).
    /// Every rename is recorded in keymap.txt
    #[arg(long, value_name = "COLS")]
    pub rename_columns: Option<String>,

    /// Rename every column EXCEPT these (use instead of --rename-columns when most columns are renamed)
    #[arg(long, alias = "no-rename", value_name = "COLS")]
    pub rename_except: Option<String>,

    /// Prefix for automatic new column names (default COL gives COL01, COL02, ...)
    #[arg(long, value_name = "PREFIX")]
    pub rename_prefix: Option<String>,

    /// Free-text columns to redact at level ≥ 2
    #[arg(long, value_name = "COLS")]
    pub freetext_columns: Option<String>,

    /// RNG seed (default 42). Same input + options + seed = identical output every run.
    /// Recorded in summary.txt. For a random seed use `seed = auto` in a config file.
    #[arg(long)]
    pub seed: Option<u64>,

    /// Field delimiter: auto, comma, tab, space, semicolon
    #[arg(long)]
    pub delimiter: Option<String>,

    /// First row is data, not a header
    #[arg(long, default_value_t = false)]
    pub no_header: bool,

    /// Header names when --no-header, comma-separated
    #[arg(long, value_name = "COLS")]
    pub columns: Option<String>,

    /// ID recoding style: sequential (default), padded, hashed
    #[arg(long, value_name = "STYLE")]
    pub id_style: Option<String>,

    /// Range of the random factor that covariates are multiplied or divided by (level >= 3), default 1-20
    #[arg(long, value_name = "MIN-MAX")]
    pub scale_range: Option<String>,

    /// Removed in 0.0.0.4 (jitter was replaced by --scale-range); kept only to give a clear error
    #[arg(long, hide = true, value_name = "FRACTION")]
    pub jitter: Option<String>,

    /// Date shift in days: a MIN-MAX window to draw a random decimal from (default 1-100, e.g. 23.6)
    /// or one fixed number (e.g. 23.6). Decimals and negative values are allowed.
    #[arg(long, alias = "date-shift", value_name = "MIN-MAX|DAYS", allow_hyphen_values = true)]
    pub date_shift_days: Option<String>,

    /// Write keymap.txt (original -> masked, plus the scaling factor of every scaled column).
    /// On by default; use `--write-keymap false` (or 0) to switch it off. Never share that file
    #[arg(long, alias = "keymap", value_name = "BOOL", num_args = 0..=1, default_missing_value = "true")]
    pub write_keymap: Option<String>,

    /// Allow writing into an output folder that already contains files (they may be replaced)
    #[arg(long, default_value_t = false)]
    pub overwrite: bool,

    /// Output dataset file name (default obfuscated.csv / .dat / .tsv)
    #[arg(long, value_name = "NAME")]
    pub output_name: Option<String>,

    /// Leading comment characters (default #;$)
    #[arg(long)]
    pub comment_chars: Option<String>,

    /// Drop free-text columns instead of redacting (on by default at level 4)
    #[arg(long, default_value_t = false)]
    pub drop_freetext: bool,

    /// Verbose debug lines in the log
    #[arg(short, long, default_value_t = false)]
    pub verbose: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct TokensArgs {
    /// File to count
    #[arg(short, long, value_name = "FILE")]
    pub input: Option<PathBuf>,

    /// Directory to write tokens.txt; other files here are also counted (except the log)
    #[arg(short, long, value_name = "DIR")]
    pub output: Option<PathBuf>,

    /// Options file
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Extra file names to exclude (log is always excluded)
    #[arg(long, value_name = "NAMES")]
    pub exclude: Option<String>,

    /// Verbose log
    #[arg(short, long, default_value_t = false)]
    pub verbose: bool,
}

pub fn resolve_run(args: &RunArgs) -> Result<RunOptions> {
    let mut opts = RunOptions::default();
    if let Some(cfg) = &args.config {
        let map = parse_config_file(cfg)?;
        apply_map(&mut opts, &map)?;
        opts.config_path = Some(cfg.clone());
    }
    if let Some(p) = &args.input {
        opts.input = p.clone();
    }
    if let Some(p) = &args.output {
        opts.output_dir = p.clone();
    }
    if let Some(l) = &args.level {
        opts.level = parse_level(l)?;
    }
    if let Some(k) = &args.keep {
        opts.keep = Some(split_list(k));
    }
    if let Some(d) = &args.drop {
        opts.drop = split_list(d);
    }
    if !args.id_column.is_empty() {
        // CLI replaces whatever the config file said for this role.
        opts.id_column.clear();
        for v in &args.id_column {
            add_cols(&mut opts.id_column, v);
        }
    }
    if !args.subject_column.is_empty() {
        // CLI replaces whatever the config file said for this role.
        opts.subject_column.clear();
        for v in &args.subject_column {
            add_cols(&mut opts.subject_column, v);
        }
    }
    if !args.study_column.is_empty() {
        // CLI replaces whatever the config file said for this role.
        opts.study_column.clear();
        for v in &args.study_column {
            add_cols(&mut opts.study_column, v);
        }
    }
    if !args.site_column.is_empty() {
        // CLI replaces whatever the config file said for this role.
        opts.site_column.clear();
        for v in &args.site_column {
            add_cols(&mut opts.site_column, v);
        }
    }
    if !args.treatment_column.is_empty() {
        // CLI replaces whatever the config file said for this role.
        opts.treatment_column.clear();
        for v in &args.treatment_column {
            add_cols(&mut opts.treatment_column, v);
        }
    }
    if !args.user_column.is_empty() {
        // CLI replaces whatever the config file said for this role.
        opts.user_column.clear();
        for v in &args.user_column {
            add_cols(&mut opts.user_column, v);
        }
    }
    if let Some(v) = &args.date_columns {
        opts.date_columns = split_list(v);
    }
    if let Some(v) = &args.sensitive {
        opts.sensitive = split_list(v);
    }
    if let Some(v) = &args.protect {
        opts.protect = split_list(v);
    }
    if let Some(v) = &args.scale_columns {
        opts.scale_columns = split_list(v);
    }
    if args.rename_columns.is_some() && args.rename_except.is_some() {
        bail!("--rename-columns and --rename-except cannot be used together: name the columns to rename, or the columns NOT to rename, not both");
    }
    // a rename option on the command line replaces the file's choice of rename mode
    if let Some(v) = &args.rename_columns {
        opts.rename_columns = split_list(v);
        opts.rename_except.clear();
    }
    if let Some(v) = &args.rename_except {
        opts.rename_except = split_list(v);
        opts.rename_columns.clear();
    }
    if let Some(v) = &args.rename_prefix {
        opts.rename_prefix = v.trim().to_string();
    }
    if let Some(v) = &args.freetext_columns {
        opts.freetext_columns = split_list(v);
    }
    if let Some(s) = args.seed {
        opts.seed = s;
        opts.seed_was_generated = false;
    }
    if let Some(d) = &args.delimiter {
        let mut m = std::collections::HashMap::new();
        m.insert("delimiter".into(), d.clone());
        apply_map(&mut opts, &m)?;
    }
    if args.no_header {
        opts.no_header = true;
    }
    if let Some(c) = &args.columns {
        opts.columns = split_list(c);
    }
    if let Some(s) = &args.id_style {
        opts.id_style = IdStyle::parse(s)?;
    }
    if args.jitter.is_some() {
        bail!("--jitter was replaced in 0.0.0.4: covariates are now multiplied or divided by a random factor per column. Use --scale-range MIN-MAX (default 1-20)");
    }
    if let Some(r) = &args.scale_range {
        let mut m = std::collections::HashMap::new();
        m.insert("scale_range".into(), r.clone());
        apply_map(&mut opts, &m)?;
    }
    if let Some(w) = &args.date_shift_days {
        let mut m = std::collections::HashMap::new();
        m.insert("date_shift_days".into(), w.clone());
        apply_map(&mut opts, &m)?;
    }
    if let Some(v) = &args.write_keymap {
        opts.write_keymap = parse_bool(v)?;
    }
    if args.overwrite {
        opts.overwrite = true;
    }
    if let Some(n) = &args.output_name {
        opts.output_name = Some(n.clone());
    }
    if let Some(c) = &args.comment_chars {
        opts.comment_chars = c.clone();
    }
    if args.drop_freetext {
        opts.drop_freetext = true;
    }
    if args.verbose {
        opts.verbose = true;
    }

    if opts.input.as_os_str().is_empty() {
        bail!("--input is required (or set input = ... in the config file)");
    }
    if opts.output_dir.as_os_str().is_empty() {
        bail!("--output is required (or set output = ... in the config file)");
    }
    opts.validate()?;
    Ok(opts)
}

/// Stops the run before anything is written if the output folder already holds files and the
/// user did not pass `--overwrite`. Returns how many entries already exist there.
pub fn check_output_dir(opts: &RunOptions) -> Result<usize> {
    let dir = &opts.output_dir;
    if !dir.exists() {
        return Ok(0);
    }
    if !dir.is_dir() {
        bail!("the output path {} exists and is not a folder", dir.display());
    }
    let existing = std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("cannot read output folder {}: {e}", dir.display()))?
        .count();
    if existing > 0 && !opts.overwrite {
        bail!(
            "output folder {} already contains {} file(s) or folder(s). Nothing was written. Choose another --output folder, or add --overwrite to replace the files this program writes there",
            dir.display(),
            existing
        );
    }
    Ok(existing)
}

pub fn resolve_tokens(args: &TokensArgs) -> Result<(TokensOptions, bool)> {
    let mut input = args.input.clone().unwrap_or_default();
    let mut output = args.output.clone().unwrap_or_default();
    let mut exclude = args
        .exclude
        .as_deref()
        .map(split_list)
        .unwrap_or_default();
    if let Some(cfg) = &args.config {
        let map = parse_config_file(cfg)?;
        if let Some(v) = map.get("input") {
            if input.as_os_str().is_empty() {
                input = PathBuf::from(v);
            }
        }
        if let Some(v) = map.get("output") {
            if output.as_os_str().is_empty() {
                output = PathBuf::from(v);
            }
        }
        if let Some(v) = map.get("exclude") {
            if exclude.is_empty() {
                exclude = split_list(v);
            }
        }
    }
    if input.as_os_str().is_empty() {
        bail!("--input is required");
    }
    if output.as_os_str().is_empty() {
        bail!("--output is required");
    }
    Ok((
        TokensOptions {
            input,
            output_dir: output,
            exclude,
        },
        args.verbose,
    ))
}
