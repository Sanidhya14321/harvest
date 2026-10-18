//! Curated release notes retain changes across tags without published releases.
use std::{
	collections::{BTreeMap, BTreeSet},
	env,
	path::{Path, PathBuf},
	process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use harvest_runtime::{JailedFs, read_bounded};
use semver::Version;
use serde::Deserialize;

const CATEGORIES: [&str; 5] = ["Breaking Changes", "Added", "Changed", "Fixed", "Removed"];

#[derive(Parser)]
#[command(name = "cargo xtask release-notes")]
struct Arguments {
	version: Option<String>,
	#[arg(default_value = "release-notes.md")]
	output:  PathBuf,
	#[arg(long)]
	root:    Option<PathBuf>,
	/// Exclusive lower bound; an empty value requests this version alone.
	#[arg(long)]
	floor:   Option<String>,
}

fn version(value: &str) -> Result<Version> {
	let value = value.trim();
	Version::parse(value.strip_prefix('v').unwrap_or(value)).context("invalid release version")
}

/// Merge version sections, retaining newer wording once and canonical
/// categories.
pub fn merge(content: &str, floor: Option<&Version>, target: &Version) -> String {
	let lines: Vec<_> = content.split('\n').collect();
	let headings: Vec<_> = lines
		.iter()
		.enumerate()
		.filter(|(_, line)| line.starts_with("## ["))
		.map(|(index, _)| index)
		.collect();
	let mut sections = Vec::new();
	for (position, start) in headings.iter().copied().enumerate() {
		let Some(name) = lines[start]
			.strip_prefix("## [")
			.and_then(|line| line.split_once(']').map(|(name, _)| name))
		else {
			continue;
		};
		let Ok(section) = Version::parse(name) else {
			continue;
		};
		if !section.pre.is_empty()
			|| !section.build.is_empty()
			|| section > *target
			|| !floor.map_or_else(|| section == *target, |floor| section > *floor)
		{
			continue;
		}
		sections.push((
			section,
			start + 1,
			headings.get(position + 1).copied().unwrap_or(lines.len()),
		));
	}
	sections.sort_by(|a, b| b.0.cmp(&a.0));
	let mut buckets = BTreeMap::<String, Vec<&str>>::new();
	let mut seen = BTreeSet::new();
	for (_, start, end) in sections {
		let mut category = None;
		for line in &lines[start..end] {
			if let Some(name) = line
				.strip_prefix("### ")
				.map(str::trim_end)
				.filter(|name| !name.is_empty())
			{
				category = Some(name.to_owned());
				continue;
			}
			if let Some(category) = &category
				&& !line.trim().is_empty()
				&& seen.insert(line.trim())
			{
				buckets.entry(category.clone()).or_default().push(line);
			}
		}
	}
	let mut categories: Vec<_> = buckets.keys().collect();
	categories.sort_by_key(|name| {
		(
			CATEGORIES
				.iter()
				.position(|known| known == name)
				.unwrap_or(CATEGORIES.len()),
			name.to_lowercase(),
			*name,
		)
	});
	categories
		.into_iter()
		.map(|name| format!("### {name}\n\n{}", buckets[name].join("\n")))
		.collect::<Vec<_>>()
		.join("\n\n")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Release {
	tag_name:      Option<String>,
	is_draft:      Option<bool>,
	is_prerelease: Option<bool>,
}

/// Exclude drafts, prereleases and the target itself when finding silent tags.
pub fn published_floor(payload: &[u8], target: &Version) -> Result<Option<Version>> {
	let releases: Vec<Release> = serde_json::from_slice(payload)
		.context("gh release list returned invalid release metadata")?;
	Ok(releases
		.into_iter()
		.filter(|release| release.is_draft != Some(true) && release.is_prerelease != Some(true))
		.filter_map(|release| {
			let tag = release.tag_name?.strip_prefix('v')?.to_owned();
			let parsed = Version::parse(&tag).ok()?;
			(parsed.pre.is_empty() && parsed.build.is_empty() && parsed < *target).then_some(parsed)
		})
		.max())
}

fn discover_floor(target: &Version) -> Result<Option<Version>> {
	let repo = ["HARVEST_REPO", "OMP_REPO", "GITHUB_REPOSITORY"]
		.iter()
		.find_map(|name| env::var(name).ok())
		.unwrap_or_else(|| "Sanidhya14321/harvest".into());
	let mut child = Command::new("gh")
		.args([
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
		])
		.env("GH_PROMPT_DISABLED", "1")
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::inherit())
		.spawn()
		.context(
			"cannot query published release floor; use --floor or OMP_RELEASE_NOTES_FLOOR for an \
			 explicit offline bound",
		)?;
	let bytes =
		match read_bounded(child.stdout.take().context("missing gh stdout")?, 2 * 1024 * 1024) {
			Ok(bytes) => bytes,
			Err(error) => {
				let _ = child.kill();
				let _ = child.wait();
				return Err(error);
			},
		};
	if !child.wait()?.success() {
		bail!("published release query failed; refusing to omit unpublished tag changes");
	}
	published_floor(&bytes, target)
}

pub fn aggregate(fs: &JailedFs, floor: Option<&Version>, target: &Version) -> Result<String> {
	let mut packages = fs.entries(Path::new("packages"), 4096)?;
	packages.sort_by(|a, b| a.name.cmp(&b.name));
	let mut sections = Vec::new();
	for package in packages {
		if !package.is_directory {
			continue;
		}
		let directory = Path::new("packages").join(&package.name);
		let Some(content) = fs.read_optional(&directory.join("CHANGELOG.md"), 16 * 1024 * 1024)?
		else {
			continue;
		};
		let body = merge(std::str::from_utf8(&content)?, floor, target);
		if body.is_empty() {
			continue;
		}
		let name = fs
			.read_optional(&directory.join("package.json"), 1024 * 1024)?
			.and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
			.and_then(|value| value["name"].as_str().map(str::to_owned))
			.unwrap_or_else(|| format!("packages/{}", package.name));
		sections.push(format!("## {name}\n\n{body}"));
	}
	Ok(if sections.is_empty() {
		String::new()
	} else {
		format!("{}\n", sections.join("\n\n"))
	})
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let args = Arguments::parse_from(std::iter::once("release-notes".into()).chain(arguments));
	let target = version(
		&args
			.version
			.or_else(|| env::var("GITHUB_REF_NAME").ok())
			.context("a release version is required")?,
	)?;
	let fs = JailedFs::open(&args.root.unwrap_or(env::current_dir()?))?;
	let notes = if let Some(suffix) = target
		.pre
		.as_str()
		.strip_prefix("canary.")
		.filter(|suffix| !suffix.is_empty() && suffix.bytes().all(|c| c.is_ascii_digit()))
	{
		let _ = suffix;
		format!(
			"This is a canary prerelease of {}.{}.{}. Install it with `omp update --canary`.\n",
			target.major, target.minor, target.patch
		)
	} else {
		let override_floor = args
			.floor
			.or_else(|| env::var("OMP_RELEASE_NOTES_FLOOR").ok());
		let floor = match override_floor {
			Some(value) if value.trim().trim_start_matches('v').is_empty() => None,
			Some(value) => Some(version(&value)?),
			None => discover_floor(&target)?,
		};
		aggregate(&fs, floor.as_ref(), &target)?
	};
	fs.atomic_write(&args.output, notes.as_bytes())?;
	println!("Curated release notes: {}", args.output.display());
	Ok(())
}
