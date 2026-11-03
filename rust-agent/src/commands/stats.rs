//! `harvest stats` (`commands/stats.ts` + `cli/stats-cli.ts`, queries from
//! `packages/stats/src/db.ts`).
//!
//! Reads the existing `stats.db` read-only. Session sync and the dashboard
//! server are not yet ported: `--summary` and `--json` (overall/byModel/byFolder
//! only) work; the dashboard mode fails with a pointer to the TypeScript CLI.

use anyhow::{Context, Result, bail};
use clap::Args;
use rusqlite::{Connection, OpenFlags, Row};
use serde_json::{Value, json};

use super::{Paint, format_duration, format_number};

#[derive(Args, Debug)]
pub struct StatsArgs {
	/// Port for the dashboard server
	#[arg(short, long, default_value_t = 3847)]
	port:    u16,
	/// Host to bind
	#[arg(long, default_value = "127.0.0.1")]
	host:    String,
	/// Output stats as JSON
	#[arg(short, long)]
	json:    bool,
	/// Print summary to console
	#[arg(short, long)]
	summary: bool,
}

const UNPRICED: &str =
	"CASE WHEN provider = 'xai-oauth' AND total_tokens > 0 AND cost_total = 0 THEN 1 ELSE 0 END";

fn aggregate_columns() -> String {
	format!(
		"COUNT(*), SUM(CASE WHEN stop_reason = 'error' THEN 1 ELSE 0 END), SUM(input_tokens), \
		 SUM(output_tokens), SUM(cache_read_tokens), SUM(cache_write_tokens), \
		 SUM(premium_requests), SUM(cost_total), SUM({UNPRICED}), SUM(CASE WHEN \
		 cost_no_cache_input > 0 THEN cost_input + cost_cache_read + cost_cache_write ELSE 0 END), \
		 SUM(cost_no_cache_input), AVG(duration), AVG(ttft), AVG(CASE WHEN duration > 0 THEN \
		 output_tokens * 1000.0 / duration ELSE NULL END), MIN(timestamp), MAX(timestamp)"
	)
}

/// `buildAggregatedStats` from column `offset` onward.
fn aggregated(row: &Row<'_>, o: usize) -> rusqlite::Result<Value> {
	let f = |i: usize| -> rusqlite::Result<f64> { Ok(row.get::<_, Option<f64>>(o + i)?.unwrap_or(0.0)) };
	let opt = |i: usize| row.get::<_, Option<f64>>(o + i);
	let total = f(0)?;
	let failed = f(1)?;
	let input = f(2)?;
	let cache_read = f(4)?;
	let cached_prompt = f(9)?;
	let no_cache = f(10)?;
	Ok(json!({
		"totalRequests": total,
		"successfulRequests": total - failed,
		"failedRequests": failed,
		"errorRate": if total > 0.0 { failed / total } else { 0.0 },
		"totalInputTokens": input,
		"totalOutputTokens": f(3)?,
		"totalCacheReadTokens": cache_read,
		"totalCacheWriteTokens": f(5)?,
		"cacheRate": if input + cache_read > 0.0 { cache_read / (input + cache_read) } else { 0.0 },
		"cacheSavings": if no_cache > 0.0 { (no_cache - cached_prompt) / no_cache } else { 0.0 },
		"totalCost": f(7)?,
		"unpricedRequests": f(8)?,
		"totalPremiumRequests": f(6)?,
		"avgDuration": opt(11)?,
		"avgTtft": opt(12)?,
		"avgTokensPerSecond": opt(13)?,
		"firstTimestamp": f(14)?,
		"lastTimestamp": f(15)?,
	}))
}

fn open_db() -> Result<Option<Connection>> {
	let path = super::root_dir()?.join("stats.db");
	if !path.is_file() {
		return Ok(None);
	}
	let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
		.with_context(|| format!("open {}", path.display()))?;
	Ok(Some(db))
}

