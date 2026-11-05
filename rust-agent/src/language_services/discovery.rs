//! Workspace LSP discovery over the existing shared declarative defaults.
//! This module selects configuration; the existing client owns every process.

use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use globset::GlobBuilder;
use harvest_runtime::JailedFs;
use serde::Serialize;
use serde_json::{Value, json};

use super::configuration::LspConfiguration;

const DEFAULTS: &str = include_str!("../../../packages/coding-agent/src/lsp/defaults.json");
const CONFIG_LIMIT: usize = 512 * 1024;
const CHAIN_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct UnavailableServer {
	pub name:   String,
	pub reason: &'static str,
}

pub struct Discovery {
	pub configurations: Vec<LspConfiguration>,
	pub unavailable:    Vec<UnavailableServer>,
}

fn has_markers(fs: &JailedFs, markers: &[String]) -> Result<bool> {
	ensure!(markers.len() <= 128, "too many LSP root markers");
	let mut entries = None;
	for marker in markers {
		ensure!(!marker.is_empty() && marker.len() <= 4096, "invalid LSP root marker");
		if marker.contains(['*', '?', '[', '{']) {
			let glob = GlobBuilder::new(marker)
				.literal_separator(true)
				.build()?
				.compile_matcher();
			if entries.is_none() {
				entries = Some(fs.entries(Path::new("."), 10_000)?);
			}
			if entries
				.as_ref()
				.is_some_and(|entries| entries.iter().any(|entry| glob.is_match(&entry.name)))
			{
				return Ok(true);
			}
		} else if fs.entry_exists(Path::new(marker))? {
			return Ok(true);
		}
	}
	Ok(false)
}

fn resolve_command(fs: &JailedFs, command: &str) -> Result<Option<PathBuf>> {
	let path = Path::new(command);
	if path.is_absolute() {
		return Ok(path.is_file().then(|| path.to_owned()));
	}
	if path.components().count() > 1 {
		let path = fs.resource_path(path)?;
		return Ok(fs
			.file_type(&path)
			.is_ok_and(|kind| kind.is_file())
			.then_some(path));
	}
	// Search the same project-local owners before PATH; no shell or runtime
	// probe is executed during discovery.
	for (markers, directories) in [
		(
			&["package.json", "package-lock.json", "yarn.lock", "pnpm-lock.yaml"][..],
			&["node_modules/.bin"][..],
		),
		(
			&[
				"pyproject.toml",
				"ty.toml",
				"requirements.txt",
				"setup.py",
				"setup.cfg",
				"Pipfile",
				"pyrightconfig.json",
				"ruff.toml",
				".ruff.toml",
			][..],
			&[".venv/bin", ".venv/Scripts", "venv/bin", "venv/Scripts", ".env/bin", ".env/Scripts"][..],
		),
		(&["Gemfile", "Gemfile.lock"][..], &["vendor/bundle/bin", "bin"][..]),
		(&["go.mod", "go.sum", "go.work"][..], &["bin"][..]),
	] {
		if !has_markers(fs, &markers.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())? {
			continue;
		}
		for directory in directories {
			for suffix in if cfg!(windows) {
				&["", ".exe", ".cmd", ".bat"][..]
			} else {
				&[""][..]
			} {
				let candidate = Path::new(directory).join(format!("{command}{suffix}"));
				if fs.entry_exists(&candidate)? {
					let resolved = fs.resource_path(&candidate)?;
					if fs.file_type(&resolved)?.is_file() {
						return Ok(Some(resolved));
					}
				}
			}
		}
	}
	Ok(crate::commands::find_executable(command))
}

fn strings(value: Option<&Value>) -> Vec<String> {
	value
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(Value::as_str)
		.map(str::to_owned)
		.collect()
}

fn native_typescript(command: &Path) -> bool {
	let real = command
		.canonicalize()
		.unwrap_or_else(|_| command.to_owned());
	let Some(parent) = command.parent() else {
		return false;
	};
	let mut candidates = vec![parent.join("../typescript"), parent.join("node_modules/typescript")];
	if real
		.parent()
		.and_then(Path::file_name)
		.is_some_and(|name| name == "bin")
	{
		if let Some(package) = real.parent().and_then(Path::parent) {
			candidates.insert(0, package.to_owned());
		}
	}
	candidates
		.into_iter()
		.find(|directory| directory.join("package.json").is_file())
		.is_some_and(|directory| !directory.join("lib/tsserver.js").is_file())
}

