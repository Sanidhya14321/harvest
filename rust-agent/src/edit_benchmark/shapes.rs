//! Edit-shape calibration from inert session records. Request/file totals are
//! cumulative activity (including retries), never reconstructed net edits.
use std::{
	collections::BTreeMap,
	io::{BufRead, BufReader, Read, Write},
	path::{Path, PathBuf},
};

use anyhow::{Result, ensure};
use harvest_runtime::{JailedFs, MAX_EVENT_BYTES, cancel::CancelToken};
use serde::Serialize;
use serde_json::Value;

pub(super) const SIZE_BANDS: [(&str, usize, usize); 5] =
	[("1", 1, 1), ("2-5", 2, 5), ("6-20", 6, 20), ("21-60", 21, 60), ("61+", 61, usize::MAX)];
const MAX_SAMPLES: usize = 100_000;
const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ChangeRun {
	pub old_count: usize,
	pub new_count: usize,
}
#[derive(Default)]
struct Collector {
	runs:    Vec<ChangeRun>,
	current: ChangeRun,
}
impl Collector {
	fn flush(&mut self) {
		if self.current.old_count + self.current.new_count > 0 {
			self.runs.push(std::mem::take(&mut self.current));
		}
	}

	fn mark(&mut self, kind: u8) {
		match kind {
			b'-' => self.current.old_count += 1,
			b'+' => self.current.new_count += 1,
			_ => self.flush(),
		}
	}
}
fn numbered_kind(line: &str, allow_space: bool) -> Option<u8> {
	let kind = *line.as_bytes().first()?;
	if !matches!(kind, b' ' | b'+' | b'-') {
		return None;
	}
	let rest = line[1..].trim_start();
	let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
	if digits == 0 {
		return None;
	}
	let suffix = &rest[digits..];
	(suffix.starts_with('|') || (allow_space && (suffix.starts_with(' ') || suffix.is_empty())))
		.then_some(kind)
}
/// Returns None for any unrecognized row, so format skew stays in coverage.
pub fn change_runs(diff: &str) -> Option<(&'static str, Vec<ChangeRun>)> {
	if diff.len() > MAX_EVENT_BYTES {
		return None;
	}
	let unified = diff.trim_start().starts_with("@@") || diff.trim_start().starts_with("--- ");
	let pipe = diff
		.split('\n')
		.take(10)
		.any(|line| numbered_kind(line, false).is_some());
	let mut collector = Collector::default();
	for line in diff.split('\n') {
		if unified {
			if line.starts_with("@@") {
				collector.flush();
			} else if line.starts_with("--- ")
				|| line.starts_with("+++ ")
				|| line.starts_with("Index:")
			{
			} else if line.starts_with('-') || line.starts_with('+') {
				collector.mark(line.as_bytes()[0]);
			} else if line.starts_with(' ') || line.is_empty() {
				collector.flush();
			} else {
				return None;
			}
		} else if let Some(kind) = numbered_kind(line, !pipe) {
			collector.mark(kind);
		} else if matches!(line.trim(), "" | "..." | "…") || line.trim().ends_with("|...") {
			collector.flush();
		} else {
			return None;
		}
	}
	collector.flush();
	(!collector.runs.is_empty())
		.then_some((if unified { "unified" } else { "numbered" }, collector.runs))
}
#[derive(Debug, Default, Serialize)]
pub struct Distribution {
	pub samples:              usize,
	pub median_changed_lines: usize,
	pub sizes:                BTreeMap<&'static str, usize>,
	pub hunks:                BTreeMap<&'static str, usize>,
	pub operations:           BTreeMap<&'static str, usize>,
	#[serde(skip)]
	changed:                  Vec<usize>,
}
impl Distribution {
	fn accumulate(&mut self, runs: &[ChangeRun]) -> Result<()> {
		ensure!(self.changed.len() < MAX_SAMPLES, "edit-shape sample limit exceeded");
		let changed = runs.iter().map(|r| r.old_count.max(r.new_count)).sum();
		self.changed.push(changed);
		self.samples += 1;
		let size = SIZE_BANDS
			.iter()
			.find(|(_, _, max)| changed <= *max)
			.expect("unbounded final size band")
			.0;
		*self.sizes.entry(size).or_default() += 1;
		*self
			.hunks
			.entry(match runs.len() {
				1 => "1",
				2 => "2",
				_ => "3+",
			})
			.or_default() += 1;
		for run in runs {
			*self
				.operations
				.entry(if run.old_count == 0 {
					"insert"
				} else if run.new_count == 0 {
					"delete"
				} else {
					"replace"
				})
				.or_default() += 1;
		}
		Ok(())
	}

