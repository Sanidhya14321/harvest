//! Bounded, jailed package-manifest preparation. Does not install or publish.
use std::{collections::BTreeMap, env, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::JailedFs;
use semver::Version;
use serde::Serialize;
use serde_json::{Value, ser::PrettyFormatter};

#[derive(Parser)]
#[command(name = "cargo xtask npm-versions")]
struct Arguments {
	#[arg(long)]
	root:      Option<PathBuf>,
	/// Reject changes without writing any manifest.
	#[arg(long)]
	check:     bool,
	/// Prepare public package versions and the root Bun catalog for a release.
	/// Private package versions and catalog/workspace dependency protocols stay
	/// intact.
	#[arg(long, value_name = "VERSION")]
	release:   Option<String>,
	/// Use workspace.package.version from Cargo.toml as the public release
	/// version.
	#[arg(long, conflicts_with = "release")]
	workspace: bool,
}

struct Manifest {
	path:     PathBuf,
	original: Vec<u8>,
	value:    Value,
}

fn load(fs: &JailedFs, path: PathBuf) -> Result<Manifest> {
	let parent = path
		.parent()
		.filter(|path| !path.as_os_str().is_empty())
		.unwrap_or_else(|| std::path::Path::new("."));
	let name = path
		.file_name()
		.and_then(|name| name.to_str())
		.context("manifest has no filename")?;
	let entry = fs
		.entries(parent, 4096)?
		.into_iter()
		.find(|entry| entry.name == name)
		.context("package manifest is missing")?;
	if entry.is_symlink || !entry.is_file {
		bail!("package manifest is not a regular file: {}", path.display());
	}
	let original = fs.read(&path, 1024 * 1024)?;
	let value: Value = serde_json::from_slice(&original)
		.with_context(|| format!("invalid package manifest {}", path.display()))?;
	if !value.is_object() {
		bail!("package manifest must be an object: {}", path.display());
	}
	Ok(Manifest { path, original, value })
}

fn render(value: &Value) -> Result<Vec<u8>> {
	let mut bytes = Vec::new();
	let mut serializer =
		serde_json::Serializer::with_formatter(&mut bytes, PrettyFormatter::with_indent(b"\t"));
	value.serialize(&mut serializer)?;
	bytes.push(b'\n');
	Ok(bytes)
}

/// Validate all input before writing; individual replacements are atomic.
///
/// Strict synchronization retains the old script's lockstep rule. Release
/// preparation preserves independent private versions and protocol-based
/// workspace links.
pub fn synchronize(fs: &JailedFs, release: Option<&str>, check: bool) -> Result<usize> {
	if let Some(version) = release {
		Version::parse(version).context("expected a bare semantic release version")?;
	}
	let mut manifests = Vec::new();
	let mut versions = BTreeMap::new();
	let mut names = BTreeMap::new();
	let mut total_bytes = 0_usize;
	for entry in fs.entries(std::path::Path::new("packages"), 4096)? {
		if entry.is_symlink {
			bail!("package directory is a symlink: {}", entry.name);
		}
		if !entry.is_directory {
			continue;
		}
		let path = PathBuf::from("packages")
			.join(entry.name)
			.join("package.json");
		let package_entry = fs
			.entries(path.parent().context("package has no parent")?, 4096)?
			.into_iter()
			.find(|entry| entry.name == "package.json");
		if package_entry.is_none() {
			continue;
		}
		let mut manifest = load(fs, path)?;
		total_bytes += manifest.original.len();
		if total_bytes > 16 * 1024 * 1024 {
			bail!("package manifests exceed the 16 MiB limit");
		}
		let name = manifest.value["name"]
			.as_str()
			.filter(|name| !name.is_empty())
			.context("package name must be a nonempty string")?
			.to_owned();
		if names.insert(name.clone(), manifest.path.clone()).is_some() {
			bail!("duplicate package name: {name}");
		}
		let version = manifest.value["version"]
			.as_str()
			.context("package version must be a string")?;
		Version::parse(version).with_context(|| format!("invalid version for {name}"))?;
		let private = match manifest.value.get("private") {
			None => false,
			Some(value) => value
				.as_bool()
				.context("package private must be a boolean")?,
		};
		if let Some(release) = release {
			if !private {
				versions.insert(name, release.to_owned());
				manifest.value["version"] = Value::String(release.to_owned());
			}
		} else {
			versions.insert(name, version.to_owned());
		}
		manifests.push(manifest);
	}
	if versions.is_empty() {
		bail!("no versioned packages selected");
	}
	if release.is_none()
		&& versions
			.values()
			.any(|version| Some(version) != versions.values().next())
	{
		bail!(
			"package versions are not in lockstep; use --release VERSION to preserve independent \
			 private versions"
		);
	}
	for manifest in &mut manifests {
		for kind in ["dependencies", "devDependencies"] {
			let Some(dependencies) = manifest.value.get_mut(kind) else {
				continue;
			};
			let dependencies = dependencies
				.as_object_mut()
				.with_context(|| format!("{kind} must be an object"))?;
			for (name, requirement) in dependencies {
				let current = requirement
					.as_str()
					.context("dependency requirement must be a string")?;
				if let Some(version) = versions.get(name) {
					if release.is_some()
						&& (current.starts_with("catalog:") || current.starts_with("workspace:"))
					{
						continue;
					}
					*requirement = Value::String(format!("^{version}"));
				}
			}
		}
	}
	if release.is_some() {
		let mut root = load(fs, PathBuf::from("package.json"))?;
		if let Some(catalog) = root.value.pointer_mut("/workspaces/catalog") {
			for (name, requirement) in catalog
				.as_object_mut()
				.context("Bun workspace catalog must be an object")?
			{
				if let Some(version) = versions.get(name) {
					*requirement = Value::String(version.clone());
				}
			}
		}
		manifests.push(root);
	}
	let mut changes = Vec::new();
	for manifest in manifests {
		// Compare parsed values so an already synchronized file retains its formatting.
		if serde_json::from_slice::<Value>(&manifest.original)? != manifest.value {
			changes.push((manifest.path, render(&manifest.value)?));
		}
	}
	if check && !changes.is_empty() {
		bail!(
			"npm package version drift in {}",
			changes
				.iter()
				.map(|(path, _)| path.display().to_string())
				.collect::<Vec<_>>()
				.join(", ")
		);
	}
	for (path, bytes) in &changes {
		fs.atomic_write(path, bytes)?;
	}
	Ok(changes.len())
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let args = Arguments::parse_from(std::iter::once("npm-versions".into()).chain(arguments));
	let fs = JailedFs::open(&args.root.unwrap_or(env::current_dir()?))?;
	let release = if args.workspace {
		let contents = String::from_utf8(fs.read(std::path::Path::new("Cargo.toml"), 1024 * 1024)?)?;
		let cargo: toml_edit::DocumentMut = contents
			.parse()
			.context("invalid Cargo workspace manifest")?;
		Some(
			cargo
				.get("workspace")
				.and_then(|value| value.get("package"))
				.and_then(|value| value.get("version"))
				.and_then(toml_edit::Item::as_str)
				.context("workspace.package.version must be a string")?
				.to_owned(),
		)
	} else {
		args.release
	};
	let count = synchronize(&fs, release.as_deref(), args.check)?;
	println!("{count} npm manifests updated; regenerate bun.lock after changes");
	Ok(())
}
