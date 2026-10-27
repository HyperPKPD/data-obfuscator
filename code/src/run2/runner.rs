//! `run2`: read the datasets, mask them together, write the masked files, keymap2.txt, llm2.txt, summary2.txt.

use super::args::Run2Options;
use super::keymap2::FileEntry;
use super::mask::{mask_all, Input};
use super::prompt;
use crate::dataset::{read_dataset, write_dataset, Delimiter};
use crate::trace::TraceLog;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Run2Artifacts {
    /// The masked datasets, in input order.
    pub masked_files: Vec<PathBuf>,
    pub keymap_path: Option<PathBuf>,
    pub llm_path: PathBuf,
    pub summary_path: PathBuf,
    pub config_used_path: PathBuf,
    pub log_path: PathBuf,
    pub warnings: usize,
}

/// Stops before anything is written if the folder already holds files and `--overwrite` was not given.
pub fn check_output_dir(opts: &Run2Options) -> Result<usize> {
    crate::reverse::check_output_dir(&opts.base.output_dir, opts.base.overwrite)
}

fn masked_name(input: &Path, delim: Delimiter, taken: &[String]) -> String {
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "dataset".into());
    let ext = match delim {
        Delimiter::Tab => "tsv",
        Delimiter::Whitespace => "dat",
        _ => "csv",
    };
    let mut name = format!("{stem}_masked.{ext}");
    let mut k = 2;
    while taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
        name = format!("{stem}_masked_{k}.{ext}");
        k += 1;
    }
    name
}

pub fn run2(opts: &Run2Options, log: &mut TraceLog) -> Result<Run2Artifacts> {
    let log: &TraceLog = log;
    let base = &opts.base;
    log.info(format!("run2: {} dataset(s), level {}, seed {}", opts.inputs.len(), base.level, base.seed));
    fs::create_dir_all(&base.output_dir).with_context(|| format!("creating {}", base.output_dir.display()))?;

    let mut inputs: Vec<Input> = Vec::new();
    for p in &opts.inputs {
        let ds = read_dataset(p, base.delimiter, base.no_header, &base.columns, &base.comment_chars)
            .with_context(|| format!("reading {}", p.display()))?;
        let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string());
        log.success(format!("read {} ({} rows x {} columns, {})", name, ds.n_rows(), ds.n_cols(), ds.delimiter.name()));
        inputs.push(Input { name, ds });
    }

    let masked = mask_all(&inputs, opts, log)?;

    let mut names: Vec<String> = Vec::new();
    let mut files: Vec<FileEntry> = Vec::new();
    let mut masked_files = Vec::new();
    for (i, (inp, out)) in inputs.iter().zip(&masked.outputs).enumerate() {
        let name = masked_name(&opts.inputs[i], out.delimiter, &names);
        let path = base.output_dir.join(&name);
        write_dataset(&path, out)?;
        let sha = hex::encode(Sha256::digest(&fs::read(&path).with_context(|| format!("re-reading {}", path.display()))?));
        log.success(format!("wrote {} ({} rows x {} columns)", path.display(), out.n_rows(), out.n_cols()));
        files.push(FileEntry { index: i + 1, original: inp.name.clone(), masked: name.clone(), rows: out.n_rows(), sha256: sha });
        names.push(name);
        masked_files.push(path);
    }

    let mut key = masked.key.clone();
    key.files = files.clone();

    let keymap_path = if base.write_keymap {
        let p = base.output_dir.join("keymap2.txt");
        fs::write(&p, key.render()).with_context(|| format!("writing {}", p.display()))?;
        log.success(format!("wrote {} - PRIVATE: it holds the real identifiers, the date shift and the scaling factors", p.display()));
        Some(p)
    } else {
        log.warn("keymap2.txt is switched off (write_keymap = false): the masking can never be reversed");
        None
    };

    let llm_path = base.output_dir.join("llm2.txt");
    fs::write(&llm_path, prompt::render(&files, &masked.outputs, &masked.plans, &masked.hints, &key, base.level))
        .with_context(|| format!("writing {}", llm_path.display()))?;
    log.success(format!("wrote {} (ready-to-paste prompts for combining the datasets)", llm_path.display()));

    let summary_path = base.output_dir.join("summary2.txt");
    fs::write(&summary_path, prompt::summary(&files, &masked.outputs, &masked.plans, &key, opts))
        .with_context(|| format!("writing {}", summary_path.display()))?;
    let config_used_path = base.output_dir.join("config.used.txt");
    fs::write(&config_used_path, opts.dump())?;

    if keymap_path.is_some() {
        log.info("share with the LLM: the *_masked files and llm2.txt. Keep private: keymap2.txt, summary2.txt, config.used.txt, run2.log");
    }
    Ok(Run2Artifacts {
        masked_files,
        keymap_path,
        llm_path,
        summary_path,
        config_used_path,
        log_path: log.path().to_path_buf(),
        warnings: log.warn_count(),
    })
}
