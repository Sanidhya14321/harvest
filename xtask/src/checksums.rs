//! Portable, deterministic SHA256SUMS for release assets.
use std::{
	collections::BTreeMap,
	env,
	fmt::Write,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::JailedFs;

const MAX_ASSETS: usize = 1024;
const MAX_ASSET_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Parser)]
#[command(name = "cargo xtask checksums")]
struct Arguments {
	output: PathBuf,
	#[arg(required = true, num_args = 1..)]
	assets: Vec<PathBuf>,
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let args = Arguments::try_parse_from(std::iter::once("checksums".to_owned()).chain(arguments))?;
	let fs = JailedFs::open(&env::current_dir()?)?;
	write(&fs, &args.output, &args.assets)
}

fn write(fs: &JailedFs, output: &Path, assets: &[PathBuf]) -> Result<()> {
	if assets.is_empty() || assets.len() > MAX_ASSETS {
		bail!("expected between 1 and {MAX_ASSETS} release assets");
	}
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
			.context("asset name must be UTF-8")?;
		// Reject names requiring GNU escaping or capable of injecting manifest rows.
		if name.len() > 255
			|| !name
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
		{
			bail!(
				"release asset name must contain only ASCII letters, digits, dot, underscore or hyphen"
			);
		}
		if paths.insert(name.to_owned(), path.clone()).is_some() {
			bail!("duplicate release asset basename: {name}");
		}
	}
	let mut manifest = String::new();
	for (name, path) in paths {
		let digest = fs
			.hash_file(&path, MAX_ASSET_BYTES)
			.with_context(|| format!("could not hash release asset {name}"))?;
		writeln!(manifest, "{digest}  {name}")?;
	}
	fs.atomic_write(&output, manifest.as_bytes())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn manifest_hashes_bytes_and_sorts_download_names() -> Result<()> {
		let root = tempfile::tempdir()?;
		let fs = JailedFs::open(root.path())?;
		fs.atomic_write(Path::new("assets/omp-a"), b"abc")?;
		fs.atomic_write(Path::new("assets/omp-z"), b"")?;
		write(&fs, Path::new("SHA256SUMS.txt"), &["assets/omp-z".into(), "assets/omp-a".into()])?;
		assert_eq!(
			String::from_utf8(fs.read(Path::new("SHA256SUMS.txt"), 1024)?)?,
			"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  \
			 omp-a\ne3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  omp-z\n"
		);
		Ok(())
	}

	#[test]
	fn bad_assets_do_not_replace_an_existing_manifest() -> Result<()> {
		let root = tempfile::tempdir()?;
		let fs = JailedFs::open(root.path())?;
		let output = Path::new("SHA256SUMS");
		fs.atomic_write(output, b"previous release")?;
		fs.atomic_write(Path::new("a/omp"), b"a")?;
		fs.atomic_write(Path::new("b/omp"), b"b")?;
		for assets in [
			vec![],
			vec!["a/omp".into(), "b/omp".into()],
			vec!["missing".into()],
			vec!["SHA256SUMS".into()],
			vec!["../outside".into()],
			vec!["line\nbreak".into()],
		] {
			assert!(write(&fs, output, &assets).is_err());
			assert_eq!(fs.read(output, 1024)?, b"previous release");
		}
		Ok(())
	}

	#[test]
	fn incremental_hash_checks_file_size_and_regular_file_contract() -> Result<()> {
		let root = tempfile::tempdir()?;
		let fs = JailedFs::open(root.path())?;
		let bytes = vec![b'x'; 200_000];
		fs.atomic_write(Path::new("asset"), &bytes)?;
		assert_eq!(fs.hash_file(Path::new("asset"), 200_000)?, harvest_runtime::content_hash(&bytes));
		assert!(fs.hash_file(Path::new("asset"), 199_999).is_err());
		assert!(fs.hash_file(Path::new("."), 200_000).is_err());
		Ok(())
	}

	#[cfg(unix)]
	#[test]
	fn escaping_assets_and_output_symlinks_preserve_external_files() -> Result<()> {
		let root = tempfile::tempdir()?;
		let outside = tempfile::tempdir()?;
		std::fs::write(outside.path().join("asset"), b"private")?;
		std::os::unix::fs::symlink(outside.path(), root.path().join("escape"))?;
		let fs = JailedFs::open(root.path())?;
		assert!(write(&fs, Path::new("out"), &["escape/asset".into()]).is_err());
		fs.atomic_write(Path::new("asset"), b"public")?;
		assert!(write(&fs, Path::new("escape/asset"), &["asset".into()]).is_err());
		assert_eq!(std::fs::read(outside.path().join("asset"))?, b"private");
		Ok(())
	}
}
