//! Reproducible source inventory and conservative migration status.
//!
//! Reading/hashing a file records inventory coverage, never semantic review or
//! behavioral parity. The capability gate remains independent of source counts.
use std::{
	collections::{BTreeMap, BTreeSet},
	env,
	path::{Path, PathBuf},
	process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use harvest_runtime::{ContentDigest, JailedFs, content_hash, read_bounded};
use regex::Regex;
use serde::Serialize;

use crate::capabilities;

const MANIFEST_LIMIT: usize = 16 * 1024 * 1024;
const FILE_LIMIT: usize = 64 * 1024 * 1024;
const FILE_COUNT_LIMIT: usize = 100_000;
const CAPABILITY_PATH: &str = "docs/rust-migration/capabilities.json";

#[derive(Parser)]
#[command(name = "cargo xtask source-audit")]
struct Arguments {
	#[command(subcommand)]
	command: MigrationCommand,
}

#[derive(Subcommand)]
enum MigrationCommand {
	/// Seal source bytes plus a supplied rollback artifact; does not certify
	/// parity.
	Baseline {
		#[arg(long)]
		root:     Option<PathBuf>,
		#[arg(long)]
		artifact: PathBuf,
		#[arg(long)]
		asset:    String,
		#[arg(long)]
		output:   PathBuf,
	},
	/// Verify affected consumers, the full transition, or final release
	/// readiness.
	Verify {
		#[arg(long)]
		root:   Option<PathBuf>,
		#[arg(long, value_enum, default_value = "affected")]
		scope:  crate::verification::Scope,
		#[arg(long)]
		base:   Option<String>,
		#[arg(long)]
		plan:   bool,
		#[arg(long, default_value = "target/migration/verification.json")]
		output: PathBuf,
	},
	/// Read every listed checkout file and write a deterministic source ledger.
	Audit {
		#[arg(long)]
		root:   Option<PathBuf>,
		#[arg(long, default_value = "docs/rust-migration/source-audit.json")]
		output: PathBuf,
	},
	/// Report actual remaining authored JS/TS separately from capability parity.
	Status {
		#[arg(long)]
		root:             Option<PathBuf>,
		/// Require no authored JS/TS AND full retained/retired capability
		/// evidence.
		#[arg(long)]
		require_complete: bool,
	},
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
	Application,
	Tooling,
	Test,
	Example,
	Declaration,
	GeneratedBinding,
	Vendored,
	LanguageFixture,
	Documentation,
	Configuration,
	Asset,
}

#[derive(Debug, Serialize)]
pub struct SourceFile {
	pub path:     String,
	pub language: String,
	pub role:     Role,
	pub bytes:    usize,
	pub lines:    Option<usize>,
	pub sha256:   String,
}

impl SourceFile {
	fn authored_js_ts(&self) -> bool {
		matches!(self.language.as_str(), "javascript" | "typescript")
			&& !matches!(self.role, Role::GeneratedBinding | Role::Vendored | Role::LanguageFixture)
	}
}

#[derive(Debug, Serialize)]
pub struct Inventory {
	pub schema_version:          u32,
	pub scope:                   &'static str,
	pub review_level:            &'static str,
	pub source_fingerprint:      String,
	pub files_read:              usize,
	pub bytes_read:              usize,
	pub authored_js_ts_files:    usize,
	pub authored_js_ts_lines:    usize,
	pub script_asset_candidates: Vec<String>,
	pub files_by_language:       BTreeMap<String, usize>,
	pub absent_tracked_paths:    Vec<String>,
	pub excluded_report:         String,
	pub files:                   Vec<SourceFile>,
}

fn language(path: &Path) -> &'static str {
	match path.extension().and_then(|value| value.to_str()) {
		Some("rs") => "rust",
		Some("ts" | "tsx" | "mts" | "cts") => "typescript",
		Some("js" | "jsx" | "mjs" | "cjs") => "javascript",
		Some("py") => "python",
		Some("sh" | "bash" | "ps1") => "shell",
		Some("kdl") => "kdl",
		Some("md" | "mdx" | "rst") => "documentation",
		Some("json" | "json5" | "jsonl" | "yaml" | "yml" | "toml" | "lock") => "configuration",
		Some("html" | "css" | "svg") => "presentation_asset",
		_ => "other",
	}
}

