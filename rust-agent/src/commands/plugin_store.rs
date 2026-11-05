//! Shared plugin storage for `plugin`, `install` and `update --plugins`.
//!
//! Mirrors `extensibility/plugins/manager.ts` (npm/link plugins under
//! `~/.harvest/plugins`) and the read side of
//! `extensibility/plugins/marketplace/registry.ts`. Every mutation goes
//! through a [`JailedFs`] rooted at the plugins directory.

use std::{
	collections::BTreeSet,
	io::Read,
	path::{Path, PathBuf},
	process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, read_bounded};
use serde_json::{Map, Value, json};

use super::paths::{Xdg, root_subdir};

/// Cap on captured child-process stderr (bun install/uninstall/update).
const CHILD_OUTPUT_LIMIT: u64 = 64 * 1024;

pub(crate) const APP_NAME: &str = "harvest";

pub(crate) fn plugins_dir() -> PathBuf {
	root_subdir("plugins", Xdg::Data)
}

fn read_json_at(path: &Path) -> Result<Option<Value>> {
	match std::fs::File::open(path) {
		Ok(file) => {
			let bytes = read_bounded(file, MAX_FILE_BYTES)?;
			Ok(Some(
				serde_json::from_slice(&bytes)
					.with_context(|| format!("invalid JSON in {}", path.display()))?,
			))
		},
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(error) => Err(error.into()),
	}
}

fn obj(value: &Value, key: &str) -> Map<String, Value> {
	value
		.get(key)
		.and_then(Value::as_object)
		.cloned()
		.unwrap_or_default()
}

// =============================================================================
// Plugin spec parsing (`parser.ts`, `classify-install-target.ts`)
// =============================================================================

pub(crate) enum Features {
	Default,
	All,
	List(Vec<String>),
}

pub(crate) fn parse_plugin_spec(spec: &str) -> (String, Features) {
	let (Some(start), Some(end)) = (spec.rfind('['), spec.rfind(']')) else {
		return (spec.to_owned(), Features::Default);
	};
	if end < start {
		return (spec.to_owned(), Features::Default);
	}
	let name = spec[..start].to_owned();
	let inner = spec[start + 1..end].trim();
	if inner == "*" {
		return (name, Features::All);
	}
	(name, Features::List(split_list(inner)))
}

pub(crate) fn split_list(text: &str) -> Vec<String> {
	text
		.split(',')
		.map(str::trim)
		.filter(|s| !s.is_empty())
		.map(str::to_owned)
		.collect()
}

pub(crate) enum InstallTarget {
	Local(String),
	Marketplace { name: String, marketplace: String },
	Npm(String),
}

fn is_local_path_spec(spec: &str) -> bool {
	matches!(spec, "." | ".." | "~")
		|| ["./", "../", ".\\", "..\\", "~/", "~\\", "/", "\\\\"]
			.iter()
			.any(|p| spec.starts_with(p))
		|| is_drive_path(spec)
}

