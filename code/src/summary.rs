//! Human-readable run summary (`summary.txt`).

#[derive(Clone, Debug)]
pub struct Summary {
    pub generated: String,
    pub program: String,
    pub command: String,
    pub input_file: String,
    pub input_format: String,
    pub input_rows: usize,
    pub input_cols: usize,
    pub input_subjects: usize,
    pub input_bytes: u64,
    pub output_file: String,
    pub output_rows: usize,
    pub output_cols: usize,
    pub output_subjects: usize,
    pub output_sha256: String,
    pub level: u8,
    pub level_name: String,
    pub seed: u64,
    pub id_style: String,
    pub recoded: Vec<String>,
    pub dropped: Vec<String>,
    pub kept: Vec<String>,
    pub protected: Vec<String>,
    pub date_shift: f64,
    /// (original name, new name) of every renamed column.
    pub renamed: Vec<(String, String)>,
    pub scaled_columns: Vec<String>,
    pub scale_range: (f64, f64),
    pub keymap: bool,
    pub status: String,
}

pub fn render(s: &Summary) -> String {
    let mut o = String::new();
    o.push_str("DATA OBFUSCATOR — RUN SUMMARY\n");
    o.push_str("=============================\n");
    o.push_str(&format!("Generated: {}\n", s.generated));
    o.push_str(&format!("Program:   {}\n", s.program));
    o.push_str(&format!("Command:   {}\n\n", s.command));

    o.push_str("INPUT\n");
    o.push_str(&format!("  File:            {}\n", s.input_file));
    o.push_str(&format!("  Format:          {}\n", s.input_format));
    o.push_str(&format!("  Rows:            {}\n", s.input_rows));
    o.push_str(&format!("  Columns (in):    {}\n", s.input_cols));
    o.push_str(&format!("  Subjects (ID):   {}\n", s.input_subjects));
    o.push_str(&format!("  Bytes:           {}\n\n", s.input_bytes));

    o.push_str("OUTPUT\n");
    o.push_str(&format!("  File:            {}\n", s.output_file));
    o.push_str(&format!("  Rows:            {}\n", s.output_rows));
    o.push_str(&format!("  Columns (out):   {}\n", s.output_cols));
    o.push_str(&format!("  Subjects (ID):   {}\n", s.output_subjects));
    o.push_str(&format!("  SHA-256:         {}\n\n", s.output_sha256));

    o.push_str("OBFUSCATION\n");
    o.push_str(&format!("  Level:           {} ({})\n", s.level, s.level_name));
    o.push_str(&format!("  Seed:            {}\n", s.seed));
    o.push_str(&format!("  ID style:        {}\n", s.id_style));
    o.push_str(&format!(
        "  Columns recoded: {}\n",
        if s.recoded.is_empty() {
            "(none)".into()
        } else {
            s.recoded.join(", ")
        }
    ));
    o.push_str(&format!(
        "  Columns dropped: {}\n",
        if s.dropped.is_empty() {
            "(none)".into()
        } else {
            s.dropped.join(", ")
        }
    ));
    o.push_str(&format!("  Columns kept:    {}\n", s.kept.join(", ")));
    if !s.renamed.is_empty() {
        o.push_str(&format!(
            "  Renamed columns: {}\n",
            s.renamed
                .iter()
                .map(|(a, b)| format!("{a} -> {b}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    o.push_str(&format!(
        "  Date shift:      {} days (one decimal value for the whole dataset; intervals preserved; exact value in keymap.txt)\n",
        crate::obfuscate::fmt_days(s.date_shift)
    ));
    o.push_str(&format!(
        "  Scaling:         {}\n",
        if s.scaled_columns.is_empty() {
            "off".into()
        } else {
            format!(
                "{} multiplied or divided by a hidden factor in {}-{} (factors are in keymap.txt)",
                s.scaled_columns.join(", "),
                s.scale_range.0,
                s.scale_range.1
            )
        }
    ));
    o.push_str(&format!(
        "  Keymap written:  {}\n\n",
        if s.keymap {
            "yes — DO NOT share keymap.txt with any LLM or external party"
        } else {
            "no"
        }
    ));

    o.push_str("REPRODUCIBILITY\n");
    o.push_str("  Same input file + same options + same seed + same program version gives\n");
    o.push_str("  a byte-identical dataset. Compare the SHA-256 above between runs to check.\n\n");

    o.push_str("INTEGRITY\n");
    o.push_str("  Within-ID time order preserved: yes\n");
    o.push_str(&format!(
        "  Protected columns untouched:    {}\n",
        if s.protected.is_empty() {
            "(none present)".into()
        } else {
            s.protected.join(", ")
        }
    ));
    o.push_str(&format!(
        "  Unique ID count unchanged:      {}\n\n",
        if s.input_subjects == s.output_subjects {
            "yes"
        } else {
            "no"
        }
    ));

    o.push_str("TOKEN COUNTS\n");
    o.push_str("  See tokens.txt for estimated token counts (GPT, Claude, Gemini, Llama,\n");
    o.push_str("  Qwen, Mistral) of the input file, the output dataset, and every other\n");
    o.push_str("  file in the output directory except the log.\n\n");
    o.push_str("LLM PROMPTS\n");
    o.push_str("  See llm.txt for ready-to-paste prompts (analysis, NONMEM control\n");
    o.push_str("  stream, QC) tailored to the masked dataset.\n\n");
    o.push_str(&format!("STATUS: {}\n", s.status));
    o
}
