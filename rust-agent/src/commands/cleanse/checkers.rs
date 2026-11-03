//! Checker discovery and execution. Ported from `cleanse/checkers.ts`.

use std::{
	collections::{BTreeSet, HashSet},
	io::Read,
	path::{Path, PathBuf},
	process::{Command, Stdio},
	time::Instant,
};

use regex::Regex;

use super::{CheckResult, Diagnostic, Report, Severity, SkippedCheck, diagnostic_key, parsers};

const IGNORED_DIRECTORIES: &[&str] =
	&[".git", ".hg", ".svn", ".terraform", ".venv", "build", "coverage", "dist", "node_modules", "target", "vendor"];
const MAX_FAILURE_OUTPUT: usize = 12_000;
const SHELLCHECK_BATCH_SIZE: usize = 200;
const MAX_CHECKER_OUTPUT: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Plan {
	pub id:         String,
	pub label:      String,
	pub language:   String,
	pub cwd:        PathBuf,
	pub executable: PathBuf,
	pub args:       Vec<String>,
	pub parser:     String,
	pub command:    String,
	pub mutates:    bool,
}

#[derive(Debug, Clone)]
pub struct Descriptor {
	pub id:       String,
	pub label:    String,
	pub language: String,
	pub cwd:      String,
	pub command:  String,
}

struct PlanRequest<'a> {
	id:         &'a str,
	label:      &'a str,
	language:   &'a str,
	root:       &'a str,
	binaries:   &'a [&'a str],
	args:       Vec<String>,
	parser:     &'a str,
	executable: Option<PathBuf>,
	mutates:    bool,
	optional:   bool,
}

impl<'a> PlanRequest<'a> {
	fn new(id: &'a str, label: &'a str, language: &'a str, root: &'a str, binaries: &'a [&'a str], args: &[&str], parser: &'a str) -> Self {
		Self {
			id,
			label,
			language,
			root,
			binaries,
			args: args.iter().map(|a| (*a).to_owned()).collect(),
			parser,
			executable: None,
			mutates: false,
			optional: false,
		}
	}
}

struct State {
	project_cwd:   PathBuf,
	files:         Vec<String>,
	file_set:      HashSet<String>,
	plans:         Vec<Plan>,
	skipped:       Vec<SkippedCheck>,
	include_tests: bool,
}

pub struct Suite {
	project_cwd:   PathBuf,
	plans:         Vec<Plan>,
	active:        Vec<Plan>,
	pub skipped:   Vec<SkippedCheck>,
	allowed_files: HashSet<String>,
}

pub trait RunEvents: Sync {
	fn checker_start(&self, _checker: &Descriptor) {}
	fn checker_end(&self, _check: &CheckResult, _duration_ms: u128) {}
}

impl Suite {
	fn descriptor(&self, plan: &Plan) -> Descriptor {
		Descriptor {
			id:       plan.id.clone(),
			label:    plan.label.clone(),
			language: plan.language.clone(),
			cwd:      parsers::relative_to(&self.project_cwd, &plan.cwd).unwrap_or_else(|| ".".into()),
			command:  plan.command.clone(),
		}
	}

	pub fn checkers(&self) -> Vec<Descriptor> {
		self.plans.iter().map(|p| self.descriptor(p)).collect()
	}

	pub fn selected(&self) -> Vec<Descriptor> {
		self.active.iter().map(|p| self.descriptor(p)).collect()
	}

	pub fn select(&mut self, ids: &[String]) {
		self.active = self.plans.iter().filter(|p| ids.contains(&p.id)).cloned().collect();
	}

	/// Run every selected checker: mutating ones sequentially first, then the
	/// read-only ones in parallel.
	pub fn run(&self, events: &dyn RunEvents) -> Report {
		let execute = |plan: &Plan| {
			events.checker_start(&self.descriptor(plan));
			let started = Instant::now();
			let check = run_checker(plan, &self.project_cwd, &self.allowed_files);
			events.checker_end(&check, started.elapsed().as_millis());
			check
		};
		let mut checks: Vec<CheckResult> = self.active.iter().filter(|p| p.mutates).map(&execute).collect();
		let parallel: Vec<CheckResult> = std::thread::scope(|scope| {
			let handles: Vec<_> = self
				.active
				.iter()
				.filter(|p| !p.mutates)
				.map(|plan| scope.spawn(|| execute(plan)))
				.collect();
			handles.into_iter().filter_map(|h| h.join().ok()).collect()
		});
		checks.extend(parallel);
		let mut seen = HashSet::new();
		let mut diagnostics: Vec<Diagnostic> = checks
			.iter()
			.flat_map(|c| c.diagnostics.clone())
			.filter(|d| seen.insert(diagnostic_key(d)))
			.collect();
		diagnostics.sort_by(|a, b| {
			a.file
				.as_deref()
				.unwrap_or("")
				.cmp(b.file.as_deref().unwrap_or(""))
				.then(a.line.unwrap_or(0).cmp(&b.line.unwrap_or(0)))
				.then(a.message.cmp(&b.message))
		});
		Report { checks, diagnostics, skipped: self.skipped.clone() }
	}
}

