//! Deterministic release templates. Rendering never publishes artifacts.
use std::{
	collections::BTreeMap,
	io::Write,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use harvest_runtime::JailedFs;

const HOMEBREW: &str = include_str!("../../rust-agent/Formula/omp.rb");
const WINGET: [(&str, &str); 3] = [
	(
		"Pulkitxm.Omp.installer.yaml",
		include_str!("../../rust-agent/packaging/winget/templates/Pulkitxm.Omp.installer.yaml"),
	),
	(
		"Pulkitxm.Omp.locale.en-US.yaml",
		include_str!("../../rust-agent/packaging/winget/templates/Pulkitxm.Omp.locale.en-US.yaml"),
	),
	(
		"Pulkitxm.Omp.yaml",
		include_str!("../../rust-agent/packaging/winget/templates/Pulkitxm.Omp.yaml"),
	),
];
const ASSETS: [(&str, &str); 4] = [
	("MACOS_AARCH64", "aarch64-apple-darwin"),
	("MACOS_X86_64", "x86_64-apple-darwin"),
	("LINUX_AARCH64", "aarch64-unknown-linux-gnu"),
	("LINUX_X86_64", "x86_64-unknown-linux-gnu"),
];

#[derive(Parser)]
#[command(name = "cargo xtask package")]
struct Arguments {
	#[arg(long, global = true, default_value = ".")]
	workspace: PathBuf,
	#[command(subcommand)]
	command:   PackageCommand,
}

#[derive(Subcommand)]
enum PackageCommand {
	/// Emit the authoritative eight-target GitHub Actions matrix.
	Matrix,
	/// Package both aliases and legal notices for a shipping target.
	Archive {
		#[arg(long)]
		target:     String,
		#[arg(long)]
		binary:     PathBuf,
		#[arg(long)]
		output_dir: PathBuf,
	},
	/// Check the complete raw/ZIP asset set and archived alias contents.
	VerifyAssets {
		#[arg(long)]
		directory: PathBuf,
	},
	/// Reject GNU executables requiring a newer libc than the shipping floor.
	VerifyGlibc {
		#[arg(long)]
		binary:    PathBuf,
		#[arg(long, default_value = "2.17")]
		max_glibc: String,
	},
	Homebrew {
		#[arg(long)]
		version:   String,
		#[arg(long,default_value = crate::artifacts::CHECKSUMS)]
		checksums: PathBuf,
		#[arg(long)]
		output:    Option<PathBuf>,
	},
	Winget {
		#[arg(long)]
		version:        String,
		#[arg(long)]
		release_date:   String,
		#[arg(long,default_value = crate::artifacts::CHECKSUMS)]
		checksums:      PathBuf,
		#[arg(long, required_unless_present = "archive_output")]
		output_dir:     Option<PathBuf>,
		#[arg(long)]
		archive_output: Option<PathBuf>,
	},
	/// Validate downloaded manifests and Windows archives before submission.
	VerifyWinget {
		#[arg(long)]
		version:    String,
		#[arg(long)]
		archive:    PathBuf,
		#[arg(long,default_value = crate::artifacts::CHECKSUMS)]
		checksums:  PathBuf,
		#[arg(long)]
		assets_dir: PathBuf,
		#[arg(long)]
		output_dir: PathBuf,
	},
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let arguments = Arguments::try_parse_from(std::iter::once("package".into()).chain(arguments))?;
	// Release jobs run from the checkout root. Output is jailed to that root,
	// including nonexistent leaves and symlinked output-directory parents.
	let fs = JailedFs::open(&arguments.workspace)?;
	match arguments.command {
		PackageCommand::Matrix => println!("{}", crate::artifacts::matrix()?),
		PackageCommand::Archive { target, binary, output_dir } => {
			crate::artifacts::archive(&fs, crate::artifacts::target(&target)?, &binary, &output_dir)?;
		},
		PackageCommand::VerifyAssets { directory } => crate::artifacts::verify(&fs, &directory)?,
		PackageCommand::VerifyGlibc { binary, max_glibc } => {
			println!("{}", crate::abi::verify_glibc(&fs, &binary, &max_glibc)?);
		},
		PackageCommand::Homebrew { version, checksums, output } => {
			let checksums = read_checksums(&String::from_utf8(fs.read(&checksums, 1024 * 1024)?)?)?;
			let formula = homebrew(&version, &checksums)?;
			if let Some(output) = output {
				fs.atomic_write(&output, formula.as_bytes())?;
			} else {
				std::io::stdout().lock().write_all(formula.as_bytes())?;
			}
		},
		PackageCommand::Winget { version, release_date, checksums, output_dir, archive_output } => {
			let checksums = read_checksums(&String::from_utf8(fs.read(&checksums, 1024 * 1024)?)?)?;
			let manifests = winget(&version, &release_date, &checksums)?;
			let archive = archive_output
				.map(|path| {
					crate::artifacts::zip_bytes(
						manifests
							.iter()
							.map(|(name, text)| (*name, text.as_bytes(), 0o644)),
					)
					.map(|bytes| (path, bytes))
				})
				.transpose()?;
			if let Some(directory) = output_dir {
				if let Some((path, _)) = &archive {
					for name in manifests.keys() {
						if fs.relative(path)? == fs.relative(&directory.join(name))? {
							bail!("manifest archive cannot replace a rendered manifest")
						}
					}
				}
				for (name, manifest) in &manifests {
					fs.atomic_write(&directory.join(name), manifest.as_bytes())?;
				}
			}
			if let Some((path, bytes)) = archive {
				fs.atomic_write(&path, &bytes)?;
			}
		},
		PackageCommand::VerifyWinget { version, archive, checksums, assets_dir, output_dir } => {
			let checksums = read_checksums(&String::from_utf8(fs.read(&checksums, 1024 * 1024)?)?)?;
			verify_winget(&fs, &version, &archive, &checksums, &assets_dir, &output_dir)?;
		},
	}
	Ok(())
}

fn read_checksums(text: &str) -> Result<BTreeMap<String, String>> {
	let mut checksums = BTreeMap::new();
	for line in text.lines().filter(|line| !line.trim().is_empty()) {
		let mut fields = line.split_whitespace();
		let digest = fields.next().context("missing checksum")?;
		let name = fields
			.next()
			.context("missing checksum asset")?
			.trim_start_matches('*');
		if fields.next().is_some()
			|| digest.len() != 64
			|| !digest
				.bytes()
				.all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
		{
			bail!("invalid SHA256SUMS entry");
		}
		if checksums.insert(name.into(), digest.into()).is_some() {
			bail!("duplicate checksum asset: {name}");
		}
	}
	Ok(checksums)
}

pub(crate) fn validate_version(version: &str) -> Result<()> {
	let parsed = semver::Version::parse(version)
		.context("expected stable release version MAJOR.MINOR.PATCH")?;
	if version.len() > 32 || !parsed.pre.is_empty() || !parsed.build.is_empty() {
		bail!("expected stable release version MAJOR.MINOR.PATCH");
	}
	Ok(())
}

fn validate_date(date: &str) -> Result<()> {
	let fields: Vec<_> = date.split('-').collect();
	if fields.len() != 3
		|| fields[0].len() != 4
		|| fields[1].len() != 2
		|| fields[2].len() != 2
		|| fields
			.iter()
			.any(|part| !part.bytes().all(|byte| byte.is_ascii_digit()))
	{
		bail!("expected release date YYYY-MM-DD");
	}
	let year = fields[0].parse::<u32>()?;
	let month = fields[1].parse::<u32>()?;
	let day = fields[2].parse::<u32>()?;
	let days = match month {
		2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
		2 => 28,
		4 | 6 | 9 | 11 => 30,
		1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
		_ => bail!("invalid release month"),
	};
	if year == 0 || day == 0 || day > days {
		bail!("invalid release date");
	}
	Ok(())
}

fn replace(mut template: String, values: &[(&str, &str)]) -> Result<String> {
	for (token, value) in values {
		if !template.contains(token) {
			bail!("missing release placeholder: {token}");
		}
		template = template.replace(token, value);
	}
	if template.contains('@') {
		bail!("unsubstituted release placeholder");
	}
	Ok(template)
}

fn homebrew(version: &str, checksums: &BTreeMap<String, String>) -> Result<String> {
	validate_version(version)?;
	let mut owned = vec![("@VERSION@".to_owned(), version.to_owned())];
	for (suffix, target) in ASSETS {
		let asset = crate::artifacts::target(target)?.artifact;
		owned.push((format!("@ASSET_{suffix}@"), asset.to_owned()));
		owned.push((
			format!("@SHA256_{suffix}@"),
			checksums
				.get(asset)
				.with_context(|| format!("missing release asset {asset}"))?
				.clone(),
		));
	}
	let values = owned
		.iter()
		.map(|(key, value)| (key.as_str(), value.as_str()))
		.collect::<Vec<_>>();
	replace(HOMEBREW.into(), &values)
}

fn winget(
	version: &str,
	date: &str,
	checksums: &BTreeMap<String, String>,
) -> Result<BTreeMap<&'static str, String>> {
	validate_version(version)?;
	validate_date(date)?;
	let mut owned = vec![
		("@VERSION@".to_owned(), version.to_owned()),
		("@RELEASE_DATE@".to_owned(), date.to_owned()),
	];
	for (suffix, target) in [("X64", "x86_64-pc-windows-msvc"), ("ARM64", "aarch64-pc-windows-msvc")]
	{
		let archive = crate::artifacts::target(target)?.archive();
		let digest = checksums
			.get(&archive)
			.context("missing Windows archive checksum")?
			.to_ascii_uppercase();
		owned.push((format!("@ASSET_WINDOWS_{suffix}@"), archive));
		owned.push((format!("@SHA256_WINDOWS_{suffix}@"), digest));
	}
	WINGET
		.into_iter()
		.map(|(name, template)| {
			// Version is present in every manifest; the date and digest occur only
			// in the installer manifest.
			let values = owned
				.iter()
				.filter(|(key, _)| template.contains(key))
				.map(|(key, value)| (key.as_str(), value.as_str()))
				.collect::<Vec<_>>();
			Ok((name, replace(template.into(), &values)?))
		})
		.collect()
}

