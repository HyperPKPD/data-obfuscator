# Examples

Everything in this folder is **simulated**. No real study, subject, site, drug or person is behind any file.

Program: data-obfuscator | Author: Obinna Obianom | Written for version 0.0.0.3 | Date: 2026-10-08

Use these files to try the program, to see what masked output looks like, and as templates for your own
data-quality (QC) test sets.

## Contents

| Path | What it is |
|---|---|
| [`datasets/qc_nearly_clean.csv`](datasets/qc_nearly_clean.csv) | **QC 1.** Oral PK, 12 subjects. Reads into NONMEM; 6 items a reviewer would still query. |
| [`datasets/qc_many_errors.csv`](datasets/qc_many_errors.csv) | **QC 2.** Same layout, 10 subjects, **36 planted defects**: format errors, wrong dose/observation coding, broken time, wrong AMT. |
| [`datasets/qc_answer_key.md`](datasets/qc_answer_key.md) | Every planted defect with its file line number and how to detect it. |
| [`datasets/pk_iv_infusion_addl_ss.csv`](datasets/pk_iv_infusion_addl_ss.csv) | IV infusion with `RATE`, `ADDL`/`II` and steady-state (`SS`) dosing, 18 subjects, 3 cohorts. |
| [`datasets/pk_oral_pooled_multi_id.csv`](datasets/pk_oral_pooled_multi_id.csv) | Three studies pooled, BLQ coded, and **several columns per role** (`STU`/`STUDYID`/`STUDY`, `USUBJID`/`SUBJID`, ...). |
| [`datasets/pkpd_dvid_endpoints.tsv`](datasets/pkpd_dvid_endpoints.tsv) | **Tab-delimited** PK/PD: two endpoints told apart by `DVID`, 12 subjects. |
| [`datasets/make_datasets.py`](datasets/make_datasets.py) | The script that builds the five datasets and the answer key (Python standard library only). |
| [`sample.csv`](sample.csv), [`sample_nonmem.dat`](sample_nonmem.dat) | The original small examples (CSV with header; whitespace NONMEM file without header). |
| [`configs/`](configs) | Ready-made option files, numbered. `00`-`27` run on `sample.csv`; `30`-`36` run on the datasets above (`35` and `36` show column renaming). |
| [`config.txt`](config.txt) | Older single config. Run it from inside this folder. |

---


## 1. Quality-check datasets

### How these two files were designed

When a pharmacometric dataset is checked before modelling, the questions come in a natural order. The two QC
files, the answer key and the QC prompt in `llm.txt` follow that same order:

| Step | Question | Answer-key codes | Examples of what goes wrong |
|---|---|---|---|
| 1 | **Will NONMEM read it at all?** | `A` | text such as `BQL`, `NA`, `<0.5` in a numeric column; a row with too few/many fields; a decimal comma in a CSV; a blank `ID`; `ID` blocks that are not contiguous; `TIME` going backwards |
| 2 | **Are dosing and observation records coded correctly?** | `B` | a dose with `EVID=0`; an observation with `EVID=1`; dose with missing/zero `AMT`; observation with missing `DV` but `MDV=0`; valid `DV` with `MDV=1`; wrong `CMT`; a subject with no dose record; a double-entered dose; a pre-dose sample with a concentration |
| 3 | **Is the date/time translated correctly into `TIME`?** | `C` | `TIME` in days instead of hours; `TIME` counted from midnight instead of from the first dose; a date typo; a missed day rollover after midnight; mixed `DD/MM/YYYY` and `YYYY-MM-DD` |
| 4 | **Does `AMT` match the intended `DOSE` on dosing rows?** | `D` | `AMT=0.1` for a 200 mg dose (unit slip); `AMT=1000` (extra zero); `DOSE` changing within a subject; negative `AMT` |
| 5 | **Everything else** | `E` and `W` | covariates out of range (`WT=0`, `WT=700`, `AGE=0`, negative `CRCL`) or text (`SEX=M`); `ADDL>0` with `II=0`; `SS=1` with `II=0`; duplicate observations; `DV=0` that should be BLQ; wrong `TAD`; a sample drawn far from its nominal time |

Both files use the same 24 columns so you can compare them side by side:

```
ID USUBJID STUDYID SITEID ARM DATE CLOCK NTIM TIME TAD EVID MDV CMT AMT RATE ADDL II SS DOSE DV WT AGE SEX CRCL
```

