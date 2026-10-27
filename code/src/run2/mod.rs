//! `run2`: mask SEVERAL related datasets (for example CDISC ADaM ADSL, ADPC, ADLB) in one pass so that
//! they can be given to an LLM, which combines them into one NONMEM-style master dataset. The masked
//! datasets share their identifier codes (the same USUBJID has the same code in every file), one date
//! shift and one factor per scaled column, so the LLM can still join them. `keymap2.txt` records all of
//! it, and `reverse` (or the library function `reverse2::reverse2`) puts the real values back into the
//! combined dataset the LLM returns.
//!
//! Everything for the new command lives in this folder. `run`, `reverse` and their files are untouched
//! apart from a small dispatch in `reverse` that hands a `keymap2.txt` over to `reverse2`.

pub mod args;
pub mod keymap2;
pub mod mask;
pub mod prompt;
pub mod reverse2;
pub mod runner;

pub use args::{resolve_run2, Run2Args, Run2Options};
pub use runner::{run2, Run2Artifacts};
