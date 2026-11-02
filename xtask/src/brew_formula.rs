//! Homebrew formula rendering for the `omp` bare-binary release assets.
//!
//! Digests come straight from the published release metadata so the formula
//! never drifts from the shipped assets.
use std::{env, fs, path::PathBuf, process::Command};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde::Deserialize;

const HOMEPAGE: &str = "https://omp.sh";
const DESC: &str = "Coding agent with the IDE wired in";
const METADATA_LIMIT: usize = 4 * 1024 * 1024;
/// Release assets the formula pins, in stanza order.
pub const TARGETS: [&str; 4] =
	["omp-darwin-arm64", "omp-darwin-x64", "omp-linux-arm64", "omp-linux-x64"];

#[derive(Parser)]
#[command(name = "cargo xtask brew-formula")]
/// Render the Homebrew formula for `omp` from a published GitHub release.
struct Arguments {
	/// Release tag, e.g. v15.10.3.
	tag:         String,
	/// Write the formula here instead of stdout.
	#[arg(long)]
	out:         Option<PathBuf>,
	/// Read `gh release view --json assets` output from a file instead of `gh`.
	#[arg(long)]
	assets_file: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Release {
	assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
	name:   String,
	digest: Option<String>,
}

fn repository() -> String {
	env::var("HARVEST_REPO")
		.or_else(|_| env::var("OMP_REPO"))
		.unwrap_or_else(|_| "harvest/harvest".to_owned())
}

/// Look up each target's sha256 digest in `gh release view --json assets`
/// output.
pub fn digests(metadata: &[u8]) -> Result<[String; 4]> {
	let release: Release = serde_json::from_slice(metadata).context("invalid release metadata")?;
	let mut sums: [String; 4] = Default::default();
	for (sum, name) in sums.iter_mut().zip(TARGETS) {
		let asset = release
			.assets
			.iter()
			.find(|asset| asset.name == name)
			.with_context(|| format!("release is missing asset {name}"))?;
		asset
			.digest
			.as_deref()
			.and_then(|digest| digest.strip_prefix("sha256:"))
			.with_context(|| {
				format!(
					"asset {name} has no sha256 digest (got {})",
					asset.digest.as_deref().unwrap_or("none")
				)
			})?
			.clone_into(sum);
	}
	Ok(sums)
}

/// Render the formula. `#{version}`/`#{bin}` are Ruby interpolations resolved
/// by Homebrew.
///
/// Each `url` carries `using: :nounzip` because the assets are bare
/// executables; without it Homebrew nests the file outside the staging CWD and
/// `bin.install nil` raises. Completions run under `with_env(HOME: buildpath)`
/// so the binary never touches the sandbox-denied real home directory.
pub fn render(version: &str, repo: &str, sums: &[String; 4]) -> String {
	let stanza = |name: &str, sum: &str| {
		format!(
			"      url \"https://github.com/{repo}/releases/download/v#{{version}}/{name}\",\n          \
			 using: :nounzip\n      sha256 \"{sum}\"\n"
		)
	};
	format!(
		r##"class Omp < Formula
  desc "{DESC}"
  homepage "{HOMEPAGE}"
  version "{version}"
  license "MIT"

  on_macos do
    on_arm do
{}    end
    on_intel do
{}    end
  end

  on_linux do
    on_arm do
{}    end
    on_intel do
{}    end
  end

  def install
    bin.install Dir["omp-*"].first => "omp"
    (bin/"omp").chmod 0555
    with_env(HOME: buildpath) do
      generate_completions_from_executable(bin/"omp", "completions", shells: [:bash, :zsh, :fish])
    end
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/omp --version")
  end
end
"##,
		stanza(TARGETS[0], &sums[0]),
		stanza(TARGETS[1], &sums[1]),
		stanza(TARGETS[2], &sums[2]),
		stanza(TARGETS[3], &sums[3]),
	)
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let Some(args) = crate::cli::arguments::<Arguments>("brew-formula", arguments)? else {
		return Ok(());
	};
	let repo = repository();
	let metadata = if let Some(path) = &args.assets_file {
		fs::read(path).with_context(|| format!("could not read {}", path.display()))?
	} else {
		let mut command = Command::new("gh");
		command.args(["release", "view", &args.tag, "--repo", &repo, "--json", "assets"]);
		crate::process::capture(command, METADATA_LIMIT)
			.with_context(|| format!("gh release view {} failed", args.tag))?
	};
	let version = args.tag.strip_prefix('v').unwrap_or(&args.tag);
	let formula = render(version, &repo, &digests(&metadata)?);
	match args.out {
		Some(out) => {
			if out.as_os_str().is_empty() {
				bail!("--out requires a path");
			}
			if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
				fs::create_dir_all(parent)?;
			}
			fs::write(&out, formula)?;
			println!("wrote {} for {}", out.display(), args.tag);
		},
		None => print!("{formula}"),
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sums() -> [String; 4] {
		["darwin_arm64_sha", "darwin_x64_sha", "linux_arm64_sha", "linux_x64_sha"].map(String::from)
	}

	// Regression (issue #2398): bare-binary URLs must opt out of UnpackStrategy.
	#[test]
	fn every_url_opts_out_of_unpacking_next_to_its_digest() {
		let formula = render("15.12.1", "harvest/harvest", &sums());
		assert_eq!(formula.matches("using: :nounzip").count(), 4);
		for (name, sum) in TARGETS.iter().zip(sums()) {
			assert!(
				formula.contains(&format!(
					"/v#{{version}}/{name}\",\n          using: :nounzip\n      sha256 \"{sum}\""
				)),
				"{name}: {formula}"
			);
		}
	}

	// Regression (issue #2398): completions only run with HOME redirected.
	#[test]
	fn completions_run_only_inside_the_home_redirect() {
		let formula = render("1.0.0", "harvest/harvest", &sums());
		let block = "    with_env(HOME: buildpath) do\n      \
		             generate_completions_from_executable(bin/\"omp\", \"completions\", shells: \
		             [:bash, :zsh, :fish])\n    end\n";
		assert!(formula.contains(block), "{formula}");
		assert!(
			!formula
				.replace(block, "")
				.contains("generate_completions_from_executable")
		);
	}

	#[test]
	fn digests_follow_target_order_and_reject_missing_or_unhashed_assets() -> Result<()> {
		let metadata = br#"{"assets":[
			{"name":"omp-linux-x64","digest":"sha256:d"},{"name":"omp-darwin-arm64","digest":"sha256:a"},
			{"name":"omp-linux-arm64","digest":"sha256:c"},{"name":"omp-darwin-x64","digest":"sha256:b"}]}"#;
		assert_eq!(digests(metadata)?, ["a", "b", "c", "d"].map(String::from));

		let missing = br#"{"assets":[{"name":"omp-darwin-arm64","digest":"sha256:a"}]}"#;
		assert_eq!(
			digests(missing).unwrap_err().to_string(),
			"release is missing asset omp-darwin-x64"
		);
		let unhashed = br#"{"assets":[{"name":"omp-darwin-arm64"}]}"#;
		assert_eq!(
			digests(unhashed).unwrap_err().to_string(),
			"asset omp-darwin-arm64 has no sha256 digest (got none)"
		);
		Ok(())
	}
}
