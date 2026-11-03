//! Rule discovery and TTSR matching shared by `harvest ttsr` and future hosts.
//! Ported from `capability/rule.ts`, `discovery/*` rule providers and
//! `export/ttsr.ts`.

use std::{
	collections::{BTreeSet, HashSet},
	path::{Path, PathBuf},
};

use anyhow::Result;
use serde_json::{Map, Value};

pub const BUILTIN_DEFAULTS_PROVIDER_ID: &str = "builtin-defaults";
pub const MAIN_AGENT_RULE_NAME: &str = "main";
const CONDITION_GLOB_SCOPE_TOOLS: [&str; 2] = ["edit", "write"];

const BUILTIN_RULE_SOURCES: &[(&str, &str)] = &[
	("go-add-cleanup", include_str!("../assets/ttsr-rules/go-add-cleanup.md")),
	("go-bench-loop", include_str!("../assets/ttsr-rules/go-bench-loop.md")),
	("go-exp-promoted", include_str!("../assets/ttsr-rules/go-exp-promoted.md")),
	("go-ioutil", include_str!("../assets/ttsr-rules/go-ioutil.md")),
	("go-join-hostport", include_str!("../assets/ttsr-rules/go-join-hostport.md")),
	("go-new-expr", include_str!("../assets/ttsr-rules/go-new-expr.md")),
	("go-rand-v2", include_str!("../assets/ttsr-rules/go-rand-v2.md")),
	("go-range-int", include_str!("../assets/ttsr-rules/go-range-int.md")),
	("rs-box-leak", include_str!("../assets/ttsr-rules/rs-box-leak.md")),
	("rs-future-prelude", include_str!("../assets/ttsr-rules/rs-future-prelude.md")),
	("rs-lazylock", include_str!("../assets/ttsr-rules/rs-lazylock.md")),
	("rs-match-ergonomics", include_str!("../assets/ttsr-rules/rs-match-ergonomics.md")),
	("rs-parking-lot", include_str!("../assets/ttsr-rules/rs-parking-lot.md")),
	("rs-result-type", include_str!("../assets/ttsr-rules/rs-result-type.md")),
	("ts-bare-catch", include_str!("../assets/ttsr-rules/ts-bare-catch.md")),
	("ts-import-type", include_str!("../assets/ttsr-rules/ts-import-type.md")),
	("ts-no-any", include_str!("../assets/ttsr-rules/ts-no-any.md")),
	("ts-no-deprecated-leftovers", include_str!("../assets/ttsr-rules/ts-no-deprecated-leftovers.md")),
	("ts-no-dynamic-import", include_str!("../assets/ttsr-rules/ts-no-dynamic-import.md")),
	("ts-no-inline-cast-access", include_str!("../assets/ttsr-rules/ts-no-inline-cast-access.md")),
	("ts-no-local-is-record", include_str!("../assets/ttsr-rules/ts-no-local-is-record.md")),
	("ts-no-return-type", include_str!("../assets/ttsr-rules/ts-no-return-type.md")),
	("ts-no-test-timers", include_str!("../assets/ttsr-rules/ts-no-test-timers.md")),
	("ts-no-tiny-functions", include_str!("../assets/ttsr-rules/ts-no-tiny-functions.md")),
	("ts-promise-with-resolvers", include_str!("../assets/ttsr-rules/ts-promise-with-resolvers.md")),
	("ts-redundant-clear-guard", include_str!("../assets/ttsr-rules/ts-redundant-clear-guard.md")),
	("ts-set-map", include_str!("../assets/ttsr-rules/ts-set-map.md")),
];

#[derive(Debug, Clone, serde::Serialize)]
pub struct Rule {
	pub name:          String,
	pub path:          String,
	pub content:       String,
	pub globs:         Option<Vec<String>>,
	pub always_apply:  bool,
	pub description:   Option<String>,
	pub condition:     Option<Vec<String>>,
	pub ast_condition: Option<Vec<String>>,
	pub scope:         Option<Vec<String>>,
	pub agents:        Option<Vec<String>>,
	pub provider:      String,
}

