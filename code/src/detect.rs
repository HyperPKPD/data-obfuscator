//! Column-role detection for NONMEM / CDISC-ish headers.

use crate::config::RunOptions;
use crate::dataset::Dataset;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Id,
    Subject,
    Study,
    Site,
    Treatment,
    User,
    Date,
    Sex,
    Race,
    Country,
    FreeText,
    Covariate,
    Protect,
    Other,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Subject => "subject",
            Self::Study => "study",
            Self::Site => "site",
            Self::Treatment => "treatment",
            Self::User => "user",
            Self::Date => "date",
            Self::Sex => "sex",
            Self::Race => "race",
            Self::Country => "country",
            Self::FreeText => "freetext",
            Self::Covariate => "covariate",
            Self::Protect => "protect",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ColumnPlan {
    pub name: String,
    pub index: usize,
    pub role: Role,
    pub recode: bool,
    pub scale: bool,
    pub redact: bool,
    pub drop: bool,
    pub protect: bool,
}

pub fn detect_role(name: &str) -> Role {
    let n = name.trim().to_ascii_uppercase().replace([' ', '-'], "_");
    let n = n.as_str();
    if matches!(n, "ID" | "I" | "NMID") {
        return Role::Id;
    }
    if matches!(
        n,
        "USUBJID" | "SUBJID" | "SUBJECT" | "PATIENT" | "PTID" | "PID" | "SUBJ"
    ) {
        return Role::Subject;
    }
    if matches!(n, "STUDY" | "STU" | "STDY" | "STUDYID" | "STUDYN" | "PROTO" | "PROTOCOL" | "STUDYNUM" | "TRIAL") {
        return Role::Study;
    }
    if matches!(n, "SITE" | "SITEID" | "CENTER" | "CENTRE" | "CENTERNO" | "SITEIDN") {
        return Role::Site;
    }
    if matches!(
        n,
        "ARM" | "TRT" | "TREAT" | "TREATMENT" | "GROUP" | "COHORT" | "TRTGRP" | "TRTA" | "TRTP"
            | "ARMCD" | "ACTARM" | "DOSEGRP"
    ) {
        return Role::Treatment;
    }
    if matches!(
        n,
        "USERID" | "USER" | "INVESTIGATOR" | "INV" | "CRA" | "OPERATOR" | "ANALYST"
    ) {
        return Role::User;
    }
    if n == "DATE"
        || n == "DATETIME"
        || n.ends_with("DTC")
        || n.ends_with("_DT")
        || matches!(n, "DOB" | "BIRTH" | "BRTHDAT" | "RFSTDTC" | "RFENDTC" | "VISITDT")
    {
        return Role::Date;
    }
    if matches!(n, "SEX" | "GENDER" | "SEXN") {
        return Role::Sex;
    }
    if matches!(n, "RACE" | "ETHNIC" | "ETHNICITY" | "RACEN") {
        return Role::Race;
    }
    if matches!(n, "COUNTRY" | "COUNTRYN" | "NAT") {
        return Role::Country;
    }
    if matches!(
        n,
        "COMMENT" | "COMMENTS" | "NOTE" | "NOTES" | "TEXT" | "EMAIL" | "PHONE" | "NAME"
            | "INITIALS" | "INIT" | "SUBJINIT" | "ADDRESS" | "FREE"
    ) {
        return Role::FreeText;
    }
    if matches!(
        n,
        "WT" | "WGT" | "WEIGHT" | "AGE" | "HT" | "HEIGHT" | "BMI" | "CRCL" | "EGFR" | "SCR"
            | "ALT" | "AST" | "BILI" | "ALB" | "BSA" | "LBWT"
    ) {
        return Role::Covariate;
    }
    if matches!(
        n,
        "TIME" | "TAD" | "DV" | "LNDV" | "AMT" | "RATE" | "EVID" | "MDV" | "CMT" | "ADDL" | "II"
            | "SS" | "DVID" | "CENS" | "LIMIT" | "BLQ" | "L2" | "OCC" | "PC" | "PCYC"
    ) {
        return Role::Protect;
    }
    Role::Other
}

