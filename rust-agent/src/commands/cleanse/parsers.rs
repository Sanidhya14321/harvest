//! Checker output parsers. Ported from `cleanse/parsers.ts`.

use std::{
	collections::HashSet,
	path::{Component, Path, PathBuf},
	sync::LazyLock,
};

use regex::Regex;
use serde_json::{Map, Value};

use super::{Diagnostic, Severity, diagnostic_key};

pub const PARSER_KINDS: &[&str] = &[
	"rust", "rust-test", "go", "go-test", "staticcheck", "golangci", "ruff", "pyright", "mypy",
	"pylint", "flake8", "ty", "eslint", "biome", "oxlint", "deno-lint", "stylelint", "rubocop",
	"phpstan", "psalm", "swiftlint", "dart", "credo", "shellcheck", "hlint", "terraform", "tflint",
	"actionlint", "generic",
];

pub struct Input<'a> {
	pub checker:     &'a str,
	pub project_cwd: &'a Path,
	pub checker_cwd: &'a Path,
	pub stdout:      &'a str,
	pub stderr:      &'a str,
}

#[derive(Default)]
struct Fields {
	file:       Option<String>,
	line:       Option<f64>,
	column:     Option<f64>,
	end_line:   Option<f64>,
	end_column: Option<f64>,
	code:       Option<String>,
	severity:   Option<Value>,
	message:    Option<String>,
	suggestion: Option<String>,
}

static ANSI: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)").expect("static"));

/// Strip terminal escapes and non-printing control characters.
pub fn sanitize_text(text: &str) -> String {
	ANSI.replace_all(text, "")
		.chars()
		.filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
		.collect()
}

pub fn parse(kind: &str, input: &Input) -> Vec<Diagnostic> {
	let parsed = match kind {
		"rust" => parse_rust(input),
		"rust-test" => parse_rust_test(input),
		"go" => parse_go(input),
		"go-test" => parse_go_test(input),
		"staticcheck" => parse_staticcheck(input),
		"golangci" => parse_golangci(input),
		"ruff" => parse_ruff(input),
		"pyright" => parse_pyright(input),
		"pylint" => parse_pylint(input),
		"flake8" => parse_flake8(input),
		"ty" => parse_ty(input),
		"eslint" => parse_eslint(input),
		"biome" => parse_biome(input),
		"oxlint" => parse_unix(input),
		"deno-lint" => parse_deno(input),
		"stylelint" => parse_stylelint(input),
		"rubocop" => parse_rubocop(input),
		"phpstan" => parse_phpstan(input),
		"psalm" => parse_psalm(input),
		"swiftlint" => parse_swiftlint(input),
		"dart" => parse_dart(input),
		"credo" => parse_credo(input),
		"shellcheck" => parse_shellcheck(input),
		"hlint" => parse_hlint(input),
		"terraform" => parse_terraform(input),
		"tflint" => parse_tflint(input),
		"actionlint" => parse_actionlint(input),
		_ => parse_generic(input),
	};
	let mut seen = HashSet::new();
	parsed.into_iter().filter(|d| seen.insert(diagnostic_key(d))).collect()
}

fn rec(value: &Value) -> Option<&Map<String, Value>> {
	value.as_object()
}
fn arr(value: Option<&Value>) -> &[Value] {
	value.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}
fn sf(record: Option<&Map<String, Value>>, key: &str) -> Option<String> {
	record?.get(key)?.as_str().filter(|s| !s.trim().is_empty()).map(str::to_owned)
}
fn nf(record: Option<&Map<String, Value>>, key: &str) -> Option<f64> {
	record?.get(key)?.as_f64().filter(|n| n.is_finite())
}
fn nested<'a>(record: Option<&'a Map<String, Value>>, key: &str) -> Option<&'a Map<String, Value>> {
	record?.get(key)?.as_object()
}
fn zero_based(value: Option<f64>) -> Option<f64> {
	value.map(|v| v + 1.0)
}
fn int(text: &str) -> Option<f64> {
	text.trim().parse::<i64>().ok().map(|v| v as f64)
}