/// Split `---` YAML frontmatter from a Markdown body.
pub fn parse_frontmatter(content: &str) -> (Map<String, Value>, String) {
	let normalized = content.replace("\r\n", "\n");
	let Some(rest) = normalized.strip_prefix("---\n") else {
		return (Map::new(), normalized);
	};
	let (yaml, body) = match rest.find("\n---") {
		Some(end) => {
			let after = &rest[end + 4..];
			let body = after.strip_prefix('\n').unwrap_or(after);
			(&rest[..end], body.to_owned())
		},
		None => return (Map::new(), normalized),
	};
	if let Ok(Value::Object(map)) = serde_yaml_ng::from_str::<Value>(yaml) {
		return (map, body);
	}
	// Fallback: tolerate malformed YAML with simple `key: value` lines.
	let mut map = Map::new();
	for line in yaml.lines() {
		if let Some((key, value)) = line.split_once(':') {
			let key = key.trim();
			if key.is_empty() || key.starts_with(' ') {
				continue;
			}
			let value = value.trim();
			let value = match value {
				"true" => Value::Bool(true),
				"false" => Value::Bool(false),
				_ => Value::String(value.to_owned()),
			};
			map.insert(key.to_owned(), value);
		}
	}
	(map, body)
}

fn normalize_rule_field(value: Option<&Value>) -> Option<Vec<String>> {
	let tokens: Vec<String> = match value? {
		Value::String(text) => {
			let token = text.trim();
			if token.is_empty() {
				return None;
			}
			vec![token.to_owned()]
		},
		Value::Array(items) => items
			.iter()
			.filter_map(Value::as_str)
			.map(|item| item.trim().to_owned())
			.filter(|item| !item.is_empty())
			.collect(),
		_ => return None,
	};
	if tokens.is_empty() {
		return None;
	}
	Some(dedupe(tokens))
}

fn dedupe(tokens: Vec<String>) -> Vec<String> {
	let mut seen = HashSet::new();
	tokens
		.into_iter()
		.filter(|token| seen.insert(token.clone()))
		.collect()
}

fn split_scope_tokens(value: &str) -> Vec<String> {
	let mut tokens = Vec::new();
	let mut current = String::new();
	let (mut paren, mut bracket, mut brace) = (0usize, 0usize, 0usize);
	let mut quote: Option<char> = None;
	let mut previous: Option<char> = None;
	for ch in value.chars() {
		if let Some(q) = quote {
			current.push(ch);
			if ch == q && previous != Some('\\') {
				quote = None;
			}
			previous = Some(ch);
			continue;
		}
		match ch {
			'"' | '\'' => {
				quote = Some(ch);
				current.push(ch);
			},
			'(' => {
				paren += 1;
				current.push(ch);
			},
			')' => {
				paren = paren.saturating_sub(1);
				current.push(ch);
			},
			'[' => {
				bracket += 1;
				current.push(ch);
			},
			']' => {
				bracket = bracket.saturating_sub(1);
				current.push(ch);
			},
			'{' => {
				brace += 1;
				current.push(ch);
			},
			'}' => {
				brace = brace.saturating_sub(1);
				current.push(ch);
			},
			',' if paren == 0 && bracket == 0 && brace == 0 => {
				let token = current.trim();
				if !token.is_empty() {
					tokens.push(token.to_owned());
				}
				current.clear();
			},
			_ => current.push(ch),
		}
		previous = Some(ch);
	}
	let tail = current.trim();
	if !tail.is_empty() {
		tokens.push(tail.to_owned());
	}
	tokens
}

fn normalize_scope_field(value: Option<&Value>) -> Option<Vec<String>> {
	let normalized = normalize_rule_field(value)?;
	let tokens: Vec<String> = normalized
		.iter()
		.flat_map(|token| split_scope_tokens(token))
		.map(|token| {
			let bytes = token.as_bytes();
			if bytes.len() >= 2
				&& (bytes[0] == b'"' || bytes[0] == b'\'')
				&& bytes[bytes.len() - 1] == bytes[0]
			{
				token[1..token.len() - 1].trim().to_owned()
			} else {
				token
			}
		})
		.filter(|token| !token.is_empty())
		.collect();
	if tokens.is_empty() {
		return None;
	}
	Some(dedupe(tokens))
}

pub fn parse_rule_agents(value: Option<&Value>) -> Option<Vec<String>> {
	let tokens = normalize_scope_field(value)?;
	let comma = regex::Regex::new(r"\s*,\s*").expect("static regex");
	Some(dedupe(
		tokens
			.into_iter()
			.map(|token| comma.replace_all(&token, ",").to_lowercase())
			.collect(),
	))
}

/// Compile a Bun.Glob-compatible pattern (`*` stays within a segment).
pub fn compile_glob(pattern: &str) -> Option<globset::GlobMatcher> {
	globset::GlobBuilder::new(pattern)
		.literal_separator(true)
		.build()
		.ok()
		.map(|glob| glob.compile_matcher())
}

