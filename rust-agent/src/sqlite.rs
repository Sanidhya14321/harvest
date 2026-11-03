//! SQLite reads for `read` (`read-sqlite.ts` + `sqlite-reader.ts`): path
//! candidates, the `:table[:key]?limit&offset&order&where` / `?q=` selector
//! grammar with its injection guards, a query-only connection, and the legacy
//! table, schema, row and listing renderers.
use std::path::Path;

use anyhow::{Result, anyhow, bail};
use harvest_runtime::JailedFs;
use rusqlite::{Connection, OpenFlags, types::ValueRef};

use crate::output::{ToolOutput, format_bytes};

const MAGIC: &[u8; 16] = b"SQLite format 3\0";
const DEFAULT_QUERY_LIMIT: u64 = 20;
const DEFAULT_SCHEMA_SAMPLE_LIMIT: u64 = 5;
const MAX_QUERY_LIMIT: u64 = 500;
const MAX_RAW_QUERY_ROWS: usize = 1000;
const MAX_RENDER_WIDTH: u32 = 120;
const MAX_COLUMN_WIDTH: usize = 40;
const MIN_COLUMN_WIDTH: usize = 3;
const COLUMN_SEPARATOR_WIDTH: usize = 3;
const TABLE_FRAME_WIDTH: usize = 1;
const ROW_COUNT_PROBE_CAP: u64 = 50_000;
const LIST_LIMIT: usize = 500;
/// `DEFAULT_MAX_LINES`.
const DEFAULT_MAX_LINES: u64 = 3000;
const TAB_WIDTH: u32 = harvest_tui::text::DEFAULT_TAB_WIDTH;

enum Selector {
	List,
	Schema {
		table:  String,
		sample: u64,
	},
	Row {
		table: String,
		key:   String,
	},
	Query {
		table:  String,
		limit:  u64,
		offset: u64,
		order:  Option<String>,
		filter: Option<String>,
	},
	Raw {
		sql: String,
	},
}

/// `parseSqlitePathCandidates`: `(db path, sub path, query string)`, longest
/// database path first.
fn candidates(path: &str) -> Vec<(String, String, String)> {
	let normalized = path.replace('\\', "/");
	let lower = normalized.to_lowercase();
	let mut out: Vec<(String, String, String)> = Vec::new();
	let mut search = 0;
	while let Some(dot) = lower[search..].find('.').map(|i| search + i) {
		search = dot + 1;
		for ext in ["sqlite3", "sqlite", "db3", "db"] {
			let end = dot + 1 + ext.len();
			if lower.get(dot + 1..end) != Some(ext) {
				continue;
			}
			if !(end == lower.len() || matches!(lower.as_bytes()[end], b':' | b'?')) {
				continue;
			}
			let remainder = &normalized[end..];
			let (sub, query) = remainder
				.split_once('?')
				.map_or((remainder, ""), |(sub, query)| (sub, query));
			let candidate =
				(path[..end].to_owned(), sub.trim_start_matches(':').to_owned(), query.to_owned());
			if !out.contains(&candidate) {
				out.push(candidate);
			}
			break;
		}
	}
	out.sort_by_key(|candidate| std::cmp::Reverse(candidate.0.len()));
	out
}

const FORBIDDEN_KEYWORDS: [&str; 8] =
	["limit", "offset", "union", "intersect", "except", "attach", "detach", "pragma"];
const TERMINATOR_ERROR: &str = "SQLite 'where' clause must not contain comments or statement \
                                terminators; use '?q=SELECT ...' for raw SQL";
const KEYWORD_ERROR: &str = "SQLite 'where' clause must not contain \
                             LIMIT/OFFSET/UNION/INTERSECT/EXCEPT/ATTACH/DETACH/PRAGMA; use \
                             '?q=SELECT ...' for raw SQL";

/// `findWhereClauseViolation`.
fn where_violation(sql: &str) -> Option<&'static str> {
	let chars: Vec<char> = sql.chars().collect();
	let (mut single, mut double) = (false, false);
	let mut token = String::new();
	let mut keyword = None;
	let mut index = 0;
	while index <= chars.len() {
		let ch = chars.get(index).copied();
		let next = chars.get(index + 1).copied();
		if single || double {
			let quote = if single { '\'' } else { '"' };
			if ch == Some(quote) && next == Some(quote) {
				index += 2;
				continue;
			}
			if ch == Some(quote) {
				single = false;
				double = false;
			}
			index += 1;
			continue;
		}
		if let Some(c) = ch.filter(|c| c.is_ascii_alphanumeric() || *c == '_') {
			token.push(c);
			index += 1;
			continue;
		}
		if keyword.is_none() && FORBIDDEN_KEYWORDS.contains(&token.to_lowercase().as_str()) {
			keyword = Some(KEYWORD_ERROR);
		}
		token.clear();
		match (ch, next) {
			(None, _) => break,
			(Some('\''), _) => single = true,
			(Some('"'), _) => double = true,
			(Some(';'), _)
			| (Some('-'), Some('-'))
			| (Some('/'), Some('*'))
			| (Some('*'), Some('/')) => {
				return Some(TERMINATOR_ERROR);
			},
			_ => {},
		}
		index += 1;
	}
	keyword
}

