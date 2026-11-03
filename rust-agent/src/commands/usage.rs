//! `harvest usage` — provider usage limits (`usage-cli.ts`).
//!
//! Ported: `invalidate`, `--history` and `clients` (local `agent.db`).
//! The live per-account snapshot needs the credential store and provider
//! usage fetchers, which the native runtime does not have yet.
use std::{
	collections::{BTreeMap, HashMap, HashSet},
	time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use clap::{Args, ValueEnum};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};

use super::{Paint, format_duration, format_number};

const HISTORY_SPARK_WIDTH: usize = 48;
const SPARK_LEVELS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
const DAY_MS: f64 = 86_400_000.0;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum UsageAction {
	Invalidate,
	Clients,
}

#[derive(Debug, Args)]
pub struct UsageArgs {
	/// Optional subcommand to execute
	#[arg(value_enum)]
	pub action:   Option<UsageAction>,
	/// Output usage reports as JSON
	#[arg(short, long)]
	pub json:     bool,
	/// Only show usage for this provider id (e.g. anthropic)
	#[arg(short, long)]
	pub provider: Option<String>,
	/// Redact account emails/ids (shortest unique prefix) for sharing screenshots
	#[arg(short, long)]
	pub redact:   bool,
	/// Show recorded usage-limit history (hourly snapshots) instead of a live snapshot
	#[arg(long)]
	pub history:  bool,
	/// History window in days (with --history or clients)
	#[arg(short, long, default_value_t = 7)]
	pub days:     i64,
}

