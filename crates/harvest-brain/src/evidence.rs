//! Runtime-owned evidence; extractor-provided counts and grades are never read.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::{Candidate, SCHEMA_VERSION, Scope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
	UserPreference,
	UserCorrection,
	VerificationSuccess,
	VerificationFailure,
	Presentation,
	BeneficialApplication,
	RejectedApplication,
	Supersession,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceEvent {
	pub id:            String,
	pub session_id:    String,
	pub task_id:       String,
	pub project_id:    Option<String>,
	pub kind:          EvidenceKind,
	pub timestamp:     u64,
	pub conditions:    BTreeSet<String>,
	pub record_hash:   Option<String>,
	pub cross_project: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceLedger {
	pub version: u32,
	pub events:  BTreeMap<String, EvidenceEvent>,
}

impl Default for EvidenceLedger {
	fn default() -> Self {
		Self { version: SCHEMA_VERSION, events: BTreeMap::new() }
	}
}

impl EvidenceLedger {
	/// Called only after the host has durably committed a genuine session event.
	/// This API is not exposed to an extraction model or the proposal tool.
	pub fn record_committed(&mut self, event: EvidenceEvent) -> Result<()> {
		if self.version != SCHEMA_VERSION {
			bail!("unsupported evidence schema");
		}
		if event.id.is_empty() || event.session_id.is_empty() || event.task_id.is_empty() {
			bail!("evidence requires committed event, session and task IDs");
		}
		if let Some(existing) = self.events.get(&event.id) {
			if serde_json::to_value(existing)? != serde_json::to_value(&event)? {
				bail!("evidence event ID already names different data");
			}
			return Ok(());
		}
		if self.events.len() >= 8192 {
			bail!("evidence ledger capacity reached");
		}
		self.events.insert(event.id.clone(), event);
		Ok(())
	}

	pub fn supporting<'a>(
		&'a self,
		candidate: &Candidate,
		scope: Scope,
	) -> Result<Vec<&'a EvidenceEvent>> {
		if self.version != SCHEMA_VERSION {
			bail!("unsupported evidence schema");
		}
		if candidate.evidence.is_empty() {
			bail!("proposal has no committed supporting evidence");
		}
		let mut events = Vec::new();
		for id in &candidate.evidence {
			let event = self
				.events
				.get(id)
				.ok_or_else(|| anyhow::anyhow!("evidence {id} is not a committed event"))?;
			if !candidate.conditions.is_subset(&event.conditions) {
				bail!("evidence does not support the declared conditions");
			}
			if !matches!(
				event.kind,
				EvidenceKind::UserPreference
					| EvidenceKind::UserCorrection
					| EvidenceKind::VerificationSuccess
			) {
				bail!("proposals require attributable intent or successful verification");
			}
			events.push(event);
		}
		if scope == Scope::Project {
			let projects: BTreeSet<_> = events
				.iter()
				.filter_map(|event| event.project_id.as_ref())
				.collect();
			if projects.len() > 1 {
				bail!("project proposal mixes different project identities");
			}
		}
		Ok(events)
	}
}
