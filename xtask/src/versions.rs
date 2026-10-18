//! Synchronize publishable path requirements with the shared release version.
//! Registry publication and lockfile regeneration remain explicit caller steps.
use std::{collections::BTreeMap, env, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::JailedFs;
use semver::Version;
use toml_edit::{DocumentMut, Item, TableLike, value};

#[derive(Parser)]
#[command(name = "cargo xtask version")]
struct Arguments {
	#[arg(long)]
	root:    Option<PathBuf>,
	/// Reject drift without writing manifests.
	#[arg(long)]
	check:   bool,
	/// Defaults to the current workspace version. Accepts stable and canary
	/// versions.
	version: Option<String>,
}

struct Manifest {
	path:     PathBuf,
	original: String,
	document: DocumentMut,
}

struct PackageVersion {
	version:   String,
	inherited: bool,
}

fn read_manifest(fs: &JailedFs, path: PathBuf) -> Result<Manifest> {
	let path = fs.relative(&fs.resource_path(&path)?)?;
	let original = String::from_utf8(fs.read(&path, 1024 * 1024)?)?;
	let document = original
		.parse()
		.with_context(|| format!("invalid manifest {}", path.display()))?;
	Ok(Manifest { path, original, document })
}

fn synchronize_dependencies(
	fs: &JailedFs,
	base: &std::path::Path,
	dependencies: &mut dyn TableLike,
	packages: &BTreeMap<PathBuf, PackageVersion>,
) -> Result<()> {
	for (name, item) in dependencies.iter_mut() {
		let Some(dependency) = item.as_table_like_mut() else {
			continue;
		};
		let Some(path) = dependency.get("path").and_then(Item::as_str) else {
			continue;
		};
		let manifest = fs.resource_path(&base.join(path).join("Cargo.toml"))?;
		let target = packages
			.get(&manifest)
			.with_context(|| format!("path dependency {name} is not a declared workspace member"))?;
		// Foreign forks keep their independent version and existing requirement.
		// Cargo still needs a version when normalizing an unversioned path.
		let required = if target.inherited {
			format!("={}", target.version)
		} else if dependency.contains_key("version") {
			continue;
		} else {
			target.version.clone()
		};
		if dependency.get("version").and_then(Item::as_str) != Some(required.as_str()) {
			dependency.insert("version", value(required));
			if let Some(inline) = item.as_inline_table_mut() {
				inline.fmt();
			}
		}
	}
	Ok(())
}

fn synchronize_table(
	fs: &JailedFs,
	base: &std::path::Path,
	table: &mut dyn TableLike,
	packages: &BTreeMap<PathBuf, PackageVersion>,
) -> Result<()> {
	for kind in ["dependencies", "dev-dependencies", "build-dependencies"] {
		if let Some(dependencies) = table.get_mut(kind).and_then(Item::as_table_like_mut) {
			synchronize_dependencies(fs, base, dependencies, packages)?;
		}
	}
	Ok(())
}

/// Validate every manifest before writing. All input and output paths are
/// jailed; each replacement is atomic. The caller regenerates the workspace
/// lockfile.
pub fn synchronize(fs: &JailedFs, version: Option<&str>, check: bool) -> Result<usize> {
	let mut workspace = read_manifest(fs, PathBuf::from("Cargo.toml"))?;
	let current = workspace
		.document
		.get("workspace")
		.and_then(|workspace| workspace.get("package"))
		.and_then(|package| package.get("version"))
		.and_then(Item::as_str)
		.context("workspace.package.version must be a string")?;
	let version = version.unwrap_or(current).to_owned();
	Version::parse(&version).context("expected a bare semantic release version")?;
	let members: Vec<String> = workspace
		.document
		.get("workspace")
		.and_then(|workspace| workspace.get("members"))
		.and_then(Item::as_array)
		.context("workspace.members must be an explicit array")?
		.iter()
		.map(|member| {
			let member = member.as_str().context("workspace member must be a path")?;
			if member.contains(['*', '?', '[', ']']) {
				bail!("version synchronization requires explicit workspace members");
			}
			Ok(member.to_owned())
		})
		.collect::<Result<_>>()?;
	let mut manifests = Vec::new();
	let mut packages = BTreeMap::new();
	for member in members {
		let path = fs.relative(&PathBuf::from(member).join("Cargo.toml"))?;
		let manifest = read_manifest(fs, path)?;
		let package = manifest
			.document
			.get("package")
			.context("workspace member has no package")?;
		let package_version = package
			.get("version")
			.context("workspace member has no version")?;
		let inherited = package_version.get("workspace").and_then(Item::as_bool) == Some(true);
		let package_version = if inherited {
			version.clone()
		} else {
			package_version
				.as_str()
				.context("package version must be a string or inherited from the workspace")?
				.to_owned()
		};
		Version::parse(&package_version).context("invalid workspace member version")?;
		packages.insert(fs.resource_path(&manifest.path)?, PackageVersion {
			version: package_version,
			inherited,
		});
		manifests.push(manifest);
	}
	if workspace.document["workspace"]["package"]["version"].as_str() != Some(&version) {
		workspace.document["workspace"]["package"]["version"] = value(&version);
	}
	manifests.insert(0, workspace);
	for manifest in &mut manifests {
		let base = manifest.path.parent().context("manifest has no parent")?;
		synchronize_table(fs, base, manifest.document.as_table_mut(), &packages)?;
		if let Some(workspace) = manifest
			.document
			.get_mut("workspace")
			.and_then(Item::as_table_like_mut)
		{
			synchronize_table(fs, base, workspace, &packages)?;
		}
		if let Some(targets) = manifest
			.document
			.get_mut("target")
			.and_then(Item::as_table_like_mut)
		{
			for (_, target) in targets.iter_mut() {
				if let Some(target) = target.as_table_like_mut() {
					synchronize_table(fs, base, target, &packages)?;
				}
			}
		}
	}
	let changed: Vec<_> = manifests
		.into_iter()
		.filter_map(|manifest| {
			let rendered = manifest.document.to_string();
			(rendered != manifest.original).then_some((manifest.path, rendered))
		})
		.collect();
	if check && !changed.is_empty() {
		bail!(
			"release version drift in {}; run cargo xtask version",
			changed
				.iter()
				.map(|(path, _)| path.display().to_string())
				.collect::<Vec<_>>()
				.join(", ")
		);
	}
	for (path, contents) in &changed {
		fs.atomic_write(path, contents.as_bytes())?;
	}
	Ok(changed.len())
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let arguments = Arguments::parse_from(std::iter::once("version".into()).chain(arguments));
	let root = arguments.root.unwrap_or(env::current_dir()?);
	let fs = JailedFs::open(&root)?;
	let changed = synchronize(&fs, arguments.version.as_deref(), arguments.check)?;
	println!("{changed} Rust manifests updated; regenerate Cargo.lock after a version change");
	Ok(())
}
