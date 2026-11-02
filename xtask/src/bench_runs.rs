//! Summarize edit-benchmark runs across (separator × model) by parsing the
//! Markdown reports produced by `bun run bench:edit`.
//!
//! Number parsing and formatting deliberately reproduce the JavaScript
//! semantics of the original tool (`parseInt`/`parseFloat` prefixes, `NaN`,
//! `toFixed`, `toLocaleString("en-US")`, `JSON.stringify`) so reports stay
//! byte-identical.
use std::{
	cmp::Ordering,
	fmt::Write as _,
	fs,
	path::{Path, PathBuf},
	sync::LazyLock,
};

use anyhow::{Result, bail};
use regex::Regex;

const SEPARATOR_ORDER: [&str; 8] = ["gt", "plus", "div", "pipe", "bslash", "tilde", "pct", "colon"];
const REPORT_LIMIT: u64 = 16 * 1024 * 1024;
const USAGE: &str = "usage: cargo xtask bench-runs <runs-dir> [<runs-dir>...] [--aggregate] \
                     [--format table|md|csv|json] [--sort sep|model|task|edit|tokens]";

fn separator_display(slug: &str) -> &str {
	match slug {
		"gt" => ">",
		"plus" => "+",
		"div" => "÷",
		"pipe" => "|",
		"bslash" => "\\",
		"tilde" => "~",
		"pct" => "%",
		"colon" => ":",
		other => other,
	}
}

// ── JavaScript number semantics ────────────────────────────────────────────

static FLOAT_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
	Regex::new(r"^[+-]?(?:Infinity|(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?)").expect("valid regex")
});

/// `Number.parseFloat`.
pub fn parse_float(text: &str) -> f64 {
	let text = text.trim_start();
	let Some(found) = FLOAT_PREFIX.find(text) else {
		return f64::NAN;
	};
	let literal = found.as_str();
	match literal.trim_start_matches(['+', '-']) {
		"Infinity" if literal.starts_with('-') => f64::NEG_INFINITY,
		"Infinity" => f64::INFINITY,
		_ => literal.parse().unwrap_or(f64::NAN),
	}
}

/// `Number.parseInt(text, 10)`.
pub fn parse_int(text: &str) -> f64 {
	let text = text.trim_start();
	let (sign, digits) = match text.as_bytes().first() {
		Some(b'-') => (-1.0, &text[1..]),
		Some(b'+') => (1.0, &text[1..]),
		_ => (1.0, text),
	};
	let end = digits.bytes().take_while(u8::is_ascii_digit).count();
	if end == 0 {
		return f64::NAN;
	}
	sign * digits[..end].parse::<f64>().unwrap_or(f64::NAN)
}

/// Round `value` to `digits` decimals with ties resolved upward (JS `toFixed`)
/// or away from zero (ICU half-expand), returning the decimal string.
fn round_decimal(value: f64, digits: usize) -> String {
	// Rust rounds the exact binary value correctly but resolves exact ties to
	// even; JS/ICU resolve them upward. An f64 has at most 1074 fractional
	// digits, so this expansion is exact and reveals true ties only.
	let exact = format!("{value:.1100}");
	let point = exact.find('.').expect("fixed notation has a point");
	let tail = &exact[point + 1 + digits..];
	let is_tie = tail.starts_with('5') && tail[1..].bytes().all(|byte| byte == b'0');
	if !is_tie {
		return format!("{value:.digits$}");
	}
	// Both `toFixed` (sign applied to the rounded magnitude) and ICU round
	// exact ties away from zero.
	let negative = exact.starts_with('-');
	let truncated = &exact[usize::from(negative)..point + usize::from(digits > 0) + digits];
	let magnitude = increment_decimal(truncated);
	if negative {
		format!("-{magnitude}")
	} else {
		magnitude
	}
}

/// Round the shortest round-trip decimal form of `value` (what ICU formats)
/// half away from zero.
fn round_shortest(value: f64, digits: usize) -> String {
	let shortest = format!("{value}");
	let (sign, magnitude) = shortest
		.strip_prefix('-')
		.map_or(("", shortest.as_str()), |rest| ("-", rest));
	let (integer, fraction) = magnitude.split_once('.').unwrap_or((magnitude, ""));
	if fraction.len() <= digits {
		return format!("{sign}{integer}.{fraction:0<digits$}");
	}
	let kept = format!("{integer}.{}", &fraction[..digits]);
	let rounded = if fraction.as_bytes()[digits] >= b'5' {
		increment_decimal(&kept)
	} else {
		kept
	};
	format!("{sign}{rounded}")
}

