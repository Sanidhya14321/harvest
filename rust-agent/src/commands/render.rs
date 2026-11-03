//! `harvest render` — draw a session's entire thread headlessly.
//!
//! Ported from `packages/coding-agent/src/cli/render-cli.ts`. The native app
//! renders the newest branch through the portable `harvest-terminal` replay
//! layout instead of the TypeScript `InteractiveMode` pipeline. The session is
//! only read, never locked or appended to.
use std::{
	io::Write,
	path::{Path, PathBuf},
	time::Instant,
};

use anyhow::{Context, Result, bail};
use harvest_protocol::{
	display::project_branch,
	session::{SESSION_LIMIT, SessionDocument, decoded_string},
};
use harvest_runtime::JailedFs;
use harvest_terminal::{layout::PlainTextLayout, replay::render_projection};

#[derive(Debug, clap::Args)]
pub struct RenderArgs {
	/// Session file path or id prefix (default: most recent for cwd)
	session: Option<String>,
	/// Render width in columns (default: terminal width)
	#[arg(short, long)]
	width:   Option<u16>,
	/// Viewport height in rows (default: terminal height)
	#[arg(long)]
	height:  Option<u16>,
	/// Print phase timings and emitted byte counts to stderr
	#[arg(short, long)]
	timing:  bool,
	/// Benchmark N extra full clear-scrollback repaints (the /tree navigation
	/// frame)
	#[arg(long, allow_negative_numbers = true)]
	repaint: Option<i64>,
	/// Strip ANSI styling from the output
	#[arg(long)]
	plain:   bool,
	/// Suppress transcript output (benchmark runs)
	#[arg(short, long)]
	quiet:   bool,
}

fn sessions_root() -> Result<PathBuf> {
	crate::config::agent_dir_from(None, |key| std::env::var_os(key))
		.map(|dir| dir.join("sessions"))
		.context("cannot locate the agent directory")
}

fn encode(prefix: &str, relative: &str) -> String {
	let encoded = relative.replace(['/', '\\', ':'], "-");
	if encoded.is_empty() {
		prefix.to_owned()
	} else if prefix.ends_with('-') {
		format!("{prefix}{encoded}")
	} else {
		format!("{prefix}-{encoded}")
	}
}

pub(crate) fn default_session_dir(cwd: &Path) -> Result<PathBuf> {
	let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
	let home = std::env::var_os("HOME").map(PathBuf::from).and_then(|h| h.canonicalize().ok());
	let temp = std::env::temp_dir().canonicalize().ok();
	let name = if let Some(rel) = home.as_ref().and_then(|h| canonical.strip_prefix(h).ok()) {
		encode("-", &rel.to_string_lossy())
	} else if let Some(rel) = temp.as_ref().and_then(|t| canonical.strip_prefix(t).ok()) {
		encode("-tmp", &rel.to_string_lossy())
	} else {
		let text = canonical.to_string_lossy();
		let trimmed = text.strip_prefix(['/', '\\']).unwrap_or(&text);
		format!("--{}--", trimmed.replace(['/', '\\', ':'], "-"))
	};
	Ok(sessions_root()?.join(name))
}

/// Session files in `dir`, newest first by modification time.
pub(crate) fn list_sessions(dir: &Path) -> Vec<PathBuf> {
	let Ok(entries) = std::fs::read_dir(dir) else {
		return Vec::new();
	};
	let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
		.flatten()
		.map(|entry| entry.path())
		.filter(|path| path.extension().is_some_and(|ext| ext == "jsonl") && path.is_file())
		.map(|path| (path.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), path))
		.collect();
	files.sort_by(|a, b| b.0.cmp(&a.0));
	files.into_iter().map(|(_, path)| path).collect()
}

fn matches(path: &Path, arg: &str) -> bool {
	let arg = arg.to_lowercase();
	let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
	let id = stem.rsplit_once('_').map_or("", |(_, id)| id);
	(!id.is_empty() && id.starts_with(&arg)) || stem.starts_with(&arg)
}

pub(crate) fn resolve_target(arg: Option<&str>, cwd: &Path) -> Result<PathBuf> {
	let local = default_session_dir(cwd)?;
	let Some(arg) = arg else {
		return list_sessions(&local).into_iter().next().with_context(|| {
			format!("No sessions found for {}. Pass a session file or id.", cwd.display())
		});
	};
	if arg.contains('/') || arg.contains('\\') || arg.ends_with(".jsonl") {
		let resolved = std::path::absolute(cwd.join(arg))?;
		if !resolved.exists() {
			bail!("Session file not found: {}", resolved.display());
		}
		return Ok(resolved);
	}
	if let Some(found) = list_sessions(&local).into_iter().find(|p| matches(p, arg)) {
		return Ok(found);
	}
	let root = sessions_root()?;
	let mut global: Vec<PathBuf> = std::fs::read_dir(&root)
		.into_iter()
		.flatten()
		.flatten()
		.filter(|entry| entry.path().is_dir())
		.flat_map(|entry| list_sessions(&entry.path()))
		.collect();
	global.retain(|p| matches(p, arg));
	global.into_iter().next().with_context(|| format!("Session \"{arg}\" not found."))
}