`DATE` + `CLOCK` are the calendar date and time of day of each record; `TIME` is hours since the subject's first
dose; `NTIM` is the nominal (protocol) time; `DOSE` is the planned dose in mg; `AMT` is what was given.
Dose records are `EVID=1, MDV=1, CMT=1` (depot); observations are `EVID=0, MDV=0, CMT=2` (central). Concentrations
are mg/L. Each subject receives two daily oral doses and 14 samples.

A matching NONMEM header (text columns dropped):

```
$INPUT ID USUBJID=DROP STUDYID=DROP SITEID=DROP ARM=DROP DATE=DROP CLOCK=DROP
       NTIM TIME TAD EVID MDV CMT AMT RATE ADDL II SS DOSE DV WT AGE SEX CRCL
$DATA  obfuscated.csv IGNORE=@
```

The masked file has the same column names, so the same `$INPUT` works on it.

### 1a. `qc_nearly_clean.csv` - few warnings

193 records, 12 subjects. It should read into NONMEM. Six things are deliberately left for a reviewer
(`W1`-`W6` in the answer key): a late sample, a documented partial dose (`AMT` 150 vs `DOSE` 200), a
subject with missing `WT`, a `DV=0` that was never coded as BLQ, a duplicate observation, and one wrong `TAD`.
Everything else is consistent: `TIME` equals the elapsed time implied by `DATE`+`CLOCK`, `AMT` equals `DOSE` on
every dose row, records are in time order and each subject is one block.

```
data-obfuscator run --config examples/configs/30-qc-nearly-clean.txt
```

Same thing without a config file:

```
data-obfuscator run --input examples/datasets/qc_nearly_clean.csv --output ./masked/qc-nearly-clean --level 2 --date-columns DATE
```

What to expect: the run completes with **no warnings**. `keymap.txt` is written by default; it can reverse the
masking, so keep it on your computer. Add `--write-keymap false` if you do not want it.

Next: open `masked/qc-nearly-clean/llm.txt`, paste prompt **3. PERFORM QUALITY CONTROL CHECKS** with
`obfuscated.csv`, and compare the model's findings table with `W1`-`W6`. A good reviewer finds all six, does
not invent a seventh, and treats `W2` as a *query*, not an error.

### 1b. `qc_many_errors.csv` - many errors

160 records, 10 subjects, **36 planted defects** (9 readability, 11 dose/observation coding, 5 time translation,
4 AMT/DOSE, 7 covariate and regimen). Several would stop NM-TRAN outright; the most dangerous ones are
*silent* - NONMEM reads them happily and the model is fed wrong information (for example a trough with
`MDV=1`, a first dose sent to the wrong compartment, or `TIME` in days for one subject).

```
data-obfuscator run --config examples/configs/31-qc-many-errors.txt
```

Without a config file:

```
data-obfuscator run --input examples/datasets/qc_many_errors.csv --output ./masked/qc-many-errors --level 2 --date-columns DATE
```

What to expect - **two warnings**:

1. `date column DATE: 4 value(s) are not in a recognised date format and were left UNSHIFTED` - the
   `DD/MM/YYYY` dates (defect `C5`). The program refuses to guess day/month order, so those four are still the
   *real* dates. Fix or drop them before sharing the file outside your organisation.
2. `2 data row(s) do not have 24 fields like the header` - defects `A4` and `A5`.

How masking treats the defects, so that QC on the masked copy is still meaningful:

- `DV`, `TIME`, `EVID`, `MDV`, `CMT`, `AMT`, `ADDL`, `II`, `SS`, `TAD` are never altered, so every coding and time
  defect survives masking exactly.
- Rows with the wrong number of fields **keep that shape** in the masked file (they are not silently repaired),
  and the row count and order are unchanged, so the line numbers in `qc_answer_key.md` apply to the masked
  file as well. Fields beyond the end of the header are copied unmasked; that is why the warning exists.
- Dates are shifted by one offset for the whole file (a random decimal number of days, 1-100 by default; the
  exact value is in `keymap.txt`). `DATE` holds a date only, so it moves by the whole days and `CLOCK` is left
  alone: `DATE`+`CLOCK` still reproduce `TIME` (and defects `C1`-`C4` are still visible), but the real
  calendar dates are gone. Use `--date-shift-days 23.6` to fix the shift yourself.
