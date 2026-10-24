//! Versioned, read-only source-data selection. Shared bounded traversal and the
//! mutation engine still own filesystem and syntax semantics respectively.
use std::path::{Path, PathBuf};

use anyhow::{Result, ensure};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken, content_hash};
use serde::Serialize;

pub(super) const POLICY_VERSION: &str = "rust-corpus-v1";

#[derive(Debug, Default, Clone, Copy, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum SourcePolicy {
	/// Existing native curated-tree behavior: retain all files as context.
	#[default]
	ReviewedTree,
	/// Legacy package/path/line/structure filters; native syntax eligibility is
	/// separate.
	LegacyPackages,
}
#[derive(Debug, Serialize)]
pub struct CorpusEntry {
	pub path:     PathBuf,
	pub sha256:   String,
	pub bytes:    usize,
	pub lines:    Option<usize>,
	pub eligible: bool,
	pub reason:   Option<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct CorpusReport {
	pub policy:         SourcePolicy,
	pub policy_version: &'static str,
	pub files:          Vec<CorpusEntry>,
	pub eligible:       usize,
	pub total_bytes:    usize,
}
fn included(path: &Path, directory: bool) -> bool {
	if path.components().any(|part| {
		part.as_os_str().to_str().is_some_and(|part| {
			matches!(
				part,
				"__tests__"
					| "__mocks__"
					| "fixtures"
					| "fixture"
					| "dist" | "build"
					| "node_modules"
					| "scripts"
			)
		})
	}) {
		return false;
	}
	if directory {
		return true;
	}
	let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
		return false;
	};
	matches!(path.extension().and_then(|s| s.to_str()), Some("js" | "jsx" | "ts" | "tsx"))
		&& ![".test.", ".spec.", ".fixture."]
			.iter()
			.any(|part| name.contains(part))
}
impl SourcePolicy {
	pub(super) fn paths(
		self,
		filesystem: &JailedFs,
		cancellation: &CancelToken,
	) -> Result<Vec<PathBuf>> {
		cancellation.heartbeat()?;
		let paths = match self {
			Self::ReviewedTree => {
				super::tasks::list_files_filtered(filesystem, Path::new(""), |_, _| true, cancellation)?
			},
			Self::LegacyPackages => super::tasks::list_files_filtered(
				filesystem,
				Path::new("packages"),
				included,
				cancellation,
			)?
			.into_iter()
			.map(|path| Path::new("packages").join(path))
			.collect(),
		};
		let mut path_bytes = 0_usize;
		for path in &paths {
			let name = path
				.to_str()
				.ok_or_else(|| anyhow::anyhow!("corpus paths must be UTF-8"))?;
			ensure!(name.len() <= 4096, "corpus path exceeds 4096 bytes");
			path_bytes += name.len();
			ensure!(path_bytes <= 1024 * 1024, "corpus paths exceed 1 MiB");
		}
		Ok(paths)
	}

	pub(super) fn rejection(self, path: &Path, bytes: &[u8]) -> Option<&'static str> {
		let text = std::str::from_utf8(bytes).ok();
		let lines = text.map(|code| code.split('\n').count());
		let supported = pi_ast::SupportLang::from_path(&path).is_some_and(|language| {
			matches!(
				language,
				pi_ast::SupportLang::JavaScript
					| pi_ast::SupportLang::TypeScript
					| pi_ast::SupportLang::Tsx
			)
		});
		if !supported {
			Some("unsupported_language")
		} else if bytes.is_empty() {
			Some("empty_source")
		} else if bytes.len() > 256 * 1024 {
			Some("source_exceeds_256_kib")
		} else if text.is_none() {
			Some("source_not_utf8")
		} else if matches!(self, Self::LegacyPackages)
			&& !lines.is_some_and(|n| (30..=800).contains(&n))
		{
			Some("line_count_outside_30_800")
		} else if matches!(self, Self::LegacyPackages)
			&& !text.is_some_and(|code| {
				["function ", "class ", "export ", "=>"]
					.iter()
					.any(|token| code.contains(token))
			}) {
			Some("no_structure_token")
		} else {
			None
		}
	}

	pub(super) fn inspect(
		self,
		filesystem: &JailedFs,
		cancellation: &CancelToken,
	) -> Result<CorpusReport> {
		let paths = self.paths(filesystem, cancellation)?;
		let mut files = Vec::with_capacity(paths.len());
		let mut total_bytes = 0_usize;
		for path in paths {
			cancellation.heartbeat()?;
			let bytes = filesystem.read(&path, MAX_FILE_BYTES)?;
			total_bytes = total_bytes
				.checked_add(bytes.len())
				.ok_or_else(|| anyhow::anyhow!("corpus size overflow"))?;
			ensure!(total_bytes <= 64 * 1024 * 1024, "corpus exceeds 64 MiB");
			let lines = std::str::from_utf8(&bytes)
				.ok()
				.map(|code| code.split('\n').count());
			let reason = self.rejection(&path, &bytes);
			files.push(CorpusEntry {
				sha256: content_hash(&bytes),
				bytes: bytes.len(),
				path,
				lines,
				eligible: reason.is_none(),
				reason,
			});
		}
		let eligible = files.iter().filter(|file| file.eligible).count();
		Ok(CorpusReport {
			policy: self,
			policy_version: POLICY_VERSION,
			files,
			eligible,
			total_bytes,
		})
	}
}
/// Inspect local inert source data; never clone repositories or execute
/// sources.
pub fn inspect_corpus(
	source: &Path,
	policy: SourcePolicy,
	cancellation: &CancelToken,
) -> Result<CorpusReport> {
	policy.inspect(&JailedFs::open(source)?, cancellation)
}