fn validate_where(value: Option<&str>) -> Result<Option<String>> {
	let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
		return Ok(None);
	};
	if let Some(violation) = where_violation(value) {
		bail!("{violation}");
	}
	Ok(Some(value.to_owned()))
}

fn parse_int(value: &str) -> Option<i64> {
	let trimmed = value.trim_start();
	let digits: String = trimmed
		.char_indices()
		.take_while(|(i, c)| c.is_ascii_digit() || (*i == 0 && (*c == '-' || *c == '+')))
		.map(|(_, c)| c)
		.collect();
	digits.parse().ok()
}

fn parse_limit(value: Option<&str>) -> Result<u64> {
	let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
		return Ok(DEFAULT_QUERY_LIMIT);
	};
	match parse_int(value) {
		Some(parsed) if parsed >= 1 => Ok((parsed as u64).min(MAX_QUERY_LIMIT)),
		_ => bail!("SQLite limit must be a positive integer; got '{value}'"),
	}
}

fn parse_offset(value: Option<&str>) -> Result<u64> {
	let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
		return Ok(0);
	};
	match parse_int(value) {
		Some(parsed) if parsed >= 0 => Ok(parsed as u64),
		_ => bail!("SQLite offset must be a non-negative integer; got '{value}'"),
	}
}

/// `parseSqliteSelector`.
fn parse_selector(sub: &str, query: &str) -> Result<Selector> {
	let sub = sub.trim_start_matches(':').trim();
	let params: Vec<(String, String)> = form_urlencoded::parse(query.as_bytes())
		.into_owned()
		.collect();
	let get = |key: &str| {
		params
			.iter()
			.find(|(k, _)| k == key)
			.map(|(_, v)| v.as_str())
	};
	let has = |key: &str| params.iter().any(|(k, _)| k == key);
	if let Some(raw) = get("q") {
		if !sub.is_empty() || params.iter().any(|(k, _)| k != "q") {
			bail!("SQLite raw queries cannot be combined with table selectors or pagination");
		}
		if raw.trim().is_empty() {
			bail!("SQLite query parameter 'q' cannot be empty");
		}
		return Ok(Selector::Raw { sql: raw.to_owned() });
	}
	if sub.is_empty() {
		if !params.is_empty() {
			bail!("SQLite query parameters require a table selector or q=SELECT...");
		}
		return Ok(Selector::List);
	}
	let (table, key) = sub
		.split_once(':')
		.map_or((sub, None), |(t, k)| (t, Some(k)));
	if table.is_empty() {
		bail!("SQLite selectors must include a table name");
	}
	if let Some(key) = key.filter(|key| !key.is_empty()) {
		if !params.is_empty() {
			bail!("SQLite row lookups cannot be combined with query parameters");
		}
		return Ok(Selector::Row { table: table.to_owned(), key: key.to_owned() });
	}
	let filter = validate_where(get("where"))?;
	let order = get("order")
		.map(str::trim)
		.filter(|o| !o.is_empty())
		.map(str::to_owned);
	if has("limit") || has("offset") || order.is_some() || filter.is_some() {
		if let Some((key, _)) = params
			.iter()
			.find(|(k, _)| !matches!(k.as_str(), "limit" | "offset" | "order" | "where"))
		{
			bail!("Unsupported SQLite query parameter '{key}'");
		}
		return Ok(Selector::Query {
			table: table.to_owned(),
			limit: parse_limit(get("limit"))?,
			offset: parse_offset(get("offset"))?,
			order,
			filter,
		});
	}
	if let Some((key, _)) = params.first() {
		bail!("Unsupported SQLite query parameter '{key}'");
	}
	Ok(Selector::Schema { table: table.to_owned(), sample: DEFAULT_SCHEMA_SAMPLE_LIMIT })
}

