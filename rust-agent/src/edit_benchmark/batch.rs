//! Bounded serial fixture collection generation. Source snapshots, selection,
//! publication and region policies reuse their existing native owners.
use std::{collections::BTreeSet, path::Path};

use anyhow::{Result, ensure};
use harvest_runtime::{
	JailedFs, MAX_FILE_BYTES, cancel::CancelToken, content_hash, limits::BoundedWriter,
};
use serde::{Deserialize, Serialize};

use super::{GenerationOptions, GenerationReport, MutationKind};

pub const BATCH_VERSION: &str = "rust-batch-v1";
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationCase {
	pub task_id:  String,
	pub mutation: String,
	pub seed:     u64,
}
#[derive(Debug, Serialize)]
pub struct GenerationCaseOutcome {
	pub task_id: String,
	pub state:   &'static str,
	pub report:  Option<GenerationReport>,
	pub error:   Option<String>,
}
#[derive(Debug, Serialize)]
pub struct GenerationBatch {
	pub version:      &'static str,
	pub complete:     bool,
	pub successful:   usize,
	pub unsuccessful: usize,
	pub cases:        Vec<GenerationCaseOutcome>,
}
fn publish(
	owner: &super::prepare::TaskCollection,
	path: &Path,
	value: &impl Serialize,
) -> Result<()> {
	let mut bytes = BoundedWriter::new(4 * 1024 * 1024);
	serde_json::to_writer(&mut bytes, value)?;
	let bytes = bytes.into_inner();
	owner.charge(bytes.len())?;
	owner.filesystem.atomic_create(path, &bytes)
}
/// Generate an explicitly ordered plan against one immutable source snapshot.
/// No resume, automatic source execution, parallel writer or archive
/// publication.
pub fn generate_batch(
	source: &Path,
	output: &Path,
	cases: &[GenerationCase],
	options: &GenerationOptions,
	cancellation: &CancelToken,
) -> Result<GenerationBatch> {
	ensure!(!cases.is_empty() && cases.len() <= 64, "generation plan must contain 1..64 cases");
	ensure!(
		options.difficulty.is_some() || options.min_score.is_none(),
		"minimum score requires explicit difficulty"
	);
	super::regions::Exclusions::new(&options.excluded_regions, cancellation)?;
	let mut ids = BTreeSet::new();
	let mut kinds = Vec::with_capacity(cases.len());
	for case in cases {
		cancellation.heartbeat()?;
		super::prepare::validate_task_id(&case.task_id)?;
		let portable = Path::new("__batch__").join(&case.task_id);
		ensure!(super::archive::normalized(&portable)? == portable, "task identity is not portable");
		ensure!(ids.insert(case.task_id.to_lowercase()), "duplicate or case-aliased task identity");
		kinds.push(
			MutationKind::from_name(&case.mutation)
				.ok_or_else(|| anyhow::anyhow!("unknown planned mutation"))?,
		);
	}
	let source = JailedFs::open(source)?;
	let proposed = super::prepare::prospective_output(output)?;
	ensure!(
		!proposed.starts_with(source.root()) && !source.root().starts_with(&proposed),
		"batch output and original source must be disjoint"
	);
	let corpus = options.source_policy.inspect(&source, cancellation)?;
	let selected: Vec<_> = corpus
		.files
		.iter()
		.filter(|file| {
			matches!(options.source_policy, super::SourcePolicy::ReviewedTree) || file.eligible
		})
		.collect();
	ensure!(!selected.is_empty(), "batch source selection is empty");
	// Bound the entire collection below the shared archive/task traversal
	// ceilings, reserving metadata space before creating an output directory.
	let bytes: usize = selected.iter().map(|entry| entry.bytes).sum();
	ensure!(
		(bytes as u64 * 2 + 512 * 1024) * cases.len() as u64 <= 48 * 1024 * 1024,
		"planned collection exceeds 48 MiB copy/metadata budget"
	);
	let entries: usize = selected
		.iter()
		.map(|entry| entry.path.components().count() + 1)
		.sum();
	ensure!(
		(entries * 2 + 8) * cases.len() + 3 <= 10_000,
		"planned collection exceeds traversal entry budget"
	);
	ensure!(
		selected
			.iter()
			.all(|entry| entry.path.components().count() + 3 <= 64 && entry.bytes > 0),
		"batch sources must be nonempty and fit archive nesting"
	);
	let temporary = tempfile::Builder::new()
		.prefix("harvest-corpus-snapshot-")
		.tempdir()?;
	let snapshot = JailedFs::open(temporary.path())?;
	for entry in selected {
		cancellation.heartbeat()?;
		let bytes = source.read(&entry.path, MAX_FILE_BYTES)?;
		ensure!(content_hash(&bytes) == entry.sha256, "source changed during batch snapshot");
		snapshot.atomic_create(&entry.path, &bytes)?;
	}
	cancellation.heartbeat()?;
	let collection = super::prepare::TaskCollection::open(output)?;
	publish(
		&collection,
		Path::new("generation-plan.json"),
		&serde_json::json!({
			"version":BATCH_VERSION,"cases":cases,"source_policy":options.source_policy,
			"difficulty":options.difficulty,"min_score":options.min_score,
			"initial_excluded_regions":options.excluded_regions,"source_inventory":corpus,
			"planned_snapshot_bytes":bytes,
		}),
	)?;
	let mut current = options.clone();
	let mut outcomes = Vec::with_capacity(cases.len());
	let mut stopped = false;
	for (case, kind) in cases.iter().zip(kinds) {
		if stopped {
			outcomes.push(GenerationCaseOutcome {
				task_id: case.task_id.clone(),
				state:   "not_started",
				report:  None,
				error:   None,
			});
			continue;
		}
		let result = (|| -> Result<GenerationReport> {
			cancellation.heartbeat()?;
			super::generate::generate_task_owned(
				snapshot.root(),
				collection.filesystem.root(),
				&case.task_id,
				kind,
				case.seed,
				&current,
				cancellation,
				Some(&collection),
			)
		})();
		match result {
			Ok(mut report) => {
				// Collection reports use paths relative to the requested output;
				// exported journals do not depend on a machine-specific root.
				report.fixture.output = Path::new(&case.task_id).to_owned();
				current
					.excluded_regions
					.extend(report.regions.iter().cloned());
				outcomes.push(GenerationCaseOutcome {
					task_id: case.task_id.clone(),
					state:   "completed",
					report:  Some(report),
					error:   None,
				});
			},
			Err(error) => {
				stopped = true;
				outcomes.push(GenerationCaseOutcome {
					task_id: case.task_id.clone(),
					state:   if cancellation.heartbeat().is_err() {
						"cancelled"
					} else {
						"failed"
					},
					report:  None,
					error:   Some(format!("{error:#}").chars().take(1024).collect()),
				});
			},
		}
		let outcome = outcomes.last().expect("current case outcome");
		publish(
			&collection,
			Path::new(&format!("generation-case-{:02}.json", outcomes.len())),
			outcome,
		)?;
	}
	let successful = outcomes
		.iter()
		.filter(|case| case.state == "completed")
		.count();
	let report = GenerationBatch {
		version: BATCH_VERSION,
		complete: successful == cases.len(),
		successful,
		unsuccessful: cases.len() - successful,
		cases: outcomes,
	};
	publish(&collection, Path::new("generation.json"), &report)?;
	Ok(report)
}
