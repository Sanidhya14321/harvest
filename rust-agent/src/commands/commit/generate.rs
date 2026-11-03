//! Conventional commit generation over the staged tree
//! (`conventional/service.ts`, `generate.ts`, `markdown.ts`,
//! `normalization.ts`, `diff.ts`, `scope.ts`).

use std::{collections::BTreeMap, path::Path};

use anyhow::{Result, anyhow};
use regex::Regex;
use serde_json::json;

use super::{GitError, git};
use crate::{chat, provider::Model};

const ANALYSIS_PROMPT: &str = include_str!("prompts/analysis.md");
const SUMMARY_PROMPT: &str = include_str!("prompts/summary.md");
const TYPES_DESCRIPTION: &str = include_str!("prompts/types-description.md");
const USER_SEPARATOR: &str = "<!-- USER -->";

const COMMIT_TYPES: &[&str] =
	&["feat", "fix", "refactor", "perf", "docs", "test", "build", "ci", "chore", "style", "revert"];
const FORBIDDEN_SCOPES: &[&str] = &[
	"src", "lib", "include", "tests", "benches", "examples", "docs", "app", "main", "entire", "all",
	"misc",
];
const EXCLUDED_FILES: &[&str] = &[
	"Cargo.lock",
	"package-lock.json",
	"npm-shrinkwrap.json",
	"yarn.lock",
	"pnpm-lock.yaml",
	"shrinkwrap.yaml",
	"bun.lock",
	"bun.lockb",
	"deno.lock",
	"composer.lock",
	"Gemfile.lock",
	"poetry.lock",
	"Pipfile.lock",
	"pdm.lock",
	"uv.lock",
	"go.sum",
	"flake.lock",
	"pubspec.lock",
	"Podfile.lock",
	"Packages.resolved",
	"mix.lock",
	"packages.lock.json",
	"gradle.lockfile",
];

/// `DEFAULT_CONVENTIONAL_GENERATION_CONFIG` (subset used natively).
struct Config {
	summary_guideline: usize,
	summary_soft_limit: usize,
	summary_hard_limit: usize,
	max_diff_length: usize,
	max_detail_tokens: usize,
}

const CONFIG: Config = Config {
	summary_guideline:  72,
	summary_soft_limit: 96,
	summary_hard_limit: 128,
	max_diff_length:    100_000,
	max_detail_tokens:  200,
};

#[derive(Debug, Clone)]
pub(crate) struct ConventionalCommit {
	pub kind:    String,
	pub scope:   Option<String>,
	pub summary: String,
	pub body:    Vec<String>,
	pub footers: Vec<String>,
}

impl ConventionalCommit {
	/// `formatConventionalCommit`.
	pub(crate) fn format(&self) -> String {
		let scope = self.scope.as_ref().map(|s| format!("({s})")).unwrap_or_default();
		let mut result = format!("{}{scope}: {}", self.kind, self.summary);
		if !self.body.is_empty() {
			let lines: Vec<String> = self.body.iter().map(|item| format!("- {item}")).collect();
			result.push_str(&format!("\n\n{}", lines.join("\n")));
		}
		if !self.footers.is_empty() {
			result.push_str(&format!("\n\n{}", self.footers.join("\n")));
		}
		result
	}
}

pub(crate) struct Generated {
	pub commit:           ConventionalCommit,
	pub validation_error: Option<String>,
}

pub(crate) enum Failure {
	Git(GitError),
	NoChanges,
	Other(anyhow::Error),
}

impl From<GitError> for Failure {
	fn from(error: GitError) -> Self {
		Self::Git(error)
	}
}

impl From<anyhow::Error> for Failure {
	fn from(error: anyhow::Error) -> Self {
		Self::Other(error)
	}
}

type ModelFactory = Box<dyn FnOnce() -> Result<Box<dyn Model>>>;

/// Lazily constructed model so whitespace-only commits never need credentials.
struct LazyModel {
	factory: Option<ModelFactory>,
	model:   Option<Box<dyn Model>>,
}

impl LazyModel {
	fn complete(&mut self, system: &str, user: String, progress: &str) -> Result<String> {
		if self.model.is_none() {
			let factory = self.factory.take().ok_or_else(|| anyhow!("model unavailable"))?;
			self.model = Some(factory()?);
		}
		println!("{progress}");
		let model = self.model.as_ref().expect("model initialized");
		let mut messages = Vec::new();
		if !system.is_empty() {
			messages.push(chat::system(system));
		}
		messages.push(chat::user(user));
		let reply = model.complete(&messages, &[])?;
		Ok(reply.content.unwrap_or_default())
	}
}

