//! Implementation evidence and release compatibility are separate gates.
use anyhow::{Context, Result, bail};
use serde_json::Value;

pub struct Progress {
	pub total:       usize,
	pub implemented: usize,
	pub verified:    usize,
}
pub fn inspect(manifest: &Value) -> Result<Progress> {
	if manifest["version"] != 2 {
		bail!("unsupported capability manifest version");
	}
	let rows = manifest["capabilities"]
		.as_array()
		.context("invalid capability manifest")?;
	if rows.is_empty() {
		bail!("empty capability manifest cannot establish parity");
	}
	let mut progress = Progress { total: rows.len(), implemented: 0, verified: 0 };
	let mut ids = std::collections::BTreeSet::new();
	for row in rows {
		let id = row["id"].as_str().context("missing capability ID")?;
		if !ids.insert(id) {
			bail!("duplicate capability ID {id}");
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
		if implementation != "absent" {
			if row["implementation"]["paths"]
				.as_array()
				.is_none_or(Vec::is_empty)
			{
				bail!("implementation missing Rust paths: {id}");
			}
			progress.implemented += 1;
		}
		if verification == "verified" {
			if implementation != "complete"
				|| row["verification"]["contract_suites"]
					.as_array()
					.is_none_or(Vec::is_empty)
				|| row["installation"] != "default-rust"
				|| row["legacy"]["removable"] != true
			{
				bail!(
					"verified capability lacks complete implementation, contract or cutover evidence: \
					 {id}"
				);
			}
			progress.verified += 1;
		}
	}
	Ok(progress)
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;
	#[test]
	fn partial_implementation_does_not_count_as_verified_parity() -> Result<()> {
		let progress = inspect(
			&json!({"version":2,"capabilities":[{"id":"session","implementation":{"status":"partial","paths":["session.rs"]},"verification":{"status":"unverified","contract_suites":["session_contract"]},"installation":"experimental-rust-cli","legacy":{"removable":false}}]}),
		)?;
		assert_eq!((progress.total, progress.implemented, progress.verified), (1, 1, 0));
		Ok(())
	}
	#[test]
	fn verified_claim_without_default_installation_is_rejected() {
		let row = json!({"id":"session","implementation":{"status":"complete","paths":["session.rs"]},"verification":{"status":"verified","contract_suites":["session_contract"]},"installation":"experimental-rust-cli","legacy":{"removable":true}});
		assert!(inspect(&json!({"version":2,"capabilities":[row]})).is_err());
		assert!(inspect(&json!({"version":2,"capabilities":[]})).is_err());
	}
}
