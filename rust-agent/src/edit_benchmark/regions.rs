//! Explicit source-hash-bound exclusions for repeated native generation. This
//! owner never writes a mutable history database or changes fixture source
//! data.
use std::{
	collections::BTreeMap,
	path::{Component, Path, PathBuf},
};

use anyhow::{Result, ensure};
use harvest_runtime::{cancel::CancelToken, content_hash};
use pi_ast::mutations::Candidate;
use serde::{Deserialize, Serialize};

pub const REGION_VERSION: &str = "rust-regions-v1";
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MutationRegion {
	pub path:            PathBuf,
	pub original_sha256: String,
	pub start_line:      usize,
	pub end_line:        usize,
}
pub(super) struct Exclusions<'a>(BTreeMap<PathBuf, Vec<&'a MutationRegion>>);
impl<'a> Exclusions<'a> {
	pub fn new(regions: &'a [MutationRegion], cancellation: &CancelToken) -> Result<Self> {
		ensure!(regions.len() <= 4096, "region history exceeds 4096 records");
		let mut paths = 0;
		let mut index: BTreeMap<PathBuf, Vec<&MutationRegion>> = BTreeMap::new();
		for region in regions {
			cancellation.heartbeat()?;
			let name = region
				.path
				.to_str()
				.ok_or_else(|| anyhow::anyhow!("region paths must be UTF-8"))?;
			ensure!(
				!name.is_empty()
					&& name.len() <= 4096
					&& region
						.path
						.components()
						.all(|part| matches!(part, Component::Normal(_))),
				"region path must be relative and normalized"
			);
			paths += name.len();
			ensure!(paths <= 1024 * 1024, "region paths exceed 1 MiB");
			ensure!(
				region.original_sha256.len() == 64
					&& region
						.original_sha256
						.bytes()
						.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
				"region SHA-256 must be 64 lowercase hex digits"
			);
			ensure!(
				region.start_line > 0
					&& region.end_line >= region.start_line
					&& region.end_line <= 256 * 1024 + 1,
				"invalid region line range"
			);
			index.entry(region.path.clone()).or_default().push(region);
		}
		Ok(Self(index))
	}

	pub fn for_source(&self, path: &Path, bytes: &[u8]) -> Vec<(usize, usize)> {
		let Some(regions) = self.0.get(path) else {
			return Vec::new();
		};
		let hash = content_hash(bytes);
		let mut ranges: Vec<_> = regions
			.iter()
			.filter(|r| r.original_sha256 == hash)
			.map(|r| (r.start_line.saturating_sub(3), r.end_line.saturating_add(3)))
			.collect();
		ranges.sort_unstable();
		let mut merged: Vec<(usize, usize)> = Vec::new();
		for (start, end) in ranges {
			if let Some(last) = merged.last_mut()
				&& start <= last.1.saturating_add(1)
			{
				last.1 = last.1.max(end);
			} else {
				merged.push((start, end));
			}
		}
		merged
	}
}
pub(super) struct LineIndex {
	newlines: Vec<usize>,
	bytes:    usize,
}
impl LineIndex {
	pub fn new(code: &str) -> Self {
		Self {
			newlines: code
				.bytes()
				.enumerate()
				.filter_map(|(index, byte)| (byte == b'\n').then_some(index))
				.collect(),
			bytes:    code.len(),
		}
	}

	fn range(&self, start: usize, end: usize) -> Result<(usize, usize)> {
		ensure!(start <= end && end <= self.bytes, "mutation region byte range is outside source");
		let last = end.saturating_sub(1).max(start);
		Ok((
			self.newlines.partition_point(|position| *position < start) + 1,
			self.newlines.partition_point(|position| *position < last) + 1,
		))
	}

	pub fn intersects(
		&self,
		candidate: &Candidate,
		excluded: &[(usize, usize)],
		cancellation: &CancelToken,
	) -> Result<bool> {
		for edit in &candidate.edits {
			cancellation.heartbeat()?;
			let (start, end) = self.range(edit.start, edit.end)?;
			let next = excluded.partition_point(|(_, high)| *high < start);
			if excluded.get(next).is_some_and(|(low, _)| *low <= end) {
				return Ok(true);
			}
		}
		Ok(false)
	}

	pub fn selected(
		&self,
		path: &Path,
		code: &str,
		candidate: &Candidate,
		cancellation: &CancelToken,
	) -> Result<Vec<MutationRegion>> {
		ensure!(candidate.edits.len() <= 4096, "mutation region output exceeds 4096 records");
		let hash = content_hash(code.as_bytes());
		let mut ranges = Vec::new();
		for edit in &candidate.edits {
			cancellation.heartbeat()?;
			ranges.push(self.range(edit.start, edit.end)?);
		}
		ranges.sort_unstable();
		ranges.dedup();
		Ok(ranges
			.into_iter()
			.map(|(start_line, end_line)| MutationRegion {
				path: path.into(),
				original_sha256: hash.clone(),
				start_line,
				end_line,
			})
			.collect())
	}
}
