//! One deterministic rubric for admission and organization, with separate
//! gates.

use std::{collections::BTreeSet, sync::LazyLock};

use anyhow::{Result, bail};
use harvest_runtime::content_hash;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{
	Candidate, EvidenceEvent, EvidenceKind, EvidenceLedger, MAX_CANDIDATE_BYTES, RecordStatus, Scope,
};

static SECRETS: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(
    r"(?i)(-----BEGIN[^\n]*PRIVATE KEY-----|\b(?:sk-[a-z0-9_-]{16,}|gh[pousr]_[a-z0-9]{20,}|github_pat_[a-z0-9_]{20,}|AKIA[A-Z0-9]{16})\b|(?:api[_-]?key|password|secret|authorization)\s*[:=]\s*[^\s]{8,})"
).expect("static secret matcher")
});
static AUTHORITY: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(
    r"(?i)(?:ignore|bypass|disable|override)\s+(?:all\s+|the\s+)?(?:approval|permission|security|system instruction|user instruction|safety)|(?:never|do not|don't)\s+(?:ask|require)\s+(?:for\s+)?(?:approval|permission)"
).expect("static authority matcher")
});
static PROJECT_CONTENT: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(
    r"(?m)(```|~~~|(?:^|\s)(?:/home/|/Users/|/tmp/|[A-Za-z]:\\)|(?:^|\s)(?:src|packages|crates|python)/|\.git\b|\.harvest/|https?://[^\s]+)"
).expect("static project content matcher")
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grade {
	pub evidence:      f64,
	pub reliability:   f64,
	pub applicability: f64,
	pub utility:       f64,
	pub compactness:   f64,
	pub total:         f64,
	pub status:        RecordStatus,
	pub reason:        String,
}

pub fn normalize_content(content: &str) -> String {
	content
		.lines()
		.map(str::trim_end)
		.collect::<Vec<_>>()
		.join("\n")
		.trim()
		.to_owned()
}

pub fn record_fingerprint(candidate: &Candidate) -> Result<String> {
	let normalized = (&candidate.kind, normalize_content(&candidate.content), &candidate.conditions);
	Ok(content_hash(&serde_json::to_vec(&normalized)?))
}

pub fn validate_content(content: &str, scope: Scope) -> Result<()> {
	if content.trim().is_empty() {
		bail!("record content is empty");
	}
	if SECRETS.is_match(content) {
		bail!("record contains possible credentials or private key material");
	}
	if AUTHORITY.is_match(content) {
		bail!("record attempts to change approval or instruction authority");
	}
	if scope == Scope::User && PROJECT_CONTENT.is_match(content) {
		bail!("global record contains project-specific material");
	}
	Ok(())
}