	fn finish(&mut self) {
		self.changed.sort_unstable();
		self.median_changed_lines = self
			.changed
			.get(self.changed.len() / 2)
			.copied()
			.unwrap_or(0);
	}
}
#[derive(Debug, Default, Serialize)]
pub struct ShapeReport {
	pub session_files:     usize,
	pub failed_files:      Vec<PathBuf>,
	pub malformed_records: usize,
	pub skipped_diffs:     usize,
	pub missing_diffs:     usize,
	pub unchanged_edits:   usize,
	pub parsed_diffs:      usize,
	pub formats:           BTreeMap<&'static str, usize>,
	pub per_call:          Distribution,
	pub per_request_file:  Distribution,
	#[serde(skip)]
	pending_runs:          usize,
}
fn call_path(args: &Value) -> Option<String> {
	if let Some(path) = args["path"].as_str().filter(|s| !s.is_empty()) {
		return Some(path.into());
	}
	if let Some(input) = args["input"].as_str() {
		let head = input
			.strip_prefix('¶')
			.and_then(|s| s.split_once('#').map(|v| v.0))
			.or_else(|| {
				input
					.strip_prefix('[')
					.and_then(|s| s.split_once('#').map(|v| v.0))
					.filter(|s| !s.contains(']'))
			})
			.or_else(|| input.lines().find_map(|l| l.strip_prefix("@@ ")));
		if let Some(path) = head.filter(|s| !s.contains('\n')) {
			return Some(path.trim().into());
		}
	}
	args["edits"]
		.get(0)
		.and_then(|v| v["path"].as_str())
		.filter(|s| !s.is_empty())
		.map(str::to_owned)
}
fn flush(groups: &mut BTreeMap<String, Vec<ChangeRun>>, report: &mut ShapeReport) -> Result<()> {
	report.pending_runs = 0;
	for runs in std::mem::take(groups).values() {
		if !runs.is_empty() {
			report.per_request_file.accumulate(runs)?;
		}
	}
	Ok(())
}
fn handle_record(
	record: Value,
	paths: &mut BTreeMap<String, String>,
	groups: &mut BTreeMap<String, Vec<ChangeRun>>,
	report: &mut ShapeReport,
) -> Result<()> {
	if record["type"] != "message" {
		return Ok(());
	}
	let message = &record["message"];
	match message["role"].as_str() {
		Some("user") => flush(groups, report)?,
		Some("assistant") => {
			if let Some(blocks) = message["content"].as_array() {
				for call in blocks {
					if call["type"] != "toolCall"
						|| !matches!(call["name"].as_str(), Some("edit" | "apply_patch"))
					{
						continue;
					}
					if let (Some(id), Some(path)) = (call["id"].as_str(), call_path(&call["arguments"]))
					{
						ensure!(paths.len() < MAX_SAMPLES, "edit-shape call identity limit exceeded");
						ensure!(
							id.len() <= 256 && path.len() <= 4096,
							"edit-shape identity exceeds byte limit"
						);
						paths.insert(id.into(), path);
					}
				}
			}
		},
		Some("toolResult")
			if matches!(message["toolName"].as_str(), Some("edit" | "apply_patch"))
				&& message["isError"] != true =>
		{
			let details = &message["details"];
			if details["noop"] == true {
				report.unchanged_edits += 1;
				return Ok(());
			}
			let Some(diff) = details["diff"].as_str().filter(|s| !s.is_empty()) else {
				report.missing_diffs += 1;
				return Ok(());
			};
			if details["op"]
				.as_str()
				.is_some_and(|s| s != "update" && !s.is_empty())
			{
				return Ok(());
			}
			let Some((format, runs)) = change_runs(diff) else {
				report.skipped_diffs += 1;
				return Ok(());
			};
			report.per_call.accumulate(&runs)?;
			report.parsed_diffs += 1;
			*report.formats.entry(format).or_default() += 1;
			let id = message["toolCallId"].as_str();
			let key = id
				.and_then(|id| paths.get(id).cloned())
				.map(|path| format!("file:{path}"))
				.unwrap_or_else(|| {
					id.map_or_else(|| format!("call:{}", report.parsed_diffs), |id| format!("call:{id}"))
				});
			ensure!(
				report.pending_runs + runs.len() <= MAX_SAMPLES,
				"request run count exceeds limit"
			);
			report.pending_runs += runs.len();
			let group = groups.entry(key).or_default();
			ensure!(group.len() + runs.len() <= MAX_SAMPLES, "request/file run limit exceeded");
			group.extend(runs);
		},
		_ => {},
	}
	Ok(())
}
/// Sorted newest-name-first selection matches legacy session naming. Zero means
/// all files, still bounded by the directory and total scan budgets.
pub fn analyze_shapes(
	directory: &Path,
	max_sessions: usize,
	cancellation: &CancelToken,
) -> Result<ShapeReport> {
	ensure!(max_sessions <= 10_000, "max sessions exceeds 10000");
	let fs = JailedFs::open(directory)?;
	let mut files: Vec<_> = fs
		.entries(Path::new(""), 100_000)?
		.into_iter()
		.filter(|e| e.is_file && !e.is_symlink && e.name.ends_with(".jsonl"))
		.collect();
	files.sort_by(|a, b| b.name.cmp(&a.name));
	files.truncate(if max_sessions == 0 {
		10_000
	} else {
		max_sessions
	});
	let mut report = ShapeReport { session_files: files.len(), ..Default::default() };
	let mut total = 0_u64;
	for entry in files {
		cancellation.heartbeat()?;
		let path = PathBuf::from(entry.name);
		ensure!(total <= 1024 * 1024 * 1024, "session scan exceeds 1 GiB limit");
		let mut paths = BTreeMap::new();
		let mut groups = BTreeMap::new();
		let scan = (|| -> Result<()> {
			let mut input = BufReader::new(fs.open_read(&path, MAX_SESSION_BYTES)?);
			let mut bytes_read = 0_usize;
			loop {
				cancellation.heartbeat()?;
				let mut bytes = Vec::new();
				let count = (&mut input)
					.take((MAX_EVENT_BYTES + 1) as u64)
					.read_until(b'\n', &mut bytes)?;
				if count == 0 {
					break;
				}
				bytes_read += count;
				total += count as u64;
				ensure!(total <= 1024 * 1024 * 1024, "session scan exceeds 1 GiB limit");
				ensure!(
					count <= MAX_EVENT_BYTES && bytes_read <= MAX_SESSION_BYTES as usize,
					"session record or file exceeds byte limit"
				);
				if bytes.iter().all(u8::is_ascii_whitespace) {
					continue;
				}
				match serde_json::from_slice(&bytes) {
					Ok(record) => handle_record(record, &mut paths, &mut groups, &mut report)?,
					Err(_) => report.malformed_records += 1,
				}
			}
			Ok(())
		})();
		cancellation.heartbeat()?;
		ensure!(total <= 1024 * 1024 * 1024, "session scan exceeds 1 GiB limit");
		if scan.is_err() {
			report.failed_files.push(path);
		}
		flush(&mut groups, &mut report)?;
	}
	report.per_call.finish();
	report.per_request_file.finish();
	Ok(report)
}
impl ShapeReport {
	pub fn write_text(&self, output: &mut impl Write) -> Result<()> {
		writeln!(output, "Scanning {} session files", self.session_files)?;
		let total = self.parsed_diffs + self.skipped_diffs;
		let percent = if total == 0 {
			0.0
		} else {
			self.parsed_diffs as f64 / total as f64 * 100.0
		};
		writeln!(
			output,
			"Parse coverage: {}/{} diff results ({percent:.1}%; {:?})",
			self.parsed_diffs, total, self.formats
		)?;
		writeln!(
			output,
			"Failed files: {}; malformed records: {}; successful edit results without diff: {}; \
			 unchanged edits: {}",
			self.failed_files.len(),
			self.malformed_records,
			self.missing_diffs,
			self.unchanged_edits
		)?;
		for (label, counters) in [
			("per tool call (calibration reference)", &self.per_call),
			("per request × file (cumulative activity — upper bound)", &self.per_request_file),
		] {
			writeln!(
				output,
				"\n=== {label}: {} samples, median changed lines: {}",
				counters.samples, counters.median_changed_lines
			)?;
			for (title, counts, order) in [
				("changed lines:", &counters.sizes, &["1", "2-5", "6-20", "21-60", "61+"][..]),
				("hunks:", &counters.hunks, &["1", "2", "3+"][..]),
				("op mix:", &counters.operations, &["replace", "insert", "delete"][..]),
			] {
				writeln!(output, "{title}")?;
				let total: usize = counts.values().sum();
				for key in order {
					let count = counts.get(key).copied().unwrap_or(0);
					let percent = if total == 0 {
						0.0
					} else {
						count as f64 / total as f64 * 100.0
					};
					writeln!(output, "  {key:<6} {count:>6}  {percent:.1}%")?;
				}
			}
		}
		Ok(())
	}
}

