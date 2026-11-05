use std::{path::Path, process::Command};

use anyhow::Result;
use harvest_runtime::JailedFs;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

fn capability() -> Value {
	json!({"id":"session","work_unit":"engine","implementation":{"status":"complete","paths":["engine/lib.rs"]},"verification":{"status":"verified","contract_suites":["engine-contracts"]},"installation":"candidate-default-rust","legacy":{"removable":false},"lifecycle":{"candidate":{"status":"pending"},"legacy":{"status":"pending"},"promotion":{"status":"pending"}}})
}

fn manifest() -> Value {
	json!({"version":4,"capabilities":[capability()],"execution":{
		"units":[{"id":"engine","paths":["engine"],"depends_on":[],"checks":["engine-contracts"],"review":"reviewed","deletion_unit":"engine","fixtures":[],"targets":["host"]}],
		"checks":[{"id":"engine-contracts","command":["cargo","xtask","test","-p","harvest-runtime"],"timeout_seconds":30,"kind":"contracts"}],
		"release_gates":["contracts","sdk"],
		"acceptance":{"sdk":{"capabilities":["session"],"checks":["engine-contracts"]}},
		"tasks":[
			{"id":"control","kind":"control","state":"in_progress","review":"reviewed","work_unit":"engine","depends_on":[],"card":"tasks/control.md","capabilities":[],"checks":["engine-contracts"],"evidence":[]},
			{"id":"port","kind":"migration","state":"planned","review":"pending","work_unit":"engine","depends_on":["control"],"card":"tasks/port.md","capabilities":["session"],"checks":["engine-contracts"],"evidence":[]}
		]
	}})
}

fn ledger() -> Value {
	json!({"version":1,"entries":[{"path":"engine/legacy.ts","task_id":"port","disposition":"review-required","implementation":"absent","behavioral_verification":"unverified","installed_verification":"unverified","legacy_removal":"retained"}]})
}

#[test]
fn work_selection_requires_reviewed_instructions_and_finished_prerequisites() -> Result<()> {
	let mut data = manifest();
	assert_eq!(xtask::playbook::inspect(&data, &ledger())?.next.unwrap().id, "control");
	data["execution"]["tasks"][1]["state"] = json!("ready");
	assert!(xtask::playbook::inspect(&data, &ledger()).is_err());
	data["execution"]["tasks"][1]["review"] = json!("reviewed");
	assert!(xtask::playbook::inspect(&data, &ledger()).is_err());
	data["execution"]["tasks"][0]["state"] = json!("done");
	data["execution"]["tasks"][0]["evidence"] = json!(["gate-transition-contract"]);
	assert_eq!(xtask::playbook::inspect(&data, &ledger())?.next.unwrap().id, "port");
	data["execution"]["tasks"][1]["state"] = json!("done");
	data["execution"]["tasks"][1]["evidence"] = json!(["alleged-parity"]);
	assert!(
		xtask::playbook::inspect(&data, &ledger())
			.unwrap_err()
			.to_string()
			.contains("unfinished source")
	);
	Ok(())
}

#[test]
fn unowned_sources_and_cyclic_tasks_cannot_disappear_from_the_queue() {
	let mut sources = ledger();
	sources["entries"][0]["task_id"] = json!("missing");
	assert!(
		xtask::playbook::inspect(&manifest(), &sources)
			.unwrap_err()
			.to_string()
			.contains("no known migration task")
	);
	let mut data = manifest();
	data["execution"]["tasks"][0]["state"] = json!("planned");
	data["execution"]["tasks"][0]["depends_on"] = json!(["port"]);
	assert!(
		xtask::playbook::inspect(&data, &ledger())
			.unwrap_err()
			.to_string()
			.contains("cyclic")
	);
}