pub fn grade(candidate: &Candidate, scope: Scope, ledger: &EvidenceLedger) -> Result<Grade> {
	if candidate.title.trim().is_empty() || candidate.title.len() > 160 {
		bail!("record requires a short title");
	}
	if candidate.content.len() > MAX_CANDIDATE_BYTES {
		bail!("candidate exceeds 8 KiB");
	}
	validate_content(&candidate.content, scope)?;
	validate_content(&candidate.title, scope)?;
	if candidate.evidence.len() > 128
		|| candidate.conditions.len() > 32
		|| candidate.depends_on.len() + candidate.relates_to.len() > 64
	{
		bail!("candidate metadata exceeds limits");
	}
	for value in candidate
		.conditions
		.iter()
		.chain(&candidate.depends_on)
		.chain(&candidate.relates_to)
	{
		if value.len() > 512 {
			bail!("candidate metadata value exceeds limits");
		}
		validate_content(value, scope)?;
	}
	let supported = ledger.supporting(candidate, scope)?;
	let explicit = supported.iter().any(|event| {
		matches!(event.kind, EvidenceKind::UserPreference | EvidenceKind::UserCorrection)
	});
	let verified_sessions: BTreeSet<_> = supported
		.iter()
		.filter(|event| event.kind == EvidenceKind::VerificationSuccess)
		.map(|event| event.session_id.as_str())
		.collect();
	let fingerprint = record_fingerprint(candidate)?;
	// Every supporting event is bound by the host to this exact lesson revision.
	// Merely successful commands are not evidence for an unrelated procedure.
	if supported
		.iter()
		.any(|event| event.record_hash.as_deref() != Some(&fingerprint))
	{
		bail!("supporting evidence does not authorize this record content");
	}
	let global_explicit = supported.iter().any(|event| {
		matches!(event.kind, EvidenceKind::UserPreference | EvidenceKind::UserCorrection)
			&& event.cross_project
	});
	let projects: BTreeSet<_> = supported
		.iter()
		.filter(|event| event.kind == EvidenceKind::VerificationSuccess)
		.filter_map(|event| event.project_id.as_deref())
		.collect();
	let scope_admitted = scope == Scope::Project || global_explicit || projects.len() >= 3;
	let related: Vec<&EvidenceEvent> = ledger
		.events
		.values()
		.filter(|event| {
			event.record_hash.as_deref() == Some(&fingerprint)
				&& candidate.conditions.is_subset(&event.conditions)
		})
		.collect();
	let successes = supported
		.iter()
		.filter(|event| event.kind == EvidenceKind::VerificationSuccess)
		.map(|event| (&event.session_id, &event.task_id))
		.chain(
			related
				.iter()
				.filter(|event| event.kind == EvidenceKind::BeneficialApplication)
				.map(|event| (&event.session_id, &event.task_id)),
		)
		.collect::<BTreeSet<_>>()
		.len();
	let failures = related
		.iter()
		.filter(|event| {
			matches!(event.kind, EvidenceKind::VerificationFailure | EvidenceKind::RejectedApplication)
		})
		.map(|event| (&event.session_id, &event.task_id))
		.collect::<BTreeSet<_>>()
		.len();
	let presentations: BTreeSet<_> = related
		.iter()
		.filter(|event| event.kind == EvidenceKind::Presentation)
		.map(|event| (&event.session_id, &event.task_id))
		.collect();
	let presentation_sessions: BTreeSet<_> = presentations
		.iter()
		.map(|(session, _)| session.as_str())
		.collect();
	let beneficial: BTreeSet<_> = related
		.iter()
		.filter(|event| event.kind == EvidenceKind::BeneficialApplication)
		.map(|event| (&event.session_id, &event.task_id))
		.collect();
	let evidence = if explicit {
		1.0
	} else {
		(verified_sessions.len() as f64 / 3.0).min(1.0)
	};
	let reliability = (successes as f64 + 1.0) / (successes as f64 + failures as f64 + 2.0);
	let applicability = if explicit || verified_sessions.len() >= 2 {
		1.0
	} else {
		0.5
	};
	let utility = if presentations.len() >= 5 && presentation_sessions.len() >= 3 {
		(beneficial.intersection(&presentations).count() as f64 / presentations.len() as f64).min(1.0)
	} else {
		0.5
	};
	let compactness = if candidate.content.len() <= 2048 {
		1.0
	} else if candidate.content.len() <= 4096 {
		0.75
	} else {
		0.5
	};
	let total = 10.0_f64.mul_add(
		compactness,
		15.0_f64.mul_add(
			utility,
			20.0_f64.mul_add(applicability, 25.0_f64.mul_add(reliability, 30.0 * evidence)),
		),
	);
	let evidence_admitted = explicit || verified_sessions.len() >= 2;
	let status = if total >= 70.0 && evidence_admitted && scope_admitted {
		RecordStatus::Active
	} else {
		RecordStatus::Candidate
	};
	let reason = if !scope_admitted {
		"keep local: global learning needs explicit intent or three distinct projects"
	} else if !evidence_admitted {
		"needs explicit intent or two independently verified sessions"
	} else if total < 70.0 {
		"grade is below the admission threshold"
	} else {
		"supported admission"
	}
	.to_owned();
	Ok(Grade { evidence, reliability, applicability, utility, compactness, total, status, reason })
}