fn normalize_severity(value: Option<&Value>) -> Severity {
	if let Some(n) = value.and_then(Value::as_f64) {
		return if n >= 2.0 {
			Severity::Error
		} else if n == 1.0 {
			Severity::Warning
		} else {
			Severity::Info
		};
	}
	let normalized = value
		.and_then(Value::as_str)
		.unwrap_or("warning")
		.to_lowercase();
	if normalized.contains("error") || normalized == "fatal" {
		Severity::Error
	} else if normalized.contains("warn") || normalized == "convention" || normalized == "refactor" {
		Severity::Warning
	} else {
		Severity::Info
	}
}

fn lexical_normalize(path: &Path) -> PathBuf {
	let mut out = PathBuf::new();
	for component in path.components() {
		match component {
			Component::ParentDir => {
				out.pop();
			},
			Component::CurDir => {},
			other => out.push(other),
		}
	}
	out
}

pub fn relative_to(base: &Path, target: &Path) -> Option<String> {
	let rel = target.strip_prefix(base).ok()?;
	let text = rel.to_string_lossy().replace('\\', "/");
	(!text.is_empty()).then_some(text)
}

fn normalize_file(raw: &str, input: &Input) -> Option<String> {
	let mut file = raw.trim().trim_matches(|c| c == '"' || c == '\'').to_owned();
	if file.is_empty() || file.starts_with('<') {
		return None;
	}
	if let Some(rest) = file.strip_prefix("file://") {
		file = form_urlencoded::parse(format!("x={}", rest.replace('+', "%2B")).as_bytes())
			.next()
			.map(|(_, v)| v.into_owned())?;
	}
	let path = Path::new(&file);
	let absolute = if path.is_absolute() {
		lexical_normalize(path)
	} else {
		lexical_normalize(&input.checker_cwd.join(path))
	};
	relative_to(input.project_cwd, &absolute)
}

fn positive(value: Option<f64>) -> Option<u32> {
	value
		.filter(|v| v.fract() == 0.0 && *v > 0.0 && *v <= f64::from(u32::MAX))
		.map(|v| v as u32)
}

fn add(target: &mut Vec<Diagnostic>, input: &Input, fields: Fields) {
	let message = sanitize_text(fields.message.as_deref().unwrap_or_default())
		.split_whitespace()
		.collect::<Vec<_>>()
		.join(" ");
	if message.is_empty() {
		return;
	}
	let file = match &fields.file {
		Some(raw) => match normalize_file(raw, input) {
			Some(file) => Some(file),
			None => return,
		},
		None => None,
	};
	target.push(Diagnostic {
		checker: input.checker.to_owned(),
		file,
		line: positive(fields.line),
		column: positive(fields.column),
		end_line: positive(fields.end_line),
		end_column: positive(fields.end_column),
		code: fields.code.map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()),
		severity: normalize_severity(fields.severity.as_ref()),
		message,
		suggestion: fields.suggestion.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()),
	});
}

fn s(text: &str) -> Option<Value> {
	Some(Value::String(text.to_owned()))
}

pub(super) fn parse_json_values(text: &str) -> Vec<Value> {
	let sanitized = sanitize_text(text);
	let sanitized = sanitized.trim();
	if sanitized.is_empty() {
		return Vec::new();
	}
	if let Ok(value) = serde_json::from_str::<Value>(sanitized) {
		return vec![value];
	}
	let bytes = sanitized.as_bytes();
	let mut values = Vec::new();
	let mut start: Option<usize> = None;
	let (mut depth, mut in_string, mut escaped) = (0i64, false, false);
	for (index, &ch) in bytes.iter().enumerate() {
		let Some(begin) = start else {
			if ch == b'{' || ch == b'[' {
				start = Some(index);
				depth = 1;
				in_string = false;
				escaped = false;
			}
			continue;
		};
		if in_string {
			if escaped {
				escaped = false;
			} else if ch == b'\\' {
				escaped = true;
			} else if ch == b'"' {
				in_string = false;
			}
			continue;
		}
		match ch {
			b'"' => in_string = true,
			b'{' | b'[' => depth += 1,
			b'}' | b']' => depth -= 1,
			_ => {},
		}
		if depth != 0 {
			continue;
		}
		if let Ok(value) = serde_json::from_str::<Value>(&sanitized[begin..=index]) {
			values.push(value);
		}
		start = None;
	}
	values
}

fn all_json(input: &Input) -> Vec<Value> {
	let mut values = parse_json_values(input.stdout);
	values.extend(parse_json_values(input.stderr));
	values
}

