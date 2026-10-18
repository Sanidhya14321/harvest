//! Inspect actual ELF metadata, rather than trusting artifact names.
#![cfg(all(target_os = "linux", target_env = "gnu"))]

use std::{fs, process::Command};

use anyhow::Result;

#[test]
fn gnu_abi_gate_rejects_too_new_requirements_and_non_elf_inputs() -> Result<()> {
	let root = tempfile::tempdir()?;
	fs::copy(env!("CARGO_BIN_EXE_xtask"), root.path().join("binary"))?;
	fs::write(root.path().join("text"), "not an executable")?;
	for (name, ceiling, success) in
		[("binary", "999.0", true), ("binary", "0.0", false), ("text", "999.0", false)]
	{
		let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
			.current_dir(root.path())
			.args(["package", "verify-glibc", "--binary", name, "--max-glibc", ceiling])
			.output()?;
		assert_eq!(output.status.success(), success, "{}", String::from_utf8_lossy(&output.stderr));
		if success {
			let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
			assert!(
				report["maxRequired"]
					.as_str()
					.is_some_and(|value| value.starts_with("2."))
			);
		} else if name == "binary" {
			assert!(String::from_utf8_lossy(&output.stderr).contains("above portability ceiling"));
		}
	}
	Ok(())
}
