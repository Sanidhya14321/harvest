//! Separately versioned Rust edit scoring. Historical Babel/Prettier scores are
//! never loaded as rust-v1 results. Fixture source text is inert input data.
pub(crate) mod cli;
pub mod hunks;
mod prepare;
mod runner;
mod score;
mod shapes;
mod tasks;
pub use prepare::{PreparedTask, prepare_task};
pub use runner::{RunConfiguration, RunOutcome, RunSummary, run_tasks};
pub use score::{Edit, EditOutcome, FileOutcome, Report, apply_and_score, compare};
pub use shapes::{ChangeRun, Distribution, ShapeReport, analyze_shapes, change_runs};
pub use tasks::{FixtureIssue, Task, list_files, load_tasks, validate_fixtures};
pub const SCORING_VERSION: &str = "rust-v1";
