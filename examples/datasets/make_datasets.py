#!/usr/bin/env python3
"""
Builds the five example datasets in this folder (standard library only, deterministic).

    python make_datasets.py            # rewrites the files next to this script

Program : data-obfuscator example data   |   Author : Obinna Obianom   |   Date : 2026-10-08

All data are SIMULATED. Nothing here comes from a real study, person, site or product.

The script is also a recipe: every dataset is built the same way -
  1. simulate "truth" (subjects, covariates, dosing history, concentrations),
  2. lay the records out the way NONMEM wants them (dose rows EVID=1, observation rows
     EVID=0, one TIME per record, dose before observation unless the observation is a
     pre-dose trough),
  3. (QC sets only) plant known problems and write them to qc_answer_key.md, so you can
     check that a QC procedure - yours, or an LLM given the masked file - finds them.
"""
import math
import random
from datetime import datetime, timedelta
from pathlib import Path

HERE = Path(__file__).resolve().parent


# ----------------------------------------------------------------------------- helpers
def f3(x):
    return f"{x:.3f}"


def f2(x):
    return f"{x:.2f}"


def g(x):
    return f"{x:g}"


def oral_conc(t_h, doses, ka, ke, v, f=1.0):
    """One-compartment first-order absorption, superposition over (time_h, amount_mg)."""
    c = 0.0
    for td, amt in doses:
        dt = t_h - td
        if dt > 0:
            c += f * amt * ka / (v * (ka - ke)) * (math.exp(-ke * dt) - math.exp(-ka * dt))
    return c


def lognorm(rng, sd):
    return math.exp(rng.gauss(0, sd))


def clip(x, lo, hi):
    return max(lo, min(hi, x))


def write_lines(path, lines):
    # LF line endings, trailing newline, ASCII only.
    Path(path).write_text("\n".join(lines) + "\n", encoding="ascii", newline="\n")


# ----------------------------------------------------------------------------- QC sets
QC_COLS = [
    "ID", "USUBJID", "STUDYID", "SITEID", "ARM", "DATE", "CLOCK", "NTIM", "TIME", "TAD",
    "EVID", "MDV", "CMT", "AMT", "RATE", "ADDL", "II", "SS", "DOSE", "DV",
    "WT", "AGE", "SEX", "CRCL",
]
OBS_NTIM = [0.5, 1, 2, 4, 8, 12, 24, 24.5, 25, 26, 28, 32, 36, 48]
TROUGHS = {24, 48}


def qc_subject(rng, sid, site, dose, start_dt, study="ABC-101"):
    """Return the records of one subject: two daily oral doses, 14 PK samples."""
    wt = round(clip(rng.gauss(75, 12), 50, 110), 1)
    age = int(clip(rng.gauss(42, 11), 19, 68))
    sex = rng.choice([0, 1])
    crcl = int(clip(rng.gauss(100, 14), 65, 140))
    ka = 1.2 * lognorm(rng, 0.3)
    v = 40 * (wt / 70) * lognorm(rng, 0.2)
    cl = 6 * (wt / 70) ** 0.75 * lognorm(rng, 0.25)
    ke = cl / v
    if abs(ka - ke) < 0.05:
        ka += 0.2

    d1 = 0
    d2 = 24 * 60 + rng.randint(-10, 20)  # minutes after first dose
    dose_min = [d1, d2]
    events = [("dose", 0.0, d1, 1), ("dose", 24.0, d2, 2)]
    for n in OBS_NTIM:
        if n == 24:  # trough: drawn just before dose 2
            m = d2 - rng.randint(5, 15)
        elif n == 48:  # trough before the (not given) third dose
            m = 48 * 60 + rng.randint(-8, 8)
        elif n < 24:
            m = round(n * 60) + rng.randint(-4, 4)
        else:
            m = d2 + round((n - 24) * 60) + rng.randint(-4, 4)
        events.append(("obs", n, m, 0))
    events.sort(key=lambda e: (e[2], 0 if e[0] == "obs" else 1))

    site_no = site[1:]
    rows = []
    for kind, ntim, m, dnum in events:
        dt = start_dt + timedelta(minutes=m)
        t_h = m / 60.0
        last_dose = max(x for x in dose_min if x <= m) if kind == "obs" else m
        r = dict(
            ID=sid, USUBJID=f"{study}-{site_no}-{sid:03d}", STUDYID=study, SITEID=site,
            ARM=f"{dose}mg", DATE=dt.strftime("%Y-%m-%d"), CLOCK=dt.strftime("%H:%M"),
            NTIM=g(ntim), TIME=f2(t_h), TAD=f2((m - last_dose) / 60.0),
            EVID=1 if kind == "dose" else 0, MDV=1 if kind == "dose" else 0,
            CMT=1 if kind == "dose" else 2, AMT=dose if kind == "dose" else 0,
            RATE=0, ADDL=0, II=0, SS=0, DOSE=dose,
            DV=0 if kind == "dose" else f3(oral_conc(t_h, [(x / 60.0, dose) for x in dose_min], ka, ke, v) * lognorm(rng, 0.15)),
            WT=wt, AGE=age, SEX=sex, CRCL=crcl,
        )
        r["_kind"] = kind
        r["_dnum"] = dnum
        r["_min"] = m
        r["_tags"] = []
        r["_conc"] = (ka, ke, v)
        r["_start"] = start_dt
        r["_dose_min"] = dose_min
        rows.append(r)
    return rows