fn combined(input: &Input) -> String {
	sanitize_text(&format!("{}\n{}", input.stdout, input.stderr))
}

fn parse_rust(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for value in parse_json_values(input.stdout) {
		let envelope = rec(&value);
		if sf(envelope, "reason").as_deref() != Some("compiler-message") {
			continue;
		}
		let message = nested(envelope, "message");
		let level = sf(message, "level");
		if !matches!(level.as_deref(), Some("error" | "warning")) {
			continue;
		}
		let spans: Vec<&Map<String, Value>> = arr(message.and_then(|m| m.get("spans"))).iter().filter_map(rec).collect();
		let primary = spans
			.iter()
			.find(|s| s.get("is_primary") == Some(&Value::Bool(true)))
			.or_else(|| spans.first())
			.copied();
		add(&mut out, input, Fields {
			file: sf(primary, "file_name"),
			line: nf(primary, "line_start"),
			column: nf(primary, "column_start"),
			end_line: nf(primary, "line_end"),
			end_column: nf(primary, "column_end"),
			code: sf(nested(message, "code"), "code"),
			severity: level.map(Value::String),
			message: sf(message, "message"),
			suggestion: sf(primary, "suggested_replacement"),
		});
	}
	out
}

fn parse_rust_test(input: &Input) -> Vec<Diagnostic> {
	let mut out = parse_rust(input);
	let re = Regex::new(r"thread '([^']+)' panicked at (.*?):(\d+):(\d+):").expect("static");
	for line in format!("{}\n{}", input.stdout, input.stderr).lines() {
		let Some(c) = re.captures(line) else { continue };
		add(&mut out, input, Fields {
			file: Some(c[2].to_owned()),
			line: int(&c[3]),
			column: int(&c[4]),
			severity: s("error"),
			code: Some("test-failure".into()),
			message: Some(format!("test {} panicked", &c[1])),
			..Default::default()
		});
	}
	out
}

fn parse_location(value: &str) -> Option<(String, Option<f64>, Option<f64>)> {
	let value = value.trim();
	if let Some(c) = Regex::new(r"^(.*?)\((\d+),(\d+)\)$").expect("static").captures(value) {
		return Some((c[1].to_owned(), int(&c[2]), int(&c[3])));
	}
	let c = Regex::new(r"^(.*?):(\d+)(?::(\d+))?$").expect("static").captures(value)?;
	Some((c[1].to_owned(), int(&c[2]), c.get(3).and_then(|m| int(m.as_str()))))
}

fn parse_go(input: &Input) -> Vec<Diagnostic> {
	fn visit(value: &Value, analyzer: Option<&str>, input: &Input, out: &mut Vec<Diagnostic>) {
		if let Some(items) = value.as_array() {
			for child in items {
				visit(child, analyzer, input, out);
			}
			return;
		}
		let Some(record) = value.as_object() else { return };
		let position = sf(Some(record), "posn").or_else(|| sf(Some(record), "position"));
		let message = sf(Some(record), "message");
		if let (Some(position), Some(message)) = (position, message) {
			if let Some((file, line, column)) = parse_location(&position) {
				add(out, input, Fields {
					file: Some(file),
					line,
					column,
					code: sf(Some(record), "category").or_else(|| analyzer.map(str::to_owned)),
					severity: s("warning"),
					message: Some(message),
					..Default::default()
				});
			}
			return;
		}
		for (key, child) in record {
			visit(child, Some(key), input, out);
		}
	}
	let mut out = Vec::new();
	for value in all_json(input) {
		visit(&value, None, input, &mut out);
	}
	if out.is_empty() { parse_generic(input) } else { out }
}

fn parse_go_test(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	let re = Regex::new(r"^\s*(?:\./)?(.+?\.go):(\d+)(?::(\d+))?:\s*(.+?)\s*$").expect("static");
	for value in parse_json_values(input.stdout) {
		let event = rec(&value);
		if let Some(output) = sf(event, "Output") {
			for line in output.split('\n') {
				let Some(c) = re.captures(line) else { continue };
				add(&mut out, input, Fields {
					file: Some(c[1].to_owned()),
					line: int(&c[2]),
					column: c.get(3).and_then(|m| int(m.as_str())),
					severity: s("error"),
					code: Some("test-failure".into()),
					message: Some(c[4].to_owned()),
					..Default::default()
				});
			}
		}
		if let Some(test) = sf(event, "Test")
			&& sf(event, "Action").as_deref() == Some("fail")
		{
			add(&mut out, input, Fields {
				severity: s("error"),
				code: Some("test-failure".into()),
				message: Some(format!("test {test} failed")),
				..Default::default()
			});
		}
	}
	if out.is_empty() { parse_generic(input) } else { out }
}

