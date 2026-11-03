//! `harvest grievances` — view, clean, and push auto-QA tool reports (`grievances-cli.ts`).
use std::{
	io::{IsTerminal, Write},
	time::Duration,
};

use anyhow::Result;
use clap::{Args, ValueEnum};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::Paint;

const FLUSH_BATCH_SIZE: i64 = 50;
const FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum GrievanceAction {
	#[default]
	List,
	Clean,
	Push,
}

#[derive(Debug, Args)]
pub struct GrievancesArgs {
	/// list (default), clean, or push
	#[arg(value_enum, default_value = "list")]
	pub action: GrievanceAction,
	/// Number of recent issues to show (list)
	#[arg(short = 'n', long, default_value_t = 20)]
	pub limit:  i64,
	/// Filter by tool name (list, clean)
	#[arg(short, long)]
	pub tool:   Option<String>,
	/// Output as JSON
	#[arg(short, long)]
	pub json:   bool,
	/// Delete a single grievance by id (clean)
	#[arg(long)]
	pub id:     Option<i64>,
	/// Delete every grievance (clean)
	#[arg(long)]
	pub all:    bool,
}

const NO_DB: &str =
	"No grievances database found. Auto-QA has not recorded any reports yet (or was disabled).";

fn open_db() -> Option<Connection> {
	let dir = super::root_dir().ok()?;
	std::fs::create_dir_all(&dir).ok()?;
	let db = Connection::open(dir.join("autoqa.db")).ok()?;
	db.busy_timeout(Duration::from_secs(5)).ok()?;
	db.execute_batch(
		"CREATE TABLE IF NOT EXISTS grievances (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			model TEXT NOT NULL,
			version TEXT NOT NULL,
			tool TEXT NOT NULL,
			report TEXT NOT NULL,
			created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
			pushed INTEGER NOT NULL DEFAULT 0
		);",
	)
	.ok()?;
	let has_created_at: bool = db
		.query_row(
			"SELECT 1 FROM pragma_table_info('grievances') WHERE name = 'created_at'",
			[],
			|_| Ok(true),
		)
		.unwrap_or(false);
	if !has_created_at {
		db.execute_batch(
			"ALTER TABLE grievances ADD COLUMN created_at TEXT NOT NULL DEFAULT '';
			 UPDATE grievances SET created_at = CURRENT_TIMESTAMP WHERE created_at = '';",
		)
		.ok()?;
	}
	db.execute_batch(
		"CREATE INDEX IF NOT EXISTS grievances_pushed_created_at_idx ON grievances (pushed, created_at, id);",
	)
	.ok()?;
	Some(db)
}

struct Row {
	id:      i64,
	model:   String,
	version: String,
	tool:    String,
	report:  String,
}

impl Row {
	fn json(&self) -> Value {
		json!({"id": self.id, "model": self.model, "version": self.version, "tool": self.tool, "report": self.report})
	}
}

fn query_rows(db: &Connection, sql: &str, bind: impl rusqlite::Params) -> Result<Vec<Row>> {
	let mut stmt = db.prepare(sql)?;
	let rows = stmt
		.query_map(bind, |r| {
			Ok(Row { id: r.get(0)?, model: r.get(1)?, version: r.get(2)?, tool: r.get(3)?, report: r.get(4)? })
		})?
		.collect::<rusqlite::Result<Vec<_>>>()?;
	Ok(rows)
}

pub fn execute(args: GrievancesArgs) -> Result<u8> {
	match args.action {
		GrievanceAction::List => list(&args),
		GrievanceAction::Clean => clean(&args),
		GrievanceAction::Push => push(&args),
	}
}