/// Add one unit in the last place to a non-negative decimal string.
fn increment_decimal(text: &str) -> String {
	let mut bytes = text.as_bytes().to_vec();
	for index in (0..bytes.len()).rev() {
		match bytes[index] {
			b'.' => {},
			b'9' => bytes[index] = b'0',
			digit => {
				bytes[index] = digit + 1;
				return String::from_utf8(bytes).expect("ascii");
			},
		}
	}
	format!("1{}", String::from_utf8(bytes).expect("ascii"))
}

/// `Number.prototype.toFixed`.
pub fn to_fixed(value: f64, digits: usize) -> String {
	round_decimal(value, digits)
}

/// `Math.round`.
fn js_round(value: f64) -> f64 {
	if value.is_finite() {
		(value + 0.5).floor()
	} else {
		value
	}
}

/// `Number.prototype.toLocaleString()` in en-US: grouping, at most three
/// fraction digits.
pub fn to_locale(value: f64) -> String {
	let rounded = round_shortest(value, 3);
	let (sign, digits) = rounded
		.strip_prefix('-')
		.map_or(("", rounded.as_str()), |rest| ("-", rest));
	let (integer, fraction) = digits.split_once('.').unwrap_or((digits, ""));
	let fraction = fraction.trim_end_matches('0');
	let mut grouped = String::new();
	for (index, ch) in integer.chars().enumerate() {
		if index > 0 && (integer.len() - index) % 3 == 0 {
			grouped.push(',');
		}
		grouped.push(ch);
	}
	let sign = if grouped.chars().all(|ch| ch == '0' || ch == ',') && fraction.is_empty() {
		// ICU renders negative zero as "-0".
		if value.is_sign_negative() { "-" } else { "" }
	} else {
		sign
	};
	if fraction.is_empty() {
		format!("{sign}{grouped}")
	} else {
		format!("{sign}{grouped}.{fraction}")
	}
}

/// `Number.prototype.toString()` for finite values.
pub fn js_number(value: f64) -> String {
	if value == 0.0 {
		return "0".to_owned();
	}
	let abs = value.abs();
	if (1e-6..1e21).contains(&abs) {
		return format!("{value}");
	}
	let exponent = format!("{value:e}");
	let (mantissa, power) = exponent.split_once('e').expect("exponent form");
	if power.starts_with('-') {
		format!("{mantissa}e{power}")
	} else {
		format!("{mantissa}e+{power}")
	}
}

fn json_number(value: f64) -> String {
	if value.is_finite() {
		js_number(value)
	} else {
		"null".to_owned()
	}
}

fn json_string(value: &str) -> String {
	serde_json::to_string(value).expect("strings serialize")
}

fn fmt_pct(value: f64) -> String {
	if value.is_finite() {
		format!("{}%", to_fixed(value, 1))
	} else {
		"—".to_owned()
	}
}

fn fmt_num(value: f64) -> String {
	if value.is_finite() {
		to_locale(value)
	} else {
		"—".to_owned()
	}
}

fn utf16_len(text: &str) -> usize {
	text.encode_utf16().count()
}

fn pad_end(text: &str, width: usize) -> String {
	let mut out = text.to_owned();
	out.extend(std::iter::repeat_n(' ', width.saturating_sub(utf16_len(text))));
	out
}

fn js_compare(value: f64) -> Ordering {
	if value < 0.0 {
		Ordering::Less
	} else if value > 0.0 {
		Ordering::Greater
	} else {
		Ordering::Equal
	}
}

// ── Report parsing ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Row {
	file:                 String,
	sep_slug:             String,
	model:                String,
	total_tasks:          f64,
	total_runs:           f64,
	successful_runs:      f64,
	task_success_pct:     f64,
	verified_pct:         f64,
	edit_tool_usage_pct:  f64,
	edit_success_pct:     f64,
	patch_failure_pct:    f64,
	patch_failures:       f64,
	patch_attempts:       f64,
	mutation_intent_pct:  f64,
	autocorrect_free_pct: f64,
	tasks_all_passing:    f64,
	tasks_flaky_failing:  f64,
	timeout_runs:         f64,
	input_tokens_total:   f64,
	output_tokens_total:  f64,
	total_tokens:         f64,
	input_tokens_avg:     f64,
	output_tokens_avg:    f64,
	total_tokens_avg:     f64,
	duration_total:       String,
	duration_avg:         String,
	avg_indent_score:     Option<f64>,
	read_total:           f64,
	edit_total:           f64,
	write_total:          f64,
}