- `SEX` labels are recoded together, so the letter `M` (defect `E4`) shows up as a code outside 0/1 rather
  than as `M`. Still detectable, just not identical.

Score yourself (or the LLM) against [`datasets/qc_answer_key.md`](datasets/qc_answer_key.md): one row per
defect with severity (`ERROR` stops NONMEM, `SILENT` reads but is wrong), file line, and the rule that finds it.
A useful measure is *which severity classes were missed*, not just the total.

### 1c. Building your own QC test set

The recipe `make_datasets.py` uses, which you can copy for your own data:

1. **Simulate a clean truth first.** Subjects, covariates, dosing times, then concentrations from a simple model
   (`oral_conc`, `iv_conc` in the script). A defect only means something against a known-good baseline.
2. **Lay it out the NONMEM way.** One record per event; dose record before an observation at the same time
   unless the observation is a pre-dose trough; `TIME` non-decreasing within `ID`; every row numeric in the
   numeric columns.
3. **Derive the redundant columns from the same truth** (`DATE`, `CLOCK`, `TIME`, `NTIM`, `TAD`, `DOSE`, `AMT`).
   QC works by cross-checking redundant columns, so they must agree before you break them.
4. **Plant one defect at a time**, one per subject where possible, and write down code, row, severity and the
   detection rule at the moment you plant it (the `Key.add(...)` calls). Do not reconstruct the key afterwards.
5. **Plant a mix of loud and silent defects.** Loud ones (text in `DV`, missing fields) test whether the file
   is read; silent ones (`MDV=1` on a valid sample, `TIME` in days) test whether the review thinks.
6. **Keep a nearly clean twin.** A reviewer that cannot say "this dataset is fine apart from these six
   items" is as unhelpful as one that misses errors.
7. **Mask it and check the defects survived** (`tests/qc_examples.rs` does this for the files here).

Rerun `python examples/datasets/make_datasets.py` to rebuild the five datasets and the key. The seeds are fixed,
so the output is byte-identical every time.

---

## 2. `pk_iv_infusion_addl_ss.csv` - infusion, ADDL/II and steady state

162 records, 18 subjects in three cohorts of six. Two-compartment IV model, 1-hour infusions, concentrations in
mg/L, all in `CMT=1`.

| Cohort | Regimen | How it is coded |
|---|---|---|
| A | 50 mg once | one dose row: `AMT=50 RATE=50` (50 mg over 1 h) |
| B | 100 mg every 24 h, 5 doses | one dose row: `AMT=100 RATE=100 ADDL=4 II=24`; troughs at 23.9 h and 95.9 h |
| C | 200 mg every 12 h at steady state | one dose row: `AMT=200 RATE=200 SS=1 II=12`; samples across one interval |

`.` marks "not applicable" (for example `DV` on dose rows, `AMT`/`RATE` on observation rows).

```
data-obfuscator run --config examples/configs/32-iv-infusion-nonmem.txt
```

Without a config file: `data-obfuscator run -i examples/datasets/pk_iv_infusion_addl_ss.csv -o ./masked/iv-infusion --level 2`

Expected: no warnings. `RATE`, `ADDL`, `II` and `SS` are protected, so the dosing logic the LLM sees is the
real one, and `llm.txt` mentions zero-order infusions, repeated doses and steady-state flags. Paste prompt
**2. CREATE A NONMEM CONTROL STREAM**: a good answer uses `ADVAN3 TRANS4`, reads `RATE` from the data,
and does not try to add its own doses for cohort B.

---

## 3. `pk_oral_pooled_multi_id.csv` - several columns per role

288 records, 24 subjects from three studies. First-order oral absorption; pre-dose samples and late samples
below the limit of quantification are coded `BLQ=1` with `DV=LLOQ` (0.05), the NONMEM "M3" convention.

This file describes the same thing in several ways, as pooled extracts often do:

| Role | Columns | Example values |
|---|---|---|
| subject | `USUBJID`, `SUBJID` | `XR-101-S01-1001`, `1001` |
| study | `STU`, `STUDYID`, `STUDY`, `PROTNAME` | `1`, `XR-101`, `Phase 1 SAD`, `Single ascending dose of XR-7 in healthy adults` |
| site | `SITEID`, `SITENAME` | `S01`, `Harbor Point Research Unit` |
| treatment | `ARM`, `ARMCD` | `50mg_single`, `101D50` |
| NONMEM id | `ID` | `1` |