fn list(args: &GrievancesArgs) -> Result<u8> {
	let p = Paint::stdout();
	let Some(db) = open_db() else {
		println!("{}", if args.json { "[]".to_owned() } else { p.dim(NO_DB) });
		return Ok(0);
	};
	let rows = if let Some(tool) = &args.tool {
		query_rows(
			&db,
			"SELECT id, model, version, tool, report FROM grievances WHERE tool = ? ORDER BY id DESC LIMIT ?",
			params![tool, args.limit],
		)?
	} else {
		query_rows(
			&db,
			"SELECT id, model, version, tool, report FROM grievances ORDER BY id DESC LIMIT ?",
			params![args.limit],
		)?
	};
	if args.json {
		let values: Vec<Value> = rows.iter().map(Row::json).collect();
		println!("{}", serde_json::to_string_pretty(&values)?);
		return Ok(0);
	}
	if rows.is_empty() {
		println!("{}", p.dim("No grievances recorded yet."));
		return Ok(0);
	}
	for row in &rows {
		println!(
			"{} {} {}",
			p.dim(&format!("#{}", row.id)),
			p.cyan(&row.tool),
			p.dim(&format!("({} v{})", row.model, row.version))
		);
		println!("  {}", row.report);
		println!();
	}
	let scope = args.tool.as_ref().map(|t| format!(" for {t}")).unwrap_or_default();
	println!("{}", p.dim(&format!("Showing {} most recent{scope}", rows.len())));
	Ok(0)
}

fn clean(args: &GrievancesArgs) -> Result<u8> {
	let p = Paint::stdout();
	let tool = args.tool.as_deref().filter(|t| !t.is_empty());
	let selectors = [args.id.is_some(), tool.is_some(), args.all].iter().filter(|b| **b).count();
	if selectors == 0 {
		eprintln!("{}", Paint::stderr().red("Specify exactly one of --id, --tool, or --all."));
		return Ok(1);
	}
	if selectors > 1 {
		eprintln!("{}", Paint::stderr().red("--id, --tool, and --all are mutually exclusive."));
		return Ok(1);
	}
	let Some(db) = open_db() else {
		println!("{}", if args.json { json!({"deleted": 0}).to_string() } else { p.dim(NO_DB) });
		return Ok(0);
	};
	let deleted = if let Some(id) = args.id {
		db.execute("DELETE FROM grievances WHERE id = ?", params![id])?
	} else if let Some(tool) = tool {
		db.execute("DELETE FROM grievances WHERE tool = ?", params![tool])?
	} else {
		let n = db.execute("DELETE FROM grievances", [])?;
		let _ = db.execute("DELETE FROM sqlite_sequence WHERE name = 'grievances'", []);
		n
	};
	if args.json {
		println!("{}", json!({ "deleted": deleted }));
		return Ok(0);
	}
	if deleted == 0 {
		println!("{}", p.dim("No matching grievances to delete."));
		return Ok(0);
	}
	let scope = match (args.id, tool) {
		(Some(id), _) => format!("#{id}"),
		(None, Some(tool)) => format!("for {tool}"),
		_ => "(all entries)".to_owned(),
	};
	let plural = if deleted == 1 { "" } else { "s" };
	println!("{}", p.green(&format!("Deleted {deleted} grievance{plural} {scope}.")));
	Ok(0)
}