pub(crate) fn is_drive_path(spec: &str) -> bool {
	let b = spec.as_bytes();
	b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

const NPM_DIST_TAGS: &[&str] =
	&["latest", "next", "beta", "alpha", "canary", "rc", "dev", "stable", "nightly", "experimental"];

pub(crate) fn classify_install_target(spec: &str, known: &BTreeSet<String>) -> InstallTarget {
	if is_local_path_spec(spec) {
		return InstallTarget::Local(spec.to_owned());
	}
	if spec.starts_with('@') {
		return InstallTarget::Npm(spec.to_owned());
	}
	if let Some(at) = spec.rfind('@').filter(|&at| at > 0) {
		let rhs = &spec[at + 1..];
		let versionish = rhs.starts_with(|c: char| c.is_ascii_digit() || "~^>=<".contains(c));
		if !NPM_DIST_TAGS.contains(&rhs) && !versionish && known.contains(rhs) {
			return InstallTarget::Marketplace {
				name:        spec[..at].to_owned(),
				marketplace: rhs.to_owned(),
			};
		}
	}
	InstallTarget::Npm(spec.to_owned())
}

/// `extractPackageName`.
fn extract_package_name(spec: &str) -> String {
	let spec = spec.strip_prefix("npm:").unwrap_or(spec);
	if let Some(rest) = spec.strip_prefix('@') {
		if let Some(slash) = rest.find('/') {
			let tail = &rest[slash + 1..];
			let end = tail.find('@').map_or(tail.len(), |i| i);
			return format!("@{}/{}", &rest[..slash], &tail[..end]);
		}
		return spec.to_owned();
	}
	spec
		.rfind('@')
		.filter(|&i| i > 0)
		.map_or_else(|| spec.to_owned(), |i| spec[..i].to_owned())
}

// =============================================================================
// Installed plugin model
// =============================================================================

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstalledPlugin {
	pub name:             String,
	pub version:          String,
	pub path:             PathBuf,
	pub manifest:         Value,
	pub enabled_features: Option<Vec<String>>,
	pub enabled:          bool,
}

impl InstalledPlugin {
	pub(crate) fn feature_names(&self) -> Vec<String> {
		obj(&self.manifest, "features").keys().cloned().collect()
	}

	pub(crate) fn description(&self) -> Option<&str> {
		self.manifest.get("description").and_then(Value::as_str)
	}
}

fn manifest_of(pkg: &Value) -> Value {
	let mut manifest = ["harvest", "omp", "pi"]
		.iter()
		.find_map(|k| pkg.get(*k).filter(|v| v.is_object()).cloned())
		.unwrap_or_else(|| json!({}));
	manifest["version"] = pkg.get("version").cloned().unwrap_or(Value::Null);
	manifest
}

#[derive(serde::Serialize)]
pub(crate) struct DoctorCheck {
	pub name:    String,
	pub status:  &'static str,
	pub message: String,
	#[serde(skip_serializing_if = "std::ops::Not::not")]
	pub fixed:   bool,
}

pub(crate) struct PluginManager {
	dir:    PathBuf,
	config: Value,
	cwd:    PathBuf,
}

impl PluginManager {
	pub(crate) fn new() -> Result<Self> {
		let dir = plugins_dir();
		let config = read_json_at(&Self::lockfile_in(&dir))?.unwrap_or_else(|| json!({}));
		let mut config = if config.is_object() {
			config
		} else {
			json!({})
		};
		for key in ["plugins", "settings"] {
			if !config.get(key).is_some_and(Value::is_object) {
				config[key] = json!({});
			}
		}
		Ok(Self { dir, config, cwd: std::env::current_dir()? })
	}

	fn lockfile_in(dir: &Path) -> PathBuf {
		let harvest = dir.join("harvest-plugins.lock.json");
		let legacy = dir.join("omp-plugins.lock.json");
		if !harvest.exists() && legacy.exists() {
			legacy
		} else {
			harvest
		}
	}

	fn lockfile_name(&self) -> PathBuf {
		PathBuf::from(Self::lockfile_in(&self.dir).file_name().unwrap_or_default())
	}

	fn node_modules(&self) -> PathBuf {
		self.dir.join("node_modules")
	}

	fn jail(&self) -> Result<JailedFs> {
		JailedFs::open_or_create(&self.dir)
	}

	fn save_config(&self) -> Result<()> {
		let fs = self.jail()?;
		fs.atomic_write(&self.lockfile_name(), serde_json::to_string_pretty(&self.config)?.as_bytes())
	}

	fn project_overrides(&self) -> Value {
		read_json_at(&self.cwd.join(".harvest").join("plugin-overrides.json"))
			.ok()
			.flatten()
			.unwrap_or_else(|| json!({}))
	}

	fn deps(&self) -> Result<Map<String, Value>> {
		Ok(read_json_at(&self.dir.join("package.json"))?
			.map(|v| obj(&v, "dependencies"))
			.unwrap_or_default())
	}

	fn resolve(&self, fallback: &str, path: &Path) -> Result<Option<InstalledPlugin>> {
		let Some(pkg) = read_json_at(&path.join("package.json"))? else {
			return Ok(None);
		};
		let name = pkg
			.get("name")
			.and_then(Value::as_str)
			.filter(|s| !s.is_empty())
			.unwrap_or(fallback)
			.to_owned();
		let version = pkg
			.get("version")
			.and_then(Value::as_str)
			.unwrap_or("")
			.to_owned();
		let state = self.config["plugins"].get(&name);
		let overrides = self.project_overrides();
		let disabled_in_project = overrides
			.get("disabled")
			.and_then(Value::as_array)
			.is_some_and(|a| a.iter().any(|v| v.as_str() == Some(&name)));
		let features = overrides
			.get("features")
			.and_then(|f| f.get(&name))
			.or_else(|| state.and_then(|s| s.get("enabledFeatures")))
			.and_then(Value::as_array)
			.map(|a| {
				a.iter()
					.filter_map(Value::as_str)
					.map(str::to_owned)
					.collect()
			});
		let enabled = state
			.and_then(|s| s.get("enabled"))
			.and_then(Value::as_bool)
			.unwrap_or(true);
		Ok(Some(InstalledPlugin {
			name,
			version,
			path: path.to_path_buf(),
			manifest: manifest_of(&pkg),
			enabled_features: features,
			enabled: enabled && !disabled_in_project,
		}))
	}

	pub(crate) fn get_plugin(
		&self,
		name: &str,
		path: Option<&Path>,
	) -> Result<Option<InstalledPlugin>> {
		if let Some(path) = path {
			return self.resolve(name, path);
		}
		if self.deps()?.contains_key(name) || self.config["plugins"].get(name).is_some() {
			return self.resolve(name, &self.node_modules().join(name));
		}
		Ok(None)
	}

	pub(crate) fn list(&self) -> Result<Vec<InstalledPlugin>> {
		let mut out = Vec::new();
		let mut names: BTreeSet<String> = self.deps()?.keys().cloned().collect();
		// Linked plugins live only in the runtime config.
		names.extend(
			self.config["plugins"]
				.as_object()
				.into_iter()
				.flat_map(|m| m.keys().cloned()),
		);
		for name in names {
			if let Some(plugin) = self.resolve(&name, &self.node_modules().join(&name))? {
				out.push(plugin);
			}
		}
		Ok(out)
	}

	fn ensure_package_json(&self, fs: &JailedFs) -> Result<()> {
		if fs
			.read_optional(Path::new("package.json"), MAX_FILE_BYTES)?
			.is_none()
		{
			let body = json!({"name": "harvest-plugins", "private": true, "dependencies": {}});
			fs.atomic_write(
				Path::new("package.json"),
				serde_json::to_string_pretty(&body)?.as_bytes(),
			)?;
		}
		Ok(())
	}

	fn bun(&self, args: &[&str]) -> Result<()> {
		let bun =
			which("bun").context("bun is required to manage npm plugins but was not found in PATH")?;
		let mut child = Command::new(bun)
			.args(args)
			.current_dir(&self.dir)
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::piped())
			.spawn()?;
		let mut tail = Vec::new();
		if let Some(mut stderr) = child.stderr.take() {
			(&mut stderr)
				.take(CHILD_OUTPUT_LIMIT)
				.read_to_end(&mut tail)?;
			std::io::copy(&mut stderr, &mut std::io::sink())?;
		}
		let status = child.wait()?;
		if !status.success() {
			bail!("bun {} failed: {}", args[0], String::from_utf8_lossy(&tail).trim());
		}
		Ok(())
	}

	pub(crate) fn install(
		&mut self,
		spec: &str,
		force: bool,
		dry_run: bool,
	) -> Result<InstalledPlugin> {
		let (package, features) = parse_plugin_spec(spec);
		if package.is_empty() || package.starts_with('-') || package.contains(['\n', '\r', '\0']) {
			bail!("invalid plugin specifier: {spec}");
		}
		let guessed = extract_package_name(&package);
		if dry_run {
			return Ok(InstalledPlugin {
				name:             guessed.clone(),
				version:          String::new(),
				path:             self.node_modules().join(&guessed),
				manifest:         json!({}),
				enabled_features: None,
				enabled:          true,
			});
		}
		let fs = self.jail()?;
		fs.open_or_create_dir_compat(Path::new("node_modules"))?;
		self.ensure_package_json(&fs)?;
		let before_pkg = fs.read(Path::new("package.json"), MAX_FILE_BYTES)?;
		let before_lock = fs.read_optional(Path::new("bun.lock"), MAX_FILE_BYTES * 4)?;
		let before = self.deps()?;
		let result = (|| -> Result<InstalledPlugin> {
			self.bun(&["install", &package])?;
			let after = self.deps()?;
			let name = if after.contains_key(&guessed) {
				guessed.clone()
			} else {
				after
					.iter()
					.find(|(k, v)| before.get(*k) != Some(*v))
					.map(|(k, _)| k.clone())
					.with_context(|| {
						format!(
							"Installed {package} but could not determine package name from \
							 plugins/package.json"
						)
					})?
			};
			if force && before.contains_key(&name) {
				self.bun(&["update", &name])?;
			}
			let path = self.node_modules().join(&name);
			let mut plugin = self.resolve(&name, &path)?.with_context(|| {
				format!("Package installed but package.json not found at {}", path.display())
			})?;
			let available = plugin.feature_names();
			let chosen = match features {
				Features::Default => self.config["plugins"]
					.get(&name)
					.and_then(|s| s.get("enabledFeatures"))
					.cloned()
					.unwrap_or(Value::Null),
				Features::All => json!(available),
				Features::List(list) => {
					if let Some(bad) = list.iter().find(|f| !available.contains(f)) {
						bail!("Unknown feature \"{bad}\" in {name}. Available: {}", available.join(", "));
					}
					json!(list)
				},
			};
			self.config["plugins"][&name] =
				json!({"version": plugin.version, "enabledFeatures": chosen, "enabled": true});
			self.save_config()?;
			plugin.enabled_features = chosen.as_array().map(|a| {
				a.iter()
					.filter_map(Value::as_str)
					.map(str::to_owned)
					.collect()
			});
			Ok(plugin)
		})();
		if result.is_err() {
			let _ = fs.atomic_write(Path::new("package.json"), &before_pkg);
			match before_lock {
				Some(lock) => {
					let _ = fs.atomic_write(Path::new("bun.lock"), &lock);
				},
				None => {
					let _ = fs.remove(Path::new("bun.lock"));
				},
			}
		}
		result
	}

	pub(crate) fn uninstall(&mut self, name: &str) -> Result<()> {
		let fs = self.jail()?;
		if self.deps()?.contains_key(name) {
			self.bun(&["uninstall", name])?;
		} else {
			// Linked plugin: drop the symlink only, never the linked source.
			remove_link(&fs, &Path::new("node_modules").join(name))?;
		}
		if let Some(map) = self.config["plugins"].as_object_mut() {
			map.remove(name);
		}
		if let Some(map) = self.config["settings"].as_object_mut() {
			map.remove(name);
		}
		self.save_config()
	}

	pub(crate) fn link(&mut self, local: &str) -> Result<InstalledPlugin> {
		let expanded = expand_tilde(local);
		let absolute = self.cwd.join(expanded);
		let absolute = absolute.canonicalize().unwrap_or(absolute);
		let pkg = read_json_at(&absolute.join("package.json"))?
			.with_context(|| format!("package.json not found at {}", absolute.display()))?;
		let name = pkg
			.get("name")
			.and_then(Value::as_str)
			.filter(|s| !s.is_empty())
			.context("package.json must have a name field")?
			.to_owned();
		if name.contains("..") || name.starts_with('/') || name.contains('\\') {
			bail!("invalid package name: {name}");
		}
		let fs = self.jail()?;
		let relative = Path::new("node_modules").join(&name);
		if let Some(parent) = relative.parent() {
			fs.open_or_create_dir_compat(parent)?;
		}
		remove_link(&fs, &relative)?;
		// Jail-resolved destination; the link target is the user-supplied source.
		let link_path = fs.resource_path(&relative)?;
		symlink_dir(&absolute, &link_path)?;
		let version = pkg
			.get("version")
			.and_then(Value::as_str)
			.unwrap_or("")
			.to_owned();
		self.config["plugins"][&name] =
			json!({"version": version, "enabledFeatures": null, "enabled": true});
		self.save_config()?;
		Ok(InstalledPlugin {
			name,
			version,
			path: absolute,
			manifest: manifest_of(&pkg),
			enabled_features: None,
			enabled: true,
		})
	}

	pub(crate) fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<()> {
		let Some(state) = self.config["plugins"].get_mut(name) else {
			bail!("Plugin {name} not found in runtime config");
		};
		state["enabled"] = json!(enabled);
		self.save_config()
	}

	pub(crate) fn enabled_features(&self, name: &str) -> Option<Vec<String>> {
		self.config["plugins"]
			.get(name)?
			.get("enabledFeatures")?
			.as_array()
			.map(|a| {
				a.iter()
					.filter_map(Value::as_str)
					.map(str::to_owned)
					.collect()
			})
	}

	pub(crate) fn set_enabled_features(&mut self, name: &str, features: Vec<String>) -> Result<()> {
		if self.config["plugins"].get(name).is_none() {
			bail!("Plugin {name} not found in runtime config");
		}
		if !features.is_empty()
			&& let Some(plugin) = self.get_plugin(name, Some(&self.node_modules().join(name)))?
		{
			let available = plugin.feature_names();
			if !available.is_empty()
				&& let Some(bad) = features.iter().find(|f| !available.contains(f))
			{
				bail!("Unknown feature \"{bad}\" in {name}. Available: {}", available.join(", "));
			}
		}
		self.config["plugins"][name]["enabledFeatures"] = json!(features);
		self.save_config()
	}

	pub(crate) fn settings(&self, name: &str) -> Map<String, Value> {
		let mut merged = obj(&self.config["settings"], name);
		let project = self.project_overrides();
		if let Some(p) = project
			.get("settings")
			.and_then(|s| s.get(name))
			.and_then(Value::as_object)
		{
			merged.extend(p.clone());
		}
		merged
	}

	pub(crate) fn set_setting(&mut self, name: &str, key: &str, value: Value) -> Result<()> {
		if !self.config["settings"]
			.get(name)
			.is_some_and(Value::is_object)
		{
			self.config["settings"][name] = json!({});
		}
		self.config["settings"][name][key] = value;
		self.save_config()
	}

	pub(crate) fn delete_setting(&mut self, name: &str, key: &str) -> Result<()> {
		if let Some(map) = self.config["settings"]
			.get_mut(name)
			.and_then(Value::as_object_mut)
		{
			map.remove(key);
			self.save_config()?;
		}
		Ok(())
	}

	pub(crate) fn doctor(&mut self, fix: bool) -> Result<Vec<DoctorCheck>> {
		let check = |name: String, status: &'static str, message: String| DoctorCheck {
			name,
			status,
			message,
			fixed: false,
		};
		let mut checks = Vec::new();
		let dir_exists = self.dir.is_dir();
		checks.push(check(
			"plugins_directory".into(),
			if dir_exists { "ok" } else { "warning" },
			if dir_exists {
				format!("Found at {}", self.dir.display())
			} else {
				"Not created yet".into()
			},
		));
		let has_pkg = self.dir.join("package.json").is_file();
		checks.push(check(
			"package_manifest".into(),
			if has_pkg { "ok" } else { "warning" },
			if has_pkg { "Found" } else { "Not created yet" }.into(),
		));
		let has_nm = self.node_modules().is_dir();
		let mut nm = check(
			"node_modules".into(),
			if has_nm {
				"ok"
			} else if has_pkg {
				"error"
			} else {
				"warning"
			},
			if has_nm {
				"Found"
			} else {
				"Missing (run npm install in plugins dir)"
			}
			.into(),
		);
		if !has_nm && has_pkg && fix && self.bun(&["install"]).is_ok() {
			nm.fixed = true;
		}
		checks.push(nm);
		for (name, _) in self.deps()? {
			let path = self.node_modules().join(&name);
			match self.resolve(&name, &path)? {
				None => {
					let mut c =
						check(format!("plugin:{name}"), "error", "Missing from node_modules".into());
					if fix && self.bun(&["install"]).is_ok() {
						c.fixed = true;
					}
					checks.push(c);
				},
				Some(plugin) => {
					let has_manifest = plugin.manifest.as_object().is_some_and(|m| m.len() > 1);
					checks.push(check(
						format!("plugin:{name}"),
						if has_manifest { "ok" } else { "warning" },
						if has_manifest {
							format!("v{}", plugin.version)
						} else {
							"No harvest/omp/pi manifest".into()
						},
					));
					for key in ["tools", "hooks"] {
						if let Some(entry) = plugin.manifest.get(key).and_then(Value::as_str)
							&& !path.join(entry).exists()
						{
							checks.push(check(
								format!("plugin:{name}:{key}"),
								"error",
								format!(
									"{}{} entry \"{entry}\" not found",
									key[..1].to_uppercase(),
									&key[1..]
								),
							));
						}
					}
					for ext in plugin
						.manifest
						.get("extensions")
						.and_then(Value::as_array)
						.into_iter()
						.flatten()
					{
						if let Some(ext) = ext.as_str()
							&& !path.join(ext).exists()
						{
							checks.push(check(
								format!("plugin:{name}:extension:{ext}"),
								"error",
								format!("Extension entry \"{ext}\" not found"),
							));
						}
					}
					let available = plugin.feature_names();
					for feat in plugin.enabled_features.iter().flatten() {
						if !available.contains(feat) {
							checks.push(check(
								format!("plugin:{name}:feature:{feat}"),
								"warning",
								format!("Enabled feature \"{feat}\" not in manifest"),
							));
						}
					}
				},
			}
		}
		let deps = self.deps()?;
		let orphans: Vec<String> = self.config["plugins"]
			.as_object()
			.into_iter()
			.flat_map(|m| m.keys())
			.filter(|n| !deps.contains_key(*n) && !self.node_modules().join(n).exists())
			.cloned()
			.collect();
		for name in orphans {
			let mut c =
				check(format!("orphan:{name}"), "warning", "Plugin in config but not installed".into());
			if fix && let Some(map) = self.config["plugins"].as_object_mut() {
				map.remove(&name);
				c.fixed = true;
			}
			checks.push(c);
		}
		if fix && checks.iter().any(|c| c.fixed) {
			self.save_config()?;
		}
		Ok(checks)
	}
}

