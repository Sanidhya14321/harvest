//! `harvest gc` — on-disk storage maintenance (port of
//! `packages/coding-agent/src/cli/gc-cli.ts`). Dry-run unless `--apply`.
//!
//! Not ported: stats.db row reconciliation for archived sessions
//! (`statsRowsDeleted` is always 0).

use std::{
	collections::{BTreeSet, HashMap},
	fs,
	io::{Read, Write},
	path::{Path, PathBuf},
	time::UNIX_EPOCH,
};

use anyhow::{Context, Result, bail};
use clap::Args;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use harvest_runtime::JailedFs;
use regex::Regex;
use serde::Serialize;

use super::paths;

const DAY_MS: f64 = 86_400_000.0;
const GC_WRITE_GRACE_MS: f64 = 5.0 * 60_000.0;
const SESSION_SUFFIX: &str = ".jsonl";
const COMPRESSED_SESSION_SUFFIX: &str = ".jsonl.gz";
const PREFIX_BYTES: u64 = 4096;
const SUFFIX_BYTES: u64 = 32_768;

#[derive(Debug, Args)]
pub struct GcArgs {
	/// Apply changes (default is dry-run)
	#[arg(long)]
	apply:                   bool,
	/// Output JSON
	#[arg(long)]
	json:                    bool,
	/// Agent directory to maintain
	#[arg(long = "agent-dir")]
	agent_dir:               Option<PathBuf>,
	/// Sweep unreferenced blobs
	#[arg(long)]
	blobs:                   bool,
	/// Archive cold sessions
	#[arg(long)]
	archive:                 bool,
	/// Checkpoint history/model database WAL files
	#[arg(long)]
	wal:                     bool,
	/// Minimum session age before archiving
	#[arg(long = "cold-archive-after-days")]
	cold_archive_after_days: Option<i64>,
	/// Always keep this many newest sessions active
	#[arg(long = "retain-newest-global")]
	retain_newest_global:    Option<i64>,
	/// Always keep this many newest sessions per cwd active
	#[arg(long = "retain-newest-per-cwd")]
	retain_newest_per_cwd:   Option<i64>,
}