fn parse_staticcheck(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for value in all_json(input) {
		let r = rec(&value);
		let loc = nested(r, "location");
		let end = nested(r, "end");
		add(&mut out, input, Fields {
			file: sf(loc, "file"),
			line: nf(loc, "line"),
			column: nf(loc, "column"),
			end_line: nf(end, "line"),
			end_column: nf(end, "column"),
			code: sf(r, "code"),
			severity: Some(Value::String(sf(r, "severity").unwrap_or_else(|| "warning".into()))),
			message: sf(r, "message"),
			..Default::default()
		});
	}
	out
}

fn line_parser(input: &Input, pattern: &str, build: impl Fn(&regex::Captures) -> Fields) -> Vec<Diagnostic> {
	let re = Regex::new(pattern).expect("static");
	let mut out = Vec::new();
	for line in combined(input).split('\n') {
		if let Some(c) = re.captures(line.trim()) {
			add(&mut out, input, build(&c));
		}
	}
	out
}

fn parse_golangci(input: &Input) -> Vec<Diagnostic> {
	line_parser(input, r"^(.+?):(\d+):(\d+):\s+(.*?)\s+\(([A-Za-z0-9_-]+)\)$", |c| Fields {
		file: Some(c[1].to_owned()),
		line: int(&c[2]),
		column: int(&c[3]),
		code: Some(c[5].to_owned()),
		severity: s("warning"),
		message: Some(c[4].to_owned()),
		..Default::default()
	})
}

fn parse_flake8(input: &Input) -> Vec<Diagnostic> {
	line_parser(input, r"^(.+?):(\d+):(\d+):\s+([A-Z]+\d+)\s+(.*)$", |c| Fields {
		file: Some(c[1].to_owned()),
		line: int(&c[2]),
		column: int(&c[3]),
		code: Some(c[4].to_owned()),
		severity: s(if c[4].starts_with('F') || c[4].starts_with("E9") { "error" } else { "warning" }),
		message: Some(c[5].to_owned()),
		..Default::default()
	})
}

fn parse_ty(input: &Input) -> Vec<Diagnostic> {
	line_parser(input, r"^(.+?):(\d+):(\d+):\s+(error|warning|info)\[([^\]]+)\]\s+(.*)$", |c| Fields {
		file: Some(c[1].to_owned()),
		line: int(&c[2]),
		column: int(&c[3]),
		code: Some(c[5].to_owned()),
		severity: s(&c[4]),
		message: Some(c[6].to_owned()),
		..Default::default()
	})
}

fn parse_unix(input: &Input) -> Vec<Diagnostic> {
	line_parser(input, r"(?i)^(.+?):(\d+):(\d+):\s+(.*?)\s+\[(Error|Warning)/([^\]]+)\]$", |c| Fields {
		file: Some(c[1].to_owned()),
		line: int(&c[2]),
		column: int(&c[3]),
		code: Some(c[6].to_owned()),
		severity: s(&c[5]),
		message: Some(c[4].to_owned()),
		..Default::default()
	})
}

fn each_array_record(input: &Input, mut f: impl FnMut(&Map<String, Value>, &mut Vec<Diagnostic>)) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for root in all_json(input) {
		for value in arr(Some(&root)) {
			if let Some(r) = rec(value) {
				f(r, &mut out);
			} else {
				f(&Map::new(), &mut out);
			}
		}
	}
	out
}

fn parse_ruff(input: &Input) -> Vec<Diagnostic> {
	each_array_record(input, |r, out| {
		let r = Some(r);
		let loc = nested(r, "location");
		let end = nested(r, "end_location");
		add(out, input, Fields {
			file: sf(r, "filename"),
			line: nf(loc, "row"),
			column: nf(loc, "column"),
			end_line: nf(end, "row"),
			end_column: nf(end, "column"),
			code: sf(r, "code"),
			severity: s("warning"),
			message: sf(r, "message"),
			suggestion: sf(nested(r, "fix"), "message"),
		});
	})
}