#[test]
fn playbook_cli_rejects_new_extensionless_scripts_until_they_have_an_owner() -> Result<()> {
	let root = tempdir()?;
	assert!(
		Command::new("git")
			.args(["init", "--quiet"])
			.arg(root.path())
			.status()?
			.success()
	);
	let fs = JailedFs::open(root.path())?;
	fs.atomic_write(
		Path::new("docs/rust-migration/capabilities.json"),
		&serde_json::to_vec(&manifest())?,
	)?;
	fs.atomic_write(
		Path::new("docs/rust-migration/source-ledger.json"),
		&serde_json::to_vec(&ledger())?,
	)?;
	fs.atomic_write(Path::new("tasks/control.md"), b"reviewed control instructions")?;
	fs.atomic_write(Path::new("tasks/port.md"), b"pending source migration")?;
	fs.atomic_write(Path::new("tools/new-command"), b"#!/bin/sh\nexit 0\n")?;
	let run = || {
		Command::new(env!("CARGO_BIN_EXE_xtask"))
			.args(["playbook", "check", "--root"])
			.arg(root.path())
			.output()
	};
	let unowned = run()?;
	assert!(!unowned.status.success());
	assert!(String::from_utf8_lossy(&unowned.stderr).contains("tools/new-command"));
	let mut sources = ledger();
	let mut added = sources["entries"][0].clone();
	added["path"] = json!("tools/new-command");
	sources["entries"].as_array_mut().unwrap().push(added);
	fs.atomic_write(
		Path::new("docs/rust-migration/source-ledger.json"),
		&serde_json::to_vec(&sources)?,
	)?;
	let owned = run()?;
	assert!(owned.status.success(), "{}", String::from_utf8_lossy(&owned.stderr));
	Ok(())
}

#[test]
fn acceptance_requires_installed_evidence_then_dispatches_real_contract_commands() -> Result<()> {
	let mut data = manifest();
	assert!(
		xtask::acceptance::inspect(&data, "sdk")
			.unwrap_err()
			.to_string()
			.contains("unverified installed")
	);
	data["capabilities"][0]["lifecycle"]["candidate"] =
		json!({"status":"verified","evidence":["installed-consumer-receipt"]});
	let checks = xtask::acceptance::inspect(&data, "sdk")?;
	assert_eq!(checks[0].command, ["cargo", "xtask", "test", "-p", "harvest-runtime"]);
	data["execution"]["acceptance"]["sdk"]["checks"] = json!([]);
	assert!(
		xtask::acceptance::inspect(&data, "sdk")
			.unwrap_err()
			.to_string()
			.contains("pending")
	);
	data["execution"]["acceptance"]["sdk"]["checks"] = json!(["engine-contracts"]);
	data["execution"]["checks"][0]["command"] = json!(["cargo", "xtask", "acceptance", "sdk"]);
	assert!(
		xtask::acceptance::inspect(&data, "sdk")
			.unwrap_err()
			.to_string()
			.contains("recursively")
	);
	Ok(())
}

#[test]
fn capabilities_cli_rejects_parity_without_candidate_or_legacy_evidence() -> Result<()> {
	let root = tempdir()?;
	let fs = JailedFs::open(root.path())?;
	let mut data = manifest();
	let run = |data: &Value| -> Result<std::process::Output> {
		fs.atomic_write(
			Path::new("docs/rust-migration/capabilities.json"),
			&serde_json::to_vec(data)?,
		)?;
		Ok(Command::new(env!("CARGO_BIN_EXE_xtask"))
			.args(["capabilities", "--require-complete", "--root"])
			.arg(root.path())
			.output()?)
	};
	let missing = run(&data)?;
	assert!(!missing.status.success());
	assert!(String::from_utf8_lossy(&missing.stderr).contains("distribution cutover blocked"));
	data["capabilities"][0]["lifecycle"]["candidate"] =
		json!({"status":"verified","evidence":["installed-consumer"]});
	assert!(!run(&data)?.status.success());
	data["capabilities"][0]["legacy"]["removable"] = json!(true);
	data["capabilities"][0]["lifecycle"]["legacy"] =
		json!({"status":"verified","evidence":["removed-consumers"]});
	let complete = run(&data)?;
	assert!(complete.status.success(), "{}", String::from_utf8_lossy(&complete.stderr));
	Ok(())
}

