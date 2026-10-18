//! Curated release notes retain sections finalized under unpublished tags.
use std::{
	collections::{BTreeMap, BTreeSet},
	env,
	path::{Path, PathBuf},
	process::Command,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::JailedFs;
use regex::Regex;
use semver::Version;
use serde::Deserialize;

const FILE_LIMIT: usize = 1024 * 1024;
const OUTPUT_LIMIT: usize = 8 * FILE_LIMIT;
const CATEGORY_ORDER: &[&str] = &["Breaking Changes", "Added", "Changed", "Fixed", "Removed"];

#[derive(Parser)]
#[command(name = "cargo xtask release-notes")]
struct Arguments {
	/// Target tag/version; defaults to `GITHUB_REF_NAME`.
	version:            Option<String>,
	#[arg(default_value = "release-notes.md")]
	output:             PathBuf,
	#[arg(long, default_value = ".")]
	workspace:          PathBuf,
	/// Merge versions strictly above this stable version through the target.
	#[arg(long, conflicts_with_all = ["single_version", "published_releases"])]
	floor:              Option<String>,
	/// Explicitly include only the target section without querying GitHub.
	#[arg(long, conflicts_with = "published_releases")]
	single_version:     bool,
	/// Read a gh-release-list JSON snapshot instead of querying GitHub.
	#[arg(long)]
	published_releases: Option<PathBuf>,
	#[arg(long)]
	repo:               Option<String>,
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let args =
		Arguments::try_parse_from(std::iter::once("release-notes".to_owned()).chain(arguments))?;
	let target = args
		.version
		.as_deref()
		.map(str::to_owned)
		.or_else(|| env::var("GITHUB_REF_NAME").ok())
		.context("release version is required")?;
	let target = parse_version(&target)?;
	let fs = JailedFs::open(&args.workspace)?;
	let output = fs.relative(&args.output)?;
	if output
		.file_name()
		.and_then(|name| name.to_str())
		.is_some_and(|name| matches!(name, "CHANGELOG.md" | "Cargo.toml" | "package.json"))
		|| args
			.published_releases
			.as_ref()
			.map(|path| fs.relative(path))
			.transpose()?
			.as_ref()
			== Some(&output)
	{
		bail!("release notes output cannot replace a source manifest, changelog or release snapshot");
	}
	let body = if target.pre.is_empty() {
		let floor = resolve_floor(&fs, &args, &target)?;
		collect(&fs, floor.as_ref(), &target)?
	} else if target
		.pre
		.as_str()
		.strip_prefix("canary.")
		.is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()))
	{
		format!(
			"This is a canary prerelease of {}.{}.{}. Install it with `omp update --canary`.\n",
			target.major, target.minor, target.patch
		)
	} else {
		bail!("release notes require a stable version or canary.NUMBER prerelease");
	};
	fs.atomic_write(&output, body.as_bytes())?;
	println!("Wrote release notes for {target} to {}", output.display());
	Ok(())
}

fn parse_version(input: &str) -> Result<Version> {
	let input = input.trim();
	let version = Version::parse(input.strip_prefix('v').unwrap_or(input))?;
	if !version.build.is_empty() {
		bail!("release version cannot contain build metadata");
	}
	Ok(version)
}

fn stable_version(input: &str) -> Result<Version> {
	let version = parse_version(input)?;
	if !version.pre.is_empty() {
		bail!("published release floor must be a stable version");
	}
	Ok(version)
}

