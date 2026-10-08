//! Token estimation for several model families.
//!
//! One shared pretokenizer (the published cl100k_base regex rules: contractions,
//! letter runs, 1-3 digit groups, punctuation, newlines) is scored with a small
//! per-family profile. The profile captures the differences that matter most for
//! numeric CSV files:
//!
//! * **digit handling** - GPT-4/4o, Llama 3, Claude 3+ and Gemini group digits
//!   (up to 3 per token); Mistral 7B / Llama 2 / Qwen emit one token per digit,
//!   which makes numeric datasets noticeably more expensive;
//! * **word compression** - how many letters a long, rare identifier packs into
//!   one token (bigger vocabularies pack more);
//! * **punctuation merging** - whether ",TIME" is one pretoken or two.
//!
//! These are *estimates* (typically within roughly +-10-25 % of the real tokenizer on
//! CSV text). For exact counts use the provider's own counter. File-content counts
//! do **not** add BOS/EOS special tokens.

use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::Path;

/// How one model family is scored.
#[derive(Clone, Copy, Debug)]
pub struct ModelProfile {
    pub id: &'static str,
    pub label: &'static str,
    /// Digits merged per token (3 = grouped, 1 = one token per digit).
    pub digit_group: u64,
    /// Words up to this many letters are a single token.
    pub word_cap: u64,
    /// Letters per token for longer, rarer identifiers.
    pub long_chars: f64,
    /// Does "," + "TIME" merge into one token?
    pub prefix_merge: bool,
}

/// Index 0 is the primary (GPT-4 / cl100k) model used for headline numbers.
pub const MODELS: &[ModelProfile] = &[
    ModelProfile {
        id: "gpt4",
        label: "GPT-4 / GPT-3.5 (cl100k_base)",
        digit_group: 3,
        word_cap: 10,
        long_chars: 4.0,
        prefix_merge: true,
    },
    ModelProfile {
        id: "gpt4o",
        label: "GPT-4o / GPT-4.1 / o-series / GPT-5 (o200k_base)",
        digit_group: 3,
        word_cap: 12,
        long_chars: 4.5,
        prefix_merge: true,
    },
    ModelProfile {
        id: "claude",
        label: "Claude 3 and later (Anthropic)",
        digit_group: 3,
        word_cap: 8,
        long_chars: 3.5,
        prefix_merge: true,
    },
    ModelProfile {
        id: "gemini",
        label: "Gemini (Google)",
        digit_group: 3,
        word_cap: 12,
        long_chars: 4.5,
        prefix_merge: false,
    },
    ModelProfile {
        id: "llama3",
        label: "Llama 3 / 3.1 / 3.3 (Meta)",
        digit_group: 3,
        word_cap: 10,
        long_chars: 4.0,
        prefix_merge: true,
    },
    ModelProfile {
        id: "qwen",
        label: "Qwen 2 / 2.5 / 3 (Alibaba)",
        digit_group: 1,
        word_cap: 10,
        long_chars: 4.0,
        prefix_merge: true,
    },
    ModelProfile {
        id: "mistral",
        label: "Mistral 7B / Mixtral / Llama 2 (32k vocab)",
        digit_group: 1,
        word_cap: 6,
        long_chars: 3.0,
        prefix_merge: false,
    },
];

/// Headline (GPT-4 / cl100k) token count.
pub fn count_tokens(text: &str) -> u64 {
    let pieces = pretokens(text);
    pieces
        .iter()
        .map(|p| tokens_for_piece_with(&MODELS[0], p))
        .sum()
}

/// Token estimates for every model in [`MODELS`], in the same order.
pub fn count_all_models(text: &str) -> Vec<u64> {
    let pieces = pretokens(text);
    MODELS
        .iter()
        .map(|m| pieces.iter().map(|p| tokens_for_piece_with(m, p)).sum())
        .collect()
}

pub fn count_file_tokens(path: &Path) -> Result<u64> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading {} for token count", path.display()))?;
    Ok(count_tokens(&text))
}

pub fn pretokens(text: &str) -> Vec<String> {
    let chs: Vec<char> = text.chars().collect();
    let n = chs.len();
    let mut i = 0usize;
    let mut out = Vec::new();
    while i < n {
        if let Some(len) = match_contraction(&chs, i) {
            out.push(chs[i..i + len].iter().collect());
            i += len;
            continue;
        }
        if let Some(len) = match_opt_other_plus_letters(&chs, i) {
            out.push(chs[i..i + len].iter().collect());
            i += len;
            continue;
        }
        if let Some(len) = match_numbers(&chs, i) {
            out.push(chs[i..i + len].iter().collect());
            i += len;
            continue;
        }
        if let Some(len) = match_punct(&chs, i) {
            out.push(chs[i..i + len].iter().collect());
            i += len;
            continue;
        }
        if let Some(len) = match_ws_nl(&chs, i) {
            out.push(chs[i..i + len].iter().collect());
            i += len;
            continue;
        }
        if let Some(len) = match_trailing_ws(&chs, i) {
            out.push(chs[i..i + len].iter().collect());
            i += len;
            continue;
        }
        if let Some(len) = match_ws(&chs, i) {
            out.push(chs[i..i + len].iter().collect());
            i += len;
            continue;
        }
        out.push(chs[i].to_string());
        i += 1;
    }
    out
}