fn resolve_cwd(project_cwd: &Path) -> PathBuf {
	let absolute = std::path::absolute(project_cwd).unwrap_or_else(|_| project_cwd.to_path_buf());
	absolute.canonicalize().unwrap_or(absolute)
}

fn has_ignored_directory(file: &str) -> bool {
	file.split('/').any(|s| IGNORED_DIRECTORIES.contains(&s) || s.starts_with("bazel-"))
}

/// Project files that are tracked or untracked-but-not-ignored.
fn list_project_files(cwd: &Path) -> Vec<String> {
	let options = pi_walker::WalkOptions {
		include_hidden: true,
		use_gitignore: true,
		skip_git: true,
		skip_node_modules: true,
		..Default::default()
	};
	let entries = pi_walker::WalkRequest::from_options(cwd, options).collect_files().unwrap_or_default();
	let set: BTreeSet<String> = entries
		.into_iter()
		.map(|e| e.path.replace('\\', "/").trim_start_matches("./").to_owned())
		.filter(|f| !f.is_empty() && !has_ignored_directory(f))
		.collect();
	set.into_iter().collect()
}

fn dirname(file: &str) -> String {
	file.rfind('/').map_or_else(|| ".".to_owned(), |i| file[..i].to_owned())
}

fn basename(file: &str) -> &str {
	file.rsplit('/').next().unwrap_or(file)
}

fn depth(root: &str) -> usize {
	if root == "." { 0 } else { root.split('/').count() }
}

fn topmost_roots(roots: impl IntoIterator<Item = String>) -> Vec<String> {
	let mut sorted: Vec<String> = roots.into_iter().collect::<BTreeSet<_>>().into_iter().collect();
	sorted.sort_by(|a, b| depth(a).cmp(&depth(b)).then(a.cmp(b)));
	let mut selected: Vec<String> = Vec::new();
	for root in sorted {
		if selected
			.iter()
			.any(|p| &root == p || p == "." || root.starts_with(&format!("{p}/")))
		{
			continue;
		}
		selected.push(root);
	}
	selected
}

fn marker_roots(files: &[String], matches: impl Fn(&str, &str) -> bool) -> Vec<String> {
	topmost_roots(files.iter().filter(|f| matches(f, basename(f))).map(|f| dirname(f)))
}

fn roots_for(files: &[String], names: &[&str]) -> Vec<String> {
	marker_roots(files, |_, base| names.contains(&base))
}

fn roots_pattern(files: &[String], pattern: &str) -> Vec<String> {
	let re = Regex::new(pattern).expect("static");
	marker_roots(files, |file, _| re.is_match(file))
}

fn contains_ext(files: &[String], exts: &[&str]) -> bool {
	files.iter().any(|f| {
		let lower = f.to_lowercase();
		exts.iter().any(|e| lower.ends_with(e))
	})
}

/// `$which` with the project-local binary directories prepended.
fn resolve_binary(project_cwd: &Path, root: &str, names: &[&str]) -> Option<PathBuf> {
	let cwd = project_cwd.join(root);
	let mut dirs = Vec::new();
	let mut current = cwd.clone();
	let bin = if cfg!(windows) { "Scripts" } else { "bin" };
	loop {
		dirs.push(current.join("node_modules/.bin"));
		dirs.push(current.join(".venv").join(bin));
		dirs.push(current.join("venv").join(bin));
		dirs.push(current.join("vendor/bin"));
		if current == project_cwd {
			break;
		}
		match current.parent() {
			Some(parent) if parent.starts_with(project_cwd) => current = parent.to_path_buf(),
			_ => break,
		}
	}
	if let Some(path) = std::env::var_os("PATH") {
		dirs.extend(std::env::split_paths(&path));
	}
	for name in names {
		for dir in &dirs {
			let candidate = dir.join(name);
			if is_executable(&candidate) {
				return Some(candidate);
			}
			if cfg!(windows) {
				for ext in ["exe", "cmd", "bat"] {
					let candidate = candidate.with_extension(ext);
					if candidate.is_file() {
						return Some(candidate);
					}
				}
			}
		}
	}
	None
}

fn is_executable(path: &Path) -> bool {
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
	}
	#[cfg(not(unix))]
	{
		path.is_file()
	}
}

fn format_command(argv: &[String]) -> String {
	let safe = Regex::new(r"^[A-Za-z0-9_./:=@+-]+$").expect("static");
	argv.iter()
		.map(|a| if safe.is_match(a) { a.clone() } else { serde_json::to_string(a).unwrap_or_default() })
		.collect::<Vec<_>>()
		.join(" ")
}