fn now_ms() -> f64 {
	SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

fn open_agent_db(write: bool) -> Result<Option<Connection>> {
	let path = super::agent_dir()?.join("agent.db");
	if !path.is_file() {
		return Ok(None);
	}
	let flags = if write {
		OpenFlags::SQLITE_OPEN_READ_WRITE
	} else {
		OpenFlags::SQLITE_OPEN_READ_ONLY
	};
	let db = Connection::open_with_flags(&path, flags)?;
	db.busy_timeout(Duration::from_secs(5))?;
	Ok(Some(db))
}

pub fn execute(args: UsageArgs) -> Result<u8> {
	match args.action {
		Some(UsageAction::Invalidate) => invalidate(&args),
		Some(UsageAction::Clients) => clients(&args),
		None if args.history => history(&args),
		None => {
			eprintln!(
				"{}",
				Paint::stderr().yellow(
					"Live usage snapshots are not available in the native runtime yet (the provider \
					 usage fetchers are still TypeScript-only). Use `harvest usage --history` or \
					 `harvest usage clients`."
				)
			);
			Ok(1)
		},
	}
}

fn invalidate(args: &UsageArgs) -> Result<u8> {
	let provider = args.provider.as_deref().map(str::to_lowercase);
	if let Some(db) = open_agent_db(true)? {
		let prefix = match &provider {
			Some(p) => format!("usage_cache:report:{p}:"),
			None => "usage_cache:report:".to_owned(),
		};
		let has_cache: bool = db
			.query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name='cache'", [], |_| Ok(true))
			.unwrap_or(false);
		if has_cache {
			db.execute(
				"DELETE FROM cache WHERE substr(key, 1, ?) = ?",
				params![prefix.chars().count() as i64, prefix],
			)?;
		}
	}
	match provider {
		Some(p) => println!("Invalidated cached usage reports for provider \"{p}\"."),
		None => println!("Invalidated cached usage reports for all providers."),
	}
	Ok(0)
}

fn window(args: &UsageArgs) -> (f64, f64) {
	let days = if args.days > 0 { args.days } else { 7 };
	let now = now_ms();
	(now, now - days as f64 * DAY_MS)
}

// ── redaction ────────────────────────────────────────────────────────────

fn prefix_chars(value: &str, n: usize) -> String {
	value.chars().take(n).collect()
}

fn find_distinguishing_infix(value: &[char], peers: &[Vec<char>]) -> Option<String> {
	let start = value.len().min(2);
	let center = value.len() as f64 / 2.0;
	let contains = |hay: &[char], needle: &[char]| hay.windows(needle.len()).any(|w| w == needle);
	for length in 1..=value.len().saturating_sub(start) {
		let mut best: Option<(String, f64)> = None;
		let mut pos = start;
		while pos + length <= value.len() {
			let candidate = &value[pos..pos + length];
			if !peers.iter().any(|peer| contains(peer, candidate)) {
				let distance = (pos as f64 + length as f64 / 2.0 - center).abs();
				if best.as_ref().is_none_or(|(_, d)| distance < *d) {
					best = Some((candidate.iter().collect(), distance));
				}
			}
			pos += 1;
		}
		if let Some((infix, _)) = best {
			return Some(infix);
		}
	}
	None
}

/// `buildRedactionMap`.
fn build_redaction_map(values: impl IntoIterator<Item = String>) -> HashMap<String, String> {
	let mut seen = HashSet::new();
	let unique: Vec<String> = values.into_iter().filter(|v| seen.insert(v.clone())).collect();
	let mut by_anchor: HashMap<String, Vec<&String>> = HashMap::new();
	for value in &unique {
		by_anchor.entry(prefix_chars(value, 2)).or_default().push(value);
	}
	let mut map = HashMap::new();
	for value in &unique {
		let anchor = prefix_chars(value, 2);
		let peers: Vec<Vec<char>> = by_anchor[&anchor]
			.iter()
			.filter(|o| **o != value)
			.map(|o| o.chars().collect())
			.collect();
		if peers.is_empty() {
			map.insert(value.clone(), format!("{anchor}*"));
			continue;
		}
		let chars: Vec<char> = value.chars().collect();
		let mask = match find_distinguishing_infix(&chars, &peers) {
			Some(infix) => format!("{anchor}*{infix}*"),
			None => format!("{anchor}*"),
		};
		map.insert(value.clone(), mask);
	}
	let mut by_mask: HashMap<String, Vec<String>> = HashMap::new();
	for value in &unique {
		by_mask.entry(map[value].clone()).or_default().push(value.clone());
	}
	for collided in by_mask.values().filter(|c| c.len() >= 2) {
		for value in collided {
			let count = value.chars().count();
			let mut length = count.min(2);
			while length < count
				&& collided
					.iter()
					.any(|o| o != value && o.starts_with(&prefix_chars(value, length)))
			{
				length += 1;
			}
			map.insert(value.clone(), format!("{}*", prefix_chars(value, length)));
		}
	}
	map
}

// ── history ──────────────────────────────────────────────────────────────

struct HistoryEntry {
	recorded_at:   f64,
	provider:      String,
	account_key:   String,
	email:         Option<String>,
	account_id:    Option<String>,
	limit_id:      String,
	label:         String,
	window_label:  Option<String>,
	used_fraction: Option<f64>,
	status:        Option<String>,
	resets_at:     Option<f64>,
}

fn status_of(fraction: Option<f64>, status: Option<&str>) -> &'static str {
	match status {
		Some("exhausted") => return "exhausted",
		Some("warning") => return "warning",
		Some("ok") => return "ok",
		_ => {},
	}
	match fraction {
		None => "unknown",
		Some(f) if f >= 1.0 => "exhausted",
		Some(f) if f >= 0.8 => "warning",
		Some(_) => "ok",
	}
}

fn paint_status(p: &Paint, status: &str, text: &str) -> String {
	match status {
		"exhausted" => p.red(text),
		"warning" => p.yellow(text),
		"ok" => p.green(text),
		_ => p.dim(text),
	}
}

fn provider_name(provider: &str) -> String {
	provider
		.split(['-', '_'])
		.map(|part| {
			let mut chars = part.chars();
			chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
		})
		.collect::<Vec<_>>()
		.join(" ")
}