fn force_role(name: &str, opts: &RunOptions) -> Option<Role> {
    let eq = |a: &Vec<String>| a.iter().any(|x| x.trim().eq_ignore_ascii_case(name.trim()));
    if eq(&opts.id_column) {
        return Some(Role::Id);
    }
    if eq(&opts.subject_column) {
        return Some(Role::Subject);
    }
    if eq(&opts.study_column) {
        return Some(Role::Study);
    }
    if eq(&opts.site_column) {
        return Some(Role::Site);
    }
    if eq(&opts.treatment_column) {
        return Some(Role::Treatment);
    }
    if eq(&opts.user_column) {
        return Some(Role::User);
    }
    if opts.date_columns.iter().any(|x| x.eq_ignore_ascii_case(name)) {
        return Some(Role::Date);
    }
    if opts.freetext_columns.iter().any(|x| x.eq_ignore_ascii_case(name)) {
        return Some(Role::FreeText);
    }
    if opts.scale_columns.iter().any(|x| x.eq_ignore_ascii_case(name)) {
        return Some(Role::Covariate);
    }
    if opts.protect.iter().any(|x| x.eq_ignore_ascii_case(name)) {
        return Some(Role::Protect);
    }
    if opts.sensitive.iter().any(|x| x.eq_ignore_ascii_case(name)) {
        return Some(Role::Subject);
    }
    None
}

pub fn plan_columns(ds: &Dataset, opts: &RunOptions) -> Vec<ColumnPlan> {
    let level = opts.level;
    let drop_freetext = opts.drop_freetext || level >= 4;
    let keep_set: Option<Vec<String>> = opts.keep.clone();

    ds.headers
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let role = force_role(name, opts).unwrap_or_else(|| detect_role(name));
            let protect = role == Role::Protect
                || opts
                    .protect
                    .iter()
                    .any(|x| x.eq_ignore_ascii_case(name));
            let recode = !protect
                && matches!(
                    role,
                    Role::Id
                        | Role::Subject
                        | Role::Study
                        | Role::Site
                        | Role::User
                        | Role::Treatment
                        | Role::Sex
                        | Role::Race
                        | Role::Country
                )
                && match role {
                    Role::Id | Role::Subject | Role::Study | Role::Site | Role::User => true,
                    Role::Treatment | Role::Sex | Role::Race | Role::Country => level >= 2,
                    _ => false,
                };
            let scale = !protect && role == Role::Covariate && level >= 3;
            let redact = !protect && role == Role::FreeText && level >= 2;
            let date_shift = !protect && role == Role::Date && level >= 2;
            let recode = recode || date_shift;
            let in_drop = opts.drop.iter().any(|x| x.eq_ignore_ascii_case(name));
            let in_keep = keep_set
                .as_ref()
                .map(|ks| ks.iter().any(|x| x.eq_ignore_ascii_case(name)))
                .unwrap_or(true);
            let drop_this_freetext = drop_freetext && role == Role::FreeText && keep_set
                .as_ref()
                .map(|ks| !ks.iter().any(|x| x.eq_ignore_ascii_case(name)))
                .unwrap_or(true);
            let drop = in_drop || !in_keep || drop_this_freetext;
            ColumnPlan {
                name: name.clone(),
                index,
                role,
                recode,
                scale,
                redact,
                drop,
                protect,
            }
        })
        .collect()
}

pub fn output_headers(plan: &[ColumnPlan], keep: &Option<Vec<String>>, original: &[String]) -> Vec<String> {
    if let Some(ks) = keep {
        ks.iter()
            .filter(|k| {
                original.iter().any(|h| h.eq_ignore_ascii_case(k))
                    && !plan
                        .iter()
                        .find(|p| p.name.eq_ignore_ascii_case(k))
                        .map(|p| p.drop)
                        .unwrap_or(false)
            })
            .map(|k| {
                original
                    .iter()
                    .find(|h| h.eq_ignore_ascii_case(k))
                    .cloned()
                    .unwrap_or_else(|| (*k).clone())
            })
            .collect()
    } else {
        plan.iter()
            .filter(|p| !p.drop)
            .map(|p| p.name.clone())
            .collect()
    }
}