fn parse_pyright(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for root in all_json(input) {
		for value in arr(rec(&root).and_then(|r| r.get("generalDiagnostics"))) {
			let r = rec(value);
			let range = nested(r, "range");
			let start = nested(range, "start");
			let end = nested(range, "end");
			add(&mut out, input, Fields {
				file: sf(r, "file"),
				line: zero_based(nf(start, "line")),
				column: zero_based(nf(start, "character")),
				end_line: zero_based(nf(end, "line")),
				end_column: zero_based(nf(end, "character")),
				code: sf(r, "rule"),
				severity: sf(r, "severity").map(Value::String),
				message: sf(r, "message"),
				..Default::default()
			});
		}
	}
	out
}

fn parse_pylint(input: &Input) -> Vec<Diagnostic> {
	each_array_record(input, |r, out| {
		let r = Some(r);
		add(out, input, Fields {
			file: sf(r, "path"),
			line: nf(r, "line"),
			column: zero_based(nf(r, "column")),
			end_line: nf(r, "endLine"),
			end_column: zero_based(nf(r, "endColumn")),
			code: sf(r, "symbol").or_else(|| sf(r, "message-id")),
			severity: sf(r, "type").map(Value::String),
			message: sf(r, "message"),
			..Default::default()
		});
	})
}

fn parse_eslint(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for root in all_json(input) {
		for file_value in arr(Some(&root)) {
			let file = rec(file_value);
			for message_value in arr(file.and_then(|f| f.get("messages"))) {
				let m = rec(message_value);
				add(&mut out, input, Fields {
					file: sf(file, "filePath"),
					line: nf(m, "line"),
					column: nf(m, "column"),
					end_line: nf(m, "endLine"),
					end_column: nf(m, "endColumn"),
					code: sf(m, "ruleId"),
					severity: m.and_then(|m| m.get("severity")).filter(|v| v.is_number()).cloned(),
					message: sf(m, "message"),
					suggestion: nested(m, "fix").map(|_| "automatic fix available".to_owned()),
				});
			}
		}
	}
	out
}

fn parse_biome(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for root in all_json(input) {
		for value in arr(rec(&root).and_then(|r| r.get("diagnostics"))) {
			let r = rec(value);
			let location = nested(r, "location");
			add(&mut out, input, Fields {
				file: sf(nested(location, "path"), "file").or_else(|| sf(location, "path")),
				code: sf(r, "category"),
				severity: sf(r, "severity").map(Value::String),
				message: sf(r, "description").or_else(|| sf(r, "message")).or_else(|| sf(r, "title")),
				..Default::default()
			});
		}
	}
	out
}

fn parse_deno(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for root in all_json(input) {
		let Some(record) = rec(&root) else { continue };
		for value in arr(record.get("diagnostics")) {
			let e = rec(value);
			let range = nested(e, "range");
			let (start, end) = (nested(range, "start"), nested(range, "end"));
			add(&mut out, input, Fields {
				file: sf(e, "filename"),
				line: nf(start, "line"),
				column: zero_based(nf(start, "col")),
				end_line: nf(end, "line"),
				end_column: zero_based(nf(end, "col")),
				code: sf(e, "code"),
				severity: s("warning"),
				message: sf(e, "message"),
				suggestion: sf(e, "hint"),
			});
		}
		for value in arr(record.get("errors")) {
			let e = rec(value);
			add(&mut out, input, Fields {
				file: sf(e, "file_path"),
				severity: s("error"),
				message: sf(e, "message"),
				..Default::default()
			});
		}
	}
	out
}

fn parse_stylelint(input: &Input) -> Vec<Diagnostic> {
	each_array_record(input, |r, out| {
		let Some(source) = sf(Some(r), "source") else { return };
		for warning in arr(r.get("warnings")) {
			let e = rec(warning);
			add(out, input, Fields {
				file: Some(source.clone()),
				line: nf(e, "line"),
				column: nf(e, "column"),
				end_line: nf(e, "endLine"),
				end_column: nf(e, "endColumn"),
				code: sf(e, "rule"),
				severity: sf(e, "severity").map(Value::String),
				message: sf(e, "text"),
				..Default::default()
			});
		}
	})
}