enum Field<'a> {
	Num(f64),
	Text(&'a str),
	Nullable(Option<f64>),
}

impl Row {
	fn fields(&self) -> [(&'static str, Field<'_>); 30] {
		use Field::{Nullable, Num, Text};
		[
			("file", Text(&self.file)),
			("sepSlug", Text(&self.sep_slug)),
			("model", Text(&self.model)),
			("totalTasks", Num(self.total_tasks)),
			("totalRuns", Num(self.total_runs)),
			("successfulRuns", Num(self.successful_runs)),
			("taskSuccessPct", Num(self.task_success_pct)),
			("verifiedPct", Num(self.verified_pct)),
			("editToolUsagePct", Num(self.edit_tool_usage_pct)),
			("editSuccessPct", Num(self.edit_success_pct)),
			("patchFailurePct", Num(self.patch_failure_pct)),
			("patchFailures", Num(self.patch_failures)),
			("patchAttempts", Num(self.patch_attempts)),
			("mutationIntentPct", Num(self.mutation_intent_pct)),
			("autocorrectFreePct", Num(self.autocorrect_free_pct)),
			("tasksAllPassing", Num(self.tasks_all_passing)),
			("tasksFlakyFailing", Num(self.tasks_flaky_failing)),
			("timeoutRuns", Num(self.timeout_runs)),
			("inputTokensTotal", Num(self.input_tokens_total)),
			("outputTokensTotal", Num(self.output_tokens_total)),
			("totalTokens", Num(self.total_tokens)),
			("inputTokensAvg", Num(self.input_tokens_avg)),
			("outputTokensAvg", Num(self.output_tokens_avg)),
			("totalTokensAvg", Num(self.total_tokens_avg)),
			("durationTotal", Text(&self.duration_total)),
			("durationAvg", Text(&self.duration_avg)),
			("avgIndentScore", Nullable(self.avg_indent_score)),
			("readTotal", Num(self.read_total)),
			("editTotal", Num(self.edit_total)),
			("writeTotal", Num(self.write_total)),
		]
	}

	fn json_field(&self, name: &str) -> String {
		self
			.fields()
			.into_iter()
			.find(|(key, _)| *key == name)
			.map_or_else(
				|| "\"\"".to_owned(),
				|(_, value)| match value {
					Field::Num(value) => json_number(value),
					Field::Text(text) => json_string(text),
					// `null ?? ""` serializes as an empty string in the CSV.
					Field::Nullable(None) => "\"\"".to_owned(),
					Field::Nullable(Some(value)) => json_number(value),
				},
			)
	}
}

fn label_pattern(label: &str) -> String {
	format!(r"^\|\s*\*?\*?{}\*?\*?\s*\|", regex::escape(label))
}

fn get_cell(text: &str, label: &str) -> Option<String> {
	let pattern = format!(r"(?m){}\s*\*?\*?(.+?)\*?\*?\s*\|\s*$", label_pattern(label));
	Regex::new(&pattern)
		.ok()?
		.captures(text)
		.map(|captures| captures[1].trim().to_owned())
}

fn get_row(text: &str, label: &str) -> Option<Vec<String>> {
	let pattern = format!(r"(?m){}(.+)\|\s*$", label_pattern(label));
	let captures = Regex::new(&pattern).ok()?.captures(text)?;
	Some(
		captures[1]
			.split('|')
			.map(|cell| {
				let cell = cell.trim();
				let cell = cell.strip_prefix("**").unwrap_or(cell);
				cell.strip_suffix("**").unwrap_or(cell).to_owned()
			})
			.collect(),
	)
}

fn parse_number(text: &str) -> f64 {
	parse_float(&text.replace(',', ""))
}

fn parse_percent(value: Option<&str>) -> f64 {
	static PERCENT: LazyLock<Regex> =
		LazyLock::new(|| Regex::new(r"([0-9]+(?:\.[0-9]+)?)\s*%").expect("valid regex"));
	value
		.and_then(|value| PERCENT.captures(value))
		.map_or(f64::NAN, |captures| parse_float(&captures[1]))
}

fn parse_rate_pair(value: Option<&str>) -> (f64, f64, f64) {
	static PAIR: LazyLock<Regex> = LazyLock::new(|| {
		Regex::new(r"([0-9.]+)\s*%\s*\(\s*([0-9,]+)\s*/\s*([0-9,]+)\s*\)").expect("valid regex")
	});
	let Some(value) = value else {
		return (0.0, 0.0, f64::NAN);
	};
	PAIR.captures(value).map_or_else(
		|| (0.0, 0.0, parse_percent(Some(value))),
		|captures| {
			(parse_number(&captures[2]), parse_number(&captures[3]), parse_float(&captures[1]))
		},
	)
}