/// Project layers are merged from low to high priority. Global/user/plugin
/// sources remain owned by the application's configuration loader.
pub fn discover(root: &Path) -> Result<Discovery> {
	let fs = JailedFs::open(root)?;
	let mut defaults: BTreeMap<String, Value> = serde_json::from_str(DEFAULTS)?;
	let mut bytes_read = 0usize;
	for directory in [".claude", ".pi", ".omp", ".harvest", ""] {
		for file in [".lsp.yml", "lsp.yml", ".lsp.yaml", "lsp.yaml", ".lsp.json", "lsp.json"] {
			let path = Path::new(directory).join(file);
			let Some(bytes) = fs.read_optional(&path, CONFIG_LIMIT)? else {
				continue;
			};
			bytes_read += bytes.len();
			ensure!(bytes_read <= CHAIN_LIMIT, "LSP project config chain exceeds limit");
			let document: Value = if file.ends_with("json") {
				serde_json::from_slice(&bytes)?
			} else {
				serde_yaml_ng::from_slice(&bytes)?
			};
			let servers = document
				.get("servers")
				.unwrap_or(&document)
				.as_object()
				.context("invalid LSP project server mapping")?;
			ensure!(servers.len() <= 128, "too many LSP project server overrides");
			for (name, patch) in servers {
				if name == "idleTimeoutMs" {
					continue;
				}
				ensure!(name.len() <= 256 && patch.is_object(), "invalid LSP server override");
				let base = defaults.entry(name.clone()).or_insert_with(|| json!({}));
				base
					.as_object_mut()
					.context("invalid LSP default server")?
					.extend(patch.as_object().context("invalid LSP override")?.clone());
			}
		}
	}
	ensure!(defaults.len() <= 256, "LSP discovery server inventory exceeds limit");
	let mut configurations = Vec::new();
	let mut unavailable = Vec::new();
	let mut defaults: Vec<_> = defaults.into_iter().collect();
	defaults.sort_by_key(|(name, raw)| (raw["isLinter"] == true, name.clone()));
	for (name, raw) in defaults {
		let skip = |reason| UnavailableServer { name: name.clone(), reason };
		if raw["disabled"] == true {
			unavailable.push(skip("disabled"));
			continue;
		}
		if name == "swiftlint"
			&& raw["command"] == "swiftlint"
			&& strings(raw.get("args")).iter().any(|arg| arg == "lint")
		{
			unavailable.push(skip("custom_linter_transport_pending"));
			continue;
		}
		let extensions = if raw.get("fileTypes").is_some() {
			strings(raw.get("fileTypes"))
		} else {
			raw["extensionToLanguage"]
				.as_object()
				.map(|m| m.keys().cloned().collect())
				.unwrap_or_default()
		};
		let markers = if raw.get("rootMarkers").is_some() {
			strings(raw.get("rootMarkers"))
		} else if raw.get("extensionToLanguage").is_some() {
			vec![".".into()]
		} else {
			Vec::new()
		};
		let Some(command) = raw["command"]
			.as_str()
			.filter(|command| !command.is_empty())
		else {
			unavailable.push(skip("invalid_configuration"));
			continue;
		};
		if extensions.is_empty() || markers.is_empty() {
			unavailable.push(skip("invalid_configuration"));
			continue;
		}
		if !has_markers(&fs, &markers)? {
			unavailable.push(skip("project_markers_absent"));
			continue;
		}
		let Some(command) = resolve_command(&fs, command)? else {
			unavailable.push(skip("executable_unavailable"));
			continue;
		};
		if configurations.len() == 16 {
			unavailable.push(skip("native_server_limit"));
			continue;
		}
		let mut arguments = strings(raw.get("args"));
		for argument in &mut arguments {
			if argument == "$PID" {
				*argument = std::process::id().to_string();
			}
		}
		let config: LspConfiguration = serde_json::from_value(json!({
			"name":name,"command":command,"args":arguments,"env":raw.get("env").cloned().unwrap_or_else(||json!({})),
			"cwd":raw.get("cwd"),"timeout_ms":raw.get("timeout_ms").and_then(Value::as_u64).unwrap_or(30_000),
			"language_id":raw.get("languageId").or_else(|| raw.get("language_id")).and_then(Value::as_str).unwrap_or(""),
			"extensions":extensions,"initialization_options":raw.get("initOptions").or_else(||raw.get("initializationOptions")).cloned().unwrap_or_else(||json!({})),
			"settings":raw.get("settings").cloned().unwrap_or_else(||json!({})),"is_linter":raw["isLinter"]==true,"startup_approval":true,
		}))?;
		config.validate()?;
		configurations.push(config);
	}
	if let Some(native) = configurations
		.iter()
		.find(|c| c.process.name == "typescript-native")
	{
		let omit = if native_typescript(&native.process.command) {
			"typescript-language-server"
		} else {
			"typescript-native"
		};
		configurations.retain(|c| c.process.name != omit);
	}
	configurations.sort_by_key(|config| (config.is_linter, config.process.name.clone()));
	Ok(Discovery { configurations, unavailable })
}
