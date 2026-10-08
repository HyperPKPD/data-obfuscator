//! NONMEM-style dataset reader/writer (CSV, TSV, semicolon, whitespace).

use anyhow::{bail, Context, Result};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delimiter {
    Comma,
    Tab,
    Semicolon,
    Whitespace,
}

impl Delimiter {
    pub fn as_char(self) -> Option<char> {
        match self {
            Self::Comma => Some(','),
            Self::Tab => Some('\t'),
            Self::Semicolon => Some(';'),
            Self::Whitespace => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Comma => "CSV (comma)",
            Self::Tab => "TSV (tab)",
            Self::Semicolon => "CSV (semicolon)",
            Self::Whitespace => "NONMEM whitespace",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Dataset {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub delimiter: Delimiter,
    pub comments: Vec<String>,
    pub source_name: String,
    /// Data rows whose field count differs from the header (kept as they are).
    pub ragged_rows: usize,
}

impl Dataset {
    pub fn n_rows(&self) -> usize {
        self.rows.len()
    }
    pub fn n_cols(&self) -> usize {
        self.headers.len()
    }
    pub fn col_index(&self, name: &str) -> Option<usize> {
        let t = name.trim();
        self.headers
            .iter()
            .position(|h| h.eq_ignore_ascii_case(t))
    }
    pub fn column(&self, idx: usize) -> Vec<&str> {
        self.rows
            .iter()
            .map(|r| r.get(idx).map(|s| s.as_str()).unwrap_or(""))
            .collect()
    }
}

pub fn read_dataset(
    path: &Path,
    delimiter: Option<char>,
    no_header: bool,
    columns: &[String],
    comment_chars: &str,
) -> Result<Dataset> {
    let file = fs::File::open(path).with_context(|| format!("opening input {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut raw_lines: Vec<String> = Vec::new();
    for line in reader.lines() {
        raw_lines.push(line?);
    }
    // Excel's "CSV UTF-8" adds a byte-order mark that would otherwise end up inside the
    // first column name (so "ID" would not be recognised as ID).
    if let Some(first) = raw_lines.first_mut() {
        if let Some(rest) = first.strip_prefix('\u{feff}') {
            *first = rest.to_string();
        }
    }
    parse_lines(
        &raw_lines,
        delimiter,
        no_header,
        columns,
        comment_chars,
        path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
    )
}

pub fn parse_lines(
    raw_lines: &[String],
    delimiter: Option<char>,
    no_header: bool,
    columns: &[String],
    comment_chars: &str,
    source_name: String,
) -> Result<Dataset> {
    let mut comments = Vec::new();
    let mut data_lines: Vec<&str> = Vec::new();
    for line in raw_lines {
        let t = line.trim_start();
        if t.is_empty() {
            continue;
        }
        let first = t.chars().next().unwrap_or('\0');
        if comment_chars.contains(first) || t.starts_with("$") {
            comments.push(line.clone());
            continue;
        }
        data_lines.push(line);
    }
    if data_lines.is_empty() {
        bail!("no data rows found in {source_name}");
    }

    let delim = match delimiter {
        Some(',') => Delimiter::Comma,
        Some('\t') => Delimiter::Tab,
        Some(';') => Delimiter::Semicolon,
        Some(' ') => Delimiter::Whitespace,
        Some(other) => {
            // treat as single-char csv
            let _ = other;
            Delimiter::Comma
        }
        None => detect_delimiter(data_lines[0]),
    };

    let mut records: Vec<Vec<String>> = Vec::new();
    for line in &data_lines {
        records.push(split_line(line, delim)?);
    }
    if records.is_empty() {
        bail!("failed to parse any rows");
    }

    let headers: Vec<String>;
    let rows: Vec<Vec<String>>;
    if no_header {
        if columns.is_empty() {
            let n = records[0].len();
            headers = (1..=n).map(|i| format!("COL{i}")).collect();
        } else {
            headers = columns.to_vec();
        }
        rows = records;
    } else if !columns.is_empty() {
        headers = columns.to_vec();
        rows = records;
    } else {
        headers = records[0].clone();
        rows = records[1..].to_vec();
    }

    if headers.is_empty() {
        bail!("header row is empty");
    }
    let width = headers.len();
    // Rows keep their own field count: a row with too few or too many fields is exactly the
    // kind of defect a data-QC pass must still be able to see in the masked copy.
    let mut norm_rows = Vec::with_capacity(rows.len());
    let mut ragged_rows = 0usize;
    for r in rows {
        if r.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        if r.len() != width {
            ragged_rows += 1;
        }
        norm_rows.push(r);
    }
    Ok(Dataset {
        headers,
        rows: norm_rows,
        delimiter: delim,
        comments,
        source_name,
        ragged_rows,
    })
}

fn detect_delimiter(header: &str) -> Delimiter {
    let commas = header.matches(',').count();
    let tabs = header.matches('\t').count();
    let semis = header.matches(';').count();
    if tabs > 0 && tabs >= commas && tabs >= semis {
        return Delimiter::Tab;
    }
    if commas > 0 && commas >= semis {
        return Delimiter::Comma;
    }
    if semis > 0 {
        return Delimiter::Semicolon;
    }
    Delimiter::Whitespace
}

fn split_line(line: &str, delim: Delimiter) -> Result<Vec<String>> {
    match delim {
        Delimiter::Whitespace => Ok(line
            .split_whitespace()
            .map(|s| s.to_string())
            .collect()),
        other => {
            let sep = other.as_char().unwrap_or(',');
            let mut rdr = csv::ReaderBuilder::new()
                .has_headers(false)
                .delimiter(sep as u8)
                .flexible(true)
                .from_reader(line.as_bytes());
            if let Some(rec) = rdr.records().next() {
                let rec = rec.with_context(|| format!("CSV parse: {line}"))?;
                Ok(rec.iter().map(|s| s.to_string()).collect())
            } else {
                Ok(Vec::new())
            }
        }
    }
}

pub fn write_dataset(path: &Path, ds: &Dataset) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::File::create(path)
        .with_context(|| format!("creating output dataset {}", path.display()))?;
    match ds.delimiter {
        Delimiter::Whitespace => {
            writeln!(f, "{}", ds.headers.join(" "))?;
            for row in &ds.rows {
                writeln!(f, "{}", row.join(" "))?;
            }
        }
        other => {
            let sep = other.as_char().unwrap_or(',');
            let mut wtr = csv::WriterBuilder::new()
                .delimiter(sep as u8)
                .flexible(true) // rows may legitimately keep a ragged field count
                .from_writer(f);
            wtr.write_record(&ds.headers)?;
            for row in &ds.rows {
                wtr.write_record(row)?;
            }
            wtr.flush()?;
            return Ok(());
        }
    }
    f.flush()?;
    Ok(())
}

pub fn dataset_to_string(ds: &Dataset) -> String {
    let mut s = String::new();
    match ds.delimiter {
        Delimiter::Whitespace => {
            s.push_str(&ds.headers.join(" "));
            s.push('\n');
            for row in &ds.rows {
                s.push_str(&row.join(" "));
                s.push('\n');
            }
        }
        other => {
            let sep = other.as_char().unwrap_or(',');
            s.push_str(&join_csv(&ds.headers, sep));
            s.push('\n');
            for row in &ds.rows {
                s.push_str(&join_csv(row, sep));
                s.push('\n');
            }
        }
    }
    s
}

fn join_csv(fields: &[String], sep: char) -> String {
    fields
        .iter()
        .map(|f| {
            if f.contains(sep) || f.contains('"') || f.contains('\n') {
                format!("\"{}\"", f.replace('"', "\"\""))
            } else {
                f.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(&sep.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_whitespace_nonmem() {
        let lines = vec![
            "ID TIME DV AMT".into(),
            "1 0 . 100".into(),
            "1 1 12.3 .".into(),
        ];
        let ds = parse_lines(&lines, Some(' '), false, &[], "#", "x.dat".into()).unwrap();
        assert_eq!(ds.headers, vec!["ID", "TIME", "DV", "AMT"]);
        assert_eq!(ds.n_rows(), 2);
        assert_eq!(ds.delimiter, Delimiter::Whitespace);
    }

    #[test]
    fn utf8_bom_is_not_part_of_the_first_header() {
        let dir = std::env::temp_dir().join(format!("do-bom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bom.csv");
        std::fs::write(&p, "\u{feff}ID,TIME,DV\n1,0,.\n").unwrap();
        let ds = read_dataset(&p, None, false, &[], "#;$").unwrap();
        assert_eq!(ds.headers[0], "ID");
    }

    #[test]
    fn parses_csv_and_skips_comments() {
        let lines = vec![
            "# protocol ABC".into(),
            "ID,TIME,DV".into(),
            "1,0,.".into(),
        ];
        let ds = parse_lines(&lines, None, false, &[], "#;$", "x.csv".into()).unwrap();
        assert_eq!(ds.delimiter, Delimiter::Comma);
        assert_eq!(ds.n_rows(), 1);
        assert_eq!(ds.comments.len(), 1);
    }
}