struct Options {
	apply:                   bool,
	json:                    bool,
	agent_dir:               PathBuf,
	run_blobs:               bool,
	run_archive:             bool,
	run_wal:                 bool,
	cold_archive_after_days: u64,
	retain_newest_global:    u64,
	retain_newest_per_cwd:   u64,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct BlobGcResult {
	referenced:   usize,
	candidates:   usize,
	would_delete: usize,
	deleted:      usize,
	bytes:        u64,
	errors:       Vec<String>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ArchiveGcResult {
	scanned:              usize,
	skipped_active:       usize,
	kept_newest_global:   usize,
	kept_newest_per_cwd:  usize,
	would_archive:        usize,
	archived:             usize,
	history_rows_deleted: u64,
	stats_rows_deleted:   u64,
	fts_rebuilt:          bool,
	errors:               Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WalCheckpointResult {
	db_path:             String,
	wal_bytes:           u64,
	would_checkpoint:    bool,
	checkpointed:        bool,
	busy:                i64,
	log:                 i64,
	checkpointed_frames: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WalGcResult {
	databases:        Vec<WalCheckpointResult>,
	wal_bytes:        u64,
	would_checkpoint: bool,
	checkpointed:     bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GcResult {
	agent_dir: String,
	apply:     bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	blobs:     Option<BlobGcResult>,
	#[serde(skip_serializing_if = "Option::is_none")]
	archive:   Option<ArchiveGcResult>,
	#[serde(skip_serializing_if = "Option::is_none")]
	wal:       Option<WalGcResult>,
	lock_path: String,
}

fn clamp_count(value: Option<i64>, setting: Option<f64>, default: u64) -> u64 {
	if let Some(value) = value {
		return value.max(0) as u64;
	}
	match setting {
		Some(number) if number.is_finite() => number.max(0.0).floor() as u64,
		_ => default,
	}
}

fn resolve_options(args: &GcArgs) -> Result<Options> {
	let explicit = args.agent_dir.clone();
	let agent_dir = crate::config::agent_dir_from(explicit, |key| std::env::var_os(key))
		.context("cannot resolve the agent directory")?;
	let agent_dir = if agent_dir.is_absolute() {
		agent_dir
	} else {
		std::env::current_dir()?.join(agent_dir)
	};
	let selected = args.blobs || args.archive || args.wal;
	let needs_archive_settings = selected
		&& args.archive
		&& (args.cold_archive_after_days.is_none()
			|| args.retain_newest_global.is_none()
			|| args.retain_newest_per_cwd.is_none());
	let settings = if !selected || needs_archive_settings {
		crate::settings::Settings::load(Some(&agent_dir), &std::env::current_dir()?, &[]).ok()
	} else {
		None
	};
	let boolean = |key: &str| {
		settings
			.as_ref()
			.and_then(|s| s.get(key))
			.and_then(|v| v.as_bool())
			.unwrap_or(true)
	};
	let number = |key: &str| settings.as_ref().and_then(|s| s.number(key));
	Ok(Options {
		apply: args.apply,
		json: args.json,
		run_blobs: if selected {
			args.blobs
		} else {
			boolean("gc.blobs")
		},
		run_archive: if selected {
			args.archive
		} else {
			boolean("gc.archive")
		},
		run_wal: if selected {
			args.wal
		} else {
			boolean("gc.wal")
		},
		cold_archive_after_days: clamp_count(
			args.cold_archive_after_days,
			number("gc.coldArchiveAfterDays"),
			30,
		),
		retain_newest_global: clamp_count(
			args.retain_newest_global,
			number("gc.retainNewestGlobal"),
			20,
		),
		retain_newest_per_cwd: clamp_count(
			args.retain_newest_per_cwd,
			number("gc.retainNewestPerCwd"),
			10,
		),
		agent_dir,
	})
}

pub fn execute(args: GcArgs) -> Result<u8> {
	let options = resolve_options(&args)?;
	let archive_root = options.agent_dir.join("archive").join("sessions");
	fs::create_dir_all(&options.agent_dir)?;
	let jail = JailedFs::open(&options.agent_dir)?;
	let lock_path = options.agent_dir.join("gc.lock");
	acquire_lock(&jail, &lock_path)?;
	let run = (|| -> Result<GcResult> {
		let mut result = GcResult {
			agent_dir: options.agent_dir.to_string_lossy().into_owned(),
			apply:     options.apply,
			blobs:     None,
			archive:   None,
			wal:       None,
			lock_path: lock_path.to_string_lossy().into_owned(),
		};
		if options.run_blobs {
			result.blobs = Some(run_blob_gc(&options, &jail, &archive_root)?);
		}
		if options.run_archive {
			result.archive = Some(run_archive_gc(&options, &jail, &archive_root)?);
		}
		if options.run_wal {
			result.wal = Some(run_wal_gc(&options)?);
		}
		Ok(result)
	})();
	let released = jail.remove(&lock_path);
	let result = run?;
	if let Err(error) = released
		&& lock_path.exists()
	{
		return Err(error);
	}

	let output = if options.json {
		format!("{}\n", serde_json::to_string_pretty(&result)?)
	} else {
		render_text(&result)
	};
	std::io::stdout().lock().write_all(output.as_bytes())?;

	let errors: Vec<String> = result
		.blobs
		.iter()
		.flat_map(|b| b.errors.iter().map(|e| format!("blobs: {e}")))
		.chain(
			result
				.archive
				.iter()
				.flat_map(|a| a.errors.iter().map(|e| format!("archive: {e}"))),
		)
		.collect();
	if errors.is_empty() {
		return Ok(0);
	}
	let plural = if errors.len() == 1 { "" } else { "s" };
	let list: Vec<String> = errors.iter().map(|e| format!("- {e}")).collect();
	eprintln!("GC completed with {} error{plural}:\n{}", errors.len(), list.join("\n"));
	Ok(1)
}

// ───────────────────────────── lock ─────────────────────────────

fn lock_contents() -> String {
	format!("{}\n{}\n", std::process::id(), chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"))
}

fn lock_is_stale(lock_path: &Path) -> bool {
	let Ok(text) = fs::read_to_string(lock_path) else {
		return false;
	};
	let mut lines = text.lines();
	if let Some(pid) = lines
		.next()
		.and_then(|line| line.trim().parse::<i64>().ok())
		.filter(|pid| *pid > 0)
	{
		return matches!(paths::probe_pid(pid), Err(code) if code == libc::ESRCH || code == libc::EINVAL);
	}
	let created = lines
		.next()
		.and_then(|line| chrono::DateTime::parse_from_rfc3339(line.trim()).ok())
		.map(|time| time.timestamp_millis() as f64)
		.or_else(|| mtime_ms(lock_path));
	created.is_some_and(|created| paths::now_ms() - created > GC_WRITE_GRACE_MS)
}

fn acquire_lock(jail: &JailedFs, lock_path: &Path) -> Result<()> {
	for _attempt in 0..2 {
		if jail
			.atomic_create(lock_path, lock_contents().as_bytes())
			.is_ok()
		{
			return Ok(());
		}
		if !lock_path.exists() {
			continue;
		}
		if !lock_is_stale(lock_path) {
			break;
		}
		jail.remove(lock_path)?;
	}
	bail!("GC already running: {}", lock_path.display())
}

// ───────────────────────────── helpers ─────────────────────────────

fn mtime_ms(path: &Path) -> Option<f64> {
	let modified = fs::metadata(path).ok()?.modified().ok()?;
	Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as f64)
}

/// Recursive file walk (no symlink following), sorted.
fn walk_files(root: &Path, accept: &dyn Fn(&str) -> bool) -> Vec<PathBuf> {
	let mut out = Vec::new();
	let mut stack = vec![root.to_path_buf()];
	while let Some(dir) = stack.pop() {
		let Ok(entries) = fs::read_dir(&dir) else {
			continue;
		};
		for entry in entries.flatten() {
			let Ok(kind) = entry.file_type() else {
				continue;
			};
			let path = entry.path();
			if kind.is_dir() {
				stack.push(path);
			} else if kind.is_file() && accept(&entry.file_name().to_string_lossy()) {
				out.push(path);
			}
		}
	}
	out.sort();
	out
}

fn read_session_text(file: &Path) -> Result<String> {
	let bytes = match fs::read(file) {
		Ok(bytes) => bytes,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
		Err(error) => return Err(error.into()),
	};
	if file.to_string_lossy().ends_with(COMPRESSED_SESSION_SUFFIX) {
		let mut text = String::new();
		GzDecoder::new(bytes.as_slice()).read_to_string(&mut text)?;
		return Ok(text);
	}
	Ok(String::from_utf8_lossy(&bytes).into_owned())
}

// ───────────────────────────── blobs ─────────────────────────────

fn run_blob_gc(options: &Options, jail: &JailedFs, archive_root: &Path) -> Result<BlobGcResult> {
	let blob_dir = options.agent_dir.join("blobs");
	let sessions_root = options.agent_dir.join("sessions");
	let reference = Regex::new(r"(?i)\bblob:sha256:([a-f0-9]{64})\b")?;
	let blob_file = Regex::new(r"^([a-f0-9]{64})(?:\.[A-Za-z0-9][A-Za-z0-9._-]{0,31})?$")?;
	let session_like = |name: &str| {
		name.ends_with(SESSION_SUFFIX)
			|| name.ends_with(COMPRESSED_SESSION_SUFFIX)
			|| (name.contains(".jsonl.") && name.ends_with(".bak"))
	};

	let mut referenced = BTreeSet::new();
	for root in [&sessions_root, &archive_root.to_path_buf()] {
		for file in walk_files(root, &session_like) {
			let text = read_session_text(&file)?;
			for capture in reference.captures_iter(&text) {
				referenced.insert(capture[1].to_ascii_lowercase());
			}
		}
	}

	struct Candidate {
		paths: Vec<PathBuf>,
		bytes: u64,
		mtime: f64,
	}
	let mut candidates: std::collections::BTreeMap<String, Candidate> = Default::default();
	if let Ok(entries) = fs::read_dir(&blob_dir) {
		for entry in entries.flatten() {
			let name = entry.file_name().to_string_lossy().into_owned();
			let Some(hash) = blob_file.captures(&name).map(|c| c[1].to_owned()) else {
				continue;
			};
			let Ok(meta) = fs::metadata(entry.path()) else {
				continue;
			};
			if !meta.is_file() {
				continue;
			}
			let mtime = mtime_ms(&entry.path()).unwrap_or(0.0);
			let candidate =
				candidates
					.entry(hash)
					.or_insert(Candidate { paths: Vec::new(), bytes: 0, mtime });
			candidate.paths.push(entry.path());
			candidate.bytes += meta.len();
			candidate.mtime = candidate.mtime.max(mtime);
		}
	}

	let mut result = BlobGcResult {
		referenced: referenced.len(),
		candidates: candidates.len(),
		..Default::default()
	};
	let delete_before = paths::now_ms() - GC_WRITE_GRACE_MS;
	for (hash, candidate) in &candidates {
		if referenced.contains(hash) || candidate.mtime > delete_before {
			continue;
		}
		result.would_delete += candidate.paths.len();
		result.bytes += candidate.bytes;
		if !options.apply {
			continue;
		}
		for file in &candidate.paths {
			match jail.remove(file) {
				Ok(()) => result.deleted += 1,
				Err(_) if !file.exists() => {},
				Err(error) => result.errors.push(format!("{}: {error:#}", file.display())),
			}
		}
	}
	Ok(result)
}

// ───────────────────────────── archive ─────────────────────────────

struct SessionInfo {
	path:     PathBuf,
	id:       String,
	cwd:      String,
	modified: f64,
	status:   &'static str,
}

fn read_slices(file: &Path) -> Option<(String, String)> {
	use std::io::{Seek, SeekFrom};
	let mut handle = fs::File::open(file).ok()?;
	let size = handle.metadata().ok()?.len();
	let mut prefix = vec![0; PREFIX_BYTES.min(size) as usize];
	handle.read_exact(&mut prefix).ok()?;
	let start = size.saturating_sub(SUFFIX_BYTES);
	handle.seek(SeekFrom::Start(start)).ok()?;
	let mut suffix = Vec::new();
	handle.read_to_end(&mut suffix).ok()?;
	Some((
		String::from_utf8_lossy(&prefix).into_owned(),
		String::from_utf8_lossy(&suffix).into_owned(),
	))
}

/// Session header: first record (after an optional title slot) of type
/// `session`.
fn session_header(text: &str) -> Option<serde_json::Value> {
	let mut saw_title = false;
	for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
		let record: serde_json::Value = serde_json::from_str(line).ok()?;
		let kind = record.get("type").and_then(|v| v.as_str());
		if !saw_title && kind == Some("title") {
			saw_title = true;
			continue;
		}
		let valid = kind == Some("session")
			&& record
				.get("id")
				.and_then(|v| v.as_str())
				.is_some_and(|id| !id.is_empty());
		return valid.then_some(record);
	}
	None
}

fn derive_status(suffix: &str) -> &'static str {
	for line in suffix.split('\n').rev() {
		if !line.starts_with('{') {
			continue;
		}
		let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) else {
			continue;
		};
		if entry.get("type").and_then(|v| v.as_str()) != Some("message") {
			continue;
		}
		let Some(message) = entry.get("message") else {
			continue;
		};
		return match message.get("role").and_then(|v| v.as_str()) {
			Some("assistant") => match message.get("stopReason").and_then(|v| v.as_str()) {
				Some("error") => "error",
				Some("aborted") => "aborted",
				Some("length") => "interrupted",
				_ => {
					let has_tool_call = message
						.get("content")
						.and_then(|c| c.as_array())
						.is_some_and(|blocks| {
							blocks
								.iter()
								.any(|b| b.get("type").and_then(|t| t.as_str()) == Some("toolCall"))
						});
					if has_tool_call {
						"interrupted"
					} else {
						"complete"
					}
				},
			},
			Some("toolResult") => "interrupted",
			Some("user") => "pending",
			_ => "unknown",
		};
	}
	"unknown"
}

fn is_active(status: &str) -> bool {
	matches!(status, "pending" | "interrupted" | "unknown")
}

fn list_sessions_in(dir: &Path) -> Vec<SessionInfo> {
	let Ok(entries) = fs::read_dir(dir) else {
		return Vec::new();
	};
	let mut sessions = Vec::new();
	for entry in entries.flatten() {
		let path = entry.path();
		if !entry.file_type().is_ok_and(|kind| kind.is_file())
			|| !entry
				.file_name()
				.to_string_lossy()
				.ends_with(SESSION_SUFFIX)
		{
			continue;
		}
		let Some((prefix, suffix)) = read_slices(&path) else {
			continue;
		};
		let Some(header) = session_header(&prefix) else {
			continue;
		};
		sessions.push(SessionInfo {
			id: header
				.get("id")
				.and_then(|v| v.as_str())
				.unwrap_or_default()
				.to_owned(),
			cwd: header
				.get("cwd")
				.and_then(|v| v.as_str())
				.unwrap_or_default()
				.to_owned(),
			modified: mtime_ms(&path).unwrap_or(0.0),
			status: derive_status(&suffix),
			path,
		});
	}
	sessions
}

fn sort_newest_first(sessions: &mut [SessionInfo]) {
	sessions.sort_by(|a, b| {
		b.modified
			.total_cmp(&a.modified)
			.then_with(|| b.path.cmp(&a.path))
	});
}

fn artifacts_path(session: &Path) -> PathBuf {
	let text = session.to_string_lossy();
	let stripped = text
		.strip_suffix(COMPRESSED_SESSION_SUFFIX)
		.or_else(|| text.strip_suffix(SESSION_SUFFIX))
		.unwrap_or(&text);
	PathBuf::from(stripped)
}

fn has_live_nested_sessions(session: &SessionInfo, archive_before: f64) -> bool {
	let files = walk_files(&artifacts_path(&session.path), &|name| name.ends_with(SESSION_SUFFIX));
	let dirs: BTreeSet<PathBuf> = files
		.iter()
		.filter_map(|f| f.parent().map(Path::to_path_buf))
		.collect();
	dirs
		.iter()
		.flat_map(|dir| list_sessions_in(dir))
		.any(|nested| is_active(nested.status) || nested.modified > archive_before)
}

fn gzip_session(jail: &JailedFs, source: &Path, destination: &Path) -> Result<()> {
	let bytes = jail.read(source, usize::MAX)?;
	let mut encoder = GzEncoder::new(Vec::new(), Compression::new(9));
	encoder.write_all(&bytes)?;
	let compressed = encoder.finish()?;
	jail.atomic_create(destination, &compressed)?;
	if let Err(error) = jail.remove(source) {
		let _ = jail.remove(destination);
		return Err(error);
	}
	Ok(())
}

fn move_session_with_artifacts(jail: &JailedFs, session: &Path, destination: &Path) -> Result<()> {
	let legacy = PathBuf::from(
		destination
			.to_string_lossy()
			.trim_end_matches(".gz")
			.to_owned(),
	);
	let source_artifacts = artifacts_path(session);
	let dest_artifacts = artifacts_path(destination);
	if destination.exists() {
		bail!("archive destination exists: {}", destination.display());
	}
	if legacy.exists() {
		bail!("archive destination exists: {}", legacy.display());
	}
	if source_artifacts.exists() && dest_artifacts.exists() {
		bail!("archive artifacts destination exists: {}", dest_artifacts.display());
	}
	gzip_session(jail, session, destination)?;
	if source_artifacts.exists() {
		let moved = (|| -> Result<()> {
			let from = jail.resource_path(&source_artifacts)?;
			let to = jail.resource_path(&dest_artifacts)?;
			if let Some(parent) = to.parent() {
				fs::create_dir_all(parent)?;
			}
			fs::rename(&from, &to)?;
			Ok(())
		})();
		if let Err(error) = moved {
			// Roll back the compressed session so the next scan sees the original.
			let restored = read_session_text(destination)
				.and_then(|text| jail.atomic_write(session, text.as_bytes()))
				.and_then(|()| jail.remove(destination));
			let _ = restored;
			return Err(error);
		}
	}
	Ok(())
}

fn run_archive_gc(
	options: &Options,
	jail: &JailedFs,
	archive_root: &Path,
) -> Result<ArchiveGcResult> {
	let sessions_root = options.agent_dir.join("sessions");
	let mut sessions = Vec::new();
	if let Ok(entries) = fs::read_dir(&sessions_root) {
		for entry in entries.flatten() {
			if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
				sessions.extend(list_sessions_in(&entry.path()));
			}
		}
	}
	sort_newest_first(&mut sessions);
	let now = paths::now_ms();
	let cutoff = now - options.cold_archive_after_days as f64 * DAY_MS;
	let archive_before = now - GC_WRITE_GRACE_MS;
	let mut result = ArchiveGcResult { scanned: sessions.len(), ..Default::default() };
	let mut candidates: Vec<(&SessionInfo, PathBuf)> = Vec::new();
	let mut inactive_seen = 0u64;
	let mut seen_by_cwd: HashMap<String, u64> = HashMap::new();

	for session in &sessions {
		if is_active(session.status)
			|| session.modified > archive_before
			|| has_live_nested_sessions(session, archive_before)
		{
			result.skipped_active += 1;
			continue;
		}
		let relative = session.path.strip_prefix(&sessions_root).ok();
		let cwd_key = relative
			.and_then(Path::parent)
			.filter(|parent| !parent.as_os_str().is_empty())
			.map(|parent| parent.to_string_lossy().into_owned())
			.unwrap_or_else(|| {
				if session.cwd.is_empty() {
					".".to_owned()
				} else {
					session.cwd.clone()
				}
			});
		let cwd_seen = *seen_by_cwd.get(&cwd_key).unwrap_or(&0);
		let keep_global = inactive_seen < options.retain_newest_global;
		let keep_per_cwd = cwd_seen < options.retain_newest_per_cwd;
		inactive_seen += 1;
		seen_by_cwd.insert(cwd_key, cwd_seen + 1);
		if keep_global {
			result.kept_newest_global += 1;
			continue;
		}
		if keep_per_cwd {
			result.kept_newest_per_cwd += 1;
			continue;
		}
		if options.cold_archive_after_days > 0 && session.modified > cutoff {
			continue;
		}
		let Some(relative) = relative else { continue };
		if !relative.to_string_lossy().ends_with(SESSION_SUFFIX) {
			continue;
		}
		let destination = archive_root.join(format!("{}.gz", relative.to_string_lossy()));
		candidates.push((session, destination));
	}

	result.would_archive = candidates.len();
	if !options.apply {
		return Ok(result);
	}
	let mut archived_ids = BTreeSet::new();
	for (session, destination) in candidates {
		match move_session_with_artifacts(jail, &session.path, &destination) {
			Ok(()) => {
				result.archived += 1;
				archived_ids.insert(session.id.clone());
			},
			Err(error) => result
				.errors
				.push(format!("{}: {error:#}", session.path.display())),
		}
	}
	cleanup_history_rows(options, archive_root, archived_ids, &mut result);
	Ok(result)
}

fn cleanup_history_rows(
	options: &Options,
	archive_root: &Path,
	mut ids: BTreeSet<String>,
	result: &mut ArchiveGcResult,
) {
	let db_path = options.agent_dir.join("history.db");
	if !db_path.exists() {
		return;
	}
	for file in walk_files(archive_root, &|name| name.ends_with(COMPRESSED_SESSION_SUFFIX)) {
		match read_session_text(&file) {
			Ok(text) => {
				if let Some(id) =
					session_header(&text).and_then(|h| h.get("id")?.as_str().map(str::to_owned))
				{
					ids.insert(id);
				}
			},
			Err(error) => {
				result
					.errors
					.push(format!("history cleanup scan: {error:#}"));
				break;
			},
		}
	}
	match delete_history_rows(&db_path, &ids) {
		Ok((deleted, rebuilt)) => {
			result.history_rows_deleted = deleted;
			result.fts_rebuilt = rebuilt;
		},
		Err(error) => result.errors.push(format!("history cleanup: {error:#}")),
	}
}

fn table_exists(db: &rusqlite::Connection, table: &str) -> Result<bool> {
	Ok(db
		.query_row(
			"SELECT 1 FROM sqlite_master WHERE type IN ('table','view') AND name = ?",
			[table],
			|_| Ok(()),
		)
		.is_ok())
}

fn delete_history_rows(db_path: &Path, ids: &BTreeSet<String>) -> Result<(u64, bool)> {
	if ids.is_empty() {
		return Ok((0, false));
	}
	let mut db = rusqlite::Connection::open(db_path)?;
	db.busy_timeout(std::time::Duration::from_millis(5000))?;
	if !table_exists(&db, "history")? {
		return Ok((0, false));
	}
	let has_session_id = {
		let mut statement = db.prepare("PRAGMA table_info(history)")?;
		let names: Vec<String> = statement
			.query_map([], |row| row.get::<_, String>(1))?
			.flatten()
			.collect();
		names.iter().any(|name| name == "session_id")
	};
	if !has_session_id {
		return Ok((0, false));
	}
	let has_fts = table_exists(&db, "history_fts")?;
	let tx = db.transaction()?;
	let mut deleted = 0u64;
	for id in ids {
		deleted += tx.execute("DELETE FROM history WHERE session_id = ?", [id])? as u64;
	}
	if deleted > 0 && has_fts {
		tx.execute("INSERT INTO history_fts(history_fts) VALUES('rebuild')", [])?;
	}
	tx.commit()?;
	Ok((deleted, deleted > 0 && has_fts))
}

// ───────────────────────────── wal ─────────────────────────────

fn wal_size(db_path: &Path) -> u64 {
	fs::metadata(format!("{}-wal", db_path.display())).map_or(0, |meta| meta.len())
}

fn checkpoint_wal(db_path: &Path, apply: bool) -> Result<WalCheckpointResult> {
	let wal_bytes = wal_size(db_path);
	let mut result = WalCheckpointResult {
		db_path: db_path.to_string_lossy().into_owned(),
		wal_bytes,
		would_checkpoint: wal_bytes > 0,
		checkpointed: false,
		busy: 0,
		log: 0,
		checkpointed_frames: 0,
	};
	if !apply || !db_path.exists() {
		return Ok(result);
	}
	{
		let db = rusqlite::Connection::open(db_path)?;
		db.busy_timeout(std::time::Duration::from_millis(5000))?;
		let (busy, log, frames) = db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
			Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?))
		})?;
		result.busy = busy;
		result.log = log;
		result.checkpointed_frames = frames;
	}
	result.wal_bytes = wal_size(db_path);
	if result.busy > 0 || result.wal_bytes > 0 {
		bail!(
			"WAL checkpoint failed for {}: busy={}, walBytes={}",
			db_path.display(),
			result.busy,
			result.wal_bytes
		);
	}
	result.checkpointed = true;
	Ok(result)
}

