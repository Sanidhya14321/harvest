use std::{path::Path, process::Command};

use anyhow::Result;
use harvest_runtime::JailedFs;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use xtask::verification::{Check, Scope, inspect_graph, plan};

fn manifest() -> Value {
	let checks = ["engine", "ui", "other"].map(
		|id| json!({"id":id,"command":["cargo","--version"],"timeout_seconds":30,"kind":"contracts"}),
	);
	let units = [
		("engine", vec!["engine"], vec![]),
		("ui", vec!["ui"], vec!["engine"]),
		("other", vec!["other"], vec![]),
	];
	json!({"version":4,"capabilities":[{"id":"session","work_unit":"engine","implementation":{"status":"partial","paths":["engine/lib.rs"]},"verification":{"status":"unverified"},"lifecycle":{"candidate":{"status":"pending"},"legacy":{"status":"pending"},"promotion":{"status":"pending"}}}],"execution":{
		"units": units.into_iter().map(|(id,paths,deps)| json!({"id":id,"paths":paths,"depends_on":deps,"checks":[id],"review":"reviewed","deletion_unit":id,"fixtures":[],"targets":["host"]})).collect::<Vec<_>>(),
		"checks": checks,"release_gates":["contracts"]
	}})
}

#[test]
fn changed_engines_verify_consumers_but_ui_edits_do_not_retest_unrelated_consumers() -> Result<()> {
	let graph = inspect_graph(&manifest())?;
	let engine = plan(&graph, Scope::Affected, &["engine/src/lib.rs".into()])?;
	assert_eq!(engine.units, ["engine", "ui"]);
	assert_eq!(
		engine
			.checks
			.iter()
			.map(|check| check.id.as_str())
			.collect::<Vec<_>>(),
		["engine", "ui"]
	);
	let ui = plan(&graph, Scope::Affected, &["ui/view.rs".into()])?;
	assert_eq!(ui.units, ["engine", "ui"]);
	let other = plan(&graph, Scope::Affected, &["other/test.rs".into()])?;
	assert_eq!(other.units, ["other"]);
	Ok(())
}

#[test]
fn unknown_shared_or_unreviewed_changes_widen_to_full_and_cycles_fail() -> Result<()> {
	let mut data = manifest();
	for path in ["unknown/file.rs", "Cargo.lock", "engineer/file.rs"] {
		assert_eq!(
			plan(&inspect_graph(&data)?, Scope::Affected, &[path.into()])?.effective_scope,
			Scope::Full
		);
	}
	data["execution"]["units"][0]["review"] = json!("pending");
	assert_eq!(
		plan(&inspect_graph(&data)?, Scope::Affected, &["engine/file.rs".into()])?.effective_scope,
		Scope::Full
	);
	data["execution"]["units"][0]["depends_on"] = json!(["ui"]);
	assert!(
		inspect_graph(&data)
			.unwrap_err()
			.to_string()
			.contains("cyclic")
	);
	Ok(())
}

fn fixture() -> Result<(TempDir, String)> {
	let root = tempdir()?;
	assert!(
		Command::new("git")
			.args(["init", "--quiet"])
			.arg(root.path())
			.status()?
			.success()
	);
	let fs = JailedFs::open(root.path())?;
	fs.atomic_write(Path::new(".gitignore"), b"/target/\n")?;
	fs.atomic_write(Path::new("rust-toolchain.toml"), b"[toolchain]\nchannel = 'fixture'\n")?;
	fs.atomic_write(Path::new("engine/lib.rs"), b"pub fn baseline() {}\n")?;
	fs.atomic_write(
		Path::new("target/probe.rs"),
		br#"fn main() {
 let args: Vec<String> = std::env::args().collect();
 match args.get(1).map(String::as_str) {
  Some("fail") => std::process::exit(7),
  Some("mutate") => std::fs::write("engine/lib.rs", "changed during verification").unwrap(),
  Some("wait") => std::thread::sleep(std::time::Duration::from_secs(60)),
  _ => ()
 }
}"#,
	)?;
	let binary = root
		.path()
		.join(format!("target/probe{}", std::env::consts::EXE_SUFFIX));
	let status = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
		.arg(root.path().join("target/probe.rs"))
		.arg("-o")
		.arg(&binary)
		.status()?;
	assert!(status.success());
	Ok((root, binary.to_str().unwrap().to_owned()))
}

#[test]
fn receipts_record_failures_and_detect_sources_changed_by_a_check() -> Result<()> {
	let (root, binary) = fixture()?;
	let fs = JailedFs::open(root.path())?;
	let mut data = manifest();
	for mode in ["fail", "mutate"] {
		data["execution"]["checks"][0]["command"] = json!([binary, mode]);
		fs.atomic_write(
			Path::new("docs/rust-migration/capabilities.json"),
			&serde_json::to_vec(&data)?,
		)?;
		let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
			.args(["source-audit", "verify", "--scope", "full", "--root"])
			.arg(root.path())
			.output()?;
		assert!(!output.status.success(), "{mode} incorrectly passed");
		let receipt: Value = serde_json::from_slice(
			&fs.read(Path::new("target/migration/verification.json"), 1024 * 1024)?,
		)?;
		assert_eq!(receipt["passed"], false);
		assert_eq!(receipt["certifies_product_parity"], false);
		if mode == "fail" {
			assert_eq!(receipt["results"][0]["exit_code"], 7);
			assert_eq!(receipt["results"].as_array().unwrap().len(), 1);
		}
		fs.atomic_write(Path::new("engine/lib.rs"), b"pub fn baseline() {}\n")?;
	}
	Ok(())
}

#[test]
fn missing_programs_and_expired_deadlines_cannot_produce_success_receipts() -> Result<()> {
	let (root, binary) = fixture()?;
	let check = Check {
		id:              "deadline".into(),
		command:         vec![binary, "wait".into()],
		timeout_seconds: 1,
		kind:            "contracts".into(),
	};
	let expired = xtask::verification::run_check(root.path(), &check)?;
	assert!(expired.timed_out);
	assert!(!expired.passed);
	let missing =
		Check { command: vec![root.path().join("does-not-exist").display().to_string()], ..check };
	let unavailable = xtask::verification::run_check(root.path(), &missing)?;
	assert!(!unavailable.passed);
	assert!(unavailable.error.is_some());
	Ok(())
}
