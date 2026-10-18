//! Dependency-aware contract execution with source-bound result receipts.
//!
//! Passing transition checks never marks a product capability as verified.
use std::{
	collections::BTreeSet,
	env,
	path::{Path, PathBuf},
	process::{Command, Stdio},
	time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use harvest_runtime::{JailedFs, content_hash};
use serde::{Deserialize, Serialize};

use crate::{capabilities, migration};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
	Affected,
	Full,
	Release,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
	pub id:              String,
	pub command:         Vec<String>,
	pub timeout_seconds: u64,
	pub kind:            String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Unit {
	pub id:            String,
	pub paths:         Vec<String>,
	pub depends_on:    Vec<String>,
	pub checks:        Vec<String>,
	pub review:        String,
	pub deletion_unit: String,
	pub fixtures:      Vec<String>,
	pub targets:       Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
	pub units:         Vec<Unit>,
	pub checks:        Vec<Check>,
	pub release_gates: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Plan {
	pub requested_scope: Scope,
	pub effective_scope: Scope,
	pub reason:          String,
	pub units:           Vec<String>,
	pub checks:          Vec<Check>,
}

fn owns(prefix: &str, path: &str) -> bool {
	path == prefix
		|| path
			.strip_prefix(prefix)
			.is_some_and(|tail| tail.starts_with('/'))
}

fn source_path(path: &str) -> bool {
	!path.is_empty()
		&& !path.contains('\\')
		&& !Path::new(path).is_absolute()
		&& path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

pub fn inspect_graph(manifest: &serde_json::Value) -> Result<Graph> {
	let graph: Graph = serde_json::from_value(manifest["execution"].clone())
		.context("missing or invalid migration execution graph")?;
	let ids: BTreeSet<_> = graph.units.iter().map(|unit| unit.id.as_str()).collect();
	let checks: BTreeSet<_> = graph.checks.iter().map(|check| check.id.as_str()).collect();
	if ids.is_empty()
		|| ids.len() != graph.units.len()
		|| checks.is_empty()
		|| checks.len() != graph.checks.len()
	{
		bail!("empty or duplicate work units/checks");
	}
	for check in &graph.checks {
		if check.id.is_empty()
			|| check.command.is_empty()
			|| check.command[0].is_empty()
			|| !(1..=7200).contains(&check.timeout_seconds)
			|| check.kind.is_empty()
		{
			bail!("invalid executable check {}", check.id);
		}
	}
	for unit in &graph.units {
		if unit.paths.is_empty()
			|| !unit.paths.iter().all(|path| source_path(path))
			|| !matches!(unit.review.as_str(), "pending" | "reviewed")
			|| unit.deletion_unit.is_empty()
			|| unit.targets.is_empty()
			|| unit.checks.is_empty()
			|| !unit
				.checks
				.iter()
				.all(|check| checks.contains(check.as_str()))
			|| !unit
				.depends_on
				.iter()
				.all(|dep| ids.contains(dep.as_str()) && dep != &unit.id)
		{
			bail!("invalid work unit {}", unit.id);
		}
	}
	// Kahn's algorithm rejects cycles before executing any subprocess.
	let mut resolved = BTreeSet::new();
	loop {
		let old = resolved.len();
		for unit in &graph.units {
			if unit.depends_on.iter().all(|dep| resolved.contains(dep)) {
				resolved.insert(unit.id.clone());
			}
		}
		if resolved.len() == graph.units.len() {
			break;
		}
		if resolved.len() == old {
			bail!("cyclic work-unit dependencies");
		}
	}
	for row in manifest["capabilities"]
		.as_array()
		.context("missing capabilities")?
	{
		if !row["work_unit"]
			.as_str()
			.is_some_and(|unit| ids.contains(unit))
		{
			bail!("capability lacks a known work unit: {}", row["id"]);
		}
	}
	Ok(graph)
}

pub fn plan(graph: &Graph, scope: Scope, changed: &[String]) -> Result<Plan> {
	if !changed.iter().all(|path| source_path(path)) {
		bail!("invalid changed source path");
	}
	let broad = [
		"Cargo.toml",
		"Cargo.lock",
		"bun.lock",
		"rust-toolchain.toml",
		"package.json",
		".cargo",
		".config",
		".github",
		"xtask",
		"crates/harvest-protocol",
		"crates/harvest-runtime",
		"docs/rust-migration",
	];
	let mut reason = "explicit complete verification".to_owned();
	let mut effective = scope;
	let mut selected = BTreeSet::new();
	if scope == Scope::Affected {
		for path in changed {
			for unit in &graph.units {
				if unit.paths.iter().any(|prefix| owns(prefix, path)) {
					selected.insert(unit.id.clone());
				}
			}
		}
		let unknown = changed.iter().any(|path| {
			!graph
				.units
				.iter()
				.any(|unit| unit.paths.iter().any(|prefix| owns(prefix, path)))
		});
		let unreviewed = graph
			.units
			.iter()
			.any(|unit| selected.contains(&unit.id) && unit.review != "reviewed");
		if changed.is_empty()
			|| unknown
			|| unreviewed
			|| changed
				.iter()
				.any(|path| broad.iter().any(|prefix| owns(prefix, path)))
		{
			effective = Scope::Full;
			reason = "empty, unknown, unreviewed or shared/security/build inputs require full \
			          verification"
				.into();
		} else {
			reason = "changed work units plus transitive dependencies and consumers".into();
			// Dependents of changed units are affected. Include their prerequisites
			// afterwards without treating every unchanged prerequisite as a change.
			loop {
				let previous = selected.len();
				for unit in &graph.units {
					if unit.depends_on.iter().any(|dep| selected.contains(dep)) {
						selected.insert(unit.id.clone());
					}
				}
				if previous == selected.len() {
					break;
				}
			}
			loop {
				let previous = selected.len();
				for unit in &graph.units {
					if selected.contains(&unit.id) {
						selected.extend(unit.depends_on.iter().cloned());
					}
				}
				if previous == selected.len() {
					break;
				}
			}
		}
	}
	if effective != Scope::Affected {
		selected.extend(graph.units.iter().map(|unit| unit.id.clone()));
	}
	let requested: BTreeSet<_> = graph
		.units
		.iter()
		.filter(|unit| selected.contains(&unit.id))
		.flat_map(|unit| unit.checks.iter())
		.collect();
	let checks = graph
		.checks
		.iter()
		.filter(|check| requested.contains(&check.id))
		.cloned()
		.collect();
	Ok(Plan {
		requested_scope: scope,
		effective_scope: effective,
		reason,
		units: selected.into_iter().collect(),
		checks,
	})
}

#[derive(Serialize)]
pub struct CheckResult {
	pub check:      Check,
	pub passed:     bool,
	pub exit_code:  Option<i32>,
	pub timed_out:  bool,
	pub elapsed_ms: u128,
	pub error:      Option<String>,
}

pub fn run_check(root: &Path, check: &Check) -> Result<CheckResult> {
	let start = Instant::now();
	let mut command = Command::new(&check.command[0]);
	command
		.args(&check.command[1..])
		.current_dir(root)
		.stdin(Stdio::null())
		.stdout(Stdio::inherit())
		.stderr(Stdio::inherit());
	let mut child = match command.spawn() {
		Ok(child) => child,
		Err(error) => {
			return Ok(CheckResult {
				check:      check.clone(),
				passed:     false,
				exit_code:  None,
				timed_out:  false,
				elapsed_ms: start.elapsed().as_millis(),
				error:      Some(error.to_string()),
			});
		},
	};
	let mut timed_out = false;
	let status = loop {
		if let Some(status) = child.try_wait()? {
			break status;
		}
		if start.elapsed() >= Duration::from_secs(check.timeout_seconds) {
			timed_out = true;
			child.kill()?;
			break child.wait()?;
		}
		std::thread::sleep(Duration::from_millis(25));
	};
	Ok(CheckResult {
		check: check.clone(),
		passed: status.success() && !timed_out,
		exit_code: status.code(),
		timed_out,
		elapsed_ms: start.elapsed().as_millis(),
		error: None,
	})
}

#[derive(Serialize)]
struct Receipt {
	schema_version:           u32,
	source_fingerprint:       String,
	manifest_sha256:          String,
	toolchain:                String,
	profile:                  &'static str,
	target:                   String,
	plan:                     Plan,
	results:                  Vec<CheckResult>,
	passed:                   bool,
	certifies_product_parity: bool,
}

pub fn execute(
	root: Option<PathBuf>,
	scope: Scope,
	base: Option<String>,
	plan_only: bool,
	output: PathBuf,
) -> Result<()> {
	let fs = JailedFs::open(&root.unwrap_or(env::current_dir()?))?;
	let output = fs.relative(&output)?;
	// Receipts must not modify the source fingerprint they certify.
	if !output.starts_with("target/migration") {
		bail!("verification receipts must be under target/migration");
	}
	let manifest_bytes =
		fs.read(Path::new("docs/rust-migration/capabilities.json"), 16 * 1024 * 1024)?;
	let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)?;
	let progress = capabilities::inspect(&manifest)?;
	let graph = inspect_graph(&manifest)?;
	let inventory = migration::current_inventory(&fs)?;
	let mut changed = Vec::new();
	if scope == Scope::Affected {
		let base = base.as_deref().unwrap_or("HEAD");
		if base.starts_with('-') {
			bail!("base must be a Git revision");
		}
		let tracked = migration::git_bytes(fs.root(), &[
			"diff",
			"--no-ext-diff",
			"--no-textconv",
			"--name-only",
			"-z",
			base,
			"--",
		])?;
		let untracked =
			migration::git_bytes(fs.root(), &["ls-files", "--others", "--exclude-standard", "-z"])?;
		for name in tracked
			.split(|byte| *byte == 0)
			.chain(untracked.split(|byte| *byte == 0))
			.filter(|name| !name.is_empty())
		{
			changed.push(std::str::from_utf8(name)?.to_owned());
		}
	}
	let plan = plan(&graph, scope, &changed)?;
	if plan_only {
		println!("{}", serde_json::to_string_pretty(&plan)?);
		return Ok(());
	}
	if scope == Scope::Release {
		progress.require_complete()?;
		if inventory.authored_js_ts_files != 0 || !inventory.script_asset_candidates.is_empty() {
			bail!("release source ownership remains incomplete");
		}
		for gate in &graph.release_gates {
			if !plan.checks.iter().any(|check| &check.kind == gate) {
				bail!("release gate {gate} has no executable verification command");
			}
		}
		for required in [
			"contracts",
			"sdk",
			"plugins",
			"browser",
			"installed",
			"performance",
			"registry",
			"signatures",
		] {
			if !graph.release_gates.iter().any(|gate| gate == required) {
				bail!("missing mandatory release gate {required}");
			}
		}
	}
	let toolchain = String::from_utf8(fs.read(Path::new("rust-toolchain.toml"), 64 * 1024)?)
		.context("toolchain configuration must be UTF-8")?;
	let mut receipt = Receipt {
		schema_version: 1,
		source_fingerprint: inventory.source_fingerprint,
		manifest_sha256: content_hash(&manifest_bytes),
		toolchain,
		profile: "per-check",
		target: format!("{}-{}", env::consts::ARCH, env::consts::OS),
		plan,
		results: Vec::new(),
		passed: false,
		certifies_product_parity: false,
	};
	for check in &receipt.plan.checks {
		println!("Verifying {}", check.id);
		let result = run_check(fs.root(), check)?;
		let passed = result.passed;
		receipt.results.push(result);
		if !passed {
			break;
		}
	}
	let stable = migration::current_inventory(&fs)?.source_fingerprint == receipt.source_fingerprint;
	receipt.passed = stable
		&& receipt.results.len() == receipt.plan.checks.len()
		&& receipt.results.iter().all(|result| result.passed);
	fs.atomic_write(&output, &serde_json::to_vec_pretty(&receipt)?)?;
	if !receipt.passed {
		bail!("verification failed or source changed; receipt: {}", output.display());
	}
	println!(
		"Transition checks passed; product parity is separately gated. Receipt: {}",
		output.display()
	);
	Ok(())
}