fn quote(identifier: &str) -> String {
	format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn utf16(text: &str) -> Vec<u16> {
	text.encode_utf16().collect()
}

fn width(text: &str) -> usize {
	harvest_tui::text::visible_width(&utf16(text), TAB_WIDTH)
}

fn truncate(text: &str, max: u32) -> String {
	String::from_utf16_lossy(&harvest_tui::text::truncate_to_width(
		&utf16(text),
		max,
		None,
		None,
		TAB_WIDTH,
	))
}

fn replace_tabs(text: &str) -> String {
	text.replace('\t', &" ".repeat(TAB_WIDTH as usize))
}

fn sanitize(value: &str) -> String {
	replace_tabs(value)
		.replace("\r\n", "\\n")
		.replace('\n', "\\n")
}

/// `stringifySqliteValue`.
fn stringify(value: ValueRef<'_>) -> String {
	match value {
		ValueRef::Null => "NULL".to_owned(),
		ValueRef::Integer(n) => n.to_string(),
		ValueRef::Real(f) => {
			if f.fract() == 0.0 && f.abs() < 1e21 {
				format!("{f:.0}")
			} else {
				f.to_string()
			}
		},
		ValueRef::Text(bytes) => String::from_utf8_lossy(bytes).into_owned(),
		ValueRef::Blob(bytes) => format!("<BLOB {}>", format_bytes(bytes.len())),
	}
}

struct Rows {
	columns: Vec<String>,
	rows:    Vec<Vec<String>>,
}

fn pad_cell(value: &str, width_target: usize) -> String {
	let truncated = truncate(&sanitize(value), width_target.max(MIN_COLUMN_WIDTH) as u32);
	let visible = width(&truncated);
	if visible >= width_target {
		truncated
	} else {
		format!("{truncated}{}", " ".repeat(width_target - visible))
	}
}

fn vertical_blocks(data: &Rows) -> String {
	if data.rows.is_empty() {
		return "(no rows)".to_owned();
	}
	let name_width = data
		.columns
		.iter()
		.map(|c| width(&sanitize(c)))
		.fold(MIN_COLUMN_WIDTH, usize::max)
		.min(MAX_COLUMN_WIDTH);
	data
		.rows
		.iter()
		.enumerate()
		.map(|(index, row)| {
			let mut block = vec![format!("── Row {} ──", index + 1)];
			for (column, value) in data.columns.iter().zip(row) {
				block.push(truncate(
					&format!("{}: {}", pad_cell(column, name_width), sanitize(value)),
					MAX_RENDER_WIDTH,
				));
			}
			block.join("\n")
		})
		.collect::<Vec<_>>()
		.join("\n\n")
}

/// `buildAsciiTable`.
fn ascii_table(data: &Rows) -> String {
	let columns = &data.columns;
	if columns.is_empty() {
		return if data.rows.is_empty() {
			"(no rows)".to_owned()
		} else {
			"(rows returned without named columns)".to_owned()
		};
	}
	if MIN_COLUMN_WIDTH * columns.len() + COLUMN_SEPARATOR_WIDTH * columns.len() + TABLE_FRAME_WIDTH
		> MAX_RENDER_WIDTH as usize
	{
		return vertical_blocks(data);
	}
	let mut widths: Vec<usize> = columns
		.iter()
		.map(|c| width(&sanitize(c)).clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH))
		.collect();
	for row in &data.rows {
		for (index, value) in row.iter().enumerate() {
			widths[index] = widths[index].max(width(&sanitize(value)).min(MAX_COLUMN_WIDTH));
		}
	}
	let overhead = columns.len() * COLUMN_SEPARATOR_WIDTH + TABLE_FRAME_WIDTH;
	while widths.iter().sum::<usize>() + overhead > MAX_RENDER_WIDTH as usize {
		let mut widest: Option<usize> = None;
		let mut widest_width = MIN_COLUMN_WIDTH;
		for (index, w) in widths.iter().enumerate() {
			if *w > widest_width {
				widest = Some(index);
				widest_width = *w;
			}
		}
		let Some(index) = widest else { break };
		widths[index] = (widths[index] - 1).max(MIN_COLUMN_WIDTH);
	}
	let row_line = |cells: Vec<String>| format!("| {} |", cells.join(" | "));
	let mut lines = vec![
		row_line(
			columns
				.iter()
				.zip(&widths)
				.map(|(c, w)| pad_cell(c, *w))
				.collect(),
		),
		row_line(
			widths
				.iter()
				.map(|w| "-".repeat((*w).max(MIN_COLUMN_WIDTH)))
				.collect(),
		),
	];
	if data.rows.is_empty() {
		lines.push("(no rows)".to_owned());
	}
	for row in &data.rows {
		lines.push(row_line(
			row.iter()
				.zip(&widths)
				.map(|(v, w)| pad_cell(v, *w))
				.collect(),
		));
	}
	lines
		.iter()
		.map(|line| truncate(&replace_tabs(line), MAX_RENDER_WIDTH))
		.collect::<Vec<_>>()
		.join("\n")
}

fn more_rows_notice(remaining: u64, table: &str, limit: u64, offset: u64) -> String {
	truncate(
		&replace_tabs(&format!(
			"[{remaining} more rows; append :{table}?limit={limit}&offset={offset} to the database \
			 path to continue]"
		)),
		MAX_RENDER_WIDTH,
	)
}

