//! Lockstep `@harvest/*` inter-package dependency versions in `packages/*`.
use std::{
	collections::BTreeMap,
	fmt::Write as _,
	path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use clap::Parser;
use harvest_runtime::{JailedFs, MAX_FILE_BYTES};
use serde::Serialize;
use serde_json::{Value, ser::PrettyFormatter};

const PACKAGES: &str = "packages";
const ENTRY_LIMIT: usize = 4096;

#[derive(Parser)]
#[command(name = "cargo xtask sync-versions")]
/// Sync every inter-package dependency to `^<lockstep version>`.
struct Arguments {
	#[arg(long, default_value = ".")]
	workspace: PathBuf,
}

/// One rewritten dependency, for reporting.
#[derive(Debug, PartialEq, Eq)]
pub struct Update {
	pub package:    String,
	pub dependency: String,
	pub from:       String,
	pub to:         String,
	pub dev:        bool,
}

/// Render JSON like `JSON.stringify(value, null, "\t")` plus a trailing
/// newline.
pub fn render(value: &Value) -> Result<String> {
	let mut out = Vec::new();
	value.serialize(&mut serde_json::Serializer::with_formatter(
		&mut out,
		PrettyFormatter::with_indent(b"\t"),
	))?;
	out.push(b'\n');
	Ok(String::from_utf8(out)?)
}

/// Rewrite dependencies of `manifest` that name a workspace package.
pub fn sync(manifest: &mut Value, versions: &BTreeMap<String, String>) -> Vec<Update> {
	let package = manifest
		.get("name")
		.and_then(Value::as_str)
		.unwrap_or_default()
		.to_owned();
	let mut updates = Vec::new();
	for (field, dev) in [("dependencies", false), ("devDependencies", true)] {
		let Some(dependencies) = manifest.get_mut(field).and_then(Value::as_object_mut) else {
			continue;
		};
		for (dependency, current) in dependencies.iter_mut() {
			let Some(version) = versions.get(dependency) else {
				continue;
			};
			let wanted = format!("^{version}");
			if current.as_str() == Some(&wanted) {
				continue;
			}
			updates.push(Update {
				package: package.clone(),
				dependency: dependency.clone(),
				from: current
					.as_str()
					.map_or_else(|| current.to_string(), str::to_owned),
				to: wanted.clone(),
				dev,
			});
			*current = Value::String(wanted);
		}
	}
	updates
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let Some(args) = crate::cli::arguments::<Arguments>("sync-versions", arguments)? else {
		return Ok(());
	};
	let (report, ok) = run(&args.workspace)?;
	print!("{report}");
	if !ok {
		bail!("packages are not at a single lockstep version");
	}
	Ok(())
}

/// Returns the human report and whether the lockstep check passed.
fn run(root: &Path) -> Result<(String, bool)> {
	let fs = JailedFs::open(root)?;
	let mut manifests: Vec<(PathBuf, Value)> = Vec::new();
	let mut versions = BTreeMap::new();
	let mut report = String::new();
	let mut dirs: Vec<String> = fs
		.entries(Path::new(PACKAGES), ENTRY_LIMIT)?
		.into_iter()
		.filter(|entry| !entry.is_file && !entry.is_symlink)
		.map(|entry| entry.name)
		.collect();
	dirs.sort();
	for dir in dirs {
		let path = Path::new(PACKAGES).join(&dir).join("package.json");
		let Some(bytes) = fs.read_optional(&path, MAX_FILE_BYTES)? else {
			eprintln!("Failed to read {}: not found", path.display());
			continue;
		};
		let manifest: Value = match serde_json::from_slice(&bytes) {
			Ok(manifest) => manifest,
			Err(error) => {
				eprintln!("Failed to read {}: {error}", path.display());
				continue;
			},
		};
		if let (Some(name), Some(version)) = (
			manifest.get("name").and_then(Value::as_str),
			manifest.get("version").and_then(Value::as_str),
		) {
			versions.insert(name.to_owned(), version.to_owned());
		}
		manifests.push((path, manifest));
	}

	report.push_str("Current versions:\n");
	for (name, version) in &versions {
		writeln!(report, "  {name}: {version}")?;
	}
	let distinct: std::collections::BTreeSet<_> = versions.values().collect();
	if distinct.len() > 1 {
		report.push_str(
			"\nERROR: Not all packages have the same version!\nExpected lockstep versioning. Set \
			 every package version consistently, then re-run cargo xtask sync-versions.\n",
		);
		return Ok((report, false));
	}
	report.push_str("\nAll packages at same version (lockstep)\n");

	let mut total = 0;
	for (path, mut manifest) in manifests {
		let updates = sync(&mut manifest, &versions);
		if updates.is_empty() {
			continue;
		}
		for update in &updates {
			let dev = if update.dev { " (devDependencies)" } else { "" };
			writeln!(
				report,
				"\n{}:\n  {}: {} → {}{dev}",
				update.package, update.dependency, update.from, update.to
			)?;
		}
		total += updates.len();
		fs.atomic_write(&path, render(&manifest)?.as_bytes())?;
	}
	if total == 0 {
		report.push_str("\nAll inter-package dependencies already in sync.\n");
	} else {
		writeln!(report, "\nUpdated {total} dependency version(s)")?;
	}
	Ok((report, true))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn write(root: &Path, dir: &str, json: &str) {
		let path = root.join(PACKAGES).join(dir);
		std::fs::create_dir_all(&path).unwrap();
		std::fs::write(path.join("package.json"), json).unwrap();
	}

	#[test]
	fn rewrites_only_workspace_dependencies_and_keeps_key_order() -> Result<()> {
		let root = tempfile::tempdir()?;
		write(root.path(), "a", "{\"name\":\"@h/a\",\"version\":\"2.0.0\"}");
		write(
			root.path(),
			"b",
			"{\"version\":\"2.0.0\",\"name\":\"@h/b\",\"dependencies\":{\"zod\":\"1\",\"@h/a\":\"^1.\
			 0.0\"},\"devDependencies\":{\"@h/a\":\"^2.0.0\"}}",
		);
		let (report, ok) = run(root.path())?;
		assert!(ok);
		assert!(report.contains("@h/b:\n  @h/a: ^1.0.0 → ^2.0.0\n"), "{report}");
		assert!(report.ends_with("Updated 1 dependency version(s)\n"), "{report}");
		let written = std::fs::read_to_string(root.path().join("packages/b/package.json"))?;
		assert_eq!(
			written,
			"{\n\t\"version\": \"2.0.0\",\n\t\"name\": \"@h/b\",\n\t\"dependencies\": \
			 {\n\t\t\"zod\": \"1\",\n\t\t\"@h/a\": \"^2.0.0\"\n\t},\n\t\"devDependencies\": \
			 {\n\t\t\"@h/a\": \"^2.0.0\"\n\t}\n}\n"
		);
		Ok(())
	}

	#[test]
	fn mixed_versions_fail_without_writing() -> Result<()> {
		let root = tempfile::tempdir()?;
		let b = "{\"name\":\"@h/b\",\"version\":\"1.0.0\",\"dependencies\":{\"@h/a\":\"^0\"}}";
		write(root.path(), "a", "{\"name\":\"@h/a\",\"version\":\"2.0.0\"}");
		write(root.path(), "b", b);
		let (report, ok) = run(root.path())?;
		assert!(!ok);
		assert!(report.contains("Not all packages have the same version"));
		assert_eq!(std::fs::read_to_string(root.path().join("packages/b/package.json"))?, b);
		Ok(())
	}
}