The program recognises many of these names by itself (`USUBJID`, `STUDYID`, `STU`, `SITEID`, `ARM`, ...), but
not `PROTNAME` or `SITENAME`. **Run it without telling it anything and it says so:**

```
data-obfuscator run -i examples/datasets/pk_oral_pooled_multi_id.csv -o ./masked/pooled-bare
```

```
[WARN] text column(s) not recognised and passed through UNCHANGED: PROTNAME, SITENAME - if they identify a
       study, site, person or sample, name them in sensitive / freetext_columns / a role option ...
```

Now name every column that belongs to each role. Each role option takes **one or several** column names:

```
data-obfuscator run --config examples/configs/33-pooled-multi-column-roles.txt
```

Without a config file (comma-separated, or repeat the flag):

```
data-obfuscator run -i examples/datasets/pk_oral_pooled_multi_id.csv -o ./masked/pooled-multi-column --level 3 --seed 2024 --id-column ID --subject-column USUBJID,SUBJID --study-column STU,STUDYID,STUDY,PROTNAME --site-column SITEID,SITENAME --treatment-column ARM,ARMCD --user-column INVESTIGATOR --date-columns DATE --freetext-columns COMMENT
```

The config-file version is identical to the command line:

```
study_column     = STU, STUDYID, STUDY, PROTNAME    # several names: comma, semicolon or pipe
site_column      = SITEID, SITENAME
subject_column   = USUBJID, SUBJID
treatment_column = ARM, ARMCD
```

Repeating a key (`study_column = STU` then `study_column = STUDYID`) adds to the list, and so do the alias keys
(`study`, `site`, `arm`, ...). On the command line, a role flag replaces whatever the config file said for that
role.

What the program does with several columns of one role:

- **Every listed column is masked** with that role's rules.
- **Columns that describe the same thing share one label.** If the values of two columns of the same role pair
  up one-to-one, row by row (each `STUDYID` value always appears with exactly one `STU` value and vice versa),
  both get the same masked label. After the run `STU`, `STUDYID`, `STUDY` and `PROTNAME` hold the same code
  (`STU001`, `STU002`, `STU003`), `USUBJID` and `SUBJID` hold the same subject code, and `SITEID` and
  `SITENAME` the same site code. Open `obfuscator.log` and look for "reuse the labels of the other study
  column(s)".
- **Columns that do not pair up get their own labels** (the log says how many values), so a coarse column can
  never be merged into a finer one by accident.
- A column name that is not in the file is reported as a warning and ignored.

Expected: no warnings. Level 3 multiplies or divides every number in `WT`, `AGE`, `CRCL`, `ALB` by one hidden
factor between 1 and 20 per column (`keymap.txt` lists the operation and factor of each; `seed` makes them
repeatable; `--scale-range` changes the range). `DV`, `AMT`, `TIME`, `BLQ` and `LLOQ` stay exact.

This file is a data-management-style extract with free-text columns, not a NONMEM-ready file; drop the text
columns in `$INPUT` (or `--drop`) before modelling.

---

## 4. `pkpd_dvid_endpoints.tsv` - tab-delimited PK/PD

216 records, 12 subjects, single oral dose of 50, 100 or 200 mg. Two endpoints share the `DV` column and are
separated by `DVID`:

| `DVID` | Endpoint | `CMT` | Times (h) |
|---|---|---|---|
| 1 | plasma concentration (mg/L) | 2 | 0.5 to 48 |
| 2 | biomarker response (% of reference, baseline in `BASE`) | 3 | 0 (baseline) to 96 |

Dose rows have `DVID=0`. `SAMPLEID` carries the subject number inside a text label, so it is declared as an
identifier. The delimiter (tab) is detected automatically and the output is written as `obfuscated.tsv`.

```
data-obfuscator run --config examples/configs/34-pkpd-tab-delimited.txt
```

Without a config file: `data-obfuscator run -i examples/datasets/pkpd_dvid_endpoints.tsv -o ./masked/pkpd-tsv --sensitive SAMPLEID --freetext-columns COMMENT --date-columns DATE`

Expected: no warnings. `llm.txt` lists both endpoints under "Endpoints (DVID)" and the prompts tell the model
not to pool them (analysis), to use a separate residual error per `DVID` (control stream), and to check that
each `DVID` maps to one `CMT` (QC). Leave out `--sensitive SAMPLEID` to see the unrecognised-text warning.

