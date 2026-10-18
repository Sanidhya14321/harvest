//! Grep candidate discovery uses the same native walker as the existing host.
use std::path::Path;

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken, truncate_bytes};
use pi_walker::{
	FileType, FollowLinks, WalkDecision, WalkError, WalkOrder, WalkRequest, WalkStatus,
};
use regex::{Regex, RegexBuilder};
use serde::Deserialize;

const MATCH_LIMIT: usize = 50;
const ENTRY_LIMIT: usize = 10_000;
const MAX_DEPTH: usize = 32;
const LINE_BYTES: usize = 2000;

#[derive(Debug, Deserialize)]
pub(crate) struct Args {
	pattern:        String,
	path:           Option<String>,
	#[serde(rename = "case")]
	case_sensitive: Option<bool>,
	gitignore:      Option<bool>,
}

pub(crate) fn grep(root: &Path, args: &Args, cancellation: &CancelToken) -> Result<String> {
	search(root, args, &|| cancellation.heartbeat())
}

fn search(root: &Path, args: &Args, heartbeat: &dyn Fn() -> Result<()>) -> Result<String> {
	heartbeat()?;
	let pattern = RegexBuilder::new(&args.pattern)
		.case_insensitive(!args.case_sensitive.unwrap_or(true))
		.build()
		.context("invalid regular expression")?;
	let fs = JailedFs::open(root)?;
	let start = fs.relative(Path::new(args.path.as_deref().unwrap_or(".")))?;
	let kind = fs.file_type(&start)?;
	let mut hits = Vec::new();
	let limited = if kind.is_file() {
		scan_file(&fs, &start, &pattern, &mut hits, heartbeat)?
	} else if kind.is_dir() {
		let mut visited = 0;
		let mut depth_limited = false;
		let status = WalkRequest::new(fs.root().join(&start))
			.hidden(true)
			.gitignore(args.gitignore.unwrap_or(true))
			.skip_git(true)
			.skip_node_modules(true)
			.follow_links(FollowLinks::Never)
			.order(WalkOrder::Path)
			.cache(false)
			.depth(1, MAX_DEPTH)
			.for_each_entry_with_heartbeat(
				heartbeat,
				|entry| {
					heartbeat()?;
					visited += 1;
					if visited > ENTRY_LIMIT {
						return Ok(WalkDecision::Stop);
					}
					if entry.file_type == FileType::Dir {
						if entry
							.absolute_path
							.file_name()
							.and_then(|name| name.to_str())
							.is_some_and(|name| matches!(name, ".omp" | ".harvest" | "target"))
						{
							return Ok(WalkDecision::SkipDescend);
						}
						if entry.depth >= MAX_DEPTH {
							depth_limited = true;
						}
					} else if entry.file_type == FileType::File {
						let path = fs.relative(&entry.absolute_path)?;
						if scan_file(&fs, &path, &pattern, &mut hits, heartbeat)? {
							return Ok(WalkDecision::Stop);
						}
					}
					Ok(WalkDecision::Include)
				},
				|_| Ok::<WalkDecision, anyhow::Error>(WalkDecision::Skip),
			)
			.map_err(|error| match error {
				WalkError::Interrupted(error) => error,
				error => anyhow::anyhow!("{error}"),
			})?;
		status != WalkStatus::Complete || depth_limited
	} else {
		bail!("search target must be a regular file or directory");
	};
	heartbeat()?;
	if limited {
		hits.push("[search limit reached]".into());
	}
	Ok(hits.join("\n"))
}

fn scan_file(
	fs: &JailedFs,
	path: &Path,
	pattern: &Regex,
	hits: &mut Vec<String>,
	heartbeat: &dyn Fn() -> Result<()>,
) -> Result<bool> {
	heartbeat()?;
	let Ok(bytes) = fs.read(path, MAX_FILE_BYTES) else {
		return Ok(false);
	};
	if bytes.contains(&0) {
		return Ok(false);
	}
	let Ok(text) = std::str::from_utf8(&bytes) else {
		return Ok(false);
	};
	for (index, line) in text.lines().enumerate() {
		heartbeat()?;
		if pattern.is_match(line) {
			hits.push(format!(
				"{}:{}:{}",
				path.display(),
				index + 1,
				truncate_bytes(line, LINE_BYTES)
			));
			if hits.len() >= MATCH_LIMIT {
				return Ok(true);
			}
		}
	}
	Ok(false)
}

#[cfg(test)]
mod tests {
	use std::cell::Cell;

	use super::*;

	#[test]
	fn cancellation_during_a_valid_scan_discards_partial_results() -> Result<()> {
		let root = tempfile::tempdir()?;
		std::fs::write(root.path().join("note"), "needle\nneedle\nneedle\n")?;
		let args = Args {
			pattern:        "needle".into(),
			path:           Some("note".into()),
			case_sensitive: None,
			gitignore:      None,
		};
		assert_eq!(search(root.path(), &args, &|| Ok(()))?.lines().count(), 3);
		let mut token = CancelToken::default();
		let abort = token.emplace_abort_token();
		let progress = Cell::new(0);
		let error = search(root.path(), &args, &|| {
			progress.set(progress.get() + 1);
			if progress.get() >= 4 {
				abort.abort(harvest_runtime::cancel::AbortReason::User);
			}
			token.heartbeat()
		})
		.unwrap_err();
		assert!(error.to_string().contains("Aborted: User"));
		Ok(())
	}
}
