//! Secret redaction for share uploads, ported from
//! `packages/coding-agent/src/secrets/index.ts` (entry loading) and a reduced
//! `SecretObfuscator`. Share blobs leave the machine, so redaction here is
//! one-way: obfuscate-mode matches become `#<NAME>_REDACTED#` markers rather
//! than the TS keyed reversible placeholders.
use std::path::Path;

use fancy_regex::Regex;
use harvest_runtime::JailedFs;
use serde_json::Value;

const MIN_ENV_VALUE_LENGTH: usize = 8;
const SENSITIVE_TOKEN_RE: &str = r"(?i)(?<![a-zA-Z0-9_*-])(gh[opusr]_[a-zA-Z0-9_*]{36,}|github_pat_[a-zA-Z0-9_*]{36,}|glpat-[a-zA-Z0-9_*-]{20,}|sk-proj-[a-zA-Z0-9_*-]{36,}|sk-ant-[a-zA-Z0-9_*-]{36,}|sk-[a-zA-Z0-9_*-]{48,})(?![a-zA-Z0-9_*-])";

struct Entry {
	regex:       bool,
	content:     String,
	replace:     bool,
	replacement: Option<String>,
	flags:       Option<String>,
	name:        Option<String>,
}

pub(crate) struct SecretObfuscator {
	plain: Vec<(String, String)>,
	regex: Vec<(Regex, String)>,
}

fn marker(name: Option<&str>) -> String {
	let cleaned: String = name
		.unwrap_or("SECRET")
		.chars()
		.map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
		.collect();
	format!("#{}_REDACTED#", cleaned.trim_matches('_'))
}

/// `compileSecretRegex`: accept `/pattern/flags` literal syntax plus flags.
fn compile(pattern: &str, flags: Option<&str>) -> Option<Regex> {
	let mut pat = pattern.to_owned();
	let mut flags: String = flags.unwrap_or_default().to_owned();
	if let Some(rest) = pattern.strip_prefix('/')
		&& let Some(idx) = rest.rfind('/')
		&& rest[idx + 1..].chars().all(|c| " gimsuy".contains(c))
	{
		flags.push_str(&rest[idx + 1..]);
		pat = rest[..idx].to_owned();
	}
	let inline: String = flags.chars().filter(|c| "ims".contains(*c)).collect();
	let full = if inline.is_empty() { pat } else { format!("(?{inline}){pat}") };
	Regex::new(&full).ok()
}

fn load_file(path: &Path) -> Vec<Entry> {
	let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return Vec::new() };
	let Ok(fs) = JailedFs::open(dir) else { return Vec::new() };
	let Ok(Some(bytes)) = fs.read_optional(Path::new(name), 1024 * 1024) else { return Vec::new() };
	let Ok(Value::Array(raw)) = serde_yaml_ng::from_slice::<Value>(&bytes) else { return Vec::new() };
	raw.into_iter()
		.filter_map(|e| {
			let kind = e.get("type")?.as_str()?;
			let content = e.get("content")?.as_str().filter(|c| !c.is_empty())?.to_owned();
			let mode = e.get("mode").and_then(Value::as_str).unwrap_or("obfuscate");
			if !matches!(kind, "plain" | "regex") || !matches!(mode, "obfuscate" | "replace") {
				return None;
			}
			let flags = e.get("flags").and_then(Value::as_str).map(str::to_owned);
			if kind == "regex" && compile(&content, flags.as_deref()).is_none() {
				return None;
			}
			Some(Entry {
				regex: kind == "regex",
				content,
				replace: mode == "replace",
				replacement: e.get("replacement").and_then(Value::as_str).map(str::to_owned),
				flags,
				name: e.get("friendlyName").and_then(Value::as_str).map(str::to_owned),
			})
		})
		.collect()
}

impl SecretObfuscator {
	/// `buildSecretObfuscator`: env secrets + secrets.yml (global, project
	/// overrides) + built-in credential patterns.
	pub(crate) fn build(cwd: &Path, agent_dir: &Path) -> Self {
		let global = load_file(&agent_dir.join("secrets.yml"));
		let project = load_file(&cwd.join(".omp").join("secrets.yml"));
		let mut entries: Vec<Entry> = Vec::new();
		let secret_env =
			Regex::new(r"(?i)(?:KEY|SECRET|TOKEN|PASSWORD|PASS|AUTH|CREDENTIAL|PRIVATE|OAUTH)(?:_|$)")
				.expect("valid regex");
		let mut seen = std::collections::HashSet::new();
		for (name, value) in std::env::vars() {
			if value.len() < MIN_ENV_VALUE_LENGTH || !secret_env.is_match(&name).unwrap_or(false) {
				continue;
			}
			if seen.insert(value.clone()) {
				entries.push(Entry {
					regex:       false,
					content:     value,
					replace:     false,
					replacement: None,
					flags:       None,
					name:        None,
				});
			}
		}
		let project_contents: std::collections::HashSet<String> =
			project.iter().map(|e| e.content.clone()).collect();
		entries.extend(global.into_iter().filter(|e| !project_contents.contains(&e.content)));
		entries.extend(project);
		entries.push(Entry {
			regex:       true,
			content:     SENSITIVE_TOKEN_RE.into(),
			replace:     false,
			replacement: None,
			flags:       None,
			name:        Some("Credential".into()),
		});

		let mut plain = Vec::new();
		let mut regex = Vec::new();
		for e in entries {
			let out = match (&e.replacement, e.replace) {
				(Some(r), _) => r.clone(),
				(None, true) => "#REDACTED#".into(),
				(None, false) => marker(e.name.as_deref()),
			};
			if e.regex {
				if let Some(re) = compile(&e.content, e.flags.as_deref()) {
					regex.push((re, out));
				}
			} else {
				plain.push((e.content, out));
			}
		}
		// Longest plain secrets first so overlapping values redact fully.
		plain.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
		Self { plain, regex }
	}

	pub(crate) fn has_secrets(&self) -> bool {
		!self.plain.is_empty() || !self.regex.is_empty()
	}

	pub(crate) fn obfuscate(&self, text: &str) -> String {
		let mut out = text.to_owned();
		for (secret, repl) in &self.plain {
			if out.contains(secret.as_str()) {
				out = out.replace(secret.as_str(), repl);
			}
		}
		for (re, repl) in &self.regex {
			out = re.replace_all(&out, fancy_regex::NoExpand(repl)).into_owned();
		}
		out
	}
}