fn collect(
	statement: &mut rusqlite::Statement<'_>,
	params: impl rusqlite::Params,
	cap: usize,
) -> Result<(Rows, bool)> {
	let columns: Vec<String> = statement
		.column_names()
		.iter()
		.map(|c| (*c).to_owned())
		.collect();
	let count = columns.len();
	let mut rows = statement.query(params)?;
	let mut out = Vec::new();
	let mut truncated = false;
	while let Some(row) = rows.next()? {
		if out.len() >= cap {
			truncated = true;
			break;
		}
		out.push(
			(0..count)
				.map(|i| row.get_ref(i).map(stringify))
				.collect::<rusqlite::Result<Vec<_>>>()?,
		);
	}
	Ok((Rows { columns, rows: out }, truncated))
}

fn table_sql(db: &Connection, table: &str) -> Result<Option<String>> {
	let mut statement = db.prepare(
		"SELECT name, sql FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND \
		 name = ?",
	)?;
	let mut rows = statement.query([table])?;
	match rows.next()? {
		Some(row) => Ok(row.get::<_, Option<String>>(1)?),
		None => bail!("SQLite table '{table}' not found"),
	}
}

struct ColumnInfo {
	name: String,
	kind: String,
	pk:   i64,
}

fn table_info(db: &Connection, table: &str) -> Result<Vec<ColumnInfo>> {
	table_sql(db, table)?;
	let mut statement = db.prepare(&format!("PRAGMA table_info({})", quote(table)))?;
	let rows = statement.query_map([], |row| {
		Ok(ColumnInfo {
			name: row.get(1)?,
			kind: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
			pk:   row.get(5)?,
		})
	})?;
	Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn query_rows(
	db: &Connection,
	table: &str,
	limit: u64,
	offset: u64,
	order: Option<&str>,
	filter: Option<&str>,
) -> Result<(Rows, u64)> {
	let columns: Vec<String> = table_info(db, table)?.into_iter().map(|c| c.name).collect();
	let filter = validate_where(filter)?;
	let where_clause = filter.map_or_else(String::new, |f| format!(" WHERE {f}"));
	let order_clause = match order.map(str::trim).filter(|o| !o.is_empty()) {
		None => String::new(),
		Some(order) => {
			let (column, direction) = order
				.rsplit_once(':')
				.map_or_else(|| (order, "asc".to_owned()), |(c, d)| (c, d.trim().to_lowercase()));
			if !columns.iter().any(|c| c == column) {
				bail!("SQLite order column '{column}' not found in table schema");
			}
			if direction != "asc" && direction != "desc" {
				bail!("SQLite order direction must be 'asc' or 'desc'; got '{direction}'");
			}
			format!(" ORDER BY {} {}", quote(column), direction.to_uppercase())
		},
	};
	let total: u64 = db.query_row(
		&format!("SELECT COUNT(*) AS count FROM {}{where_clause}", quote(table)),
		[],
		|row| row.get::<_, i64>(0),
	)? as u64;
	let mut statement = db.prepare(&format!(
		"SELECT * FROM {}{where_clause}{order_clause} LIMIT ? OFFSET ?",
		quote(table)
	))?;
	if statement.parameter_count() != 2 {
		bail!(
			"SQLite where clause changed the expected pagination parameters; use q=SELECT ... for \
			 raw SQL"
		);
	}
	let (rows, _) = collect(&mut statement, [limit as i64, offset as i64], usize::MAX)?;
	Ok((rows, total))
}

fn list_tables(db: &Connection) -> Result<Vec<String>> {
	let names: Vec<String> = db
		.prepare(
			"SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER \
			 BY name COLLATE NOCASE",
		)?
		.query_map([], |row| row.get(0))?
		.collect::<rusqlite::Result<_>>()?;
	let mut estimates = std::collections::HashMap::<String, u64>::new();
	let has_stat: bool = db
		.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'sqlite_stat1'")?
		.exists([])?;
	if has_stat {
		let mut statement = db.prepare("SELECT tbl, stat FROM sqlite_stat1")?;
		let rows = statement
			.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))?;
		for row in rows {
			let (table, stat) = row?;
			if let Some(rows) = stat.as_deref().and_then(parse_int).filter(|r| *r >= 0) {
				let entry = estimates.entry(table).or_insert(0);
				*entry = (*entry).max(rows as u64);
			}
		}
	}
	names
		.into_iter()
		.map(|name| {
			let count = match estimates.get(&name) {
				Some(estimate) if *estimate > ROW_COUNT_PROBE_CAP => format!("~{estimate} rows"),
				_ => {
					let counted: i64 = db.query_row(
						&format!(
							"SELECT COUNT(*) AS count FROM (SELECT 1 FROM {} LIMIT {})",
							quote(&name),
							ROW_COUNT_PROBE_CAP + 1
						),
						[],
						|row| row.get(0),
					)?;
					if counted as u64 > ROW_COUNT_PROBE_CAP {
						format!("{ROW_COUNT_PROBE_CAP}+ rows")
					} else {
						format!("{counted} rows")
					}
				},
			};
			Ok(truncate(&replace_tabs(&format!("{name} ({count})")), MAX_RENDER_WIDTH))
		})
		.collect()
}