fn resolve_floor(fs: &JailedFs, args: &Arguments, target: &Version) -> Result<Option<Version>> {
	if args.single_version {
		return Ok(None);
	}
	if let Some(floor) = &args.floor {
		return stable_version(floor).map(Some);
	}
	if let Some(snapshot) = &args.published_releases {
		return published_floor(&fs.read(snapshot, FILE_LIMIT)?, target);
	}
	if let Some(floor) =
		env::var_os("HARVEST_RELEASE_NOTES_FLOOR").or_else(|| env::var_os("OMP_RELEASE_NOTES_FLOOR"))
	{
		let floor = floor
			.into_string()
			.map_err(|_| anyhow::anyhow!("release floor must be UTF-8"))?;
		return if floor.trim().is_empty() {
			Ok(None)
		} else {
			stable_version(&floor).map(Some)
		};
	}
	let repo = args
		.repo
		.clone()
		.or_else(|| env::var("HARVEST_REPO").ok())
		.or_else(|| env::var("OMP_REPO").ok())
		.or_else(|| env::var("GITHUB_REPOSITORY").ok())
		.unwrap_or_else(|| "harvest/harvest".into());
	let parts = repo.split('/').collect::<Vec<_>>();
	if parts.len() != 2
		|| parts.iter().any(|part| {
			part.is_empty()
				|| part.len() > 100
				|| !part
					.bytes()
					.all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
		}) {
		bail!("release repository must be OWNER/REPO");
	}
	let mut command = Command::new("gh");
	command.args([
		"release",
		"list",
		"--repo",
		&repo,
		"--limit",
		"200",
		"--exclude-drafts",
		"--exclude-pre-releases",
		"--json",
		"tagName,isDraft,isPrerelease",
	]);
	let bytes = crate::process::capture(command, FILE_LIMIT).context(
		"could not resolve published release floor; authenticate gh or explicitly use \
		 --floor/--single-version",
	)?;
	published_floor(&bytes, target)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishedRelease {
	tag_name:      Option<String>,
	#[serde(default)]
	is_draft:      bool,
	#[serde(default)]
	is_prerelease: bool,
}

fn published_floor(bytes: &[u8], target: &Version) -> Result<Option<Version>> {
	let releases: Vec<PublishedRelease> = serde_json::from_slice(bytes)
		.context("published releases must be a JSON array of release records")?;
	Ok(releases
		.iter()
		.filter(|release| !release.is_draft && !release.is_prerelease)
		.filter_map(|release| release.tag_name.as_deref()?.strip_prefix('v'))
		.filter_map(|tag| Version::parse(tag).ok())
		.filter(|version| version.pre.is_empty() && version.build.is_empty())
		.filter(|version| version < target)
		.max())
}

fn collect(fs: &JailedFs, floor: Option<&Version>, target: &Version) -> Result<String> {
	let mut directories = Vec::new();
	for base in ["packages", "crates"] {
		if fs.exists(Path::new(base))? {
			for entry in fs.entries(Path::new(base), 4096)? {
				if entry.is_directory && !entry.is_symlink {
					directories.push(Path::new(base).join(entry.name));
				}
			}
		}
	}
	if fs.exists(Path::new("rust-agent"))? {
		directories.push("rust-agent".into());
	}
	directories.sort();
	let mut sections = Vec::new();
	let mut total = 0;
	for directory in directories {
		let Some(bytes) = fs.read_optional(&directory.join("CHANGELOG.md"), FILE_LIMIT)? else {
			continue;
		};
		let body = merge(std::str::from_utf8(&bytes)?, floor, target)?;
		if body.is_empty() {
			continue;
		}
		let name = package_name(fs, &directory)?;
		let section = format!("## {name}\n\n{body}");
		total += section.len() + 2;
		if total > OUTPUT_LIMIT {
			bail!("release notes exceed output limit")
		}
		sections.push(section);
	}
	Ok(if sections.is_empty() {
		String::new()
	} else {
		sections.join("\n\n") + "\n"
	})
}

fn package_name(fs: &JailedFs, directory: &Path) -> Result<String> {
	if let Some(bytes) = fs.read_optional(&directory.join("Cargo.toml"), FILE_LIMIT)? {
		let manifest: toml_edit::DocumentMut = std::str::from_utf8(&bytes)?.parse()?;
		if let Some(name) = manifest
			.get("package")
			.and_then(|package| package.get("name"))
			.and_then(toml_edit::Item::as_str)
		{
			return Ok(name.to_owned());
		}
	}
	if let Some(bytes) = fs.read_optional(&directory.join("package.json"), FILE_LIMIT)?
		&& let Ok(manifest) = serde_json::from_slice::<serde_json::Value>(&bytes)
		&& let Some(name) = manifest.get("name").and_then(serde_json::Value::as_str)
	{
		return Ok(name.to_owned());
	}
	Ok(directory.to_string_lossy().into_owned())
}

fn merge(content: &str, floor: Option<&Version>, target: &Version) -> Result<String> {
	let heading = Regex::new(r"^## \[(\d+\.\d+\.\d+)\]")?;
	let lines = content.split('\n').collect::<Vec<_>>();
	let boundaries = lines
		.iter()
		.enumerate()
		.filter_map(|(index, line)| line.starts_with("## [").then_some(index))
		.collect::<Vec<_>>();
	let mut spans = Vec::new();
	for (position, start) in boundaries.iter().copied().enumerate() {
		if let Some(captures) = heading.captures(lines[start]) {
			let version = stable_version(&captures[1])?;
			if version <= *target && floor.map_or(version == *target, |floor| version > *floor) {
				spans.push((
					version,
					start + 1,
					boundaries.get(position + 1).copied().unwrap_or(lines.len()),
				));
			}
		}
	}
	spans.sort_by(|left, right| right.0.cmp(&left.0));
	let mut buckets: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
	let mut seen = BTreeSet::new();
	for (_, start, end) in spans {
		let mut category = None;
		for line in &lines[start..end] {
			if let Some(name) = line
				.strip_prefix("### ")
				.map(str::trim_end)
				.filter(|name| !name.is_empty())
			{
				category = Some(name);
			} else if let Some(category) = category {
				let key = line.trim();
				if !key.is_empty() && seen.insert(key) {
					buckets.entry(category).or_default().push(line);
				}
			}
		}
	}
	let mut categories = buckets.keys().copied().collect::<Vec<_>>();
	categories.sort_by_key(|name| {
		(
			CATEGORY_ORDER
				.iter()
				.position(|known| known == name)
				.unwrap_or(CATEGORY_ORDER.len()),
			*name,
		)
	});
	let sections = categories
		.iter()
		.map(|name| format!("### {name}\n\n{}", buckets[name].join("\n")))
		.collect::<Vec<_>>();
	Ok(sections.join("\n\n"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[derive(Deserialize)]
	struct Case {
		name:     String,
		floor:    Option<String>,
		target:   String,
		content:  String,
		expected: String,
	}

	#[test]
	fn version_range_merging_matches_legacy_release_notes() -> Result<()> {
		let cases: Vec<Case> =
			serde_json::from_str(include_str!("../tests/fixtures/release-notes.json"))?;
		for case in cases {
			let floor = case.floor.as_deref().map(stable_version).transpose()?;
			assert_eq!(
				merge(&case.content, floor.as_ref(), &stable_version(&case.target)?)?,
				case.expected,
				"{}",
				case.name
			);
		}
		Ok(())
	}

	#[test]
	fn published_floor_excludes_drafts_prereleases_malformed_and_newer_tags() -> Result<()> {
		let bytes = br#"[
			{"tagName":"v1.2.0"},
			{"tagName":"v1.3.0","isDraft":true},
			{"tagName":"v1.3.1","isPrerelease":true},
			{"tagName":"v1.4.0-canary.1"},
			{"tagName":"vv1.3.9"},
			{"tagName":"1.3.8"},
			{"tagName":"v1.3.7+build"},
			{"tagName":"v1.4.0"},
			{"tagName":"v2.0.0"}
		]"#;
		assert_eq!(
			published_floor(bytes, &stable_version("1.4.0")?)?,
			Some(stable_version("1.2.0")?)
		);
		assert_eq!(published_floor(bytes, &stable_version("1.2.0")?)?, None);
		for invalid in
			[b"{}".as_slice(), b"not json", br#"[{"tagName":42}]"#, br#"[{"isDraft":"false"}]"#]
		{
			assert!(published_floor(invalid, &stable_version("1.4.0")?).is_err());
		}
		Ok(())
	}
}
