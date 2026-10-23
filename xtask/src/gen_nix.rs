//! Generate nix/bun.nix from bun.lock using the pinned bun2nix input.
use std::{env, fs, process::Command};

pub fn normalize_lockfile_version(contents: &str) -> anyhow::Result<String> {
	let re = regex::Regex::new(r#"(?m)^(\s*"lockfileVersion":\s*)(\d+)(,)"#)?;
	let caps = re
		.captures(contents)
		.ok_or_else(|| anyhow::anyhow!("bun.lock is missing a lockfileVersion stamp"))?;
	let version: u32 = caps[2].parse()?;
	if version <= 1 {
		return Ok(contents.to_owned());
	}
	if version > 2 {
		anyhow::bail!(
			"bun.lock is lockfileVersion {version}, which changes content (scoped overrides) and \
			 cannot be downgraded for bun2nix"
		);
	}
	let start = caps.get(0).unwrap().start();
	let end = caps.get(0).unwrap().end();
	let prefix = &contents[..start];
	let stamp1 = &caps[1];
	let stamp3 = &caps[3];
	let suffix = &contents[end..];
	Ok(format!("{prefix}{stamp1}1{stamp3}{suffix}"))
}

pub fn normalize_generated_nix(contents: &str) -> String {
	format!("{}\n", contents.trim_end_matches(['\r', '\n']))
}

fn executable(path: &std::path::Path) -> bool {
	let Ok(metadata) = path.metadata() else {
		return false;
	};
	if !metadata.is_file() {
		return false;
	}
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		metadata.permissions().mode() & 0o111 != 0
	}
	#[cfg(not(unix))]
	{
		true
	}
}

fn find_executable(command: &str) -> Option<String> {
	let path_var = env::var_os("PATH")?;
	for dir in env::split_paths(&path_var) {
		let candidate = dir.join(command);
		if executable(&candidate) {
			return Some(candidate.to_string_lossy().into_owned());
		}
		#[cfg(windows)]
		{
			let candidate = dir.join(format!("{command}.exe"));
			if executable(&candidate) {
				return Some(candidate.to_string_lossy().into_owned());
			}
		}
	}
	None
}

pub enum Generator {
	Bun2Nix(String),
	Nix(String),
	Bunx,
}

pub fn resolve_generator() -> Generator {
	if let Some(exe) = find_executable("bun2nix") {
		return Generator::Bun2Nix(exe);
	}
	if let Some(exe) = find_executable("nix") {
		return Generator::Nix(exe);
	}
	Generator::Bunx
}

pub fn execute() -> anyhow::Result<()> {
	let root = env::current_dir()?;
	let lock_path = root.join("bun.lock");
	let contents = fs::read_to_string(&lock_path)?;
	let normalized = normalize_lockfile_version(&contents)?;
	if normalized != contents {
		fs::write(&lock_path, &normalized)?;
	}

	let generator = resolve_generator();
	match generator {
		Generator::Bun2Nix(exe) => {
			Command::new(&exe)
				.args(["-l", "bun.lock", "-c", "../", "-o", "nix/bun.nix"])
				.current_dir(&root)
				.status()?
				.success()
				.then_some(())
				.ok_or_else(|| anyhow::anyhow!("bun2nix failed"))?;
		},
		Generator::Nix(exe) => {
			Command::new(&exe)
				.args([
					"--extra-experimental-features",
					"nix-command flakes",
					"--accept-flake-config",
					"develop",
					"--command",
					"bun2nix",
					"-l",
					"bun.lock",
					"-c",
					"../",
					"-o",
					"nix/bun.nix",
				])
				.current_dir(&root)
				.status()?
				.success()
				.then_some(())
				.ok_or_else(|| anyhow::anyhow!("nix bun2nix failed"))?;
		},
		Generator::Bunx => {
			Command::new("bun")
				.args(["x", "bun2nix@2.1.2", "-l", "bun.lock", "-c", "../", "-o", "nix/bun.nix"])
				.current_dir(&root)
				.status()?
				.success()
				.then_some(())
				.ok_or_else(|| anyhow::anyhow!("bunx bun2nix failed"))?;
		},
	}

	let output_path = root.join("nix/bun.nix");
	let contents = fs::read_to_string(&output_path)?;
	let normalized = normalize_generated_nix(&contents);
	if normalized != contents {
		fs::write(&output_path, &normalized)?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{normalize_generated_nix, normalize_lockfile_version};

	#[test]
	fn lock_restamping_preserves_every_other_byte_and_rejects_unsupported_or_missing_stamps() {
		let version_two = "{\r\n  \"lockfileVersion\": 2,\r\n  \"workspaces\": {},\r\n}\r\n";
		assert_eq!(
			normalize_lockfile_version(version_two).unwrap(),
			version_two.replacen(": 2,", ": 1,", 1)
		);
		let version_one = version_two.replacen(": 2,", ": 1,", 1);
		assert_eq!(normalize_lockfile_version(&version_one).unwrap(), version_one);
		for invalid in ["{}\n", "prefix \"lockfileVersion\": 2,\n", "{\n \"lockfileVersion\": 3,\n}"]
		{
			assert!(normalize_lockfile_version(invalid).is_err());
		}
	}

	#[test]
	fn generated_output_canonicalizes_only_trailing_line_endings() {
		for input in ["}", "}\n", "}\r\n\r\n"] {
			assert_eq!(normalize_generated_nix(input), "}\n");
		}
		assert_eq!(normalize_generated_nix("}\r\n "), "}\r\n \n");
	}
}