/// `parseSettingValue`.
pub(crate) fn parse_setting_value(text: &str, schema: &Value) -> Value {
	match schema.get("type").and_then(Value::as_str) {
		Some("number") => text.trim().parse::<f64>().map_or(Value::Null, |n| json!(n)),
		Some("boolean") => json!(matches!(text, "true" | "yes" | "1")),
		_ => json!(text),
	}
}

/// `validateSetting`.
pub(crate) fn validate_setting(value: &Value, schema: &Value) -> Result<(), String> {
	match schema.get("type").and_then(Value::as_str) {
		Some("string") if !value.is_string() => Err("Expected string".into()),
		Some("number") => {
			let n = value.as_f64().ok_or("Expected number")?;
			if let Some(min) = schema.get("min").and_then(Value::as_f64)
				&& n < min
			{
				return Err(format!("Must be >= {min}"));
			}
			if let Some(max) = schema.get("max").and_then(Value::as_f64)
				&& n > max
			{
				return Err(format!("Must be <= {max}"));
			}
			Ok(())
		},
		Some("boolean") if !value.is_boolean() => Err("Expected boolean".into()),
		Some("enum") => {
			let values: Vec<&str> = schema
				.get("values")
				.and_then(Value::as_array)
				.into_iter()
				.flatten()
				.filter_map(Value::as_str)
				.collect();
			let text = value
				.as_str()
				.map_or_else(|| value.to_string(), str::to_owned);
			if values.contains(&text.as_str()) {
				Ok(())
			} else {
				Err(format!("Must be one of: {}", values.join(", ")))
			}
		},
		_ => Ok(()),
	}
}

