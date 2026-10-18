use std::{fs, path::Path, process::Command};

use anyhow::Result;
use harvest_runtime::{JailedFs, content_hash};
use serde_json::json;
use tempfile::tempdir;
use xtask::migration::{Role, inspect_files};

#[test]
fn inventory_tracks_active_sources_deletions_and_allowed_exceptions_without_self_hashing()
-> Result<()> {
	let root = tempdir()?;
	let jailed = JailedFs::open(root.path())?;
	let files = [
		("src/core.ts", "export const value = 1;\n"),
		("src/model.generated.ts", "export const generatedNameOnly = 2;\n"),
		("src/cli/gallery-fixtures/demo.ts", "render();\n"),
		("tests/fixtures/input.ts", "invalid syntax fixture\n"),
		(
			"packages/coding-agent/test/fixtures/security/seeded-repository/src/safe-lookalike.ts",
			"scanner source input\n",
		),
		("vendor/library.js", "external();\n"),
		("packages/tui/src/fuzzy.js", "generatedBinding();\n"),
		("src/lib.rs", "pub fn run() {}\n"),
		("source-audit.json", "previous report\n"),
	];
	for (path, content) in files {
		jailed.atomic_write(Path::new(path), content.as_bytes())?;
	}
	let manifest = files
		.iter()
		.map(|(path, _)| *path)
		.chain(["deleted.ts", "src/core.ts"])
		.flat_map(|path| path.bytes().chain([0]))
		.collect::<Vec<_>>();
	let inventory = inspect_files(&jailed, &manifest, Path::new("source-audit.json"))?;
	assert_eq!(inventory.authored_js_ts_files, 4);
	assert_eq!(inventory.authored_js_ts_lines, 4);
	assert_eq!(inventory.files_read, 8);
	assert_eq!(inventory.absent_tracked_paths, ["deleted.ts"]);
	let core = inventory
		.files
		.iter()
		.find(|file| file.path == "src/core.ts")
		.unwrap();
	assert_eq!(core.sha256, content_hash(files[0].1.as_bytes()));
	let harness = inventory
		.files
		.iter()
		.find(|file| file.path == "tests/fixtures/input.ts")
		.unwrap();
	assert_eq!(harness.role, Role::Test);
	let fixture = inventory
		.files
		.iter()
		.find(|file| file.path.ends_with("safe-lookalike.ts"))
		.unwrap();
	assert_eq!(fixture.role, Role::LanguageFixture);
	// Report rewrites must not change the source fingerprint. Real source edits
	// must.
	jailed.atomic_write(Path::new("source-audit.json"), b"new report")?;
	assert_eq!(
		inspect_files(&jailed, &manifest, Path::new("source-audit.json"))?.source_fingerprint,
		inventory.source_fingerprint
	);
	jailed.atomic_write(Path::new("src/core.ts"), b"export const value = 3;\n")?;
	assert_ne!(
		inspect_files(&jailed, &manifest, Path::new("source-audit.json"))?.source_fingerprint,
		inventory.source_fingerprint
	);
	Ok(())
}

#[test]
fn public_cutover_rejects_untracked_authored_code_even_with_complete_capability_evidence()
-> Result<()> {
	let root = tempdir()?;
	let initialized = Command::new("git")
		.args(["init", "--quiet"])
		.arg(root.path())
		.status()?;
	assert!(initialized.success());
	let jailed = JailedFs::open(root.path())?;
	jailed.atomic_write(Path::new("docs/rust-migration/capabilities.json"), &serde_json::to_vec(&json!({
		"version": 3,
		"capabilities": [{"id":"fixture-session","scope":"retained","implementation":{"status":"complete","paths":["src/lib.rs"]},"verification":{"status":"verified","contract_suites":["session-contract"]},"installation":"default-rust","legacy":{"removable":true}}]
	}))?)?;
	jailed.atomic_write(Path::new("src/lib.rs"), b"pub fn run() {}\n")?;
	let status = || {
		Command::new(env!("CARGO_BIN_EXE_xtask"))
			.args(["migration", "status", "--root"])
			.arg(root.path())
			.arg("--require-complete")
			.output()
	};
	let complete = status()?;
	assert!(complete.status.success(), "{}", String::from_utf8_lossy(&complete.stderr));
	// A generated-looking filename is not proof that executable code is generated.
	jailed.atomic_write(Path::new("src/leftover.generated.ts"), b"legacy();\n")?;
	let blocked = status()?;
	assert!(!blocked.status.success());
	assert!(String::from_utf8_lossy(&blocked.stderr).contains("authored JS/TS remains"));
	fs::remove_file(root.path().join("src/leftover.generated.ts"))?;
	jailed.atomic_write(Path::new("web/index.html"), b"<body onload='legacy()'>\n")?;
	let inline_script = status()?;
	assert!(!inline_script.status.success());
	assert!(
		String::from_utf8_lossy(&inline_script.stderr).contains("HTML/SVG script assets remain")
	);
	fs::remove_file(root.path().join("web/index.html"))?;
	let mut manifest: serde_json::Value = serde_json::from_slice(
		&jailed.read(Path::new("docs/rust-migration/capabilities.json"), 4096)?,
	)?;
	manifest["capabilities"][0]["verification"]["status"] = json!("unverified");
	jailed.atomic_write(
		Path::new("docs/rust-migration/capabilities.json"),
		&serde_json::to_vec(&manifest)?,
	)?;
	let missing_parity = status()?;
	assert!(!missing_parity.status.success());
	assert!(
		String::from_utf8_lossy(&missing_parity.stderr).contains("distribution cutover blocked")
	);
	Ok(())
}

#[cfg(unix)]
#[test]
fn inventory_preserves_literal_filename_backslashes_and_rejects_escaping_symlinks() -> Result<()> {
	let root = tempdir()?;
	let outside = tempdir()?;
	fs::write(root.path().join("literal\\name.ts"), b"legacy();\n")?;
	let jailed = JailedFs::open(root.path())?;
	let inventory = inspect_files(&jailed, b"literal\\name.ts\0", Path::new("report.json"))?;
	assert_eq!(inventory.files[0].path, "literal\\name.ts");
	fs::write(outside.path().join("secret.ts"), b"private")?;
	std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))?;
	assert!(inspect_files(&jailed, b"escape/secret.ts\0", Path::new("report.json")).is_err());
	assert!(inspect_files(&jailed, b"../secret.ts\0", Path::new("report.json")).is_err());
	assert!(inspect_files(&jailed, b"unterminated.ts", Path::new("report.json")).is_err());
	Ok(())
}
