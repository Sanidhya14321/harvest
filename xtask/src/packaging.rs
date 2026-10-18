//! Deterministic release templates. Rendering never publishes artifacts.
use std::{collections::BTreeMap, env, io::Write, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use harvest_runtime::JailedFs;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

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
	("@SHA256_MACOS_AARCH64@", "omp-darwin-arm64"),
	("@SHA256_MACOS_X86_64@", "omp-darwin-x64"),
	("@SHA256_LINUX_AARCH64@", "omp-linux-musl-arm64"),
	("@SHA256_LINUX_X86_64@", "omp-linux-musl-x64"),
];

#[derive(Parser)]
#[command(name = "cargo xtask package")]
struct Arguments {
	#[command(subcommand)]
	command: PackageCommand,
}

#[derive(Subcommand)]
enum PackageCommand {
	/// Create the two-alias payloads consumed by cargo-binstall and `WinGet`.
	Archives {
		#[arg(long)]
		assets_dir: PathBuf,
	},
	Homebrew {
		#[arg(long)]
		version:   String,
		#[arg(long)]
		checksums: PathBuf,
		#[arg(long)]
		output:    Option<PathBuf>,
	},
	Winget {
		#[arg(long)]
		version:      String,
		#[arg(long)]
		release_date: String,
		#[arg(long)]
		checksums:    PathBuf,
		#[arg(long)]
		output_dir:   PathBuf,
	},
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let arguments = Arguments::parse_from(std::iter::once("package".into()).chain(arguments));
	// Release jobs run from the checkout root. Output is jailed to that root,
	// including nonexistent leaves and symlinked output-directory parents.
	let fs = JailedFs::open(&env::current_dir()?)?;
	match arguments.command {
		PackageCommand::Archives { assets_dir } => archives(&fs, &assets_dir)?,
		PackageCommand::Homebrew { version, checksums, output } => {
			let checksums = read_checksums(&String::from_utf8(fs.read(&checksums, 1024 * 1024)?)?)?;
			let formula = homebrew(&version, &checksums)?;
			if let Some(output) = output {
				fs.atomic_write(&output, formula.as_bytes())?;
			} else {
				std::io::stdout().lock().write_all(formula.as_bytes())?;
			}
		},
		PackageCommand::Winget { version, release_date, checksums, output_dir } => {
			let checksums = read_checksums(&String::from_utf8(fs.read(&checksums, 1024 * 1024)?)?)?;
			let manifests = winget(&version, &release_date, &checksums)?;
			for (name, manifest) in manifests {
				fs.atomic_write(&output_dir.join(name), manifest.as_bytes())?;
			}
		},
	}
	Ok(())
}

/// Stream each platform binary into a deterministic archive containing both
/// public aliases. Keep the existing eight standalone downloads unchanged.
pub fn archives(fs: &JailedFs, assets_dir: &std::path::Path) -> Result<()> {
	const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;
	let assets_dir = fs.relative(assets_dir)?;
	for asset in crate::checksums::PLATFORM_ASSETS {
		let source = assets_dir.join(asset);
		let archive = assets_dir.join(format!("{}.zip", asset.trim_end_matches(".exe")));
		let extension = if std::path::Path::new(asset)
			.extension()
			.is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
		{
			".exe"
		} else {
			""
		};
		fs.atomic_write_with(&archive, |file| {
			let mut zip = ZipWriter::new(file);
			let options = SimpleFileOptions::default()
				.compression_method(CompressionMethod::Deflated)
				.unix_permissions(0o755);
			for alias in ["omp", "harvest"] {
				zip.start_file(format!("{alias}{extension}"), options)?;
				if fs.copy_file_to(&source, &mut zip, MAX_BINARY_BYTES)? == 0 {
					bail!("empty release asset: {asset}");
				}
			}
			zip.finish()?;
			Ok(())
		})?;
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
			bail!("invalid SHA256SUMS.txt entry");
		}
		if checksums.insert(name.into(), digest.into()).is_some() {
			bail!("duplicate checksum asset: {name}");
		}
	}
	Ok(checksums)
}

fn validate_version(version: &str) -> Result<()> {
	let fields: Vec<_> = version.split('.').collect();
	if version.len() > 32
		|| fields.len() != 3
		|| fields
			.iter()
			.any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
	{
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
	let mut values = vec![("@VERSION@", version)];
	for (token, asset) in ASSETS {
		values.push((
			token,
			checksums
				.get(asset)
				.with_context(|| format!("missing release asset {asset}"))?
				.as_str(),
		));
	}
	replace(HOMEBREW.into(), &values)
}

fn winget(
	version: &str,
	date: &str,
	checksums: &BTreeMap<String, String>,
) -> Result<BTreeMap<&'static str, String>> {
	validate_version(version)?;
	validate_date(date)?;
	let digest = checksums
		.get("omp-windows-x64.zip")
		.context("missing Windows archive checksum")?
		.to_ascii_uppercase();
	WINGET
		.into_iter()
		.map(|(name, template)| {
			// Version is present in every manifest; the date and digest occur only
			// in the installer manifest.
			let mut values = vec![("@VERSION@", version)];
			if template.contains("@RELEASE_DATE@") {
				values.push(("@RELEASE_DATE@", date));
			}
			if template.contains("@SHA256_WINDOWS_X86_64@") {
				values.push(("@SHA256_WINDOWS_X86_64@", digest.as_str()));
			}
			Ok((name, replace(template.into(), &values)?))
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use std::fmt::Write as _;

	use super::*;

	#[test]
	fn rendered_release_binds_assets_to_their_own_checksums() {
		let mut listing = String::new();
		for (index, (_, asset)) in ASSETS.iter().enumerate() {
			writeln!(listing, "{}  {asset}", index.to_string().repeat(64)).unwrap();
		}
		let checksums = read_checksums(&listing).unwrap();
		let formula = homebrew("1.2.3", &checksums).unwrap();
		assert!(formula.contains("version \"1.2.3\""));
		for (index, (_, asset)) in ASSETS.iter().enumerate() {
			let start = formula.find(&format!("/{asset}\"")).unwrap();
			let section = formula[start..].split("    end").next().unwrap();
			assert!(section.contains(&format!("sha256 \"{}\"", index.to_string().repeat(64))));
			assert!(
				section.contains("using: :nounzip"),
				"bare binary must bypass Homebrew archive extraction"
			);
		}
		assert!(
			formula
				.contains("with_env(HOME: buildpath) do\n      generate_completions_from_executable"),
			"completion subprocess must use a sandbox-writable home"
		);
		let checksums =
			read_checksums(&format!("{} *omp-windows-x64.zip\n", "a".repeat(64))).unwrap();
		let manifests = winget("1.2.3", "2024-02-29", &checksums).unwrap();
		let installer = &manifests["Pulkitxm.Omp.installer.yaml"];
		assert!(installer.contains("/v1.2.3/omp-windows-x64.zip"));
		assert!(installer.contains(&format!("InstallerSha256: \"{}\"", "A".repeat(64))));
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