def pick(rows, kind, ntim=None, dnum=None):
    for r in rows:
        if r["_kind"] != kind:
            continue
        if kind == "obs" and g(ntim) != r["NTIM"]:
            continue
        if kind == "dose" and dnum != r["_dnum"]:
            continue
        return r
    raise KeyError((kind, ntim, dnum))


def retime(r, minutes_shift_dates=0):
    """Recompute TIME/DATE/CLOCK of an observation after its actual minute changed."""
    dt = r["_start"] + timedelta(minutes=r["_min"])
    r["DATE"], r["CLOCK"], r["TIME"] = dt.strftime("%Y-%m-%d"), dt.strftime("%H:%M"), f2(r["_min"] / 60.0)


def serialise(r):
    if "_raw" in r:
        return r["_raw"]
    return ",".join(str(r[c]) for c in QC_COLS)


class Key:
    """Collects planted problems and later resolves them to file line numbers."""

    def __init__(self):
        self.items = []

    def add(self, code, sev, cat, sid, rows, what, how):
        rows = rows if isinstance(rows, list) else [rows]
        for r in rows:
            r["_tags"].append(code)
        self.items.append(dict(code=code, sev=sev, cat=cat, sid=sid, what=what, how=how))

    def resolve(self, final_rows):
        for it in self.items:
            lines = [i + 2 for i, r in enumerate(final_rows) if it["code"] in r["_tags"]]
            it["lines"] = lines

    def lines_text(self, it):
        ls = it["lines"]
        if not ls:
            return "(rows absent)"
        if len(ls) == 1:
            return f"{ls[0]}"
        if ls == list(range(ls[0], ls[-1] + 1)) and len(ls) > 3:
            return f"{ls[0]}-{ls[-1]}"
        return ", ".join(str(x) for x in ls)