fn coerce_integer(key: &str, label: &str) -> Result<i64> {
	let trimmed = key.trim();
	let valid = trimmed.strip_prefix('-').unwrap_or(trimmed);
	if valid.is_empty() || !valid.bytes().all(|b| b.is_ascii_digit()) {
		bail!("{label} must be an integer; got '{key}'");
	}
	trimmed
		.parse()
		.map_err(|_| anyhow!("{label} must be an integer; got '{key}'"))
}

fn read_row(db: &Connection, table: &str, key: &str) -> Result<Option<Rows>> {
	let primary: Vec<ColumnInfo> = {
		let mut columns: Vec<ColumnInfo> = table_info(db, table)?
			.into_iter()
			.filter(|c| c.pk > 0)
			.collect();
		columns.sort_by_key(|c| c.pk);
		columns
	};
	let (sql, value): (String, rusqlite::types::Value) = match primary.as_slice() {
		[column] => {
			let kind = column.kind.trim().to_uppercase();
			let value = if kind.contains("INT") {
				rusqlite::types::Value::Integer(coerce_integer(key, &format!("Primary key '{key}'"))?)
			} else if (kind.contains("REAL") || kind.contains("FLOA") || kind.contains("DOUB"))
				&& let Ok(number) = key.trim().parse::<f64>()
			{
				rusqlite::types::Value::Real(number)
			} else {
				rusqlite::types::Value::Text(key.to_owned())
			};
			(
				format!("SELECT * FROM {} WHERE {} = ? LIMIT 1", quote(table), quote(&column.name)),
				value,
			)
		},
		[] => {
			let schema = table_sql(db, table)?.unwrap_or_default();
			let upper = schema.to_uppercase();
			if upper
				.split_whitespace()
				.collect::<Vec<_>>()
				.windows(2)
				.any(|w| w == ["WITHOUT", "ROWID"])
			{
				bail!("SQLite table '{table}' does not expose ROWID; use '?where=' instead");
			}
			(
				format!("SELECT * FROM {} WHERE rowid = ? LIMIT 1", quote(table)),
				rusqlite::types::Value::Integer(coerce_integer(key, "SQLite ROWID")?),
			)
		},
		_ => bail!("SQLite table '{table}' has a composite primary key; use '?where=' instead"),
	};
	let mut statement = db.prepare(&sql)?;
	let (rows, _) = collect(&mut statement, [value], 1)?;
	Ok((!rows.rows.is_empty()).then_some(rows))
}

fn open(path: &Path) -> Result<Connection> {
	let db = Connection::open_with_flags(
		path,
		OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
	)?;
	db.execute_batch("PRAGMA query_only = ON; PRAGMA busy_timeout = 3000;")?;
	db.query_row("SELECT name FROM sqlite_master LIMIT 1", [], |_| Ok(()))
		.or_else(|error| match error {
			rusqlite::Error::QueryReturnedNoRows => Ok(()),
			error => Err(error),
		})?;
	Ok(db)
}

fn render(db: &Connection, selector: Selector) -> Result<ToolOutput> {
	Ok(match selector {
		Selector::List => {
			let mut tables = list_tables(db)?;
			let mut limits = Vec::new();
			if tables.len() >= LIST_LIMIT {
				tables.truncate(LIST_LIMIT);
				limits.push(format!(
					"{LIST_LIMIT} results limit reached. Use limit={} for more",
					LIST_LIMIT * 2
				));
			}
			let text = if tables.is_empty() {
				"(no tables)".to_owned()
			} else {
				tables.join("\n")
			};
			let (text, truncation) = crate::output::head_window(&text, 50 * 1024);
			ToolOutput { text, truncation, next_offset: None, limits }
		},
		Selector::Schema { table, sample } => {
			let (rows, total) = query_rows(db, &table, sample, 0, None, None)?;
			let sql = table_sql(db, &table)?
				.ok_or_else(|| anyhow!("SQLite schema for table '{table}' is unavailable"))?;
			let schema: Vec<String> = replace_tabs(&sql)
				.split('\n')
				.map(|line| truncate(line, MAX_RENDER_WIDTH))
				.collect();
			let mut text = format!("{}\n\nSample rows:\n{}", schema.join("\n"), ascii_table(&rows));
			let shown = rows.rows.len() as u64;
			if shown < total {
				let _ = std::fmt::Write::write_fmt(
					&mut text,
					format_args!(
						"\n[{} more rows; append :{table}?limit=20&offset={shown} to the database path \
						 to continue]",
						total - shown
					),
				);
			}
			ToolOutput::text(text)
		},
		Selector::Row { table, key } => match read_row(db, &table, &key)? {
			None => ToolOutput::text(format!("No row found in table '{table}' for key '{key}'.")),
			Some(rows) => ToolOutput::text(if rows.columns.is_empty() {
				"(no columns)".to_owned()
			} else {
				rows
					.columns
					.iter()
					.zip(&rows.rows[0])
					.map(|(column, value)| {
						truncate(&replace_tabs(&format!("{column}: {value}")), MAX_RENDER_WIDTH)
					})
					.collect::<Vec<_>>()
					.join("\n")
			}),
		},
		Selector::Query { table, limit, offset, order, filter } => {
			let (rows, total) =
				query_rows(db, &table, limit, offset, order.as_deref(), filter.as_deref())?;
			let mut text = ascii_table(&rows);
			let shown = total.min(offset + rows.rows.len() as u64);
			if shown < total {
				text.push('\n');
				text.push_str(&more_rows_notice(
					total - shown,
					&table,
					limit,
					offset + rows.rows.len() as u64,
				));
			}
			ToolOutput::text(text)
		},
		Selector::Raw { sql } => {
			let mut statement = db.prepare(&sql)?;
			if statement.parameter_count() > 0 {
				bail!("SQLite raw queries do not support bound parameters");
			}
			let (rows, truncated) = collect(&mut statement, [], MAX_RAW_QUERY_ROWS)?;
			let _ = DEFAULT_MAX_LINES;
			let mut text = ascii_table(&rows);
			if truncated {
				let _ = std::fmt::Write::write_fmt(
					&mut text,
					format_args!(
						"\n[Output capped at {MAX_RAW_QUERY_ROWS} rows; add a LIMIT/OFFSET clause to \
						 the query to page through more]"
					),
				);
			}
			ToolOutput::text(text)
		},
	})
}

