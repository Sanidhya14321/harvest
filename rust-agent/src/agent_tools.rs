//! `eval`, `github`, `hub`, `search_code` and `security_scan`, converted from
//! `packages/coding-agent/src/tools/{eval,gh,hub,search-code,security-scan}.ts`.
//!
//! Every child process runs through [`run_capped`]: bounded stdout/stderr,
//! a wall-clock deadline and the session cancellation token.

use std::{
	collections::HashMap,
	io::{Read, Write},
	path::{Path, PathBuf},
	process::{Child, Command, Stdio},
	sync::{Arc, Mutex},
	time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use harvest_runtime::cancel::CancelToken;
use serde::Deserialize;

/// Per-stream byte cap for child output.
const CHILD_OUTPUT_CAP: usize = 64 * 1024;

/// GitHub ops that never mutate (`GITHUB_READONLY_OPS`).
const GITHUB_READONLY_OPS: &[&str] = &[
	"repo_view",
	"file_read",
	"search_issues",
	"search_prs",
	"search_code",
	"search_commits",
	"search_repos",
	"run_watch",
];

/// Whether `name` called with `arguments` needs human approval.
pub fn requires_approval(name: &str, arguments: &str) -> bool {
	match name {
		"eval" | "hub" | "security_scan" => true,
		"github" => serde_json::from_str::<serde_json::Value>(arguments)
			.ok()
			.and_then(|value| value.get("op").and_then(|op| op.as_str()).map(str::to_owned))
			.is_none_or(|op| !GITHUB_READONLY_OPS.contains(&op.as_str())),
		_ => false,
	}
}

struct Captured {
	stdout:    String,
	stderr:    String,
	code:      Option<i32>,
	timed_out: bool,
}

fn drain(mut stream: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
	std::thread::spawn(move || {
		let mut kept = Vec::new();
		let mut buf = [0u8; 8192];
		let mut dropped = 0usize;
		while let Ok(n) = stream.read(&mut buf) {
			if n == 0 {
				break;
			}
			let room = CHILD_OUTPUT_CAP.saturating_sub(kept.len());
			kept.extend_from_slice(&buf[..n.min(room)]);
			dropped += n.saturating_sub(room);
		}
		let mut text = String::from_utf8_lossy(&kept).into_owned();
		if dropped > 0 {
			text.push_str(&format!("\n[…{dropped} bytes truncated…]"));
		}
		text
	})
}

fn run_capped(
	mut command: Command,
	stdin: Option<&str>,
	timeout: Duration,
	cancellation: &CancelToken,
) -> Result<Captured> {
	command
		.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
		.stdout(Stdio::piped())
		.stderr(Stdio::piped());
	let mut child = command.spawn().context("failed to spawn process")?;
	if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
		let input = input.to_owned();
		std::thread::spawn(move || {
			let _ = pipe.write_all(input.as_bytes());
		});
	}
	let out = drain(child.stdout.take().context("stdout")?);
	let err = drain(child.stderr.take().context("stderr")?);
	let started = Instant::now();
	let mut timed_out = false;
	let status = loop {
		if let Some(status) = child.try_wait()? {
			break Some(status);
		}
		if cancellation.heartbeat().is_err() {
			let _ = child.kill();
			let _ = child.wait();
			cancellation.heartbeat()?;
		}
		if started.elapsed() >= timeout {
			timed_out = true;
			let _ = child.kill();
			break child.wait().ok();
		}
		std::thread::sleep(Duration::from_millis(20));
	};
	Ok(Captured {
		stdout: out.join().unwrap_or_default(),
		stderr: err.join().unwrap_or_default(),
		code: status.and_then(|status| status.code()),
		timed_out,
	})
}

// ---------------------------------------------------------------- eval

#[derive(Debug, Deserialize)]
pub struct EvalArgs {
	language: Option<String>,
	code:     String,
	timeout:  Option<f64>,
}