def build_clean():
    rng = random.Random(20261008)
    base = datetime(2024, 3, 4, 8, 0)
    subjects = []
    for sid in range(1, 13):
        site = "S01" if sid <= 6 else "S02"
        dose = 100 if sid % 2 else 200
        start = base + timedelta(days=(sid - 1) // 3, minutes=15 * ((sid * 7) % 6))
        subjects.append(qc_subject(rng, sid, site, dose, start))
    key = Key()

    # W1: a sample taken ~55 min late. Actual TIME is right; NTIM (nominal) differs.
    s = subjects[4]
    r = pick(s, "obs", 2)
    r["_min"] = 2 * 60 + 55
    ka, ke, v = r["_conc"]
    r["DV"] = f3(oral_conc(r["_min"] / 60, [(0, 100)] + [(x / 60, 100) for x in r["_dose_min"][1:]], ka, ke, v))
    r["TAD"] = f2(r["_min"] / 60.0)
    retime(r)
    key.add("W1", "WARN", "Time", 5, r, "Sample nominally 2 h post-dose was taken 55 min late (TIME 2.92 vs NTIM 2).",
            "Compare TIME with NTIM per record; flag records where TIME and NTIM differ by more than your tolerance (e.g. 30 min). TIME itself is correct.")

    # W2: partial dose.
    s = subjects[6]
    r = pick(s, "dose", dnum=2)
    r["AMT"] = 150
    key.add("W2", "WARN", "AMT vs DOSE", 7, r, "Second dose AMT=150 while DOSE=200 (subject on the 200 mg arm).",
            "For every EVID=1 row check AMT == DOSE; here it is a documented partial dose, so confirm against the CRF rather than 'fix' it.")

    # W3: missing weight.
    s = subjects[8]
    for r in s:
        r["WT"] = "."
    key.add("W3", "WARN", "Covariate", 9, s, "WT missing for the whole subject ('.' is read as 0 by NONMEM).",
            "Count missing/zero covariates per subject; a WT=0 breaks allometric scaling such as (WT/70)**0.75.")

    # W4: zero concentration coded as a real observation.
    s = subjects[2]
    r = pick(s, "obs", 0.5)
    r["DV"] = "0.000"
    key.add("W4", "WARN", "Obs coding", 3, r, "DV=0 at 0.5 h post-dose with MDV=0 - probably BLQ that was never coded as BLQ/CENS.",
            "List observation rows with DV<=0 and MDV=0; they poison log-transformed or proportional-error fits.")

    # W5: duplicate observation.
    s = subjects[10]
    r = pick(s, "obs", 4)
    dup = dict(r)
    dup["_tags"] = []
    dup["DV"] = f3(float(r["DV"]) * 1.04)
    s.insert(s.index(r) + 1, dup)
    key.add("W5", "WARN", "Duplicate", 11, [r, dup], "Two observation records at the same TIME for the 4 h sample (repeat assay).",
            "Check duplicated (ID, TIME, EVID, CMT) keys; keep one value or average, and document the rule.")

    # W6: TAD not reset.
    s = subjects[1]
    r = pick(s, "obs", 28)
    r["TAD"] = r["TIME"]
    key.add("W6", "WARN", "Time", 2, r, "TAD equals TIME (28.0x) instead of time since the second dose (about 4.0x).",
            "Recompute TAD from the last EVID=1 row and compare with the supplied TAD.")

    final = [r for s in subjects for r in s]
    key.resolve(final)
    write_lines(HERE / "qc_nearly_clean.csv", [",".join(QC_COLS)] + [serialise(r) for r in final])
    return key, final


def build_errors():
    rng = random.Random(20261009)
    base = datetime(2024, 5, 6, 8, 0)
    subj = {}
    for sid in range(1, 11):
        site = "S01" if sid <= 5 else "S02"
        dose = 100 if sid % 2 else 200
        start = base + timedelta(days=(sid - 1) // 3, minutes=15 * ((sid * 5) % 6))
        if sid == 7:
            start = start.replace(hour=16, minute=0)  # so the 8 h and 12 h samples cross midnight
        subj[sid] = qc_subject(rng, sid, site, dose, start)
    key = Key()
    ERR, WRONG = "ERROR", "SILENT"

    # ---- subject 1 -------------------------------------------------------------------
    s = subj[1]
    d1, d2 = pick(s, "dose", dnum=1), pick(s, "dose", dnum=2)
    pre = dict(d1)
    pre.update(_tags=[], _kind="obs", _dnum=0, EVID=0, MDV=0, CMT=2, AMT=0, DV="3.200", NTIM="0", TAD=".",
               TIME=f2(-0.25), DATE=d1["DATE"], CLOCK=(d1["_start"] - timedelta(minutes=15)).strftime("%H:%M"))
    s.insert(0, pre)
    key.add("B10", WRONG, "Dose/obs coding", 1, pre, "Observation 15 min BEFORE the first dose with DV=3.2 (should be ~0 / BLQ).",
            "For each subject list observations with TIME below the first dose time; a non-zero value there means a wrong date/time or a mis-assigned sample.")
    d1b = dict(d1)
    d1b["_tags"] = []
    s.insert(s.index(d1) + 1, d1b)
    key.add("B11", WRONG, "Dose/obs coding", 1, [d1, d1b], "First dose record appears twice (same TIME, same AMT): a double dose.",
            "Check duplicated (ID, TIME, EVID=1) rows and AMT totals per subject against the protocol.")
    d2["EVID"], d2["MDV"] = 0, 0
    key.add("B1", ERR, "Dose/obs coding", 1, d2, "Second dose has EVID=0 / MDV=0: the dose is coded as an observation (AMT=100, DV=0).",
            "Every row with AMT>0 must have EVID=1 (or 4); every EVID=0 row must have AMT=0 and a DV.")

    # ---- subject 2 -------------------------------------------------------------------
    s = subj[2]
    r = pick(s, "obs", 2); r["DV"] = "BQL"
    key.add("A1", ERR, "NONMEM readability", 2, r, "DV is the text 'BQL'.", "Scan numeric columns for non-numeric strings (BQL, NA, <0.5, ND).")
    r = pick(s, "dose", dnum=2); r["AMT"] = "."
    key.add("B3", ERR, "Dose/obs coding", 2, r, "Dose record (EVID=1) with missing AMT.", "EVID=1 rows need AMT>0 (EVID=1 with AMT '.' = zero dose).")
    r = pick(s, "obs", 8); r["DV"] = "."
    key.add("B9", WRONG, "Dose/obs coding", 2, r, "Observation with missing DV but EVID=0, MDV=0: NONMEM reads it as DV=0.", "MDV should be 1 whenever DV is missing.")
    r = pick(s, "dose", dnum=1); r["AMT"] = "0.1"
    key.add("D1", WRONG, "AMT vs DOSE", 2, r, "AMT=0.1 while DOSE=200 - a g vs mg unit slip.", "Compare AMT with DOSE on every dose row; look at the ratio (0.1/200 is not a dose, 1000x is a unit error).")
    for r in s:
        r["CRCL"] = -5
    key.add("E7", WRONG, "Covariate", 2, s, "CRCL=-5 for the whole subject.", "Range-check covariates (CRCL, WT, AGE) against physiological limits.")

    # ---- subject 3 -------------------------------------------------------------------
    s = subj[3]
    r = pick(s, "dose", dnum=1); r["CMT"] = 2
    key.add("B5", WRONG, "Dose/obs coding", 3, r, "First dose is sent to CMT=2 (central) instead of CMT=1 (depot).", "Doses of an oral model must target the depot; list CMT by EVID.")
    r = pick(s, "obs", 4)
    vals = [str(r[c]) for c in QC_COLS][:-4]
    r["_raw"] = ",".join(vals)
    key.add("A4", ERR, "NONMEM readability", 3, r, "Record has 20 fields instead of 24 (truncated).", "Count fields per line; every line must match the header.")
    late = False
    for r in s:
        if r["_kind"] == "dose" and r["_dnum"] == 2:
            late = True
        if late:
            r["DOSE"] = 200
    key.add("D3", WRONG, "AMT vs DOSE", 3, [r for r in s if r["DOSE"] == 200], "DOSE jumps from 100 to 200 after the second dose although AMT stays 100.", "DOSE should be constant within a subject unless a titration dose record exists.")

    # ---- subject 4 -------------------------------------------------------------------
    s = subj[4]
    r = pick(s, "obs", 1); r["DV"] = "<0.5"
    key.add("A2", ERR, "NONMEM readability", 4, r, "DV is '<0.5'.", "Replace BLQ strings with a numeric rule (M3: DV=LLOQ + CENS=1; or M1 drop with MDV=1).")
    r = pick(s, "obs", 8)
    r["DATE"] = (datetime.strptime(r["DATE"], "%Y-%m-%d") + timedelta(days=30)).strftime("%Y-%m-%d")
    key.add("C3", WRONG, "Time translation", 4, r, "DATE is 30 days later than TIME/CLOCK imply (month typo).", "Recompute elapsed hours from DATE+CLOCK minus first dose and compare with TIME.")

    # ---- subject 5 -------------------------------------------------------------------
    s = subj[5]
    r = pick(s, "obs", 8)
    vals = [str(r[c]) for c in QC_COLS]
    vals[QC_COLS.index("DV")] = "4,7"
    r["_raw"] = ",".join(vals)
    key.add("A5", ERR, "NONMEM readability", 5, r, "DV written with a decimal comma (4,7) in a comma-separated file: 25 fields.", "Field-count check; also search for ',' inside what should be one numeric value.")
    for n in (24, 48):
        r = pick(s, "obs", n); r["MDV"] = 1
    key.add("B7", WRONG, "Dose/obs coding", 5, [pick(s, "obs", 24), pick(s, "obs", 48)], "Two trough observations carry a DV but MDV=1: they are silently excluded from the fit.", "For EVID=0 rows cross-tabulate MDV against 'DV present'.")
    tail = s[-4:]
    for r in tail:
        d = datetime.strptime(r["DATE"], "%Y-%m-%d")
        r["DATE"] = d.strftime("%d/%m/%Y")
    key.add("C5", WRONG, "Time translation", 5, tail, "Last four records use DD/MM/YYYY dates while all others are ISO YYYY-MM-DD.", "Parse every DATE with ONE format; mixed formats make day/month swaps possible.")

    # ---- subject 6 -------------------------------------------------------------------
    s = subj[6]
    r = pick(s, "obs", 12); r["DV"] = "NA"
    key.add("A3", ERR, "NONMEM readability", 6, r, "DV is the text 'NA'.", "Same scan as A1.")
    r = pick(s, "obs", 4); r["EVID"], r["MDV"] = 1, 1
    key.add("B2", ERR, "Dose/obs coding", 6, r, "Observation row coded EVID=1 with AMT=0: a zero-amount dose.", "EVID=1 requires AMT>0; observations are EVID=0.")
    for r in s:
        r["TIME"] = f2(float(r["TIME"]) + 8)
    key.add("C2", WRONG, "Time translation", 6, s, "TIME is 8 h too large on every record: counted from study day 1 00:00 instead of from the first dose.", "First dose row must have TIME=0 (or a defined reference); compare TIME with DATE+CLOCK minus first-dose DATE+CLOCK.")
    r = pick(s, "dose", dnum=2); r["AMT"] = 1000
    key.add("D2", WRONG, "AMT vs DOSE", 6, r, "AMT=1000 while DOSE=200: an extra zero.", "AMT/DOSE must equal 1 (or a documented fraction).")

    # ---- subject 7 -------------------------------------------------------------------
    s = subj[7]
    r = pick(s, "obs", 8); r["ID"] = ""
    key.add("A7", WRONG, "NONMEM readability", 7, r, "ID is blank on one record (NONMEM would see a new subject 0).", "Check for empty cells in ID/TIME/EVID/CMT.")
    r = pick(s, "obs", 2); r["CMT"] = 3
    key.add("B6", ERR, "Dose/obs coding", 7, r, "Observation with CMT=3 in a model that only has a depot and a central compartment.", "List CMT values by EVID and compare with the model's compartments.")
    r = pick(s, "obs", 12)
    d = datetime.strptime(r["DATE"], "%Y-%m-%d") - timedelta(days=1)
    r["DATE"] = d.strftime("%Y-%m-%d")
    key.add("C4", WRONG, "Time translation", 7, r, "Sample after midnight keeps the previous calendar date (04:00 on the dose day), implying -12 h.", "Elapsed time from DATE+CLOCK goes backwards around midnight.")
    r = pick(s, "dose", dnum=1); r["ADDL"] = 6
    key.add("E5", ERR, "Regimen items", 7, r, "ADDL=6 with II=0: additional doses with no interval.", "ADDL>0 requires II>0.")

    # ---- subject 8 -------------------------------------------------------------------
    s = subj[8]
    removed = [r for r in s if r["_kind"] == "dose"]
    for r in removed:
        s.remove(r)
    first = s[0]
    key.add("B4", ERR, "Dose/obs coding", 8, first, "Subject has 14 observations and NO dose record at all (first row of the subject is flagged).", "Every subject with EVID=0 rows needs at least one EVID=1 row before them.")
    for r in s:
        r["WT"] = 0
        r["SEX"] = "M"
    key.add("E1", WRONG, "Covariate", 8, s, "WT=0 for the whole subject.", "Range-check WT.")
    key.add("E4", ERR, "Covariate", 8, s, "SEX is the letter M (text) in a numeric column.", "Covariates must be numeric; code SEX 0/1.")

    # ---- subject 9 -------------------------------------------------------------------
    s = subj[9]
    for r in s:
        r["TIME"] = f3(float(r["TIME"]) / 24.0)
        r["WT"] = 700
    key.add("C1", WRONG, "Time translation", 9, s, "TIME in DAYS (0.042, 0.083, ...) while TAD and every other subject are in hours.", "Look at the TIME range per subject: 48 h of sampling should reach ~48, not ~2.")
    key.add("E2", WRONG, "Covariate", 9, s, "WT=700 (likely 70.0 or a lb/kg slip).", "Range-check WT.")
    r = pick(s, "obs", 1); r["TIME"] = "1h"
    key.add("A6", ERR, "NONMEM readability", 9, r, "TIME is the text '1h'.", "Numeric check on TIME.")

    # ---- subject 10 ------------------------------------------------------------------
    s = subj[10]
    a, b = pick(s, "obs", 8), pick(s, "obs", 12)
    ia, ib = s.index(a), s.index(b)
    s[ia], s[ib] = b, a
    key.add("A9", ERR, "NONMEM readability", 10, [a, b], "Records for the 12 h and 8 h samples are in the wrong order: TIME decreases.", "TIME must be non-decreasing within a subject.")
    r = pick(s, "obs", 28); r["AMT"] = 100
    key.add("B8", WRONG, "Dose/obs coding", 10, r, "Observation row (EVID=0) carries AMT=100.", "AMT>0 only on EVID=1/4 rows.")
    for r in s:
        r["AGE"] = 0
    key.add("E3", WRONG, "Covariate", 10, s, "AGE=0.", "Range-check AGE.")
    r = pick(s, "dose", dnum=1); r["SS"] = 1
    key.add("E6", ERR, "Regimen items", 10, r, "SS=1 with II=0: steady state requested without a dosing interval.", "SS>0 requires II>0.")
    r = pick(s, "dose", dnum=2); r["AMT"] = -100
    key.add("D4", WRONG, "AMT vs DOSE", 10, r, "AMT=-100.", "AMT must not be negative on EVID=1 rows.")

    # ---- flatten, then the non-contiguous ID move (A8) -------------------------------
    blocks = [subj[i] for i in range(1, 11)]
    moved = blocks[3][-3:]
    blocks[3] = blocks[3][:-3]
    blocks[4] = blocks[4] + moved
    key.add("A8", WRONG, "NONMEM readability", 4, moved, "The last three records of subject 4 sit after subject 5: ID 4 is not contiguous.", "Check that each ID forms ONE block; NONMEM would treat the strays as another subject.")
    final = [r for b in blocks for r in b]
    key.resolve(final)
    write_lines(HERE / "qc_many_errors.csv", [",".join(QC_COLS)] + [serialise(r) for r in final])
    return key, final


def write_key(clean_key, err_key):
    L = [
        "# QC answer key",
        "",
        "Program: data-obfuscator example data  |  Author: Obinna Obianom  |  Date: 2026-10-08",
        "",
        "Generated by `make_datasets.py`. All data are simulated. Line numbers count the header as line 1,",
        "so they match what a text editor shows.",
        "",
        "Severity labels (the author's classification of *typical* behaviour; exact messages depend on your",
        "NONMEM version, `$INPUT` and `$PK`):",
        "",
        "- **WARN**: reads and runs; a careful reviewer would still query it.",
        "- **ERROR**: NM-TRAN or the NONMEM run is expected to stop with a data error.",
        "- **SILENT**: NONMEM reads it without complaint but the model is fed wrong information.",
        "",
        "## qc_nearly_clean.csv - 6 findings, all WARN",
        "",
        "| Code | Subj | Lines | Category | What is wrong | How to detect it |",
        "|---|---|---|---|---|---|",
    ]
    for it in clean_key.items:
        L.append(f"| {it['code']} | {it['sid']} | {clean_key.lines_text(it)} | {it['cat']} | {it['what']} | {it['how']} |")
    L += [
        "",
        "Everything else in this file is meant to be correct: TIME agrees with DATE + CLOCK, TAD is time since",
        "the last dose, AMT equals DOSE on every dose row (except W2), dose rows use EVID=1/MDV=1/CMT=1, observation",
        "rows use EVID=0/MDV=0/CMT=2, and records are in time order with each subject contiguous.",
        "",
        f"## qc_many_errors.csv - {len(err_key.items)} planted findings",
        "",
        "| Code | Subj | Lines | Severity | Category | What is wrong | How to detect it |",
        "|---|---|---|---|---|---|---|",
    ]
    for it in sorted(err_key.items, key=lambda x: (x["code"][0], int(x["code"][1:]))):
        L.append(f"| {it['code']} | {it['sid']} | {err_key.lines_text(it)} | {it['sev']} | {it['cat']} | {it['what']} | {it['how']} |")
    L += [
        "",
        "Code prefixes: A = NONMEM readability, B = dose/observation coding, C = time translation,",
        "D = AMT versus DOSE, E = covariates and regimen items (ADDL/II/SS).",
        "",
        "A note on subject 4 (code A8): its three displaced records are listed under that code; their DV values",
        "are otherwise fine.",
        "",
    ]
    write_lines(HERE / "qc_answer_key.md", L)


# ----------------------------------------------------------------------------- IV infusion
def biexp(cl, v1, q, v2):
    k10, k12, k21 = cl / v1, q / v1, q / v2
    s = k10 + k12 + k21
    root = math.sqrt(s * s - 4 * k10 * k21)
    al, be = (s + root) / 2, (s - root) / 2
    a = (al - k21) / (v1 * (al - be))
    b = (k21 - be) / (v1 * (al - be))
    return a, al, b, be


def iv_conc(t, doses, par, dur=1.0):
    """2-compartment IV, zero-order input over `dur` h for each (time_h, amount_mg)."""
    a, al, b, be = par
    c = 0.0
    for td, amt in doses:
        x = t - td
        if x <= 0:
            continue
        rate = amt / dur
        if x <= dur:
            c += rate * (a / al * (1 - math.exp(-al * x)) + b / be * (1 - math.exp(-be * x)))
        else:
            c += rate * (a / al * math.exp(-al * (x - dur)) * (1 - math.exp(-al * dur))
                         + b / be * math.exp(-be * (x - dur)) * (1 - math.exp(-be * dur)))
    return c


def build_iv():
    rng = random.Random(20261010)
    cols = ["ID", "USUBJID", "STUDYID", "SITEID", "COHORT", "TIME", "TAD", "EVID", "MDV", "CMT",
            "AMT", "RATE", "ADDL", "II", "SS", "DV", "WT", "AGE", "SEX", "CRCL", "OCC"]
    out = []
    sid = 0
    cohorts = [
        ("A", 50, "single", [0.5, 1, 1.5, 2, 4, 8, 12, 24, 48]),
        ("B", 100, "addl", [1, 4, 12, 23.9, 95.9, 97, 100, 108, 120]),
        ("C", 200, "ss", [0.5, 1, 2, 4, 8, 11.5]),
    ]
    for cname, dose, mode, times in cohorts:
        for k in range(6):
            sid += 1
            site = "S01" if k < 3 else "S02"
            wt = round(clip(rng.gauss(78, 13), 50, 115), 1)
            age = int(clip(rng.gauss(55, 12), 25, 80))
            sex = rng.choice([0, 1])
            crcl = int(clip(rng.gauss(90, 22), 40, 150))
            cl = 5 * (wt / 70) ** 0.75 * (crcl / 100) ** 0.4 * lognorm(rng, 0.25)
            v1 = 15 * (wt / 70) * lognorm(rng, 0.2)
            par = biexp(cl, v1, 8 * lognorm(rng, 0.2), 30 * lognorm(rng, 0.25))
            if mode == "single":
                hist = [(0.0, dose)]
            elif mode == "addl":
                hist = [(24.0 * i, dose) for i in range(5)]
            else:  # steady state, 12-hourly: 40 prior doses stand in for infinity
                hist = [(-12.0 * i, dose) for i in range(0, 41)]
            base = dict(ID=sid, USUBJID=f"IVX-201-{site[1:]}-{sid:03d}", STUDYID="IVX-201", SITEID=site,
                        COHORT=cname, WT=wt, AGE=age, SEX=sex, CRCL=crcl, OCC=1)
            drow = dict(base, TIME="0", TAD="0", EVID=1, MDV=1, CMT=1, AMT=dose, RATE=dose,
                        ADDL=4 if mode == "addl" else 0, II={"addl": 24, "ss": 12}.get(mode, 0),
                        SS=1 if mode == "ss" else 0, DV=".")
            rows = [drow]
            for t in times:
                tt = t + rng.randint(-3, 3) / 60.0
                tt = round(tt, 2)
                # time since the most recent dose (cohort B: doses at 0, 24, ..., 96; others: dose at 0)
                tad = tt - 24.0 * int(min(tt, 96) // 24) if mode == "addl" else tt
                conc = iv_conc(tt, hist, par) * lognorm(rng, 0.12)
                rows.append(dict(base, TIME=g(tt), TAD=f2(tad), EVID=0, MDV=0, CMT=1, AMT=".", RATE=".",
                                 ADDL=".", II=".", SS=".", DV=f3(conc)))
            # NONMEM order: time ascending (dose first); pre-dose troughs (23.9, 95.9) come before dose rows
            # of the same day only because ADDL expands them internally, so a single dose row is enough.
            rows.sort(key=lambda r: (float(r["TIME"]), 0 if r["EVID"] == 1 else 1))
            for r in rows:
                out.append(",".join(str(r.get(c, ".")) for c in cols))
    write_lines(HERE / "pk_iv_infusion_addl_ss.csv", [",".join(cols)] + out)


# ----------------------------------------------------------------------------- pooled oral, multi-ID columns
def build_pooled():
    rng = random.Random(20261011)
    cols = ["ID", "USUBJID", "SUBJID", "STU", "STUDYID", "STUDY", "PROTNAME", "SITEID", "SITENAME",
            "INVESTIGATOR", "COUNTRY", "ARM", "ARMCD", "DATE", "TIME", "TAD", "EVID", "MDV", "CMT",
            "AMT", "ADDL", "II", "DV", "BLQ", "LLOQ", "WT", "AGE", "SEX", "RACE", "CRCL", "ALB", "COMMENT"]
    studies = [
        dict(stu=1, sid="XR-101", name="Phase 1 SAD", prot="Single ascending dose of XR-7 in healthy adults",
             start=datetime(2023, 2, 6, 8, 0), regimen="sad", doses=[50, 100, 200, 400]),
        dict(stu=2, sid="XR-102", name="Phase 1 MAD", prot="Multiple ascending dose of XR-7 in healthy adults",
             start=datetime(2023, 9, 11, 8, 0), regimen="mad", doses=[100, 200]),
        dict(stu=3, sid="XR-201", name="Phase 2 Dose Finding", prot="Dose finding of XR-7 in patients with sparse sampling",
             start=datetime(2024, 1, 15, 9, 0), regimen="sparse", doses=[150]),
    ]
    sites = {
        "S01": ("Harbor Point Research Unit", "Dr. Okafor", "United States"),
        "S02": ("Eastvale Medical Center", "Dr. Novak", "Germany"),
        "S03": ("Lakeshore Clinical Trials Unit", "Dr. Silva", "Brazil"),
    }
    races = ["White", "Black", "Asian", "Other"]
    LLOQ = 0.05
    out = []
    nid = 0
    for st in studies:
        for k in range(8):
            nid += 1
            dose = st["doses"][k % len(st["doses"])] if st["regimen"] != "sad" else st["doses"][k // 2]
            site = ["S01", "S02", "S03"][k % 3] if st["stu"] != 3 else ["S02", "S03"][k % 2]
            sname, inv, country = sites[site]
            wt = round(clip(rng.gauss(76, 14), 48, 120), 1)
            age = int(clip(rng.gauss(38 if st["stu"] < 3 else 57, 11), 18, 78))
            sex = rng.choice([0, 1])
            crcl = int(clip(rng.gauss(105, 20), 50, 160))
            alb = round(clip(rng.gauss(4.2 if st["stu"] < 3 else 3.7, 0.4), 2.6, 5.2), 1)
            race = rng.choice(races)
            ka = 1.0 * lognorm(rng, 0.3)
            v = 45 * (wt / 70) * lognorm(rng, 0.2)
            cl = 7 * (wt / 70) ** 0.75 * lognorm(rng, 0.25)
            ke = cl / v
            if abs(ka - ke) < 0.05:
                ka += 0.2
            start = st["start"] + timedelta(days=2 * (k // 3), minutes=15 * (k % 4))
            subj_no = 1000 * st["stu"] + k + 1
            usubjid = f"{st['sid']}-{site}-{subj_no:04d}"
            arm = f"{dose}mg_" + {"sad": "single", "mad": "QD5", "sparse": "QD28"}[st["regimen"]]
            armcd = f"{st['sid'][-3:]}D{dose}"
            # dosing history (hours)
            if st["regimen"] == "sad":
                dose_times = [0.0]
                obs = [0, 0.5, 1, 2, 4, 8, 12, 24, 48]
                dose_rows = [(0.0, dose, 0, 0)]
            elif st["regimen"] == "mad":
                dose_times = [24.0 * i for i in range(5)]
                obs = [0, 1, 2, 4, 8, 24, 48, 72, 96, 97, 98, 100, 104, 108, 120]
                dose_rows = [(24.0 * i, dose, 0, 0) for i in range(5)]
            else:
                dose_times = [24.0 * i for i in range(28)]
                obs = [2, 12, 167.9, 335.9, 671.9]
                dose_rows = [(0.0, dose, 27, 24)]
            hist = [(t, dose) for t in dose_times]
            events = []
            for (t, amt, addl, ii) in dose_rows:
                events.append((t, 0, "dose", amt, addl, ii))
            for t in obs:
                jitter = rng.randint(-3, 3) / 60.0 if t not in (0,) else 0.0
                tt = round(max(0.0, t + jitter), 2) if t != 0 else 0.0
                events.append((tt, -1 if t == 0 else 1, "obs", 0, 0, 0))
            events.sort(key=lambda e: (e[0], e[1]))
            for (t, _, kind, amt, addl, ii) in events:
                dt = start + timedelta(hours=t)
                last = max([x for x in dose_times if x <= t + 1e-9], default=0.0)
                if kind == "dose":
                    row = dict(EVID=1, MDV=1, CMT=1, AMT=amt, ADDL=addl, II=ii, DV=".", BLQ=".", TAD="0")
                else:
                    c = oral_conc(t, hist, ka, ke, v) * lognorm(rng, 0.18)
                    blq = 1 if (c < LLOQ) else 0
                    tad = (t - last) if t > 0 or any(x == 0 for x in dose_times) else 0
                    row = dict(EVID=0, MDV=0, CMT=2, AMT=".", ADDL=".", II=".",
                               DV=f3(LLOQ if blq else c), BLQ=blq, TAD=f2(tad))
                    if t == 0:
                        row.update(DV=f3(LLOQ), BLQ=1)  # pre-dose sample, below the limit by definition
                comment = ""
                if kind == "obs" and rng.random() < 0.03:
                    comment = rng.choice(["hemolyzed sample", "vomited within 1 h of dose", "late draw per site log"])
                r = dict(ID=nid, USUBJID=usubjid, SUBJID=subj_no, STU=st["stu"], STUDYID=st["sid"],
                         STUDY=st["name"], PROTNAME=st["prot"], SITEID=site, SITENAME=sname,
                         INVESTIGATOR=inv, COUNTRY=country, ARM=arm, ARMCD=armcd,
                         DATE=dt.strftime("%Y-%m-%d"), TIME=g(t) if kind == "dose" else f2(t), LLOQ=LLOQ,
                         WT=wt, AGE=age, SEX=sex, RACE=race, CRCL=crcl, ALB=alb, COMMENT=comment)
                r.update(row)
                out.append(",".join(str(r[c]) for c in cols))
    write_lines(HERE / "pk_oral_pooled_multi_id.csv", [",".join(cols)] + out)


# ----------------------------------------------------------------------------- PK/PD, tab separated
def build_pkpd():
    rng = random.Random(20261012)
    cols = ["ID", "USUBJID", "STUDYID", "TRT", "VISIT", "SAMPLEID", "DATE", "TIME", "TAD", "EVID", "MDV",
            "CMT", "DVID", "AMT", "DOSE", "DV", "BASE", "WT", "AGE", "SEX", "OCC", "COMMENT"]
    out = []
    pk_t = [0.5, 1, 2, 4, 8, 12, 24, 48]
    pd_t = [0, 2, 4, 8, 12, 24, 48, 72, 96]
    ke0, imax, ic50 = 0.25, 0.9, 1.0
    sid = 0
    for dose in (50, 100, 200):
        for k in range(4):
            sid += 1
            wt = round(clip(rng.gauss(74, 11), 52, 105), 1)
            age = int(clip(rng.gauss(36, 9), 20, 60))
            sex = rng.choice([0, 1])
            base = round(clip(rng.gauss(100, 8), 80, 125), 1)
            ka = 1.1 * lognorm(rng, 0.3)
            v = 40 * (wt / 70) * lognorm(rng, 0.2)
            cl = 6 * (wt / 70) ** 0.75 * lognorm(rng, 0.25)
            ke = cl / v
            if abs(ka - ke) < 0.05:
                ka += 0.2
            ic = ic50 * lognorm(rng, 0.3)
            start = datetime(2024, 9, 2, 8, 0) + timedelta(days=k // 2, minutes=20 * (sid % 3))

            def ce(t):
                # effect-site concentration: ke0 * integral of Cp(s) exp(-ke0 (t-s)) ds, midpoint rule
                n = max(1, int(t / 0.05))
                h = t / n
                tot = 0.0
                for i in range(n):
                    s = (i + 0.5) * h
                    tot += oral_conc(s, [(0.0, dose)], ka, ke, v) * math.exp(-ke0 * (t - s)) * h
                return ke0 * tot

            ev = [(0.0, "dose", -1)]
            ev += [(t + rng.randint(-3, 3) / 60.0 if t else 0.0, "pk", 1) for t in pk_t]
            ev += [(t + (rng.randint(-3, 3) / 60.0 if t else 0.0), "pd", 1 if t else 0) for t in pd_t]
            ev.sort(key=lambda e: (round(e[0], 2), e[2]))
            for t, kind, _ in ev:
                t = round(max(0.0, t), 2)
                dt = start + timedelta(hours=t)
                day = int(t // 24) + 1
                r = dict(ID=sid, USUBJID=f"PDX-301-{sid:03d}", STUDYID="PDX-301", TRT=f"{dose}mg",
                         VISIT=day, SAMPLEID=f"PDX301-{sid:03d}-{kind.upper()}-{t:06.2f}",
                         DATE=dt.strftime("%Y-%m-%d"), TIME=g(t), TAD=g(t), DOSE=dose, BASE=base,
                         WT=wt, AGE=age, SEX=sex, OCC=1, COMMENT="")
                if kind == "dose":
                    r.update(EVID=1, MDV=1, CMT=1, DVID=0, AMT=dose, DV=".", TAD="0", SAMPLEID=".")
                elif kind == "pk":
                    c = oral_conc(t, [(0.0, dose)], ka, ke, v) * lognorm(rng, 0.15)
                    r.update(EVID=0, MDV=0, CMT=2, DVID=1, AMT=".", DV=f3(c))
                else:
                    eff = base * (1 - imax * ce(t) / (ic + ce(t))) if t > 0 else base
                    eff *= 1 + rng.gauss(0, 0.04)
                    r.update(EVID=0, MDV=0, CMT=3, DVID=2, AMT=".", DV=f"{eff:.1f}")
                if kind != "dose" and rng.random() < 0.03:
                    r["COMMENT"] = rng.choice(["fasting sample", "patient reported nausea", "re-drawn after clotting"])
                out.append("\t".join(str(r[c]) for c in cols))
    write_lines(HERE / "pkpd_dvid_endpoints.tsv", ["\t".join(cols)] + out)


if __name__ == "__main__":
    ck, _ = build_clean()
    ek, _ = build_errors()
    write_key(ck, ek)
    build_iv()
    build_pooled()
    build_pkpd()
    for n in ["qc_nearly_clean.csv", "qc_many_errors.csv", "pk_iv_infusion_addl_ss.csv",
              "pk_oral_pooled_multi_id.csv", "pkpd_dvid_endpoints.tsv", "qc_answer_key.md"]:
        p = HERE / n
        print(f"{n:32s} {len(p.read_text().splitlines()):5d} lines")
