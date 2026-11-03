//! Persistent work selection. Inventory coverage is distinct from semantic
//! review, source retirement and installed capability parity.
use std::{
	collections::{BTreeMap, BTreeSet},
	env,
	path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use harvest_runtime::JailedFs;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
	pub id:           String,
	pub kind:         String,
	pub state:        String,
	pub review:       String,
	pub work_unit:    String,
	pub depends_on:   Vec<String>,
	pub card:         String,
	pub capabilities: Vec<String>,
	pub checks:       Vec<String>,
	#[serde(default)]
	pub evidence:     Vec<String>,
	#[serde(default)]
	pub blocker:      Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Status {
	pub tasks:              BTreeMap<String, usize>,
	pub sources:            BTreeMap<String, usize>,
	pub capabilities:       BTreeMap<String, usize>,
	pub next:               Option<Task>,
	pub pending_acceptance: Vec<String>,
}

fn text(value: &str) -> bool {
	!value.trim().is_empty()
}

fn relative(path: &str) -> bool {
	!path.is_empty()
		&& !path.contains('\\')
		&& Path::new(path)
			.components()
			.all(|part| matches!(part, Component::Normal(_)))
}

/// Validate one authoritative dependency graph, source ownership and status
/// transitions before offering work to an executor.
pub fn inspect(manifest: &Value, ledger: &Value) -> Result<Status> {
	let graph = crate::verification::inspect_graph(manifest)?;
	let progress = crate::capabilities::inspect(manifest)?;
	if graph.tasks.is_empty() {
		bail!("migration task catalog is empty");
	}
	let tasks: BTreeMap<_, _> = graph
		.tasks
		.iter()
		.map(|task| (task.id.as_str(), task))
		.collect();
	if tasks.len() != graph.tasks.len() {
		bail!("duplicate migration task ID");
	}
	let units: BTreeSet<_> = graph.units.iter().map(|unit| unit.id.as_str()).collect();
	let checks: BTreeSet<_> = graph.checks.iter().map(|check| check.id.as_str()).collect();
	let capability_ids: BTreeSet<_> = manifest["capabilities"]
		.as_array()
		.context("missing capabilities")?
		.iter()
		.filter_map(|row| row["id"].as_str())
		.collect();
	let mut covered = BTreeSet::new();
	let mut counts = BTreeMap::new();
	for task in &graph.tasks {
		if !text(&task.id)
			|| !units.contains(task.work_unit.as_str())
			|| !matches!(task.kind.as_str(), "control" | "review" | "migration")
			|| !matches!(task.state.as_str(), "planned" | "ready" | "in_progress" | "blocked" | "done")
			|| !matches!(task.review.as_str(), "pending" | "reviewed")
			|| !relative(&task.card)
			|| task.checks.is_empty()
			|| task.checks.iter().any(|id| !checks.contains(id.as_str()))
			|| task
				.depends_on
				.iter()
				.any(|id| id == &task.id || !tasks.contains_key(id.as_str()))
			|| task
				.capabilities
				.iter()
				.any(|id| !capability_ids.contains(id.as_str()))
		{
			bail!("invalid migration task {}", task.id);
		}
		if matches!(task.state.as_str(), "ready" | "in_progress" | "done")
			&& (task.review != "reviewed"
				|| task
					.depends_on
					.iter()
					.any(|id| tasks[id.as_str()].state != "done"))
		{
			bail!("task {} is not reviewed or its prerequisites are unfinished", task.id);
		}
		if task.state == "done"
			&& (task.evidence.is_empty() || task.evidence.iter().any(|item| !text(item)))
		{
			bail!("completed task {} lacks verification evidence", task.id);
		}
		if task.state == "blocked" && !task.blocker.as_deref().is_some_and(text) {
			bail!("blocked task {} lacks an actionable blocker", task.id);
		}
		covered.extend(task.capabilities.iter().map(String::as_str));
		*counts.entry(task.state.clone()).or_default() += 1;
	}
	if let Some(id) = capability_ids.difference(&covered).next() {
		bail!("capability has no migration task: {id}");
	}
	let mut resolved = BTreeSet::new();
	loop {
		let previous = resolved.len();
		for task in &graph.tasks {
			if task
				.depends_on
				.iter()
				.all(|id| resolved.contains(id.as_str()))
			{
				resolved.insert(task.id.as_str());
			}
		}
		if resolved.len() == tasks.len() {
			break;
		}
		if resolved.len() == previous {
			bail!("cyclic migration task dependencies");
		}
	}
	let mut sources = BTreeMap::new();
	for row in ledger["entries"]
		.as_array()
		.context("missing source ledger entries")?
	{
		let path = row["path"].as_str().context("source path missing")?;
		let owner = row["task_id"]
			.as_str()
			.and_then(|id| tasks.get(id))
			.with_context(|| format!("source has no known migration task: {path}"))?;
		let removed = row["implementation"] == "complete"
			&& row["behavioral_verification"] == "verified"
			&& row["installed_verification"] == "verified"
			&& row["legacy_removal"] == "removed";
		let retained_inert = row["disposition"] == "fixture"
			&& serde_json::from_value::<crate::migration::Entry>(row.clone())?.verified_inert();
		let done = removed || retained_inert;
		if owner.kind == "migration" && owner.state == "done" && !done {
			bail!("completed migration task {} still owns unfinished source {path}", owner.id);
		}
		let state = if retained_inert {
			"verified_inert"
		} else if removed {
			"verified_removed"
		} else if row["disposition"] == "review-required" {
			"review_required"
		} else {
			"pending"
		};
		*sources.entry(state.into()).or_default() += 1;
	}
	let capabilities = BTreeMap::from([
		("retained".into(), progress.retained),
		("partial_or_complete".into(), progress.implemented),
		("verified".into(), progress.verified),
		("candidate_verified".into(), progress.candidate_verified),
		("legacy_removed".into(), progress.legacy_removed),
		("retired".into(), progress.retired),
		("retired_removed".into(), progress.removed),
	]);
	let next = graph
		.tasks
		.iter()
		.find(|task| task.state == "in_progress")
		.or_else(|| graph.tasks.iter().find(|task| task.state == "ready"))
		.cloned();
	let pending_acceptance = graph
		.acceptance
		.iter()
		.filter(|(_, gate)| gate.checks.is_empty())
		.map(|(kind, _)| kind.clone())
		.collect();
	Ok(Status { tasks: counts, sources, capabilities, next, pending_acceptance })
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Action {
	Check,
	Status,
	Next,
}

#[derive(Parser)]
#[command(name = "cargo xtask playbook")]
struct Arguments {
	#[arg(value_enum, default_value = "status")]
	action: Action,
	#[arg(long)]
	root:   Option<PathBuf>,
	#[arg(long)]
	json:   bool,
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let args = Arguments::try_parse_from(std::iter::once("playbook".into()).chain(arguments))?;
	let fs = JailedFs::open(&args.root.unwrap_or(env::current_dir()?))?;
	let manifest: Value = serde_json::from_slice(
		&fs.read(Path::new("docs/rust-migration/capabilities.json"), 16 * 1024 * 1024)?,
	)?;
	let ledger: Value = serde_json::from_slice(
		&fs.read(Path::new("docs/rust-migration/source-ledger.json"), 16 * 1024 * 1024)?,
	)?;
	let status = inspect(&manifest, &ledger)?;
	let owned: BTreeSet<_> = ledger["entries"]
		.as_array()
		.context("missing source ledger")?
		.iter()
		.filter_map(|row| row["path"].as_str())
		.collect();
	for path in &crate::source_audit::current_inventory(&fs)?.non_rust_executable_files {
		if !owned.contains(path.as_str()) {
			bail!("non-Rust executable source has no ledger/task ownership: {path}");
		}
	}
	let graph = crate::verification::inspect_graph(&manifest)?;
	for card in graph.tasks.iter().map(|task| &task.card) {
		if fs.read(Path::new(card), 1024 * 1024)?.is_empty() {
			bail!("empty task card {card}");
		}
	}
	if args.json {
		let value = if matches!(args.action, Action::Next) {
			serde_json::to_value(&status.next)?
		} else {
			serde_json::to_value(&status)?
		};
		println!("{}", serde_json::to_string_pretty(&value)?);
	} else {
		println!(
			"Tasks: {:?}\nSources: {:?}\nCapabilities: {:?}",
			status.tasks, status.sources, status.capabilities
		);
		if let Some(task) = status.next {
			println!("Next: {} ({}) — {}", task.id, task.state, task.card);
		} else {
			println!("No ready task. Review planned tasks or resolve their recorded blockers.");
		}
	}
	Ok(())
}
