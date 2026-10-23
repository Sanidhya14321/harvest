//! Native deterministic mutation selection and fixture production. Original
//! source bytes are snapshotted as expected data and never modified in place.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, cancel::CancelToken, content_hash};
use pi_ast::{
	SupportLang,
	mutations::{self, Candidate, MutationKind},
};
use serde::Serialize;

pub const PRODUCER_VERSION: &str = "rust-ast-v1";
struct Random(u64);
impl Random {
	fn next(&mut self) -> u64 {
		self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
		let mut x = self.0;
		x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
		x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
		x ^ (x >> 31)
	}

	fn below(&mut self, upper: u64) -> u64 {
		let cutoff = u64::MAX - u64::MAX % upper;
		loop {
			let value = self.next();
			if value < cutoff {
				return value % upper;
			}
		}
	}
}
#[derive(Debug, Serialize)]
pub struct RejectedSource {
	pub target: PathBuf,
	pub reason: String,
}
#[derive(Debug, Serialize)]
pub struct GenerationReport {
	pub producer:          &'static str,
	pub seed:              u64,
	pub mutation:          MutationKind,
	pub target:            PathBuf,
	pub candidates:        usize,
	pub rejected_sources:  usize,
	pub rejected_examples: Vec<RejectedSource>,
	pub fixture:           super::PreparedTask,
}
/// `source` is a reviewed source-data tree, not a repository discovery rule.
/// Unsupported/large/non-UTF8 files are retained as inert copies. Only the
/// supported JS/TS target is mutated. Seed semantics are versioned separately
/// from the historical Babel generator; no equivalence is claimed.
pub fn generate_task(
	source: &Path,
	output: &Path,
	task_id: &str,
	kind: MutationKind,
	seed: u64,
	cancellation: &CancelToken,
) -> Result<GenerationReport> {
	cancellation.heartbeat()?;
	let source = JailedFs::open(source)?;
	let paths = super::list_files(&source, Path::new(""))?;
	ensure!(!paths.is_empty(), "mutation source tree is empty");
	let temporary = tempfile::Builder::new()
		.prefix("harvest-mutation-")
		.tempdir()?;
	let input = JailedFs::open_or_create(&temporary.path().join("input"))?;
	let expected = JailedFs::open_or_create(&temporary.path().join("expected"))?;
	let mut random = Random(seed);
	let mut selected: Option<(PathBuf, SupportLang, Candidate)> = None;
	let mut seen = 0_usize;
	let mut rejected = 0;
	let mut rejected_examples = Vec::new();
	let mut reject = |path: &Path, reason: &str| {
		rejected += 1;
		if rejected_examples.len() < 32 {
			rejected_examples.push(RejectedSource {
				target: path.to_owned(),
				reason: reason.chars().take(256).collect(),
			});
		}
	};
	let mut total = 0_usize;
	for path in paths {
		cancellation.heartbeat()?;
		let bytes = source.read(&path, MAX_FILE_BYTES)?;
		total = total
			.checked_add(bytes.len())
			.ok_or_else(|| anyhow::anyhow!("source size overflow"))?;
		ensure!(total <= 64 * 1024 * 1024, "source snapshot exceeds 64 MiB");
		ensure!(!bytes.is_empty(), "mutation source tree contains an empty file");
		input.atomic_create(&path, &bytes)?;
		expected.atomic_create(&path, &bytes)?;
		let Some(lang) = SupportLang::from_path(&path).filter(|l| {
			matches!(l, SupportLang::JavaScript | SupportLang::TypeScript | SupportLang::Tsx)
		}) else {
			continue;
		};
		let Ok(code) = std::str::from_utf8(&bytes) else {
			reject(&path, "source is not UTF-8");
			continue;
		};
		if code.len() > 256 * 1024 {
			reject(&path, "source exceeds 256 KiB");
			continue;
		}
		let candidates =
			match mutations::candidates(code, lang, kind, &mut || cancellation.heartbeat()) {
				Ok(candidates) => candidates,
				Err(error) => {
					cancellation.heartbeat()?;
					reject(&path, &error.to_string());
					continue;
				},
			};
		for candidate in candidates {
			seen += 1;
			ensure!(seen <= 100_000, "mutation selection exceeds candidate budget");
			if random.below(seen as u64) == 0 {
				selected = Some((path.clone(), lang, candidate));
			}
		}
	}
	let (path, lang, candidate) = selected.ok_or_else(|| {
		let detail = rejected_examples
			.first()
			.map(|r| r.reason.as_str())
			.unwrap_or("no matching syntax");
		anyhow::anyhow!(
			"no eligible {} candidate: {detail} ({rejected} rejected sources)",
			kind.name()
		)
	})?;
	let original = expected.read(&path, 256 * 1024)?;
	let code = std::str::from_utf8(&original)?;
	let mutated = mutations::apply(code, lang, &candidate, &mut || cancellation.heartbeat())
		.with_context(|| {
			format!("selected {} mutation rejected in {}", kind.name(), path.display())
		})?;
	input.atomic_write(&path, mutated.as_bytes())?;
	let provenance = serde_json::json!({"producer":PRODUCER_VERSION,"seed":seed,"mutation":kind,
        "target":path,"selected":candidate,"original_sha256":content_hash(&original),
        "mutated_sha256":content_hash(mutated.as_bytes()),"candidates":seen,"rejected_sources":rejected});
	let fixture = super::prepare::prepare_task_with_provenance(
		input.root(),
		expected.root(),
		output,
		task_id,
		Some(&provenance),
		cancellation,
	)?;
	Ok(GenerationReport {
		producer: PRODUCER_VERSION,
		seed,
		mutation: kind,
		target: path,
		candidates: seen,
		rejected_sources: rejected,
		rejected_examples,
		fixture,
	})
}
