//! Drift check for impersonated external-tool versions (User-Agent spoofing).
//!
//! Upstream services may reject or deprioritize stale client versions, so this
//! compares each hard-coded default against the tool's latest GitHub release
//! and can bump the source in place.
use std::{io::Read, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::{JailedFs, MAX_FILE_BYTES};
use regex::Regex;

const PROVIDER_FILE: &str = "packages/catalog/src/wire/gemini-headers.ts";
const RESPONSE_LIMIT: u64 = 1024 * 1024;

struct VersionCheck {
	name:           &'static str,
	source_pattern: &'static str,
	repo:           &'static str,
}

const CHECKS: &[VersionCheck] = &[VersionCheck {
	name:           "Gemini CLI",
	source_pattern: r#"PI_AI_GEMINI_CLI_VERSION\s*\|\|\s*"(\d+\.\d+\.\d+)""#,
	repo:           "google-gemini/gemini-cli",
}];

#[derive(Parser)]
#[command(name = "cargo xtask spoofed-versions")]
/// Check spoofed external tool versions against their latest GitHub releases.
struct Arguments {
	/// Update the source in place when a newer release exists.
	#[arg(long)]
	update:    bool,
	#[arg(long, default_value = ".")]
	workspace: PathBuf,
}

/// Outcome of comparing one hard-coded version with its latest release.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
	/// The source pattern did not match.
	Unextractable,
	/// The latest release could not be fetched.
	Unfetchable,
	UpToDate(String),
	Drift {
		current: String,
		latest:  String,
	},
}

/// Extract `major.minor.patch` from a release tag such as `v0.35.3`.
pub fn parse_tag(tag: &str) -> Option<String> {
	Regex::new(r"(\d+\.\d+\.\d+)")
		.ok()?
		.captures(tag)
		.map(|captures| captures[1].to_owned())
}

/// Compare `source` against `latest` for one pattern, returning the outcome and
/// the source with the version bumped (only the first match is rewritten).
pub fn compare(source: &str, pattern: &str, latest: Option<&str>) -> Result<(Outcome, String)> {
	let regex = Regex::new(pattern)?;
	let Some(captures) = regex.captures(source) else {
		return Ok((Outcome::Unextractable, source.to_owned()));
	};
	let whole = captures.get(0).expect("match").as_str();
	let current = captures[1].to_owned();
	let Some(latest) = latest else {
		return Ok((Outcome::Unfetchable, source.to_owned()));
	};
	if current == latest {
		return Ok((Outcome::UpToDate(current), source.to_owned()));
	}
	let updated = source.replacen(whole, &whole.replacen(&current, latest, 1), 1);
	Ok((Outcome::Drift { current, latest: latest.to_owned() }, updated))
}

fn fetch_latest(repo: &str) -> Option<String> {
	let response = ureq::get(&format!("https://api.github.com/repos/{repo}/releases/latest"))
		.timeout(Duration::from_secs(30))
		.set("Accept", "application/vnd.github+json")
		.set("User-Agent", concat!("harvest/", env!("CARGO_PKG_VERSION")))
		.call()
		.ok()?;
	let mut body = String::new();
	response
		.into_reader()
		.take(RESPONSE_LIMIT)
		.read_to_string(&mut body)
		.ok()?;
	let value: serde_json::Value = serde_json::from_str(&body).ok()?;
	parse_tag(value.get("tag_name")?.as_str()?)
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let Some(args) = crate::cli::arguments::<Arguments>("spoofed-versions", arguments)? else {
		return Ok(());
	};
	let fs = JailedFs::open(&args.workspace)?;
	let path = std::path::Path::new(PROVIDER_FILE);
	let bytes = fs
		.read_optional(path, MAX_FILE_BYTES)?
		.with_context(|| format!("{PROVIDER_FILE} does not exist"))?;
	let mut source = String::from_utf8(bytes).context("provider file is not UTF-8")?;
	let (mut any_checked, mut any_drift, mut any_update) = (false, false, false);

	for check in CHECKS {
		let latest = fetch_latest(check.repo);
		let (outcome, updated) = compare(&source, check.source_pattern, latest.as_deref())?;
		match outcome {
			Outcome::Unextractable => {
				eprintln!("[WARN] Could not extract current {} version from source", check.name);
			},
			Outcome::Unfetchable => {
				eprintln!("[FAIL] Could not fetch latest {} version from {}", check.name, check.repo);
			},
			Outcome::UpToDate(current) => {
				any_checked = true;
				println!("[OK]   {}: {current} (up to date)", check.name);
			},
			Outcome::Drift { current, latest } => {
				any_checked = true;
				any_drift = true;
				println!("[DRIFT] {}: {current} -> {latest}", check.name);
				if args.update {
					source = updated;
					any_update = true;
					println!("       Updated in source.");
				}
			},
		}
	}

	if any_update {
		fs.atomic_write(path, source.as_bytes())?;
		println!("\nWrote updates to {PROVIDER_FILE}");
	}
	if !any_checked {
		bail!("No version checks succeeded. Cannot verify freshness.");
	}
	if any_drift && !args.update {
		bail!("Run with --update to apply version bumps.");
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	const SOURCE: &str = "const version = process.env.PI_AI_GEMINI_CLI_VERSION || \
	                      \"0.46.0\";\nconst other = \"0.46.0\";\n";

	#[test]
	fn drift_rewrites_only_the_matched_default() -> Result<()> {
		let (outcome, updated) = compare(SOURCE, CHECKS[0].source_pattern, Some("0.47.1"))?;
		assert_eq!(outcome, Outcome::Drift { current: "0.46.0".into(), latest: "0.47.1".into() });
		assert_eq!(
			updated,
			"const version = process.env.PI_AI_GEMINI_CLI_VERSION || \"0.47.1\";\nconst other = \
			 \"0.46.0\";\n"
		);
		Ok(())
	}

	#[test]
	fn up_to_date_unfetchable_and_unextractable_leave_source_untouched() -> Result<()> {
		let pattern = CHECKS[0].source_pattern;
		assert_eq!(
			compare(SOURCE, pattern, Some("0.46.0"))?,
			(Outcome::UpToDate("0.46.0".into()), SOURCE.into())
		);
		assert_eq!(compare(SOURCE, pattern, None)?, (Outcome::Unfetchable, SOURCE.into()));
		assert_eq!(compare("no version", pattern, Some("1.0.0"))?.0, Outcome::Unextractable);
		Ok(())
	}

	#[test]
	fn release_tags_reduce_to_semver() {
		assert_eq!(parse_tag("v0.35.3").as_deref(), Some("0.35.3"));
		assert_eq!(parse_tag("gemini-cli-v1.2.10-nightly").as_deref(), Some("1.2.10"));
		assert_eq!(parse_tag("latest"), None);
	}

	#[test]
	fn the_real_provider_source_matches_the_pattern() -> Result<()> {
		let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
		let source = std::fs::read_to_string(root.join(PROVIDER_FILE))?;
		assert_ne!(
			compare(&source, CHECKS[0].source_pattern, Some("0.0.0"))?.0,
			Outcome::Unextractable
		);
		Ok(())
	}
}