/// `generateGitCommit`: stage everything only when the index is empty.
pub(crate) fn generate_git_commit(
	root: &Path,
	settings: &crate::settings::Settings,
	user_context: Option<&str>,
	factory: ModelFactory,
) -> std::result::Result<Generated, Failure> {
	let _ = settings;
	let mut staged = git(root, &["diff", "--cached", "--name-only"])?;
	if staged.trim().is_empty() {
		println!("Staging all changes…");
		git(root, &["add", "-A"])?;
		staged = git(root, &["diff", "--cached", "--name-only"])?;
	}
	if staged.trim().is_empty() {
		return Err(Failure::NoChanges);
	}
	println!("Reading staged changes…");
	let initial = git(root, &["diff", "--cached"])?;
	let diff = if initial.len() <= CONFIG.max_diff_length {
		initial
	} else {
		git(root, &["diff", "--cached", "--unified=1"])?
	};
	if diff.trim().is_empty() {
		return Err(Failure::NoChanges);
	}
	let numstat = git(root, &["diff", "--cached", "--numstat"])?;
	let entries = parse_numstat(&numstat);
	let stat = render_stat(&entries);
	let context = collect_context(root);
	let mut model = LazyModel { factory: Some(factory), model: None };

	let whitespace_only = whitespace_only_files(&diff);
	let files = file_sections(&diff);
	let commit = if !files.is_empty() && whitespace_only.len() == files.len() {
		let summary = if whitespace_only.len() == 1 {
			let name = whitespace_only[0].rsplit('/').next().unwrap_or_default();
			format!("reformatted {name}")
		} else {
			format!("reformatted {} files", whitespace_only.len())
		};
		post_process(ConventionalCommit {
			kind: "style".into(),
			scope: None,
			summary,
			body: Vec::new(),
			footers: Vec::new(),
		})?
	} else {
		let prompt_diff: String = files
			.iter()
			.filter(|(name, _)| !whitespace_only.contains(name) && !is_excluded(name))
			.map(|(_, content)| content.as_str())
			.collect();
		let prompt_diff = truncate_chars(&prompt_diff, CONFIG.max_diff_length);
		let analysis = direct_analysis(
			&mut model,
			&stat,
			&prompt_diff,
			&scope_candidates(&entries),
			&context,
			user_context,
		)?;
		message_from_analysis(analysis, &stat, user_context, &mut model)?
	};
	Ok(validate_and_process(commit, &stat, &context.project_names))
}

struct Context {
	recent_commits: String,
	common_scopes:  String,
	project_names:  Vec<String>,
}

fn collect_context(root: &Path) -> Context {
	let subjects: Vec<String> = git(root, &["log", "-100", "--format=%s"])
		.map(|s| s.lines().map(str::to_owned).collect())
		.unwrap_or_default();
	let mut counts: BTreeMap<String, usize> = BTreeMap::new();
	for subject in &subjects {
		let Some(colon) = subject.find(':') else { continue };
		let prefix = &subject[..colon];
		let (Some(start), Some(end)) = (prefix.find('('), prefix.rfind(')')) else { continue };
		if start < end {
			*counts.entry(prefix[start + 1..end].to_owned()).or_default() += 1;
		}
	}
	let mut sorted: Vec<_> = counts.into_iter().collect();
	sorted.sort_by(|a, b| b.1.cmp(&a.1));
	let common = sorted
		.into_iter()
		.take(10)
		.map(|(scope, count)| format!("{scope} ({count})"))
		.collect::<Vec<_>>()
		.join(", ");
	let mut names = vec![
		root.file_name()
			.map(|n| n.to_string_lossy().into_owned())
			.unwrap_or_default(),
	];
	if let Ok(entries) = std::fs::read_dir(root) {
		let packages: Vec<String> = entries
			.flatten()
			.filter(|e| e.path().join("__init__.py").is_file())
			.map(|e| e.file_name().to_string_lossy().into_owned())
			.collect();
		if packages.len() == 1 && !names.contains(&packages[0]) {
			names.push(packages[0].clone());
		}
	}
	Context {
		recent_commits: subjects.iter().take(10).cloned().collect::<Vec<_>>().join("\n"),
		common_scopes:  common,
		project_names:  names,
	}
}

