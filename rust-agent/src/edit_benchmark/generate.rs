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
#[derive(Debug, Default, Clone)]
pub struct GenerationOptions {
	pub source_policy:    super::SourcePolicy,
	pub difficulty:       Option<super::Difficulty>,
	/// Override the requested difficulty's default minimum; requires difficulty.
	pub min_score:        Option<u32>,
	pub excluded_regions: Vec<super::MutationRegion>,
}
#[derive(Debug, Serialize)]
pub struct DifficultySelection {
	pub version:              &'static str,
	pub requested:            super::Difficulty,
	pub minimum:              u32,
	pub score:                u32,
	pub preferred_candidates: usize,
	pub fallback_candidates:  usize,
	pub used_fallback:        bool,
}

#[derive(Debug, Serialize)]
pub struct RejectedSource {
	pub target: PathBuf,
	pub reason: String,
}
#[derive(Debug, Serialize)]
pub struct GenerationReport {
	pub region_policy_version: &'static str,
	pub regions:               Vec<super::MutationRegion>,
	pub excluded_candidates:   usize,
	pub difficulty:            Option<DifficultySelection>,
	pub source_policy:         super::SourcePolicy,
	pub corpus_policy_version: &'static str,
	pub corpus_files:          usize,
	pub corpus_scanned:        usize,
	pub producer:              &'static str,
	pub seed:                  u64,
	pub mutation:              MutationKind,
	pub target:                PathBuf,
	pub candidates:            usize,
	pub rejected_sources:      usize,
	pub rejected_examples:     Vec<RejectedSource>,
	pub fixture:               super::PreparedTask,
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
	generate_task_with_policy(
		source,
		output,
		task_id,
		kind,
		seed,
		super::SourcePolicy::ReviewedTree,
		cancellation,
	)
}
/// Explicit source-data selection shares the ordinary generator and scorer.
pub fn generate_task_with_policy(
	source: &Path,
	output: &Path,
	task_id: &str,
	kind: MutationKind,
	seed: u64,
	policy: super::SourcePolicy,
	cancellation: &CancelToken,
) -> Result<GenerationReport> {
	generate_task_with_options(
		source,
		output,
		task_id,
		kind,
		seed,
		&GenerationOptions { source_policy: policy, ..Default::default() },
		cancellation,
	)
}
/// Native preference/fallback sampling remains independently versioned; no
/// historical random-attempt, formatted-score or case-distribution parity.
pub fn generate_task_with_options(
	source: &Path,
	output: &Path,
	task_id: &str,
	kind: MutationKind,
	seed: u64,
	options: &GenerationOptions,
	cancellation: &CancelToken,
) -> Result<GenerationReport> {
	ensure!(
		options.difficulty.is_some() || options.min_score.is_none(),
		"minimum score requires explicit difficulty"
	);
	let exclusions = super::regions::Exclusions::new(&options.excluded_regions, cancellation)?;
	let mut excluded_candidates = 0;
	let policy = options.source_policy;
	let minimum = options
		.difficulty
		.map(|level| options.min_score.unwrap_or_else(|| level.minimum()));
	cancellation.heartbeat()?;
	let source = JailedFs::open(source)?;
	let proposed = super::prepare::prospective_output(output)?;
	ensure!(
		!proposed.starts_with(source.root()) && !source.root().starts_with(&proposed),
		"generation output and original source trees must be disjoint"
	);
	let paths = policy.paths(&source, cancellation)?;
	ensure!(!paths.is_empty(), "mutation source selection is empty");
	let corpus_scanned = paths.len();
	let mut corpus_files = 0;
	let temporary = tempfile::Builder::new()
		.prefix("harvest-mutation-")
		.tempdir()?;
	let input = JailedFs::open_or_create(&temporary.path().join("input"))?;
	let expected = JailedFs::open_or_create(&temporary.path().join("expected"))?;
	let mut random = Random(seed);
	let mut selected: Option<(PathBuf, SupportLang, Candidate, Option<u32>)> = None;
	let mut fallback = None;
	let mut preferred_count = 0_usize;
	let mut fallback_count = 0_usize;
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
		if matches!(policy, super::SourcePolicy::LegacyPackages)
			&& policy.rejection(&path, &bytes).is_some()
		{
			continue;
		}
		corpus_files += 1;
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
		let excluded = exclusions.for_source(&path, &bytes);
		let line_index = (!excluded.is_empty()).then(|| super::regions::LineIndex::new(code));
		let mut analysis = options
			.difficulty
			.map(|_| super::difficulty::FileAnalysis::new(code, cancellation))
			.transpose()?;
		let preferred = options.difficulty.is_none_or(|level| {
			level.prefers(&analysis.as_ref().expect("requested analysis").metrics)
		});
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
			if let Some(index) = &line_index
				&& index.intersects(&candidate, &excluded, cancellation)?
			{
				excluded_candidates += 1;
				continue;
			}
			let score = analysis
				.as_mut()
				.map(|analysis| analysis.score(candidate.line_number, cancellation))
				.transpose()?;
			if minimum.is_some_and(|minimum| score.is_some_and(|score| score < minimum)) {
				continue;
			}
			if preferred {
				preferred_count += 1;
				if random.below(preferred_count as u64) == 0 {
					selected = Some((path.clone(), lang, candidate, score));
				}
			} else {
				fallback_count += 1;
				if random.below(fallback_count as u64) == 0 {
					fallback = Some((path.clone(), lang, candidate, score));
				}
			}
		}
	}
	let used_fallback = selected.is_none() && fallback.is_some();
	let (path, lang, candidate, score) = selected.or(fallback).ok_or_else(|| {
		let detail = rejected_examples
			.first()
			.map(|r| r.reason.as_str())
			.unwrap_or("no syntax candidate passes the requested selection policy");
		anyhow::anyhow!(
			"no eligible {} candidate: {detail} ({rejected} rejected sources)",
			kind.name()
		)
	})?;
	let difficulty = options.difficulty.map(|requested| DifficultySelection {
		version: super::DIFFICULTY_VERSION,
		requested,
		minimum: minimum.expect("difficulty minimum"),
		score: score.expect("selected score"),
		preferred_candidates: preferred_count,
		fallback_candidates: fallback_count,
		used_fallback,
	});
	let original = expected.read(&path, 256 * 1024)?;
	let code = std::str::from_utf8(&original)?;
	let regions =
		super::regions::LineIndex::new(code).selected(&path, code, &candidate, cancellation)?;
	let mutated = mutations::apply(code, lang, &candidate, &mut || cancellation.heartbeat())
		.with_context(|| {
			format!("selected {} mutation rejected in {}", kind.name(), path.display())
		})?;
	input.atomic_write(&path, mutated.as_bytes())?;
	let provenance = serde_json::json!({"producer":PRODUCER_VERSION,"seed":seed,"mutation":kind,
        "target":path,"selected":candidate,"original_sha256":content_hash(&original),
        "mutated_sha256":content_hash(mutated.as_bytes()),"candidates":seen,"rejected_sources":rejected,
        "difficulty":difficulty,"region_policy_version":super::REGION_VERSION,"regions":regions,"excluded_candidates":excluded_candidates,"source_policy":policy,"corpus_policy_version":super::corpus::POLICY_VERSION,"corpus_files":corpus_files,"corpus_scanned":corpus_scanned});
	let fixture = super::prepare::prepare_task_with_provenance(
		input.root(),
		expected.root(),
		output,
		task_id,
		Some(&provenance),
		cancellation,
	)?;
	Ok(GenerationReport {
		region_policy_version: super::REGION_VERSION,
		regions,
		excluded_candidates,
		difficulty,
		source_policy: policy,
		corpus_policy_version: super::corpus::POLICY_VERSION,
		corpus_files,
		corpus_scanned,
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