// =============================================================================
// Marketplace registry (read + enable/uninstall bookkeeping)
// =============================================================================

#[derive(serde::Serialize)]
pub(crate) struct MarketplacePlugin {
	pub id:      String,
	pub scope:   String,
	pub entries: Vec<Value>,
}

pub(crate) struct Marketplace {
	dir: PathBuf,
}

impl Marketplace {
	pub(crate) fn new() -> Self {
		Self { dir: plugins_dir() }
	}

	pub(crate) fn marketplaces(&self) -> Result<Vec<Value>> {
		Ok(read_json_at(&self.dir.join("marketplaces.json"))?
			.and_then(|v| v.get("marketplaces").and_then(Value::as_array).cloned())
			.unwrap_or_default())
	}

	pub(crate) fn known_names(&self) -> Result<BTreeSet<String>> {
		Ok(self
			.marketplaces()?
			.iter()
			.filter_map(|m| m.get("name").and_then(Value::as_str).map(str::to_owned))
			.collect())
	}

	fn registry(&self) -> Result<Value> {
		Ok(read_json_at(&self.dir.join("installed_plugins.json"))?
			.unwrap_or_else(|| json!({"version": 2, "plugins": {}})))
	}

	pub(crate) fn installed(&self) -> Result<Vec<MarketplacePlugin>> {
		let registry = self.registry()?;
		let mut out = Vec::new();
		for (id, entries) in obj(&registry, "plugins") {
			let entries = entries.as_array().cloned().unwrap_or_default();
			let scope = entries
				.first()
				.and_then(|e| e.get("scope"))
				.and_then(Value::as_str)
				.unwrap_or("user")
				.to_owned();
			out.push(MarketplacePlugin { id, scope, entries });
		}
		Ok(out)
	}

