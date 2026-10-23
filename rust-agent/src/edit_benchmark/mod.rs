//! Separately versioned Rust edit scoring. Historical Babel/Prettier scores are
//! never loaded as rust-v1 results. Fixture source text is inert input data.
pub(crate) mod cli;
mod score;
mod tasks;
pub use score::{Edit, EditOutcome, FileOutcome, Report, apply_and_score, compare};
pub use tasks::{Task, list_files, load_tasks};
pub const SCORING_VERSION: &str = "rust-v1";
