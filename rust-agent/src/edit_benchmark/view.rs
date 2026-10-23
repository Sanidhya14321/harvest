//! Read-only native view of completed rust-v1 run summaries. Historical scores
//! use a different schema/version and are never silently imported.
use std::{io::Write, path::Path};

use anyhow::{Result, ensure};
use harvest_runtime::JailedFs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
	NotStarted,
	Cancelled,
	Completed,
	TimedOut,
	Failed,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct RunView {
	pub index:            usize,
	pub task_id:          String,
	pub repetition:       u32,
	pub state:            RunState,
	pub success:          bool,
	pub elapsed_micros:   u128,
	// Older native summaries predate attempt accounting. Unknown is not zero.
	pub attempts:         Option<u32>,
	pub no_edit_retries:  Option<u32>,
	pub provider_retries: Option<u32>,
	pub early_stopped:    Option<bool>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct SummaryView {
	pub scoring_version: String,
	pub runs:            Vec<RunView>,
	pub successful:      usize,
	pub unsuccessful:    usize,
}
impl SummaryView {
	pub fn load(output: &Path) -> Result<Self> {
		let filesystem = JailedFs::open(output)?;
		let bytes = filesystem.read(Path::new("summary.json"), 2 * 1024 * 1024)?;
		let summary: Self = serde_json::from_slice(&bytes)?;
		ensure!(
			summary.scoring_version == super::SCORING_VERSION,
			"unsupported benchmark scoring version"
		);
		ensure!(
			!summary.runs.is_empty() && summary.runs.len() <= 1000,
			"invalid benchmark run count"
		);
		for (index, run) in summary.runs.iter().enumerate() {
			ensure!(run.index == index, "unordered or duplicate benchmark run index");
			ensure!(
				!run.task_id.is_empty() && run.task_id.len() <= 1024,
				"invalid benchmark task identity"
			);
			ensure!((1..=100).contains(&run.repetition), "invalid benchmark repetition");
			ensure!(
				!run.success || matches!(run.state, RunState::Completed),
				"successful run has incomplete state"
			);
			ensure!(run.attempts.is_none_or(|n| n <= 48), "invalid benchmark attempt count");
			ensure!(
				run.no_edit_retries
					.is_none_or(|n| n <= 16 && run.attempts.is_some_and(|a| n <= a)),
				"invalid benchmark retry count"
			);
			ensure!(
				run.provider_retries
					.is_none_or(|n| n <= 16 && run.attempts.is_some_and(|a| n <= a)),
				"invalid benchmark provider retry count"
			);
			if let (Some(attempts), Some(no_edit), Some(provider)) =
				(run.attempts, run.no_edit_retries, run.provider_retries)
			{
				ensure!(no_edit + provider <= attempts, "inconsistent benchmark retry total");
			}
		}
		let successful = summary.runs.iter().filter(|run| run.success).count();
		ensure!(
			summary.successful == successful
				&& summary.unsuccessful == summary.runs.len() - successful,
			"inconsistent benchmark summary counts"
		);
		Ok(summary)
	}

	pub fn write_text(&self, output: &mut impl Write) -> Result<()> {
		writeln!(
			output,
			"{}: {} successful, {} unsuccessful",
			self.scoring_version, self.successful, self.unsuccessful
		)?;
		writeln!(
			output,
			"{}",
			concat!(
				"index\ttask\trepetition\tstate\tsuccess\telapsed_us\tattempts",
				"\tno_edit_retries\tprovider_retries\tearly_stopped"
			)
		)?;
		for run in &self.runs {
			// JSON escaping prevents task labels from injecting terminal controls
			// or additional table rows. No paths or files in a summary are opened.
			writeln!(
				output,
				"{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
				run.index,
				serde_json::to_string(&run.task_id)?,
				run.repetition,
				serde_json::to_string(&run.state)?,
				run.success,
				run.elapsed_micros,
				run.attempts
					.map_or_else(|| "unavailable".into(), |n| n.to_string()),
				run.no_edit_retries
					.map_or_else(|| "unavailable".into(), |n| n.to_string()),
				run.provider_retries
					.map_or_else(|| "unavailable".into(), |n| n.to_string()),
				run.early_stopped
					.map_or_else(|| "unavailable".into(), |n| n.to_string())
			)?;
		}
		Ok(())
	}
}