pub fn eval(root: &Path, args: &EvalArgs, cancellation: &CancelToken) -> Result<String> {
	if args.code.trim().is_empty() {
		bail!("code is empty");
	}
	let language = args.language.as_deref().unwrap_or("js");
	let mut command = match language {
		"py" | "python" => {
			let python = which("python3").or_else(|| which("python")).context("python3 not found on PATH")?;
			let mut command = Command::new(python);
			command.arg("-c").arg(&args.code);
			command
		},
		"js" | "javascript" | "ts" => {
			if let Some(bun) = which("bun") {
				let mut command = Command::new(bun);
				// `bun run -` reads the module from stdin, enabling top-level await.
				command.args(["run", "--bun", "-"]);
				command
			} else {
				let node = which("node").context("neither bun nor node found on PATH")?;
				let mut command = Command::new(node);
				command.args(["--input-type=module", "-"]);
				command
			}
		},
		other => bail!("unsupported eval language: {other} (expected py or js)"),
	};
	command.current_dir(root);
	let stdin = (!matches!(language, "py" | "python")).then_some(args.code.as_str());
	let seconds = args.timeout.unwrap_or(30.0).clamp(1.0, 600.0);
	let result = run_capped(command, stdin, Duration::from_secs_f64(seconds), cancellation)?;
	let mut text = result.stdout;
	if !result.stderr.is_empty() {
		if !text.is_empty() {
			text.push('\n');
		}
		text.push_str(&result.stderr);
	}
	if text.is_empty() {
		text.push_str("(no output)");
	}
	if result.timed_out {
		text.push_str(&format!("\n\n[Cell timed out after {seconds} seconds]"));
	} else if let Some(code) = result.code.filter(|code| *code != 0) {
		text.push_str(&format!("\n\n[Cell exited with code {code}]"));
	}
	Ok(text)
}

fn which(binary: &str) -> Option<PathBuf> {
	std::env::var_os("PATH").and_then(|paths| {
		std::env::split_paths(&paths)
			.map(|dir| dir.join(binary))
			.find(|candidate| candidate.is_file())
	})
}

// ---------------------------------------------------------------- github

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubArgs {
	op:                String,
	repo:              Option<String>,
	branch:            Option<String>,
	path:              Option<String>,
	pr:                Option<serde_json::Value>,
	force:             Option<bool>,
	force_with_lease:  Option<bool>,
	title:             Option<String>,
	body:              Option<String>,
	base:              Option<String>,
	head:              Option<String>,
	draft:             Option<bool>,
	fill:              Option<bool>,
	#[serde(default)]
	reviewer:          Vec<String>,
	#[serde(default)]
	assignee:          Vec<String>,
	#[serde(default)]
	label:             Vec<String>,
	query:             Option<String>,
	since:             Option<String>,
	until:             Option<String>,
	date_field:        Option<String>,
	limit:             Option<f64>,
	run:               Option<String>,
	tail:              Option<f64>,
}

const GH_TIMEOUT: Duration = Duration::from_secs(120);

fn gh(root: &Path, args: &[String], cancellation: &CancelToken) -> Result<String> {
	let binary = which("gh").context("GitHub CLI (gh) not found on PATH")?;
	let mut command = Command::new(binary);
	command.args(args).current_dir(root).env("GH_PROMPT_DISABLED", "1").env("NO_COLOR", "1");
	let result = run_capped(command, None, GH_TIMEOUT, cancellation)?;
	if result.timed_out {
		bail!("gh {} timed out", args.first().map_or("", String::as_str));
	}
	if result.code != Some(0) {
		let message = if result.stderr.trim().is_empty() { result.stdout } else { result.stderr };
		bail!("gh failed: {}", message.trim());
	}
	Ok(result.stdout)
}

fn git(root: &Path, args: &[&str], cancellation: &CancelToken) -> Result<String> {
	let mut command = Command::new("git");
	command.args(args).current_dir(root).env("GIT_TERMINAL_PROMPT", "0");
	let result = run_capped(command, None, GH_TIMEOUT, cancellation)?;
	if result.code != Some(0) {
		bail!("git {} failed: {}", args.join(" "), result.stderr.trim());
	}
	Ok(result.stdout)
}