fn is_l(c: char) -> bool {
    c.is_alphabetic()
}
fn is_n(c: char) -> bool {
    c.is_numeric()
}
fn is_ws(c: char) -> bool {
    c.is_whitespace()
}
fn is_nl(c: char) -> bool {
    c == '\n' || c == '\r'
}

fn match_contraction(chs: &[char], i: usize) -> Option<usize> {
    if chs[i] != '\'' {
        return None;
    }
    const ENDS: &[&str] = &["s", "t", "re", "ve", "m", "ll", "d"];
    let rest: String = chs[i + 1..].iter().collect();
    let lower = rest.to_ascii_lowercase();
    for e in ENDS {
        if lower.starts_with(e)
            && (e.len() == rest.len()
                || rest
                    .chars()
                    .nth(e.len())
                    .map(|c| !is_l(c))
                    .unwrap_or(true))
        {
            return Some(1 + e.len());
        }
    }
    None
}

fn match_opt_other_plus_letters(chs: &[char], i: usize) -> Option<usize> {
    let n = chs.len();
    let mut k = i;
    if k < n && !is_nl(chs[k]) && !is_l(chs[k]) && !is_n(chs[k]) {
        k += 1;
    }
    if k >= n || !is_l(chs[k]) {
        return None;
    }
    k += 1;
    while k < n && is_l(chs[k]) {
        k += 1;
    }
    Some(k - i)
}

fn match_numbers(chs: &[char], i: usize) -> Option<usize> {
    if !is_n(chs[i]) {
        return None;
    }
    let mut k = i + 1;
    let n = chs.len();
    while k < n && k - i < 3 && is_n(chs[k]) {
        k += 1;
    }
    Some(k - i)
}

fn match_punct(chs: &[char], i: usize) -> Option<usize> {
    let n = chs.len();
    let mut k = i;
    if k < n && chs[k] == ' ' {
        k += 1;
    }
    let start_punct = k;
    while k < n && !is_ws(chs[k]) && !is_l(chs[k]) && !is_n(chs[k]) {
        k += 1;
    }
    if k == start_punct {
        return None;
    }
    while k < n && is_nl(chs[k]) {
        k += 1;
    }
    Some(k - i)
}

fn match_ws_nl(chs: &[char], i: usize) -> Option<usize> {
    let n = chs.len();
    let mut k = i;
    while k < n && is_ws(chs[k]) && !is_nl(chs[k]) {
        k += 1;
    }
    let after_ws = k;
    if k >= n || !is_nl(chs[k]) {
        return None;
    }
    while k < n && is_nl(chs[k]) {
        k += 1;
    }
    if k == after_ws {
        None
    } else {
        Some(k - i)
    }
}

fn match_trailing_ws(chs: &[char], i: usize) -> Option<usize> {
    if !is_ws(chs[i]) {
        return None;
    }
    let n = chs.len();
    let mut k = i;
    while k < n && is_ws(chs[k]) {
        k += 1;
    }
    if k == n {
        Some(k - i)
    } else {
        None
    }
}

fn match_ws(chs: &[char], i: usize) -> Option<usize> {
    if !is_ws(chs[i]) {
        return None;
    }
    let n = chs.len();
    let mut k = i + 1;
    while k < n && is_ws(chs[k]) {
        k += 1;
    }
    Some(k - i)
}

pub fn tokens_for_piece(s: &str) -> u64 {
    tokens_for_piece_with(&MODELS[0], s)
}