	fn mutate_entries(
		&self,
		id: &str,
		scope: Option<&str>,
		dry_run: bool,
		mut f: impl FnMut(&mut Vec<Value>, Option<&str>) -> Result<()>,
	) -> Result<()> {
		let mut registry = self.registry()?;
		let Some(entries) = registry["plugins"]
			.get_mut(id)
			.and_then(Value::as_array_mut)
		else {
			bail!("Plugin {id} is not installed");
		};
		if let Some(scope) = scope
			&& !entries
				.iter()
				.any(|e| e.get("scope").and_then(Value::as_str) == Some(scope))
		{
			bail!("Plugin {id} is not installed in {scope} scope");
		}
		f(entries, scope)?;
		if entries.is_empty()
			&& let Some(map) = registry["plugins"].as_object_mut()
		{
			map.remove(id);
		}
		if dry_run {
			return Ok(());
		}
		let fs = JailedFs::open_or_create(&self.dir)?;
		fs.atomic_write(
			Path::new("installed_plugins.json"),
			serde_json::to_string_pretty(&registry)?.as_bytes(),
		)
	}

	pub(crate) fn set_enabled(&self, id: &str, enabled: bool, scope: Option<&str>) -> Result<()> {
		self.mutate_entries(id, scope, false, |entries, scope| {
			for entry in entries.iter_mut() {
				if scope.is_none_or(|s| entry.get("scope").and_then(Value::as_str) == Some(s)) {
					entry["enabled"] = json!(enabled);
				}
			}
			Ok(())
		})
	}