fn series_title(entry: &HistoryEntry) -> String {
	let Some(window) = entry.window_label.as_deref().filter(|w| !w.is_empty()) else {
		return entry.label.clone();
	};
	let lower = window.to_lowercase();
	if lower == "quota window" || entry.label.to_lowercase().contains(&lower) {
		return entry.label.clone();
	}
	format!("{} ({window})", entry.label)
}

fn sparkline(p: &Paint, entries: &[&HistoryEntry], since: f64, now: f64) -> String {
	let span = (now - since).max(1.0);
	let mut buckets: Vec<Option<f64>> = vec![None; HISTORY_SPARK_WIDTH];
	for entry in entries {
		let Some(fraction) = entry.used_fraction else { continue };
		let offset = (((entry.recorded_at - since) / span) * HISTORY_SPARK_WIDTH as f64).floor();
		let index = offset.clamp(0.0, (HISTORY_SPARK_WIDTH - 1) as f64) as usize;
		buckets[index] = Some(buckets[index].map_or(fraction, |prev: f64| prev.max(fraction)));
	}
	buckets
		.into_iter()
		.map(|bucket| match bucket {
			None => p.dim("·"),
			Some(f) => {
				let clamped = f.clamp(0.0, 1.0);
				let level = ((clamped * SPARK_LEVELS.len() as f64).floor() as usize).min(SPARK_LEVELS.len() - 1);
				paint_status(p, status_of(Some(clamped), None), SPARK_LEVELS[level])
			},
		})
		.collect()
}

fn pad_end(text: &str, width: usize) -> String {
	let len = text.chars().count();
	format!("{text}{}", " ".repeat(width.saturating_sub(len)))
}

fn pad_start(text: &str, width: usize) -> String {
	let len = text.chars().count();
	format!("{}{text}", " ".repeat(width.saturating_sub(len)))
}

fn format_history(
	entries: &[HistoryEntry],
	since: f64,
	now: f64,
	redaction: Option<&HashMap<String, String>>,
) -> String {
	type Series<'a> = (String, Vec<&'a HistoryEntry>);
	let mut providers: BTreeMap<&str, Vec<(String, String, Vec<(String, Series)>)>> = BTreeMap::new();
	for entry in entries {
		let accounts = providers.entry(&entry.provider).or_default();
		let account = match accounts.iter().position(|(key, ..)| *key == entry.account_key) {
			Some(i) => &mut accounts[i],
			None => {
				let label = entry
					.email
					.clone()
					.or_else(|| entry.account_id.clone())
					.unwrap_or_else(|| entry.account_key.clone());
				accounts.push((entry.account_key.clone(), label, Vec::new()));
				accounts.last_mut().expect("pushed")
			},
		};
		let series = match account.2.iter().position(|(id, _)| *id == entry.limit_id) {
			Some(i) => &mut account.2[i].1,
			None => {
				account.2.push((entry.limit_id.clone(), (String::new(), Vec::new())));
				&mut account.2.last_mut().expect("pushed").1
			},
		};
		series.0 = series_title(entry);
		series.1.push(entry);
	}
	let p = Paint::stdout();
	let mut lines = vec![format!(
		"{}{}",
		p.bold("Usage history"),
		p.dim(&format!(" · last {} · peak per bucket", format_duration(now - since)))
	)];
	for (provider, accounts) in &mut providers {
		lines.push(String::new());
		let noun = if accounts.len() == 1 { "account" } else { "accounts" };
		lines.push(format!(
			"{} {}",
			p.bold_cyan(&provider_name(provider)),
			p.dim(&format!("— {} {noun}", accounts.len()))
		));
		accounts.sort_by(|a, b| a.1.cmp(&b.1));
		for (_, label, series) in accounts.iter_mut() {
			let shown = redaction.and_then(|r| r.get(label.as_str())).unwrap_or(label);
			lines.push(format!("  {}", p.bold(shown)));
			let width = series.iter().map(|(_, s)| s.0.chars().count()).max().unwrap_or(0);
			series.sort_by(|a, b| a.1.0.cmp(&b.1.0));
			for (_, (title, items)) in series.iter() {
				let fractions: Vec<f64> = items.iter().filter_map(|e| e.used_fraction).collect();
				let latest = fractions.last().copied();
				let peak = fractions.iter().copied().reduce(f64::max);
				let status = status_of(latest, items.last().and_then(|e| e.status.as_deref()));
				let mut details = Vec::new();
				if let Some(f) = latest {
					details.push(format!("latest {:.1}%", f * 100.0));
				}
				if let Some(f) = peak {
					details.push(format!("peak {:.1}%", f * 100.0));
				}
				let plural = if items.len() == 1 { "" } else { "s" };
				details.push(format!("{} snapshot{plural}", items.len()));
				lines.push(format!(
					"      {} {}  {}  {}",
					paint_status(&p, status, "●"),
					pad_end(title, width),
					sparkline(&p, items, since, now),
					p.dim(&details.join(" · "))
				));
			}
		}
	}
	lines.join("\n")
}

