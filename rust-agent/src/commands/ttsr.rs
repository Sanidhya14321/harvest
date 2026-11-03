//! `harvest ttsr` — inspect and test Time-Traveling Stream Rules.
//! Ported from `commands/ttsr.ts` and `cli/ttsr-cli.ts`.

use std::{
	io::{IsTerminal, Read, Write},
	path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use serde_json::json;

use super::rules::{
	self, MatchContext, MatchSource, Rule, TtsrManager, TtsrSettings, compile_rule_condition,
	derive_lang,
};

const STDIN_MARKER: &str = "-";
const BINARY_PROBE_BYTES: usize = 8192;
const DEFAULT_MAX_SCAN_BYTES: u64 = 5 * 1024 * 1024;
const SOURCE_FILE_EXT: &str = r"(?i)^\.(ts|tsx|js|jsx|mjs|cjs|rs|py|go|java|kt|swift|c|cc|cpp|h|hpp|rb|php|lua|css|scss|html|json|ya?ml|toml|md|mdc|cs|razor|cshtml|fs|fsx|vb|sh|bash|sql|zig|dart|scala|ex|exs|proto|tf)$";

#[derive(Debug, clap::Args)]
#[command(after_help = "Examples:
  harvest ttsr list
  harvest ttsr test 'const x: any = 1'
  harvest ttsr test src/foo.ts
  harvest ttsr test --file src/foo.ts --source text
  harvest ttsr test --rule .harvest/rules/no-any.md --source tool --path src/foo.ts 'const x: any = 1'
  harvest ttsr test --agent scout 'const x: any = 1'
  echo 'Box::leak(&mut v)' | harvest ttsr test --file - --path src/lib.rs
  harvest ttsr scan
  harvest ttsr scan -r .harvest/rules/no-any.md src/")]
pub struct Args {
	/// TTSR action
	#[arg(value_parser = ["test", "list", "scan"])]
	action:       Option<String>,
	/// Inline snippet text to test (ttsr test) or directory to scan (ttsr scan)
	snippet:      Option<String>,
	/// Snippet file path, or - for stdin (ttsr test)
	#[arg(long)]
	file:         Option<String>,
	/// Rule markdown file to test in isolation (skips project rule loading)
	#[arg(short, long)]
	rule:         Option<String>,
	/// Match source: text, thinking, or tool (inferred from --file when omitted)
	#[arg(long, value_parser = ["text", "thinking", "tool"])]
	source:       Option<String>,
	/// Tool name when source is tool (e.g. edit, write); defaults to edit
	#[arg(long)]
	tool:         Option<String>,
	/// Candidate file path for scope/glob matching and AST language inference
	#[arg(short, long)]
	path:         Option<String>,
	/// Agent name to evaluate rule `agents` scoping as (ttsr test); defaults to main
	#[arg(long)]
	agent:        Option<String>,
	/// Show every evaluated rule, not just triggered ones
	#[arg(short, long)]
	verbose:      bool,
	/// Output JSON
	#[arg(long)]
	json:         bool,
	/// Include files excluded by .gitignore (ttsr scan)
	#[arg(long = "no-gitignore")]
	no_gitignore: bool,
	/// Maximum file size to scan in bytes; 0 disables the limit (ttsr scan)
	#[arg(long = "max-bytes")]
	max_bytes:    Option<i64>,
	/// Agent directory override
	#[arg(long, hide = true)]
	agent_dir:    Option<PathBuf>,
}

struct Paint(bool);
impl Paint {
	fn wrap(&self, code: &str, text: &str) -> String {
		if self.0 { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_owned() }
	}
	fn bold(&self, t: &str) -> String { self.wrap("1", t) }
	fn dim(&self, t: &str) -> String { self.wrap("2", t) }
	fn red(&self, t: &str) -> String { self.wrap("31", t) }
	fn green(&self, t: &str) -> String { self.wrap("32", t) }
	fn yellow(&self, t: &str) -> String { self.wrap("33", t) }
	fn magenta(&self, t: &str) -> String { self.wrap("35", t) }
	fn cyan(&self, t: &str) -> String { self.wrap("36", t) }
}

#[derive(serde::Serialize, Clone)]
struct Conditions {
	regex: Vec<String>,
	ast:   Vec<String>,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RuleMatchDetail {
	name:            String,
	path:            String,
	#[serde(skip_serializing_if = "Option::is_none")]
	source_provider: Option<String>,
	matched:         Conditions,
	defined:         Conditions,
	#[serde(skip_serializing_if = "Option::is_none")]
	skipped_ast:     Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	agents:          Option<Vec<String>>,
}

fn detail(rule: &Rule, regex: Vec<String>, ast: Vec<String>) -> RuleMatchDetail {
	RuleMatchDetail {
		name: rule.name.clone(),
		path: rule.path.clone(),
		source_provider: Some(rule.provider.clone()),
		matched: Conditions { regex, ast },
		defined: Conditions {
			regex: rule.condition.clone().unwrap_or_default(),
			ast:   rule.ast_condition.clone().unwrap_or_default(),
		},
		skipped_ast: None,
		agents: rule.agents.clone(),
	}
}

fn fail(json: bool, message: &str, paint: &Paint, red: bool) -> Result<u8> {
	if json {
		println!("{}", json!({ "error": message }));
	} else if red {
		eprintln!("{}", paint.red(message));
	} else {
		eprintln!("{}", paint.yellow(message));
	}
	Ok(1)
}

pub fn execute(args: Args) -> Result<u8> {
	let cwd = std::env::current_dir()?;
	let agent_dir = crate::config::agent_dir_from(args.agent_dir.clone(), |k| std::env::var_os(k));
	let paint = Paint(std::io::stdout().is_terminal());
	match args.action.as_deref().unwrap_or("list") {
		"test" => run_test(&args, &cwd, agent_dir.as_deref(), &paint),
		"list" => run_list(&args, &cwd, agent_dir.as_deref(), &paint),
		"scan" => run_scan(&args, &cwd, agent_dir.as_deref(), &paint),
		other => {
			eprintln!("{}", paint.red(&format!("error: unknown ttsr action: {other}")));
			Ok(1)
		},
	}
}

fn read_snippet(snippet: Option<&str>, file: Option<&str>) -> Result<String> {
	if let Some(file) = file {
		if file == STDIN_MARKER {
			let mut text = String::new();
			std::io::stdin().read_to_string(&mut text)?;
			return Ok(text);
		}
		let resolved = std::path::absolute(file)?;
		return std::fs::read_to_string(&resolved)
			.map_err(|_| anyhow::anyhow!("Snippet file not found: {}", resolved.display()));
	}
	if let Some(snippet) = snippet {
		return Ok(snippet.to_owned());
	}
	if !std::io::stdin().is_terminal() {
		let mut text = String::new();
		std::io::stdin().read_to_string(&mut text)?;
		return Ok(text);
	}
	bail!("No snippet provided. Pass inline text, --file <path>, or pipe via --file -.")
}

fn preview_snippet(text: &str) -> String {
	let single = text.split_whitespace().collect::<Vec<_>>().join(" ");
	if single.chars().count() > 80 {
		format!("{}…", single.chars().take(77).collect::<String>())
	} else {
		single
	}
}

fn extname(path: &str) -> String {
	let base = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or_default();
	match base.rfind('.') {
		Some(index) if index > 0 => base[index..].to_owned(),
		_ => String::new(),
	}
}

fn regex_matches(rule: &Rule, snippet: &str) -> Vec<String> {
	rule.condition
		.iter()
		.flatten()
		.filter(|p| compile_rule_condition(p).is_ok_and(|re| re.is_match(snippet).unwrap_or(false)))
		.cloned()
		.collect()
}

fn ast_matches(rule: &Rule, snippet: &str, lang: &str) -> Vec<String> {
	rule.ast_condition
		.iter()
		.flatten()
		.filter(|p| rules::ast_any_match(&[(*p).clone()], snippet, lang).unwrap_or(false))
		.cloned()
		.collect()
}

fn read_isolated_rule(rule_path: &str) -> Result<Rule> {
	let resolved = std::path::absolute(rule_path)?;
	let content = std::fs::read_to_string(&resolved)
		.map_err(|_| anyhow::anyhow!("Rule file not found: {}", resolved.display()))?;
	let base = resolved.file_name().and_then(|n| n.to_str()).unwrap_or_default();
	let name = base.strip_suffix(".mdc").or_else(|| base.strip_suffix(".md")).unwrap_or(base);
	Ok(rules::build_rule_from_markdown(name, &content, &resolved.to_string_lossy(), "ttsr-cli", Some(name)).0)
}

fn load_project_manager(cwd: &Path, agent_dir: Option<&Path>, agent: Option<&str>) -> TtsrManager {
	let settings = TtsrSettings::load(cwd, agent_dir);
	let mut manager = TtsrManager::new(settings.enabled);
	for rule in rules::load_rules(cwd, agent_dir) {
		if settings.disabled_rules.contains(&rule.name) {
			continue;
		}
		if !settings.builtin_rules && rule.provider == rules::BUILTIN_DEFAULTS_PROVIDER_ID {
			continue;
		}
		if !rules::rule_applies_to_agent(&rule, agent) {
			continue;
		}
		manager.add_rule(&rule);
	}
	manager
}

fn run_test(args: &Args, cwd: &Path, agent_dir: Option<&Path>, paint: &Paint) -> Result<u8> {
	let mut file = args.file.clone();
	let mut snippet = args.snippet.clone();
	if let Some(text) = &snippet
		&& file.is_none()
	{
		let resolved = std::path::absolute(text)?;
		if resolved.exists() {
			file = Some(resolved.to_string_lossy().into_owned());
			snippet = None;
		}
	}
	let snippet = read_snippet(snippet.as_deref(), file.as_deref())?;
	let file_path = args.path.clone().or_else(|| {
		file.as_deref()
			.filter(|f| *f != STDIN_MARKER)
			.and_then(|f| std::path::absolute(f).ok())
			.map(|p| p.to_string_lossy().into_owned())
	});
	let source_re = regex::Regex::new(SOURCE_FILE_EXT).expect("static regex");
	let source = match args.source.as_deref() {
		Some("text") => MatchSource::Text,
		Some("thinking") => MatchSource::Thinking,
		Some("tool") => MatchSource::Tool,
		Some(other) => bail!("Invalid --source: {other}. Expected one of: text, thinking, tool"),
		None if file_path.as_deref().is_some_and(|p| source_re.is_match(&extname(p))) => MatchSource::Tool,
		None => MatchSource::Text,
	};
	let tool = args.tool.clone().or_else(|| (source == MatchSource::Tool).then(|| "edit".to_owned()));
	let inference_note = match (&args.source, &file_path) {
		(None, Some(path)) if source == MatchSource::Text => {
			let ext = extname(path);
			Some(format!(
				"inferred --source text from '{}' (not in the source-file extension set); pass --source tool --tool edit to evaluate tool-scoped rules",
				if ext.is_empty() { path.as_str() } else { ext.as_str() }
			))
		},
		_ => None,
	};
	let context = MatchContext {
		source,
		tool_name: tool.clone(),
		file_paths: file_path.iter().cloned().collect(),
	};
	let agent = args
		.agent
		.as_deref()
		.map(|a| a.trim().to_lowercase())
		.filter(|a| !a.is_empty())
		.unwrap_or_else(|| rules::MAIN_AGENT_RULE_NAME.to_owned());
	let manager = if let Some(rule_path) = &args.rule {
		let rule = read_isolated_rule(rule_path)?;
		if !rules::rule_applies_to_agent(&rule, Some(&agent)) {
			let msg = format!(
				"Rule \"{}\" is scoped to agents [{}] and does not apply to agent \"{agent}\". Re-run with --agent <one of those names>.",
				rule.name,
				rule.agents.clone().unwrap_or_default().join(", ")
			);
			return fail(args.json, &msg, paint, false);
		}
		let mut manager = TtsrManager::new(true);
		if !manager.add_rule(&rule) {
			bail!(
				"Rule \"{}\" has no usable TTSR condition. Add a `condition` (regex) or `astCondition` (ast-grep pattern) to its frontmatter.",
				rule.name
			);
		}
		manager
	} else {
		load_project_manager(cwd, agent_dir, Some(&agent))
	};
	let rules = manager.rules();
	if rules.is_empty() {
		let msg = if args.rule.is_some() {
			"Rule registered but produced no TTSR entry.".to_owned()
		} else {
			format!(
				"No TTSR rules registered for this project as agent \"{agent}\". Rules scoped to other agents via `agents` are excluded — run `omp ttsr list` to see every rule, or pass --agent <name>."
			)
		};
		return fail(args.json, &msg, paint, false);
	}
	let mut hits = manager.check_snapshot(&snippet, &context);
	if source == MatchSource::Tool && !context.file_paths.is_empty() {
		hits.extend(manager.check_ast_snapshot(&snippet, &context));
	}
	let lang = derive_lang(&context.file_paths);
	let ast_eligible = source == MatchSource::Tool && lang.is_some();
	let mut triggered = Vec::new();
	let mut not_triggered = Vec::new();
	for rule in &rules {
		let regex = regex_matches(rule, &snippet);
		let ast = match (&lang, ast_eligible) {
			(Some(lang), true) => ast_matches(rule, &snippet, lang),
			_ => Vec::new(),
		};
		let mut d = detail(rule, regex, ast);
		if !ast_eligible && rule.ast_condition.as_ref().is_some_and(|a| !a.is_empty()) {
			d.skipped_ast =
				Some("astCondition requires --source tool and a --path with a file extension".to_owned());
		}
		if hits.contains(&rule.name) { triggered.push(d) } else { not_triggered.push(d) }
	}
	if args.json {
		println!(
			"{}",
			json!({
				"source": source,
				"tool": tool,
				"filePath": file_path,
				"snippetPreview": preview_snippet(&snippet),
				"snippetBytes": snippet.encode_utf16().count(),
				"evaluated": rules.len(),
				"triggered": triggered,
				"notTriggered": not_triggered,
				"inferenceNote": inference_note,
				"agent": agent,
			})
		);
		return Ok(0);
	}
	let ctx_label = if source == MatchSource::Tool {
		format!("tool:{}", tool.as_deref().unwrap_or("?"))
	} else if source == MatchSource::Text {
		"text".to_owned()
	} else {
		"thinking".to_owned()
	};
	let path_label = file_path.as_ref().map(|p| format!(" path={p}")).unwrap_or_default();
	let mut out = std::io::stdout().lock();
	writeln!(
		out,
		"{} — source={}{path_label} agent={} snippet={}",
		paint.bold("TTSR test"),
		paint.cyan(&ctx_label),
		paint.cyan(&agent),
		paint.dim(&format!("{}b", snippet.encode_utf16().count()))
	)?;
	writeln!(out, "{}\n", paint.dim(&format!("  \"{}\"", preview_snippet(&snippet))))?;
	if let Some(note) = &inference_note {
		writeln!(out, "{}\n", paint.yellow(&format!("note: {note}")))?;
	}
	if triggered.is_empty() {
		writeln!(out, "{} (evaluated {})", paint.red("No rules triggered."), rules.len())?;
	} else {
		writeln!(out, "{}", paint.green(&paint.bold(&format!("Triggered ({})", triggered.len()))))?;
		for d in &triggered {
			render_rule_detail(&mut out, d, true, paint)?;
		}
	}
	if args.verbose && !not_triggered.is_empty() {
		writeln!(out, "\n{}", paint.dim(&format!("Not triggered ({})", not_triggered.len())))?;
		for d in &not_triggered {
			render_rule_detail(&mut out, d, false, paint)?;
		}
	}
	Ok(u8::from(args.rule.is_some() && triggered.is_empty()))
}

fn render_rule_detail(out: &mut impl Write, d: &RuleMatchDetail, hit: bool, paint: &Paint) -> Result<()> {
	let mark = if hit { paint.green("✓") } else { paint.red("✗") };
	let (regex, ast) = if hit { (&d.matched.regex, &d.matched.ast) } else { (&d.defined.regex, &d.defined.ast) };
	let mut parts = Vec::new();
	if !regex.is_empty() {
		parts.push(format!(
			"condition: {}",
			regex.iter().map(|c| paint.yellow(&format!("/{c}/"))).collect::<Vec<_>>().join(", ")
		));
	}
	if !ast.is_empty() {
		parts.push(format!(
			"astCondition: {}",
			ast.iter().map(|c| paint.magenta(c)).collect::<Vec<_>>().join(", ")
		));
	}
	if let Some(agents) = d.agents.as_ref().filter(|a| !a.is_empty()) {
		parts.push(format!("agents: {}", agents.join(", ")));
	}
	if let Some(skipped) = &d.skipped_ast {
		parts.push(paint.dim(&format!("astCondition: {skipped}")));
	}
	let label = if parts.is_empty() { paint.dim("no active conditions") } else { parts.join("  ") };
	let provider = d.source_provider.as_ref().map(|p| paint.dim(&format!(" [{p}]"))).unwrap_or_default();
	writeln!(out, "  {mark} {}  {label}{provider}", paint.bold(&d.name))?;
	Ok(())
}

fn run_list(args: &Args, cwd: &Path, agent_dir: Option<&Path>, paint: &Paint) -> Result<u8> {
	let rules = load_project_manager(cwd, agent_dir, None).rules();
	if args.json {
		let items: Vec<_> = rules
			.iter()
			.map(|r| {
				json!({
					"name": r.name, "path": r.path, "provider": r.provider,
					"condition": r.condition.clone().unwrap_or_default(),
					"astCondition": r.ast_condition.clone().unwrap_or_default(),
					"scope": r.scope.clone().unwrap_or_default(),
					"globs": r.globs.clone().unwrap_or_default(),
					"agents": r.agents.clone().unwrap_or_default(),
					"description": r.description,
				})
			})
			.collect();
		println!("{}", serde_json::Value::Array(items));
		return Ok(0);
	}
	if rules.is_empty() {
		println!("{}", paint.yellow("No TTSR rules registered for this project."));
		return Ok(0);
	}
	println!("{}", paint.bold(&format!("TTSR rules ({})", rules.len())));
	for rule in &rules {
		let mut parts = Vec::new();
		for (label, value) in [
			("condition", &rule.condition),
			("astCondition", &rule.ast_condition),
			("scope", &rule.scope),
			("globs", &rule.globs),
			("agents", &rule.agents),
		] {
			if let Some(v) = value.as_ref().filter(|v| !v.is_empty()) {
				parts.push(format!("{label}: {}", v.join(", ")));
			}
		}
		let conds = if parts.is_empty() { "no conditions".to_owned() } else { parts.join("  ") };
		println!("  {}{} {}", paint.bold(&rule.name), paint.dim(&format!(" [{}]", rule.provider)), paint.dim(&conds));
		if let Some(description) = &rule.description {
			println!("{}", paint.dim(&format!("    {description}")));
		}
	}
	Ok(0)
}

struct ScanPlan {
	rule:           Rule,
	globs:          Option<Vec<globset::GlobMatcher>>,
	default_tool:   bool,
	scopes:         Vec<rules::ToolScope>,
	regexes:        Vec<(String, fancy_regex::Regex)>,
	ast:            Vec<String>,
	ast_prefilters: Vec<fancy_regex::Regex>,
	ast_full_scan:  bool,
}

fn compile_ast_prefilter(pattern: &str) -> Option<fancy_regex::Regex> {
	if regex::Regex::new(r"\bas\s*\{").expect("static").is_match(pattern) {
		return fancy_regex::Regex::new(r"\bas\b(?:\s|//[^\n]*(?:\n|$)|/\*[\s\S]*?\*/)*\{").ok();
	}
	let ignored = ["if", "as", "const", "let", "var", "return", "true", "false", "null", "undefined"];
	let upper = regex::Regex::new(r"^[A-Z_]+$").expect("static");
	let mut tokens: Vec<&str> = regex::Regex::new(r"\b[A-Za-z_][A-Za-z0-9_]*\b")
		.expect("static")
		.find_iter(pattern)
		.map(|m| m.as_str())
		.filter(|t| !ignored.contains(t) && !upper.is_match(t))
		.collect();
	tokens.sort_by(|a, b| b.len().cmp(&a.len()));
	tokens.first().and_then(|t| fancy_regex::Regex::new(&format!(r"\b{}\b", regex::escape(t))).ok())
}

fn compile_scan_plans(rules: Vec<Rule>) -> Vec<ScanPlan> {
	rules
		.into_iter()
		.map(|rule| {
			let mut scopes = Vec::new();
			let mut default_tool = rule.scope.as_ref().is_none_or(Vec::is_empty);
			for raw in rule.scope.iter().flatten() {
				let token = raw.trim();
				let lower = token.to_lowercase();
				if token.is_empty() || lower == "text" || lower == "thinking" {
					continue;
				}
				if lower == "tool" || lower == "toolcall" {
					scopes.push(rules::ToolScope { tool_name: None, path_glob: None });
					continue;
				}
				let Some(scope) = rules::parse_tool_scope_token(token) else { continue };
				if scope.tool_name.is_none() && scope.path_glob.is_none() {
					default_tool = true;
					continue;
				}
				scopes.push(scope);
			}
			let regexes = rule
				.condition
				.iter()
				.flatten()
				.filter_map(|p| compile_rule_condition(p).ok().map(|r| (p.clone(), r)))
				.collect();
			let ast: Vec<String> = rule
				.ast_condition
				.iter()
				.flatten()
				.map(|p| p.trim().to_owned())
				.filter(|p| !p.is_empty())
				.collect();
			let mut ast_prefilters = Vec::new();
			let mut ast_full_scan = false;
			for pattern in &ast {
				match compile_ast_prefilter(pattern) {
					Some(prefilter) => ast_prefilters.push(prefilter),
					None => ast_full_scan = true,
				}
			}
			ScanPlan {
				globs: rules::compile_path_globs(rule.globs.as_ref()),
				rule,
				default_tool,
				scopes,
				regexes,
				ast,
				ast_prefilters,
				ast_full_scan,
			}
		})
		.collect()
}

fn plan_matches_tool_scope(plan: &ScanPlan, paths: &[String]) -> bool {
	if !plan.globs.as_ref().is_none_or(|g| g.iter().any(|g| rules::matches_glob(g, paths))) {
		return false;
	}
	if plan.default_tool {
		return true;
	}
	plan.scopes.iter().any(|scope| {
		scope.path_glob.as_ref().is_none_or(|g| rules::matches_glob(g, paths))
			&& scope.tool_name.as_deref().is_none_or(|t| t == "edit" || t == "write")
	})
}

fn plan_matches_content(plan: &ScanPlan, content: &str, lang: Option<&str>, details: bool) -> Option<RuleMatchDetail> {
	let mut regex_hit = false;
	let mut matched_regex = Vec::new();
	for (pattern, regex) in &plan.regexes {
		if regex.is_match(content).unwrap_or(false) {
			regex_hit = true;
			if details {
				matched_regex.push(pattern.clone());
			}
		}
	}
	let mut matched_ast = Vec::new();
	let mut ast_hit = false;
	if (details || !regex_hit)
		&& let Some(lang) = lang
		&& !plan.ast.is_empty()
	{
		if details {
			matched_ast = ast_matches(&plan.rule, content, lang);
			ast_hit = !matched_ast.is_empty();
		} else {
			ast_hit = rules::ast_any_match(&plan.ast, content, lang).unwrap_or(false);
		}
	}
	if !regex_hit && !ast_hit {
		return None;
	}
	Some(detail(&plan.rule, matched_regex, matched_ast))
}

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Skipped {
	binary:           usize,
	large:            usize,
	unreadable:       usize,
	no_relevant_rules: usize,
}
impl Skipped {
	const fn total(&self) -> usize {
		self.binary + self.large + self.unreadable + self.no_relevant_rules
	}
}

fn discover_scan_files(scan_dir: &Path, gitignore: bool) -> Vec<(String, Option<f64>)> {
	let options = pi_walker::WalkOptions {
		include_hidden: true,
		use_gitignore: gitignore,
		skip_git: true,
		detail: pi_walker::WalkDetail::Full,
		..Default::default()
	};
	let Ok(entries) = pi_walker::WalkRequest::from_options(scan_dir, options).collect_files() else {
		return Vec::new();
	};
	let mut files: Vec<_> = entries
		.into_iter()
		.filter(|e| !e.path.is_empty() && e.path != ".git" && !e.path.starts_with(".git/"))
		.map(|e| (e.path, e.size))
		.collect();
	files.sort_by(|a, b| a.0.cmp(&b.0));
	files
}

fn rel_path(base: &Path, target: &Path) -> String {
	let mut base_parts: Vec<_> = base.components().collect();
	let target_parts: Vec<_> = target.components().collect();
	let common = base_parts.iter().zip(&target_parts).take_while(|(a, b)| a == b).count();
	base_parts.drain(..common);
	let mut parts: Vec<String> = base_parts.iter().map(|_| "..".to_owned()).collect();
	parts.extend(target_parts[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
	parts.join("/")
}

fn run_scan(args: &Args, cwd: &Path, agent_dir: Option<&Path>, paint: &Paint) -> Result<u8> {
	let scan_dir = args.snippet.as_ref().map_or_else(|| cwd.to_path_buf(), |d| cwd.join(d));
	let scan_dir = std::path::absolute(&scan_dir)?;
	if !scan_dir.exists() {
		if args.json {
			println!("{}", json!({ "error": format!("Directory not found: {}", scan_dir.display()) }));
		} else {
			eprintln!("{}", paint.red(&format!("error: scan directory not found: {}", scan_dir.display())));
		}
		return Ok(1);
	}
	let has_condition = |r: &Rule| {
		r.condition.as_ref().is_some_and(|c| !c.is_empty()) || r.ast_condition.as_ref().is_some_and(|c| !c.is_empty())
	};
	let rules: Vec<Rule> = if let Some(rule) = &args.rule {
		vec![read_isolated_rule(rule)?].into_iter().filter(has_condition).collect()
	} else {
		let settings = TtsrSettings::load(cwd, agent_dir);
		if settings.enabled {
			rules::load_rules(cwd, agent_dir)
				.into_iter()
				.filter(|r| !settings.disabled_rules.contains(&r.name))
				.filter(|r| settings.builtin_rules || r.provider != rules::BUILTIN_DEFAULTS_PROVIDER_ID)
				.filter(has_condition)
				.collect()
		} else {
			Vec::new()
		}
	};
	if rules.is_empty() {
		let msg = if args.rule.is_some() {
			"Rule registered but produced no TTSR entry."
		} else {
			"No TTSR rules registered for this project."
		};
		return fail(args.json, msg, paint, false);
	}
	let rule_count = rules.len();
	let plans: Vec<ScanPlan> = compile_scan_plans(rules)
		.into_iter()
		.filter(|p| !p.regexes.is_empty() || !p.ast.is_empty())
		.collect();
	if plans.is_empty() {
		let msg = if args.rule.is_some() {
			"Rule registered but produced no usable TTSR condition."
		} else {
			"No usable TTSR rules registered for this project."
		};
		return fail(args.json, msg, paint, false);
	}
	let gitignore = !args.no_gitignore;
	let max_bytes = args.max_bytes.map_or(DEFAULT_MAX_SCAN_BYTES, |b| u64::try_from(b.max(0)).unwrap_or(0));
	let details = args.json || args.verbose;
	let files = discover_scan_files(&scan_dir, gitignore);
	if files.is_empty() {
		if args.json {
			println!(
				"{}",
				json!({"files": [], "summary": {"totalFiles": 0, "scannedFiles": 0, "matchedFiles": 0, "totalMatches": 0,
					"evaluatedRules": plans.len(), "skippedFiles": 0, "skipped": Skipped::default(), "gitignore": gitignore, "maxBytes": max_bytes}})
			);
		} else {
			println!("{}", paint.yellow(&format!("No files found to scan in {}", scan_dir.display())));
		}
		return Ok(0);
	}
	let mut results: Vec<(String, Vec<RuleMatchDetail>)> = Vec::new();
	let mut skipped = Skipped::default();
	let (mut scanned, mut matched_files, mut total_matches) = (0usize, 0usize, 0usize);
	for (file, size) in &files {
		let abs = scan_dir.join(file);
		let rel = rel_path(cwd, &abs);
		let base = abs.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
		let paths = vec![abs.to_string_lossy().replace('\\', "/"), rel.clone(), base];
		let relevant: Vec<&ScanPlan> = plans.iter().filter(|p| plan_matches_tool_scope(p, &paths)).collect();
		if relevant.is_empty() {
			skipped.no_relevant_rules += 1;
			continue;
		}
		#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
		let file_size = size.map_or_else(|| std::fs::metadata(&abs).map_or(0, |m| m.len()), |s| s as u64);
		if max_bytes > 0 && file_size > max_bytes {
			skipped.large += 1;
			continue;
		}
		let Ok(bytes) = std::fs::read(&abs) else {
			skipped.unreadable += 1;
			continue;
		};
		if bytes[..bytes.len().min(BINARY_PROBE_BYTES)].contains(&0) {
			skipped.binary += 1;
			continue;
		}
		let content = String::from_utf8_lossy(&bytes).into_owned();
		let lang = derive_lang(&paths);
		scanned += 1;
		let mut found = Vec::new();
		let mut count = 0usize;
		let mut pending_ast = Vec::new();
		for plan in &relevant {
			let hit = if details {
				plan_matches_content(plan, &content, lang.as_deref(), true)
			} else {
				plan_matches_content(plan, &content, None, false)
			};
			if let Some(d) = hit {
				count += 1;
				if details {
					found.push(d);
				}
				continue;
			}
			if !details
				&& !plan.ast.is_empty()
				&& (plan.ast_full_scan || plan.ast_prefilters.iter().any(|p| p.is_match(&content).unwrap_or(false)))
			{
				pending_ast.push(*plan);
			}
		}
		if !details && !pending_ast.is_empty() {
			let any = lang.as_deref().map_or(Some(false), |lang| {
				let patterns: Vec<String> = pending_ast.iter().flat_map(|p| p.ast.clone()).collect();
				match rules::ast_match_detail(&patterns, &content, lang) {
					Ok((true, _)) => Some(true),
					Ok((false, true)) | Err(_) => None,
					Ok((false, false)) => Some(false),
				}
			});
			if any != Some(false) {
				for plan in pending_ast {
					if plan_matches_content(plan, &content, lang.as_deref(), false).is_some() {
						count += 1;
					}
				}
			}
		}
		if count > 0 {
			matched_files += 1;
			total_matches += count;
			if details {
				results.push((rel, found));
			}
		}
	}
	if args.json {
		let files_json: Vec<_> = results
			.iter()
			.map(|(file, matches)| {
				json!({"filePath": file, "matches": matches.iter().map(|m| json!({"name": m.name, "path": m.path, "matched": m.matched})).collect::<Vec<_>>()})
			})
			.collect();
		println!(
			"{}",
			json!({"files": files_json, "summary": {"totalFiles": files.len(), "scannedFiles": scanned, "matchedFiles": matched_files,
				"totalMatches": total_matches, "evaluatedRules": plans.len(), "skippedFiles": skipped.total(), "skipped": skipped,
				"gitignore": gitignore, "maxBytes": max_bytes}})
		);
		return Ok(0);
	}
	println!(
		"{} — directory={} files={} scanned={} rules={} gitignore={} max-bytes={}",
		paint.bold("TTSR scan"),
		paint.cyan(&scan_dir.display().to_string()),
		paint.dim(&files.len().to_string()),
		paint.dim(&scanned.to_string()),
		paint.dim(&plans.len().to_string()),
		paint.dim(if gitignore { "on" } else { "off" }),
		paint.dim(&if max_bytes == 0 { "off".to_owned() } else { max_bytes.to_string() }),
	);
	if skipped.total() > 0 {
		println!(
			"{}",
			paint.dim(&format!(
				"  skipped: binary={} large={} unreadable={} no-relevant-rules={}",
				skipped.binary, skipped.large, skipped.unreadable, skipped.no_relevant_rules
			))
		);
	}
	if matched_files == 0 {
		println!(
			"{} (evaluated {rule_count} rules on {scanned}/{} files)",
			paint.green(&paint.bold("No rule matches found.")),
			files.len()
		);
		return Ok(0);
	}
	println!(
		"{} ({total_matches} matches across {matched_files} files)",
		paint.red(&paint.bold("Found violations/matches:"))
	);
	if !details {
		println!("{}", paint.dim("  rerun with --verbose to list matched files and conditions"));
		return Ok(0);
	}
	println!();
	let mut out = std::io::stdout().lock();
	for (file, matches) in &results {
		writeln!(out, "{}", paint.wrap("1;4", file))?;
		for d in matches {
			render_rule_detail(&mut out, d, true, paint)?;
		}
		writeln!(out)?;
	}
	Ok(0)
}