fn dashboard_stats(db: Option<&Connection>) -> Result<Value> {
	let cols = aggregate_columns();
	let Some(db) = db else {
		return Ok(json!({"overall": Value::Null, "byModel": [], "byFolder": []}));
	};
	let overall = db.query_row(&format!("SELECT {cols} FROM messages"), [], |r| aggregated(r, 0))?;
	let mut stmt = db.prepare(&format!(
		"SELECT model, provider, {cols} FROM messages GROUP BY model, provider ORDER BY COUNT(*) DESC"
	))?;
	let by_model = stmt
		.query_map([], |r| {
			let mut v = aggregated(r, 2)?;
			v["model"] = r.get::<_, String>(0)?.into();
			v["provider"] = r.get::<_, String>(1)?.into();
			Ok(v)
		})?
		.collect::<rusqlite::Result<Vec<_>>>()?;
	let mut stmt = db.prepare(&format!(
		"SELECT folder, {cols} FROM messages GROUP BY folder ORDER BY COUNT(*) DESC"
	))?;
	let by_folder = stmt
		.query_map([], |r| {
			let mut v = aggregated(r, 1)?;
			v["folder"] = r.get::<_, String>(0)?.into();
			Ok(v)
		})?
		.collect::<rusqlite::Result<Vec<_>>>()?;
	Ok(json!({"overall": overall, "byModel": by_model, "byFolder": by_folder}))
}

fn format_cost(n: f64) -> String {
	if n < 0.01 {
		format!("${n:.4}")
	} else if n < 1.0 {
		format!("${n:.3}")
	} else {
		format!("${n:.2}")
	}
}

fn pct(r: f64) -> String {
	format!("{:.1}%", r * 100.0)
}

pub fn execute(args: StatsArgs) -> Result<u8> {
	let db = open_db()?;
	if args.json {
		println!("{}", serde_json::to_string_pretty(&dashboard_stats(db.as_ref())?)?);
		return Ok(0);
	}
	if args.summary {
		print_summary(&dashboard_stats(db.as_ref())?);
		return Ok(0);
	}
	let host_ok = args.host == "localhost"
		|| args.host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
	if !host_ok {
		bail!("stats dashboard binds loopback only (got '{}')", args.host);
	}
	let _ = args.port;
	bail!(
		"the stats dashboard server and session sync are not yet ported to the native \
		 application; use `--summary`/`--json` or the TypeScript CLI"
	)
}

fn print_summary(stats: &Value) {
	let p = Paint::stdout();
	let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
	let dur = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).map_or_else(|| "-".into(), format_duration);
	println!("{}", p.bold("\n=== AI Usage Statistics ===\n"));
	let empty = json!({});
	let o = stats.get("overall").filter(|v| !v.is_null()).unwrap_or(&empty);
	println!("{}", p.bold("Overall:"));
	println!(
		"  Requests: {} ({} errors)",
		format_number(n(o, "totalRequests")),
		format_number(n(o, "failedRequests"))
	);
	println!("  Error Rate: {}", pct(n(o, "errorRate")));
	println!(
		"  Total Tokens: {}",
		format_number(n(o, "totalInputTokens") + n(o, "totalOutputTokens"))
	);
	println!("  Input Tokens: {}", format_number(n(o, "totalInputTokens")));
	println!("  Output Tokens: {}", format_number(n(o, "totalOutputTokens")));
	println!("  Cache Rate: {}", pct(n(o, "cacheRate")));
	println!("  Cache Savings: {}", pct(n(o, "cacheSavings")));
	println!("  Total Cost: {}", format_cost(n(o, "totalCost")));
	let premium = (n(o, "totalPremiumRequests") * 100.0).round() / 100.0;
	println!("  Premium Requests: {}", format_number(premium));
	println!("  Avg Duration: {}", dur(o, "avgDuration"));
	println!("  Avg TTFT: {}", dur(o, "avgTtft"));
	if let Some(tps) = o.get("avgTokensPerSecond").and_then(Value::as_f64) {
		println!("  Avg Tokens/s: {tps:.1}");
	}
	let list = |k: &str| stats.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
	let by_model = list("byModel");
	if !by_model.is_empty() {
		println!("{}", p.bold("\nBy Model:"));
		for m in by_model.iter().take(10) {
			println!(
				"  {}: {} reqs, {}, {} cache rate, {} cache savings",
				m.get("model").and_then(Value::as_str).unwrap_or(""),
				format_number(n(m, "totalRequests")),
				format_cost(n(m, "totalCost")),
				pct(n(m, "cacheRate")),
				pct(n(m, "cacheSavings"))
			);
		}
	}
	let by_folder = list("byFolder");
	if !by_folder.is_empty() {
		println!("{}", p.bold("\nBy Folder:"));
		for f in by_folder.iter().take(10) {
			println!(
				"  {}: {} reqs, {}",
				f.get("folder").and_then(Value::as_str).unwrap_or(""),
				format_number(n(f, "totalRequests")),
				format_cost(n(f, "totalCost"))
			);
		}
	}
	println!();
}
