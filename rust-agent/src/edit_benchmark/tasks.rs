use std::path::{Path, PathBuf};

use anyhow::{Result, ensure};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES};
use serde::Serialize;
use serde_json::value::RawValue;

const MAX_FILES: usize = 10_000;
const MAX_TREE_BYTES: u64 = 64 * 1024 * 1024;
#[derive(Debug, Serialize)]
pub struct Task {
	pub id:                 String,
	pub prompt:             String,
	pub input_directory:    PathBuf,
	pub expected_directory: PathBuf,
	pub files:              Vec<PathBuf>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub metadata:           Option<Box<RawValue>>,
}

/// Sorted, capability-relative traversal; symlinks are not benchmark inputs.
pub fn list_files(filesystem: &JailedFs, directory: &Path) -> Result<Vec<PathBuf>> {
	list_files_filtered(filesystem, directory, |_, _| true, &Default::default())
}
pub(super) fn list_files_filtered(
	filesystem: &JailedFs,
	directory: &Path,
	include: impl Fn(&Path, bool) -> bool,
	cancellation: &harvest_runtime::cancel::CancelToken,
) -> Result<Vec<PathBuf>> {
	let mut pending = vec![(directory.to_path_buf(), 0_u32)];
	let mut files = Vec::new();
	let mut total = 0_u64;
	let mut visited = 0_usize;
	while let Some((path, depth)) = pending.pop() {
		cancellation.heartbeat()?;
		ensure!(depth <= 64, "benchmark directory nesting exceeds limit");
		for entry in filesystem.entries(&path, MAX_FILES)? {
			cancellation.heartbeat()?;
			visited += 1;
			ensure!(visited <= MAX_FILES, "benchmark traversal entry count exceeds limit");
			let path = path.join(&entry.name);
			if !include(&path, entry.is_directory) {
				continue;
			}
			ensure!(!entry.is_symlink, "benchmark fixtures cannot contain symlinks");
			if entry.is_directory {
				pending.push((path, depth + 1));
			} else if entry.is_file {
				total = total
					.checked_add(entry.size)
					.ok_or_else(|| anyhow::anyhow!("benchmark input size overflow"))?;
				ensure!(
					total <= MAX_TREE_BYTES && entry.size <= MAX_FILE_BYTES as u64,
					"benchmark input exceeds size limit"
				);
				files.push(path.strip_prefix(directory)?.to_path_buf());
			}
			ensure!(files.len() + pending.len() <= MAX_FILES, "benchmark file count exceeds limit");
		}
	}
	files.sort();
	Ok(files)
}
pub fn load_tasks(fixtures: &Path) -> Result<Vec<Task>> {
	let filesystem = JailedFs::open(fixtures)?;
	let mut tasks = Vec::new();
	let mut metadata_bytes = 0_usize;
	for entry in filesystem.entries(Path::new(""), MAX_FILES)? {
		ensure!(!entry.is_symlink, "benchmark task cannot be a symlink");
		if !entry.is_directory {
			continue;
		}
		let root = PathBuf::from(&entry.name);
		let input = root.join("input");
		let expected = root.join("expected");
		let prompt = String::from_utf8(filesystem.read(&root.join("prompt.md"), 256 * 1024)?)?;
		let files = list_files(&filesystem, &input)?;
		let _expected_files = list_files(&filesystem, &expected)?;
		let metadata = filesystem
			.read_optional(&root.join("metadata.json"), 256 * 1024)?
			.map(|bytes| {
				let raw = String::from_utf8(bytes)?;
				Ok::<_, anyhow::Error>(RawValue::from_string(raw)?)
			})
			.transpose()?;
		metadata_bytes = metadata_bytes
			.checked_add(prompt.len() + metadata.as_ref().map_or(0, |raw| raw.get().len()))
			.ok_or_else(|| anyhow::anyhow!("benchmark metadata size overflow"))?;
		ensure!(metadata_bytes <= 64 * 1024 * 1024, "benchmark task metadata exceeds limit");
		tasks.push(Task {
			id: entry.name,
			prompt: prompt.trim().into(),
			input_directory: fixtures.join(input),
			expected_directory: fixtures.join(expected),
			files,
			metadata,
		});
	}
	// entries() is sorted already; explicitly preserve task ordering in the API.
	tasks.sort_by(|a, b| a.id.cmp(&b.id));
	Ok(tasks)
}