fn history(args: &UsageArgs) -> Result<u8> {
	let (now, since) = window(args);
	let provider = args.provider.as_deref().map(str::to_lowercase);
	let mut entries = Vec::new();
	if let Some(db) = open_agent_db(false)? {
		let mut stmt = db.prepare(
			"SELECT recorded_at, provider, account_key, email, account_id, limit_id, label, window_label, \
			 used_fraction, status, resets_at FROM usage_history WHERE recorded_at >= ? AND (? IS NULL OR \
			 provider = ?) ORDER BY recorded_at ASC",
		)?;
		let rows = stmt.query_map(params![since as i64, provider, provider], |r| {
			Ok(HistoryEntry {
				recorded_at:   r.get::<_, i64>(0)? as f64,
				provider:      r.get(1)?,
				account_key:   r.get(2)?,
				email:         r.get(3)?,
				account_id:    r.get(4)?,
				limit_id:      r.get(5)?,
				label:         r.get(6)?,
				window_label:  r.get(7)?,
				used_fraction: r.get(8)?,
				status:        r.get(9)?,
				resets_at:     r.get::<_, Option<i64>>(10)?.map(|v| v as f64),
			})
		})?;
		for row in rows {
			entries.push(row?);
		}
	}
	let redaction = args.redact.then(|| {
		build_redaction_map(entries.iter().flat_map(|e| {
			[e.email.clone(), e.account_id.clone(), Some(e.account_key.clone())]
				.into_iter()
				.flatten()
				.filter(|v| !v.is_empty())
		}))
	});
	if args.json {
		let mask = |v: &Option<String>| -> Value {
			match (v, &redaction) {
				(None, _) => Value::Null,
				(Some(v), Some(r)) => Value::from(r.get(v).cloned().unwrap_or_else(|| v.clone())),
				(Some(v), None) => Value::from(v.clone()),
			}
		};
		let list: Vec<Value> = entries
			.iter()
			.map(|e| {
				let mut obj = json!({
					"recordedAt": e.recorded_at,
					"provider": e.provider,
					"accountKey": mask(&Some(e.account_key.clone())),
					"limitId": e.limit_id,
					"label": e.label,
				});
				let map = obj.as_object_mut().expect("object");
				let mut put = |key: &str, value: Value| {
					if !value.is_null() {
						map.insert(key.into(), value);
					}
				};
				put("email", mask(&e.email));
				put("accountId", mask(&e.account_id));
				put("windowLabel", e.window_label.clone().map_or(Value::Null, Value::from));
				put("usedFraction", e.used_fraction.map_or(Value::Null, Value::from));
				put("status", e.status.clone().map_or(Value::Null, Value::from));
				put("resetsAt", e.resets_at.map_or(Value::Null, Value::from));
				obj
			})
			.collect();
		let payload = json!({"generatedAt": now as i64, "sinceMs": since as i64, "entries": list});
		println!("{}", serde_json::to_string_pretty(&payload)?);
		return Ok(0);
	}
	if entries.is_empty() {
		let scope = args
			.provider
			.as_ref()
			.map(|p| format!(" for provider \"{p}\""))
			.unwrap_or_default();
		eprintln!(
			"{}",
			Paint::stderr().yellow(&format!(
				"No usage history recorded{scope} yet. Snapshots accumulate whenever usage is fetched \
				 (TUI footer, /usage, harvest usage)."
			))
		);
		return Ok(1);
	}
	println!("{}", format_history(&entries, since, now, redaction.as_ref()));
	Ok(0)
}