fn parse_rubocop(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for root in all_json(input) {
		for file_value in arr(rec(&root).and_then(|r| r.get("files"))) {
			let file = rec(file_value);
			for offense_value in arr(file.and_then(|f| f.get("offenses"))) {
				let o = rec(offense_value);
				let loc = nested(o, "location");
				add(&mut out, input, Fields {
					file: sf(file, "path"),
					line: nf(loc, "start_line"),
					column: nf(loc, "start_column"),
					end_line: nf(loc, "last_line"),
					end_column: nf(loc, "last_column"),
					code: sf(o, "cop_name"),
					severity: sf(o, "severity").map(Value::String),
					message: sf(o, "message"),
					suggestion: (o.and_then(|o| o.get("corrected")) == Some(&Value::Bool(true)))
						.then(|| "automatic correction available".to_owned()),
				});
			}
		}
	}
	out
}

fn parse_phpstan(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for root in all_json(input) {
		let r = rec(&root);
		if let Some(files) = nested(r, "files") {
			for (name, file) in files {
				for message_value in arr(file.get("messages")) {
					let m = rec(message_value);
					add(&mut out, input, Fields {
						file: Some(name.clone()),
						line: nf(m, "line"),
						code: sf(m, "identifier"),
						severity: s("error"),
						message: sf(m, "message"),
						..Default::default()
					});
				}
			}
		}
		for error in arr(r.and_then(|r| r.get("errors"))) {
			if let Some(text) = error.as_str() {
				add(&mut out, input, Fields { severity: s("error"), message: Some(text.to_owned()), ..Default::default() });
			}
		}
	}
	out
}

fn parse_psalm(input: &Input) -> Vec<Diagnostic> {
	each_array_record(input, |r, out| {
		let r = Some(r);
		add(out, input, Fields {
			file: sf(r, "file_name").or_else(|| sf(r, "file_path")),
			line: nf(r, "line_from"),
			column: nf(r, "column_from"),
			end_line: nf(r, "line_to"),
			end_column: nf(r, "column_to"),
			code: sf(r, "type").or_else(|| nf(r, "shortcode").map(|n| format!("{n}"))),
			severity: sf(r, "severity").map(Value::String),
			message: sf(r, "message"),
			..Default::default()
		});
	})
}

fn parse_swiftlint(input: &Input) -> Vec<Diagnostic> {
	each_array_record(input, |r, out| {
		let r = Some(r);
		add(out, input, Fields {
			file: sf(r, "file"),
			line: nf(r, "line"),
			column: nf(r, "character"),
			code: sf(r, "rule_id"),
			severity: sf(r, "severity").map(Value::String),
			message: sf(r, "reason"),
			..Default::default()
		});
	})
}

fn parse_dart(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for line in combined(input).split('\n') {
		let fields: Vec<&str> = line.split('|').collect();
		if fields.len() < 8 {
			continue;
		}
		add(&mut out, input, Fields {
			severity: s(fields[0]),
			code: Some(fields[2].to_owned()),
			file: Some(fields[3].to_owned()),
			line: int(fields[4]),
			column: int(fields[5]),
			message: Some(fields[7..].join("|")),
			..Default::default()
		});
	}
	out
}

fn parse_credo(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for root in all_json(input) {
		for value in arr(rec(&root).and_then(|r| r.get("issues"))) {
			let r = rec(value);
			add(&mut out, input, Fields {
				file: sf(r, "filename"),
				line: nf(r, "line_no").or_else(|| nf(r, "line")),
				column: nf(r, "column"),
				code: sf(r, "check"),
				severity: sf(r, "priority").or_else(|| sf(r, "category")).map(Value::String),
				message: sf(r, "message"),
				..Default::default()
			});
		}
	}
	out
}

fn parse_shellcheck(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for root in all_json(input) {
		let values = if root.is_array() { arr(Some(&root)) } else { arr(rec(&root).and_then(|r| r.get("comments"))) };
		for value in values {
			let r = rec(value);
			add(&mut out, input, Fields {
				file: sf(r, "file"),
				line: nf(r, "line"),
				column: nf(r, "column"),
				end_line: nf(r, "endLine"),
				end_column: nf(r, "endColumn"),
				code: nf(r, "code").map(|c| format!("SC{c}")),
				severity: sf(r, "level").map(Value::String),
				message: sf(r, "message"),
				suggestion: r
					.and_then(|r| r.get("fix"))
					.filter(|f| !f.is_null() && f != &&Value::Bool(false))
					.map(|_| "automatic fix available".to_owned()),
			});
		}
	}
	out
}

