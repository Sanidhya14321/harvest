//! Release checksum generation owns no publishing or version-changing actions.
use std::{
	collections::BTreeMap,
	env,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::JailedFs;

pub const PLATFORM_ASSETS: [&str; 8] = [
	"omp-linux-x64",
	"omp-linux-arm64",
	"omp-linux-musl-x64",
	"omp-linux-musl-arm64",
	"omp-darwin-x64",
	"omp-darwin-arm64",
	"omp-windows-x64.exe",
	"omp-windows-arm64.exe",
];

#[derive(Parser)]
#[command(name = "cargo xtask checksums")]
struct Arguments {
	/// Explicit filesystem capability for both assets and output.
	#[arg(long)]
	root:              Option<PathBuf>,
	/// Require all eight nonempty platform binaries before writing a manifest.
	#[arg(long)]
	require_platforms: bool,
	output:            PathBuf,
	#[arg(required = true)]
	assets:            Vec<PathBuf>,
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let arguments = Arguments::parse_from(std::iter::once("checksums".into()).chain(arguments));
	let fs = JailedFs::open(&arguments.root.unwrap_or(env::current_dir()?))?;
	generate(&fs, &arguments.output, &arguments.assets, arguments.require_platforms)
}

pub fn generate(
	fs: &JailedFs,
	output: &Path,
	assets: &[PathBuf],
	require_platforms: bool,
) -> Result<()> {
	let output = fs.relative(output)?;
	let mut paths = BTreeMap::new();
	for path in assets {
		let path = fs.relative(path)?;
		if path == output {
			bail!("checksum output cannot also be an input asset");
		}
		let name = path
			.file_name()
			.and_then(|name| name.to_str())
			.context("asset requires a UTF-8 basename")?
			.to_owned();
		if name.is_empty() || name.chars().any(|c| c.is_whitespace() || c == '\\') {
			bail!("asset basename cannot be represented in a checksum manifest");
		}
		if paths.insert(name.clone(), path).is_some() {
			bail!("duplicate checksum asset: {name}");
		}
	}
	let mut hashes = BTreeMap::new();
	for (name, path) in paths {
		hashes.insert(name, fs.hash_file(&path)?);
	}
	if require_platforms {
		for name in PLATFORM_ASSETS {
			let hash = hashes
				.get(name)
				.with_context(|| format!("missing release asset: {name}"))?;
			if hash == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" {
				bail!("empty release asset: {name}");
			}
		}
	}
	let mut manifest = String::new();
	for (name, hash) in hashes {
		manifest.push_str(&hash);
		manifest.push_str("  ");
		manifest.push_str(&name);
		manifest.push('\n');
	}
	fs.atomic_write(&output, manifest.as_bytes())
}