struct NumstatEntry {
	path:    String,
	added:   Option<u64>,
	removed: Option<u64>,
}

fn parse_numstat(output: &str) -> Vec<NumstatEntry> {
	output
		.lines()
		.filter_map(|line| {
			let mut parts = line.splitn(3, '\t');
			let added = parts.next()?.parse().ok();
			let removed = parts.next()?.parse().ok();
			let path = extract_rename_path(parts.next()?);
			Some(NumstatEntry { path, added, removed })
		})
		.collect()
}

/// `extractPathFromRename` (`git/diff.ts`).
fn extract_rename_path(part: &str) -> String {
	if let Some(start) = part.find('{')
		&& let Some(arrow) = part[start..].find(" => ").map(|i| i + start)
		&& let Some(end) = part[arrow..].find('}').map(|i| i + arrow)
	{
		return format!("{}{}{}", &part[..start], part[arrow + 4..end].trim(), &part[end + 1..]);
	}
	part.split(" => ").last().unwrap_or(part).trim().to_owned()
}

/// `renderStat` from `service.ts`.
fn render_stat(entries: &[NumstatEntry]) -> String {
	if entries.is_empty() {
		return String::new();
	}
	let (mut insertions, mut deletions) = (0, 0);
	let mut lines: Vec<String> = entries
		.iter()
		.map(|e| {
			let (a, r) = (e.added.unwrap_or(0), e.removed.unwrap_or(0));
			insertions += a;
			deletions += r;
			format!(
				" {} | {} {}{}",
				e.path,
				a + r,
				"+".repeat(a.min(40) as usize),
				"-".repeat(r.min(40) as usize)
			)
		})
		.collect();
	let plural = |n: u64| if n == 1 { "" } else { "s" };
	lines.push(format!(
		" {} file{} changed, {insertions} insertion{}(+), {deletions} deletion{}(-)",
		entries.len(),
		if entries.len() == 1 { "" } else { "s" },
		plural(insertions),
		plural(deletions)
	));
	format!("{}\n", lines.join("\n"))
}

fn is_excluded(path: &str) -> bool {
	let lower = path.to_lowercase();
	EXCLUDED_FILES.iter().any(|n| lower.ends_with(&n.to_lowercase()))
		|| [".lock.yml", ".lock.yaml", "-lock.yml", "-lock.yaml"]
			.iter()
			.any(|s| lower.ends_with(s))
}

/// `extractScopeCandidates` (simplified): rank top-level components by
/// changed lines.
fn scope_candidates(entries: &[NumstatEntry]) -> String {
	let mut weights: BTreeMap<String, u64> = BTreeMap::new();
	let mut total = 0;
	for entry in entries.iter().filter(|e| !is_excluded(&e.path)) {
		let lines = entry.added.unwrap_or(0) + entry.removed.unwrap_or(0);
		total += lines;
		let parts: Vec<&str> = entry.path.split('/').collect();
		if parts.len() < 2 {
			continue;
		}
		let component = parts
			.iter()
			.take(parts.len() - 1)
			.find(|p| !FORBIDDEN_SCOPES.contains(p) && !["packages", "crates"].contains(p))
			.map(|p| (*p).to_owned());
		if let Some(component) = component {
			*weights.entry(component).or_default() += lines;
		}
	}
	let mut sorted: Vec<_> = weights.into_iter().collect();
	sorted.sort_by(|a, b| b.1.cmp(&a.1));
	if sorted.is_empty() || total == 0 {
		return "(none - multi-component change)".into();
	}
	sorted
		.into_iter()
		.take(5)
		.map(|(scope, lines)| format!("{scope} ({}%)", lines * 100 / total))
		.collect::<Vec<_>>()
		.join(", ")
}

/// Split a diff into `(filename, block)` sections, keeping block bytes.
fn file_sections(diff: &str) -> Vec<(String, String)> {
	let header = Regex::new(r"^diff --git a/(.+?) b/(.+)$").expect("valid regex");
	let mut sections: Vec<(String, String)> = Vec::new();
	for line in diff.split_inclusive('\n') {
		if line.starts_with("diff --git ") {
			let name = header
				.captures(line.trim_end())
				.and_then(|c| c.get(2))
				.map(|m| m.as_str().to_owned())
				.unwrap_or_default();
			sections.push((name, String::new()));
		}
		if let Some(last) = sections.last_mut() {
			last.1.push_str(line);
		}
	}
	sections
}