fn role(path: &str, language: &str) -> Role {
	let parts: Vec<_> = path.split('/').collect();
	if parts.contains(&"vendor") || parts.contains(&"third_party") {
		return Role::Vendored;
	}
	// Most fixture directories contain executable worker/server harnesses.
	// Only these reviewed security-scanner source inputs are language data.
	if path
		.strip_prefix("packages/coding-agent/test/fixtures/security/seeded-repository/src/")
		.is_some_and(|name| {
			matches!(
				name,
				"authorization-bypass.ts"
					| "command-injection.ts"
					| "fake-secret.ts"
					| "path-traversal.ts"
					| "safe-lookalike.ts"
					| "sql-injection.ts"
					| "ssrf.ts"
					| "unsafe-deserialization.ts"
			)
		}) {
		return Role::LanguageFixture;
	}
	if matches!(
		path,
		"packages/tui/src/fuzzy.js"
			| "packages/tui/src/fuzzy.d.ts"
			| "packages/tui/wasm/terminal.js"
			| "packages/tui/wasm/terminal.d.ts"
			| "packages/natives/native/terminal.js"
			| "packages/natives/native/terminal.d.ts"
			| "packages/natives/native/index.js"
			| "packages/natives/native/index.d.ts"
	) {
		return Role::GeneratedBinding;
	}
	if path.ends_with(".d.ts") {
		return Role::Declaration;
	}
	if parts
		.iter()
		.any(|part| matches!(*part, "test" | "tests" | "__tests__"))
		|| path.contains(".test.")
		|| path.contains(".spec.")
	{
		return Role::Test;
	}
	if parts.contains(&"examples") {
		return Role::Example;
	}
	if parts
		.iter()
		.any(|part| matches!(*part, "scripts" | "bench" | "benches"))
		|| path.starts_with("xtask/")
		|| path.starts_with(".github/")
	{
		return Role::Tooling;
	}
	match language {
		"documentation" => Role::Documentation,
		"configuration" | "kdl" => Role::Configuration,
		"presentation_asset" | "other" => Role::Asset,
		_ => Role::Application,
	}
}

fn portable_path(path: &Path) -> Result<String> {
	path
		.components()
		.map(|part| {
			part
				.as_os_str()
				.to_str()
				.context("source path must be UTF-8")
		})
		.collect::<Result<Vec<_>>>()
		.map(|parts| parts.join("/"))
}

/// Classify and hash the exact files selected by Git, including untracked
/// source. Missing tracked files are tombstones; unreadable/escaping/oversized
/// files fail.
pub fn inspect_files(fs: &JailedFs, manifest: &[u8], output: &Path) -> Result<Inventory> {
	if manifest.len() > MANIFEST_LIMIT || (!manifest.is_empty() && !manifest.ends_with(&[0])) {
		bail!("expected a bounded NUL-delimited source manifest");
	}
	let output = fs.relative(output)?;
	// Conservatively flag script elements, event handlers and script URLs.
	// This is a review candidate list, not an HTML parser or proof of execution.
	let script_assets = Regex::new(r"(?i)<\s*script(?:[\s>])|\bon[a-z]+\s*=|javascript\s*:")?;
	let mut paths = BTreeSet::new();
	for name in manifest
		.split(|byte| *byte == 0)
		.filter(|name| !name.is_empty())
	{
		let path = PathBuf::from(std::str::from_utf8(name).context("source path must be UTF-8")?);
		let path = fs.relative(&path)?;
		if path != output {
			paths.insert(path);
		}
		if paths.len() > FILE_COUNT_LIMIT {
			bail!("source file count exceeds limit");
		}
	}
	let mut inventory = Inventory {
		schema_version:          1,
		scope:                   "Git tracked plus nonignored untracked checkout files; ignored \
		                          build outputs and .git internals excluded",
		review_level:            "inventory-only; a file hash or source count does not establish \
		                          semantic review or Rust parity",
		source_fingerprint:      String::new(),
		files_read:              0,
		bytes_read:              0,
		authored_js_ts_files:    0,
		authored_js_ts_lines:    0,
		script_asset_candidates: Vec::new(),
		files_by_language:       BTreeMap::new(),
		absent_tracked_paths:    Vec::new(),
		excluded_report:         portable_path(&output)?,
		files:                   Vec::new(),
	};
	let mut fingerprint = ContentDigest::default();
	for path in paths {
		let path_string = portable_path(&path)?;
		let Some(bytes) = fs.read_optional(&path, FILE_LIMIT)? else {
			inventory.absent_tracked_paths.push(path_string);
			continue;
		};
		let language = language(&path);
		if matches!(
			path.extension().and_then(|part| part.to_str()),
			Some("html" | "htm" | "xhtml" | "svg")
		) && script_assets
			.is_match(std::str::from_utf8(&bytes).context("script asset must be UTF-8")?)
		{
			inventory.script_asset_candidates.push(path_string.clone());
		}
		let file = SourceFile {
			role:     role(&path_string, language),
			language: language.into(),
			path:     path_string,
			bytes:    bytes.len(),
			lines:    std::str::from_utf8(&bytes)
				.ok()
				.filter(|text| !text.contains('\0'))
				.map(|text| text.lines().count()),
			sha256:   content_hash(&bytes),
		};
		fingerprint.update(file.path.as_bytes());
		fingerprint.update(&[0]);
		fingerprint.update(file.sha256.as_bytes());
		fingerprint.update(&[0]);
		inventory.bytes_read += file.bytes;
		*inventory
			.files_by_language
			.entry(file.language.clone())
			.or_default() += 1;
		if file.authored_js_ts() {
			inventory.authored_js_ts_files += 1;
			inventory.authored_js_ts_lines += file.lines.unwrap_or_default();
		}
		inventory.files.push(file);
	}
	inventory.files_read = inventory.files.len();
	inventory.source_fingerprint = fingerprint.hash();
	Ok(inventory)
}