pub fn tokens_for_piece_with(m: &ModelProfile, s: &str) -> u64 {
    if s.is_empty() {
        return 0;
    }
    if s.chars().all(is_ws) {
        return 1;
    }
    let group = m.digit_group.max(1);
    let trimmed = s.trim_start_matches(' ');
    if !trimmed.is_empty() && trimmed.chars().all(is_n) {
        let digits = trimmed.chars().count() as u64;
        return digits.div_ceil(group);
    }
    if !s.chars().any(|c| is_l(c) || is_n(c)) {
        let n = s.chars().count() as u64;
        return if n <= 2 { 1 } else { n.div_ceil(2) };
    }
    let letters = s.chars().filter(|c| is_l(*c)).count() as u64;
    let digits = s.chars().filter(|c| is_n(*c)).count() as u64;
    if letters == 0 && digits > 0 {
        return digits.div_ceil(group);
    }
    // A leading punctuation mark is a separate token for families that do not merge it.
    let split_prefix = !m.prefix_merge
        && s.chars()
            .next()
            .is_some_and(|c| !is_l(c) && !is_n(c) && c != ' ');
    let mut t = if letters <= m.word_cap {
        1 + if group == 1 {
            digits
        } else if digits > 3 {
            digits.div_ceil(group) - 1
        } else {
            0
        }
    } else {
        ((letters as f64 / m.long_chars).ceil() as u64).max(1) + digits.div_ceil(group)
    };
    if split_prefix {
        t += 1;
    }
    t
}

#[derive(Debug, Clone)]
pub struct TokenFileRow {
    pub label: String,
    pub path: String,
    pub bytes: u64,
    /// Headline (GPT-4 / cl100k) estimate.
    pub tokens: u64,
    /// Estimates for every entry of [`MODELS`], same order.
    pub by_model: Vec<u64>,
    pub excluded: bool,
}

#[derive(Debug, Clone)]
pub struct TokenReport {
    pub tokenizer: String,
    pub input: Option<TokenFileRow>,
    pub output_dataset: Option<TokenFileRow>,
    pub other: Vec<TokenFileRow>,
    pub excluded: Vec<TokenFileRow>,
}

impl TokenReport {
    pub fn total_input_tokens(&self) -> u64 {
        self.input.as_ref().map(|r| r.tokens).unwrap_or(0)
    }
    pub fn total_output_dataset_tokens(&self) -> u64 {
        self.output_dataset.as_ref().map(|r| r.tokens).unwrap_or(0)
    }
    pub fn total_other_tokens(&self) -> u64 {
        self.other.iter().map(|r| r.tokens).sum()
    }
    pub fn total_counted_tokens(&self) -> u64 {
        self.total_input_tokens() + self.total_output_dataset_tokens() + self.total_other_tokens()
    }

    /// (input, output dataset, other files) estimates for model index `i`.
    pub fn model_totals(&self, i: usize) -> (u64, u64, u64) {
        let pick = |r: &TokenFileRow| r.by_model.get(i).copied().unwrap_or(0);
        (
            self.input.as_ref().map(pick).unwrap_or(0),
            self.output_dataset.as_ref().map(pick).unwrap_or(0),
            self.other.iter().map(pick).sum(),
        )
    }
}

pub fn file_row(label: &str, path: &Path) -> Result<TokenFileRow> {
    let meta = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading {} for token count", path.display()))?;
    let by_model = count_all_models(&text);
    let tokens = by_model.first().copied().unwrap_or(0);
    Ok(TokenFileRow {
        label: label.to_string(),
        path: path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
        bytes: meta.len(),
        tokens,
        by_model,
        excluded: false,
    })
}

