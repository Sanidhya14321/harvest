//! Implementation evidence, intentional retirement and distribution cutover are
//! separate gates.
use anyhow::{Context, Result, bail};
use serde_json::Value;

pub struct Progress {
	pub total:              usize,
	pub retained:           usize,
	pub retired:            usize,
	pub removed:            usize,
	pub implemented:        usize,
	pub verified:           usize,
	pub candidate_verified: usize,
	pub legacy_removed:     usize,
	pub publicly_promoted:  usize,
}
impl Progress {
	/// Retired interfaces must be removed; their removal never establishes
	/// parity for a retained capability.
	pub fn require_complete(&self) -> Result<()> {
		if self.verified != self.retained
			|| self.candidate_verified != self.retained
			|| self.legacy_removed != self.retained
			|| self.removed != self.retired
		{
			bail!(
				"distribution cutover blocked: retained capabilities lack verified Rust parity or \
				 retired interfaces lack verified removal"
			);
		}
		Ok(())
	}
}

fn nonempty_text(value: &Value) -> bool {
	value.as_str().is_some_and(|text| !text.trim().is_empty())
}
fn evidence_list(value: &Value) -> bool {
	value
		.as_array()
		.is_some_and(|items| !items.is_empty() && items.iter().all(nonempty_text))
}

