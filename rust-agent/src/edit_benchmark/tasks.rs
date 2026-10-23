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
	let mut pending = vec![(directory.to_path_buf(), 0_u32)];
	let mut files = Vec::new();
	let mut total = 0_u64;
	while let Some((path, depth)) = pending.pop() {
		ensure!(depth <= 64, "benchmark directory nesting exceeds limit");
		for entry in filesystem.entries(&path, MAX_FILES)? {
			ensure!(!entry.is_symlink, "benchmark fixtures cannot contain symlinks");
			let path = path.join(&entry.name);
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