#[test]
fn strict_inventory_counts_generated_scripts_templates_and_non_ts_runtimes() -> Result<()> {
	let root = tempdir()?;
	let fs = JailedFs::open(root.path())?;
	let files = [
		("packages/tui/src/fuzzy.js", b"export const generated = 1;".as_slice()),
		("xtask/templates/loader.js.in", b"loadWasm();".as_slice()),
		("tools/job.py", b"print('job')".as_slice()),
		("tools/job.jl", b"println(1)".as_slice()),
		("install.ps1", b"Write-Output 'install'".as_slice()),
		("tools/extensionless", b"#!/bin/sh\nexec tool".as_slice()),
		("crates/vendor/dependency/script.py", b"third_party()".as_slice()),
		("data/foreign-code.txt", b"const fixture = 'inert';".as_slice()),
		("engine.rs", b"fn main() {}".as_slice()),
	];
	let mut paths = Vec::new();
	for (name, bytes) in &files {
		fs.atomic_write(Path::new(name), bytes)?;
		paths.extend_from_slice(name.as_bytes());
		paths.push(0);
	}
	let inventory = xtask::source_audit::inspect_files(&fs, &paths, Path::new("report.json"))?;
	assert_eq!(inventory.non_rust_executable_files, [
		"install.ps1",
		"packages/tui/src/fuzzy.js",
		"tools/extensionless",
		"tools/job.jl",
		"tools/job.py",
		"xtask/templates/loader.js.in"
	]);
	assert_eq!(inventory.files_by_language["julia"], 1);
	assert_eq!(inventory.files_by_language["javascript"], 2);
	let fingerprint = inventory.source_fingerprint;
	fs.atomic_write(Path::new("xtask/templates/loader.js.in"), b"changedGeneratedLoader();")?;
	assert_ne!(
		xtask::source_audit::inspect_files(&fs, &paths, Path::new("report.json"))?.source_fingerprint,
		fingerprint
	);
	Ok(())
}

#[test]
fn native_replacement_retirement_cannot_remove_unrelated_retained_behavior() -> Result<()> {
	let mut data = manifest();
	let retired = json!({"id":"surface:browser-wasm","scope":"retired","implementation":{"status":"absent"},"verification":{"status":"unverified"},"legacy":{"removable":false},"retirement":{"decision":"native-interface-replacement","reason":"Approved native replacement","evidence":"scope.md","replacement_capabilities":["session"],"status":"pending"}});
	data["capabilities"].as_array_mut().unwrap().push(retired);
	assert_eq!(xtask::capabilities::inspect(&data)?.retired, 1);
	data["capabilities"][1]["id"] = json!("tool:bash");
	assert!(xtask::capabilities::inspect(&data).is_err());
	data["capabilities"][1]["id"] = json!("tool:eval");
	data["capabilities"][1]["retirement"]["decision"] = json!("remove-language-eval");
	assert_eq!(xtask::capabilities::inspect(&data)?.retired, 1);
	data["capabilities"][1]["id"] = json!("surface:python-automation");
	assert!(xtask::capabilities::inspect(&data).is_err());
	Ok(())
}

#[test]
fn verified_inert_inputs_have_a_separate_state_and_cannot_disguise_workers() -> Result<()> {
	let mut data = manifest();
	for task in data["execution"]["tasks"].as_array_mut().unwrap() {
		task["state"] = json!("done");
		task["review"] = json!("reviewed");
		task["evidence"] = json!(["native-fixture-consumer-contract"]);
	}
	let mut sources = ledger();
	let row = &mut sources["entries"][0];
	*row = json!({"path":"packages/coding-agent/test/fixtures/security/seeded-repository/src/safe-lookalike.ts",
		"task_id":"port","disposition":"fixture","implementation":"complete","behavioral_verification":"verified",
		"installed_verification":"verified","legacy_removal":"retained","contract":"Inert scanner input",
		"consumers":"Rust scanner", "persisted_formats":"Source data", "external_dependencies":"none",
		"rust_destination":"Rust scanner test", "verification_requirements":"Native read-only scan"});
	let status = xtask::playbook::inspect(&data, &sources)?;
	assert_eq!(status.sources.get("verified_inert"), Some(&1));
	assert!(!status.sources.contains_key("verified_removed"));
	sources["entries"][0]["installed_verification"] = json!("unverified");
	assert!(xtask::playbook::inspect(&data, &sources).is_err());
	sources["entries"][0]["installed_verification"] = json!("verified");
	sources["entries"][0]["path"] = json!("fixtures/executable-worker.ts");
	assert!(xtask::playbook::inspect(&data, &sources).is_err());
	Ok(())
}