fn cell_int(text: &str, label: &str) -> f64 {
	parse_int(get_cell(text, label).as_deref().unwrap_or("0"))
}

fn first_two(row: Option<Vec<String>>, default: &str) -> (String, Option<String>) {
	let row = row.unwrap_or_else(|| vec![default.to_owned(), default.to_owned()]);
	(row.first().cloned().unwrap_or_default(), row.get(1).cloned())
}

fn parse_report(file: &Path, text: &str) -> Row {
	let name = file
		.file_name()
		.map(|name| name.to_string_lossy().into_owned())
		.unwrap_or_default();
	let base = name.strip_suffix(".md").unwrap_or(&name);
	let mut parts = base.split("__");
	let sep_slug = parts.next().unwrap_or_default().to_owned();
	let model = parts.collect::<Vec<_>>().join("__").replace('_', "/");

	let cell = |label: &str| get_cell(text, label);
	let (edit_tool, patch) = (
		parse_rate_pair(cell("Edit Tool Usage Rate").as_deref()),
		parse_rate_pair(cell("Patch Failure Rate").as_deref()),
	);
	let (input_total, input_avg) = first_two(get_row(text, "Input Tokens"), "0");
	let (output_total, output_avg) = first_two(get_row(text, "Output Tokens"), "0");
	let (tokens_total, tokens_avg) = first_two(get_row(text, "Total Tokens"), "0");
	let (duration_total, duration_avg) = first_two(get_row(text, "Duration"), "");
	let (_, indent) = first_two(get_row(text, "Avg Indent Score"), "—");
	let num = |value: Option<String>| value.map_or(f64::NAN, |value| parse_number(&value));
	let indent = indent.map(|value| {
		value
			.chars()
			.filter(|ch| !(*ch == '*' || ch.is_whitespace() || *ch == '—' || *ch == '-'))
			.collect::<String>()
	});

	Row {
		file: file.to_string_lossy().into_owned(),
		sep_slug,
		model,
		total_tasks: cell_int(text, "Total Tasks"),
		total_runs: cell_int(text, "Total Runs"),
		successful_runs: cell_int(text, "Successful Runs"),
		task_success_pct: parse_percent(cell("Task Success Rate").as_deref()),
		verified_pct: parse_percent(cell("Verified Rate").as_deref()),
		edit_tool_usage_pct: edit_tool.2,
		edit_success_pct: parse_percent(cell("Edit Success Rate").as_deref()),
		patch_failure_pct: patch.2,
		patch_failures: patch.0,
		patch_attempts: patch.1,
		mutation_intent_pct: parse_percent(cell("Mutation Intent Match Rate").as_deref()),
		autocorrect_free_pct: parse_percent(cell("Autocorrect-Free Success Rate").as_deref()),
		tasks_all_passing: cell_int(text, "Tasks All Passing"),
		tasks_flaky_failing: cell_int(text, "Tasks Flaky/Failing"),
		timeout_runs: cell_int(text, "Timeout Runs"),
		input_tokens_total: parse_number(&input_total),
		output_tokens_total: parse_number(&output_total),
		total_tokens: parse_number(&tokens_total),
		input_tokens_avg: num(input_avg),
		output_tokens_avg: num(output_avg),
		total_tokens_avg: num(tokens_avg),
		duration_total,
		duration_avg: duration_avg.unwrap_or_default(),
		avg_indent_score: indent
			.filter(|value| !value.is_empty())
			.map(|value| parse_float(&value)),
		read_total: parse_number(&first_two(get_row(text, "Read"), "0").0),
		edit_total: parse_number(&first_two(get_row(text, "Edit"), "0").0),
		write_total: parse_number(&first_two(get_row(text, "Write"), "0").0),
	}
}

// ── Aggregation and sorting ────────────────────────────────────────────────

fn finite_sum(rows: &[Row], pick: impl Fn(&Row) -> f64) -> f64 {
	rows
		.iter()
		.map(&pick)
		.filter(|value| value.is_finite())
		.sum()
}

fn finite_avg(rows: &[Row], pick: impl Fn(&Row) -> f64) -> f64 {
	let values: Vec<f64> = rows
		.iter()
		.map(pick)
		.filter(|value| value.is_finite())
		.collect();
	if values.is_empty() {
		f64::NAN
	} else {
		values.iter().sum::<f64>() / values.len() as f64
	}
}