	/// Drops the registry entry; cached plugin trees under `cache/plugins`
	/// are left for `harvest gc` to reclaim.
	pub(crate) fn uninstall(&self, id: &str, scope: Option<&str>, dry_run: bool) -> Result<()> {
		self.mutate_entries(id, scope, dry_run, |entries, scope| {
			entries
				.retain(|e| scope.is_some_and(|s| e.get("scope").and_then(Value::as_str) != Some(s)));
			Ok(())
		})
	}
}

// =============================================================================
// Misc helpers
// =============================================================================

pub(crate) fn which(binary: &str) -> Option<PathBuf> {
	if binary.is_empty() || Path::new(binary).components().count() != 1 {
		return None;
	}
	let path = std::env::var_os("PATH")?;
	let mut names = vec![binary.to_owned()];
	if cfg!(windows) && Path::new(binary).extension().is_none() {
		for suffix in std::env::var("PATHEXT")
			.unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
			.split(';')
			.take(32)
		{
			let suffix = suffix.trim();
			if suffix.starts_with('.')
				&& suffix.len() <= 16
				&& suffix[1..].chars().all(|ch| ch.is_ascii_alphanumeric())
			{
				names.push(format!("{binary}{suffix}"));
			}
		}
	}
	for directory in std::env::split_paths(&path) {
		for name in &names {
			let candidate = directory.join(name);
			let Ok(metadata) = candidate.metadata() else {
				continue;
			};
			if !metadata.is_file() {
				continue;
			}
			#[cfg(unix)]
			{
				use std::os::unix::fs::PermissionsExt;
				if metadata.permissions().mode() & 0o111 == 0 {
					continue;
				}
			}
			return Some(candidate);
		}
	}
	None
}