/// `classifyDiffWhitespace`: files whose +/- lines differ only in whitespace.
fn whitespace_only_files(diff: &str) -> Vec<String> {
	file_sections(diff)
		.into_iter()
		.filter(|(_, content)| {
			if content.lines().any(|l| l.starts_with("Binary files ")) {
				return false;
			}
			let (mut added, mut removed, mut changed) = (String::new(), String::new(), false);
			for line in content.lines() {
				if line.starts_with("+++") || line.starts_with("---") {
					continue;
				}
				if let Some(rest) = line.strip_prefix('+') {
					changed = true;
					added.extend(rest.chars().filter(|c| !c.is_whitespace()));
				} else if let Some(rest) = line.strip_prefix('-') {
					changed = true;
					removed.extend(rest.chars().filter(|c| !c.is_whitespace()));
				}
			}
			changed && added == removed
		})
		.map(|(name, _)| name)
		.collect()
}

fn truncate_chars(text: &str, max: usize) -> String {
	if text.chars().count() <= max {
		return text.to_owned();
	}
	let mut out: String = text.chars().take(max).collect();
	out.push_str("\n... (diff truncated)\n");
	out
}

fn render_prompt(template: &str, data: &serde_json::Value) -> Result<(String, String)> {
	let mut engine = handlebars::Handlebars::new();
	engine.register_escape_fn(handlebars::no_escape);
	let (system, user) = match template.find(USER_SEPARATOR) {
		Some(index) => (template[..index].trim(), &template[index + USER_SEPARATOR.len()..]),
		None => ("", template),
	};
	Ok((system.to_owned(), engine.render_template(user, data)?.trim().to_owned()))
}

struct Analysis {
	kind:       String,
	scope:      Option<String>,
	summary:    Option<String>,
	details:    Vec<String>,
	issue_refs: Vec<String>,
}

fn direct_analysis(
	model: &mut LazyModel,
	stat: &str,
	diff: &str,
	scope_candidates: &str,
	context: &Context,
	user_context: Option<&str>,
) -> Result<Analysis> {
	let (system, mut user) = render_prompt(
		ANALYSIS_PROMPT,
		&json!({
			"project_context": "",
			"types_description": TYPES_DESCRIPTION.trim(),
			"stat": stat,
			"scope_candidates": scope_candidates,
			"common_scopes": context.common_scopes,
			"recent_commits": context.recent_commits,
			"diff": diff,
		}),
	)?;
	if let Some(extra) = user_context {
		user.push_str(&format!("\n\n<user_context>\n{extra}\n</user_context>"));
	}
	let text = model.complete(&system, user, "Analyzing staged changes…")?;
	parse_analysis(&text)
}

/// `parseConventionalAnalysisMarkdown`.
fn parse_analysis(text: &str) -> Result<Analysis> {
	let heading_re = Regex::new(
		r"(?i)^\s*(?:#+\s*)?([a-z][a-z0-9-]*)(?:\(([^)]+)\))?!?\s*:\s*(.+?)\s*$",
	)
	.expect("valid regex");
	let issue_re = Regex::new(r"#\d+(?:\s*-\s*#?\d+)?").expect("valid regex");
	let cleaned = text.trim().trim_start_matches("```markdown").trim_matches('`');
	let lines: Vec<&str> = cleaned.lines().collect();
	let mut heading = None;
	for (index, line) in lines.iter().take(5).enumerate() {
		if let Some(c) = heading_re.captures(line) {
			let kind = c[1].to_lowercase();
			let kind = if COMMIT_TYPES.contains(&kind.as_str()) { kind } else { "chore".into() };
			heading = Some((
				index,
				kind,
				c.get(2).map(|m| m.as_str().trim().to_owned()).filter(|s| !s.is_empty()),
				c[3].to_owned(),
			));
			break;
		}
	}
	let (index, kind, scope, summary) =
		heading.ok_or_else(|| anyhow!("Markdown analysis type(scope): summary heading not found"))?;
	let mut details = Vec::new();
	let mut issue_refs = Vec::new();
	for line in &lines[index + 1..] {
		let stripped = line.trim();
		if stripped.is_empty() {
			continue;
		}
		let lower = stripped.to_lowercase();
		if ["fixes:", "closes:", "resolves:"].iter().any(|p| lower.starts_with(p)) {
			let rest = &stripped[stripped.find(':').unwrap_or(0) + 1..];
			issue_refs.extend(rest.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from));
			continue;
		}
		let bullet = stripped.trim_start_matches(['-', '*', '•', '+']).trim();
		let bullet = bullet
			.split_once(". ")
			.filter(|(n, _)| n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty())
			.map_or(bullet, |(_, rest)| rest);
		if bullet.is_empty() || bullet.starts_with('#') {
			continue;
		}
		let sentence = if bullet.ends_with('.') { bullet.to_owned() } else { format!("{bullet}.") };
		issue_refs.extend(issue_re.find_iter(bullet).map(|m| m.as_str().to_owned()));
		if !details.contains(&sentence) {
			details.push(sentence);
		}
	}
	issue_refs.dedup();
	Ok(Analysis { kind, scope, summary: Some(summary), details, issue_refs })
}

