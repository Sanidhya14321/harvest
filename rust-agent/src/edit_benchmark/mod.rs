//! Separately versioned Rust edit scoring. Historical Babel/Prettier scores are
//! never loaded as rust-v1 results. Fixture source text is inert input data.
mod archive;
pub(crate) mod cli;
pub use archive::{ArchiveReport, FixtureSource, PackedArchive, pack_fixtures, unpack_fixtures};
mod evidence;
pub use evidence::{ToolEvidence, tool_evidence};
mod calibration;
pub use calibration::{
	CALIBRATION_VERSION, CONTENT_VERSION, CalibrationSource, ContentIdentity, GenerationQuota,
	MeasuredShape, QuotaPlan, ShapeConstraint, measure_shape, plan_calibrated_quotas, plan_quotas,
};
mod batch;
pub use batch::{
	BATCH_VERSION, GenerationBatch, GenerationCase, GenerationCaseOutcome, generate_batch,
	generate_quota_batch,
};
mod corpus;
pub use corpus::{
	CorpusEntry, CorpusReport, SourcePolicy, inspect_corpus, inspect_corpus_with_difficulty,
};
mod difficulty;
pub use difficulty::{
	DIFFICULTY_VERSION, Difficulty, DifficultyMetrics, FunctionRange, difficulty_metrics,
};
mod regions;
pub use regions::{MutationRegion, REGION_VERSION};
mod generate;
pub use generate::{
	DifficultySelection, GenerationOptions, GenerationReport, RejectedSource, generate_task,
	generate_task_with_options, generate_task_with_policy,
};
pub use pi_ast::mutations::MutationKind;
pub mod hunks;
mod prepare;
mod retry;
mod runner;
mod score;
mod shapes;
mod tasks;
mod view;
pub use prepare::{PreparedTask, prepare_task};
pub use runner::{RunConfiguration, RunOutcome, RunSummary, run_tasks};
pub use score::{Edit, EditOutcome, FileOutcome, Report, apply_and_score, compare};
pub use shapes::{ChangeRun, Distribution, ShapeReport, analyze_shapes, change_runs};
pub use tasks::{FixtureIssue, Task, list_files, load_tasks, validate_fixtures};
pub use view::{RunState, RunView, SummaryView};
pub const SCORING_VERSION: &str = "rust-v1";

mod guidance;