/// Group by key preserving first-seen order, like a JavaScript `Map`.
fn group_by(rows: &[Row], key: impl Fn(&Row) -> String) -> Vec<(String, Vec<Row>)> {
	let mut groups: Vec<(String, Vec<Row>)> = Vec::new();
	for row in rows {
		let key = key(row);
		match groups.iter_mut().find(|(existing, _)| *existing == key) {
			Some((_, list)) => list.push(row.clone()),
			None => groups.push((key, vec![row.clone()])),
		}
	}
	groups
}

fn merge_rows(rows: &[Row]) -> Vec<Row> {
	let ratio = |numerator: f64, denominator: f64| {
		if denominator == 0.0 {
			f64::NAN
		} else {
			numerator / denominator * 100.0
		}
	};
	group_by(rows, |row| format!("{}::{}", row.sep_slug, row.model))
		.into_iter()
		.map(|(_, list)| {
			if list.len() == 1 {
				return list[0].clone();
			}
			let total_runs = finite_sum(&list, |row| row.total_runs);
			let successful_runs = finite_sum(&list, |row| row.successful_runs);
			let patch_failures = finite_sum(&list, |row| row.patch_failures);
			let patch_attempts = finite_sum(&list, |row| row.patch_attempts);
			let total_tokens = finite_sum(&list, |row| row.total_tokens);
			let input_tokens_total = finite_sum(&list, |row| row.input_tokens_total);
			let output_tokens_total = finite_sum(&list, |row| row.output_tokens_total);
			let mean_of_finite = |pick: fn(&Row) -> f64| {
				list
					.iter()
					.map(&pick)
					.map(|value| if value.is_finite() { value } else { 0.0 })
					.sum::<f64>()
					/ list.len() as f64
			};
			let per_run = |total: f64| {
				if total_runs == 0.0 {
					0.0
				} else {
					js_round(total / total_runs)
				}
			};
			let indents: Vec<f64> = list.iter().filter_map(|row| row.avg_indent_score).collect();
			Row {
				file: list
					.iter()
					.map(|row| row.file.as_str())
					.collect::<Vec<_>>()
					.join(","),
				sep_slug: list[0].sep_slug.clone(),
				model: list[0].model.clone(),
				total_tasks: finite_sum(&list, |row| row.total_tasks),
				total_runs,
				successful_runs,
				task_success_pct: ratio(successful_runs, total_runs),
				verified_pct: ratio(successful_runs, total_runs),
				edit_tool_usage_pct: ratio(
					finite_sum(&list, |row| js_round(row.edit_tool_usage_pct / 100.0 * row.total_runs)),
					total_runs,
				),
				edit_success_pct: ratio(patch_attempts - patch_failures, patch_attempts),
				patch_failure_pct: ratio(patch_failures, patch_attempts),
				patch_failures,
				patch_attempts,
				mutation_intent_pct: mean_of_finite(|row| row.mutation_intent_pct),
				autocorrect_free_pct: mean_of_finite(|row| row.autocorrect_free_pct),
				tasks_all_passing: finite_sum(&list, |row| row.tasks_all_passing),
				tasks_flaky_failing: finite_sum(&list, |row| row.tasks_flaky_failing),
				timeout_runs: finite_sum(&list, |row| row.timeout_runs),
				input_tokens_total,
				output_tokens_total,
				total_tokens,
				input_tokens_avg: per_run(input_tokens_total),
				output_tokens_avg: per_run(output_tokens_total),
				total_tokens_avg: per_run(total_tokens),
				duration_total: list
					.iter()
					.map(|row| row.duration_total.as_str())
					.collect::<Vec<_>>()
					.join(" + "),
				duration_avg: list
					.iter()
					.map(|row| row.duration_avg.as_str())
					.collect::<Vec<_>>()
					.join(" / "),
				avg_indent_score: (!indents.is_empty())
					.then(|| indents.iter().sum::<f64>() / indents.len() as f64),
				read_total: finite_sum(&list, |row| row.read_total),
				edit_total: finite_sum(&list, |row| row.edit_total),
				write_total: finite_sum(&list, |row| row.write_total),
			}
		})
		.collect()
}

fn sep_index(slug: &str) -> f64 {
	SEPARATOR_ORDER
		.iter()
		.position(|known| *known == slug)
		.map_or(-1.0, |index| index as f64)
}

fn model_order(model: &str) -> f64 {
	if model.contains("glm") {
		0.0
	} else if model.contains("gpt") {
		1.0
	} else if model.contains("claude") {
		2.0
	} else {
		3.0
	}
}