fn verify_winget(
	fs: &JailedFs,
	version: &str,
	path: &Path,
	checksums: &BTreeMap<String, String>,
	assets: &Path,
	output: &Path,
) -> Result<()> {
	validate_version(version)?;
	let filename = path
		.file_name()
		.and_then(|name| name.to_str())
		.context("manifest archive needs a UTF-8 filename")?;
	for (name, limit) in [
		(filename, 4 * 1024 * 1024_u64),
		("omp-windows-x64.zip", 520 * 1024 * 1024),
		("omp-windows-arm64.zip", 520 * 1024 * 1024),
	] {
		let input = if name == filename {
			path.to_path_buf()
		} else {
			assets.join(name)
		};
		let expected = checksums
			.get(name)
			.context("missing submitted archive checksum")?;
		if fs.hash_file(&input, limit)? != *expected {
			bail!("submitted archive checksum mismatch")
		}
	}
	let mut archive = crate::artifacts::read_zip(fs, path, 4 * 1024 * 1024)?;
	if archive.len() != WINGET.len() {
		bail!("manifest archive has unexpected entries")
	}
	let mut manifests = BTreeMap::new();
	let mut values = BTreeMap::new();
	for (name, _) in WINGET {
		let file = archive.by_name(name)?;
		if !file.is_file() {
			bail!("manifest archive entries must be regular files")
		}
		let bytes = harvest_runtime::read_bounded(file, 1024 * 1024)?;
		let value: serde_yaml_ng::Value =
			serde_yaml_ng::from_slice(&bytes).context("invalid submitted manifest YAML")?;
		values.insert(name, value);
		manifests.insert(name, bytes);
	}
	let date = values["Pulkitxm.Omp.installer.yaml"]
		.get("ReleaseDate")
		.and_then(serde_yaml_ng::Value::as_str)
		.context("submitted installer has no release date")?;
	let expected = winget(version, date, checksums)?;
	for (name, text) in expected {
		let expected: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text)?;
		if values[name] != expected {
			bail!(
				"submitted manifest does not match the release version, aliases, architectures, URLs \
				 or checksums"
			)
		}
	}
	// Validate the complete set before publishing any extracted manifest.
	for (name, bytes) in manifests {
		fs.atomic_write(&output.join(name), &bytes)?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use std::fmt::Write as _;

	use super::*;

	#[test]
	fn rendered_release_binds_assets_to_their_own_checksums() {
		let mut listing = String::new();
		for (index, (_, target)) in ASSETS.iter().enumerate() {
			let asset = crate::artifacts::target(target).unwrap().artifact;
			writeln!(listing, "{}  {asset}", index.to_string().repeat(64)).unwrap();
		}
		let checksums = read_checksums(&listing).unwrap();
		let formula = homebrew("1.2.3", &checksums).unwrap();
		assert!(formula.contains("version \"1.2.3\""));
		for (index, (_, target)) in ASSETS.iter().enumerate() {
			let asset = crate::artifacts::target(target).unwrap().artifact;
			let start = formula.find(&format!("/{asset}\"")).unwrap();
			let section = formula[start..].split("    end").next().unwrap();
			assert!(section.contains(&format!("sha256 \"{}\"", index.to_string().repeat(64))));
		}
		let checksums = read_checksums(&format!(
			"{} *omp-windows-x64.zip\n{} *omp-windows-arm64.zip\n",
			"a".repeat(64),
			"b".repeat(64)
		))
		.unwrap();
		let manifests = winget("1.2.3", "2024-02-29", &checksums).unwrap();
		let installer = &manifests["Pulkitxm.Omp.installer.yaml"];
		assert!(installer.contains("/v1.2.3/omp-windows-x64.zip"));
		assert!(installer.contains(&format!("InstallerSha256: \"{}\"", "A".repeat(64))));
		assert!(installer.contains("/v1.2.3/omp-windows-arm64.zip"));
		assert!(installer.contains(&format!("InstallerSha256: \"{}\"", "B".repeat(64))));
		assert!(installer.contains("ReleaseDate: \"2024-02-29\""));
		for manifest in manifests.values() {
			assert!(manifest.contains("PackageVersion: \"1.2.3\""));
		}
	}

	#[test]
	fn ambiguous_checksums_and_invalid_release_values_cannot_produce_packages() {
		let row = format!("{}  omp-windows-x64.zip\n", "a".repeat(64));
		assert!(read_checksums(&(row.clone() + &row)).is_err());
		assert!(read_checksums("abcd archive.zip").is_err());
		let checksums = read_checksums(&row).unwrap();
		assert!(winget("v1.2.3", "2024-02-29", &checksums).is_err());
		assert!(winget("1.2.3", "2023-02-29", &checksums).is_err());
		assert!(winget("1.2.3", "2024-02-29", &BTreeMap::new()).is_err());
		assert!(homebrew("1.2.3", &checksums).is_err());
		assert!(replace("@UNRECOGNIZED@".into(), &[]).is_err());
	}
}