/// `resolveSqliteReadPath` + `readSqlite`; `None` when `path` names no SQLite
/// database.
pub(crate) fn read_sqlite(fs: &JailedFs, path: &str) -> Result<Option<ToolOutput>> {
	for (db_path, sub, query) in candidates(path) {
		let Ok(relative) = fs.relative(Path::new(&db_path)) else {
			continue;
		};
		if !fs.file_type(&relative).is_ok_and(|kind| kind.is_file()) {
			continue;
		}
		let absolute = fs.root().join(&relative);
		let canonical = std::fs::canonicalize(&absolute)?;
		if !canonical.starts_with(fs.root()) {
			bail!("path escapes the authorized root");
		}
		let mut header = [0u8; 16];
		let is_sqlite = std::fs::File::open(&canonical)
			.and_then(|mut file| std::io::Read::read_exact(&mut file, &mut header))
			.is_ok()
			&& &header == MAGIC;
		if !is_sqlite {
			continue;
		}
		let selector = parse_selector(&sub, &query)?;
		let db = open(&canonical).map_err(|error| anyhow!("{error}"))?;
		return render(&db, selector)
			.map(Some)
			.map_err(|error| anyhow!("{error}"));
	}
	Ok(None)
}

fn json_binding(value: &serde_json::Value, column: &str) -> Result<rusqlite::types::Value> {
	use rusqlite::types::Value;
	Ok(match value {
		serde_json::Value::Null => Value::Null,
		serde_json::Value::Bool(flag) => Value::Integer(i64::from(*flag)),
		serde_json::Value::Number(number) => match number.as_i64() {
			Some(integer) => Value::Integer(integer),
			None => Value::Real(number.as_f64().unwrap_or(f64::NAN)),
		},
		serde_json::Value::String(text) => Value::Text(text.clone()),
		_ => bail!("SQLite column '{column}' only accepts JSON scalar values or null"),
	})
}