#[test]
fn text_wrapped_scripts_and_reviewed_embedded_bundles_cannot_be_inert_assets() -> Result<()> {
	let root = tempdir()?;
	let fs = JailedFs::open(root.path())?;
	let names = [
		"assets/background.js.txt",
		"assets/nested.py.txt.in",
		"packages/stats/src/embedded-client.generated.txt",
		"packages/coding-agent/src/eval/js/shared/prelude.txt",
		"packages/coding-agent/src/tools/browser/aria/aria-snapshot.bundle.txt",
		"packages/coding-agent/src/tools/puppeteer/00_stealth_tampering.txt",
		"assets/options.html.txt",
		"assets/LICENSE.txt",
		"assets/tips.txt",
	];
	let mut paths = Vec::new();
	for name in names {
		fs.atomic_write(
			Path::new(name),
			if name.contains("html") {
				b"<script src='background.js'></script>"
			} else {
				b"fixture payload"
			},
		)?;
		paths.extend_from_slice(name.as_bytes());
		paths.push(0);
	}
	let inventory = xtask::source_audit::inspect_files(&fs, &paths, Path::new("report.json"))?;
	assert_eq!(inventory.non_rust_executable_files.len(), 6);
	assert!(
		inventory
			.non_rust_executable_files
			.iter()
			.all(|path| !path.ends_with("LICENSE.txt") && !path.ends_with("tips.txt"))
	);
	assert_eq!(inventory.script_asset_candidates, ["assets/options.html.txt"]);
	assert_eq!(inventory.files_by_language["javascript"], 5);
	assert_eq!(inventory.files_by_language["python"], 1);
	let baseline: xtask::migration::Baseline = serde_json::from_value(
		json!({"version":1,"commit":"c".repeat(40),"paths":["engine/legacy.ts"]}),
	)?;
	let sources: xtask::migration::Ledger = serde_json::from_value(ledger())?;
	let error = xtask::migration::inspect(&baseline, &sources, &[
		"engine/legacy.ts".into(),
		"assets/new.js.txt".into(),
	])
	.unwrap_err();
	assert_eq!(error.to_string(), "missing migration entry: assets/new.js.txt");
	Ok(())
}

fn source_audit_git(root: &Path, arguments: &[&str]) -> Result<()> {
	// Use the existing bounded-duration check runner; fixture Git never needs
	// network, hooks, signing, a terminal, or an in-memory output capture.
	let mut command: Vec<String> = [
		"git",
		"-c",
		"user.name=source-audit-fixture",
		"-c",
		"user.email=fixture@example.invalid",
		"-c",
		"commit.gpgsign=false",
		"-c",
		"core.hooksPath=nonexistent-fixture-hooks",
		"-c",
		"core.fsmonitor=false",
	]
	.into_iter()
	.map(str::to_owned)
	.collect();
	command.extend(arguments.iter().map(|argument| (*argument).to_owned()));
	let result = xtask::verification::run_check(root, &xtask::verification::Check {
		id: "source-audit-fixture-git".into(),
		command,
		timeout_seconds: 10,
		kind: "contracts".into(),
	})?;
	assert!(
		result.passed,
		"fixture Git failed: timeout={} exit={:?} error={:?}",
		result.timed_out, result.exit_code, result.error
	);
	Ok(())
}