pub fn inspect(manifest: &Value) -> Result<Progress> {
	if !matches!(manifest["version"].as_u64(), Some(2..=4)) {
		bail!("unsupported capability manifest version");
	}
	let rows = manifest["capabilities"]
		.as_array()
		.context("invalid capability manifest")?;
	if rows.is_empty() {
		bail!("empty capability manifest cannot establish parity");
	}
	let mut progress = Progress {
		total:              rows.len(),
		retained:           0,
		retired:            0,
		removed:            0,
		implemented:        0,
		verified:           0,
		candidate_verified: 0,
		legacy_removed:     0,
		publicly_promoted:  0,
	};
	let mut ids = std::collections::BTreeMap::new();
	for row in rows {
		let id = row["id"]
			.as_str()
			.filter(|id| !id.trim().is_empty())
			.context("missing capability ID")?;
		if ids.insert(id, row).is_some() {
			bail!("duplicate capability ID {id}");
		}
	}
	for (id, row) in &ids {
		let scope = if row["scope"].is_null() {
			"retained"
		} else {
			row["scope"].as_str().context("invalid capability scope")?
		};
		if !matches!(scope, "retained" | "retired")
			|| (scope == "retired" && manifest["version"] == 2)
		{
			bail!("invalid capability scope {id}");
		}
		let implementation = row["implementation"]["status"]
			.as_str()
			.context("missing implementation status")?;
		let verification = row["verification"]["status"]
			.as_str()
			.context("missing verification status")?;
		if !matches!(implementation, "absent" | "partial" | "complete")
			|| !matches!(verification, "unverified" | "verified")
		{
			bail!("invalid capability status {id}");
		}
		if scope == "retired" {
			if implementation != "absent" || verification != "unverified" {
				bail!("retirement cannot count as Rust parity: {id}");
			}
			let retirement = &row["retirement"];
			if !nonempty_text(&retirement["reason"]) || !nonempty_text(&retirement["evidence"]) {
				bail!("retirement missing approved scope evidence: {id}");
			}
			match retirement["decision"].as_str() {
				Some("rust-api-replacement")
					if id.starts_with("sdk-import:")
						|| matches!(
							*id,
							"compat:extension-identity"
								| "compat:custom-editor"
								| "compat:custom-renderers"
						) =>
				{
					if !evidence_list(&retirement["replacement_capabilities"]) {
						bail!("retired interface missing retained replacements: {id}");
					}
					for replacement in retirement["replacement_capabilities"]
						.as_array()
						.context("invalid replacements")?
					{
						let replacement_id = replacement.as_str().context("invalid replacement ID")?;
						let replacement = ids
							.get(replacement_id)
							.context("unknown replacement capability")?;
						if replacement["scope"] == "retired" || replacement_id == *id {
							bail!("replacement must be a retained capability: {id}");
						}
					}
				},
				Some("remove-js-eval")
					if *id == "tool:eval-javascript" || id.starts_with("worker:eval/js/") => {},
				_ => bail!("unapproved retirement decision: {id}"),
			}
			match retirement["status"].as_str() {
				Some("pending") => {},
				Some("removed")
					if evidence_list(&retirement["contract_suites"])
						&& row["legacy"]["removable"] == true =>
				{
					progress.removed += 1;
				},
				_ => bail!("retirement lacks removal contract evidence: {id}"),
			}
			progress.retired += 1;
			continue;
		}
		if !row["retirement"].is_null() {
			bail!("retained capability cannot declare retirement: {id}");
		}
		progress.retained += 1;
		if implementation != "absent" {
			if !evidence_list(&row["implementation"]["paths"]) {
				bail!("implementation missing Rust paths: {id}");
			}
			progress.implemented += 1;
		}
		if verification == "verified" {
			if implementation != "complete"
				|| !evidence_list(&row["verification"]["contract_suites"])
				|| (manifest["version"] != 4
					&& (row["installation"] != "default-rust" || row["legacy"]["removable"] != true))
			{
				bail!(
					"verified capability lacks complete implementation, contract or cutover evidence: \
					 {id}"
				);
			}
			progress.verified += 1;
		}
		if manifest["version"] == 4 {
			let stages = &row["lifecycle"];
			let mut previous_complete = verification == "verified";
			for (stage, count) in [
				("candidate", &mut progress.candidate_verified),
				("legacy", &mut progress.legacy_removed),
				("promotion", &mut progress.publicly_promoted),
			] {
				match stages[stage]["status"].as_str() {
					Some("pending") => previous_complete = false,
					Some("verified")
						if previous_complete && evidence_list(&stages[stage]["evidence"]) =>
					{
						if stage == "candidate"
							&& !matches!(
								row["installation"].as_str(),
								Some("candidate-default-rust" | "default-rust")
							) {
							bail!("candidate verification lacks Rust installation: {id}");
						}
						if stage == "legacy" && row["legacy"]["removable"] != true {
							bail!("legacy verification lacks removal evidence: {id}");
						}
						*count += 1;
					},
					_ => bail!("invalid or premature {stage} lifecycle evidence: {id}"),
				}
			}
		} else if verification == "verified" {
			progress.candidate_verified += 1;
			progress.legacy_removed += 1;
		}
	}
	if progress.retained == 0 {
		bail!("retired interfaces alone cannot establish parity");
	}
	Ok(progress)
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	fn retained() -> Value {
		json!({"id":"session","implementation":{"status":"complete","paths":["session.rs"]},"verification":{"status":"verified","contract_suites":["session_contract"]},"installation":"default-rust","legacy":{"removable":true}})
	}

	#[test]
	fn candidate_verification_and_source_removal_precede_public_promotion_without_a_cycle()
	-> Result<()> {
		let mut row = retained();
		row["installation"] = json!("experimental-rust-cli");
		row["legacy"]["removable"] = json!(false);
		row["lifecycle"] = json!({"candidate":{"status":"pending"},"legacy":{"status":"pending"},"promotion":{"status":"pending"}});
		let progress = inspect(&json!({"version":4,"capabilities":[row.clone()]}))?;
		assert_eq!((progress.verified, progress.candidate_verified), (1, 0));
		assert!(progress.require_complete().is_err());
		row["lifecycle"]["legacy"] = json!({"status":"verified","evidence":["removed"]});
		assert!(inspect(&json!({"version":4,"capabilities":[row.clone()]})).is_err());
		row["installation"] = json!("candidate-default-rust");
		row["lifecycle"]["candidate"] =
			json!({"status":"verified","evidence":["installed-artifact-receipt"]});
		row["legacy"]["removable"] = json!(true);
		let progress = inspect(&json!({"version":4,"capabilities":[row]}))?;
		progress.require_complete()?;
		assert_eq!(progress.publicly_promoted, 0);
		Ok(())
	}
	fn retired() -> Value {
		json!({"id":"sdk-import:legacy","scope":"retired","implementation":{"status":"absent"},"verification":{"status":"unverified"},"legacy":{"removable":false},"retirement":{"decision":"rust-api-replacement","reason":"Rust API replaces JS import","evidence":"scope.md","replacement_capabilities":["session"],"status":"pending"}})
	}

	#[test]
	fn partial_implementation_does_not_allow_distribution_cutover() -> Result<()> {
		let mut row = retained();
		row["implementation"]["status"] = json!("partial");
		row["verification"]["status"] = json!("unverified");
		let progress = inspect(&json!({"version":2,"capabilities":[row]}))?;
		assert_eq!((progress.total, progress.implemented, progress.verified), (1, 1, 0));
		assert!(progress.require_complete().is_err());
		Ok(())
	}
	#[test]
	fn verified_claim_without_default_installation_or_evidence_is_rejected() {
		let mut row = retained();
		row["installation"] = json!("experimental-rust-cli");
		assert!(inspect(&json!({"version":2,"capabilities":[row]})).is_err());
		let mut row = retained();
		row["verification"]["contract_suites"] = json!([""]);
		assert!(inspect(&json!({"version":2,"capabilities":[row]})).is_err());
		assert!(inspect(&json!({"version":2,"capabilities":[]})).is_err());
	}
	#[test]
	fn pending_removal_blocks_release_even_when_retained_replacements_pass() -> Result<()> {
		let progress = inspect(&json!({"version":3,"capabilities":[retained(),retired()]}))?;
		assert_eq!(
			(progress.total, progress.retained, progress.retired, progress.removed, progress.verified),
			(2, 1, 1, 0, 1)
		);
		assert!(progress.require_complete().is_err());
		let mut row = retired();
		row["retirement"]["status"] = json!("removed");
		row["retirement"]["contract_suites"] = json!(["rust-sdk-consumer"]);
		row["legacy"]["removable"] = json!(true);
		let progress = inspect(&json!({"version":3,"capabilities":[retained(),row]}))?;
		assert_eq!((progress.verified, progress.removed), (1, 1));
		progress.require_complete()?;
		Ok(())
	}
	#[test]
	fn retirement_cannot_hide_missing_replacements_or_claim_verified_parity() {
		let mut row = retired();
		row["retirement"]["replacement_capabilities"] = json!(["missing"]);
		assert!(inspect(&json!({"version":3,"capabilities":[retained(),row]})).is_err());
		let mut row = retired();
		row["verification"]["status"] = json!("verified");
		assert!(inspect(&json!({"version":3,"capabilities":[retained(),row]})).is_err());
		let mut row = retired();
		row["retirement"]["status"] = json!("removed");
		assert!(inspect(&json!({"version":3,"capabilities":[retained(),row]})).is_err());
	}
	#[test]
	fn removal_policy_cannot_retire_python_eval_or_the_entire_application() {
		let mut row = retired();
		row["id"] = json!("tool:eval");
		row["retirement"]["decision"] = json!("remove-js-eval");
		assert!(inspect(&json!({"version":3,"capabilities":[retained(),row]})).is_err());
		let mut row = retired();
		row["id"] = json!("tool:eval-javascript");
		row["retirement"]["decision"] = json!("remove-js-eval");
		assert!(inspect(&json!({"version":3,"capabilities":[row]})).is_err());
		let mut row = retained();
		row["scope"] = json!("typo");
		assert!(inspect(&json!({"version":3,"capabilities":[row]})).is_err());
	}
}