// ── clients ──────────────────────────────────────────────────────────────

struct ProviderUsage {
	app:         Option<String>,
	provider:    String,
	requests:    f64,
	input:       f64,
	output:      f64,
	cache_read:  f64,
	cache_write: f64,
	cost_usd:    f64,
}

struct ClientSummary {
	install_id: String,
	hostname:   Option<String>,
	first_seen: f64,
	last_seen:  f64,
	providers:  Vec<ProviderUsage>,
}

fn token_count(value: f64) -> String {
	if value >= 1e9 {
		format!("{:.2}B", value / 1e9)
	} else if value >= 1e6 {
		format!("{:.1}M", value / 1e6)
	} else if value >= 1e3 {
		format!("{:.1}k", value / 1e3)
	} else {
		format!("{}", value.round())
	}
}

fn load_clients(since: f64) -> Result<Vec<ClientSummary>> {
	let Some(db) = open_agent_db(false)? else { return Ok(Vec::new()) };
	let mut clients: Vec<ClientSummary> = db
		.prepare("SELECT install_id, hostname, first_seen, last_seen FROM clients ORDER BY last_seen DESC")?
		.query_map([], |r| {
			Ok(ClientSummary {
				install_id: r.get(0)?,
				hostname:   r.get(1)?,
				first_seen: r.get::<_, i64>(2)? as f64,
				last_seen:  r.get::<_, i64>(3)? as f64,
				providers:  Vec::new(),
			})
		})?
		.collect::<rusqlite::Result<_>>()?;
	let mut stmt = db.prepare(
		"SELECT install_id, app, provider, SUM(requests), SUM(input_tokens), SUM(output_tokens), \
		 SUM(cache_read_tokens), SUM(cache_write_tokens), SUM(cost_usd) FROM client_usage WHERE \
		 recorded_at >= ? GROUP BY install_id, app, provider ORDER BY install_id, SUM(input_tokens + \
		 output_tokens + cache_read_tokens + cache_write_tokens) DESC",
	)?;
	let rows = stmt.query_map(params![since as i64], |r| {
		let app: String = r.get(1)?;
		Ok((r.get::<_, String>(0)?, ProviderUsage {
			app:         (!app.is_empty()).then_some(app),
			provider:    r.get(2)?,
			requests:    r.get::<_, i64>(3)? as f64,
			input:       r.get::<_, i64>(4)? as f64,
			output:      r.get::<_, i64>(5)? as f64,
			cache_read:  r.get::<_, i64>(6)? as f64,
			cache_write: r.get::<_, i64>(7)? as f64,
			cost_usd:    r.get(8)?,
		}))
	})?;
	for row in rows {
		let (install, usage) = row?;
		if let Some(client) = clients.iter_mut().find(|c| c.install_id == install) {
			client.providers.push(usage);
		}
	}
	Ok(clients)
}

