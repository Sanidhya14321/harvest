//! Read and update the inherited workspace release version without rewriting
//! TOML.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::{JailedFs, MAX_FILE_BYTES};
use toml_edit::{DocumentMut, Item, Value};

#[derive(Parser)]
struct Arguments {
	#[arg(long, default_value = ".")]
	workspace: PathBuf,
	#[arg(long)]
	set:       Option<String>,
	#[arg(long, requires = "set")]
	expect:    Option<String>,
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let arguments = Arguments::try_parse_from(
		std::iter::once("cargo xtask version".to_owned()).chain(arguments),
	)?;
	println!(
		"{}",
		update(&arguments.workspace, arguments.set.as_deref(), arguments.expect.as_deref())?
	);
	Ok(())
}

fn update(root: &Path, next: Option<&str>, expected: Option<&str>) -> Result<String> {
	if let Some(next) = next {
		crate::packaging::validate_version(next)?;
	}
	let fs = JailedFs::open(root)?;
	let _lock = next
		.map(|_| fs.lock(Path::new(".cargo/harvest-version.lock")))
		.transpose()?;
	let bytes = fs.read(Path::new("Cargo.toml"), MAX_FILE_BYTES)?;
	let mut document = std::str::from_utf8(&bytes)?.parse::<DocumentMut>()?;
	let version = document
		.get_mut("workspace")
		.and_then(|item| item.get_mut("package"))
		.and_then(|item| item.get_mut("version"))
		.and_then(Item::as_value_mut)
		.context("Cargo.toml must declare workspace.package.version")?;
	let current = version
		.as_str()
		.context("workspace version must be a string")?
		.to_owned();
	crate::packaging::validate_version(&current)?;
	if expected.is_some_and(|expected| expected != current) {
		bail!(
			"workspace version changed: expected {}, found {current}",
			expected.unwrap_or_default()
		);
	}
	if let Some(next) = next {
		let decor = version.decor().clone();
		*version = Value::from(next);
		*version.decor_mut() = decor;
		fs.atomic_write(Path::new("Cargo.toml"), document.to_string().as_bytes())?;
		return Ok(next.to_owned());
	}
	Ok(current)
}

#[cfg(test)]
mod tests {
	use super::*;

	const MANIFEST: &str = "# release\n[workspace.package]\nversion = \"1.2.3\" # \
	                        inherited\nlicense = \"MIT\"\n\n[workspace]\nmembers = []\n";

	#[test]
	fn bump_preserves_other_manifest_fields_and_comments() -> Result<()> {
		let root = tempfile::tempdir()?;
		std::fs::write(root.path().join("Cargo.toml"), MANIFEST)?;
		assert_eq!(update(root.path(), None, None)?, "1.2.3");
		assert!(!root.path().join(".cargo").exists());
		assert_eq!(update(root.path(), Some("1.2.4"), Some("1.2.3"))?, "1.2.4");
		assert_eq!(
			std::fs::read_to_string(root.path().join("Cargo.toml"))?,
			MANIFEST.replace("1.2.3", "1.2.4")
		);
		Ok(())
	}

	#[test]
	fn stale_or_invalid_versions_never_change_manifest() -> Result<()> {
		let root = tempfile::tempdir()?;
		let path = root.path().join("Cargo.toml");
		std::fs::write(&path, MANIFEST)?;
		assert!(update(root.path(), Some("1.2.4"), Some("1.2.2")).is_err());
		for invalid in ["01.2.4", "1.2.4-rc.1", "1.2.4+build", "1.2", "garbage"] {
			assert!(update(root.path(), Some(invalid), None).is_err());
		}
		assert_eq!(std::fs::read_to_string(path)?, MANIFEST);
		Ok(())
	}

	#[test]
	fn package_version_cannot_be_mistaken_for_workspace_version() -> Result<()> {
		let root = tempfile::tempdir()?;
		let path = root.path().join("Cargo.toml");
		let manifest = "[package]\nname = \"fixture\"\nversion = \"9.0.0\"\n";
		std::fs::write(&path, manifest)?;
		assert!(update(root.path(), Some("9.0.1"), None).is_err());
		assert_eq!(std::fs::read_to_string(path)?, manifest);
		Ok(())
	}
	#[cfg(unix)]
	#[test]
	fn escaping_manifest_symlink_cannot_modify_another_workspace() -> Result<()> {
		let root = tempfile::tempdir()?;
		let outside = tempfile::tempdir()?;
		let path = outside.path().join("Cargo.toml");
		std::fs::write(&path, MANIFEST)?;
		std::os::unix::fs::symlink(&path, root.path().join("Cargo.toml"))?;
		assert!(update(root.path(), Some("1.2.4"), None).is_err());
		assert_eq!(std::fs::read_to_string(path)?, MANIFEST);
		Ok(())
	}
}