fn resolve_repo(root: &Path, repo: Option<&str>, cancellation: &CancelToken) -> Result<String> {
	if let Some(repo) = repo.map(str::trim).filter(|repo| !repo.is_empty()) {
		return Ok(repo.to_owned());
	}
	let text = gh(
		root,
		&["repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"].map(str::to_owned),
		cancellation,
	)?;
	Ok(text.trim().to_owned())
}

/// `parseSearchDateBound`: relative `<n><unit>` or ISO date/datetime.
fn date_bound(raw: &str) -> Result<String> {
	let raw = raw.trim();
	let relative = regex::Regex::new(r"(?i)^(\d+)\s*(mo|m|h|d|w|y)$").expect("static regex");
	if let Some(caps) = relative.captures(raw) {
		let n: i64 = caps[1].parse()?;
		let duration = match caps[2].to_ascii_lowercase().as_str() {
			"m" => chrono::Duration::minutes(n),
			"h" => chrono::Duration::hours(n),
			"d" => chrono::Duration::days(n),
			"w" => chrono::Duration::weeks(n),
			"mo" => chrono::Duration::days(30 * n),
			_ => chrono::Duration::days(365 * n),
		};
		return Ok((chrono::Utc::now() - duration).format("%Y-%m-%dT%H:%M:%SZ").to_string());
	}
	if regex::Regex::new(r"^\d{4}-\d{2}-\d{2}").expect("static regex").is_match(raw) {
		return Ok(raw.to_owned());
	}
	bail!("invalid date bound '{raw}': use <n>m/h/d/w/mo/y, YYYY-MM-DD, or an ISO datetime")
}

fn search(root: &Path, args: &GithubArgs, kind: &str, cancellation: &CancelToken) -> Result<String> {
	let mut parts: Vec<String> = Vec::new();
	if let Some(query) = args.query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
		parts.push(query.to_owned());
	} else if kind == "code" {
		bail!("search_code requires query");
	}
	if kind == "code" && (args.since.is_some() || args.until.is_some()) {
		bail!("search_code does not support since/until");
	}
	let scoped = parts
		.first()
		.is_some_and(|q| regex::Regex::new(r"(?i)(?:^|\s)-?(?:repo|org|user|owner):\S").expect("static").is_match(q));
	if kind != "repositories" && !scoped {
		if let Ok(repo) = resolve_repo(root, args.repo.as_deref(), cancellation) {
			parts.push(format!("repo:{repo}"));
		}
	}
	match kind {
		"issues" => parts.push("is:issue".into()),
		"prs" => parts.push("is:pr".into()),
		_ => {},
	}
	let field = match (kind, args.date_field.as_deref()) {
		("repositories", Some("updated")) => "pushed",
		("commits", _) => "committer-date",
		(_, Some("updated")) => "updated",
		_ => "created",
	};
	match (args.since.as_deref(), args.until.as_deref()) {
		(Some(since), Some(until)) => parts.push(format!("{field}:{}..{}", date_bound(since)?, date_bound(until)?)),
		(Some(since), None) => parts.push(format!("{field}:>={}", date_bound(since)?)),
		(None, Some(until)) => parts.push(format!("{field}:<={}", date_bound(until)?)),
		(None, None) => {},
	}
	if parts.is_empty() {
		bail!("search requires query or since/until");
	}
	let limit = args.limit.map_or(10, |limit| (limit as usize).clamp(1, 50));
	let endpoint = match kind {
		"issues" | "prs" => "search/issues",
		"code" => "search/code",
		"commits" => "search/commits",
		_ => "search/repositories",
	};
	let query = parts.join(" ");
	let text = gh(
		root,
		&[
			"api".into(),
			"-X".into(),
			"GET".into(),
			endpoint.into(),
			"-f".into(),
			format!("q={query}"),
			"-f".into(),
			format!("per_page={limit}"),
		],
		cancellation,
	)?;
	let value: serde_json::Value = serde_json::from_str(&text).context("invalid gh search response")?;
	let items = value["items"].as_array().cloned().unwrap_or_default();
	let mut lines = vec![format!("# {} {kind} results for `{query}`", items.len())];
	for item in items.iter().take(limit) {
		let line = match kind {
			"code" => format!(
				"- {}:{} ({})",
				item["repository"]["full_name"].as_str().unwrap_or("?"),
				item["path"].as_str().unwrap_or("?"),
				item["html_url"].as_str().unwrap_or("")
			),
			"commits" => format!(
				"- {} {} — {}",
				item["sha"].as_str().unwrap_or("").get(..12).unwrap_or(""),
				item["commit"]["message"].as_str().unwrap_or("").lines().next().unwrap_or(""),
				item["commit"]["author"]["date"].as_str().unwrap_or("")
			),
			"repositories" => format!(
				"- {} ★{} — {}",
				item["full_name"].as_str().unwrap_or("?"),
				item["stargazers_count"].as_u64().unwrap_or(0),
				item["description"].as_str().unwrap_or("")
			),
			_ => format!(
				"- #{} [{}] {} — {} ({})",
				item["number"].as_u64().unwrap_or(0),
				item["state"].as_str().unwrap_or("?"),
				item["title"].as_str().unwrap_or(""),
				item["user"]["login"].as_str().unwrap_or("?"),
				item["updated_at"].as_str().unwrap_or("")
			),
		};
		lines.push(line);
	}
	Ok(lines.join("\n"))
}