fn parse_hlint(input: &Input) -> Vec<Diagnostic> {
	each_array_record(input, |r, out| {
		let r = Some(r);
		add(out, input, Fields {
			file: sf(r, "file"),
			line: nf(r, "startLine"),
			column: nf(r, "startColumn"),
			end_line: nf(r, "endLine"),
			end_column: nf(r, "endColumn"),
			code: sf(r, "hint"),
			severity: sf(r, "severity").map(Value::String),
			message: sf(r, "hint").or_else(|| sf(r, "from")),
			suggestion: sf(r, "to"),
		});
	})
}

fn parse_terraform(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for root in all_json(input) {
		for value in arr(rec(&root).and_then(|r| r.get("diagnostics"))) {
			let r = rec(value);
			let range = nested(r, "range");
			let (start, end) = (nested(range, "start"), nested(range, "end"));
			let message = [sf(r, "summary"), sf(r, "detail")].into_iter().flatten().collect::<Vec<_>>().join(": ");
			add(&mut out, input, Fields {
				file: sf(range, "filename"),
				line: nf(start, "line"),
				column: nf(start, "column"),
				end_line: nf(end, "line"),
				end_column: nf(end, "column"),
				severity: sf(r, "severity").map(Value::String),
				message: Some(message),
				..Default::default()
			});
		}
	}
	out
}

fn parse_tflint(input: &Input) -> Vec<Diagnostic> {
	let mut out = Vec::new();
	for root in all_json(input) {
		let root_record = rec(&root);
		for value in arr(root_record.and_then(|r| r.get("issues"))) {
			let r = rec(value);
			let rule = nested(r, "rule");
			let range = nested(r, "range");
			let (start, end) = (nested(range, "start"), nested(range, "end"));
			add(&mut out, input, Fields {
				file: sf(range, "filename"),
				line: nf(start, "line"),
				column: nf(start, "column"),
				end_line: nf(end, "line"),
				end_column: nf(end, "column"),
				code: sf(rule, "name"),
				severity: sf(rule, "severity").map(Value::String),
				message: sf(r, "message"),
				..Default::default()
			});
		}
		for value in arr(root_record.and_then(|r| r.get("errors"))) {
			add(&mut out, input, Fields { severity: s("error"), message: sf(rec(value), "message"), ..Default::default() });
		}
	}
	out
}

fn parse_actionlint(input: &Input) -> Vec<Diagnostic> {
	each_array_record(input, |r, out| {
		let r = Some(r);
		add(out, input, Fields {
			file: sf(r, "filepath"),
			line: nf(r, "line"),
			column: nf(r, "column"),
			code: sf(r, "kind"),
			severity: s("error"),
			message: sf(r, "message"),
			..Default::default()
		});
	})
}

fn parse_generic(input: &Input) -> Vec<Diagnostic> {
	let msvc = Regex::new(r"(?i)^(.*?)\((\d+),(\d+)\):\s*(error|warning|info)(?:\s+([A-Za-z]+\d+))?:\s*(.*?)(?:\s+\[[^\]]+\])?$").expect("static");
	let gcc = Regex::new(r"(?i)^(.*?):(\d+)(?::(\d+))?:\s*(error|warning|info|note)(?:\s*\[([^\]]+)\])?:\s*(.*)$").expect("static");
	let mut out = Vec::new();
	for line in combined(input).split('\n') {
		let trimmed = line.trim();
		if trimmed.is_empty() {
			continue;
		}
		if let Some(c) = msvc.captures(trimmed) {
			add(&mut out, input, Fields {
				file: Some(c[1].to_owned()),
				line: int(&c[2]),
				column: int(&c[3]),
				severity: s(&c[4]),
				code: c.get(5).map(|m| m.as_str().to_owned()),
				message: Some(c[6].to_owned()),
				..Default::default()
			});
			continue;
		}
		let Some(c) = gcc.captures(trimmed) else { continue };
		add(&mut out, input, Fields {
			file: Some(c[1].to_owned()),
			line: int(&c[2]),
			column: c.get(3).and_then(|m| int(m.as_str())),
			severity: s(&c[4]),
			code: c.get(5).map(|m| m.as_str().to_owned()),
			message: Some(c[6].to_owned()),
			..Default::default()
		});
	}
	out
}
