//! The installed orchestrator selects executable generators and propagates
//! failures.
use std::{fs, path::PathBuf, process::Command, sync::OnceLock};

fn fixture() -> PathBuf {
	static BUILD: OnceLock<tempfile::TempDir> = OnceLock::new();
	let root = BUILD.get_or_init(|| {
		let root = tempfile::tempdir().unwrap();
		let output = root
			.path()
			.join(format!("generator{}", std::env::consts::EXE_SUFFIX));
		let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
		let result = Command::new(compiler)
			.arg("--edition=2024")
			.arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gen_nix_generator.rs"))
			.arg("-o")
			.arg(&output)
			.output()
			.unwrap();
		assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
		root
	});
	root
		.path()
		.join(format!("generator{}", std::env::consts::EXE_SUFFIX))
}

#[test]
fn installed_generator_selection_and_output_work_without_language_runtimes() {
	for (available, expected_prefix) in [
		(vec!["bun2nix", "nix", "bun"], Vec::<&str>::new()),
		(vec!["nix", "bun"], vec![
			"--extra-experimental-features",
			"nix-command flakes",
			"--accept-flake-config",
			"develop",
			"--command",
			"bun2nix",
		]),
		(vec!["bun"], vec!["x", "bun2nix@2.1.2"]),
	] {
		let root = tempfile::tempdir().unwrap();
		let bin = root.path().join("bin");
		fs::create_dir(&bin).unwrap();
		fs::create_dir(root.path().join("nix")).unwrap();
		for executable in available {
			fs::copy(fixture(), bin.join(format!("{executable}{}", std::env::consts::EXE_SUFFIX)))
				.unwrap();
		}
		fs::write(root.path().join("bun.lock"), "{\n  \"lockfileVersion\": 2,\n}\n").unwrap();
		let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
			.arg("gen-nix")
			.current_dir(root.path())
			.env("PATH", &bin)
			.output()
			.unwrap();
		assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
		let mut args = expected_prefix;
		args.extend(["-l", "bun.lock", "-c", "../", "-o", "nix/bun.nix"]);
		let expected = format!("{}\n{{\n  \"lockfileVersion\": 1,\n}}\n", args.join("\n"));
		assert_eq!(fs::read_to_string(root.path().join("invocation.txt")).unwrap(), expected);
		assert_eq!(fs::read(root.path().join("nix/bun.nix")).unwrap(), b"{ generated = true; }\n");
	}
}

#[test]
fn generator_failure_does_not_fallback_or_rewrite_existing_output_and_future_locks_fail_before_launch()
 {
	let root = tempfile::tempdir().unwrap();
	fs::create_dir(root.path().join("bin")).unwrap();
	fs::create_dir(root.path().join("nix")).unwrap();
	fs::copy(
		fixture(),
		root
			.path()
			.join("bin")
			.join(format!("bun2nix{}", std::env::consts::EXE_SUFFIX)),
	)
	.unwrap();
	fs::write(root.path().join("bun.lock"), "{\n  \"lockfileVersion\": 2,\n}\n").unwrap();
	fs::write(root.path().join("nix/bun.nix"), "original\r\n\r\n").unwrap();
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.arg("gen-nix")
		.current_dir(root.path())
		.env("PATH", root.path().join("bin"))
		.env("HARVEST_GENERATOR_FAIL", "1")
		.output()
		.unwrap();
	assert!(!output.status.success());
	assert!(String::from_utf8_lossy(&output.stderr).contains("bun2nix failed"));
	assert_eq!(fs::read(root.path().join("nix/bun.nix")).unwrap(), b"original\r\n\r\n");
	assert!(
		fs::read_to_string(root.path().join("bun.lock"))
			.unwrap()
			.contains(": 1,")
	);
	fs::remove_file(root.path().join("invocation.txt")).unwrap();
	let future = "{\n  \"lockfileVersion\": 3,\n}\n";
	fs::write(root.path().join("bun.lock"), future).unwrap();
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.arg("gen-nix")
		.current_dir(root.path())
		.env("PATH", root.path().join("bin"))
		.output()
		.unwrap();
	assert!(!output.status.success());
	assert!(!root.path().join("invocation.txt").exists());
	assert_eq!(fs::read_to_string(root.path().join("bun.lock")).unwrap(), future);
}

#[cfg(unix)]
#[test]
fn non_executable_path_entries_do_not_shadow_a_working_generator() {
	use std::os::unix::fs::PermissionsExt;
	let root = tempfile::tempdir().unwrap();
	let bin = root.path().join("bin");
	fs::create_dir(&bin).unwrap();
	fs::create_dir(root.path().join("nix")).unwrap();
	fs::write(bin.join("bun2nix"), "not executable").unwrap();
	fs::set_permissions(bin.join("bun2nix"), fs::Permissions::from_mode(0o644)).unwrap();
	fs::copy(fixture(), bin.join("nix")).unwrap();
	fs::write(root.path().join("bun.lock"), "{\n  \"lockfileVersion\": 1,\n}\n").unwrap();
	let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
		.arg("gen-nix")
		.current_dir(root.path())
		.env("PATH", &bin)
		.output()
		.unwrap();
	assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
	assert!(
		fs::read_to_string(root.path().join("invocation.txt"))
			.unwrap()
			.starts_with("--extra-experimental-features\n")
	);
}