fn sort_rows(rows: &mut [Row], by: &str) {
	let key: Option<fn(&Row, &Row) -> f64> = match by {
		"sep" => Some(|a, b| {
			let first = sep_index(&a.sep_slug) - sep_index(&b.sep_slug);
			if first == 0.0 {
				model_order(&a.model) - model_order(&b.model)
			} else {
				first
			}
		}),
		"model" => Some(|a, b| {
			let first = model_order(&a.model) - model_order(&b.model);
			if first == 0.0 {
				sep_index(&a.sep_slug) - sep_index(&b.sep_slug)
			} else {
				first
			}
		}),
		"task" => Some(|a, b| b.task_success_pct - a.task_success_pct),
		"edit" => Some(|a, b| b.edit_success_pct - a.edit_success_pct),
		"tokens" => Some(|a, b| a.total_tokens_avg - b.total_tokens_avg),
		// An unknown key leaves the order untouched, as the legacy tool did.
		_ => None,
	};
	if let Some(key) = key {
		js_sort(rows, key);
	}
}

/// Stable sort by a JavaScript-style difference comparator. Pairs whose
/// difference is `NaN` (a missing metric) are not ordered by the legacy
/// engines consistently, so rows with a `NaN` difference against a finite row
/// deterministically sort after it; all finite comparisons match the legacy
/// order exactly.
fn js_sort(rows: &mut [Row], compare: fn(&Row, &Row) -> f64) {
	rows.sort_by(|a, b| {
		let difference = compare(a, b);
		if !difference.is_nan() {
			return js_compare(difference);
		}
		let (a_missing, b_missing) = (compare(a, a).is_nan(), compare(b, b).is_nan());
		a_missing.cmp(&b_missing)
	});
}

// ── Rendering ──────────────────────────────────────────────────────────────

fn table(headers: &[&str], data: &[Vec<String>], out: &mut String) {
	let widths: Vec<usize> = headers
		.iter()
		.enumerate()
		.map(|(index, header)| {
			data
				.iter()
				.map(|row| utf16_len(&row[index]))
				.fold(utf16_len(header), usize::max)
		})
		.collect();
	let line = |cells: Vec<String>| {
		cells
			.iter()
			.zip(&widths)
			.map(|(cell, width)| pad_end(cell, *width))
			.collect::<Vec<_>>()
			.join("  ")
	};
	out.push_str(&line(headers.iter().map(|header| (*header).to_owned()).collect()));
	out.push('\n');
	out.push_str(
		&widths
			.iter()
			.map(|width| "-".repeat(*width))
			.collect::<Vec<_>>()
			.join("  "),
	);
	out.push('\n');
	for row in data {
		out.push_str(&line(row.clone()));
		out.push('\n');
	}
}

fn markdown(headers: &[&str], data: &[Vec<String>], out: &mut String) {
	let _ = writeln!(out, "| {} |", headers.join(" | "));
	let _ = writeln!(out, "|{}|", vec!["---"; headers.len()].join("|"));
	for row in data {
		let _ = writeln!(out, "| {} |", row.join(" | "));
	}
}

fn short_model(model: &str) -> String {
	model
		.rsplit('/')
		.next()
		.unwrap_or(model)
		.replacen(":nitro", "", 1)
}

fn aggregates(rows: &[Row], md: bool, out: &mut String) {
	let summarize = |label: String, list: &[Row]| {
		vec![
			label,
			fmt_pct(finite_avg(list, |row| row.task_success_pct)),
			fmt_pct(finite_avg(list, |row| row.edit_success_pct)),
			format!(
				"{}/{}",
				js_number(finite_sum(list, |row| row.patch_failures)),
				js_number(finite_sum(list, |row| row.patch_attempts))
			),
			fmt_num(js_round(finite_avg(list, |row| row.total_tokens_avg))),
		]
	};
	let by_sep: Vec<Vec<String>> = group_by(rows, |row| row.sep_slug.clone())
		.into_iter()
		.map(|(slug, list)| summarize(separator_display(&slug).to_owned(), &list))
		.collect();
	let by_model: Vec<Vec<String>> = group_by(rows, |row| row.model.clone())
		.into_iter()
		.map(|(model, list)| summarize(short_model(&model), &list))
		.collect();
	let suffix = ["task ✓ (avg)", "edit ✓ (avg)", "patch fail (sum)", "tok/run (avg)"];
	let sep_headers = [&["sep"][..], &suffix].concat();
	let model_headers = [&["model"][..], &suffix].concat();
	if md {
		out.push_str("### Per separator (avg across models)\n\n");
		markdown(&sep_headers, &by_sep, out);
		out.push_str("\n### Per model (avg across separators)\n\n");
		markdown(&model_headers, &by_model, out);
	} else {
		out.push_str("\nPer separator (avg across models):\n");
		table(&sep_headers, &by_sep, out);
		out.push_str("\nPer model (avg across separators):\n");
		table(&model_headers, &by_model, out);
	}
}