NONMEM reads comma- or space-separated files; convert the `.tsv` to CSV first if you want to fit it. This file
is mainly here to show tab-delimited input and multi-endpoint handling.

---

## 5. The original examples and the numbered configs

| Config | Purpose | Input |
|---|---|---|
| `00-minimal` | the smallest working config | `sample.csv` |
| `01-full` | every option, annotated | `sample.csv` |
| `10`-`13` | masking levels 1 to 4 | `sample.csv` |
| `20-purpose-qc` | masking tuned for QC | `sample.csv` |
| `21-purpose-nonmem` | NONMEM code generation | `sample.csv` |
| `22-purpose-mrgsolve` | mrgsolve code generation | `sample.csv` |
| `23-purpose-eda-nca` | exploration / NCA | `sample.csv` |
| `24-purpose-llm-small-context` | smallest token footprint | `sample.csv` |
| `25-purpose-external-share` | most conservative | `sample.csv` |
| `26-purpose-reproducible-audit` | fixed seed, audit trail | `sample.csv` |
| `27-purpose-whitespace-nonmem-nohdr` | whitespace file without a header | `sample_nonmem.dat` |
| `30-qc-nearly-clean` | QC, few warnings | `datasets/qc_nearly_clean.csv` |
| `31-qc-many-errors` | QC, many errors | `datasets/qc_many_errors.csv` |
| `32-iv-infusion-nonmem` | infusion / ADDL / SS control stream | `datasets/pk_iv_infusion_addl_ss.csv` |
| `33-pooled-multi-column-roles` | several columns per role | `datasets/pk_oral_pooled_multi_id.csv` |
| `34-pkpd-tab-delimited` | tab file, two endpoints | `datasets/pkpd_dvid_endpoints.tsv` |
| `35-rename-listed-columns` | rename the columns you list (`PROTNAME`, `SITENAME`, `INVESTIGATOR`, `WT=BODYSIZE`) | `datasets/pk_oral_pooled_multi_id.csv` |
| `36-rename-all-except` | rename every column except a keep list | `datasets/pk_oral_pooled_multi_id.csv` |

**Renaming columns.** Configs `35` and `36` run on the pooled dataset and rename column *names* (the cells are
masked as usual). Open `masked/rename-listed/keymap.txt` afterwards: section 1, `COLUMN RENAMING`, lists every
original name and its new name. Without a config file:

```
data-obfuscator run -i examples/datasets/pk_oral_pooled_multi_id.csv -o ./masked/rename-cli --rename-columns PROTNAME,SITENAME,WT=BODYSIZE
data-obfuscator run -i examples/datasets/pk_oral_pooled_multi_id.csv -o ./masked/rename-cli2 --rename-except ID,TIME,DV,AMT,EVID,MDV,CMT
```

Every config starts with a commented header (program, version, author, date, purpose, how to run it); the
parser ignores every `#` and `//` line. Command-line flags override anything in a config:

```
data-obfuscator run --config examples/configs/30-qc-nearly-clean.txt --level 3 --seed 7
```

`sample.csv` and `sample_nonmem.dat`:

```
data-obfuscator run --config examples/configs/00-minimal.txt
data-obfuscator run --config examples/configs/27-purpose-whitespace-nonmem-nohdr.txt
```

---

## 6. Things worth knowing

- **Reproducible.** The same input, options, seed and program version give a byte-identical masked dataset.
  `summary.txt` records the dataset's SHA-256. Compare two runs with `fc /b a.csv b.csv` (Windows) or `cmp`.
- **Never share** `keymap.txt`, `summary.txt`, `config.used.txt` or `obfuscator.log`: they reveal the seed, the
  date shift or file paths. Share `obfuscated.*` and `llm.txt` only.
- **Masking is not formal anonymisation.** It lowers re-identification risk; review the warnings before sending
  data outside your organisation.
- **The severity labels** in the answer key (`WARN`, `ERROR`, `SILENT`) are the author's classification of
  typical behaviour. Exact NM-TRAN/NONMEM messages depend on your NONMEM version and your `$INPUT`/`$PK` code.
- **Compact dates.** Dates written as `YYYYMMDD` (for example `20240325`) are shifted as real dates and stay
  compact (`20240404` after +10 days). Formats the program cannot read, such as `DD/MM/YYYY`, are left
  unshifted and reported as a warning.