fn env_override(key: &str) -> Option<String> {
	std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn push_config() -> Option<(String, Option<String>)> {
	let cwd = std::env::current_dir().ok()?;
	let agent_dir = super::agent_dir().ok();
	let settings = crate::settings::Settings::load(agent_dir.as_deref(), &cwd, &[]).ok();
	let setting = |path: &str| {
		settings
			.as_ref()
			.and_then(|s| s.get(path))
			.and_then(|v| v.as_str().map(str::to_owned))
	};
	let endpoint = env_override("PI_AUTO_QA_PUSH_URL").or_else(|| setting("dev.autoqaPush.endpoint"))?;
	if endpoint.trim().is_empty() {
		return None;
	}
	let token = env_override("PI_AUTO_QA_PUSH_TOKEN")
		.or_else(|| setting("dev.autoqaPush.token"))
		.filter(|t| !t.is_empty());
	Some((endpoint.trim().to_owned(), token))
}

struct Progress {
	total: i64,
	tty:   bool,
}

impl Progress {
	fn render(&self, done: i64) {
		if !self.tty || self.total == 0 {
			return;
		}
		let width = 30usize;
		let ratio = (done as f64 / self.total as f64).min(1.0);
		let filled = (ratio * width as f64).round() as usize;
		let bar = format!("{}{}", "█".repeat(filled), "░".repeat(width - filled));
		let pct = format!("{:>3}%", (ratio * 100.0).floor() as i64);
		print!("\r{} [{bar}] {pct} {done}/{}", Paint::stdout().cyan("Pushing"), self.total);
		let _ = std::io::stdout().flush();
	}

	fn finish(&self) {
		if self.tty && self.total > 0 {
			println!();
		}
	}
}

fn push(args: &GrievancesArgs) -> Result<u8> {
	let p = Paint::stdout();
	let Some(db) = open_db() else {
		if args.json {
			println!("{}", json!({"pushed": 0, "ok": false, "skipped": true, "reason": "no_db"}));
		} else {
			println!("{}", p.dim("No grievances database found — nothing to push."));
		}
		return Ok(0);
	};
	let Some((endpoint, token)) = push_config() else {
		if args.json {
			println!("{}", json!({"pushed": 0, "ok": false, "skipped": true}));
		} else {
			println!(
				"{}",
				p.yellow(
					"Push skipped — no endpoint configured. Set `dev.autoqaPush.endpoint` or \
					 `PI_AUTO_QA_PUSH_URL`."
				)
			);
		}
		return Ok(0);
	};
	let total: i64 = db.query_row("SELECT COUNT(*) FROM grievances WHERE pushed = 0", [], |r| r.get(0))?;
	let bar = Progress { total, tty: !args.json && std::io::stdout().is_terminal() };
	bar.render(0);
	let agent = ureq::AgentBuilder::new().timeout(FLUSH_TIMEOUT).build();
	let mut pushed = 0i64;
	let ok = loop {
		let rows = query_rows(
			&db,
			"SELECT id, model, version, tool, report FROM grievances WHERE pushed = 0 ORDER BY id ASC LIMIT ?",
			params![FLUSH_BATCH_SIZE],
		)?;
		if rows.is_empty() {
			break true;
		}
		let body = json!({
			"agent": {"name": "omp", "version": env!("CARGO_PKG_VERSION")},
			"platform": std::env::consts::OS,
			"arch": std::env::consts::ARCH,
			"entries": rows.iter().map(Row::json).collect::<Vec<_>>(),
		});
		let mut request = agent.post(&endpoint).set("content-type", "application/json");
		if let Some(token) = &token {
			request = request.set("authorization", &format!("Bearer {token}"));
		}
		if request.send_string(&body.to_string()).is_err() {
			break false;
		}
		let tx = db.unchecked_transaction()?;
		for row in &rows {
			tx.execute("UPDATE grievances SET pushed = 1 WHERE id = ?", params![row.id])?;
		}
		tx.commit()?;
		pushed += rows.len() as i64;
		bar.render(pushed);
	};
	bar.finish();
	if args.json {
		println!("{}", json!({"pushed": pushed, "ok": ok}));
		return Ok(if ok { 0 } else { 1 });
	}
	if total == 0 {
		println!("{}", p.dim("Nothing to push — all grievances are already shipped."));
		return Ok(0);
	}
	if ok {
		let plural = if pushed == 1 { "" } else { "s" };
		println!("{}", p.green(&format!("Pushed {pushed}/{total} grievance{plural}.")));
		return Ok(0);
	}
	let remaining = total - pushed;
	let plural = if remaining == 1 { "" } else { "s" };
	println!(
		"{}",
		p.red(&format!(
			"Push failed after {pushed}/{total}; {remaining} grievance{plural} remain unpushed."
		))
	);
	Ok(1)
}
