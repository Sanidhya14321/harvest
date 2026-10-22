//! Release gates execute registered behavior suites only after the required
//! retained capabilities have independently verified candidate installations.
use std::{
	env,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::JailedFs;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gate {
	pub capabilities: Vec<String>,
	pub checks:       Vec<String>,
}

pub fn inspect(manifest: &Value, kind: &str) -> Result<Vec<crate::verification::Check>> {
	crate::capabilities::inspect(manifest)?;
	let graph = crate::verification::inspect_graph(manifest)?;
	let gate = graph
		.acceptance
		.get(kind)
		.with_context(|| format!("unknown acceptance gate {kind}"))?;
	if gate.capabilities.is_empty() || gate.checks.is_empty() {
		bail!(
			"acceptance gate {kind} is pending: register its installed behavioral suites before \
			 release"
		);
	}
	for id in &gate.capabilities {
		let row = manifest["capabilities"]
			.as_array()
			.context("missing capabilities")?
			.iter()
			.find(|row| row["id"] == *id)
			.with_context(|| format!("unknown required capability {id}"))?;
		if row["scope"] == "retired"
			|| row["verification"]["status"] != "verified"
			|| (manifest["version"] == 4 && row["lifecycle"]["candidate"]["status"] != "verified")
		{
			bail!("acceptance gate {kind} blocked by unverified installed capability {id}");
		}
	}
	gate
		.checks
		.iter()
		.map(|id| {
			let check = graph
				.checks
				.iter()
				.find(|check| &check.id == id)
				.with_context(|| format!("acceptance gate {kind} has unknown check {id}"))?;
			if graph.acceptance.contains_key(&check.kind)
				|| check
					.command
					.iter()
					.any(|arg| matches!(arg.as_str(), "acceptance" | "source-audit"))
			{
				bail!("acceptance gate {kind} cannot recursively execute gate {id}");
			}
			Ok(check.clone())
		})
		.collect()
}

#[derive(Parser)]
#[command(name = "cargo xtask acceptance")]
struct Arguments {
	kind: String,
	#[arg(long)]
	root: Option<PathBuf>,
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let args = Arguments::try_parse_from(std::iter::once("acceptance".into()).chain(arguments))?;
	let fs = JailedFs::open(&args.root.unwrap_or(env::current_dir()?))?;
	let bytes = fs.read(Path::new("docs/rust-migration/capabilities.json"), 16 * 1024 * 1024)?;
	let manifest: Value = serde_json::from_slice(&bytes)?;
	let checks = inspect(&manifest, &args.kind)?;
	let inventory = crate::source_audit::current_inventory(&fs)?;
	let mut results = Vec::new();
	for check in &checks {
		let result = crate::verification::run_check(fs.root(), check)?;
		let passed = result.passed;
		results.push(result);
		if !passed {
			break;
		}
	}
	let passed = results.len() == checks.len()
		&& results.iter().all(|result| result.passed)
		&& crate::source_audit::current_inventory(&fs)?.source_fingerprint
			== inventory.source_fingerprint;
	let output = PathBuf::from("target/migration/acceptance").join(format!("{}.json", args.kind));
	fs.atomic_write(
		&output,
		&serde_json::to_vec_pretty(&serde_json::json!({
			"gate": args.kind, "source_fingerprint": inventory.source_fingerprint,
			"manifest_sha256": harvest_runtime::content_hash(&bytes), "results": results, "passed": passed
		}))?,
	)?;
	if !passed {
		bail!("acceptance gate failed; receipt: {}", output.display());
	}
	println!("Acceptance gate passed; receipt: {}", output.display());
	Ok(())
}
