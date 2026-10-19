use std::{
	env, fs,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
	pub(crate) home:       PathBuf,
	pub(crate) data:       PathBuf,
	pub(crate) state:      PathBuf,
	pub(crate) executable: PathBuf,
}

pub(crate) fn install(shell: &str, layout: &Layout, script: &str) -> Result<PathBuf> {
	let script_path = script_path(shell, layout)?;
	write_file(&script_path, script.as_bytes())?;
	place_shortcut(&layout.executable)?;
	write_file(&layout.state.join("shortcut-installed"), b"installed\n")?;
	write_file(&layout.state.join(format!("{shell}-installed")), b"installed\n")?;
	Ok(script_path)
}

pub(crate) fn detected_shell() -> Option<String> {
	env::var_os("SHELL").and_then(|value| {
		Path::new(&value)
			.file_stem()
			.and_then(|stem| stem.to_str())
			.map(str::to_ascii_lowercase)
	})
}

fn script_path(shell: &str, layout: &Layout) -> Result<PathBuf> {
	let path = match shell {
		"bash" => layout.data.join("bash-completion/completions/omp"),
		"zsh" => layout.home.join(".zfunc/_omp"),
		"fish" => layout.home.join(".config/fish/completions/omp.fish"),
		"powershell" | "pwsh" => layout.home.join("omp-completions.ps1"),
		"elvish" => layout.home.join(".config/elvish/lib/omp.elv"),
		_ => bail!("unsupported shell {shell}"),
	};
	Ok(path)
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
	if let Some(parent) = path.parent() {
		fs::create_dir_all(parent)
			.with_context(|| format!("failed to create {}", parent.display()))?;
	}
	fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

fn place_shortcut(executable: &Path) -> Result<()> {
	let parent = executable
		.parent()
		.context("the omp executable has no parent directory")?;
	// ZIP/binstall installations carry both real binaries. Do not create a
	// wrapper over the already installed executable or over this invocation.
	#[cfg(windows)]
	{
		let native_alias = parent.join("harvest.exe");
		if same_binary(&native_alias, executable) {
			return Ok(());
		}
		if native_alias.symlink_metadata().is_ok() {
			bail!("refusing to replace {}", native_alias.display());
		}
	}
	let shortcut = parent.join(shortcut_name());
	if same_shortcut(&shortcut, executable) {
		return Ok(());
	}
	if shortcut.symlink_metadata().is_ok() {
		bail!("refusing to replace {}", shortcut.display());
	}
	link(executable, &shortcut)
}

fn same_shortcut(shortcut: &Path, executable: &Path) -> bool {
	if shortcut.symlink_metadata().is_err() {
		return false;
	}
	#[cfg(unix)]
	{
		same_binary(shortcut, executable)
	}
	#[cfg(windows)]
	{
		fs::read_to_string(shortcut).unwrap_or_default() == shortcut_body(executable)
	}
}

fn same_binary(alias: &Path, executable: &Path) -> bool {
	if alias.symlink_metadata().is_err() {
		return false;
	}
	if let (Ok(alias), Ok(executable)) = (fs::canonicalize(alias), fs::canonicalize(executable))
		&& alias == executable
		&& executable.is_file()
	{
		return true;
	}

	let Some(parent) = executable.parent() else {
		return false;
	};
	let Ok(fs) = harvest_runtime::JailedFs::open(parent) else {
		return false;
	};
	let Some(source) = executable.file_name() else {
		return false;
	};
	let Some(alias) = alias.file_name() else {
		return false;
	};
	const BINARY_LIMIT: u64 = 256 * 1024 * 1024;
	match (
		fs.hash_file(Path::new(source), BINARY_LIMIT),
		fs.hash_file(Path::new(alias), BINARY_LIMIT),
	) {
		(Ok(left), Ok(right)) => left == right,
		_ => false,
	}
}

const fn shortcut_name() -> &'static str {
	if cfg!(windows) {
		"harvest.cmd"
	} else {
		"harvest"
	}
}

#[cfg(unix)]
fn link(executable: &Path, shortcut: &Path) -> Result<()> {
	std::os::unix::fs::symlink(executable, shortcut)
		.with_context(|| format!("failed to link {} to {}", shortcut.display(), executable.display()))
}

#[cfg(windows)]
fn link(executable: &Path, shortcut: &Path) -> Result<()> {
	fs::write(shortcut, shortcut_body(executable))
		.with_context(|| format!("failed to write {}", shortcut.display()))
}

#[cfg(windows)]
fn shortcut_body(executable: &Path) -> String {
	format!("@echo off\r\n\"{}\" %*\r\n", executable.display())
}

