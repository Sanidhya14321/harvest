//! Persistent source inventory: deletion is not evidence of a completed port.
use std::{collections::BTreeSet, path::Path, process::Command};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Baseline {
	version: u32,
	commit:  String,
	paths:   Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
	version: u32,
	entries: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
	path: String,
	disposition: String,
	implementation: String,
	behavioral_verification: String,
	installed_verification: String,
	legacy_removal: String,
	contract: Option<String>,
	consumers: Option<String>,
	persisted_formats: Option<String>,
	external_dependencies: Option<String>,
	rust_destination: Option<String>,
	verification_requirements: Option<String>,
	retirement_reason: Option<String>,
}

fn source(path: &str) -> bool {
	Path::new(path)
		.extension()
		.and_then(|value| value.to_str())
		.is_some_and(|extension| {
			matches!(
				extension,
				"ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" | "py" | "jl"
			)
		})
}

fn documented(value: Option<&str>) -> bool {
	value.is_some_and(|text| !text.trim().is_empty())
}

/// Returns completed entries only after independent contract and installation
/// evidence.
pub fn inspect(baseline: &Baseline, ledger: &Ledger, tracked: &[String]) -> Result<usize> {
	if baseline.version != 1
		|| ledger.version != 1
		|| baseline.commit.len() != 40
		|| baseline.paths.is_empty()
	{
		bail!("invalid migration baseline or ledger version");
	}
	let required: BTreeSet<&str> = baseline
		.paths
		.iter()
		.map(String::as_str)
		.chain(
			tracked
				.iter()
				.filter(|path| source(path))
				.map(String::as_str),
		)
		.collect();
	let mut seen = BTreeSet::new();
	let mut completed = 0;
	for entry in &ledger.entries {
		if !seen.insert(entry.path.as_str()) {
			bail!("duplicate migration entry: {}", entry.path);
		}
		if !matches!(
			entry.disposition.as_str(),
			"review-required" | "replace" | "retire" | "fixture" | "browser-glue"
		) || !matches!(entry.implementation.as_str(), "absent" | "partial" | "complete")
			|| !matches!(entry.behavioral_verification.as_str(), "unverified" | "verified")
			|| !matches!(entry.installed_verification.as_str(), "unverified" | "verified")
			|| !matches!(entry.legacy_removal.as_str(), "retained" | "removed")
		{
			bail!("invalid migration status: {}", entry.path);
		}
		if entry.disposition == "browser-glue"
			&& !matches!(
				Path::new(&entry.path)
					.extension()
					.and_then(|value| value.to_str()),
				Some("js" | "mjs" | "cjs")
			) {
			bail!("browser glue must be JavaScript bootstrap: {}", entry.path);
		}
		let reviewed = entry.disposition != "review-required";
		if reviewed
			&& [
				entry.contract.as_deref(),
				entry.consumers.as_deref(),
				entry.persisted_formats.as_deref(),
				entry.external_dependencies.as_deref(),
				entry.verification_requirements.as_deref(),
			]
			.iter()
			.any(|field| !documented(*field))
		{
			bail!("reviewed entry lacks contract analysis: {}", entry.path);
		}
		if reviewed
			&& !documented(if entry.disposition == "retire" {
				entry.retirement_reason.as_deref()
			} else {
				entry.rust_destination.as_deref()
			}) {
			bail!("entry lacks destination or retirement rationale: {}", entry.path);
		}
		let verified =
			entry.behavioral_verification == "verified" && entry.installed_verification == "verified";
		let complete = reviewed && entry.implementation == "complete" && verified;
		if (entry.behavioral_verification == "verified" || entry.installed_verification == "verified")
			&& (!reviewed || entry.implementation != "complete")
		{
			bail!("verification claimed before implementation: {}", entry.path);
		}
		let removed = !tracked.contains(&entry.path);
		if (entry.legacy_removal == "removed") != removed {
			bail!("legacy removal disagrees with tracked inventory: {}", entry.path);
		}
		if removed && !complete {
			bail!("source removed without verified replacement or retirement: {}", entry.path);
		}
		if complete && (removed || entry.disposition == "browser-glue") {
			completed += 1;
		}
	}
	if let Some(missing) = required.difference(&seen).next() {
		bail!("missing migration entry: {missing}");
	}
	Ok(completed)
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	if arguments
		.iter()
		.any(|argument| argument != "--require-complete")
	{
		bail!("usage: cargo xtask migration [--require-complete]");
	}
	let root = Path::new(env!("CARGO_MANIFEST_DIR"))
		.parent()
		.context("workspace root missing")?;
	let baseline = serde_json::from_slice(&std::fs::read(
		root.join("docs/rust-migration/source-baseline.json"),
	)?)?;
	let ledger: Ledger =
		serde_json::from_slice(&std::fs::read(root.join("docs/rust-migration/source-ledger.json"))?)?;
	let output = Command::new("git")
		.args(["ls-files", "-z"])
		.current_dir(root)
		.output()?;
	if !output.status.success() {
		bail!("could not enumerate tracked sources");
	}
	let tracked: Vec<String> = String::from_utf8(output.stdout)?
		.split('\0')
		.filter(|path| !path.is_empty())
		.map(str::to_owned)
		.collect();
	let completed = inspect(&baseline, &ledger, &tracked)?;
	println!("{completed}/{} source entries migrated and verified", ledger.entries.len());
	if arguments
		.iter()
		.any(|argument| argument == "--require-complete")
		&& completed != ledger.entries.len()
	{
		bail!("source migration incomplete; distribution cutover blocked");
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn missing_rows_and_deleted_sources_cannot_hide_pending_work() -> Result<()> {
		let baseline: Baseline =
			serde_json::from_value(json!({"version":1,"commit":"a".repeat(40),"paths":["app.ts"]}))?;
		let empty: Ledger = serde_json::from_value(json!({"version":1,"entries":[]}))?;
		assert!(inspect(&baseline, &empty, &[]).is_err());
		let pending: Ledger = serde_json::from_value(json!({"version":1,"entries":[{
			"path":"app.ts","disposition":"review-required","implementation":"absent",
			"behavioral_verification":"unverified","installed_verification":"unverified","legacy_removal":"retained"
		}]}))?;
		assert_eq!(inspect(&baseline, &pending, &["app.ts".into()])?, 0);
		assert!(inspect(&baseline, &pending, &[]).is_err());
		assert!(inspect(&baseline, &pending, &["app.ts".into(), "new.tsx".into()]).is_err());
		Ok(())
	}
	#[test]
	fn deletion_requires_both_verification_stages_and_retirement_rationale() -> Result<()> {
		let baseline: Baseline =
			serde_json::from_value(json!({"version":1,"commit":"a".repeat(40),"paths":["eval.ts"]}))?;
		let mut entry = json!({
			"path":"eval.ts", "disposition":"retire", "implementation":"complete",
			"behavioral_verification":"verified", "installed_verification":"verified",
			"legacy_removal":"removed", "contract":"JavaScript evaluation",
			"consumers":"CLI evaluation tool", "persisted_formats":"none",
			"external_dependencies":"Bun", "verification_requirements":"Migration error and native installation workflows",
			"retirement_reason":"Breaking release removes JavaScript execution"
		});
		let ledger = |value: &serde_json::Value| -> Result<Ledger> {
			Ok(serde_json::from_value(json!({"version":1,"entries":[value]}))?)
		};
		assert_eq!(inspect(&baseline, &ledger(&entry)?, &[])?, 1);
		entry["installed_verification"] = json!("unverified");
		assert!(inspect(&baseline, &ledger(&entry)?, &[]).is_err());
		entry["installed_verification"] = json!("verified");
		entry["retirement_reason"] = serde_json::Value::Null;
		assert!(inspect(&baseline, &ledger(&entry)?, &[]).is_err());
		Ok(())
	}
}
