//! Native deterministic task preparation from reviewed input/expected trees.
//! The native AST mutation producer reuses the same exact hunk/copy contract.
use std::{
	collections::BTreeSet,
	path::{Path, PathBuf},
};

use anyhow::{Result, ensure};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken, content_hash};
use serde::Serialize;

use super::{SCORING_VERSION, hunks, list_files};

#[derive(Debug, Serialize)]
pub struct PreparedTask {
	pub task_id:       String,
	pub files:         usize,
	pub changed_files: usize,
	pub output:        PathBuf,
}
#[derive(Serialize)]
struct PromptHunk {
	old:        String,
	new:        String,
	start_line: usize,
	unique:     bool,
}
#[derive(Serialize)]
struct PromptFile {
	path:  String,
	fence: String,
	hunks: Vec<PromptHunk>,
}
#[derive(Serialize)]
struct FileIdentity {
	path:            PathBuf,
	input_sha256:    String,
	expected_sha256: String,
}

/// Resolve a proposed destination without creating it. Pin existing ancestry
/// through the shared capability resolver before testing source separation.
pub(crate) fn prospective_output(output: &Path) -> Result<PathBuf> {
	let absolute = if output.is_absolute() {
		output.to_owned()
	} else {
		std::env::current_dir()?.join(output)
	};
	let mut ancestor = absolute.as_path();
	while !ancestor.exists() {
		ancestor = ancestor
			.parent()
			.ok_or_else(|| anyhow::anyhow!("output has no existing ancestor"))?;
	}
	let fs = JailedFs::open(ancestor)?;
	let suffix = absolute.strip_prefix(ancestor)?;
	if suffix.as_os_str().is_empty() {
		Ok(fs.root().to_owned())
	} else {
		fs.resource_path(suffix)
	}
}

/// Writes one new unpacked task, never overwriting an existing task. No input
/// or expected source is executed or mutated. Copies use the shared filesystem
/// jail.
pub fn prepare_task(
	input: &Path,
	expected: &Path,
	output: &Path,
	task_id: &str,
	cancellation: &CancelToken,
) -> Result<PreparedTask> {
	prepare_task_with_provenance(input, expected, output, task_id, None, cancellation)
}

pub(crate) fn prepare_task_with_provenance(
	input: &Path,
	expected: &Path,
	output: &Path,
	task_id: &str,
	provenance: Option<&serde_json::Value>,
	cancellation: &CancelToken,
) -> Result<PreparedTask> {
	ensure!(
		!task_id.is_empty()
			&& task_id.len() <= 128
			&& task_id
				.bytes()
				.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
		"task id must contain 1..128 ASCII letters, digits, hyphens or underscores"
	);
	let input = JailedFs::open(input)?;
	let expected = JailedFs::open(expected)?;
	let proposed = prospective_output(output)?;
	for source in [&input, &expected] {
		ensure!(
			!proposed.starts_with(source.root()) && !source.root().starts_with(&proposed),
			"prepared output must be disjoint from source trees"
		);
	}
	let input_files: BTreeSet<_> = list_files(&input, Path::new(""))?.into_iter().collect();
	let expected_files: BTreeSet<_> = list_files(&expected, Path::new(""))?.into_iter().collect();
	ensure!(
		!input_files.is_empty() && input_files == expected_files,
		"prepared task requires identical nonempty file sets"
	);
	let mut prompt_files = Vec::new();
	let mut copies = Vec::new();
	let mut identities = Vec::new();
	let mut copy_bytes = 0_usize;
	for path in input_files {
		cancellation.heartbeat()?;
		let before = input.read(&path, MAX_FILE_BYTES)?;
		let after = expected.read(&path, MAX_FILE_BYTES)?;
		ensure!(!before.is_empty() && !after.is_empty(), "fixture files must be nonempty");
		copy_bytes = copy_bytes
			.checked_add(before.len() + after.len())
			.ok_or_else(|| anyhow::anyhow!("fixture byte count overflow"))?;
		ensure!(copy_bytes <= 128 * 1024 * 1024, "fixture copy exceeds 128 MiB budget");
		identities.push(FileIdentity {
			path:            path.clone(),
			input_sha256:    content_hash(&before),
			expected_sha256: content_hash(&after),
		});
		if before != after {
			let hunks = hunks::prepare_hunks(
				std::str::from_utf8(&before)?,
				std::str::from_utf8(&after)?,
				cancellation,
			)?;
			let fence = hunks::fence(&hunks);
			prompt_files.push(PromptFile {
				path: path
					.to_str()
					.ok_or_else(|| anyhow::anyhow!("fixture path must be UTF-8"))?
					.into(),
				fence,
				hunks: hunks
					.into_iter()
					.map(|h| PromptHunk {
						old:        h.old_block.join("\n"),
						new:        h.new_block.join("\n"),
						start_line: h.start_line,
						unique:     h.unique,
					})
					.collect(),
			});
		}
		copies.push((path, before, after));
	}
	ensure!(!prompt_files.is_empty(), "task has no changed files");
	let mut templates = handlebars::Handlebars::new();
	templates.set_strict_mode(true);
	let prompt = templates.render_template(
		include_str!("../prompts/benchmark-task.md"),
		&serde_json::json!({"files":prompt_files}),
	)?;
	ensure!(prompt.len() <= 256 * 1024, "generated task prompt exceeds 256 KiB");
	let metadata = serde_json::to_vec(&serde_json::json!({"scoring_version":SCORING_VERSION,
        "producer":if provenance.is_some(){super::generate::PRODUCER_VERSION}else{"rust-paired-tree-v1"},"mutation":provenance,"file_path":prompt_files[0].path,"files":identities}))?;
	ensure!(metadata.len() <= 256 * 1024, "fixture metadata exceeds 256 KiB");
	let output = JailedFs::open_or_create(output)?;
	for source in [&input, &expected] {
		ensure!(
			!output.root().starts_with(source.root()) && !source.root().starts_with(output.root()),
			"prepared output must be disjoint from source trees"
		);
	}
	let _owner = output.resource_lock(Path::new(".fixture-owner"))?;
	ensure!(
		output
			.entries(Path::new(""), 4)?
			.iter()
			.all(|e| e.name == ".fixture-owner.lock"),
		"prepared output must be empty"
	);
	for (path, before, after) in &copies {
		cancellation.heartbeat()?;
		output.atomic_create(&Path::new(task_id).join("input").join(path), before)?;
		output.atomic_create(&Path::new(task_id).join("expected").join(path), after)?;
	}
	output.atomic_create(&Path::new(task_id).join("metadata.json"), &metadata)?;
	cancellation.heartbeat()?;
	// Prompt publication is last: interrupted preparations cannot load as tasks.
	output.atomic_create(&Path::new(task_id).join("prompt.md"), prompt.as_bytes())?;
	Ok(PreparedTask {
		task_id:       task_id.into(),
		files:         copies.len(),
		changed_files: prompt_files.len(),
		output:        output.root().join(task_id),
	})
}