#[cfg(test)]
mod tests {
	use std::fs;

	use super::{Layout, install};

	#[test]
	fn bash_install_writes_the_completion_state_and_shortcut() {
		let root = tempfile::tempdir().unwrap();
		let executable = root.path().join("bin/omp");
		fs::create_dir_all(executable.parent().unwrap()).unwrap();
		fs::write(&executable, "bin").unwrap();
		let layout = Layout {
			home:       root.path().join("home"),
			data:       root.path().join("data"),
			state:      root.path().join("state"),
			executable: executable.clone(),
		};
		let written = install("bash", &layout, "complete omp\n").unwrap();
		assert_eq!(written, root.path().join("data/bash-completion/completions/omp"));
		assert_eq!(fs::read_to_string(&written).unwrap(), "complete omp\n");
		assert_eq!(
			fs::read_to_string(root.path().join("state/bash-installed")).unwrap(),
			"installed\n"
		);
		let length = fs::metadata(root.path().join("state/shortcut-installed"))
			.unwrap()
			.len();
		assert!(length > 0, "shortcut state should not be empty");
		drop(install("bash", &layout, "complete omp\n").unwrap());
		#[cfg(unix)]
		{
			assert_eq!(fs::read_link(root.path().join("bin/harvest")).unwrap(), executable);
		}
		#[cfg(windows)]
		{
			assert!(
				fs::read_to_string(root.path().join("bin/harvest.cmd"))
					.unwrap()
					.contains("omp")
			);
		}
	}

	#[test]
	fn shell_marker_stays_absent_when_the_shortcut_cannot_be_replaced() {
		let root = tempfile::tempdir().unwrap();
		let executable = root.path().join("bin/omp");
		fs::create_dir_all(executable.parent().unwrap()).unwrap();
		fs::write(&executable, "bin").unwrap();
		let blocked = if cfg!(windows) {
			root.path().join("bin/harvest.cmd")
		} else {
			root.path().join("bin/harvest")
		};
		fs::write(&blocked, "other").unwrap();
		let layout = Layout {
			home: root.path().join("home"),
			data: root.path().join("data"),
			state: root.path().join("state"),
			executable,
		};
		let error = install("bash", &layout, "complete omp\n").unwrap_err();
		assert!(error.to_string().contains("refusing"), "{error}");
		drop(fs::metadata(root.path().join("state/bash-installed")).unwrap_err());
	}

	#[test]
	fn unknown_shell_is_rejected() {
		let root = tempfile::tempdir().unwrap();
		let layout = Layout {
			home:       root.path().join("home"),
			data:       root.path().join("data"),
			state:      root.path().join("state"),
			executable: root.path().join("omp"),
		};
		let error = install("tcsh", &layout, "script").unwrap_err();
		assert!(error.to_string().contains("unsupported"), "{error}");
	}

	#[test]
	fn identical_installed_aliases_accept_completions_from_either_entrypoint() {
		let root = tempfile::tempdir().unwrap();
		let bin = root.path().join("bin");
		fs::create_dir(&bin).unwrap();
		let omp = bin.join(if cfg!(windows) { "omp.exe" } else { "omp" });
		let harvest = bin.join(if cfg!(windows) {
			"harvest.exe"
		} else {
			"harvest"
		});
		fs::write(&omp, b"same installed executable").unwrap();
		fs::copy(&omp, &harvest).unwrap();
		for executable in [omp, harvest.clone()] {
			let layout = Layout {
				home: root.path().join("home"),
				data: root.path().join("data"),
				state: root.path().join("state"),
				executable,
			};
			let path = install("bash", &layout, "completions").unwrap();
			assert_eq!(fs::read(path).unwrap(), b"completions");
			assert_eq!(fs::read(&harvest).unwrap(), b"same installed executable");
		}
	}

	#[cfg(unix)]
	#[test]
	fn relative_package_manager_alias_is_not_replaced_or_rejected() {
		let root = tempfile::tempdir().unwrap();
		let bin = root.path().join("bin");
		fs::create_dir(&bin).unwrap();
		let executable = bin.join("omp");
		fs::write(&executable, "installed").unwrap();
		std::os::unix::fs::symlink("omp", bin.join("harvest")).unwrap();
		let layout = Layout {
			home: root.path().join("home"),
			data: root.path().join("data"),
			state: root.path().join("state"),
			executable,
		};
		assert_eq!(fs::read(install("bash", &layout, "complete").unwrap()).unwrap(), b"complete");
		assert_eq!(fs::read_link(bin.join("harvest")).unwrap(), std::path::Path::new("omp"));
	}
}