fn clients(args: &UsageArgs) -> Result<u8> {
	let (now, since) = window(args);
	let clients = load_clients(since)?;
	if args.json {
		let list: Vec<Value> = clients
			.iter()
			.map(|c| {
				json!({
					"installId": c.install_id,
					"hostname": c.hostname,
					"firstSeen": c.first_seen,
					"lastSeen": c.last_seen,
					"providers": c.providers.iter().map(|u| json!({
						"app": u.app,
						"provider": u.provider,
						"requests": u.requests,
						"inputTokens": u.input,
						"outputTokens": u.output,
						"cacheReadTokens": u.cache_read,
						"cacheWriteTokens": u.cache_write,
						"costUsd": u.cost_usd,
					})).collect::<Vec<_>>(),
				})
			})
			.collect();
		let payload = json!({"generatedAt": now as i64, "sinceMs": since as i64, "clients": list});
		println!("{}", serde_json::to_string_pretty(&payload)?);
		return Ok(0);
	}
	if clients.is_empty() {
		eprintln!(
			"{}",
			Paint::stderr().yellow(
				"No per-client usage recorded yet. Broker-connected clients and the auth-gateway \
				 report token burn automatically; set OMP_AUTH_BROKER_URL (or run this on the broker \
				 host)."
			)
		);
		return Ok(1);
	}
	let p = Paint::stdout();
	let date = chrono::DateTime::from_timestamp_millis(since as i64)
		.map(|d| d.format("%Y-%m-%d").to_string())
		.unwrap_or_default();
	let mut lines = vec![p.bold(&format!("Per-client token burn since {date}"))];
	let headers = ["app", "provider", "requests", "input", "output", "cache r", "cache w", "total", "est cost"];
	for client in &clients {
		let label = client.hostname.clone().unwrap_or_else(|| client.install_id.clone());
		let id_note = if client.hostname.is_some() {
			format!(" · {}", prefix_chars(&client.install_id, 8))
		} else {
			String::new()
		};
		let last_seen = format!("last seen {} ago", format_duration((now - client.last_seen).max(0.0)));
		lines.push(String::new());
		lines.push(format!("{}{} {}", p.cyan(&label), p.dim(&id_note), p.dim(&format!("· {last_seen}"))));
		if client.providers.is_empty() {
			lines.push(p.dim("  no usage in this window"));
			continue;
		}
		let mut rows: Vec<Vec<String>> = Vec::new();
		let (mut requests, mut tokens, mut cost) = (0.0, 0.0, 0.0);
		for u in &client.providers {
			let total = u.input + u.output + u.cache_read + u.cache_write;
			requests += u.requests;
			tokens += total;
			cost += u.cost_usd;
			rows.push(vec![
				u.app.clone().unwrap_or_else(|| "—".into()),
				u.provider.clone(),
				format_number(u.requests),
				token_count(u.input),
				token_count(u.output),
				token_count(u.cache_read),
				token_count(u.cache_write),
				token_count(total),
				format!("${:.2}", u.cost_usd),
			]);
		}
		rows.push(vec![
			String::new(),
			"total".into(),
			format_number(requests),
			String::new(),
			String::new(),
			String::new(),
			String::new(),
			token_count(tokens),
			format!("${cost:.2}"),
		]);
		let widths: Vec<usize> = (0..headers.len())
			.map(|c| {
				rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0).max(headers[c].len())
			})
			.collect();
		let render = |cells: &[String]| {
			let parts: Vec<String> = cells
				.iter()
				.enumerate()
				.map(|(c, cell)| if c < 2 { pad_end(cell, widths[c]) } else { pad_start(cell, widths[c]) })
				.collect();
			format!("  {}", parts.join("  "))
		};
		let header_cells: Vec<String> = headers.iter().map(|h| (*h).to_owned()).collect();
		lines.push(p.dim(&render(&header_cells)));
		let last = rows.len() - 1;
		for (i, row) in rows.iter().enumerate() {
			let rendered = render(row);
			lines.push(if i == last { p.bold(&rendered) } else { rendered });
		}
	}
	println!("{}", lines.join("\n"));
	Ok(0)
}