/// Read-only discovery for the historical utility's --repo default. Unlike the
/// application opener, analysis never renames, merges or creates session roots.
/// Ambiguous historical layouts require an explicit --sessions-dir.
pub fn discover_sessions(repo: &Path) -> Result<PathBuf> {
	let repo = repo.canonicalize()?;
	let home_path = std::env::var_os("HOME")
		.or_else(|| std::env::var_os("USERPROFILE"))
		.map(PathBuf::from)
		.ok_or_else(|| anyhow::anyhow!("cannot locate home; pass --sessions-dir"))?;
	let nonempty = |key: &str| std::env::var(key).ok().filter(|v| !v.is_empty());
	let config = nonempty("HARVEST_CONFIG_DIR")
		.or_else(|| nonempty("PI_CONFIG_DIR"))
		.unwrap_or_else(|| ".harvest".into());
	let profile = ["HARVEST_PROFILE", "OMP_PROFILE", "PI_PROFILE"]
		.into_iter()
		.find_map(|key| std::env::var(key).ok())
		.unwrap_or_default();
	let profile = profile.trim();
	let profile = (!profile.is_empty() && profile != "default").then_some(profile);
	if let Some(profile) = profile {
		ensure!(
			profile.len() <= 64
				&& (profile.as_bytes()[0].is_ascii_lowercase()
					|| profile.as_bytes()[0].is_ascii_digit()),
			"invalid profile; pass --sessions-dir"
		);
		ensure!(
			profile.bytes().all(|b| b.is_ascii_lowercase()
				|| b.is_ascii_digit()
				|| matches!(b, b'.' | b'_' | b'-'))
				&& !profile.ends_with('.'),
			"invalid profile; pass --sessions-dir"
		);
	}
	let config_root = match profile {
		Some(profile) => home_path.join(config).join("profiles").join(profile),
		None => home_path.join(config),
	};
	let default_agent = config_root.join("agent");
	let agent = if profile.is_some() {
		default_agent.clone()
	} else {
		nonempty("HARVEST_CODING_AGENT_DIR")
			.or_else(|| nonempty("PI_CODING_AGENT_DIR"))
			.map(PathBuf::from)
			.unwrap_or_else(|| default_agent.clone())
	};
	let xdg = if cfg!(any(target_os = "linux", target_os = "macos")) && agent == default_agent {
		nonempty("XDG_DATA_HOME")
			.map(|p| {
				let base = PathBuf::from(p).join("harvest");
				match profile {
					Some(profile) => base.join("profiles").join(profile),
					None => base,
				}
			})
			.filter(|p| p.exists())
	} else {
		None
	};
	let root = xdg.unwrap_or(agent).join("sessions");
	let encode = |path: &Path| -> Result<String> {
		Ok(path
			.to_str()
			.ok_or_else(|| anyhow::anyhow!("session path is not UTF-8"))?
			.replace(['/', '\\', ':'], "-"))
	};
	let home = std::env::var_os("HOME")
		.or_else(|| std::env::var_os("USERPROFILE"))
		.and_then(|p| PathBuf::from(p).canonicalize().ok());
	let temp = std::env::temp_dir().canonicalize().ok();
	let (scope, name) = if let Some(relative) = home.as_ref().and_then(|p| repo.strip_prefix(p).ok())
	{
		("home", format!("-{}", encode(relative)?))
	} else if let Some(relative) = temp.as_ref().and_then(|p| repo.strip_prefix(p).ok()) {
		let encoded = encode(relative)?;
		(
			"tmp",
			if encoded.is_empty() {
				"-tmp".into()
			} else {
				format!("-tmp-{encoded}")
			},
		)
	} else {
		let text = repo
			.to_str()
			.ok_or_else(|| anyhow::anyhow!("session path is not UTF-8"))?;
		(
			"abs",
			format!(
				"--{}--",
				text
					.trim_start_matches(['/', '\\'])
					.replace(['/', '\\', ':'], "-")
			),
		)
	};
	let normalized = repo
		.to_str()
		.ok_or_else(|| anyhow::anyhow!("session path is not UTF-8"))?
		.replace('\\', "/");
	let readable = repo
		.file_name()
		.and_then(|p| p.to_str())
		.unwrap_or("project");
	let mut slug = String::new();
	for c in readable.chars() {
		if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
			slug.push(c);
		} else if !slug.ends_with('-') {
			slug.push('-');
		}
	}
	let slug = slug.trim_matches('-');
	let slug = if slug.is_empty() {
		"project"
	} else {
		&slug[slug.len().saturating_sub(80)..]
	};
	let hashed = format!("{scope}-{slug}-{}", harvest_runtime::content_hash(normalized.as_bytes()));
	let text = repo
		.to_str()
		.ok_or_else(|| anyhow::anyhow!("session path is not UTF-8"))?;
	let legacy = format!(
		"--{}--",
		text
			.trim_start_matches(['/', '\\'])
			.replace(['/', '\\', ':'], "-")
	);
	let mut candidates = [root.join(name), root.join(legacy), root.join(hashed)].to_vec();
	candidates.sort();
	candidates.dedup();
	candidates.retain(|p| p.is_dir());
	ensure!(
		candidates.len() == 1,
		"session directory missing or ambiguous; pass --sessions-dir (analysis never migrates \
		 stored data)"
	);
	Ok(candidates.remove(0))
}
