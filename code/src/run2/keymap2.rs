//! keymap2.txt: the private key of a `run2` (multi-dataset) masking.
//!
//! TAB-separated, one record per line, first field names the record. Lines starting with `#` are comments.
//!
//! ```text
//! format      keymap2   1
//! program     0.0.0.13
//! seed        42
//! level       2
//! date_shift_days        23.6
//! date_shift_whole_days  23
//! file        1   adpc.csv   adpc_masked.csv   120   <sha256 of the masked file>
//! domain_column  subject  USUBJID
//! recode      subject  ABC-001-0001  SUBJ001
//! date_column ADT
//! scale       AVAL  multiply  3.25
//! not_restorable  adae.csv  AETERM  free text redacted
//! ```
//!
//! A *domain* is a kind of identifier (subject, study, site, treatment, ...). Every column of every file that
//! belongs to the domain shares ONE original -> masked table, so equal real values have equal codes in all files.

use anyhow::{bail, Result};
use std::collections::BTreeMap;

pub const MARKER: &str = "data-obfuscator KEYMAP2";

#[derive(Clone, Debug, PartialEq)]
pub struct FileEntry {
    pub index: usize,
    pub original: String,
    pub masked: String,
    pub rows: usize,
    pub sha256: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Domain {
    pub name: String,
    /// Column names (upper case) of every input file that belong to this domain.
    pub columns: Vec<String>,
    /// (original, masked) in order of first appearance.
    pub pairs: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scale2 {
    /// Column name (upper case): one factor for the column in every file that has it.
    pub column: String,
    pub divide: bool,
    pub factor: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyMap2 {
    pub program: Option<String>,
    pub seed: Option<u64>,
    pub level: Option<u8>,
    pub date_shift: f64,
    pub date_whole: i64,
    pub files: Vec<FileEntry>,
    pub domains: Vec<Domain>,
    /// Upper-case names of the columns whose dates were shifted.
    pub date_columns: Vec<String>,
    pub scales: Vec<Scale2>,
    /// (file, column, reason)
    pub not_restorable: Vec<(String, String, String)>,
}

pub fn is_keymap2(text: &str) -> bool {
    text.lines().take(40).any(|l| l.contains(MARKER) || l.trim_start().starts_with("format\tkeymap2"))
}

/// Tabs and line breaks cannot be stored in a TAB-separated file.
pub fn clean(s: &str) -> String {
    s.trim().replace(['\t', '\n', '\r'], " ")
}

impl KeyMap2 {
    pub fn render(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("# {MARKER} - PRIVATE. Never share this file.\n"));
        s.push_str("# It turns the masked ADaM datasets (and the NONMEM-style dataset an LLM builds from them) back into\n");
        s.push_str("# real values: `data-obfuscator reverse --input master.csv --keymap keymap2.txt`.\n");
        s.push_str("#\n");
        s.push_str("# Records (TAB-separated; the first field names the record):\n");
        s.push_str("#   file <n> <original name> <masked name> <rows> <sha256 of the masked file>\n");
        s.push_str("#   domain_column <domain> <COLUMN>      a column (in any file) that belongs to the identifier domain\n");
        s.push_str("#   recode <domain> <original> <masked>   one table per domain, shared by every file\n");
        s.push_str("#   date_column <COLUMN>                  dates in this column were moved by the shift below\n");
        s.push_str("#   scale <COLUMN> <multiply|divide> <factor>   one factor per column name, the same in every file\n");
        s.push_str("#   not_restorable <file> <COLUMN> <why>  values that were redacted or dropped and cannot come back\n");
        s.push_str("#\n");
        s.push_str(&format!("format\tkeymap2\t1\n"));
        s.push_str(&format!("program\t{}\n", self.program.clone().unwrap_or_else(|| crate::APP_VERSION.to_string())));
        if let Some(v) = self.seed {
            s.push_str(&format!("seed\t{v}\n"));
        }
        if let Some(v) = self.level {
            s.push_str(&format!("level\t{v}\n"));
        }
        s.push_str(&format!("date_shift_days\t{}\n", self.date_shift));
        s.push_str(&format!("date_shift_whole_days\t{}\n", self.date_whole));
        s.push_str("\n# 1. FILES\n");
        for f in &self.files {
            s.push_str(&format!("file\t{}\t{}\t{}\t{}\t{}\n", f.index, clean(&f.original), clean(&f.masked), f.rows, f.sha256));
        }
        s.push_str("\n# 2. IDENTIFIER AND LABEL RECODING (shared by all files)\n");
        for d in &self.domains {
            for c in &d.columns {
                s.push_str(&format!("domain_column\t{}\t{}\n", d.name, clean(c)));
            }
        }
        for d in &self.domains {
            for (o, m) in &d.pairs {
                s.push_str(&format!("recode\t{}\t{}\t{}\n", d.name, clean(o), clean(m)));
            }
        }
        s.push_str("\n# 3. DATE SHIFT (days above; applies to these columns)\n");
        for c in &self.date_columns {
            s.push_str(&format!("date_column\t{}\n", clean(c)));
        }
        s.push_str("\n# 4. NUMERIC SCALING (original = masked x factor for divide, masked / factor for multiply)\n");
        for sc in &self.scales {
            s.push_str(&format!("scale\t{}\t{}\t{}\n", clean(&sc.column), if sc.divide { "divide" } else { "multiply" }, sc.factor));
        }
        s.push_str("\n# 5. NOT RESTORABLE\n");
        for (f, c, why) in &self.not_restorable {
            s.push_str(&format!("not_restorable\t{}\t{}\t{}\n", clean(f), clean(c), clean(why)));
        }
        s
    }
}

pub fn parse(text: &str) -> Result<KeyMap2> {
    if !is_keymap2(text) {
        bail!("this is not a keymap2.txt (written by `run2`)");
    }
    let mut km = KeyMap2::default();
    let mut dom: BTreeMap<String, usize> = BTreeMap::new();
    let mut get_dom = |km: &mut KeyMap2, name: &str| -> usize {
        if let Some(&i) = dom.get(name) {
            return i;
        }
        km.domains.push(Domain { name: name.to_string(), ..Default::default() });
        dom.insert(name.to_string(), km.domains.len() - 1);
        km.domains.len() - 1
    };
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let need = |k: usize| -> Result<()> {
            if f.len() < k {
                bail!("keymap2.txt line {}: '{}' needs {} TAB-separated fields, found {}", n + 1, f[0], k, f.len());
            }
            Ok(())
        };
        match f[0] {
            "format" => {
                need(3)?;
                if f[2].trim() != "1" {
                    bail!("keymap2.txt format {} is newer than this program understands", f[2]);
                }
            }
            "program" => {
                need(2)?;
                km.program = Some(f[1].to_string());
            }
            "seed" => {
                need(2)?;
                km.seed = f[1].trim().parse().ok();
            }
            "level" => {
                need(2)?;
                km.level = f[1].trim().parse().ok();
            }
            "date_shift_days" => {
                need(2)?;
                km.date_shift = f[1].trim().parse().map_err(|_| anyhow::anyhow!("keymap2.txt line {}: date_shift_days '{}' is not a number", n + 1, f[1]))?;
            }
            "date_shift_whole_days" => {
                need(2)?;
                km.date_whole = f[1].trim().parse().map_err(|_| anyhow::anyhow!("keymap2.txt line {}: date_shift_whole_days '{}' is not a whole number", n + 1, f[1]))?;
            }
            "file" => {
                need(6)?;
                km.files.push(FileEntry {
                    index: f[1].trim().parse().unwrap_or(km.files.len() + 1),
                    original: f[2].to_string(),
                    masked: f[3].to_string(),
                    rows: f[4].trim().parse().unwrap_or(0),
                    sha256: f[5].trim().to_string(),
                });
            }
            "domain_column" => {
                need(3)?;
                let i = get_dom(&mut km, f[1]);
                let c = f[2].trim().to_ascii_uppercase();
                if !km.domains[i].columns.contains(&c) {
                    km.domains[i].columns.push(c);
                }
            }
            "recode" => {
                need(4)?;
                let i = get_dom(&mut km, f[1]);
                km.domains[i].pairs.push((f[2].to_string(), f[3].to_string()));
            }
            "date_column" => {
                need(2)?;
                let c = f[1].trim().to_ascii_uppercase();
                if !km.date_columns.contains(&c) {
                    km.date_columns.push(c);
                }
            }
            "scale" => {
                need(4)?;
                let factor: f64 = f[3].trim().parse().map_err(|_| anyhow::anyhow!("keymap2.txt line {}: factor '{}' is not a number", n + 1, f[3]))?;
                if !(factor.is_finite() && factor != 0.0) {
                    bail!("keymap2.txt line {}: factor must be a non-zero number", n + 1);
                }
                km.scales.push(Scale2 { column: f[1].trim().to_ascii_uppercase(), divide: f[2].trim() == "divide", factor });
            }
            "not_restorable" => {
                need(4)?;
                km.not_restorable.push((f[1].to_string(), f[2].to_string(), f[3].to_string()));
            }
            _ => {} // unknown record: ignored, so newer programs can add records
        }
    }
    Ok(km)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_then_parse_round_trips() {
        let km = KeyMap2 {
            program: Some("0.0.0.1".into()),
            seed: Some(7),
            level: Some(3),
            date_shift: 23.6,
            date_whole: 23,
            files: vec![FileEntry { index: 1, original: "adpc.csv".into(), masked: "adpc_masked.csv".into(), rows: 3, sha256: "ab".into() }],
            domains: vec![Domain {
                name: "subject".into(),
                columns: vec!["USUBJID".into()],
                pairs: vec![("A-1".into(), "SUBJ001".into()), ("A-2".into(), "SUBJ002".into())],
            }],
            date_columns: vec!["ADT".into()],
            scales: vec![Scale2 { column: "AVAL".into(), divide: true, factor: 3.123456789012345 }],
            not_restorable: vec![("adae.csv".into(), "AETERM".into(), "free text redacted".into())],
        };
        let text = km.render();
        assert!(is_keymap2(&text));
        assert_eq!(parse(&text).unwrap(), km);
    }

    #[test]
    fn an_ordinary_keymap_is_not_a_keymap2() {
        assert!(!is_keymap2("# data-obfuscator keymap - PRIVATE\nrenamed_column\tA\tB\n"));
        assert!(parse("renamed_column\tA\tB\n").is_err());
    }
}