pub(crate) fn git_bytes(root: &Path, arguments: &[&str]) -> Result<Vec<u8>> {
	let mut child = Command::new("git")
		.current_dir(root)
		.args(arguments)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()
		.context("could not enumerate checkout files with Git")?;
	let output = child.stdout.take().context("missing Git stdout")?;
	let bytes = match read_bounded(output, MANIFEST_LIMIT) {
		Ok(bytes) => bytes,
		Err(error) => {
			let _ = child.kill();
			let _ = child.wait();
			return Err(error);
		},
	};
	if !child.wait()?.success() {
		bail!("Git source enumeration failed");
	}
	Ok(bytes)
}

pub(crate) fn current_inventory(fs: &JailedFs) -> Result<Inventory> {
	inspect_files(
		fs,
		&git_bytes(fs.root(), &["ls-files", "-z", "--cached", "--others", "--exclude-standard"])?,
		Path::new("docs/rust-migration/source-audit.json"),
	)
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let arguments = Arguments::parse_from(std::iter::once("migration".into()).chain(arguments));
	if let MigrationCommand::Baseline { root, artifact, asset, output } = arguments.command {
		let fs = JailedFs::open(&root.unwrap_or(env::current_dir()?))?;
		return crate::baseline::capture(&fs, &current_inventory(&fs)?, &artifact, &asset, &output);
	}
	if let MigrationCommand::Verify { root, scope, base, plan, output } = arguments.command {
		return crate::verification::execute(root, scope, base, plan, output);
	}
	let (root, output, require_complete, write_report) = match arguments.command {
		MigrationCommand::Audit { root, output } => (root, output, false, true),
		MigrationCommand::Status { root, require_complete } => {
			(root, "docs/rust-migration/source-audit.json".into(), require_complete, false)
		},
		MigrationCommand::Verify { .. } => unreachable!(),
		MigrationCommand::Baseline { .. } => unreachable!(),
	};
	let fs = JailedFs::open(&root.unwrap_or(env::current_dir()?))?;
	let inventory = inspect_files(
		&fs,
		&git_bytes(fs.root(), &["ls-files", "-z", "--cached", "--others", "--exclude-standard"])?,
		&output,
	)?;
	let manifest: serde_json::Value =
		serde_json::from_slice(&fs.read(Path::new(CAPABILITY_PATH), 16 * 1024 * 1024)?)?;
	let progress = capabilities::inspect(&manifest)?;
	if write_report {
		fs.atomic_write(&output, &serde_json::to_vec_pretty(&inventory)?)?;
	}
	println!(
		"Read and fingerprinted {} files ({} bytes); {} authored JS/TS files ({} lines) remain",
		inventory.files_read,
		inventory.bytes_read,
		inventory.authored_js_ts_files,
		inventory.authored_js_ts_lines
	);
	println!(
		"{} HTML/SVG script assets require reviewed Rust ownership",
		inventory.script_asset_candidates.len()
	);
	println!(
		"{}/{} retained capabilities implemented (partial or complete); {}/{} verified; {}/{} \
		 retirements removed",
		progress.implemented,
		progress.retained,
		progress.verified,
		progress.retained,
		progress.removed,
		progress.retired
	);
	println!(
		"Inventory is not semantic review or parity certification. Source fingerprint: {}",
		inventory.source_fingerprint
	);
	println!(
		"{}/{} installed candidates verified; {}/{} legacy replacements removed; {}/{} publicly \
		 promoted",
		progress.candidate_verified,
		progress.retained,
		progress.legacy_removed,
		progress.retained,
		progress.publicly_promoted,
		progress.retained
	);
	if write_report {
		println!("Source ledger: {}", output.display());
	}
	if require_complete {
		progress.require_complete()?;
		crate::migration::execute(vec!["--require-complete".into()])?;
		if inventory.authored_js_ts_files != 0 {
			bail!(
				"authored JS/TS remains outside generated bindings, vendored sources and language \
				 fixtures"
			);
		}
		if !inventory.script_asset_candidates.is_empty() {
			bail!("HTML/SVG script assets remain without reviewed Rust ownership");
		}
	}
	Ok(())
}