fn source_audit_fixture() -> Result<TempDir> {
	let root = tempdir()?;
	source_audit_git(root.path(), &["init", "--quiet"])?;
	let fs = JailedFs::open(root.path())?;
	fs.atomic_write(Path::new("engine.ts"), b"export const engine = 1;\n")?;
	fs.atomic_write(Path::new("docs/rust-migration/capabilities.json"), &serde_json::to_vec(&json!({
		"version": 2,
		"capabilities": [{ "id": "fixture", "implementation": { "status": "absent" }, "verification": { "status": "unverified" } }]
	}))?)?;
	source_audit_git(root.path(), &["add", "engine.ts", "docs/rust-migration/capabilities.json"])?;
	source_audit_git(root.path(), &["commit", "--quiet", "-m", "Capture fixture source"])?;
	Ok(root)
}

fn run_source_audit(root: &Path) -> Result<()> {
	xtask::source_audit::execute(vec![
		"audit".into(),
		"--root".into(),
		root.to_str().unwrap().into(),
		"--output".into(),
		"report.json".into(),
	])
}

#[test]
fn nested_registered_worktree_tokens_are_excluded_without_hiding_claude_or_sibling_sources()
-> Result<()> {
	let root = source_audit_fixture()?;
	// NUL worktree records must preserve spaces and, where supported, newlines.
	let name = if cfg!(unix) {
		".claude/worktrees/agent space\nline"
	} else {
		".claude/worktrees/agent space"
	};
	source_audit_git(root.path(), &["worktree", "add", "--quiet", "--detach", name, "HEAD"])?;
	let fs = JailedFs::open(root.path())?;
	fs.atomic_write(
		&Path::new(name).join("separate-checkout-only.ts"),
		b"export const separate = true;\n",
	)?;
	fs.atomic_write(Path::new(".claude/local.ts"), b"export const owned = true;\n")?;
	let sibling = format!("{name}-sibling/worker.py");
	fs.atomic_write(Path::new(&sibling), b"print('owned sibling source')\n")?;
	run_source_audit(root.path())?;
	let report: Value = serde_json::from_slice(&fs.read(Path::new("report.json"), 1024 * 1024)?)?;
	assert_eq!(report["excluded_worktrees"], json!([name]));
	assert_eq!(
		report["non_rust_executable_files"],
		json!([".claude/local.ts", sibling, "engine.ts"])
	);
	Ok(())
}

#[test]
fn ordinary_nested_sources_stay_in_inventory_and_an_unregistered_repository_fails_with_its_path()
-> Result<()> {
	let root = source_audit_fixture()?;
	let fs = JailedFs::open(root.path())?;
	let nested = ".claude/worktrees/unregistered";
	fs.atomic_write(&Path::new(nested).join("worker.ts"), b"export const owned = true;\n")?;
	run_source_audit(root.path())?;
	let report: Value = serde_json::from_slice(&fs.read(Path::new("report.json"), 1024 * 1024)?)?;
	assert_eq!(
		report["non_rust_executable_files"],
		json!([format!("{nested}/worker.ts"), "engine.ts"])
	);
	assert_eq!(report["excluded_worktrees"], json!([]));
	source_audit_git(&root.path().join(nested), &["init", "--quiet"])?;
	let error = format!("{:#}", run_source_audit(root.path()).unwrap_err());
	assert!(error.contains(nested), "{error}");
	assert!(error.contains("expected a regular file"), "{error}");
	Ok(())
}

#[test]
fn stale_worktree_registration_cannot_hide_a_replacement_repository() -> Result<()> {
	let root = source_audit_fixture()?;
	let nested = ".claude/worktrees/replaced";
	source_audit_git(root.path(), &["worktree", "add", "--quiet", "--detach", nested, "HEAD"])?;
	std::fs::remove_file(root.path().join(nested).join(".git"))?;
	source_audit_git(&root.path().join(nested), &["init", "--quiet"])?;
	let error = format!("{:#}", run_source_audit(root.path()).unwrap_err());
	assert!(error.contains("registration does not match checkout"), "{error}");
	assert!(error.contains(nested), "{error}");
	Ok(())
}