pub(crate) fn expand_tilde(path: &str) -> PathBuf {
	if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
		&& let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
	{
		return PathBuf::from(home).join(rest);
	}
	if path == "~"
		&& let Some(home) = std::env::var_os("HOME")
	{
		return PathBuf::from(home);
	}
	PathBuf::from(path)
}

/// Remove a symlinked plugin entry inside the jail. The parent is resolved
/// through the jail first, so only the final link itself is unlinked.
fn remove_link(fs: &JailedFs, relative: &Path) -> Result<()> {
	let parent = fs.resource_path(relative.parent().context("link has no parent")?)?;
	let leaf = parent.join(relative.file_name().context("link has no name")?);
	match std::fs::symlink_metadata(&leaf) {
		Ok(meta) if meta.file_type().is_symlink() => Ok(std::fs::remove_file(&leaf)?),
		Ok(_) => bail!("refusing to replace non-symlink entry {}", leaf.display()),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(error) => Err(error.into()),
	}
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> Result<()> {
	std::os::unix::fs::symlink(target, link)
		.with_context(|| format!("failed to link {}", link.display()))
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> Result<()> {
	std::os::windows::fs::symlink_dir(target, link)
		.with_context(|| format!("failed to link {}", link.display()))
}

/// Directory creation inside the jail via a placeholder-free path: JailedFs
/// creates parents on write, so materialize directories through
/// `open_or_create` on the resolved child.
trait DirCompat {
	fn open_or_create_dir_compat(&self, relative: &Path) -> Result<()>;
}

impl DirCompat for JailedFs {
	fn open_or_create_dir_compat(&self, relative: &Path) -> Result<()> {
		let resolved = self.resource_path(relative)?;
		JailedFs::open_or_create(&resolved).map(drop)
	}
}