fn pr_refs(value: Option<&serde_json::Value>) -> Vec<String> {
	match value {
		Some(serde_json::Value::String(text)) => vec![text.clone()],
		Some(serde_json::Value::Number(number)) => vec![number.to_string()],
		Some(serde_json::Value::Array(items)) => items
			.iter()
			.filter_map(|item| match item {
				serde_json::Value::String(text) => Some(text.clone()),
				serde_json::Value::Number(number) => Some(number.to_string()),
				_ => None,
			})
			.collect(),
		_ => Vec::new(),
	}
}

pub fn github(root: &Path, args: &GithubArgs, cancellation: &CancelToken) -> Result<String> {
	match args.op.as_str() {
		"repo_view" => {
			let mut argv = vec!["repo".to_owned(), "view".to_owned()];
			argv.extend(args.repo.clone());
			argv.extend(
				["--json", "nameWithOwner,description,defaultBranchRef,url,visibility,stargazerCount,isFork"]
					.map(str::to_owned),
			);
			gh(root, &argv, cancellation)
		},
		"file_read" => {
			let path = args.path.as_deref().map(str::trim).filter(|p| !p.is_empty()).context("path is required")?;
			let repo = resolve_repo(root, args.repo.as_deref(), cancellation)?;
			let mut endpoint = format!("repos/{repo}/contents/{}", path.trim_start_matches('/'));
			if let Some(branch) = &args.branch {
				endpoint.push_str(&format!("?ref={}", form_urlencoded::byte_serialize(branch.as_bytes()).collect::<String>()));
			}
			let text = gh(root, &["api".into(), endpoint], cancellation)?;
			let value: serde_json::Value = serde_json::from_str(&text).context("invalid contents response")?;
			if value.is_array() {
				bail!("{path} is a directory in {repo}");
			}
			let encoded: String = value["content"].as_str().unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect();
			use base64::Engine as _;
			let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).context("invalid base64 content")?;
			if bytes.iter().take(8000).any(|byte| *byte == 0) {
				return Ok(format!("{path}: binary file ({} bytes)", bytes.len()));
			}
			Ok(String::from_utf8_lossy(&bytes).into_owned())
		},
		"search_issues" => search(root, args, "issues", cancellation),
		"search_prs" => search(root, args, "prs", cancellation),
		"search_code" => search(root, args, "code", cancellation),
		"search_commits" => search(root, args, "commits", cancellation),
		"search_repos" => search(root, args, "repositories", cancellation),
		"pr_create" => {
			let mut argv = vec!["pr".to_owned(), "create".to_owned()];
			for (flag, value) in [
				("--repo", &args.repo),
				("--title", &args.title),
				("--body", &args.body),
				("--base", &args.base),
				("--head", &args.head),
			] {
				if let Some(value) = value {
					argv.extend([flag.to_owned(), value.clone()]);
				}
			}
			if args.title.is_none() && args.body.is_none() && args.fill != Some(false) {
				argv.push("--fill".into());
			}
			if args.draft == Some(true) {
				argv.push("--draft".into());
			}
			for (flag, values) in [("--reviewer", &args.reviewer), ("--assignee", &args.assignee), ("--label", &args.label)] {
				for value in values {
					argv.extend([flag.to_owned(), value.clone()]);
				}
			}
			gh(root, &argv, cancellation)
		},
		"pr_checkout" => {
			let refs = pr_refs(args.pr.as_ref());
			if refs.is_empty() {
				bail!("pr is required");
			}
			let top = git(root, &["rev-parse", "--show-toplevel"], cancellation)?;
			let top = PathBuf::from(top.trim());
			let mut out = Vec::new();
			for pr in refs {
				let number = pr.rsplit('/').next().unwrap_or(&pr).to_owned();
				let dir = top.join(".worktrees").join(format!("pr-{number}"));
				let dir_text = dir.to_string_lossy().into_owned();
				if !dir.exists() {
					git(root, &["worktree", "add", "--detach", &dir_text], cancellation)?;
				}
				let mut argv = vec!["pr".to_owned(), "checkout".to_owned(), pr.clone()];
				if let Some(repo) = &args.repo {
					argv.extend(["--repo".to_owned(), repo.clone()]);
				}
				if args.force == Some(true) {
					argv.push("--force".into());
				}
				gh(&dir, &argv, cancellation)?;
				out.push(format!("PR {pr} checked out at {dir_text}"));
			}
			Ok(out.join("\n"))
		},
		"pr_push" => {
			let refs = pr_refs(args.pr.as_ref());
			let pr = refs.first().context("pr is required")?;
			let number = pr.rsplit('/').next().unwrap_or(pr);
			let top = git(root, &["rev-parse", "--show-toplevel"], cancellation)?;
			let dir = PathBuf::from(top.trim()).join(".worktrees").join(format!("pr-{number}"));
			if !dir.is_dir() {
				bail!("PR {pr} has no checkout; run op: pr_checkout first");
			}
			let mut argv = vec!["push"];
			if args.force_with_lease == Some(true) {
				argv.push("--force-with-lease");
			}
			git(&dir, &argv, cancellation)?;
			Ok(format!("Pushed PR {pr} from {}", dir.display()))
		},
		"run_watch" => {
			let mut argv = vec!["run".to_owned()];
			match &args.run {
				Some(run) => {
					let id = run.rsplit('/').find(|part| part.chars().all(|c| c.is_ascii_digit()) && !part.is_empty());
					argv.extend(["watch".into(), id.unwrap_or(run).to_owned(), "--exit-status".into()]);
				},
				None => {
					argv.extend(["list".into(), "--limit".into(), "20".into()]);
					let branch = match &args.branch {
						Some(branch) => branch.clone(),
						None => git(root, &["rev-parse", "--abbrev-ref", "HEAD"], cancellation)?.trim().to_owned(),
					};
					argv.extend(["--branch".into(), branch]);
				},
			}
			if let Some(repo) = &args.repo {
				argv.extend(["--repo".into(), repo.clone()]);
			}
			match gh(root, &argv, cancellation) {
				Ok(text) => Ok(text),
				Err(error) => {
					let Some(run) = &args.run else { return Err(error) };
					let tail = args.tail.map_or(80, |tail| (tail as usize).clamp(1, 1000));
					let logs = gh(root, &["run".into(), "view".into(), run.clone(), "--log-failed".into()], cancellation)
						.unwrap_or_default();
					let lines: Vec<&str> = logs.lines().collect();
					let start = lines.len().saturating_sub(tail);
					Ok(format!("{error}\n\nFailed job log (last {tail} lines):\n{}", lines[start..].join("\n")))
				},
			}
		},
		other => bail!("unknown github op: {other}"),
	}
}

