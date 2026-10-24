//! Explicit native fixture shape selection and quota expansion. Measurements
//! reuse the exact shared hunk engine, never Babel/Prettier historical scores.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use harvest_runtime::{cancel::CancelToken, content_hash};
use serde::{Deserialize, Serialize};

use super::{GenerationCase, MutationKind};

pub const CALIBRATION_VERSION: &str = "rust-calibration-v1";
pub const CONTENT_VERSION: &str = "rust-content-v1";
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShapeConstraint {
	pub min_changed_lines: usize,
	pub max_changed_lines: usize,
	pub min_hunks:         usize,
	pub max_hunks:         usize,
}
impl ShapeConstraint {
	pub fn validate(&self) -> Result<()> {
		ensure!(
			self.min_changed_lines > 0
				&& self.min_changed_lines <= self.max_changed_lines
				&& self.max_changed_lines <= 4000,
			"invalid changed-line constraint"
		);
		ensure!(
			self.min_hunks > 0 && self.min_hunks <= self.max_hunks && self.max_hunks <= 128,
			"invalid hunk constraint"
		);
		Ok(())
	}

	fn admits(&self, shape: &MeasuredShape) -> bool {
		(self.min_changed_lines..=self.max_changed_lines).contains(&shape.changed_lines)
			&& (self.min_hunks..=self.max_hunks).contains(&shape.hunks)
	}
}
#[derive(Debug, Clone, Serialize)]
pub struct MeasuredShape {
	pub version:        &'static str,
	pub changed_lines:  usize,
	pub hunks:          usize,
	pub inserted_lines: usize,
	pub removed_lines:  usize,
}
pub fn measure_shape(
	input: &str,
	expected: &str,
	cancellation: &CancelToken,
) -> Result<MeasuredShape> {
	let placements = super::hunks::placements_from_diff(input, expected, cancellation)?;
	Ok(MeasuredShape {
		version:        CALIBRATION_VERSION,
		changed_lines:  placements
			.iter()
			.map(|p| p.old_len.max(p.new_lines.len()))
			.sum(),
		hunks:          placements.len(),
		inserted_lines: placements.iter().map(|p| p.new_lines.len()).sum(),
		removed_lines:  placements.iter().map(|p| p.old_len).sum(),
	})
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct ContentIdentity {
	pub original_sha256: String,
	pub mutated_sha256:  String,
}
impl ContentIdentity {
	pub(super) fn new(original: &str, mutated: &str) -> Self {
		Self {
			original_sha256: content_hash(original.as_bytes()),
			mutated_sha256:  content_hash(mutated.as_bytes()),
		}
	}

	pub fn validate(&self) -> Result<()> {
		for hash in [&self.original_sha256, &self.mutated_sha256] {
			ensure!(
				hash.len() == 64
					&& hash
						.bytes()
						.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
				"invalid content SHA-256"
			);
		}
		Ok(())
	}
}
/// A bounded read-only history; identity is the exact expected/input byte pair,
/// independent of filename, task ID, mutation label or machine-local path.
pub(super) fn content_history(history: &[ContentIdentity]) -> Result<BTreeSet<ContentIdentity>> {
	ensure!(history.len() <= 4096, "content history exceeds 4096 records");
	for id in history {
		id.validate()?;
	}
	Ok(history.iter().cloned().collect())
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationQuota {
	pub mutation: String,
	pub count:    usize,
	#[serde(default)]
	pub shape:    Option<ShapeConstraint>,
}
#[derive(Debug, Serialize)]
pub struct QuotaPlan {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub calibration: Option<CalibrationSource>,
	pub version:     &'static str,
	pub seed:        u64,
	pub cases:       Vec<GenerationCase>,
}
/// Expand explicit counts in input order. No implicit difficulty fallback,
/// distribution rounding, new PRNG or historical case-count equivalence.
pub fn plan_quotas(
	quotas: &[GenerationQuota],
	seed: u64,
	cancellation: &CancelToken,
) -> Result<QuotaPlan> {
	ensure!(!quotas.is_empty() && quotas.len() <= 64, "quota plan must contain 1..64 rows");
	let mut total = 0usize;
	for quota in quotas {
		cancellation.heartbeat()?;
		ensure!(MutationKind::from_name(&quota.mutation).is_some(), "unknown quota mutation");
		ensure!((1..=64).contains(&quota.count), "invalid quota case count");
		total = total
			.checked_add(quota.count)
			.ok_or_else(|| anyhow::anyhow!("quota count overflow"))?;
		ensure!(total <= 64, "quota plan exceeds 64 cases");
		if let Some(shape) = &quota.shape {
			shape.validate()?;
		}
	}
	let mut random = super::generate::Random::new(seed);
	let mut cases = Vec::with_capacity(total);
	for quota in quotas {
		for _ in 0..quota.count {
			cancellation.heartbeat()?;
			cases.push(GenerationCase {
				task_id:  format!("case-{:03}", cases.len() + 1),
				mutation: quota.mutation.clone(),
				seed:     random.next(),
				shape:    quota.shape.clone(),
			});
		}
	}
	Ok(QuotaPlan { version: CALIBRATION_VERSION, seed, cases, calibration: None })
}
/// Work quota applies before expensive candidate materialization, syntax
/// revalidation or line diff. It never silently lowers a shape constraint.
pub(super) struct CalibrationBudget {
	pub materialized: usize,
	bytes:            usize,
	line_work:        usize,
}
impl CalibrationBudget {
	pub fn new() -> Self {
		Self { materialized: 0, bytes: 0, line_work: 0 }
	}

	pub fn charge(
		&mut self,
		original: &str,
		estimated_mutated_bytes: usize,
		estimated_mutated_lines: usize,
	) -> Result<()> {
		self.materialized += 1;
		self.bytes = self
			.bytes
			.checked_add(original.len())
			.and_then(|n| n.checked_add(estimated_mutated_bytes))
			.ok_or_else(|| anyhow::anyhow!("calibration byte work overflow"))?;
		let old_lines = original.split('\n').count();
		self.line_work = self
			.line_work
			.checked_add(
				old_lines
					.checked_mul(estimated_mutated_lines)
					.ok_or_else(|| anyhow::anyhow!("calibration line work overflow"))?,
			)
			.ok_or_else(|| anyhow::anyhow!("calibration line work overflow"))?;
		ensure!(
			self.materialized <= 4096
				&& self.bytes <= 64 * 1024 * 1024
				&& self.line_work <= 16_000_000,
			"calibration exceeds candidate/byte/diff work budget"
		);
		Ok(())
	}
}
impl MeasuredShape {
	pub(super) fn admitted_by(&self, constraint: Option<&ShapeConstraint>) -> bool {
		constraint.is_none_or(|c| c.admits(self))
	}
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlanDocument {
	pub version:     String,
	pub seed:        u64,
	pub cases:       Vec<GenerationCase>,
	#[serde(default)]
	pub calibration: Option<CalibrationSource>,
}
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum PlanInput {
	Array(Vec<GenerationCase>),
	Document(PlanDocument),
}
pub(super) fn decode_plan(bytes: &[u8]) -> Result<PlanInput> {
	ensure!(bytes.len() <= 1024 * 1024, "case plan exceeds byte limit");
	Ok(match serde_json::from_slice(bytes)? {
		PlanInput::Array(cases) => PlanInput::Array(cases),
		PlanInput::Document(plan) => {
			ensure!(plan.version == CALIBRATION_VERSION, "unsupported generation plan version");
			if let Some(calibration) = &plan.calibration {
				calibration.validate()?;
			}
			PlanInput::Document(plan)
		},
	})
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationSource {
	pub source_sha256:     String,
	pub samples:           usize,
	pub sizes:             BTreeMap<String, usize>,
	pub apportioned_cases: usize,
	pub allocated:         BTreeMap<String, usize>,
}
use super::shapes::SIZE_BANDS as BANDS;

impl CalibrationSource {
	pub fn validate(&self) -> Result<()> {
		ContentIdentity {
			original_sha256: self.source_sha256.clone(),
			mutated_sha256:  self.source_sha256.clone(),
		}
		.validate()?;
		ensure!(
			self.samples > 0 && self.samples <= 100_000 && self.apportioned_cases <= 64,
			"invalid calibration sample/case count"
		);
		let valid = |map: &BTreeMap<String, usize>| {
			map.keys()
				.all(|key| BANDS.iter().any(|(name, ..)| key == name))
		};
		ensure!(valid(&self.sizes) && valid(&self.allocated), "unknown calibration size bucket");
		ensure!(
			self.sizes.values().all(|n| *n <= 100_000) && self.allocated.values().all(|n| *n <= 64),
			"calibration bucket count exceeds bound"
		);
		ensure!(
			self.sizes.values().sum::<usize>() == self.samples
				&& self.allocated.values().sum::<usize>() == self.apportioned_cases,
			"inconsistent calibration counts"
		);
		Ok(())
	}
}
#[derive(Deserialize)]
struct CalibrationInput {
	per_call: CalibrationDistribution,
}
#[derive(Deserialize)]
struct CalibrationDistribution {
	samples: usize,
	sizes:   BTreeMap<String, usize>,
}
/// Largest remainder apportionment, canonical band order for equal remainders.
/// Counts apply across unshaped cases; explicit constraints are never replaced.
pub fn plan_calibrated_quotas(
	quotas: &[GenerationQuota],
	seed: u64,
	bytes: &[u8],
	cancellation: &CancelToken,
) -> Result<QuotaPlan> {
	ensure!(bytes.len() <= 1024 * 1024, "calibration input exceeds byte limit");
	let mut plan = plan_quotas(quotas, seed, cancellation)?;
	let input: CalibrationInput = serde_json::from_slice(bytes)?;
	let mut source = CalibrationSource {
		source_sha256:     content_hash(bytes),
		samples:           input.per_call.samples,
		sizes:             input.per_call.sizes,
		apportioned_cases: 0,
		allocated:         BTreeMap::new(),
	};
	source.validate()?;
	let count = plan.cases.iter().filter(|c| c.shape.is_none()).count();
	let mut allocated = [0usize; 5];
	let mut remainders = Vec::new();
	for (index, (name, ..)) in BANDS.iter().enumerate() {
		let numerator = count * source.sizes.get(*name).copied().unwrap_or(0);
		allocated[index] = numerator / source.samples;
		remainders.push((numerator % source.samples, index));
	}
	remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
	for (_, index) in remainders
		.iter()
		.take(count - allocated.iter().sum::<usize>())
	{
		allocated[*index] += 1;
	}
	let mut constraints = Vec::with_capacity(count);
	for (index, (name, min, max)) in BANDS.iter().enumerate() {
		source.allocated.insert((*name).into(), allocated[index]);
		for _ in 0..allocated[index] {
			constraints.push(ShapeConstraint {
				min_changed_lines: *min,
				max_changed_lines: (*max).min(4000),
				min_hunks:         1,
				max_hunks:         128,
			});
		}
	}
	// Fill unshaped cases in canonical band order; allocation is
	// explicit/versioned, not a historical random size distribution.
	let mut constraints = constraints.into_iter();
	for case in &mut plan.cases {
		cancellation.heartbeat()?;
		if case.shape.is_none() {
			case.shape = constraints.next();
		}
	}
	source.apportioned_cases = count;
	source.validate()?;
	plan.calibration = Some(source);
	Ok(plan)
}