pub fn render_token_report(report: &TokenReport) -> String {
    let mut s = String::new();
    s.push_str("DATA OBFUSCATOR — TOKEN REPORT\n");
    s.push_str("==============================\n");
    s.push_str(&format!("Tokenizer: {}\n", report.tokenizer));
    s.push_str("Special tokens: not counted (raw file content, no BOS/EOS)\n\n");
    s.push_str(&format!(
        "{:<28} {:>12} {:>12}\n",
        "FILE", "BYTES", "TOKENS"
    ));
    s.push_str(&format!("{:-<54}\n", ""));

    fn emit(s: &mut String, section: &str, rows: &[&TokenFileRow]) {
        s.push_str(section);
        s.push('\n');
        if rows.is_empty() {
            s.push_str("  (none)\n");
            return;
        }
        for r in rows {
            s.push_str(&format!(
                "  {:<26} {:>12} {:>12}\n",
                r.path, r.bytes, r.tokens
            ));
        }
    }

    if let Some(r) = &report.input {
        emit(&mut s, "INPUT", &[r]);
        s.push('\n');
    }
    if let Some(r) = &report.output_dataset {
        emit(&mut s, "OUTPUT DATASET", &[r]);
        s.push('\n');
    }
    let other_refs: Vec<&TokenFileRow> = report.other.iter().collect();
    emit(&mut s, "OTHER OUTPUT FILES (excluding log)", &other_refs);
    s.push('\n');
    if !report.excluded.is_empty() {
        let ex: Vec<&TokenFileRow> = report.excluded.iter().collect();
        emit(&mut s, "EXCLUDED (log / not counted in totals)", &ex);
        s.push('\n');
    }
    s.push_str(&format!("{:-<54}\n", ""));
    s.push_str(&format!(
        "{:<28} {:>12} {:>12}\n",
        "TOTAL INPUT",
        report.input.as_ref().map(|r| r.bytes).unwrap_or(0),
        report.total_input_tokens()
    ));
    s.push_str(&format!(
        "{:<28} {:>12} {:>12}\n",
        "TOTAL OUTPUT DATASET",
        report
            .output_dataset
            .as_ref()
            .map(|r| r.bytes)
            .unwrap_or(0),
        report.total_output_dataset_tokens()
    ));
    s.push_str(&format!(
        "{:<28} {:>12} {:>12}\n",
        "TOTAL OTHER FILES",
        report.other.iter().map(|r| r.bytes).sum::<u64>(),
        report.total_other_tokens()
    ));
    s.push_str(&format!(
        "{:<28} {:>12} {:>12}\n",
        "TOTAL ALL COUNTED",
        report.input.as_ref().map(|r| r.bytes).unwrap_or(0)
            + report
                .output_dataset
                .as_ref()
                .map(|r| r.bytes)
                .unwrap_or(0)
            + report.other.iter().map(|r| r.bytes).sum::<u64>(),
        report.total_counted_tokens()
    ));
    s.push('\n');
    s.push_str("MULTI-MODEL ESTIMATES (approximate - same files, other tokenizers)\n");
    s.push_str(&format!(
        "{:<50} {:>10} {:>10} {:>10} {:>11}\n",
        "MODEL", "INPUT", "OUT DATA", "OTHER", "ALL COUNTED"
    ));
    s.push_str(&format!("{:-<95}\n", ""));
    for (i, m) in MODELS.iter().enumerate() {
        let (inp, outd, oth) = report.model_totals(i);
        s.push_str(&format!(
            "{:<50} {:>10} {:>10} {:>10} {:>11}\n",
            m.label,
            inp,
            outd,
            oth,
            inp + outd + oth
        ));
    }
    s.push_str("\nNotes:\n");
    for line in [
        "- Shared pretokenization follows the published cl100k_base regex rules",
        "  (contractions, letter runs, 1-3 digit groups, punctuation, newlines);",
        "  each model family is then scored with its own profile.",
        "- Digit handling dominates numeric CSV files. GPT-4/4o, Llama 3, Claude 3+",
        "  and Gemini group digits (up to 3 per token); Qwen, Mistral 7B and",
        "  Llama 2 spend one token per digit, so expect noticeably higher counts.",
        "  The grouped-digit families therefore land close together on a dataset;",
        "  they diverge more on prose such as llm.txt and summary.txt.",
        "- Treat every figure as an estimate (typically within about +-10-25 %).",
        "  For exact numbers use the provider's counter: tiktoken (OpenAI), the",
        "  count_tokens endpoint (Anthropic), countTokens (Gemini), or the model's",
        "  Hugging Face tokenizer (Llama, Qwen, Mistral).",
        "- No BOS/EOS special tokens are added (file content, not a chat turn).",
        "- The log file is excluded from totals as specified.",
    ] {
        s.push_str(line);
        s.push('\n');
    }
    s
}

pub fn write_token_report(path: &Path, report: &TokenReport) -> Result<()> {
    let body = render_token_report(report);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::File::create(path)
        .with_context(|| format!("creating token report {}", path.display()))?;
    f.write_all(body.as_bytes())?;
    f.flush()?;
    Ok(())
}

pub const TOKENIZER_NAME: &str =
    "Primary: cl100k_base pretokenization + BPE-length model (GPT-4 family); other models estimated below";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_group_by_three() {
        let p = pretokens("12345");
        assert_eq!(p, vec!["123".to_string(), "45".to_string()]);
        assert_eq!(count_tokens("12345"), 2);
    }

    #[test]
    fn csv_header_splits_like_cl100k() {
        // cl100k: optional prefix + letters consumes ",TIME" as one pretoken
        let p = pretokens("ID,TIME,DV");
        assert_eq!(p, vec!["ID", ",TIME", ",DV"]);
        assert_eq!(count_tokens("ID,TIME,DV"), 3);
    }

    #[test]
    fn single_digit_families_cost_more_on_numbers() {
        let csv = "101,0.287,12.5,0.756\n";
        let all = count_all_models(csv);
        assert_eq!(all.len(), MODELS.len());
        let gpt = all[0];
        let qwen = all[MODELS.iter().position(|m| m.id == "qwen").unwrap()];
        assert!(qwen > gpt, "one token per digit must exceed 3-digit grouping");
    }

    #[test]
    fn primary_model_matches_headline_count() {
        let t = "ID,TIME,DV\n1,0.5,12.25\n";
        assert_eq!(count_all_models(t)[0], count_tokens(t));
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(count_tokens(""), 0);
    }
}