pub fn rule_applies_to_agent(rule: &Rule, agent: Option<&str>) -> bool {
	let Some(patterns) = rule.agents.as_ref().filter(|p| !p.is_empty()) else {
		return true;
	};
	let Some(agent) = agent else { return true };
	let name = agent.trim().to_lowercase();
	patterns.iter().any(|pattern| {
		pattern == &name || compile_glob(pattern).is_some_and(|glob| glob.is_match(&name))
	})
}

fn is_likely_file_glob(value: &str) -> bool {
	let token = value.trim();
	if token.is_empty() {
		return false;
	}
	if token.chars().any(|c| matches!(c, '\\' | '^' | '$' | '+' | '|' | '(' | ')')) {
		return false;
	}
	if !token.chars().any(|c| matches!(c, '?' | '*' | '[' | ']' | '{' | '}')) {
		return false;
	}
	if token.contains('/') {
		return true;
	}
	regex::Regex::new(r"^\*\.[^\s/]+$")
		.expect("static regex")
		.is_match(token)
}

fn parse_condition_and_scope(
	frontmatter: &Map<String, Value>,
) -> (Option<Vec<String>>, Option<Vec<String>>, Option<Vec<String>>) {
	let raw = frontmatter
		.get("condition")
		.or_else(|| frontmatter.get("ttsr_trigger"))
		.or_else(|| frontmatter.get("ttsrTrigger"));
	let parsed_condition = normalize_rule_field(raw);
	let ast_condition = normalize_rule_field(frontmatter.get("astCondition"));
	let parsed_scope = normalize_scope_field(frontmatter.get("scope"));
	let mut inferred = Vec::new();
	let mut condition = Vec::new();
	for token in parsed_condition.unwrap_or_default() {
		if is_likely_file_glob(&token) {
			for tool in CONDITION_GLOB_SCOPE_TOOLS {
				inferred.push(format!("tool:{tool}({token})"));
			}
			continue;
		}
		condition.push(token);
	}
	if condition.is_empty() && !inferred.is_empty() {
		condition.push(".*".to_owned());
	}
	let mut scope = parsed_scope.unwrap_or_default();
	scope.extend(inferred);
	(
		(!condition.is_empty()).then(|| dedupe(condition)),
		ast_condition,
		(!scope.is_empty()).then(|| dedupe(scope)),
	)
}