fn run_wal_gc(options: &Options) -> Result<WalGcResult> {
	let databases = [options.agent_dir.join("history.db"), options.agent_dir.join("models.db")]
		.iter()
		.map(|db| checkpoint_wal(db, options.apply))
		.collect::<Result<Vec<_>>>()?;
	Ok(WalGcResult {
		wal_bytes: databases.iter().map(|db| db.wal_bytes).sum(),
		would_checkpoint: databases.iter().any(|db| db.would_checkpoint),
		checkpointed: databases.iter().any(|db| db.checkpointed),
		databases,
	})
}

// ───────────────────────────── output ─────────────────────────────

fn format_bytes(bytes: u64) -> String {
	let value = bytes as f64;
	if bytes < 1024 {
		format!("{bytes} B")
	} else if bytes < 1024 * 1024 {
		format!("{:.1} KiB", value / 1024.0)
	} else if bytes < 1024 * 1024 * 1024 {
		format!("{:.1} MiB", value / (1024.0 * 1024.0))
	} else {
		format!("{:.1} GiB", value / (1024.0 * 1024.0 * 1024.0))
	}
}

fn render_text(result: &GcResult) -> String {
	let mut lines = vec![format!(
		"GC {} ({})",
		if result.apply { "applied" } else { "dry-run" },
		result.agent_dir
	)];
	if let Some(blobs) = &result.blobs {
		lines.push(format!(
			"blobs: {}/{} files, {}, {} refs",
			blobs.deleted,
			blobs.would_delete,
			format_bytes(blobs.bytes),
			blobs.referenced
		));
		if !blobs.errors.is_empty() {
			lines.push(format!("blob errors: {}", blobs.errors.len()));
		}
	}
	if let Some(archive) = &result.archive {
		lines.push(format!(
			"sessions: {}/{} archived, {} history rows and {} stats rows removed",
			archive.archived,
			archive.would_archive,
			archive.history_rows_deleted,
			archive.stats_rows_deleted
		));
		if archive.skipped_active > 0 {
			lines.push(format!("sessions skipped active: {}", archive.skipped_active));
		}
		if !archive.errors.is_empty() {
			lines.push(format!("session errors: {}", archive.errors.len()));
		}
	}
	if let Some(wal) = &result.wal {
		let state = if wal.checkpointed {
			"checkpointed"
		} else {
			"checkpoint dry-run"
		};
		lines.push(format!(
			"wal: {state}, {} across {} dbs",
			format_bytes(wal.wal_bytes),
			wal.databases.len()
		));
	}
	format!("{}\n", lines.join("\n"))
}
