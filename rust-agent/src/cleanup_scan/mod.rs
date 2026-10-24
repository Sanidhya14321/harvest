//! Read-only cleanup discovery: six heuristic reports, never an automatic
//! deletion decision. Foreign-language sources are parsed as inert input data.
mod clones;
mod render;
use std::{
	collections::{BTreeMap, BTreeSet},
	io::{self, Write},
	path::{Component, Path, PathBuf},
	sync::LazyLock,
};

use anyhow::{Result, ensure};
use harvest_runtime::{JailedFs, cancel::CancelToken};
use pi_ast::cleanup::Structure;
use regex::Regex;
use serde::Serialize;

pub const VERSION: &str = "rust-cleanup-v1";
#[derive(Debug, clap::Args)]
pub(crate) struct Arguments {
	#[arg(long, default_value = ".")]
	workspace: PathBuf,
	#[arg(long)]
	json:      bool,
	#[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=1000))]
	top:       u16,
	#[arg(long = "pkg")]
	packages:  Option<String>,
}
#[derive(Debug, Clone)]
pub struct Options {
	pub packages:     Option<Vec<String>>,
	pub all_packages: bool,
	pub top:          usize,
}
impl Default for Options {
	fn default() -> Self {
		Self { packages: None, all_packages: false, top: 20 }
	}
}
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checks {
	pub as_casts:      usize,
	pub opt_chain:     usize,
	pub coalesce:      usize,
	pub typeof_narrow: usize,
	pub is_array:      usize,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GodObject {
	pub file:                 String,
	pub loc:                  usize,
	pub exports:              usize,
	pub top_level_statements: usize,
	pub classes:              Vec<pi_ast::cleanup::Class>,
	pub god_score:            u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneCluster {
	pub duplicated_sig_lines: usize,
	pub regions:              Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct JunkDrawer {
	pub file:    String,
	pub loc:     usize,
	pub exports: usize,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadExport {
	pub file:      String,
	pub name:      String,
	pub tier:      &'static str,
	pub test_only: bool,
}
#[derive(Debug, Serialize)]
pub struct DeepImport {
	pub file:  String,
	pub count: usize,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckDensity {
	pub file:   String,
	pub loc:    usize,
	pub per100: f64,
	#[serde(flatten)]
	pub checks: Checks,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
	pub version:       &'static str,
	pub packages:      Vec<String>,
	pub scanned_files: usize,
	pub corpus_files:  usize,
	pub diagnostics:   Vec<String>,
	pub god_objects:   Vec<GodObject>,
	pub clones:        Vec<CloneCluster>,
	pub junk_drawers:  Vec<JunkDrawer>,
	pub dead_exports:  Vec<DeadExport>,
	pub deep_imports:  Vec<DeepImport>,
	pub check_density: Vec<CheckDensity>,
}
pub(super) struct FileInfo {
	rel:       String,
	id:        usize,
	text:      String,
	loc:       usize,
	structure: Structure,
	checks:    Checks,
}
static IDENTIFIERS: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"[A-Za-z_$][A-Za-z0-9_$]*").expect("static identifier index"));
static JUNK: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"(^|[-_.])(utils?|helpers?|misc|common)\.ts$").expect("static junk-name heuristic")
});
static CHECKS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
	let ws = crate::source_text::JS_SPACE;
	[
		format!(r"{ws}as{ws}+(any|unknown|[A-Z][A-Za-z0-9_.]*)"),
		r"\?\.".into(),
		r"\?\?".into(),
		format!(r"(?-u:\b)typeof{ws}+[A-Za-z0-9_$.]+{ws}*[!=]==?"),
		r"Array\.isArray\(".into(),
	]
	.into_iter()
	.map(|pattern| Regex::new(&pattern).expect("static check heuristic"))
	.collect()
});
fn checks(text: &str) -> Checks {
	let counts: Vec<_> = CHECKS
		.iter()
		.map(|regex| regex.find_iter(text).count())
		.collect();
	Checks {
		as_casts:      counts[0],
		opt_chain:     counts[1],
		coalesce:      counts[2],
		typeof_narrow: counts[3],
		is_array:      counts[4],
	}
}
fn excluded_directory(name: &str) -> bool {
	name == "vendor" || name == "node_modules" || name.ends_with("-gen")
}
fn relative(path: &Path) -> Result<String> {
	let parts: Vec<_> = path
		.components()
		.map(|part| {
			part
				.as_os_str()
				.to_str()
				.ok_or_else(|| anyhow::anyhow!("cleanup paths must be UTF-8"))
		})
		.collect::<Result<_>>()?;
	let path = parts.join("/");
	ensure!(path.len() <= 4096, "cleanup path exceeds 4096 bytes");
	Ok(path)
}
fn sources(filesystem: &JailedFs, cancellation: &CancelToken) -> Result<Vec<String>> {
	let roots = filesystem.entries(Path::new(""), 1000)?;
	let mut pending: Vec<_> = roots
		.into_iter()
		.filter(|entry| {
			matches!(entry.name.as_str(), "packages" | "scripts")
				&& entry.is_directory
				&& !entry.is_symlink
		})
		.map(|entry| (PathBuf::from(entry.name), 0))
		.collect();
	let mut result = Vec::new();
	let mut visited = 0;
	let mut path_bytes = 0;
	while let Some((path, depth)) = pending.pop() {
		cancellation.heartbeat()?;
		ensure!(depth <= 64, "cleanup nesting exceeds 64");
		for entry in filesystem.entries(&path, 50_000)? {
			cancellation.heartbeat()?;
			visited += 1;
			ensure!(visited <= 100_000, "cleanup tree exceeds 100000 entries");
			if entry.is_symlink || excluded_directory(&entry.name) {
				continue;
			}
			let child = path.join(&entry.name);
			if entry.is_directory {
				pending.push((child, depth + 1));
			} else if entry.is_file && child.extension().is_some_and(|ext| ext == "ts") {
				let path = relative(&child)?;
				path_bytes += path.len();
				ensure!(path_bytes <= 4 * 1024 * 1024, "cleanup paths exceed 4 MiB");
				result.push(path);
				ensure!(result.len() <= 20_000, "cleanup corpus exceeds 20000 files");
			}
		}
	}
	result.sort();
	Ok(result)
}
fn selected_packages(filesystem: &JailedFs, options: &Options) -> Result<Vec<String>> {
	ensure!(
		!(options.all_packages && options.packages.is_some()),
		"all packages and explicit packages are mutually exclusive"
	);
	let entries = filesystem.entries(Path::new("packages"), 1000)?;
	let available: BTreeSet<_> = entries
		.into_iter()
		.filter(|e| e.is_directory && !e.is_symlink && !excluded_directory(&e.name))
		.map(|e| e.name)
		.collect();
	let names = if options.all_packages {
		available.iter().cloned().collect()
	} else {
		options.packages.clone().unwrap_or_else(|| {
			["coding-agent", "ai", "catalog", "utils"]
				.map(str::to_owned)
				.to_vec()
		})
	};
	ensure!(!names.is_empty() && names.len() <= 256, "select 1..256 packages");
	let mut unique = BTreeSet::new();
	for name in &names {
		ensure!(
			available.contains(name) && unique.insert(name.clone()),
			"unknown, duplicate or unsafe package selection"
		);
	}
	Ok(names)
}
fn resolve(from: &str, spec: &str, known: &BTreeMap<String, usize>) -> Option<String> {
	let from = Path::new(from).parent()?.join(spec.replace('\\', "/"));
	let mut components = Vec::new();
	for part in from.components() {
		match part {
			Component::Normal(value) => components.push(value.to_str()?),
			Component::CurDir => {},
			Component::ParentDir => {
				components.pop()?;
			},
			_ => return None,
		}
	}
	let base = components.join("/");
	[base.clone(), format!("{base}.ts"), format!("{base}/index.ts")]
		.into_iter()
		.find(|name| known.contains_key(name))
}
fn surface(
	filesystem: &JailedFs,
	packages: &[String],
	files: &[FileInfo],
	known: &BTreeMap<String, usize>,
	diagnostics: &mut Vec<String>,
	cancellation: &CancelToken,
) -> Result<(BTreeSet<String>, BTreeMap<String, BTreeSet<String>>)> {
	let mut barrels = BTreeSet::new();
	let mut names: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
	for package in packages {
		cancellation.heartbeat()?;
		let path = format!("packages/{package}/package.json");
		let value = filesystem
			.read(Path::new(&path), 1024 * 1024)
			.and_then(|bytes| Ok(serde_json::from_slice::<serde_json::Value>(&bytes)?));
		let value = match value {
			Ok(value) => value,
			Err(_) => {
				diagnostics.push(format!("{path}: exports metadata unavailable"));
				continue;
			},
		};
		let mut queue = Vec::new();
		if let Some(exports) = value.get("exports").and_then(serde_json::Value::as_object) {
			for (key, value) in exports {
				if key.contains('*') {
					continue;
				}
				let target = value
					.as_str()
					.or_else(|| value.get("import").and_then(serde_json::Value::as_str));
				if let Some(target) = target.filter(|target| target.ends_with(".ts")) {
					if let Some(found) =
						resolve(&format!("packages/{package}/package.json"), target, known)
					{
						queue.push(found);
					}
				}
			}
		}
		while let Some(path) = queue.pop() {
			cancellation.heartbeat()?;
			if !barrels.insert(path.clone()) {
				continue;
			}
			let file = &files[known[&path]];
			for spec in &file.structure.export_star {
				if let Some(target) = resolve(&path, spec, known) {
					queue.push(target);
				}
			}
			for entry in &file.structure.named_reexports {
				if let Some(target) = resolve(&path, &entry.from, known) {
					names.entry(target).or_default().insert(entry.name.clone());
				}
			}
		}
	}
	Ok((barrels, names))
}
pub fn scan(workspace: &Path, options: &Options, cancellation: &CancelToken) -> Result<Report> {
	ensure!((1..=1000).contains(&options.top), "top must be 1..1000");
	let filesystem = JailedFs::open(workspace)?;
	let packages = selected_packages(&filesystem, options)?;
	let paths = sources(&filesystem, cancellation)?;
	let corpus_files = paths.len();
	let mut identifiers: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
	let mut test_files = BTreeSet::new();
	let mut files = Vec::new();
	let mut known = BTreeMap::new();
	let mut bytes = 0;
	let mut refs = 0;
	let mut diagnostics = Vec::new();
	for (id, path) in paths.iter().enumerate() {
		cancellation.heartbeat()?;
		let raw = filesystem.read(Path::new(path), 4 * 1024 * 1024)?;
		bytes += raw.len();
		ensure!(bytes <= 128 * 1024 * 1024, "cleanup corpus exceeds 128 MiB");
		let text = String::from_utf8_lossy(&raw).into_owned();
		if std::str::from_utf8(&raw).is_err() {
			diagnostics.push(format!("{path}: invalid UTF-8 replaced for source analysis"));
		}
		if path.ends_with(".test.ts") || path.contains("__tests__/") {
			test_files.insert(id);
		}
		let names: BTreeSet<_> = IDENTIFIERS
			.find_iter(&text)
			.map(|found| found.as_str())
			.filter(|name| name.len() >= 3)
			.collect();
		for name in names {
			refs += 1;
			ensure!(refs <= 4_000_000, "cleanup reference index exceeds four million pairs");
			identifiers.entry(name.into()).or_default().insert(id);
		}
		ensure!(identifiers.len() <= 500_000, "cleanup reference vocabulary exceeds 500000");
		let in_scope = packages
			.iter()
			.any(|package| path.starts_with(&format!("packages/{package}/src/")))
			&& !path.ends_with(".d.ts")
			&& !path.ends_with(".test.ts")
			&& !["__tests__/", "/fixtures/", "/snapshots/"]
				.iter()
				.any(|part| path.contains(part));
		if !in_scope {
			continue;
		}
		let structure = pi_ast::cleanup::inspect(&text, &mut || cancellation.heartbeat())?;
		if structure.recovered {
			diagnostics.push(format!("{path}: parser recovery used; confirm structural candidates"));
		}
		let loc = text
			.split('\n')
			.filter(|line| !pi_edit::text::js_trim(line).is_empty())
			.count();
		let checks = checks(&text);
		known.insert(path.clone(), files.len());
		files.push(FileInfo { rel: path.clone(), id, text, loc, structure, checks });
	}
	let (barrels, names) =
		surface(&filesystem, &packages, &files, &known, &mut diagnostics, cancellation)?;
	let mut god_files: Vec<_> = files.iter().filter(|file| file.loc >= 800).collect();
	god_files.sort_by(|a, b| b.loc.cmp(&a.loc).then_with(|| a.rel.cmp(&b.rel)));
	god_files.truncate(options.top);
	let mut god_objects: Vec<_> = god_files
		.iter()
		.map(|file| {
			let mut classes: Vec<_> = file
				.structure
				.classes
				.iter()
				.filter(|class| class.methods >= 10)
				.map(|class| pi_ast::cleanup::Class {
					name:    class.name.clone(),
					methods: class.methods,
				})
				.collect();
			classes.sort_by(|a, b| b.methods.cmp(&a.methods).then_with(|| a.name.cmp(&b.name)));
			let maximum = file
				.structure
				.classes
				.iter()
				.map(|class| class.methods)
				.max()
				.unwrap_or(0);
			GodObject {
				file: file.rel.clone(),
				loc: file.loc,
				exports: file.structure.exported_names.len(),
				top_level_statements: file.structure.top_level_statements,
				classes,
				god_score: (file.loc as f64
					* (1.0 + file.structure.exported_names.len() as f64 / 20.0 + maximum as f64 / 30.0))
					.round() as u64,
			}
		})
		.collect();
	god_objects.sort_by(|a, b| {
		b.god_score
			.cmp(&a.god_score)
			.then_with(|| a.file.cmp(&b.file))
	});
	let mut clones = clones::detect(&files, cancellation)?;
	clones.truncate(options.top);
	let mut junk_drawers: Vec<_> = files
		.iter()
		.filter(|file| {
			JUNK.is_match(&file.rel) || file.rel.contains("/utils/") || file.rel.contains("/helpers/")
		})
		.map(|file| JunkDrawer {
			file:    file.rel.clone(),
			loc:     file.loc,
			exports: file.structure.exported_names.len(),
		})
		.collect();
	junk_drawers.sort_by(|a, b| b.loc.cmp(&a.loc).then_with(|| a.file.cmp(&b.file)));
	junk_drawers.truncate(options.top);
	let mut dead_exports = Vec::new();
	for file in &files {
		cancellation.heartbeat()?;
		for name in &file.structure.exported_names {
			let Some(references) = identifiers.get(name) else {
				continue;
			};
			let others: Vec<_> = references.iter().filter(|id| **id != file.id).collect();
			if others.iter().any(|id| !test_files.contains(id)) {
				continue;
			}
			ensure!(dead_exports.len() < 100_000, "cleanup export candidates exceed 100000");
			dead_exports.push(DeadExport {
				file:      file.rel.clone(),
				name:      name.clone(),
				tier:      if barrels.contains(&file.rel)
					|| names
						.get(&file.rel)
						.is_some_and(|names| names.contains(name))
				{
					"barrel-public"
				} else {
					"wildcard-only"
				},
				test_only: !others.is_empty(),
			});
		}
	}
	dead_exports.sort_by(|a, b| a.file.cmp(&b.file).then_with(|| a.name.cmp(&b.name)));
	let mut deep_imports: Vec<_> = files
		.iter()
		.filter(|file| file.structure.deep_imports > 0)
		.map(|file| DeepImport { file: file.rel.clone(), count: file.structure.deep_imports })
		.collect();
	deep_imports.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.file.cmp(&b.file)));
	deep_imports.truncate(options.top);
	let scanned_files = files.len();
	let mut check_density = Vec::new();
	for file in files.into_iter().filter(|file| file.loc >= 150) {
		let c = &file.checks;
		let total = c.as_casts + c.opt_chain + c.coalesce + c.typeof_narrow + c.is_array;
		check_density.push(CheckDensity {
			file:   file.rel,
			loc:    file.loc,
			per100: ((total as f64 / file.loc as f64) * 1000.0).round() / 10.0,
			checks: file.checks,
		});
	}
	check_density.sort_by(|a, b| {
		b.per100
			.total_cmp(&a.per100)
			.then_with(|| a.file.cmp(&b.file))
	});
	check_density.truncate(options.top);
	Ok(Report {
		version: VERSION,
		packages,
		scanned_files,
		corpus_files,
		diagnostics,
		god_objects,
		clones,
		junk_drawers,
		dead_exports,
		deep_imports,
		check_density,
	})
}
pub(crate) fn execute(arguments: Arguments) -> Result<u8> {
	let options = Options {
		all_packages: arguments.packages.as_deref() == Some("all"),
		packages:     arguments
			.packages
			.filter(|value| value != "all")
			.map(|value| value.split(',').map(str::to_owned).collect()),
		top:          usize::from(arguments.top),
	};
	let report = scan(&arguments.workspace, &options, &Default::default())?;
	let mut bytes = harvest_runtime::limits::BoundedWriter::new(16 * 1024 * 1024);
	if arguments.json {
		serde_json::to_writer_pretty(&mut bytes, &report)?;
		writeln!(&mut bytes)?;
	} else {
		report.write_text(&mut bytes)?;
	}
	io::stdout().lock().write_all(&bytes.into_inner())?;
	Ok(0)
}
