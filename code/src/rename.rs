//! Optional column renaming: give selected columns neutral names (`COL07`) or names the user
//! chooses, so that revealing column names (`SITE_NAME_XYZ`, `PROTO_ABC_FLAG`) do not travel with
//! the data. Either name the columns to rename, or name the columns to keep and rename the rest.

use crate::config::RunOptions;
use crate::dataset::Dataset;
use anyhow::{bail, Result};
use std::collections::HashSet;

/// NONMEM and mrgsolve expect these names; renaming them is allowed but warned about.
const CORE_COLUMNS: [&str; 7] = ["ID", "TIME", "DV", "AMT", "EVID", "MDV", "CMT"];

pub struct RenamePlan {
    /// (original name, new name) in column order. Empty when nothing is renamed.
    pub pairs: Vec<(String, String)>,
    /// New names that were generated automatically (their meaning is not visible); names the
    /// user chose with `OLD=NEW` are not in this list.
    pub auto_names: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn is_active(opts: &RunOptions) -> bool {
    !opts.rename_columns.is_empty() || !opts.rename_except.is_empty()
}

/// `WT` -> (`WT`, None); `WT=BODYSIZE` -> (`WT`, Some(`BODYSIZE`)).
pub fn split_entry(e: &str) -> (String, Option<String>) {
    match e.split_once('=') {
        Some((a, b)) => (a.trim().to_string(), Some(b.trim().to_string())),
        None => (e.trim().to_string(), None),
    }
}

fn check_new_name(n: &str) -> Result<()> {
    if n.is_empty() {
        bail!("rename: a new column name is empty (write OLD=NEW with a name after the '=')");
    }
    if n.chars().any(|c| c.is_whitespace() || matches!(c, ',' | ';' | '|' | '"' | '\'' | '=' )) {
        bail!("rename: the new column name '{n}' may not contain spaces, commas, semicolons, '|', '=' or quotes");
    }
    Ok(())
}

/// Checks that need no dataset; called from `RunOptions::validate`.
pub fn validate_options(opts: &RunOptions) -> Result<()> {
    if !opts.rename_columns.is_empty() && !opts.rename_except.is_empty() {
        bail!("rename_columns and rename_except cannot be used together: name the columns to rename, or the columns NOT to rename, not both");
    }
    check_new_name(&opts.rename_prefix).map_err(|_| {
        anyhow::anyhow!("rename_prefix '{}' must be a non-empty name without spaces, commas, semicolons, quotes or '='", opts.rename_prefix)
    })?;
    for e in &opts.rename_columns {
        let (old, new) = split_entry(e);
        if old.is_empty() {
            bail!("rename_columns: '{e}' has no column name before the '='");
        }
        if let Some(n) = new {
            check_new_name(&n)?;
        }
    }
    for e in &opts.rename_except {
        if e.contains('=') {
            bail!("rename_except lists columns to leave alone and takes plain names, not OLD=NEW ('{e}')");
        }
    }
    Ok(())
}

fn find(headers: &[String], name: &str) -> Option<usize> {
    let t = name.trim();
    headers.iter().position(|h| h.eq_ignore_ascii_case(t))
}

/// Work out which output columns get which new name.
pub fn plan(headers: &[String], opts: &RunOptions) -> Result<RenamePlan> {
    let mut warnings = Vec::new();
    // (column index, name chosen by the user)
    let mut targets: Vec<(usize, Option<String>)> = Vec::new();

    if !opts.rename_columns.is_empty() {
        let mut seen = HashSet::new();
        for e in &opts.rename_columns {
            let (old, new) = split_entry(e);
            match find(headers, &old) {
                Some(i) => {
                    if seen.insert(i) {
                        targets.push((i, new));
                    }
                }
                None => warnings.push(format!(
                    "rename_columns: '{old}' is not a column of the masked dataset (not in the file, or removed by keep/drop) - ignored"
                )),
            }
        }
    } else if !opts.rename_except.is_empty() {
        let mut keep_idx = HashSet::new();
        for name in &opts.rename_except {
            match find(headers, name) {
                Some(i) => {
                    keep_idx.insert(i);
                }
                None => warnings.push(format!(
                    "rename_except: '{}' is not a column of the masked dataset - ignored",
                    name.trim()
                )),
            }
        }
        for i in 0..headers.len() {
            if !keep_idx.contains(&i) {
                targets.push((i, None));
            }
        }
        if targets.is_empty() {
            warnings.push("rename_except lists every column: nothing was renamed".into());
        }
    }
    if targets.is_empty() {
        return Ok(RenamePlan { pairs: Vec::new(), auto_names: Vec::new(), warnings });
    }
    targets.sort_by_key(|t| t.0);

    let renamed_idx: HashSet<usize> = targets.iter().map(|t| t.0).collect();
    let mut used: HashSet<String> = headers
        .iter()
        .enumerate()
        .filter(|(i, _)| !renamed_idx.contains(i))
        .map(|(_, h)| h.to_ascii_lowercase())
        .collect();

    let mut new_names: Vec<Option<String>> = vec![None; targets.len()];
    // names chosen by the user first: they must be unique and may not clash with kept columns
    for (k, (_, explicit)) in targets.iter().enumerate() {
        if let Some(n) = explicit {
            if !used.insert(n.to_ascii_lowercase()) {
                bail!("rename: the new name '{n}' is already used by another column (or chosen twice)");
            }
            new_names[k] = Some(n.clone());
        }
    }
    let width = headers.len().to_string().len().max(2);
    for (k, (i, _)) in targets.iter().enumerate() {
        if new_names[k].is_none() {
            let mut n = format!("{}{:0w$}", opts.rename_prefix, i + 1, w = width);
            while used.contains(&n.to_ascii_lowercase()) {
                n.push('_');
            }
            used.insert(n.to_ascii_lowercase());
            new_names[k] = Some(n);
        }
    }

    let mut pairs = Vec::new();
    let mut auto_names = Vec::new();
    let mut core = Vec::new();
    for (k, (i, _)) in targets.iter().enumerate() {
        let new = new_names[k].clone().unwrap();
        if new == headers[*i] {
            continue;
        }
        if CORE_COLUMNS.iter().any(|c| c.eq_ignore_ascii_case(&headers[*i])) {
            core.push(headers[*i].clone());
        }
        if targets[k].1.is_none() {
            auto_names.push(new.clone());
        }
        pairs.push((headers[*i].clone(), new));
    }
    if !core.is_empty() {
        warnings.push(format!(
            "renamed NONMEM/mrgsolve core column(s) {}: the masked file no longer has the standard names (llm.txt uses the new names). List them in rename_except to keep them",
            core.join(", ")
        ));
    }
    Ok(RenamePlan { pairs, auto_names, warnings })
}

/// Rename the header cells of `ds` (matched by original name).
pub fn apply(ds: &mut Dataset, pairs: &[(String, String)]) {
    for (old, new) in pairs {
        if let Some(i) = ds.col_index(old) {
            ds.headers[i] = new.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    fn opts(cols: &[&str], except: &[&str]) -> RunOptions {
        let mut o = RunOptions::default();
        o.rename_columns = h(cols);
        o.rename_except = h(except);
        o
    }

    #[test]
    fn listed_columns_get_numbered_names_by_position() {
        let p = plan(&h(&["ID", "TIME", "WT", "AGE", "SITE_NAME"]), &opts(&["wt", "SITE_NAME"], &[])).unwrap();
        assert_eq!(p.pairs, vec![("WT".into(), "COL03".into()), ("SITE_NAME".into(), "COL05".into())]);
        assert!(p.warnings.is_empty());
    }

    #[test]
    fn except_mode_renames_everything_else() {
        let p = plan(&h(&["ID", "TIME", "DV", "WT", "AGE"]), &opts(&[], &["ID", "time", "DV"])).unwrap();
        assert_eq!(p.pairs, vec![("WT".into(), "COL04".into()), ("AGE".into(), "COL05".into())]);
    }

    #[test]
    fn user_chosen_names_and_collisions() {
        let p = plan(&h(&["ID", "WT", "AGE"]), &opts(&["WT=BODYSIZE"], &[])).unwrap();
        assert_eq!(p.pairs, vec![("WT".into(), "BODYSIZE".into())]);
        // an automatic name never collides with an existing column
        let p = plan(&h(&["ID", "COL02", "AGE"]), &opts(&["AGE", "ID"], &[])).unwrap();
        assert_eq!(p.pairs[1], ("AGE".into(), "COL03".into()));
        // a chosen name that is already taken is an error
        assert!(plan(&h(&["ID", "WT", "AGE"]), &opts(&["WT=AGE"], &[])).is_err());
        assert!(plan(&h(&["ID", "WT", "AGE"]), &opts(&["WT=X", "AGE=x"], &[])).is_err());
    }

    #[test]
    fn unknown_names_warn_and_core_columns_warn() {
        let p = plan(&h(&["ID", "TIME", "WT"]), &opts(&["NOPE", "TIME"], &[])).unwrap();
        assert_eq!(p.pairs.len(), 1);
        assert_eq!(p.warnings.len(), 2);
        assert!(p.warnings.iter().any(|w| w.contains("NOPE")));
        assert!(p.warnings.iter().any(|w| w.contains("core")));
    }

    #[test]
    fn option_validation() {
        let mut o = opts(&["WT"], &["ID"]);
        assert!(validate_options(&o).is_err(), "both lists");
        o = opts(&["WT="], &[]);
        assert!(validate_options(&o).is_err(), "empty new name");
        o = opts(&["WT=A B"], &[]);
        assert!(validate_options(&o).is_err(), "space in name");
        o = opts(&[], &["ID=X"]);
        assert!(validate_options(&o).is_err(), "= in except");
        o = opts(&["WT=X1"], &[]);
        o.rename_prefix = String::new();
        assert!(validate_options(&o).is_err(), "empty prefix");
        assert!(validate_options(&opts(&["WT=X1", "AGE"], &[])).is_ok());
    }
}