/// Build a canonical rule from Markdown (`buildRuleFromMarkdown`).
pub fn build_rule_from_markdown(
	name: &str,
	content: &str,
	path: &str,
	provider: &str,
	rule_name: Option<&str>,
) -> (Rule, bool) {
	let (frontmatter, body) = parse_frontmatter(content);
	let (condition, ast_condition, scope) = parse_condition_and_scope(&frontmatter);
	let globs = match frontmatter.get("globs") {
		Some(Value::Array(items)) => {
			Some(items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
		},
		Some(Value::String(text)) => Some(vec![text.clone()]),
		_ => None,
	};
	let resolved = rule_name.map_or_else(
		|| {
			name
				.strip_suffix(".mdc")
				.or_else(|| name.strip_suffix(".md"))
				.unwrap_or(name)
				.to_owned()
		},
		str::to_owned,
	);
	let enabled = frontmatter.get("enabled") != Some(&Value::Bool(false));
	(
		Rule {
			name: resolved,
			path: path.to_owned(),
			content: body,
			globs,
			always_apply: frontmatter.get("alwaysApply") == Some(&Value::Bool(true)),
			description: frontmatter
				.get("description")
				.and_then(Value::as_str)
				.map(str::to_owned),
			condition,
			ast_condition,
			scope,
			agents: parse_rule_agents(frontmatter.get("agents")),
			provider: provider.to_owned(),
		},
		enabled,
	)
}

/// Compile a rule `condition`, translating a leading `(?ims)` group.
pub fn compile_rule_condition(pattern: &str) -> Result<fancy_regex::Regex> {
	Ok(fancy_regex::Regex::new(pattern)?)
}

fn load_dir(dir: &Path, extensions: &[&str], provider: &str, out: &mut Vec<Rule>) {
	let Ok(entries) = std::fs::read_dir(dir) else { return };
	let mut files: Vec<PathBuf> = entries
		.filter_map(|entry| entry.ok().map(|e| e.path()))
		.filter(|path| {
			path.is_file()
				&& path
					.extension()
					.and_then(|e| e.to_str())
					.is_some_and(|e| extensions.contains(&e))
		})
		.collect();
	files.sort();
	for file in files {
		let Ok(content) = std::fs::read_to_string(&file) else { continue };
		let name = file.file_name().and_then(|n| n.to_str()).unwrap_or_default();
		let (rule, enabled) =
			build_rule_from_markdown(name, &content, &file.to_string_lossy(), provider, None);
		if enabled {
			out.push(rule);
		}
	}
}

fn load_single(path: &Path, rule_name: &str, provider: &str, always: bool, out: &mut Vec<Rule>) {
	let Ok(content) = std::fs::read_to_string(path) else { return };
	if content.trim().is_empty() {
		return;
	}
	let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
	let (mut rule, enabled) =
		build_rule_from_markdown(name, &content, &path.to_string_lossy(), provider, Some(rule_name));
	if enabled {
		rule.always_apply |= always;
		out.push(rule);
	}
}

fn home() -> Option<PathBuf> {
	std::env::var_os("HOME")
		.or_else(|| std::env::var_os("USERPROFILE"))
		.map(PathBuf::from)
}

fn config_dir_name() -> String {
	std::env::var("HARVEST_CONFIG_DIR")
		.or_else(|_| std::env::var("PI_CONFIG_DIR"))
		.unwrap_or_else(|_| ".harvest".to_owned())
}

fn find_upwards(start: &Path, name: &str) -> Option<PathBuf> {
	let mut current = Some(start);
	while let Some(dir) = current {
		let candidate = dir.join(name);
		if candidate.exists() {
			return Some(candidate);
		}
		current = dir.parent();
	}
	None
}

/// Load every rule the project/user config exposes, first-wins by name in
/// provider priority order (native, cursor, windsurf, cline, builtin).
pub fn load_rules(cwd: &Path, agent_dir: Option<&Path>) -> Vec<Rule> {
	let mut items = Vec::new();
	let config = config_dir_name();
	// native
	load_dir(&cwd.join(&config).join("rules"), &["md", "mdc"], "native", &mut items);
	if let Some(dir) = agent_dir {
		load_dir(&dir.join("rules"), &["md", "mdc"], "native", &mut items);
		load_single(&dir.join("RULES.md"), "RULES", "native", true, &mut items);
	}
	if let Some(project) = find_upwards(cwd, &config) {
		load_single(&project.join("RULES.md"), "RULES@project", "native", true, &mut items);
	}
	// cursor
	if let Some(home) = home() {
		load_dir(&home.join(".cursor/rules"), &["mdc", "md"], "cursor", &mut items);
	}
	load_dir(&cwd.join(".cursor/rules"), &["mdc", "md"], "cursor", &mut items);
	load_single(&cwd.join(".cursorrules"), "cursorrules", "cursor", false, &mut items);
	// windsurf
	if let Some(home) = home() {
		load_single(
			&home.join(".codeium/windsurf/memories/global_rules.md"),
			"global_rules",
			"windsurf",
			false,
			&mut items,
		);
	}
	load_dir(&cwd.join(".windsurf/rules"), &["md"], "windsurf", &mut items);
	load_single(&cwd.join(".windsurfrules"), "windsurfrules", "windsurf", false, &mut items);
	// cline
	if let Some(found) = find_upwards(cwd, ".clinerules") {
		if found.is_dir() {
			load_dir(&found, &["md"], "cline", &mut items);
		} else {
			load_single(&found, "clinerules", "cline", false, &mut items);
		}
	}
	// builtin defaults
	for (name, content) in BUILTIN_RULE_SOURCES {
		let virtual_path = format!("{BUILTIN_DEFAULTS_PROVIDER_ID}:{name}.md");
		let (rule, _) = build_rule_from_markdown(
			name,
			content,
			&virtual_path,
			BUILTIN_DEFAULTS_PROVIDER_ID,
			Some(name),
		);
		items.push(rule);
	}
	let mut seen = BTreeSet::new();
	items
		.into_iter()
		.filter(|rule| !rule.name.is_empty() && seen.insert(rule.name.clone()))
		.collect()
}

/// TTSR settings group (`ttsr.*`).
#[derive(Debug, Clone)]
pub struct TtsrSettings {
	pub enabled:        bool,
	pub builtin_rules:  bool,
	pub disabled_rules: Vec<String>,
}

impl TtsrSettings {
	pub fn load(cwd: &Path, agent_dir: Option<&Path>) -> Self {
		let settings = crate::settings::Settings::load(agent_dir, cwd, &[]).ok();
		let get = |key: &str| settings.as_ref().and_then(|s| s.get(key));
		Self {
			enabled:        get("ttsr.enabled").and_then(|v| v.as_bool()).unwrap_or(true),
			builtin_rules:  get("ttsr.builtinRules")
				.and_then(|v| v.as_bool())
				.unwrap_or(true),
			disabled_rules: get("ttsr.disabledRules")
				.and_then(|v| v.as_array().cloned())
				.unwrap_or_default()
				.iter()
				.filter_map(|v| v.as_str().map(|s| s.trim().to_owned()))
				.filter(|s| !s.is_empty())
				.collect(),
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchSource {
	Text,
	Thinking,
	Tool,
}

pub struct MatchContext {
	pub source:     MatchSource,
	pub tool_name:  Option<String>,
	pub file_paths: Vec<String>,
}

pub struct ToolScope {
	pub tool_name: Option<String>,
	pub path_glob: Option<globset::GlobMatcher>,
}

/// Parse `tool:edit(*.ts)` / `edit(*.ts)` / `tool` scope tokens.
pub fn parse_tool_scope_token(token: &str) -> Option<ToolScope> {
	let re = regex::Regex::new(
		r"(?i)^(?:(?P<prefix>tool)(?::(?P<tool>[a-z0-9_-]+))?|(?P<bare>[a-z0-9_-]+))(?:\((?P<path>[^)]+)\))?$",
	)
	.expect("static regex");
	let caps = re.captures(token)?;
	let has_prefix = caps.name("prefix").is_some();
	let tool = caps
		.name("tool")
		.map(|m| m.as_str())
		.or_else(|| if has_prefix { None } else { caps.name("bare").map(|m| m.as_str()) })
		.map(|t| t.trim().to_lowercase());
	let path = caps.name("path").map(|m| m.as_str().trim().to_owned());
	Some(ToolScope {
		tool_name: tool,
		path_glob: path.and_then(|p| compile_glob(&p)),
	})
}

pub fn matches_glob(glob: &globset::GlobMatcher, file_paths: &[String]) -> bool {
	for file in file_paths {
		let normalized = file.replace('\\', "/");
		if glob.is_match(&normalized) {
			return true;
		}
		if let Some(index) = normalized.rfind('/') {
			if glob.is_match(&normalized[index + 1..]) {
				return true;
			}
		}
	}
	false
}

pub fn compile_path_globs(globs: Option<&Vec<String>>) -> Option<Vec<globset::GlobMatcher>> {
	let compiled: Vec<_> = globs?
		.iter()
		.map(|g| g.trim())
		.filter(|g| !g.is_empty())
		.filter_map(compile_glob)
		.collect();
	(!compiled.is_empty()).then_some(compiled)
}

struct Entry {
	rule:          Rule,
	conditions:    Vec<fancy_regex::Regex>,
	ast_conditions: Vec<String>,
	allow_text:    bool,
	allow_thinking: bool,
	allow_any_tool: bool,
	tool_scopes:   Vec<ToolScope>,
	globs:         Option<Vec<globset::GlobMatcher>>,
}

/// Snapshot matcher equivalent to `TtsrManager.checkSnapshot`/`checkAstSnapshot`.
pub struct TtsrManager {
	enabled: bool,
	entries: Vec<Entry>,
}

impl TtsrManager {
	pub const fn new(enabled: bool) -> Self {
		Self { enabled, entries: Vec::new() }
	}

	pub fn add_rule(&mut self, rule: &Rule) -> bool {
		if !self.enabled || self.entries.iter().any(|e| e.rule.name == rule.name) {
			return false;
		}
		let conditions: Vec<_> = rule
			.condition
			.iter()
			.flatten()
			.filter_map(|p| compile_rule_condition(p).ok())
			.collect();
		let ast_conditions: Vec<String> = rule
			.ast_condition
			.iter()
			.flatten()
			.map(|p| p.trim().to_owned())
			.filter(|p| !p.is_empty())
			.collect();
		if conditions.is_empty() && ast_conditions.is_empty() {
			return false;
		}
		let (mut text, mut thinking, mut any_tool, mut scopes) = (true, false, true, Vec::new());
		if let Some(scope) = rule.scope.as_ref().filter(|s| !s.is_empty()) {
			text = false;
			any_tool = false;
			for raw in scope {
				let token = raw.trim();
				let lower = token.to_lowercase();
				if token.is_empty() {
					continue;
				}
				match lower.as_str() {
					"text" => text = true,
					"thinking" => thinking = true,
					"tool" | "toolcall" => any_tool = true,
					_ => {
						let Some(parsed) = parse_tool_scope_token(token) else { continue };
						if parsed.tool_name.is_none() && parsed.path_glob.is_none() {
							any_tool = true;
						} else {
							scopes.push(parsed);
						}
					},
				}
			}
		}
		if !(text || thinking || any_tool || !scopes.is_empty()) {
			return false;
		}
		self.entries.push(Entry {
			rule: rule.clone(),
			conditions,
			ast_conditions,
			allow_text: text,
			allow_thinking: thinking,
			allow_any_tool: any_tool,
			tool_scopes: scopes,
			globs: compile_path_globs(rule.globs.as_ref()),
		});
		true
	}

	pub fn rules(&self) -> Vec<Rule> {
		self.entries.iter().map(|e| e.rule.clone()).collect()
	}

	fn scope_ok(entry: &Entry, context: &MatchContext) -> bool {
		let scope = match context.source {
			MatchSource::Text => entry.allow_text,
			MatchSource::Thinking => entry.allow_thinking,
			MatchSource::Tool => {
				entry.allow_any_tool || {
					let tool = context.tool_name.as_ref().map(|t| t.trim().to_lowercase());
					entry.tool_scopes.iter().any(|scope| {
						scope.tool_name.as_ref().is_none_or(|n| Some(n) == tool.as_ref())
							&& scope
								.path_glob
								.as_ref()
								.is_none_or(|g| matches_glob(g, &context.file_paths))
					})
				}
			},
		};
		scope
			&& entry
				.globs
				.as_ref()
				.is_none_or(|globs| globs.iter().any(|g| matches_glob(g, &context.file_paths)))
	}

	pub fn check_snapshot(&self, snapshot: &str, context: &MatchContext) -> Vec<String> {
		if !self.enabled {
			return Vec::new();
		}
		self
			.entries
			.iter()
			.filter(|e| Self::scope_ok(e, context))
			.filter(|e| e.conditions.iter().any(|c| c.is_match(snapshot).unwrap_or(false)))
			.map(|e| e.rule.name.clone())
			.collect()
	}

	pub fn check_ast_snapshot(&self, snapshot: &str, context: &MatchContext) -> Vec<String> {
		if !self.enabled || context.source != MatchSource::Tool {
			return Vec::new();
		}
		let Some(lang) = derive_lang(&context.file_paths) else { return Vec::new() };
		self
			.entries
			.iter()
			.filter(|e| !e.ast_conditions.is_empty() && Self::scope_ok(e, context))
			.filter(|e| ast_any_match(&e.ast_conditions, snapshot, &lang).unwrap_or(false))
			.map(|e| e.rule.name.clone())
			.collect()
	}
}

pub fn derive_lang(file_paths: &[String]) -> Option<String> {
	for file in file_paths {
		let normalized = file.replace('\\', "/");
		let base = normalized.rsplit('/').next().unwrap_or(&normalized);
		if let Some(index) = base.rfind('.')
			&& index > 0
			&& index + 1 < base.len()
		{
			return Some(base[index + 1..].to_lowercase());
		}
	}
	None
}

/// Returns Ok(true) on a match, Ok(false) on none, Err on matcher failure.
/// The second tuple field reports parse errors.
pub fn ast_match_detail(patterns: &[String], source: &str, lang: &str) -> Result<(bool, bool)> {
	let result = pi_ast::engine::ast_match(
		pi_ast::engine::AstMatchOptions {
			source:       source.to_owned(),
			lang:         lang.to_owned(),
			patterns:     patterns.to_vec(),
			selector:     None,
			strictness:   Some(pi_ast::engine::AstMatchStrictness::Smart),
			limit:        Some(1),
			offset:       None,
			include_meta: None,
		},
		harvest_runtime::cancel::CancelToken::new(None),
	)
	.map_err(|e| anyhow::anyhow!("{e}"))?;
	Ok((result.total_matches > 0, result.parse_errors.is_some_and(|e| !e.is_empty())))
}

pub fn ast_any_match(patterns: &[String], source: &str, lang: &str) -> Result<bool> {
	ast_match_detail(patterns, source, lang).map(|(hit, _)| hit)
}
