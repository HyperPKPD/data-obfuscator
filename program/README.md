# How to easy run the program

A quick, step-by-step guide to running the ready-made program on **Windows** or **Linux**. You do not need to install
anything, build anything, or be online.

*Written and tested with version 0.0.0.23. Every command and result below was produced by running the program.*

**Real data should not be shared with LLM not hosted in your organization.** This program masks a NONMEM-style
pharmacometric dataset on your own computer, so the masked copy can go to an LLM or a collaborator while the real
values stay with you. It can also put the real values back afterwards.

---

## Contents

1. [What is in the folder](#1-what-is-in-the-folder)
2. [Set up on Windows](#2-set-up-on-windows)
3. [Set up on Linux](#3-set-up-on-linux)
4. [Make a practice dataset](#4-make-a-practice-dataset)
5. [Your first run](#5-your-first-run)
6. [The four masking levels, with examples](#6-the-four-masking-levels-with-examples)
7. [The commands: run, tokens, reverse, run2](#7-the-commands)
8. [Handy options for run](#8-handy-options-for-run)
9. [Options files](#9-options-files)
10. [Count tokens: tokens](#10-count-tokens-tokens)
11. [Undo the masking: reverse](#11-undo-the-masking-reverse)
12. [Mask several datasets together: run2](#12-mask-several-datasets-together-run2)
13. [Cheat sheet](#13-cheat-sheet)
14. [If something goes wrong](#14-if-something-goes-wrong)
15. [Keep it safe](#15-keep-it-safe)
16. [Author, license and citation](#16-author-license-and-citation)

---

## 1. What is in the folder

```text
data-obfuscator-release/
├── RELEASE_USAGE.md        <- this guide
├── windows/
│   └── data-obfuscator.exe   <- for Windows 10 / 11 (64-bit)
└── linux/
    └── data-obfuscator       <- for 64-bit Linux (x86-64)
```

Use **one** of the two program folders: `windows` on a Windows computer, `linux` on a Linux computer. The program is a
single file. It reads your dataset from a file on your computer and writes the results into a new folder on your
computer. It never connects to the internet.

**Good to know before you start**

- Copy the whole folder to a normal place on your own drive (for example `Documents`), not a network drive or a
  read-only location.
- The program works on text tables: CSV, TSV, semicolon-separated or whitespace-separated files, with or without a header.
- Every command in this guide is **the same on Windows and Linux** once the setup in sections 2 or 3 is done.

---

## 2. Set up on Windows

**Step 1. Open PowerShell inside the `windows` folder.** In File Explorer, open the `windows` folder, click the address
bar, type `powershell` and press Enter.

**Step 2. If Windows warns you about the file.** Because the program came from someone else, Windows may show *"Windows
protected your PC"* the first time you run it. Click **More info**, then **Run anyway**. To avoid the warning, unblock
the file once:

```powershell
Unblock-File .\data-obfuscator.exe
```

**Step 3. Check that it starts:**

```powershell
.\data-obfuscator.exe --version
```

You should see a banner that begins with `DATA OBFUSCATOR` and a version number such as `v0.0.0.23`.

**Step 4. Make `data-obfuscator` easy to type, and create a work folder.** These three lines let you type just
`data-obfuscator` (without `.\` and `.exe`) in this PowerShell window, and keep your practice files in a separate
folder:

```powershell
$env:Path = "$PWD;$env:Path"
mkdir work
cd work
```

Check:

```powershell
data-obfuscator --version
```

> The `$env:Path` line only lasts as long as this window. When you open a new PowerShell window, repeat Step 4
> (go to the `windows` folder first). If you prefer not to change anything, type `..\data-obfuscator.exe` instead of
> `data-obfuscator` in every command below while you are inside `work`.
>
> The `cmd.exe` equivalent of the PATH line is `set PATH=%CD%;%PATH%`.

---

## 3. Set up on Linux

**Step 1. Open a terminal inside the `linux` folder:**

```bash
cd path/to/linux
```

**Step 2. Make the program executable** (files that arrive in a zip, an e-mail or a download often lose this):

```bash
chmod +x data-obfuscator
```

**Step 3. Check that it starts:**

```bash
./data-obfuscator --version
```

You should see a banner that begins with `DATA OBFUSCATOR` and a version number such as `v0.0.0.23`.

**Step 4. Make `data-obfuscator` easy to type, and create a work folder:**

```bash
export PATH="$PWD:$PATH"
mkdir work
cd work
```

Check:

```bash
data-obfuscator --version
```

> The `export PATH` line only lasts as long as this terminal window. In a new terminal, repeat Step 4 (go to the
> `linux` folder first). If you prefer not to change anything, type `../data-obfuscator` instead of
> `data-obfuscator` in every command below while you are inside `work`.

---

## 4. Make a practice dataset

You need a data file to try the program on. Here is a small NONMEM-style one: 3 subjects, 15 rows, with the
identifying columns a real study has (subject, study, site, treatment, dates, a comment and an e-mail address).

**Save the block below as `sample.csv` inside the `work` folder.**

```text
ID,USUBJID,STUDYID,SITEID,ARM,TIME,AMT,DV,EVID,MDV,CMT,WT,AGE,SEX,RACE,DATE,COMMENT,EMAIL
1,ABC-101-0001,ABC-101,0142,Drug 50 mg,0,50,.,1,1,1,72.5,34,M,WHITE,2025-03-10,screened by Dr Lee,a.jones@clinic-one.org
1,ABC-101-0001,ABC-101,0142,Drug 50 mg,1,0,12.4,0,0,2,72.5,34,M,WHITE,2025-03-10,.,.
1,ABC-101-0001,ABC-101,0142,Drug 50 mg,2,0,18.9,0,0,2,72.5,34,M,WHITE,2025-03-10,.,.
1,ABC-101-0001,ABC-101,0142,Drug 50 mg,4,0,14.2,0,0,2,72.5,34,M,WHITE,2025-03-10,.,.
1,ABC-101-0001,ABC-101,0142,Drug 50 mg,8,0,6.1,0,0,2,72.5,34,M,WHITE,2025-03-10,.,.
2,ABC-101-0002,ABC-101,0142,Drug 100 mg,0,100,.,1,1,1,88.1,51,F,ASIAN,2025-03-12,patient called the site,b.smith@clinic-one.org
2,ABC-101-0002,ABC-101,0142,Drug 100 mg,1,0,25.7,0,0,2,88.1,51,F,ASIAN,2025-03-12,.,.
2,ABC-101-0002,ABC-101,0142,Drug 100 mg,2,0,37.2,0,0,2,88.1,51,F,ASIAN,2025-03-12,.,.
2,ABC-101-0002,ABC-101,0142,Drug 100 mg,4,0,28.8,0,0,2,88.1,51,F,ASIAN,2025-03-12,.,.
2,ABC-101-0002,ABC-101,0142,Drug 100 mg,8,0,12.5,0,0,2,88.1,51,F,ASIAN,2025-03-12,.,.
3,ABC-101-0003,ABC-101,0275,Placebo,0,0,.,1,1,1,65.3,43,F,BLACK,2025-03-14,visit moved by request,c.diaz@clinic-two.org
3,ABC-101-0003,ABC-101,0275,Placebo,1,0,0,0,0,2,65.3,43,F,BLACK,2025-03-14,.,.
3,ABC-101-0003,ABC-101,0275,Placebo,2,0,0,0,0,2,65.3,43,F,BLACK,2025-03-14,.,.
3,ABC-101-0003,ABC-101,0275,Placebo,4,0,0,0,0,2,65.3,43,F,BLACK,2025-03-14,.,.
3,ABC-101-0003,ABC-101,0275,Placebo,8,0,0,0,0,2,65.3,43,F,BLACK,2025-03-14,.,.
```

- **Windows:** open Notepad, paste, then *File > Save as*, set **Save as type** to **All files**, name it `sample.csv`
  and save it in `work`. (Without "All files", Notepad saves `sample.csv.txt`, which will not be found.)
- **Linux:** `nano sample.csv`, paste, save with Ctrl+O and Enter, leave with Ctrl+X.

Check that the file is there:

```text
dir          (Windows)
ls           (Linux)
```

> **Your own data.** Everything in this guide works the same on your own file. Put it in `work` (or give its full
> path with `--input`) and use its name instead of `sample.csv`.

**What the columns are.** `TIME`, `AMT`, `DV`, `EVID`, `MDV` and `CMT` are the PK/PD structure: the program **never
changes them**. `ID`, `USUBJID`, `STUDYID`, `SITEID` and `ARM` identify subjects, study, site and treatment. `SEX` and
`RACE` are labels, `DATE` is a date, `COMMENT` and `EMAIL` are free text, and `WT` and `AGE` are covariates.

---

## 5. Your first run

Make a masked copy at the default level (level 2). The `--seed 2026` makes the result repeatable: with the same seed you
will get exactly the values shown in this guide.

```text
data-obfuscator run --input sample.csv --output masked_level2 --level 2 --seed 2026
```

The program prints one line per step (timestamps shortened here):

```text
[OK]   read 15 rows × 18 cols (CSV (comma)) from sample.csv
[INFO] date shift (dataset-wide, intervals preserved): 50.8 days
[OK]   recoded 3 unique values in column ID (id)
[OK]   recoded 3 unique values in column USUBJID (subject)
[OK]   recoded 1 unique values in column STUDYID (study)
[OK]   recoded 2 unique values in column SITEID (site)
[OK]   recoded 3 unique values in column ARM (treatment)
[OK]   recoded 2 unique values in column SEX (sex)
[OK]   recoded 3 unique values in column RACE (race)
[OK]   wrote dataset masked_level2/obfuscated.csv (15 rows × 18 cols)
...
[OK] Run complete. Files written:
```

| Tag | Meaning |
|---|---|
| `[OK]` | a step succeeded |
| `[INFO]` | a fact, for example the date shift |
| `[WARN]` | something to look at before you share the result |
| `[ERROR]` | the run stopped (a bad option stops it before anything is written) |

### What is in the output folder

Open `masked_level2`:

| File | What it is | Share it? |
|---|---|---|
| `obfuscated.csv` | **the masked dataset** | **yes** |
| `llm.txt` | ready-to-paste prompts for an LLM (analysis, NONMEM control stream, QC), written for this dataset | **yes** |
| `keymap.txt` | **the private key** that turns the masked data back into the real data | **never** |
| `keymap_columns.txt` | (level 4 only) the private per-value factors | **never** |
| `summary.txt` | the run on one page: level, seed, date shift, integrity checks | no |
| `column_report.txt` | what the program decided for each column | no |
| `config.used.txt` | the options that were used | no |
| `tokens.txt` | how many tokens the files use for GPT, Claude, Gemini and others | for you |
| `obfuscator.log` | the log of the run | no |

Compare the masked dataset with your original:

```text
original : 1,ABC-101-0001,ABC-101,0142,Drug 50 mg,0,50,.,1,1,1,72.5,34,M,WHITE,2025-03-10,screened by Dr Lee,a.jones@clinic-one.org
level 2  : 1,SUBJ001,STU001,SITE001,ARM_A,0,50,.,1,1,1,72.5,34,0,CAT001,2025-04-29,.,.
```

The identifiers, labels and date changed; the free text was removed; the dosing and observation columns
(`0,50,.,1,1,1`) are exactly the same.

**The three-step habit:** read the `[WARN]` lines, check `obfuscated.csv`, then share **only** `obfuscated.csv` and
`llm.txt`.

> **The output folder must be new (or empty).** If it already holds files, the program stops and writes nothing. Pick a
> new folder name, or add `--overwrite` to replace the files the program writes (section 8).

---

## 6. The four masking levels, with examples

The levels are cumulative: each one hides what the one before it hid, and more. Level 2 is the default. `--seed 2026`
is used in every example so you can compare your results with the ones shown.

| Level | Name | Choose it when | What is hidden |
|---|---|---|---|
| **1** | light | the file stays inside your team | subject, study, site and user identifiers |
| **2** | clinical (default) | the file leaves your team | level 1, plus labels (treatment, sex, race, country), dates (shifted), free text (removed) |
| **3** | covariate | covariate values must not be recognisable | level 2, plus covariates such as `WT` and `AGE` multiplied or divided by a hidden factor |
| **4** | maximum | highest protection | level 3, but every single value gets its own factor; free-text columns are dropped; rows are sorted; an extra key file is written |

You can also write the level as a word: `light`, `clinical`, `covariate`, `maximum`.

At **every** level the PK/PD columns (`TIME`, `TAD`, `DV`, `AMT`, `EVID`, `MDV`, `CMT`, `RATE`, `ADDL`, `II`, `SS`,
`DVID`, ...) stay untouched.

### Level 1: identifiers only

```text
data-obfuscator run --input sample.csv --output masked_level1 --level 1 --seed 2026
```

```text
1,SUBJ001,STU001,SITE001,Drug 50 mg,0,50,.,1,1,1,72.5,34,M,WHITE,2025-03-10,screened by Dr Lee,a.jones@clinic-one.org
```

Only `USUBJID`, `STUDYID` and `SITEID` changed (`ID` stays `1` because the original IDs were already 1, 2, 3).
The treatment, sex, race, date, comment and e-mail are still real. The program says so:

```text
[WARN] date column DATE is kept but NOT shifted at level 1 - use level 2 or higher to shift dates
[WARN] column EMAIL appears to contain e-mail addresses and was not redacted - add it with --freetext-columns or --drop
```

This is the program telling you what level 1 leaves visible. Use level 1 only inside your team.

### Level 2: clinical (the default)

This is the run you already did in section 5 (the folder `masked_level2` exists). The first masked row:

```text
1,SUBJ001,STU001,SITE001,ARM_A,0,50,.,1,1,1,72.5,34,0,CAT001,2025-04-29,.,.
```

Treatment became `ARM_A`, sex became `0`, race became `CAT001`, the date moved by 50.8 days (every date moves by the
**same** amount, so intervals between dates stay correct), and the comment and e-mail were removed. Covariate values
(`72.5,34`) are still real.

### Level 3: covariates too

```text
data-obfuscator run --input sample.csv --output masked_level3 --level 3 --seed 2026
```

```text
1,SUBJ001,STU001,SITE001,ARM_A,0,50,.,1,1,1,935.87887,5.9954789,0,CAT001,2025-04-29,.,.
```

`WT` was multiplied by a hidden factor (12.9086740154) and `AGE` divided by another (5.6709398522). All rows of a
column use the **same** factor, so differences between subjects keep their proportions. The factors are written to
`keymap.txt`, which is how `reverse` undoes it.

### Level 4: maximum

```text
data-obfuscator run --input sample.csv --output masked_level4 --level 4 --seed 2026
```

```text
ID,USUBJID,STUDYID,SITEID,ARM,TIME,AMT,DV,EVID,MDV,CMT,WT,AGE,SEX,RACE,DATE
1,SUBJ001,STU001,SITE001,ARM_A,0,50,.,1,1,1,1018.283,6.8140228,0,CAT001,2025-04-29
1,SUBJ001,STU001,SITE001,ARM_A,1,0,12.4,0,0,2,830.37054,6.4888167,0,CAT001,2025-04-29
1,SUBJ001,STU001,SITE001,ARM_A,2,0,18.9,0,0,2,876.4464,6.0878952,0,CAT001,2025-04-29
```

Three differences from level 3:

- `WT` is no longer the same on every row of the same subject (`1018.283`, `830.37054`, `876.4464`): each value got its
  **own** factor, within plus or minus 15 % of the column's base factor. Nobody can recover the real subject-level
  weight by dividing by one number.
- The free-text columns `COMMENT` and `EMAIL` are **dropped**, not just emptied.
- The rows are **sorted**, and a second key file, `keymap_columns.txt`, is written. It holds the factor of every value
  and is needed to get the numbers back exactly.

### Compare all four at a glance

| Column | Original | Level 1 | Level 2 | Level 3 | Level 4 |
|---|---|---|---|---|---|
| `USUBJID` | ABC-101-0001 | SUBJ001 | SUBJ001 | SUBJ001 | SUBJ001 |
| `ARM` | Drug 50 mg | Drug 50 mg | ARM_A | ARM_A | ARM_A |
| `SEX` | M | M | 0 | 0 | 0 |
| `WT` | 72.5 | 72.5 | 72.5 | 935.87887 | 1018.283 |
| `DATE` | 2025-03-10 | 2025-03-10 | 2025-04-29 | 2025-04-29 | 2025-04-29 |
| `COMMENT` | screened by Dr Lee | screened by Dr Lee | `.` | `.` | *(column dropped)* |
| `DV` | 12.4 | 12.4 | 12.4 | 12.4 | 12.4 |

---

## 7. The commands

The program has four commands. Put the command name right after `data-obfuscator`.

| Command | Use it to | Section |
|---|---|---|
| `run` | mask **one** dataset | 5, 6, 8, 9 |
| `tokens` | count how many tokens a file uses, before you paste it into an LLM | 10 |
| `reverse` | **undo** a masking with the private key | 11 |
| `run2` | mask **several related datasets together** (for example CDISC ADaM) so an LLM can combine them | 12 |

Help is built in. These change nothing:

```text
data-obfuscator --help
data-obfuscator run --help
data-obfuscator reverse --help
data-obfuscator run2 --help
data-obfuscator tokens --help
data-obfuscator --version
```

Three things to know about typing commands:

- **Lists of columns** are separated by commas and must be **in double quotes**: `--drop "EMAIL,COMMENT"`. (In
  Windows PowerShell a list without quotes is not read correctly.)
- **Names with spaces** need quotes too: `--output "my masked data"`.
- **No colours**: add `--no-color` if you want plain text, for example when you save the output to a file.

---

## 8. Handy options for run

Each example runs on `sample.csv` and writes to its own new folder.

| What you want | Command |
|---|---|
| Keep only some columns | `data-obfuscator run --input sample.csv --output m_keep --keep "ID,USUBJID,TIME,AMT,DV,EVID,CMT,WT"` |
| Drop columns completely | `data-obfuscator run --input sample.csv --output m_drop --drop "EMAIL,COMMENT"` |
| Hide column names too | `data-obfuscator run --input sample.csv --output m_rename --rename-columns "WT=BODYWT,AGE=YEARS"` |
| Rename every column except the ones you list | `data-obfuscator run --input sample.csv --output m_rename_most --rename-except "ID,USUBJID,TIME,AMT,DV,EVID,MDV,CMT"` |
| Harder-to-guess subject codes | `data-obfuscator run --input sample.csv --output m_hashed --id-style hashed --seed 2026` |
| Padded subject codes (`0001`) | `data-obfuscator run --input sample.csv --output m_padded --id-style padded` |
| Choose the date shift yourself | `data-obfuscator run --input sample.csv --output m_shift --date-shift-days 30` |
| Name the masked file yourself | `data-obfuscator run --input sample.csv --output m_named --output-name study_masked.csv` |
| Keep a column exactly as it is, even at level 3 | `data-obfuscator run --input sample.csv --output m_protect --protect "WT" --level 3` |
| Scale an extra numeric column (level 3 and up) | `data-obfuscator run --input sample.csv --output m_scale --level 3 --scale-columns "YOURCOL"` |
| Treat an extra column as an identifier | `data-obfuscator run --input sample.csv --output m_sens --sensitive "YOURCOL"` |
| Tell the program which column is the subject | `data-obfuscator run --input sample.csv --output m_roles --id-column ID --subject-column USUBJID --site-column SITEID` |
| Replace the files of an earlier run | `data-obfuscator run --input sample.csv --output masked_level2 --overwrite --seed 2026` |
| A different seed (different random parts) | `data-obfuscator run --input sample.csv --output m_seed --seed 99` |

`YOURCOL` stands for the name of a column in **your own** file (the practice file has no such column; a name that is not in the file is simply ignored).

Four of these, with the real results:

**Keep only some columns (`--keep`)**, with hashed subject codes (`--id-style hashed`):

```text
data-obfuscator run --input sample.csv --output m_hashed --id-style hashed --seed 2026 --keep "ID,USUBJID,TIME,AMT,DV,EVID,CMT,WT"
```

```text
ID,USUBJID,TIME,AMT,DV,EVID,CMT,WT
14041,SUBJ7DA8C2C3,0,50,.,1,1,72.5
14041,SUBJ7DA8C2C3,1,0,12.4,0,2,72.5
```

**Drop and rename:**

```text
data-obfuscator run --input sample.csv --output m_rename --drop "EMAIL,COMMENT" --rename-columns "WT=BODYWT,AGE=YEARS" --seed 2026
```

```text
ID,USUBJID,STUDYID,SITEID,ARM,TIME,AMT,DV,EVID,MDV,CMT,BODYWT,YEARS,SEX,RACE,DATE
1,SUBJ001,STU001,SITE001,ARM_A,0,50,.,1,1,1,72.5,34,0,CAT001,2025-04-29
```

Every rename is recorded in `keymap.txt`, so `reverse` can put the names back.

**A date shift you choose, and your own file name:**

```text
data-obfuscator run --input sample.csv --output m_shift --date-shift-days 30 --output-name study_masked.csv --seed 2026
```

The dates in `study_masked.csv` are now exactly 30 days later than the originals (`2025-03-10` became `2025-04-09`).

**Replace an earlier run.** Running into a folder that already has files is refused:

```text
[X] ERROR: output folder masked_level2 already contains 8 file(s) or folder(s). Nothing was written. Choose another --output folder, or add --overwrite to replace the files this program writes.
```

Choose a new folder, or add `--overwrite`.

**Other input formats.** The program recognises the delimiter by itself. Force it with `--delimiter comma|tab|space|semicolon`.
A file without a header row needs `--no-header --columns "ID,TIME,AMT,DV"`. Lines at the top that start with `#`, `;` or
`$` are treated as comments.

> **If your column names are unusual**, the program may not recognise their role. Open `column_report.txt` after the
> first run: it lists every column with the role the program gave it. Anything marked "passed through" was copied
> unchanged. If it identifies something, name it with `--sensitive`, a role option such as `--site-column`, or drop it.

---

## 9. Options files

When a command gets long, put the options in a text file and run it with `--config`. Save this as `my_options.txt` in
`work`:

```text
# my_options.txt - lines that start with # are comments
input    = sample.csv
output   = masked_from_options
level    = 3
seed     = 2026
id_style = padded
date_shift_days = 30
```

Run it:

```text
data-obfuscator run --config my_options.txt
```

```text
ID,USUBJID,STUDYID,SITEID,ARM,TIME,AMT,DV,EVID,MDV,CMT,WT,AGE,SEX,RACE,DATE,COMMENT,EMAIL
0001,SUBJ001,STU001,SITE001,ARM_A,0,50,.,1,1,1,764.84203,2.954278,0,CAT001,2025-04-09,.,.
```

Anything you type on the command line **overrides** the file, so one file can serve many runs:

```text
data-obfuscator run --config my_options.txt --level 2 --output masked_from_options2
```

Every run writes `config.used.txt` into its output folder, showing exactly which options were used. Keep it together with
your results so the run can be repeated.

---

## 10. Count tokens: tokens

LLMs have a size limit. `tokens` tells you how many tokens a file uses, for several model families, **before** you paste
it in. (`run` also does this automatically and writes the result to `tokens.txt`.)

```text
data-obfuscator tokens --input sample.csv --output token_check
```

```text
FILE                                BYTES       TOKENS
INPUT
  sample.csv                         1458          743

MODEL                                                   INPUT   OUT DATA      OTHER ALL COUNTED
GPT-4 / GPT-3.5 (cl100k_base)                             743          0        665        1408
GPT-4o / GPT-4.1 / o-series / GPT-5 (o200k_base)          743          0        657        1400
Claude 3 and later (Anthropic)                            743          0        703        1446
Gemini (Google)                                           853          0        676        1529
Llama 3 / 3.1 / 3.3 (Meta)                                743          0        665        1408
```

The numbers are estimates. The full report is written to `token_check/tokens.txt`.

---

## 11. Undo the masking: reverse

`reverse` puts the real values back, using the private key (`keymap.txt`) that `run` wrote. The masked file can be in
any shape: rows in a different order, columns in a different order, extra columns added by an LLM. `reverse` restores
what it recognises.

### The basic command

```text
data-obfuscator reverse --input masked_level2/obfuscated.csv --keymap masked_level2/keymap.txt --output restored_level2
```

The result is in `restored_level2`:

| File | What it is |
|---|---|
| `restored.csv` | the dataset with the real values |
| `reverse_report.txt` | exactly what was undone, what was not, and any warnings |
| `reverse.log` | the log |

```text
1,ABC-101-0001,ABC-101,0142,Drug 50 mg,0,50,.,1,1,1,72.5,34,M,WHITE,2025-03-10,.,.
```

This is your original row again. The only thing that cannot come back is the free text that level 2 removed
(`COMMENT` and `EMAIL`), which is why you see a warning. Run it for yourself and compare `restored.csv` with
`sample.csv`: all other 16 columns are identical.

### Every level can be undone

```text
data-obfuscator reverse --input masked_level1/obfuscated.csv --keymap masked_level1/keymap.txt --output restored_level1
data-obfuscator reverse --input masked_level3/obfuscated.csv --keymap masked_level3/keymap.txt --output restored_level3
data-obfuscator reverse --input masked_level4/obfuscated.csv --keymap masked_level4/keymap.txt --output restored_level4
```

For **level 4**, `reverse` finds `keymap_columns.txt` automatically when it sits next to `keymap.txt`. You will see:

```text
Per-value file:   masked_level4/keymap_columns.txt
  - matched 15 of 15 row(s) to keymap_columns.txt
  - scaling: WT (level 4, per-value factors) 15 value(s) exactly
  - dates: DATE moved back by 50.8 day(s): 15 value(s)
```

If you keep the two key files somewhere else, name the second one with `--keymap-columns path/to/keymap_columns.txt`.

### Rows and columns in any order

An LLM may sort the rows differently or reorder the columns. Try it yourself: open `masked_level4/obfuscated.csv` in a
spreadsheet (or a text editor), move some columns around, sort the rows in a different order, and save the result as
`shuffled_level4.csv` (as CSV). A shuffled file looks like this:

```text
MDV,ID,RACE,CMT,ARM,USUBJID,TIME,SITEID,STUDYID,DATE,SEX,AGE,EVID,DV,WT,AMT
0,3,CAT003,2,ARM_C,SUBJ003,2,SITE002,STU001,2025-05-03,1,6.7392735,0,0,825.53026,0
1,3,CAT003,1,ARM_C,SUBJ003,0,SITE002,STU001,2025-05-03,1,6.9413331,1,.,727.33063,0
```

Then restore it:

```text
data-obfuscator reverse --input shuffled_level4.csv --keymap masked_level4/keymap.txt --output restored_shuffled
```

`reverse` matches each row by its identifiers and time, and the restored file has the **original** column and row order
and the **exact** original numbers.

### Choosing what to undo

| Option | What it does |
|---|---|
| `--only dates,recoding` | undo only these parts. The four parts are `rename`, `scaling`, `dates`, `recoding` |
| `--skip scaling` | undo everything except these parts |
| `--strict` | stop with an error instead of approximating, skipping a missing column or leaving an unknown value unchanged |
| `--keep-order` | keep the column and row order of the input instead of restoring the original order |
| `--keymap-columns FILE` | the level 4 key file, when it is not next to `keymap.txt` |
| `--output-name NAME` | the file name of the restored dataset |
| `--overwrite` | write into a folder that already has files |

Example, undoing only the dates and the codes of a level 3 file (the covariates stay scaled):

```text
data-obfuscator reverse --input masked_level3/obfuscated.csv --keymap masked_level3/keymap.txt --only dates,recoding --output restored_only
```

```text
1,ABC-101-0001,ABC-101,0142,Drug 50 mg,0,50,.,1,1,1,935.87887,5.9954789,M,WHITE,2025-03-10,.,.
```

If you leave out `--keymap`, `reverse` looks for `keymap.txt` in the same folder as the input file.

### What a restore can and cannot give back

| Comes back exactly | Comes back approximately | Never comes back |
|---|---|---|
| identifiers, labels, dates, renamed columns, level 1 to 3 scaled numbers, level 4 numbers (with `keymap_columns.txt`) | level 4 numbers **without** `keymap_columns.txt` (within about 15 %); numbers are written in their shortest form (`70.50` returns as `70.5`) | removed free text, dropped columns, values an LLM invented |

Read `reverse_report.txt` after every restore. The restored file holds real data again: do not share it.

---

## 12. Mask several datasets together: run2

A common job: *"combine my ADaM `ADSL`, `ADEX` and `ADPC` into one NONMEM-style master dataset"*, without leaking anything.
`run2` masks the files **together**, so the same subject gets the same code in every file and the LLM can still join
them. Later, `reverse` puts the real values into the master dataset the LLM builds.

`run2` works on levels 1 to 3 (use `run` for level 4).

### Make three small ADaM files

Save each block into `work` under the name shown.

**`adsl.csv`**

```text
STUDYID,USUBJID,SITEID,ARM,AGE,SEX,RACE,WEIGHTBL,TRTSDT
ABC-101,ABC-101-0001,0142,Drug 50 mg,34,M,WHITE,72.5,2025-03-10
ABC-101,ABC-101-0002,0142,Drug 100 mg,51,F,ASIAN,88.1,2025-03-12
ABC-101,ABC-101-0003,0275,Placebo,43,F,BLACK,65.3,2025-03-14
```

**`adex.csv`**

```text
STUDYID,USUBJID,EXTRT,EXDOSE,EXSTDTC
ABC-101,ABC-101-0001,DRUG X,50,2025-03-10T08:00:00
ABC-101,ABC-101-0002,DRUG X,100,2025-03-12T08:00:00
ABC-101,ABC-101-0003,PLACEBO,0,2025-03-14T08:00:00
```

**`adpc.csv`**

```text
STUDYID,USUBJID,PARAMCD,ADTM,AVAL
ABC-101,ABC-101-0001,CONC,2025-03-10T09:00:00,12.4
ABC-101,ABC-101-0001,CONC,2025-03-10T10:00:00,18.9
ABC-101,ABC-101-0002,CONC,2025-03-12T09:00:00,25.7
ABC-101,ABC-101-0002,CONC,2025-03-12T10:00:00,37.2
ABC-101,ABC-101-0003,CONC,2025-03-14T09:00:00,0
ABC-101,ABC-101-0003,CONC,2025-03-14T10:00:00,0
```

### Mask them together

```text
data-obfuscator run2 --input adsl.csv --input adex.csv --input adpc.csv --output masked_adam --level 3 --seed 2026 --scale-columns "WEIGHTBL"
```

Three ways to give the files, all with the same result: repeat `--input` as above, give a comma-separated list
(`--input adsl.csv,adex.csv,adpc.csv`), or give a **folder** that holds only these files (`--input adam_in`; every
`.csv`, `.tsv`, `.txt`, `.dat` and `.tab` file in it is used).

`--scale-columns "WEIGHTBL"` is added because `run2` scales `AGE` automatically at level 3 but does not know that
`WEIGHTBL` is a covariate; name such columns yourself.

The output folder `masked_adam` holds:

| File | What it is | Share it? |
|---|---|---|
| `adsl_masked.csv`, `adex_masked.csv`, `adpc_masked.csv` | the masked datasets | **yes** |
| `llm2.txt` | ready-to-paste prompts that ask the LLM to combine the files and what to keep | **yes** |
| `keymap2.txt` | **the private key** | **never** |
| `summary2.txt`, `config.used.txt`, `run2.log` | the record of the run | no |

The masked `adsl_masked.csv`:

```text
STUDYID,USUBJID,SITEID,ARM,AGE,SEX,RACE,WEIGHTBL,TRTSDT
STU001,SUBJ001,SITE001,ARM_A,438.89492,0,CAT001,12.784477,2025-04-30
STU001,SUBJ002,SITE001,ARM_B,658.34237,1,CAT002,15.535344,2025-05-02
STU001,SUBJ003,SITE002,ARM_C,555.07298,1,CAT003,11.514846,2025-05-04
```

`SUBJ001` is the same person in all three files, and every date in all three files moved by the **same** 51 days, so
the LLM can join them and compute times correctly. Notice that `ADEX` and `ADPC` use the date-time columns `EXSTDTC` and
`ADTM`: `run2` recognises ADaM date names (such as `...DT`, `...DTM`, `...DTC`) by itself.

### What the LLM sends back

You paste the three masked files and `llm2.txt` into the LLM, and it builds a master dataset. Here is a small one of
the kind it returns. Save it as `master_from_llm.csv` in `work` (it matches this exact run, with seed 2026):

```text
ID,USUBJID,DATETIME,EVID,AMT,DV,WT,ARM
1,SUBJ001,2025-04-30T08:00:00,1,50,.,12.784477,ARM_A
1,SUBJ001,2025-04-30T09:00:00,0,0,12.4,12.784477,ARM_A
1,SUBJ001,2025-04-30T10:00:00,0,0,18.9,12.784477,ARM_A
2,SUBJ002,2025-05-02T08:00:00,1,100,.,15.535344,ARM_B
2,SUBJ002,2025-05-02T09:00:00,0,0,25.7,15.535344,ARM_B
2,SUBJ002,2025-05-02T10:00:00,0,0,37.2,15.535344,ARM_B
3,SUBJ003,2025-05-04T08:00:00,1,0,.,11.514846,ARM_C
3,SUBJ003,2025-05-04T09:00:00,0,0,0,11.514846,ARM_C
3,SUBJ003,2025-05-04T10:00:00,0,0,0,11.514846,ARM_C
```

The LLM has given some columns new names: `DV` (from `AVAL`), `WT` (from `WEIGHTBL`), `DATETIME` (from `ADTM`) and `ID`
(from `USUBJID`). That is normal. Tell `reverse` about the renamed ones with `--map COMBINED=SOURCE`.

### Restore the real values

```text
data-obfuscator reverse --input master_from_llm.csv --keymap masked_adam/keymap2.txt --map ID=USUBJID --map DV=AVAL --map WT=WEIGHTBL --map DATETIME=ADTM --output restored_master
```

```text
ID,USUBJID,DATETIME,EVID,AMT,DV,WT,ARM
ABC-101-0001,ABC-101-0001,2025-03-10T08:00:00,1,50,.,72.5,Drug 50 mg
ABC-101-0001,ABC-101-0001,2025-03-10T09:00:00,0,0,12.4,72.5,Drug 50 mg
ABC-101-0001,ABC-101-0001,2025-03-10T10:00:00,0,0,18.9,72.5,Drug 50 mg
ABC-101-0002,ABC-101-0002,2025-03-12T08:00:00,1,100,.,88.1,Drug 100 mg
...
```

Real subject codes, real dates (51 days back), the real weight (the scaling undone) and the real treatment.

The extra options of `reverse` for `run2` results:

| Option | What it does |
|---|---|
| `--map COMBINED=SOURCE` | the column `COMBINED` of the master dataset came from the column `SOURCE` (`ID=USUBJID`, `DV=AVAL`). Repeat it for each. |
| `--map COMBINED=SOURCE:WHENCOL=VALUE` | the same, but only for the rows where `WHENCOL` equals `VALUE` (when one column of the master mixes several source columns) |
| `--date-columns "COL1,COL2"` | extra columns of the master dataset that hold shifted dates |
| `--strict` | stop on anything that cannot be restored exactly |

Without `--map` the program still restores every column that has the same name as in the key, and **tells you what to
add**.

---

## 13. Cheat sheet

| I want to... | Command |
|---|---|
| check that the program works | `data-obfuscator --version` |
| see all options of a command | `data-obfuscator run --help` |
| mask one file (default level) | `data-obfuscator run --input FILE --output NEWFOLDER` |
| mask at a chosen level | `data-obfuscator run --input FILE --output NEWFOLDER --level 1` *(1, 2, 3 or 4)* |
| make a result repeatable | add `--seed 2026` (use your own number) |
| use an options file | `data-obfuscator run --config my_options.txt` |
| count tokens | `data-obfuscator tokens --input FILE --output NEWFOLDER` |
| undo a masking | `data-obfuscator reverse --input MASKED.csv --keymap keymap.txt --output NEWFOLDER` |
| mask several related files | `data-obfuscator run2 --input A.csv --input B.csv --output NEWFOLDER --level 3` |
| undo a combined dataset | `data-obfuscator reverse --input MASTER.csv --keymap keymap2.txt --map ID=USUBJID --output NEWFOLDER` |

Share only: `obfuscated.csv` and `llm.txt` (or the `*_masked.csv` files and `llm2.txt` from `run2`).
Keep private: `keymap.txt`, `keymap_columns.txt`, `keymap2.txt`, `summary.txt`, `config.used.txt` and the logs.

---

## 14. If something goes wrong

| What you see | What it means and what to do |
|---|---|
| Windows: *"Windows protected your PC"* | Click **More info**, then **Run anyway**, or run `Unblock-File .\data-obfuscator.exe` once (section 2). |
| Windows: `data-obfuscator : The term 'data-obfuscator' is not recognized` | You are not in a window where the PATH line was run. Repeat Step 4 of section 2, or use `..\data-obfuscator.exe` from the `work` folder. |
| Linux: `Permission denied` | Run `chmod +x data-obfuscator` (section 3). If the folder is on a drive mounted without permission to run programs, copy the folder to your home directory. |
| Linux: `command not found` | Repeat Step 4 of section 3, or use `../data-obfuscator` from the `work` folder. |
| Linux: an error about `GLIBC` or `cannot execute binary file` | The Linux program does not fit this computer (it needs 64-bit x86 Linux, and a C library recent enough for the build). Tell the person who sent you the folder. |
| The banner shows odd characters on Windows | Harmless: an older console cannot draw the box. Use Windows Terminal, or ignore it. |
| `opening input sample.csv: No such file` | The file is not in the folder you are in. Check with `dir` (Windows) or `ls` (Linux), or give the full path with `--input`. |
| On Windows the file is called `sample.csv.txt` | Notepad added `.txt`. Save again with **Save as type: All files** (section 4). |
| `output folder ... already contains N file(s)` | Choose a new `--output` folder, or add `--overwrite`. |
| `unknown level '7'` | The level must be 1, 2, 3 or 4 (or `light`, `clinical`, `covariate`, `maximum`). |
| `level 4 is not available in run2` | `run2` supports levels 1 to 3. Use `run` for level 4. |
| A `[WARN]` about a date, e-mail or text column | The program is telling you what the chosen level leaves visible. Use a higher level, `--freetext-columns`, or `--drop`. |
| A `[WARN]` in `reverse` about COMMENT or EMAIL | Free text removed by masking cannot come back. This is expected. |
| A list of columns is not understood (PowerShell) | Put the list in double quotes: `--drop "EMAIL,COMMENT"`. |

Every run also writes a log (`obfuscator.log`, `reverse.log`, `run2.log`) in its output folder; failed runs leave one too.

---

## 15. Keep it safe

- **Masking is not formal anonymisation.** Rare dates, unusual covariate combinations and rare events can still point
  to a person. Read the warnings, choose the lowest level that is enough for your audience (level 2 for anything that
  leaves your team), and follow your organisation's data-protection rules. The decision to share stays with you.
- **The key is the master key.** Whoever has the masked file and `keymap.txt` can undo the masking. Keep the keys
  offline and never paste them into an LLM or a chat. `summary.txt` and `config.used.txt` show the seed and the date
  shift, so keep them private as well.
- **Use your own seed.** The default (42) is public. Pick your own number and keep it in your options file.
- **Do not turn the key off** (`--write-keymap false`) unless you are sure you never need the real values back: without
  the key nothing can be restored.
- **Restored files are real data again.** Treat `restored.csv` and `reverse_report.txt` like the original.

---

## 16. Author, license and citation

**Pharmacometrics Data Obfuscator** was written by **Obinna (Obi) Obianom**, Senior Principal Scientist.

- Website: <https://obianom.com>
- LinkedIn: <https://www.linkedin.com/in/oobianom>
- Google Scholar: <https://scholar.google.com/citations?user=PcvZPUQAAAAJ&hl=en>

**License.** Creative Commons Attribution-NoDerivatives 4.0 International (CC BY-ND 4.0): you may use the program and
cite it, and you may pass on the **unmodified** program with credit, but you may not alter it and share the result
without the author's permission. Your data are yours: the license covers the program and its documentation, not the
datasets you mask. The program is provided as it is, without warranty. Details: <https://creativecommons.org/licenses/by-nd/4.0/>.

**How to cite (MLA).** Replace the version with the one you used (`data-obfuscator --version` prints it):

```text
Obianom, Obinna. *Pharmacometrics Data Obfuscator*. Version 0.0.0.23, 13 Oct. 2026, obianom.com.
```

(The asterisks mark the italics.)