fn message_from_analysis(
	analysis: Analysis,
	stat: &str,
	user_context: Option<&str>,
	model: &mut LazyModel,
) -> Result<ConventionalCommit> {
	let summary = match analysis.summary.clone().map(|s| strip_prefix(&s, &analysis.kind)) {
		Some(s) if !s.is_empty() => s,
		_ => summary_from_analysis(&analysis, stat, user_context, model)
			.unwrap_or_else(|_| fallback_summary(stat, &analysis.details)),
	};
	let footers = if analysis.issue_refs.is_empty() {
		Vec::new()
	} else {
		vec![format!("Fixes: {}", analysis.issue_refs.join(", "))]
	};
	Ok(ConventionalCommit {
		kind: analysis.kind,
		scope: analysis.scope,
		summary,
		body: analysis.details,
		footers,
	})
}

fn summary_from_analysis(
	analysis: &Analysis,
	stat: &str,
	user_context: Option<&str>,
	model: &mut LazyModel,
) -> Result<String> {
	let prefix = analysis.kind.len()
		+ analysis.scope.as_ref().map_or(0, |s| s.len() + 2)
		+ 2;
	let chars = CONFIG.summary_guideline.saturating_sub(prefix).max(10);
	let details = analysis
		.details
		.iter()
		.map(|d| format!("- {d}"))
		.collect::<Vec<_>>()
		.join("\n");
	let (system, user) = render_prompt(
		SUMMARY_PROMPT,
		&json!({
			"commit_type": analysis.kind,
			"scope": analysis.scope,
			"chars": chars,
			"user_context": user_context.unwrap_or(""),
			"details": details,
			"stat": stat,
		}),
	)?;
	let text = model.complete(&system, user, "Writing summary…")?;
	let tagged = text
		.find("<summary>")
		.map(|start| {
			let rest = &text[start + 9..];
			rest.find("</summary>").map_or(rest, |end| &rest[..end])
		})
		.unwrap_or(&text);
	let summary = tagged.split_whitespace().collect::<Vec<_>>().join(" ");
	let summary = strip_prefix(summary.trim_matches(['"', '\'', '`']), &analysis.kind);
	if summary.is_empty() {
		return Err(anyhow!("Markdown summary empty after normalization"));
	}
	Ok(summary)
}

fn strip_prefix(text: &str, kind: &str) -> String {
	let trimmed = text.trim();
	let re = Regex::new(r"(?i)^([a-z][a-z0-9-]*)(?:\(([^)]*)\))?:\s+(.*)$").expect("valid regex");
	if let Some(c) = re.captures(trimmed)
		&& c[1].eq_ignore_ascii_case(kind)
	{
		return c[3].trim().to_owned();
	}
	trimmed.to_owned()
}

/// `fallbackSummary` (simplified).
fn fallback_summary(stat: &str, details: &[String]) -> String {
	let candidate = details
		.iter()
		.map(|d| d.trim_start_matches(['-', '*', '•', ' ']).trim())
		.find(|d| !d.is_empty())
		.map(str::to_owned)
		.unwrap_or_else(|| {
			stat.lines()
				.next()
				.and_then(|l| l.split('|').next())
				.map(|p| p.trim().rsplit('/').next().unwrap_or("files").to_owned())
				.filter(|p| !p.is_empty())
				.map_or_else(|| "updated files".to_owned(), |p| format!("updated {p}"))
		});
	let candidate = candidate.trim_end_matches(['.', ';', ':']).to_owned();
	truncate_words(&candidate, CONFIG.summary_guideline)
}