// ---------------------------------------------------------------- hub

#[derive(Debug, Deserialize)]
pub struct HubArgs {
	op:          String,
	name:        Option<String>,
	application: Option<String>,
	#[serde(default)]
	args:        Vec<String>,
	env:         Option<HashMap<String, String>>,
	cwd:         Option<String>,
	lines:       Option<f64>,
	grep:        Option<String>,
}

struct Supervised {
	child: Child,
	log:   Arc<Mutex<Vec<u8>>>,
	label: String,
}

static PROCESSES: Mutex<Option<HashMap<String, Supervised>>> = Mutex::new(None);

fn tail_into(mut stream: impl Read + Send + 'static, log: Arc<Mutex<Vec<u8>>>) {
	std::thread::spawn(move || {
		let mut buf = [0u8; 8192];
		while let Ok(n) = stream.read(&mut buf) {
			if n == 0 {
				break;
			}
			let mut log = log.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
			log.extend_from_slice(&buf[..n]);
			// Ring-buffer: keep the newest CHILD_OUTPUT_CAP * 4 bytes.
			let cap = CHILD_OUTPUT_CAP * 4;
			if log.len() > cap {
				let excess = log.len() - cap;
				log.drain(..excess);
			}
		}
	});
}

pub fn hub(root: &Path, args: &HubArgs) -> Result<String> {
	let mut guard = PROCESSES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
	let processes = guard.get_or_insert_with(HashMap::new);
	let name = || args.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).context("name is required");
	match args.op.as_str() {
		"start" => {
			let application = args.application.as_deref().context("application is required")?;
			let label = args.name.clone().unwrap_or_else(|| {
				Path::new(application).file_name().map_or_else(|| "proc".into(), |n| n.to_string_lossy().into_owned())
			});
			if label.len() > 48 {
				bail!("name must be at most 48 characters");
			}
			if let Some(existing) = processes.get_mut(&label) {
				if existing.child.try_wait()?.is_none() {
					bail!("process {label} is already running");
				}
			}
			let cwd = match &args.cwd {
				Some(cwd) => {
					let fs = harvest_runtime::JailedFs::open(root)?;
					let absolute = fs.root().join(fs.relative(Path::new(cwd))?);
					let canonical = std::fs::canonicalize(&absolute)
						.with_context(|| format!("Working directory does not exist: {}", absolute.display()))?;
					if !canonical.starts_with(fs.root()) || !canonical.is_dir() {
						bail!("cwd must be a directory inside the workspace");
					}
					canonical
				},
				None => root.to_owned(),
			};
			let mut command = Command::new(application);
			command.args(&args.args).current_dir(&cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
			if let Some(env) = &args.env {
				command.envs(env);
			}
			let mut child = command.spawn().with_context(|| format!("failed to start {application}"))?;
			let log = Arc::new(Mutex::new(Vec::new()));
			tail_into(child.stdout.take().context("stdout")?, log.clone());
			tail_into(child.stderr.take().context("stderr")?, log.clone());
			let pid = child.id();
			processes.insert(label.clone(), Supervised { child, log, label: label.clone() });
			Ok(format!("Started {label} (pid {pid})"))
		},
		"ps" => {
			if processes.is_empty() {
				return Ok("No supervised processes.".into());
			}
			let mut rows = Vec::new();
			for process in processes.values_mut() {
				let status = match process.child.try_wait()? {
					None => "running".to_owned(),
					Some(status) => format!("exited ({})", status.code().map_or("signal".into(), |c| c.to_string())),
				};
				rows.push(format!("- {} pid {} {status}", process.label, process.child.id()));
			}
			rows.sort();
			Ok(rows.join("\n"))
		},
		"logs" => {
			let name = name()?;
			let process = processes.get(name).with_context(|| format!("no process named {name}"))?;
			let text = String::from_utf8_lossy(&process.log.lock().unwrap_or_else(std::sync::PoisonError::into_inner)).into_owned();
			let filter = args.grep.as_deref().map(regex::Regex::new).transpose()?;
			let lines: Vec<&str> = text.lines().filter(|line| filter.as_ref().is_none_or(|re| re.is_match(line))).collect();
			let count = args.lines.map_or(100, |n| (n as usize).clamp(1, 1000));
			let start = lines.len().saturating_sub(count);
			Ok(if lines.is_empty() { "(no output)".into() } else { lines[start..].join("\n") })
		},
		"stop" => {
			let name = name()?;
			let mut process = processes.remove(name).with_context(|| format!("no process named {name}"))?;
			let _ = process.child.kill();
			let _ = process.child.wait();
			Ok(format!("Stopped {name}"))
		},
		"send" | "wait" | "inbox" | "list" | "jobs" | "cancel" | "restart" | "describe" => bail!(
			"hub op '{}' requires the agent registry/job manager, which the Rust runtime does not implement yet",
			args.op
		),
		other => bail!("unknown hub op: {other}"),
	}
}