fn display_name(path: &Path) -> String {
	path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn add_plan(state: &mut State, request: PlanRequest) {
	let cwd = state.project_cwd.join(request.root);
	let executable = request
		.executable
		.clone()
		.or_else(|| resolve_binary(&state.project_cwd, request.root, request.binaries));
	let label = format!("{} ({})", request.label, request.root);
	let Some(executable) = executable else {
		if request.optional {
			return;
		}
		state.skipped.push(SkippedCheck {
			label,
			language: request.language.to_owned(),
			reason: format!("executable not found: {}", request.binaries.join(" or ")),
		});
		return;
	};
	let mut argv = vec![display_name(&executable)];
	argv.extend(request.args.iter().cloned());
	state.plans.push(Plan {
		id: format!("{}:{}", request.id, request.root),
		label,
		language: request.language.to_owned(),
		cwd,
		executable,
		args: request.args,
		parser: request.parser.to_owned(),
		command: format_command(&argv),
		mutates: request.mutates,
	});
}

pub fn discover(project_cwd: &Path, include_tests: bool) -> Suite {
	let resolved = resolve_cwd(project_cwd);
	let files = list_project_files(&resolved);
	let mut state = State {
		project_cwd: resolved.clone(),
		file_set: files.iter().cloned().collect(),
		files,
		plans: Vec::new(),
		skipped: Vec::new(),
		include_tests,
	};
	discover_rust(&mut state);
	discover_go(&mut state);
	discover_python(&mut state);
	discover_javascript(&mut state);
	discover_ruby(&mut state);
	discover_php(&mut state);
	discover_swift(&mut state);
	discover_dart(&mut state);
	discover_elixir(&mut state);
	discover_shell(&mut state);
	discover_haskell(&mut state);
	discover_terraform(&mut state);
	discover_lua(&mut state);
	discover_cpp(&mut state);
	discover_dotnet(&mut state);
	discover_zig(&mut state);
	discover_jvm(&mut state);
	discover_actions(&mut state);
	Suite {
		project_cwd:   resolved,
		active:        state.plans.clone(),
		plans:         state.plans,
		skipped:       state.skipped,
		allowed_files: state.file_set,
	}
}

/// One checker proposed by the prompted discovery agent.
#[derive(Debug, Clone)]
pub struct CustomSpec {
	pub label:    String,
	pub language: Option<String>,
	pub cwd:      Option<String>,
	pub command:  Vec<String>,
	pub parser:   Option<String>,
}

fn normalize_custom_root(project_cwd: &Path, cwd: Option<&str>) -> Option<String> {
	let trimmed = cwd.map(str::trim).unwrap_or_default();
	if trimmed.is_empty() || trimmed == "." {
		return Some(".".into());
	}
	let mut out = PathBuf::new();
	for component in Path::new(trimmed).components() {
		match component {
			std::path::Component::Normal(part) => out.push(part),
			std::path::Component::CurDir => {},
			std::path::Component::ParentDir => {
				if !out.pop() {
					return None;
				}
			},
			_ => return None,
		}
	}
	let _ = project_cwd;
	let text = out.to_string_lossy().replace('\\', "/");
	Some(if text.is_empty() { ".".into() } else { text })
}

pub fn build_custom(project_cwd: &Path, specs: &[CustomSpec]) -> Suite {
	let resolved = resolve_cwd(project_cwd);
	let files = list_project_files(&resolved);
	let mut plans = Vec::new();
	let mut skipped = Vec::new();
	for (index, spec) in specs.iter().enumerate() {
		let label = if spec.label.trim().is_empty() { format!("custom checker {}", index + 1) } else { spec.label.trim().to_owned() };
		let language = spec.language.as_deref().map(str::trim).filter(|l| !l.is_empty()).unwrap_or("Custom").to_owned();
		let Some(binary) = spec.command.first() else {
			skipped.push(SkippedCheck { label, language, reason: "empty command".into() });
			continue;
		};
		let Some(root) = normalize_custom_root(&resolved, spec.cwd.as_deref()) else {
			skipped.push(SkippedCheck {
				label,
				language,
				reason: format!("working directory escapes the project: {}", spec.cwd.clone().unwrap_or_default()),
			});
			continue;
		};
		let executable = if binary.contains('/') || binary.contains('\\') {
			let candidate = resolved.join(&root).join(binary);
			candidate.exists().then_some(candidate)
		} else {
			resolve_binary(&resolved, &root, &[binary.as_str()])
		};
		let Some(executable) = executable else {
			skipped.push(SkippedCheck { label, language, reason: format!("executable not found: {binary}") });
			continue;
		};
		let args: Vec<String> = spec.command[1..].to_vec();
		let mut argv = vec![display_name(&executable)];
		argv.extend(args.iter().cloned());
		plans.push(Plan {
			id: format!("custom-{}", index + 1),
			label,
			language,
			cwd: resolved.join(&root),
			executable,
			args,
			parser: spec
				.parser
				.as_deref()
				.filter(|p| parsers::PARSER_KINDS.contains(p))
				.unwrap_or("generic")
				.to_owned(),
			command: format_command(&argv),
			mutates: false,
		});
	}
	Suite {
		project_cwd: resolved,
		active: plans.clone(),
		plans,
		skipped,
		allowed_files: files.into_iter().collect(),
	}
}

fn cargo_workspace_packages(state: &State, root: &str, cargo: &Path) -> Result<Vec<String>, String> {
	let manifest = state.project_cwd.join(root).join("Cargo.toml");
	let output = Command::new(cargo)
		.args(["metadata", "--no-deps", "--format-version=1", "--manifest-path"])
		.arg(&manifest)
		.current_dir(state.project_cwd.join(root))
		.stdin(Stdio::null())
		.output()
		.map_err(|e| e.to_string())?;
	if !output.status.success() {
		let text = if output.stderr.is_empty() { &output.stdout } else { &output.stderr };
		return Err(parsers::sanitize_text(&String::from_utf8_lossy(text)).split_whitespace().collect::<Vec<_>>().join(" "));
	}
	let parsed: serde_json::Value =
		serde_json::from_slice(&output.stdout).map_err(|e| format!("invalid JSON: {e}"))?;
	let Some(root_obj) = parsed.as_object() else { return Err("metadata root is not an object".into()) };
	let members: HashSet<&str> = root_obj
		.get("workspace_members")
		.and_then(|v| v.as_array())
		.map(|a| a.iter().filter_map(|v| v.as_str()).collect())
		.unwrap_or_default();
	let mut packages = BTreeSet::new();
	for value in root_obj.get("packages").and_then(|v| v.as_array()).into_iter().flatten() {
		let (Some(id), Some(name), Some(manifest)) = (
			value.get("id").and_then(|v| v.as_str()),
			value.get("name").and_then(|v| v.as_str()),
			value.get("manifest_path").and_then(|v| v.as_str()),
		) else {
			continue;
		};
		if !members.contains(id) {
			continue;
		}
		if let Some(rel) = parsers::relative_to(&state.project_cwd, Path::new(manifest))
			&& state.file_set.contains(&rel)
		{
			packages.insert(name.to_owned());
		}
	}
	Ok(packages.into_iter().collect())
}

fn discover_rust(state: &mut State) {
	for root in roots_for(&state.files, &["Cargo.toml"]) {
		let Some(cargo) = resolve_binary(&state.project_cwd, &root, &["cargo"]) else {
			add_plan(state, PlanRequest::new("clippy", "cargo clippy", "Rust", &root, &["cargo"], &[], "rust"));
			continue;
		};
		let packages = match cargo_workspace_packages(state, &root, &cargo) {
			Ok(packages) => packages,
			Err(error) => {
				state.skipped.push(SkippedCheck {
					label:    format!("cargo clippy ({root})"),
					language: "Rust".into(),
					reason:   format!("cargo metadata failed: {error}"),
				});
				continue;
			},
		};
		if packages.is_empty() {
			state.skipped.push(SkippedCheck {
				label:    format!("cargo clippy ({root})"),
				language: "Rust".into(),
				reason:   "no first-party workspace packages found".into(),
			});
			continue;
		}
		let package_args: Vec<String> = packages.iter().flat_map(|p| ["--package".to_owned(), p.clone()]).collect();
		let mut args: Vec<String> = ["clippy", "--fix", "--allow-dirty", "--all-targets", "--no-deps", "--allow-staged", "--broken-code", "--allow-no-vcs"]
			.iter()
			.map(|s| (*s).to_owned())
			.collect();
		args.extend(package_args.iter().cloned());
		args.extend(["--message-format=json", "--", "-D", "warnings"].iter().map(|s| (*s).to_owned()));
		let mut request = PlanRequest::new("clippy", "cargo clippy", "Rust", &root, &["cargo"], &[], "rust");
		request.args = args;
		request.executable = Some(cargo.clone());
		request.mutates = true;
		add_plan(state, request);
		if state.include_tests {
			let mut args: Vec<String> = vec!["test".into(), "--no-fail-fast".into(), "--all-targets".into()];
			args.extend(package_args);
			args.push("--message-format=json".into());
			let mut request = PlanRequest::new("cargo-test", "cargo test", "Rust", &root, &["cargo"], &[], "rust-test");
			request.args = args;
			request.executable = Some(cargo);
			add_plan(state, request);
		}
	}
}

fn discover_go(state: &mut State) {
	let work = roots_for(&state.files, &["go.work"]);
	let roots = if work.is_empty() { roots_for(&state.files, &["go.mod"]) } else { work };
	for root in roots {
		let prefix = if root == "." { String::new() } else { format!("{root}/") };
		add_plan(state, PlanRequest::new("go-vet", "go vet", "Go", &root, &["go"], &["vet", "-json", "./..."], "go"));
		let mut request = PlanRequest::new("staticcheck", "staticcheck", "Go", &root, &["staticcheck"], &["-f", "json", "./..."], "staticcheck");
		request.optional = !state.file_set.contains(&format!("{prefix}staticcheck.conf"));
		add_plan(state, request);
		if state.include_tests {
			add_plan(state, PlanRequest::new("go-test", "go test", "Go", &root, &["go"], &["test", "-json", "./..."], "go-test"));
		}
	}
	for root in roots_pattern(&state.files, r"(?:^|/)\.golangci\.(?:ya?ml|toml|json)$") {
		add_plan(state, PlanRequest::new("golangci", "golangci-lint", "Go", &root, &["golangci-lint"], &["run"], "golangci"));
	}
}

fn discover_python(state: &mut State) {
	if !contains_ext(&state.files, &[".py", ".pyi"]) {
		return;
	}
	let roots = roots_for(&state.files, &[".ruff.toml", "pyproject.toml", "ruff.toml", "setup.cfg", "tox.ini"]);
	let roots = if roots.is_empty() { vec![".".to_owned()] } else { roots };
	for root in &roots {
		add_plan(state, PlanRequest::new("ruff", "ruff", "Python", root, &["ruff"], &["check", "--output-format=json", "."], "ruff"));
		if state.include_tests {
			add_plan(state, PlanRequest::new("pytest", "pytest", "Python", root, &["pytest"], &["-q"], "generic"));
		}
	}
	for root in roots_for(&state.files, &["pyrightconfig.json"]) {
		add_plan(state, PlanRequest::new("pyright", "pyright", "Python", &root, &["pyright", "basedpyright"], &["--outputjson"], "pyright"));
	}
	let pyproject_roots = roots_for(&state.files, &["pyproject.toml"]);
	let contents: Vec<(String, String)> = pyproject_roots
		.iter()
		.map(|root| {
			let file = if root == "." { "pyproject.toml".to_owned() } else { format!("{root}/pyproject.toml") };
			(root.clone(), std::fs::read_to_string(state.project_cwd.join(file)).unwrap_or_default())
		})
		.collect();
	let tool_roots = |state: &State, names: &[&str], section: &str| -> Vec<String> {
		let mut roots = roots_for(&state.files, names);
		roots.extend(contents.iter().filter(|(_, c)| c.contains(section)).map(|(r, _)| r.clone()));
		topmost_roots(roots)
	};
	for root in tool_roots(state, &["mypy.ini", ".mypy.ini"], "[tool.mypy") {
		add_plan(state, PlanRequest::new("mypy", "mypy", "Python", &root, &["mypy"], &["--no-error-summary", "--no-pretty", "."], "mypy"));
	}
	for root in tool_roots(state, &[".pylintrc", "pylintrc"], "[tool.pylint") {
		add_plan(state, PlanRequest::new("pylint", "pylint", "Python", &root, &["pylint"], &["--output-format=json", "--recursive=y", "."], "pylint"));
	}
	for root in roots_for(&state.files, &[".flake8"]) {
		add_plan(state, PlanRequest::new("flake8", "flake8", "Python", &root, &["flake8"], &["."], "flake8"));
	}
	for root in tool_roots(state, &["ty.toml"], "[tool.ty") {
		add_plan(state, PlanRequest::new("ty", "ty check", "Python", &root, &["ty"], &["check", "--output-format", "concise"], "ty"));
	}
}

fn discover_javascript(state: &mut State) {
	if !contains_ext(&state.files, &[".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts"]) {
		return;
	}
	const JS: &str = "JavaScript/TypeScript";
	for root in roots_for(&state.files, &["biome.json", "biome.jsonc"]) {
		add_plan(state, PlanRequest::new("biome", "biome check", JS, &root, &["biome"], &["check", ".", "--reporter=json"], "biome"));
	}
	for root in roots_pattern(&state.files, r"(?:^|/)(?:eslint\.config\.(?:js|mjs|cjs|ts|mts|cts)|\.eslintrc(?:\.(?:js|cjs|json|yaml|yml))?)$") {
		add_plan(state, PlanRequest::new("eslint", "eslint", JS, &root, &["eslint"], &[".", "--format=json"], "eslint"));
	}
	for root in roots_for(&state.files, &["tsconfig.json"]) {
		let prefix = if root == "." { String::new() } else { format!("{root}/") };
		let has_vue = state.files.iter().any(|f| f.starts_with(&prefix) && f.ends_with(".vue"));
		let binaries: &[&str] = if has_vue { &["vue-tsc", "tsgo", "tsc"] } else { &["tsgo", "tsc"] };
		add_plan(state, PlanRequest::new("typescript", if has_vue { "Vue TypeScript" } else { "TypeScript" }, "TypeScript", &root, binaries, &["--noEmit", "--pretty", "false"], "generic"));
	}
	for root in roots_for(&state.files, &[".oxlintrc.json"]) {
		add_plan(state, PlanRequest::new("oxlint", "oxlint", JS, &root, &["oxlint"], &["--format=unix"], "oxlint"));
	}
	for root in roots_for(&state.files, &["deno.json", "deno.jsonc"]) {
		add_plan(state, PlanRequest::new("deno-lint", "deno lint", JS, &root, &["deno"], &["lint", "--json"], "deno-lint"));
	}
	for root in roots_pattern(&state.files, r"(?:^|/)(?:\.stylelintrc(?:\.(?:json|ya?ml|js|cjs|mjs))?|stylelint\.config\.(?:js|cjs|mjs|ts))$") {
		add_plan(state, PlanRequest::new("stylelint", "stylelint", "CSS", &root, &["stylelint"], &["**/*.{css,scss,sass,less}", "--formatter", "json", "--allow-empty-input"], "stylelint"));
	}
	if state.include_tests {
		discover_package_tests(state);
	}
}

fn discover_package_tests(state: &mut State) {
	let mut candidates = Vec::new();
	for file in &state.files {
		if basename(file) != "package.json" {
			continue;
		}
		let Ok(text) = std::fs::read_to_string(state.project_cwd.join(file)) else { continue };
		let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
		let Some(test) = manifest.get("scripts").and_then(|s| s.get("test")).and_then(|t| t.as_str()) else { continue };
		if test.trim().is_empty() || test.to_lowercase().contains("no test specified") {
			continue;
		}
		let root = dirname(file);
		let prefix = if root == "." { String::new() } else { format!("{root}/") };
		let mut manager = manifest
			.get("packageManager")
			.and_then(|v| v.as_str())
			.map(|v| v.split('@').next().unwrap_or_default().to_lowercase())
			.unwrap_or_default();
		if !["bun", "npm", "pnpm", "yarn"].contains(&manager.as_str()) {
			manager = if state.file_set.contains(&format!("{prefix}bun.lock")) || state.file_set.contains(&format!("{prefix}bun.lockb")) {
				"bun"
			} else if state.file_set.contains(&format!("{prefix}pnpm-lock.yaml")) {
				"pnpm"
			} else if state.file_set.contains(&format!("{prefix}yarn.lock")) {
				"yarn"
			} else {
				"npm"
			}
			.to_owned();
		}
		candidates.push((root, manager));
	}
	let selected: HashSet<String> = topmost_roots(candidates.iter().map(|c| c.0.clone())).into_iter().collect();
	for (root, manager) in candidates {
		if !selected.contains(&root) {
			continue;
		}
		let id = format!("{manager}-test");
		let label = format!("{manager} test");
		let binaries = [manager.as_str()];
		add_plan(state, PlanRequest::new(&id, &label, "JavaScript/TypeScript", &root, &binaries, &["run", "test"], "generic"));
	}
}

fn discover_ruby(state: &mut State) {
	if !contains_ext(&state.files, &[".rb", ".rake"]) {
		return;
	}
	for root in roots_for(&state.files, &[".rubocop.yml", ".rubocop.yaml"]) {
		add_plan(state, PlanRequest::new("rubocop", "rubocop", "Ruby", &root, &["rubocop"], &["--format", "json", "--force-exclusion"], "rubocop"));
	}
	if state.include_tests {
		for root in roots_pattern(&state.files, r"(?:^|/)(?:\.rspec|Gemfile)$") {
			add_plan(state, PlanRequest::new("rspec", "RSpec", "Ruby", &root, &["rspec"], &["--format", "progress"], "generic"));
		}
	}
}

fn discover_php(state: &mut State) {
	if !contains_ext(&state.files, &[".php"]) {
		return;
	}
	for root in roots_pattern(&state.files, r"(?:^|/)phpstan\.neon(?:\.dist)?$") {
		add_plan(state, PlanRequest::new("phpstan", "PHPStan", "PHP", &root, &["phpstan"], &["analyse", "--error-format=json", "--no-progress"], "phpstan"));
	}
	for root in roots_pattern(&state.files, r"(?:^|/)psalm(?:\.xml|\.xml\.dist)$") {
		add_plan(state, PlanRequest::new("psalm", "Psalm", "PHP", &root, &["psalm"], &["--output-format=json", "--no-progress"], "psalm"));
	}
	if state.include_tests {
		for root in roots_pattern(&state.files, r"(?:^|/)phpunit\.xml(?:\.dist)?$") {
			add_plan(state, PlanRequest::new("phpunit", "PHPUnit", "PHP", &root, &["phpunit"], &[], "generic"));
		}
	}
}

fn discover_swift(state: &mut State) {
	if !contains_ext(&state.files, &[".swift"]) {
		return;
	}
	let configured = roots_for(&state.files, &[".swiftlint.yml"]);
	let roots = if configured.is_empty() { roots_for(&state.files, &["Package.swift"]) } else { configured };
	for root in roots {
		add_plan(state, PlanRequest::new("swiftlint", "SwiftLint", "Swift", &root, &["swiftlint"], &["lint", "--reporter", "json", "--quiet"], "swiftlint"));
		if state.include_tests {
			add_plan(state, PlanRequest::new("swift-test", "swift test", "Swift", &root, &["swift"], &["test"], "generic"));
		}
	}
}

fn discover_dart(state: &mut State) {
	if !contains_ext(&state.files, &[".dart"]) {
		return;
	}
	for root in roots_for(&state.files, &["pubspec.yaml"]) {
		add_plan(state, PlanRequest::new("dart-analyze", "dart analyze", "Dart", &root, &["dart"], &["analyze", "--format", "machine"], "dart"));
		if state.include_tests {
			add_plan(state, PlanRequest::new("dart-test", "dart test", "Dart", &root, &["dart"], &["test", "--reporter", "json"], "generic"));
		}
	}
}

fn discover_elixir(state: &mut State) {
	if !contains_ext(&state.files, &[".ex", ".exs"]) {
		return;
	}
	for root in roots_for(&state.files, &[".credo.exs"]) {
		add_plan(state, PlanRequest::new("credo", "Credo", "Elixir", &root, &["mix"], &["credo", "--format=json"], "credo"));
	}
	if state.include_tests {
		for root in roots_for(&state.files, &["mix.exs"]) {
			add_plan(state, PlanRequest::new("mix-test", "mix test", "Elixir", &root, &["mix"], &["test"], "generic"));
		}
	}
}

fn discover_shell(state: &mut State) {
	let re = Regex::new(r"\.(?:sh|bash|ksh|zsh)$").expect("static");
	let shell: Vec<String> = state.files.iter().filter(|f| re.is_match(f)).cloned().collect();
	for (index, batch) in shell.chunks(SHELLCHECK_BATCH_SIZE).enumerate() {
		let id = format!("shellcheck-{}", index + 1);
		let mut request = PlanRequest::new(&id, "ShellCheck", "Shell", ".", &["shellcheck"], &["--format=json1"], "shellcheck");
		request.args.extend(batch.iter().cloned());
		add_plan(state, request);
	}
}

fn discover_haskell(state: &mut State) {
	if !contains_ext(&state.files, &[".hs", ".lhs"]) {
		return;
	}
	let roots = roots_pattern(&state.files, r"(?:^|/)(?:\.hlint\.yaml|stack\.yaml|cabal\.project|[^/]+\.cabal)$");
	let roots = if roots.is_empty() { vec![".".to_owned()] } else { roots };
	for root in roots {
		add_plan(state, PlanRequest::new("hlint", "HLint", "Haskell", &root, &["hlint"], &["--json", "."], "hlint"));
		if state.include_tests {
			let prefix = if root == "." { String::new() } else { format!("{root}/") };
			if state.file_set.contains(&format!("{prefix}stack.yaml")) {
				add_plan(state, PlanRequest::new("stack-test", "stack test", "Haskell", &root, &["stack"], &["test"], "generic"));
			} else {
				add_plan(state, PlanRequest::new("cabal-test", "cabal test", "Haskell", &root, &["cabal"], &["test", "all"], "generic"));
			}
		}
	}
}

fn discover_terraform(state: &mut State) {
	if !contains_ext(&state.files, &[".tf"]) {
		return;
	}
	let configured = roots_for(&state.files, &[".tflint.hcl"]);
	let roots = if configured.is_empty() { roots_pattern(&state.files, r"\.tf$") } else { configured };
	for root in roots {
		add_plan(state, PlanRequest::new("tflint", "TFLint", "Terraform", &root, &["tflint"], &["--format=json", "--recursive"], "tflint"));
		add_plan(state, PlanRequest::new("terraform-validate", "terraform validate", "Terraform", &root, &["terraform"], &["validate", "-json"], "terraform"));
	}
}

fn discover_lua(state: &mut State) {
	if !contains_ext(&state.files, &[".lua"]) {
		return;
	}
	let configured = roots_for(&state.files, &[".luacheckrc"]);
	let roots = if configured.is_empty() { vec![".".to_owned()] } else { configured };
	for root in roots {
		add_plan(state, PlanRequest::new("luacheck", "Luacheck", "Lua", &root, &["luacheck"], &["."], "generic"));
	}
	let spec = Regex::new(r"(?:^|/)(?:spec|test)/.*_spec\.lua$").expect("static");
	if state.include_tests && state.files.iter().any(|f| spec.is_match(f)) {
		add_plan(state, PlanRequest::new("busted", "Busted", "Lua", ".", &["busted"], &[], "generic"));
	}
}

fn discover_cpp(state: &mut State) {
	if !contains_ext(&state.files, &[".c", ".cc", ".cpp", ".cxx", ".h", ".hh", ".hpp", ".hxx"]) {
		return;
	}
	for root in roots_for(&state.files, &["compile_commands.json"]) {
		let dir = state.project_cwd.join(&root).to_string_lossy().into_owned();
		add_plan(state, PlanRequest::new("clang-tidy", "clang-tidy", "C/C++", ".", &["run-clang-tidy", "run-clang-tidy.py"], &["-quiet", "-p", &dir], "generic"));
	}
}

fn discover_dotnet(state: &mut State) {
	if !contains_ext(&state.files, &[".cs", ".fs", ".vb"]) {
		return;
	}
	let solutions = roots_pattern(&state.files, r"(?i)\.(?:sln|slnx)$");
	let roots = if solutions.is_empty() { roots_pattern(&state.files, r"(?i)\.(?:csproj|fsproj|vbproj)$") } else { solutions };
	for root in roots {
		add_plan(state, PlanRequest::new("dotnet-build", "dotnet build", ".NET", &root, &["dotnet"], &["build", "--no-restore", "--nologo", "--verbosity", "quiet"], "generic"));
		if state.include_tests {
			add_plan(state, PlanRequest::new("dotnet-test", "dotnet test", ".NET", &root, &["dotnet"], &["test", "--no-restore", "--nologo", "--verbosity", "quiet"], "generic"));
		}
	}
}

fn discover_zig(state: &mut State) {
	if !contains_ext(&state.files, &[".zig"]) {
		return;
	}
	for root in roots_for(&state.files, &["build.zig"]) {
		add_plan(state, PlanRequest::new("zig-build", "zig build", "Zig", &root, &["zig"], &["build"], "generic"));
		if state.include_tests {
			add_plan(state, PlanRequest::new("zig-test", "zig build test", "Zig", &root, &["zig"], &["build", "test"], "generic"));
		}
	}
}

fn wrapper(state: &State, root: &str, unix: &str, windows: &str) -> Option<PathBuf> {
	let join = |name: &str| if root == "." { name.to_owned() } else { format!("{root}/{name}") };
	[join(unix), join(windows)]
		.into_iter()
		.find(|f| state.file_set.contains(f))
		.map(|f| state.project_cwd.join(f))
}

fn discover_jvm(state: &mut State) {
	if !contains_ext(&state.files, &[".java", ".kt", ".kts", ".scala"]) {
		return;
	}
	for root in roots_for(&state.files, &["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"]) {
		let wrap = wrapper(state, &root, "gradlew", "gradlew.bat");
		let mut request = PlanRequest::new("gradle-check", "Gradle check", "JVM", &root, &["gradle"], &["check", "-x", "test", "--console=plain", "--no-daemon"], "generic");
		request.executable = wrap.clone();
		add_plan(state, request);
		if state.include_tests {
			let mut request = PlanRequest::new("gradle-test", "Gradle test", "JVM", &root, &["gradle"], &["test", "--console=plain", "--no-daemon"], "generic");
			request.executable = wrap;
			add_plan(state, request);
		}
	}
	for root in roots_for(&state.files, &["pom.xml"]) {
		let wrap = wrapper(state, &root, "mvnw", "mvnw.cmd");
		let mut request = PlanRequest::new("maven-verify", "Maven verify", "JVM", &root, &["mvn"], &["-q", "-DskipTests", "-DskipITs", "verify"], "generic");
		request.executable = wrap.clone();
		add_plan(state, request);
		if state.include_tests {
			let mut request = PlanRequest::new("maven-test", "Maven test", "JVM", &root, &["mvn"], &["-q", "test"], "generic");
			request.executable = wrap;
			add_plan(state, request);
		}
	}
}

fn discover_actions(state: &mut State) {
	let re = Regex::new(r"^(?:(.*)/)?\.github/workflows/[^/]+\.ya?ml$").expect("static");
	let roots: Vec<String> = state
		.files
		.iter()
		.filter_map(|f| re.captures(f).map(|c| c.get(1).map_or(".".to_owned(), |m| m.as_str().to_owned())))
		.collect();
	for root in topmost_roots(roots) {
		let mut request = PlanRequest::new("actionlint", "actionlint", "GitHub Actions", &root, &["actionlint"], &["-format", "{{json .}}"], "actionlint");
		request.optional = true;
		add_plan(state, request);
	}
}

fn read_capped(mut stream: impl Read) -> String {
	let mut buffer = Vec::new();
	let mut chunk = [0u8; 65536];
	while let Ok(n) = stream.read(&mut chunk) {
		if n == 0 {
			break;
		}
		let remaining = MAX_CHECKER_OUTPUT.saturating_sub(buffer.len());
		buffer.extend_from_slice(&chunk[..n.min(remaining)]);
	}
	String::from_utf8_lossy(&buffer).into_owned()
}

fn failure(plan: &Plan, exit: Option<i32>, stdout: &str, stderr: &str) -> Diagnostic {
	let joined = [stderr, stdout].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
	let output = parsers::sanitize_text(&joined).trim().to_owned();
	let suffix = if output.is_empty() { String::new() } else { format!(": {}", output.chars().take(MAX_FAILURE_OUTPUT).collect::<String>()) };
	Diagnostic {
		checker: plan.label.clone(),
		file: None,
		line: None,
		column: None,
		end_line: None,
		end_column: None,
		code: None,
		severity: Severity::Error,
		message: format!("{} failed{}{suffix}", plan.label, exit.map(|c| format!(" with exit code {c}")).unwrap_or_default()),
		suggestion: None,
	}
}

fn run_checker(plan: &Plan, project_cwd: &Path, allowed: &HashSet<String>) -> CheckResult {
	let result = |exit: Option<i32>, diagnostics: Vec<Diagnostic>| CheckResult {
		id: plan.id.clone(),
		label: plan.label.clone(),
		language: plan.language.clone(),
		cwd: parsers::relative_to(project_cwd, &plan.cwd).unwrap_or_else(|| ".".into()),
		command: plan.command.clone(),
		exit_code: exit,
		diagnostics,
	};
	let child = Command::new(&plan.executable)
		.args(&plan.args)
		.current_dir(&plan.cwd)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn();
	let mut child = match child {
		Ok(child) => child,
		Err(error) => {
			let fail = failure(plan, None, "", &error.to_string());
			return result(None, vec![fail]);
		},
	};
	let stdout_pipe = child.stdout.take();
	let stderr_pipe = child.stderr.take();
	let (stdout, stderr) = std::thread::scope(|scope| {
		let out = scope.spawn(|| stdout_pipe.map(read_capped).unwrap_or_default());
		let err = scope.spawn(|| stderr_pipe.map(read_capped).unwrap_or_default());
		(out.join().unwrap_or_default(), err.join().unwrap_or_default())
	});
	let exit = child.wait().ok().and_then(|s| s.code());
	let parsed = parsers::parse(&plan.parser, &parsers::Input {
		checker:     &plan.label,
		project_cwd,
		checker_cwd: &plan.cwd,
		stdout:      &stdout,
		stderr:      &stderr,
	});
	let parsed_count = parsed.len();
	let mut diagnostics: Vec<Diagnostic> =
		parsed.into_iter().filter(|d| d.file.as_ref().is_none_or(|f| allowed.contains(f))).collect();
	if exit != Some(0) && parsed_count == 0 {
		diagnostics.push(failure(plan, exit, &stdout, &stderr));
	}
	result(exit, diagnostics)
}