fn truncate_words(text: &str, limit: usize) -> String {
	if text.chars().count() <= limit {
		return text.to_owned();
	}
	let mut out = String::new();
	for word in text.split_whitespace() {
		if out.chars().count() + word.chars().count() + 1 > limit {
			break;
		}
		if !out.is_empty() {
			out.push(' ');
		}
		out.push_str(word);
	}
	if out.is_empty() { text.chars().take(limit).collect() } else { out }
}

/// `postProcessCommitMessage` (verb tense table not ported).
fn post_process(mut message: ConventionalCommit) -> Result<ConventionalCommit> {
	let summary = message.summary.split_whitespace().collect::<Vec<_>>().join(" ");
	let summary = summary.trim().trim_end_matches(['.', ';', ':']).trim().to_owned();
	let summary = lowercase_first_token(&summary);
	if summary.len() > CONFIG.summary_hard_limit {
		return Err(anyhow!("Summary exceeds {} bytes", CONFIG.summary_hard_limit));
	}
	message.summary = summary;
	let mut body = Vec::new();
	for raw in &message.body {
		let detail = raw.trim().trim_start_matches(['•', '-', '*', '+']).trim();
		let detail = detail.split_whitespace().collect::<Vec<_>>().join(" ");
		let detail = detail.trim_end_matches(['.', ';', ',']).trim();
		if detail.is_empty() {
			continue;
		}
		let mut chars = detail.chars();
		let first = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
		body.push(format!("{first}{}.", chars.as_str()));
	}
	let mut budget = CONFIG.max_detail_tokens;
	body.retain(|d| {
		let tokens = d.len().div_ceil(4);
		if tokens > budget {
			return false;
		}
		budget -= tokens;
		true
	});
	message.body = body;
	Ok(message)
}

fn lowercase_first_token(text: &str) -> String {
	let token = text.split_whitespace().next().unwrap_or_default();
	let letters: Vec<char> = token.chars().filter(|c| c.is_alphabetic()).collect();
	if !letters.is_empty() && letters.iter().all(|c| c.is_uppercase()) {
		return text.to_owned();
	}
	let mut chars = text.chars();
	match chars.next() {
		Some(first) => format!("{}{}", first.to_lowercase(), chars.as_str()),
		None => String::new(),
	}
}

/// `validateCommitMessage` (core checks).
fn validate(message: &ConventionalCommit, project_names: &[String]) -> Vec<(&'static str, String)> {
	let mut errors = Vec::new();
	if !COMMIT_TYPES.contains(&message.kind.as_str()) {
		errors.push(("type", format!("Invalid commit type: {}", message.kind)));
	}
	if message.summary.is_empty() {
		errors.push(("summary_empty", "Summary must not be empty".to_owned()));
	}
	if let Some(scope) = &message.scope {
		let lower = scope.to_lowercase();
		if project_names.iter().any(|n| n.to_lowercase() == lower) {
			errors.push(("project_name_scope", format!("Scope '{scope}' is the project name")));
		} else if FORBIDDEN_SCOPES.contains(&lower.as_str()) {
			errors.push(("scope", format!("Scope '{scope}' is not allowed")));
		}
	}
	let first_line = message.format().lines().next().unwrap_or_default().chars().count();
	if first_line > CONFIG.summary_hard_limit {
		errors.push(("length", format!("First line exceeds {} characters", CONFIG.summary_hard_limit)));
	}
	errors
}

/// `validateAndProcess`.
fn validate_and_process(
	message: ConventionalCommit,
	stat: &str,
	project_names: &[String],
) -> Generated {
	let details = message.body.clone();
	let mut current = message;
	let mut validation_error = None;
	for attempt in 0..3 {
		if let Ok(processed) = post_process(current.clone()) {
			current = processed;
		} else {
			current.summary = fallback_summary(stat, &details);
		}
		let first_len = current.format().lines().next().unwrap_or_default().chars().count();
		if attempt == 0 && first_len > CONFIG.summary_soft_limit {
			current.summary = fallback_summary(stat, &details);
			continue;
		}
		let mut errors = validate(&current, project_names);
		if errors.is_empty() {
			return Generated { commit: current, validation_error: None };
		}
		if current.scope.is_some() && errors.iter().any(|(code, _)| *code == "project_name_scope") {
			current.scope = None;
			errors = validate(&current, project_names);
			if errors.is_empty() {
				return Generated { commit: current, validation_error: None };
			}
		}
		validation_error =
			Some(errors.iter().map(|(_, message)| message.as_str()).collect::<Vec<_>>().join("; "));
		current.summary = fallback_summary(stat, &details);
	}
	Generated { commit: current, validation_error }
}