/// `validateWriteColumns`.
fn write_columns(
	db: &Connection,
	table: &str,
	data: &serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<(String, rusqlite::types::Value)>> {
	let columns: Vec<String> = table_info(db, table)?.into_iter().map(|c| c.name).collect();
	data
		.iter()
		.map(|(column, value)| {
			if !columns.contains(column) {
				bail!("SQLite table '{table}' has no column named '{column}'");
			}
			Ok((column.clone(), json_binding(value, column)?))
		})
		.collect()
}

/// The `WHERE` clause and key binding for a row write
/// (`resolveTableRowLookup`).
fn row_target(db: &Connection, table: &str, key: &str) -> Result<(String, rusqlite::types::Value)> {
	let mut primary: Vec<ColumnInfo> = table_info(db, table)?
		.into_iter()
		.filter(|c| c.pk > 0)
		.collect();
	primary.sort_by_key(|c| c.pk);
	match primary.as_slice() {
		[column] => {
			let kind = column.kind.trim().to_uppercase();
			let value = if kind.contains("INT") {
				rusqlite::types::Value::Integer(coerce_integer(key, &format!("Primary key '{key}'"))?)
			} else if (kind.contains("REAL") || kind.contains("FLOA") || kind.contains("DOUB"))
				&& let Ok(number) = key.trim().parse::<f64>()
			{
				rusqlite::types::Value::Real(number)
			} else {
				rusqlite::types::Value::Text(key.to_owned())
			};
			Ok((format!("{} = ?", quote(&column.name)), value))
		},
		[] => {
			let upper = table_sql(db, table)?.unwrap_or_default().to_uppercase();
			if upper
				.split_whitespace()
				.collect::<Vec<_>>()
				.windows(2)
				.any(|w| w == ["WITHOUT", "ROWID"])
			{
				bail!("SQLite table '{table}' does not expose ROWID; use '?where=' instead");
			}
			Ok((
				"rowid = ?".to_owned(),
				rusqlite::types::Value::Integer(coerce_integer(key, "SQLite ROWID")?),
			))
		},
		_ => bail!("SQLite table '{table}' has a composite primary key; use '?where=' instead"),
	}
}

/// `parseSqliteWriteTarget`.
fn write_target(sub: &str, query: &str) -> Result<(String, Option<String>)> {
	if !query.trim().is_empty() {
		bail!("SQLite write paths do not support query parameters");
	}
	let normalized = sub.trim_start_matches(':').trim();
	if normalized.is_empty() {
		bail!("SQLite write path must target a table");
	}
	let (table, key) = normalized
		.split_once(':')
		.map_or((normalized, None), |(t, k)| (t, Some(k)));
	if table.is_empty() {
		bail!("SQLite write path must target a table");
	}
	if key.is_some_and(str::is_empty) {
		bail!("SQLite row writes require a non-empty row key");
	}
	Ok((table.to_owned(), key.map(str::to_owned)))
}

/// `#resolveSqliteWritePath` + `#writeSqliteRow`: insert (no key), update
/// (key + JSON object) or delete (key + empty content). `None` when the path
/// names no SQLite database.
pub(crate) fn write_sqlite(fs: &JailedFs, path: &str, content: &str) -> Result<Option<String>> {
	let candidates: Vec<_> = candidates(path)
		.into_iter()
		.filter(|(db, ..)| db != path)
		.collect();
	let Some(fallback) = candidates.last() else {
		return Ok(None);
	};
	let fallback_target = write_target(&fallback.1, &fallback.2)?;
	fs.relative(Path::new(&fallback.0))?;
	let mut saw_non_sqlite = false;
	let mut resolved = None;
	for (db_path, sub, query) in &candidates {
		let target = write_target(sub, query)?;
		let relative = fs.relative(Path::new(db_path))?;
		match fs.file_type(&relative) {
			Ok(kind) if kind.is_dir() => continue,
			Ok(_) => {},
			Err(_) => continue,
		}
		let canonical = std::fs::canonicalize(fs.root().join(&relative))?;
		if !canonical.starts_with(fs.root()) {
			bail!("path escapes the authorized root");
		}
		let mut header = [0u8; 16];
		let is_sqlite = std::fs::File::open(&canonical)
			.and_then(|mut file| std::io::Read::read_exact(&mut file, &mut header))
			.is_ok()
			&& &header == MAGIC;
		if !is_sqlite {
			saw_non_sqlite = true;
			continue;
		}
		resolved = Some((canonical, target));
		break;
	}
	let Some((db_path, (table, key))) = resolved else {
		if saw_non_sqlite {
			return Ok(None);
		}
		let _ = fallback_target;
		bail!("SQLite database '{path}' not found");
	};
	let db = Connection::open_with_flags(
		&db_path,
		OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
	)
	.map_err(|error| anyhow!("{error}"))?;
	db.execute_batch("PRAGMA busy_timeout = 3000;")?;
	let run = || -> Result<String> {
		if content.trim().is_empty() {
			let Some(key) = &key else {
				bail!("SQLite deletes require a row key in the path")
			};
			let (clause, value) = row_target(&db, &table, key)?;
			let deleted =
				db.execute(&format!("DELETE FROM {} WHERE {clause}", quote(&table)), [value])?;
			return Ok(if deleted > 0 {
				format!("Deleted row '{key}' from {table}")
			} else {
				format!("No row deleted from {table} for key '{key}'")
			});
		}
		let parsed: serde_json::Value = serde_json::from_str(content)
			.map_err(|error| anyhow!("SQLite write content must be valid JSON5: {error}"))?;
		let serde_json::Value::Object(data) = parsed else {
			bail!("SQLite write content must be a JSON object");
		};
		table_sql(&db, &table)?;
		let entries = write_columns(&db, &table, &data)?;
		if let Some(key) = &key {
			if entries.is_empty() {
				bail!("SQLite updates require at least one column value");
			}
			let (clause, value) = row_target(&db, &table, key)?;
			let assignments: Vec<String> = entries
				.iter()
				.map(|(c, _)| format!("{} = ?", quote(c)))
				.collect();
			let mut bindings: Vec<rusqlite::types::Value> =
				entries.into_iter().map(|(_, v)| v).collect();
			bindings.push(value);
			let updated = db.execute(
				&format!("UPDATE {} SET {} WHERE {clause}", quote(&table), assignments.join(", ")),
				rusqlite::params_from_iter(bindings),
			)?;
			return Ok(if updated > 0 {
				format!("Updated row '{key}' in {table}")
			} else {
				format!("No row updated in {table} for key '{key}'")
			});
		}
		if entries.is_empty() {
			db.execute(&format!("INSERT INTO {} DEFAULT VALUES", quote(&table)), [])?;
		} else {
			let columns: Vec<String> = entries.iter().map(|(c, _)| quote(c)).collect();
			let marks = vec!["?"; entries.len()].join(", ");
			db.execute(
				&format!("INSERT INTO {} ({}) VALUES ({marks})", quote(&table), columns.join(", ")),
				rusqlite::params_from_iter(entries.into_iter().map(|(_, v)| v)),
			)?;
		}
		Ok(format!("Inserted row into {table}"))
	};
	run().map(Some).map_err(|error| anyhow!("{error}"))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn fixture() -> tempfile::TempDir {
		let root = tempfile::tempdir().unwrap();
		let db = Connection::open(root.path().join("app.db")).unwrap();
		db.execute_batch(
			"CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, bio TEXT);
			 INSERT INTO users (name, bio) VALUES ('ada', 'line\nbreak'), ('bob', NULL), ('cy', 'x');
			 CREATE TABLE logs (msg TEXT);",
		)
		.unwrap();
		root
	}

	fn run(root: &Path, path: &str) -> Result<String> {
		let fs = JailedFs::open(root)?;
		Ok(read_sqlite(&fs, path)?.expect("sqlite").render())
	}

	#[test]
	fn lists_tables_reads_rows_and_pages_queries() -> Result<()> {
		let root = fixture();
		assert_eq!(run(root.path(), "app.db")?, "logs (0 rows)\nusers (3 rows)");
		assert_eq!(run(root.path(), "app.db:users:2")?, "id: 2\nname: bob\nbio: NULL");
		let page = run(root.path(), "app.db:users?limit=1&order=name:desc")?;
		assert_eq!(
			page,
			"| id  | name | bio |\n| --- | ---- | --- |\n| 3   | cy   | x   |\n[2 more rows; append \
			 :users?limit=1&offset=1 to the database path to continue]"
		);
		let raw = run(root.path(), "app.db?q=SELECT%20bio%20FROM%20users%20WHERE%20id%3D1")?;
		assert!(raw.contains("| line\\nbreak |"), "{raw}");
		Ok(())
	}

	#[test]
	fn writes_insert_update_and_delete_rows_by_key() -> Result<()> {
		let root = fixture();
		let fs = JailedFs::open(root.path())?;
		let write = |path: &str, content: &str| write_sqlite(&fs, path, content).map(Option::unwrap);
		assert_eq!(write("app.db:users", r#"{"name":"dee"}"#)?, "Inserted row into users");
		assert_eq!(write("app.db:users:4", r#"{"bio":"new"}"#)?, "Updated row '4' in users");
		assert_eq!(run(root.path(), "app.db:users:4")?, "id: 4\nname: dee\nbio: new");
		assert_eq!(write("app.db:users:4", "")?, "Deleted row '4' from users");
		assert_eq!(write("app.db:users:4", "")?, "No row deleted from users for key '4'");
		for (path, content, message) in [
			("app.db:users", r#"{"nope":1}"#, "has no column named 'nope'"),
			("app.db:users", r#"{"name":[1]}"#, "only accepts JSON scalar values"),
			("app.db:users", "[1]", "must be a JSON object"),
			("app.db:users", "", "deletes require a row key"),
			("app.db:users?x=1", "{}", "do not support query parameters"),
			("other.db:users", "{}", "SQLite database 'other.db:users' not found"),
		] {
			let error = write_sqlite(&fs, path, content).unwrap_err().to_string();
			assert!(error.contains(message), "{path}: {error}");
		}
		Ok(())
	}

	#[test]
	fn selector_guards_reject_injection_and_bad_pagination() {
		let root = fixture();
		for (path, message) in [
			("app.db:users?where=1%3D1%3B%20DROP", "statement terminators"),
			("app.db:users?where=1%20UNION%20SELECT", "LIMIT/OFFSET/UNION"),
			("app.db:users?limit=0", "positive integer"),
			("app.db:users?bogus=1", "Unsupported SQLite query parameter 'bogus'"),
			("app.db:users:x", "must be an integer"),
			("app.db:nope", "SQLite table 'nope' not found"),
			("app.db:users?q=SELECT%201", "cannot be combined"),
		] {
			let error = run(root.path(), path).unwrap_err().to_string();
			assert!(error.contains(message), "{path}: {error}");
		}
		assert!(run(root.path(), "app.db:users?where=name%3D'a;b'").is_ok());
	}
}
