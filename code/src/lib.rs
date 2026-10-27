//! Data Obfuscator — recode NONMEM-style pharmacometric datasets so they can
//! be shared with models and collaborators without exposing source identities,
//! while preserving PK/PD structure (within-subject TIME order, doses, DV).

pub mod cli;
pub mod config;
pub mod dataset;
pub mod detect;
pub mod obfuscate;
pub mod rename;
pub mod reverse;
pub mod run2;
pub mod summary;
pub mod tokens;
pub mod trace;
pub mod ui;
pub mod llm;

pub use config::{parse_config_file, IdStyle, RunOptions, TokensOptions};
pub use dataset::{Dataset, Delimiter};
pub use obfuscate::{obfuscate, RunArtifacts};
pub use tokens::{count_tokens, pretokens, write_token_report, TokenReport, MODELS};
pub use trace::TraceLog;

pub const APP_NAME: &str = "data-obfuscator";
/// 4-part version from `VERSION` (managed by build.rs; bumps whenever the code changes).
pub const APP_VERSION: &str = env!("DATA_OBFUSCATOR_VERSION");
/// Date (UTC) the current version was cut.
pub const APP_RELEASED: &str = env!("DATA_OBFUSCATOR_RELEASED");
