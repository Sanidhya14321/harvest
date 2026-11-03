//! Read-only legacy settings.
//!
//! Layers: the global `config.yml`/`config.yaml` in the agent directory, the
//! builtin project layer (`.harvest/settings.json` then
//! `.harvest/config.yml`), and explicit overlays, deep-merged like
//! `Settings.#deepMerge` (global → project → overlays) with schema defaults
//! from the generated `settings-schema.json` (`bun run gen:settings-schema`).
use std::{
	collections::BTreeMap,
	path::{Path, PathBuf},
	sync::LazyLock,
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
struct SchemaEntry {
	#[serde(rename = "type")]
	kind:    String,
	default: Option<Value>,
	values:  Option<Vec<String>>,
}

static SCHEMA: LazyLock<BTreeMap<String, SchemaEntry>> = LazyLock::new(|| {
	serde_json::from_str(include_str!("settings-schema.json")).expect("generated settings schema")
});

/// Effective settings for one workspace.
#[derive(Debug, Clone, Default)]
pub struct Settings {
	merged: Map<String, Value>,
}

/// `#deepMerge`: objects merge key by key; anything else replaces.
fn deep_merge(base: &mut Map<String, Value>, overrides: Map<String, Value>) {
	for (key, value) in overrides {
		match (base.get_mut(&key), value) {
			(Some(Value::Object(existing)), Value::Object(incoming)) => deep_merge(existing, incoming),
			(_, value) => {
				base.insert(key, value);
			},
		}
	}
}

fn yaml_mapping(text: &str, path: &Path) -> Result<Map<String, Value>> {
	let value: Value = serde_yaml_ng::from_str(text)
		.with_context(|| format!("Failed to parse {}", path.display()))?;
	match value {
		Value::Null => Ok(Map::new()),
		Value::Object(map) => Ok(map),
		_ => Ok(Map::new()),
	}
}

fn read_optional(path: &Path) -> Result<Option<String>> {
	match std::fs::read_to_string(path) {
		Ok(text) => Ok(Some(text)),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(error) => Err(error).with_context(|| format!("Failed to read {}", path.display())),
	}
}

impl Settings {
	/// Load the layers for `cwd`. Unparsable persisted files are skipped
	/// (startup quarantine); explicit `overlays` must exist and parse.
	pub fn load(agent_dir: Option<&Path>, cwd: &Path, overlays: &[PathBuf]) -> Result<Self> {
		let mut merged = Map::new();
		if let Some(dir) = agent_dir {
			for name in ["config.yml", "config.yaml"] {
				let path = dir.join(name);
				if let Some(text) = read_optional(&path)? {
					if let Ok(layer) = yaml_mapping(&text, &path) {
						deep_merge(&mut merged, layer);
					}
					break;
				}
			}
		}
		let project = cwd.join(".harvest");
		if let Some(text) = read_optional(&project.join("settings.json"))?
			&& let Ok(Value::Object(layer)) = serde_json::from_str(&text)
		{
			deep_merge(&mut merged, layer);
		}
		let config = project.join("config.yml");
		if let Some(text) = read_optional(&config)?
			&& let Ok(layer) = yaml_mapping(&text, &config)
		{
			deep_merge(&mut merged, layer);
		}
		for overlay in overlays {
			let Some(text) = read_optional(overlay)? else {
				bail!("Config overlay not found: {}", overlay.display());
			};
			let value: Value = serde_yaml_ng::from_str(&text).map_err(|error| {
				anyhow::anyhow!("Failed to parse config overlay {}: {error}", overlay.display())
			})?;
			match value {
				Value::Null => {},
				Value::Object(layer) => deep_merge(&mut merged, layer),
				_ => bail!("Config overlay must be a YAML mapping: {}", overlay.display()),
			}
		}
		Ok(Self { merged })
	}

	/// `get`: the merged value at the dotted path, else the schema default.
	pub fn get(&self, path: &str) -> Option<Value> {
		let mut current = Some(&self.merged);
		let mut value = None;
		let segments: Vec<&str> = path.split('.').collect();
		for (index, segment) in segments.iter().enumerate() {
			let Some(map) = current else { break };
			match map.get(*segment) {
				Some(Value::Object(next)) if index + 1 < segments.len() => current = Some(next),
				Some(found) if index + 1 == segments.len() => value = Some(found.clone()),
				_ => break,
			}
		}
		value.or_else(|| SCHEMA.get(path).and_then(|entry| entry.default.clone()))
	}

	pub fn bool(&self, path: &str) -> bool {
		self
			.get(path)
			.and_then(|value| value.as_bool())
			.unwrap_or(false)
	}

	pub fn number(&self, path: &str) -> Option<f64> {
		match self.get(path)? {
			Value::Number(number) => number.as_f64(),
			// Enum-backed numbers (`"5"`) and quoted YAML numbers.
			Value::String(text) => text.trim().parse().ok(),
			_ => None,
		}
	}

	pub fn string(&self, path: &str) -> Option<String> {
		match self.get(path)? {
			Value::String(text) => Some(text),
			Value::Number(number) => Some(number.to_string()),
			Value::Bool(flag) => Some(flag.to_string()),
			_ => None,
		}
	}

	/// Whether `path` is a known setting and `value` fits its declared type.
	pub fn validates(path: &str, value: &Value) -> bool {
		let Some(entry) = SCHEMA.get(path) else {
			return false;
		};
		match entry.kind.as_str() {
			"boolean" => value.is_boolean(),
			"number" => value.is_number(),
			"string" => value.is_string(),
			"enum" => value.as_str().is_some_and(|v| {
				entry
					.values
					.as_ref()
					.is_some_and(|values| values.iter().any(|x| x == v))
			}),
			"array" => value.is_array(),
			"record" | "object" => value.is_object(),
			_ => true,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn project_layers_override_global_and_defaults_fill_gaps() -> Result<()> {
		let agent = tempfile::tempdir()?;
		let cwd = tempfile::tempdir()?;
		std::fs::write(
			agent.path().join("config.yml"),
			"grep:\n  contextBefore: 4\n  contextAfter: 2\n",
		)?;
		std::fs::create_dir(cwd.path().join(".harvest"))?;
		std::fs::write(cwd.path().join(".harvest/settings.json"), r#"{"grep":{"contextAfter":7}}"#)?;
		std::fs::write(cwd.path().join(".harvest/config.yml"), "tools:\n  maxTimeout: 60\n")?;
		let settings = Settings::load(Some(agent.path()), cwd.path(), &[])?;
		assert_eq!(settings.number("grep.contextBefore"), Some(4.0));
		assert_eq!(settings.number("grep.contextAfter"), Some(7.0));
		assert_eq!(settings.number("tools.maxTimeout"), Some(60.0));
		assert_eq!(settings.string("memory.backend").as_deref(), Some("off"));
		assert!(settings.get("not.a.setting").is_none());
		Ok(())
	}

	#[test]
	fn overlays_must_exist_and_be_mappings_while_bad_persisted_files_are_skipped() -> Result<()> {
		let agent = tempfile::tempdir()?;
		let cwd = tempfile::tempdir()?;
		std::fs::write(agent.path().join("config.yml"), ": : not yaml [")?;
		let missing = cwd.path().join("nope.yml");
		let error = Settings::load(Some(agent.path()), cwd.path(), &[missing])
			.unwrap_err()
			.to_string();
		assert!(error.starts_with("Config overlay not found"), "{error}");
		let list = cwd.path().join("list.yml");
		std::fs::write(&list, "- a\n")?;
		let error = Settings::load(Some(agent.path()), cwd.path(), &[list])
			.unwrap_err()
			.to_string();
		assert!(error.starts_with("Config overlay must be a YAML mapping"), "{error}");
		assert_eq!(
			Settings::load(Some(agent.path()), cwd.path(), &[])?.number("grep.contextBefore"),
			Some(1.0)
		);
		assert!(Settings::validates("memory.backend", &Value::String("mnemopi".into())));
		assert!(!Settings::validates("memory.backend", &Value::String("bogus".into())));
		Ok(())
	}
}