fn render(rows: &[Row], format: &str) -> String {
	let mut out = String::new();
	if format == "json" {
		if rows.is_empty() {
			return "[]\n".to_owned();
		}
		out.push_str("[\n");
		for (index, row) in rows.iter().enumerate() {
			out.push_str("  {\n");
			let fields = row.fields();
			for (position, (key, value)) in fields.iter().enumerate() {
				let value = match value {
					Field::Num(value) => json_number(*value),
					Field::Text(text) => json_string(text),
					Field::Nullable(value) => value.map_or_else(|| "null".to_owned(), json_number),
				};
				let comma = if position + 1 < fields.len() { "," } else { "" };
				let _ = writeln!(out, "    \"{key}\": {value}{comma}");
			}
			out.push_str(if index + 1 < rows.len() {
				"  },\n"
			} else {
				"  }\n"
			});
		}
		out.push_str("]\n");
		return out;
	}
	if format == "csv" {
		let columns = [
			"sepSlug",
			"model",
			"totalRuns",
			"successfulRuns",
			"taskSuccessPct",
			"editToolUsagePct",
			"editSuccessPct",
			"patchFailurePct",
			"patchFailures",
			"patchAttempts",
			"mutationIntentPct",
			"avgIndentScore",
			"inputTokensTotal",
			"outputTokensTotal",
			"totalTokens",
			"totalTokensAvg",
			"durationTotal",
			"durationAvg",
			"editTotal",
			"readTotal",
		];
		let _ = writeln!(out, "{}", columns.join(","));
		for row in rows {
			let _ = writeln!(
				out,
				"{}",
				columns
					.iter()
					.map(|column| row.json_field(column))
					.collect::<Vec<_>>()
					.join(",")
			);
		}
		return out;
	}

	let headers = [
		"sep",
		"model",
		"task ✓",
		"edit ✓",
		"patch fail",
		"intent",
		"in tok/run",
		"out tok/run",
		"tok/run",
		"avg time",
		"indent",
	];
	let data: Vec<Vec<String>> = rows
		.iter()
		.map(|row| {
			vec![
				separator_display(&row.sep_slug).to_owned(),
				short_model(&row.model),
				format!(
					"{} ({}/{})",
					fmt_pct(row.task_success_pct),
					js_number_or_nan(row.successful_runs),
					js_number_or_nan(row.total_runs)
				),
				fmt_pct(row.edit_success_pct),
				format!(
					"{} ({}/{})",
					fmt_pct(row.patch_failure_pct),
					js_number_or_nan(row.patch_failures),
					js_number_or_nan(row.patch_attempts)
				),
				fmt_pct(row.mutation_intent_pct),
				fmt_num(row.input_tokens_avg),
				fmt_num(row.output_tokens_avg),
				fmt_num(row.total_tokens_avg),
				row.duration_avg.clone(),
				row.avg_indent_score.map_or_else(
					|| "—".to_owned(),
					|value| {
						if value.is_finite() {
							to_fixed(value, 2)
						} else {
							"NaN".to_owned()
						}
					},
				),
			]
		})
		.collect();
	if format == "md" {
		markdown(&headers, &data, &mut out);
		out.push('\n');
		aggregates(rows, true, &mut out);
	} else {
		table(&headers, &data, &mut out);
		aggregates(rows, false, &mut out);
	}
	out
}

/// Template-literal interpolation of a number (`${n}`).
fn js_number_or_nan(value: f64) -> String {
	if value.is_nan() {
		"NaN".to_owned()
	} else if value.is_infinite() {
		if value > 0.0 {
			"Infinity".to_owned()
		} else {
			"-Infinity".to_owned()
		}
	} else {
		js_number(value)
	}
}

pub fn execute(arguments: Vec<String>) -> Result<()> {
	let mut dirs = Vec::new();
	let (mut format, mut sort, mut aggregate) = ("table".to_owned(), "sep".to_owned(), false);
	let mut arguments = arguments.into_iter();
	while let Some(argument) = arguments.next() {
		match argument.as_str() {
			"--format" => format = arguments.next().unwrap_or_default(),
			"--sort" => sort = arguments.next().unwrap_or_default(),
			"--aggregate" => aggregate = true,
			"-h" | "--help" => {
				println!("{USAGE}");
				return Ok(());
			},
			other if !other.starts_with("--") => dirs.push(PathBuf::from(other)),
			_ => {},
		}
	}
	if dirs.is_empty() {
		bail!("{USAGE}");
	}
	print!("{}", summarize(&dirs, &format, &sort, aggregate)?);
	Ok(())
}