fn format_bytes(bytes: usize) -> String {
	let b = bytes as f64;
	if b >= 1024.0 * 1024.0 {
		format!("{:.1} MiB", b / (1024.0 * 1024.0))
	} else if b >= 1024.0 {
		format!("{:.1} KiB", b / 1024.0)
	} else {
		format!("{bytes} B")
	}
}

fn format_ms(ms: f64) -> String {
	format!("{ms:.0} ms")
}

fn strip_ansi(line: &str) -> String {
	let re = regex::Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)")
		.expect("valid regex");
	re.replace_all(line, "").into_owned()
}

fn terminal_size() -> Option<(u16, u16)> {
	let parse = |key: &str| std::env::var(key).ok().and_then(|v| v.parse::<u16>().ok());
	Some((parse("COLUMNS")?, parse("LINES")?))
}

pub fn execute(args: RenderArgs) -> Result<u8> {
	if args.repaint.is_some_and(|n| n <= 0) {
		bail!("--repaint must be a positive integer");
	}
	let cwd = std::env::current_dir()?;
	let source = resolve_target(args.session.as_deref(), &cwd)?;
	let size = terminal_size();
	let width = usize::from(args.width.or(size.map(|s| s.0)).unwrap_or(120).max(1));
	let height = args.height.or(size.map(|s| s.1)).unwrap_or(40);

	let parent = source.parent().context("session path has no parent")?;
	let name = source.file_name().context("session path has no file name")?;
	let fs = JailedFs::open(parent)?;
	let open_start = Instant::now();
	let bytes = fs.read(Path::new(name), SESSION_LIMIT)?;
	let document = SessionDocument::decode(&bytes)?;
	let open_ms = open_start.elapsed().as_secs_f64() * 1000.0;
	let entries = document.entries();
	let message_count = entries
		.iter()
		.filter(|e| decoded_string(e, "type").is_ok_and(|t| t == "message"))
		.count();
	let leaf = entries
		.iter()
		.rev()
		.find_map(|e| decoded_string(e, "id").ok())
		.context("session has no entries to render")?;

	let replay_start = Instant::now();
	let projection = project_branch(&document, &leaf)?;
	let replay_ms = replay_start.elapsed().as_secs_f64() * 1000.0;

	let paint = || -> Result<Vec<String>> {
		let mut layout = PlainTextLayout::default();
		Ok(render_projection(&projection, width, &mut layout)?)
	};
	let paint_start = Instant::now();
	let rows = paint()?;
	let paint_ms = paint_start.elapsed().as_secs_f64() * 1000.0;
	let paint_bytes: usize = rows.iter().map(|r| r.len() + 1).sum();

	let mut repaints = Vec::new();
	for _ in 0..args.repaint.unwrap_or(0) {
		let start = Instant::now();
		let again = paint()?;
		repaints.push((start.elapsed().as_secs_f64() * 1000.0, again.iter().map(|r| r.len() + 1).sum::<usize>()));
	}

	if !args.quiet {
		let mut out = std::io::stdout().lock();
		let lines: Vec<String> =
			if args.plain { rows.iter().map(|r| strip_ansi(r)).collect() } else { rows.clone() };
		writeln!(out, "{}", lines.join("\n"))?;
	}

	if args.timing || args.repaint.is_some() {
		let mut report = vec![
			format!("session  {}", source.display()),
			format!(
				"         {}, {} entries, {} messages, {} transcript rows @ {}x{}",
				format_bytes(bytes.len()),
				entries.len(),
				message_count,
				rows.len(),
				width,
				height
			),
			format!("open     {}", format_ms(open_ms)),
			format!("replay   {}  (transcript build + component construction)", format_ms(replay_ms)),
			format!(
				"paint    {}  (full frame compose + emit: {}, {} writes)",
				format_ms(paint_ms),
				format_bytes(paint_bytes),
				1
			),
		];
		if !repaints.is_empty() {
			let times: Vec<f64> = repaints.iter().map(|r| r.0).collect();
			let avg = times.iter().sum::<f64>() / times.len() as f64;
			let min = times.iter().copied().fold(f64::INFINITY, f64::min);
			let max = times.iter().copied().fold(0.0, f64::max);
			report.push(format!(
				"repaint  {} avg over {} (min {}, max {}), {}/frame",
				format_ms(avg),
				repaints.len(),
				format_ms(min),
				format_ms(max),
				format_bytes(repaints[0].1)
			));
		}
		let mut err = std::io::stderr().lock();
		writeln!(err, "{}", report.join("\n"))?;
	}
	Ok(0)
}