// ---------------------------------------------------------------- search_code

#[derive(Debug, Deserialize)]
pub struct SearchCodeArgs {
	query: String,
	mode:  Option<String>,
	limit: Option<f64>,
}

const SOURCE_EXTENSIONS: &[&str] = &[
	"rs", "ts", "tsx", "js", "jsx", "mjs", "py", "go", "java", "kt", "swift", "c", "h", "cc", "cpp", "hpp", "cs",
	"rb", "php", "scala", "lua", "zig",
];

/// Column-0 declaration: `(kind, name)` when `line` starts one.
fn declaration(line: &str) -> Option<(&'static str, String)> {
	static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
	let re = PATTERN.get_or_init(|| {
		regex::Regex::new(
			r"^(?:export\s+)?(?:default\s+)?(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(fn|function|class|interface|type|enum|struct|trait|impl|def|const|let|mod|func|object)\s+([A-Za-z_$][\w$]*)",
		)
		.expect("static regex")
	});
	let caps = re.captures(line)?;
	let kind = match &caps[1] {
		"fn" | "function" | "def" | "func" => "function",
		"class" | "object" => "class",
		"interface" | "trait" => "interface",
		"type" => "type",
		"enum" => "enum",
		"struct" => "struct",
		"impl" => "impl",
		"mod" => "module",
		_ => "variable",
	};
	Some((kind, caps[2].to_owned()))
}