#[derive(Debug, Serialize)]
pub struct FixtureIssue {
	pub task_id: String,
	pub path:    Option<PathBuf>,
	pub code:    &'static str,
}
/// Collect actionable per-task issues instead of stopping at the first bad
/// task. JSON metadata stays lossless in the loader; validation only inspects
/// fields actually required by fixture consumers. This function never repairs
/// inputs.
pub fn validate_fixtures(
	fixtures: &Path,
	cancellation: &harvest_runtime::cancel::CancelToken,
) -> Result<Vec<FixtureIssue>> {
	let fs = JailedFs::open(fixtures)?;
	let mut issues = Vec::new();
	for entry in fs.entries(Path::new(""), MAX_FILES)? {
		cancellation.heartbeat()?;
		if !entry.is_directory && !entry.is_symlink {
			continue;
		}
		let id = entry.name;
		let root = PathBuf::from(&id);
		let mut issue = |path: Option<PathBuf>, code| {
			issues.push(FixtureIssue { task_id: id.clone(), path, code });
		};
		if entry.is_symlink {
			issue(None, "task_symlink");
			continue;
		}
		match fs.read(&root.join("prompt.md"), 256 * 1024) {
			Ok(bytes) => match std::str::from_utf8(&bytes) {
				Ok(prompt) if !prompt.trim().is_empty() => {},
				Ok(_) => issue(Some("prompt.md".into()), "empty_prompt"),
				Err(_) => issue(Some("prompt.md".into()), "invalid_prompt_utf8"),
			},
			Err(_) => issue(Some("prompt.md".into()), "missing_or_unreadable_prompt"),
		}
		let mut trees = Vec::new();
		for tree in ["input", "expected"] {
			match list_files(&fs, &root.join(tree)) {
				Ok(files) => {
					if files.is_empty() {
						issue(Some(tree.into()), "empty_tree");
					}
					for path in &files {
						cancellation.heartbeat()?;
						match fs.read(&root.join(tree).join(path), MAX_FILE_BYTES) {
							Ok(bytes) if bytes.is_empty() => {
								issue(Some(Path::new(tree).join(path)), "empty_file")
							},
							Ok(_) => {},
							Err(_) => issue(Some(Path::new(tree).join(path)), "unreadable_file"),
						}
					}
					trees.push(files);
				},
				Err(_) => {
					issue(Some(tree.into()), "missing_or_invalid_tree");
					trees.push(Vec::new());
				},
			}
		}
		match fs.read(&root.join("metadata.json"), 256 * 1024) {
			Ok(bytes) => match serde_json::from_slice::<serde_json::Value>(&bytes) {
				Ok(metadata) if metadata.is_object() => {
					if let Some(path) = metadata["file_path"]
						.as_str()
						.filter(|s| !s.trim().is_empty())
					{
						let basename = path.rsplit(['/', '\\']).next().unwrap_or("");
						for (tree, files) in ["input", "expected"].into_iter().zip(&trees) {
							if !files
								.iter()
								.any(|f| f.file_name().is_some_and(|n| n == basename))
							{
								issue(Some(tree.into()), "metadata_target_missing");
							}
						}
					} else {
						issue(Some("metadata.json".into()), "metadata_file_path_missing");
					}
				},
				_ => issue(Some("metadata.json".into()), "invalid_metadata_object"),
			},
			Err(_) => issue(Some("metadata.json".into()), "missing_or_unreadable_metadata"),
		}
		ensure!(issues.len() <= 100_000, "fixture validation issue limit exceeded");
	}
	Ok(issues)
}