fn summarize(dirs: &[PathBuf], format: &str, sort: &str, aggregate: bool) -> Result<String> {
	let mut rows = Vec::new();
	for dir in dirs {
		let dir = std::path::absolute(dir)?;
		let files: Vec<PathBuf> = fs::read_dir(&dir)?
			.filter_map(Result::ok)
			.filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
			.map(|entry| entry.path())
			.filter(|path| path.to_string_lossy().ends_with(".md"))
			.collect();
		// Keep raw readdir order: the legacy tool (and its stable sort ties) used it.
		for file in files {
			if fs::metadata(&file)?.len() > REPORT_LIMIT {
				bail!("{} exceeds the report size limit", file.display());
			}
			rows.push(parse_report(&file, &fs::read_to_string(&file)?));
		}
	}
	let mut rows = if aggregate { merge_rows(&rows) } else { rows };
	sort_rows(&mut rows, sort);
	Ok(render(&rows, format))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn javascript_number_semantics() {
		assert_eq!(parse_int("12 tasks"), 12.0);
		assert!(parse_int("—").is_nan());
		assert_eq!(parse_float(" 1.5e2x"), 150.0);
		assert!(parse_float("abc").is_nan());
		assert_eq!(to_fixed(0.25, 1), "0.3");
		assert_eq!(to_fixed(12.345, 1), "12.3");
		assert_eq!(to_fixed(8.95, 1), "8.9");
		assert_eq!(to_fixed(-0.25, 1), "-0.3");
		assert_eq!(to_fixed(9.95, 0), "10");
		assert_eq!(to_locale(1_234_567.0), "1,234,567");
		assert_eq!(to_locale(1.0625), "1.063");
		assert_eq!(to_locale(-1234.5), "-1,234.5");
		assert_eq!(js_number(12.0), "12");
		assert_eq!(js_number(1e21), "1e+21");
		assert_eq!(js_number(1.5e-7), "1.5e-7");
	}

	const REPORT: &str = "| Metric | Value |\n|---|---|\n| Total Tasks | 10 |\n| Total Runs | 20 \
	                      |\n| Successful Runs | 15 |\n| **Task Success Rate** | **80.0% (8/10)** \
	                      |\n| Edit Tool Usage Rate | 90.0% (18/20) |\n| **Edit Success Rate** | \
	                      **75.5%** |\n| Patch Failure Rate | 12.5% (3/24) |\n| Read | 40 | 4.0 \
	                      |\n| Input Tokens | 1,000 | 50 | 1 | 1 | 1 |\n| Total Tokens | 3,000 | \
	                      150.5 | 1 | 1 | 1 |\n| Duration | 1m 2s | 3s | — | — | — |\n| **Avg \
	                      Indent Score** | — | **0.95** | — | — | — |\n";

	#[test]
	fn report_fields_are_extracted_from_bold_and_rate_cells() {
		let row = parse_report(Path::new("/r/pipe__openai_gpt-5.md"), REPORT);
		assert_eq!((row.sep_slug.as_str(), row.model.as_str()), ("pipe", "openai/gpt-5"));
		assert_eq!((row.total_runs, row.task_success_pct, row.edit_success_pct), (20.0, 80.0, 75.5));
		assert_eq!(
			(row.patch_failures, row.patch_attempts, row.patch_failure_pct),
			(3.0, 24.0, 12.5)
		);
		assert_eq!((row.total_tokens, row.total_tokens_avg), (3000.0, 150.5));
		assert_eq!((row.duration_total.as_str(), row.duration_avg.as_str()), ("1m 2s", "3s"));
		assert_eq!(row.avg_indent_score, Some(0.95));
		assert!(row.verified_pct.is_nan());
	}

	#[test]
	fn aggregation_pools_counts_per_separator_and_model() {
		let row = parse_report(Path::new("/r/pipe__openai_gpt-5.md"), REPORT);
		let merged = merge_rows(&[row.clone(), row]);
		assert_eq!(merged.len(), 1);
		assert_eq!((merged[0].total_runs, merged[0].patch_attempts), (40.0, 48.0));
		assert_eq!(merged[0].total_tokens_avg, 150.0);
		assert_eq!(merged[0].duration_avg, "3s / 3s");
	}
}
