use std::{
	collections::BTreeSet,
	path::{Path, PathBuf},
	time::Instant,
};

use anyhow::{Result, ensure};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::{SCORING_VERSION, list_files};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
	pub path: String,
	pub old:  String,
	pub new:  String,
}
#[derive(Debug, Serialize)]
pub struct EditOutcome {
	pub index:     usize,
	pub path:      String,
	pub attempted: bool,
	pub applied:   bool,
	pub error:     Option<String>,
}
#[derive(Debug, Serialize)]
pub struct FileOutcome {
	pub path:    PathBuf,
	pub matches: bool,
	pub reason:  Option<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct Report {
	pub scoring_version:   &'static str,
	pub success:           bool,
	pub patch_applied:     Option<bool>,
	pub output_matches:    bool,
	pub elapsed_micros:    u128,
	pub edits:             Vec<EditOutcome>,
	pub files:             Vec<FileOutcome>,
	pub usage_unavailable: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub usage:             Option<Box<RawValue>>,
}
fn normalize(bytes: &[u8]) -> Vec<u8> {
	let mut normalized = Vec::with_capacity(bytes.len());
	let mut index = 0;
	while index < bytes.len() {
		if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
			index += 1;
		}
		normalized.push(bytes[index]);
		index += 1;
	}
	normalized
}
fn compare_files(
	expected: &JailedFs,
	actual: &JailedFs,
	cancellation: &CancelToken,
) -> Result<Vec<FileOutcome>> {
	let expected_files: BTreeSet<_> = list_files(expected, Path::new(""))?.into_iter().collect();
	let actual_files: BTreeSet<_> = list_files(actual, Path::new(""))?.into_iter().collect();
	let mut outcomes = Vec::new();
	for path in expected_files.union(&actual_files) {
		cancellation.heartbeat()?;
		let reason = if !expected_files.contains(path) {
			Some("unexpected_file")
		} else if !actual_files.contains(path) {
			Some("missing_file")
		} else {
			let expected = normalize(&expected.read(path, MAX_FILE_BYTES)?);
			let actual = normalize(&actual.read(path, MAX_FILE_BYTES)?);
			(expected != actual).then_some("content_mismatch")
		};
		outcomes.push(FileOutcome { path: path.clone(), matches: reason.is_none(), reason });
	}
	Ok(outcomes)
}

/// Compare exact bytes after CRLF-to-LF normalization. Preserve every other
/// whitespace, blank line, syntax and final-newline distinction.
pub fn compare(expected: &Path, actual: &Path, cancellation: &CancelToken) -> Result<Report> {
	let started = Instant::now();
	cancellation.heartbeat()?;
	let files = compare_files(&JailedFs::open(expected)?, &JailedFs::open(actual)?, cancellation)?;
	let output_matches = files.iter().all(|file| file.matches);
	Ok(Report {
		scoring_version: SCORING_VERSION,
		success: output_matches,
		patch_applied: None,
		output_matches,
		elapsed_micros: started.elapsed().as_micros(),
		edits: Vec::new(),
		files,
		usage_unavailable: true,
		usage: None,
	})
}

/// Apply ordered edit operations using the existing jailed, atomic edit engine.
/// Stop on the first failed operation and identify all skipped operations.
pub fn apply_and_score(
	expected: &Path,
	actual: &Path,
	edits: &[Edit],
	usage: Option<Box<RawValue>>,
	cancellation: &CancelToken,
) -> Result<Report> {
	ensure!(edits.len() <= 10_000, "benchmark edit count exceeds limit");
	let started = Instant::now();
	let filesystem = JailedFs::open(actual)?;
	let mut applied = true;
	let mut outcomes = Vec::new();
	for (index, edit) in edits.iter().enumerate() {
		cancellation.heartbeat()?;
		let attempted = applied;
		let error = if attempted {
			let result = (|| {
				filesystem.relative(Path::new(&edit.path))?;
				ensure!(
					!edit.old.is_empty()
						&& edit.old.len() <= MAX_FILE_BYTES
						&& edit.new.len() <= MAX_FILE_BYTES,
					"benchmark edit text is empty or oversized"
				);
				crate::edit::replace(
					actual,
					&crate::tools::EditArgs {
						path: edit.path.clone(),
						old:  edit.old.clone(),
						new:  edit.new.clone(),
					},
					cancellation,
				)?;
				Ok::<_, anyhow::Error>(())
			})();
			cancellation.heartbeat()?;
			result.err().map(|_| "patch_application_failed".to_owned())
		} else {
			Some("skipped_after_failed_edit".into())
		};
		let success = attempted && error.is_none();
		if !success {
			applied = false;
		}
		outcomes.push(EditOutcome {
			index,
			path: edit.path.clone(),
			attempted,
			applied: success,
			error,
		});
	}
	let mut report = compare(expected, actual, cancellation)?;
	report.success &= applied;
	report.patch_applied = Some(applied);
	report.edits = outcomes;
	report.elapsed_micros = started.elapsed().as_micros();
	#[derive(Deserialize)]
	struct Measurement {
		input_tokens:  u64,
		output_tokens: u64,
	}
	report.usage_unavailable = usage.as_ref().is_none_or(|raw| {
		serde_json::from_str::<Measurement>(raw.get())
			.ok()
			.and_then(|usage| usage.input_tokens.checked_add(usage.output_tokens))
			.is_none()
	});
	report.usage = usage;
	Ok(report)
}