pub fn search_code(root: &Path, args: &SearchCodeArgs, cancellation: &CancelToken) -> Result<String> {
	let query = args.query.trim();
	let mode = args.mode.as_deref().unwrap_or("all");
	let limit = args.limit.unwrap_or(20.0);
	if query.is_empty() || limit.fract() != 0.0 || !(1.0..=100.0).contains(&limit) {
		bail!("Search requires a non-empty query and an integer limit between 1 and 100.");
	}
	if !matches!(mode, "symbols" | "docs" | "all") {
		bail!("mode must be symbols, docs or all");
	}
	let limit = limit as usize;
	let terms: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_owned).collect();
	let heartbeat = || cancellation.heartbeat();
	let mut query_glob = pi_walker::glob::GlobQuery::new(root, "**/*");
	query_glob.file_type = Some(pi_walker::FileType::File);
	query_glob.max_results = 20_000;
	let entries = pi_walker::glob::glob(&query_glob, heartbeat)?;
	let mut output = Vec::new();
	let mut total = 0;

	if mode != "docs" {
		let mut symbols: Vec<(u32, String)> = Vec::new();
		for entry in &entries {
			let ext = Path::new(&entry.path).extension().and_then(|e| e.to_str()).unwrap_or("");
			if !SOURCE_EXTENSIONS.contains(&ext) || entry.size.is_some_and(|s| s > 1_000_000.0) {
				continue;
			}
			cancellation.heartbeat()?;
			let Ok(text) = std::fs::read_to_string(root.join(&entry.path)) else { continue };
			for (index, line) in text.lines().enumerate() {
				let Some((kind, name)) = declaration(line) else { continue };
				let lower = name.to_lowercase();
				let score = if terms.iter().any(|t| *t == lower) {
					3
				} else if terms.iter().all(|t| lower.contains(t.as_str()) || line.to_lowercase().contains(t.as_str())) {
					if terms.iter().any(|t| lower.contains(t.as_str())) { 2 } else { 1 }
				} else {
					continue;
				};
				let signature: String = line.trim_end().chars().take(160).collect();
				symbols.push((
					score,
					format!("- **{name}** ({kind}) at `{}:{}`\n  `{signature}`", entry.path, index + 1),
				));
			}
		}
		symbols.sort_by(|a, b| b.0.cmp(&a.0));
		symbols.truncate(limit);
		if !symbols.is_empty() {
			output.push(format!("### Code Symbols ({} matches):", symbols.len()));
			total += symbols.len();
			output.extend(symbols.into_iter().map(|(_, line)| line));
		}
	}

	if mode != "symbols" {
		let mut sections: Vec<(f64, String)> = Vec::new();
		for entry in &entries {
			if !entry.path.ends_with(".md") || entry.size.is_some_and(|s| s > 1_000_000.0) {
				continue;
			}
			cancellation.heartbeat()?;
			let Ok(text) = std::fs::read_to_string(root.join(&entry.path)) else { continue };
			let title = text
				.lines()
				.find_map(|l| l.strip_prefix("# "))
				.unwrap_or(&entry.path)
				.trim()
				.to_owned();
			let mut heading = title.clone();
			let mut body = String::new();
			let flush = |heading: &str, body: &str, sections: &mut Vec<(f64, String)>| {
				let lower = format!("{heading}\n{body}").to_lowercase();
				let words = lower.split_whitespace().count().max(1) as f64;
				let mut score = 0.0;
				for term in &terms {
					let tf = lower.matches(term.as_str()).count() as f64;
					if tf > 0.0 {
						// BM25 term saturation with length normalisation (k1=1.2, b=0.75, avgdl≈200).
						score += tf * 2.2 / (tf + 1.2 * (0.25 + 0.75 * words / 200.0));
					}
				}
				if score > 0.0 {
					let preview: String = body.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(150).collect();
					sections.push((
						score,
						format!("- **{title}** (`{}`, score: {score:.2})\n  {heading}\n  > {preview}...", entry.path),
					));
				}
			};
			for line in text.lines() {
				if line.starts_with('#') {
					flush(&heading, &body, &mut sections);
					heading = line.trim_start_matches('#').trim().to_owned();
					body.clear();
				} else {
					body.push_str(line);
					body.push('\n');
				}
			}
			flush(&heading, &body, &mut sections);
		}
		sections.sort_by(|a, b| b.0.total_cmp(&a.0));
		sections.truncate(limit);
		if !sections.is_empty() {
			if !output.is_empty() {
				output.push(String::new());
			}
			output.push(format!("### Documentation Sections ({} matches):", sections.len()));
			total += sections.len();
			output.extend(sections.into_iter().map(|(_, line)| line));
		}
	}

	if total == 0 {
		return Ok(format!("No matches found for query '{query}'."));
	}
	Ok(output.join("\n"))
}

// ---------------------------------------------------------------- security_scan

pub fn security_scan(settings: &crate::settings::Settings, arguments: &str) -> Result<String> {
	let value: serde_json::Value = serde_json::from_str(arguments).context("invalid security_scan arguments")?;
	let action = value["action"].as_str().context("action is required")?;
	const ACTIONS: &[&str] = &[
		"preflight", "start", "status", "cancel", "validate", "cloud_scans", "cloud_start", "cloud_status", "cloud_pull",
	];
	if !ACTIONS.contains(&action) {
		bail!("unknown security_scan action: {action}");
	}
	if !settings.bool("security.enabled") {
		bail!("Security is disabled. Enable security.enabled before using security_scan.");
	}
	bail!(
		"security_scan '{action}' needs the security coordinator and Codex Security cloud client, which the Rust runtime does not implement yet"
	)
}
