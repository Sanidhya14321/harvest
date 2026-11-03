//! `harvest plugin` — port of `commands/plugin.ts` + `cli/plugin-cli.ts`.

use anyhow::Result;
use clap::{Args as ClapArgs, ValueEnum};
use serde_json::{Value, json};

use super::{
	Paint,
	plugin_store::{
		APP_NAME, InstallTarget, Marketplace, PluginManager, classify_install_target, parse_setting_value,
		split_list, validate_setting,
	},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Action {
	Install,
	Uninstall,
	List,
	Link,
	Doctor,
	Features,
	Config,
	Enable,
	Disable,
	Marketplace,
	Discover,
	Upgrade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Scope {
	User,
	Project,
}

impl Scope {
	pub(crate) fn as_str(self) -> &'static str {
		match self {
			Self::User => "user",
			Self::Project => "project",
		}
	}
}

#[derive(Debug, ClapArgs)]
#[command(about = "Manage plugins (install, uninstall, list, etc.)")]
pub struct PluginArgs {
	/// Plugin action
	#[arg(value_enum)]
	pub action:  Option<Action>,
	/// Packages, paths, or plugin names
	pub targets: Vec<String>,
	/// Output JSON
	#[arg(long)]
	pub json:    bool,
	/// Attempt to fix issues (doctor)
	#[arg(long)]
	pub fix:     bool,
	/// Force install
	#[arg(long)]
	pub force:   bool,
	/// Show actions without applying changes
	#[arg(long = "dry-run")]
	pub dry_run: bool,
	/// Operate on local plugin directory
	#[arg(short = 'l', long)]
	pub local:   bool,
	/// Enable a feature
	#[arg(long)]
	pub enable:  Option<String>,
	/// Disable a feature
	#[arg(long)]
	pub disable: Option<String>,
	/// Set plugin config (key=value)
	#[arg(long)]
	pub set:     Option<String>,
	/// Install scope: "user" (default) or "project"
	#[arg(long, value_enum)]
	pub scope:   Option<Scope>,
}

#[derive(Default)]
pub(crate) struct Flags {
	pub json:    bool,
	pub fix:     bool,
	pub force:   bool,
	pub dry_run: bool,
	pub enable:  Option<String>,
	pub disable: Option<String>,
	pub set:     Option<String>,
	pub scope:   Option<Scope>,
}

pub fn execute(args: PluginArgs) -> Result<u8> {
	let flags = Flags {
		json:    args.json,
		fix:     args.fix,
		force:   args.force,
		dry_run: args.dry_run,
		enable:  args.enable,
		disable: args.disable,
		set:     args.set,
		scope:   args.scope,
	};
	run(args.action.unwrap_or(Action::List), &args.targets, &flags)
}

pub(crate) fn run(action: Action, args: &[String], flags: &Flags) -> Result<u8> {
	let mut manager = PluginManager::new()?;
	match action {
		Action::Install => install(&mut manager, args, flags),
		Action::Uninstall => uninstall(&mut manager, args, flags),
		Action::List => list(&manager, flags),
		Action::Link => link(&mut manager, args, flags),
		Action::Doctor => doctor(&mut manager, flags),
		Action::Features => features(&mut manager, args, flags),
		Action::Config => config(&mut manager, args, flags),
		Action::Enable => set_enabled(&mut manager, args, flags, true),
		Action::Disable => set_enabled(&mut manager, args, flags, false),
		Action::Marketplace => marketplace(args),
		Action::Discover | Action::Upgrade => {
			let p = Paint::stderr();
			eprintln!(
				"{}",
				p.red(&format!(
					"Marketplace {} is not yet available in the native application; use the TypeScript CLI.",
					if action == Action::Discover { "discovery" } else { "upgrades" }
				))
			);
			Ok(1)
		},
	}
}

fn fail(message: String) -> u8 {
	eprintln!("{}", Paint::stderr().red(&message));
	1
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
	println!("{}", serde_json::to_string_pretty(value)?);
	Ok(())
}

fn install(manager: &mut PluginManager, packages: &[String], flags: &Flags) -> Result<u8> {
	let p = Paint::stdout();
	let e = Paint::stderr();
	if packages.is_empty() {
		eprintln!("{}", e.red(&format!("Usage: {APP_NAME} plugin install <source>[features] ...")));
		eprintln!("{}", e.dim("Examples:"));
		for ex in ["@harvest/exa", "name@marketplace", "github:user/repo", "https://github.com/user/repo#v1.0", "./path/to/local/plugin"] {
			eprintln!("{}", e.dim(&format!("  {APP_NAME} plugin install {ex}")));
		}
		return Ok(1);
	}
	let known = Marketplace::new().known_names()?;
	for spec in packages {
		match classify_install_target(spec, &known) {
			InstallTarget::Marketplace { name, marketplace } => {
				return Ok(fail(format!(
					"✘ Failed to install {spec}: marketplace installs ({name} from {marketplace}) are not yet available in the native application"
				)));
			},
			InstallTarget::Local(path) => {
				if flags.scope.is_some() {
					eprintln!("{}", e.yellow(&format!("Warning: --scope is only supported for marketplace installs (name@marketplace). Ignoring for {spec}.")));
				}
				if flags.force {
					eprintln!("{}", e.yellow(&format!("Warning: --force has no effect for local path installs (link is already idempotent). Ignoring for {spec}.")));
				}
				if flags.dry_run {
					if flags.json {
						print_json(&json!({"dryRun": true, "action": "link", "path": path}))?;
					} else {
						println!("{}", p.dim(&format!("[dry-run] Would link {spec}")));
					}
					continue;
				}
				match manager.link(&path) {
					Ok(result) => {
						if flags.json {
							print_json(&result)?;
						} else {
							println!("{}", p.green(&format!("✔ Linked {} from {spec}", result.name)));
							if let Some(d) = result.description() {
								println!("{}", p.dim(&format!("  {d}")));
							}
						}
					},
					Err(err) => return Ok(fail(format!("✘ Failed to install {spec}: {err:#}"))),
				}
			},
			InstallTarget::Npm(spec) => {
				if flags.scope.is_some() {
					eprintln!("{}", e.yellow(&format!("Warning: --scope is only supported for marketplace installs (name@marketplace). Ignoring for {spec}.")));
				}
				match manager.install(&spec, flags.force, flags.dry_run) {
					Ok(result) => {
						if flags.json {
							print_json(&result)?;
						} else if flags.dry_run {
							println!("{}", p.dim(&format!("[dry-run] Would install {spec}")));
						} else {
							println!("{}", p.green(&format!("✔ Installed {}@{}", result.name, result.version)));
							if let Some(f) = result.enabled_features.as_ref().filter(|f| !f.is_empty()) {
								println!("{}", p.dim(&format!("  Features: {}", f.join(", "))));
							}
							if let Some(d) = result.description() {
								println!("{}", p.dim(&format!("  {d}")));
							}
						}
					},
					Err(err) => return Ok(fail(format!("✘ Failed to install {spec}: {err:#}"))),
				}
			},
		}
	}
	Ok(0)
}

fn uninstall(manager: &mut PluginManager, packages: &[String], flags: &Flags) -> Result<u8> {
	if packages.is_empty() {
		return Ok(fail(format!("Usage: {APP_NAME} plugin uninstall <package> ...")));
	}
	let p = Paint::stdout();
	let market = Marketplace::new();
	let installed_ids: Vec<String> = market.installed()?.into_iter().map(|m| m.id).collect();
	let scope = flags.scope.map(Scope::as_str);
	for raw in packages {
		let mut name = raw.clone();
		if !installed_ids.contains(&name) {
			let candidates: Vec<&String> = installed_ids
				.iter()
				.filter(|id| id.rfind('@').is_some_and(|i| &id[..i] == raw))
				.collect();
			match candidates.len() {
				1 => name = candidates[0].clone(),
				0 => {},
				n => {
					let list: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
					return Ok(fail(format!(
						"✘ {raw} is installed from {n} marketplaces. Qualify it: {}",
						list.join(", ")
					)));
				},
			}
		}
		let via_market = installed_ids.contains(&name);
		if flags.dry_run {
			if via_market && let Err(err) = market.uninstall(&name, scope, true) {
				return Ok(fail(format!("✘ Failed to uninstall {name}: {err:#}")));
			}
			if flags.json {
				println!(
					"{}",
					json!({"dryRun": true, "action": "uninstall", "plugin": name, "source": if via_market { "marketplace" } else { "npm" }})
				);
			} else {
				println!("{}", p.dim(&format!("[dry-run] Would uninstall {name}")));
			}
			continue;
		}
		if via_market {
			match market.uninstall(&name, scope, false) {
				Ok(()) => println!("{}", p.green(&format!("✔ Uninstalled {name}"))),
				Err(err) => return Ok(fail(format!("✘ Failed to uninstall {name}: {err:#}"))),
			}
			continue;
		}
		if !manager.list()?.iter().any(|pl| pl.name == name) {
			return Ok(fail(format!("✘ {raw} is not installed")));
		}
		match manager.uninstall(&name) {
			Ok(()) => {
				if flags.json {
					println!("{}", json!({"uninstalled": name}));
				} else {
					println!("{}", p.green(&format!("✔ Uninstalled {name}")));
				}
			},
			Err(err) => return Ok(fail(format!("✘ Failed to uninstall {name}: {err:#}"))),
		}
	}
	Ok(0)
}

fn list(manager: &PluginManager, flags: &Flags) -> Result<u8> {
	let npm = manager.list()?;
	let market = Marketplace::new().installed()?;
	if flags.json {
		print_json(&json!({"npm": npm, "marketplace": market}))?;
		return Ok(0);
	}
	let p = Paint::stdout();
	if npm.is_empty() && market.is_empty() {
		println!("{}", p.dim("No plugins installed"));
		println!("{}", p.dim(&format!("\nInstall plugins with: {APP_NAME} plugin install <package>")));
		return Ok(0);
	}
	if !npm.is_empty() {
		println!("{}", p.bold("npm Plugins:\n"));
		for plugin in &npm {
			let status = if plugin.enabled { p.green("●") } else { p.dim("○") };
			println!("{status} {}@{}", plugin.name, plugin.version);
			if let Some(d) = plugin.description() {
				println!("{}", p.dim(&format!("  {d}")));
			}
			if let Some(f) = plugin.enabled_features.as_ref().filter(|f| !f.is_empty()) {
				println!("{}", p.dim(&format!("  Features: {}", f.join(", "))));
			}
			let available = plugin.feature_names();
			if !available.is_empty() {
				let enabled = plugin.enabled_features.clone().unwrap_or_default();
				let shown: Vec<String> = available
					.iter()
					.map(|f| if enabled.contains(f) { p.green(f) } else { p.dim(f) })
					.collect();
				println!("{}", p.dim(&format!("  Available: [{}]", shown.join(", "))));
			}
		}
	}
	if !market.is_empty() {
		if !npm.is_empty() {
			println!();
		}
		println!("{}", p.bold("Marketplace Plugins:\n"));
		for plugin in &market {
			let version = plugin
				.entries
				.first()
				.and_then(|e| e.get("version"))
				.and_then(Value::as_str)
				.unwrap_or("unknown");
			println!("  {} ({version}){}", plugin.id, p.dim(&format!(" ({})", plugin.scope)));
		}
	}
	Ok(0)
}

fn link(manager: &mut PluginManager, paths: &[String], flags: &Flags) -> Result<u8> {
	let Some(path) = paths.first() else {
		return Ok(fail(format!("Usage: {APP_NAME} plugin link <path>")));
	};
	match manager.link(path) {
		Ok(result) => {
			if flags.json {
				print_json(&result)?;
			} else {
				println!("{}", Paint::stdout().green(&format!("✔ Linked {} from {path}", result.name)));
			}
			Ok(0)
		},
		Err(err) => Ok(fail(format!("✘ Failed to link: {err:#}"))),
	}
}

fn doctor(manager: &mut PluginManager, flags: &Flags) -> Result<u8> {
	let checks = manager.doctor(flags.fix)?;
	if flags.json {
		print_json(&checks)?;
		return Ok(0);
	}
	let p = Paint::stdout();
	println!("{}", p.bold("Plugin Health Check\n"));
	for check in &checks {
		let icon = match check.status {
			"ok" => p.green("✔"),
			"warning" => p.yellow("⚠"),
			_ => p.red("✘"),
		};
		println!("{icon} {}: {}", check.name, check.message);
		if check.fixed {
			println!("{}", p.dim("  ❯ Fixed"));
		}
	}
	let errors = checks.iter().filter(|c| c.status == "error" && !c.fixed).count();
	let warnings = checks.iter().filter(|c| c.status == "warning" && !c.fixed).count();
	let ok = checks.iter().filter(|c| c.status == "ok").count();
	let fixed = checks.iter().filter(|c| c.fixed).count();
	println!();
	let fixed_label = if fixed > 0 { format!(", {fixed} fixed") } else { String::new() };
	println!("Summary: {ok} ok, {warnings} warnings, {errors} errors{fixed_label}");
	if errors > 0 {
		if !flags.fix {
			println!("{}", p.dim("\nRun with --fix to attempt automatic repair"));
		}
		return Ok(1);
	}
	Ok(0)
}

fn features(manager: &mut PluginManager, args: &[String], flags: &Flags) -> Result<u8> {
	let Some(name) = args.first() else {
		return Ok(fail(format!(
			"Usage: {APP_NAME} plugin features <plugin> [--enable f1,f2] [--disable f1] [--set f1,f2]"
		)));
	};
	let path = super::plugin_store::plugins_dir().join("node_modules").join(name);
	let Some(plugin) = manager.get_plugin(name, Some(&path))? else {
		return Ok(fail(format!("Plugin \"{name}\" not found")));
	};
	let p = Paint::stdout();
	if flags.enable.is_some() || flags.disable.is_some() || flags.set.is_some() {
		let mut current: Vec<String> = manager.enabled_features(name).unwrap_or_default();
		if let Some(set) = &flags.set {
			current = split_list(set);
		} else {
			for f in flags.enable.as_deref().map(split_list).unwrap_or_default() {
				if !current.contains(&f) {
					current.push(f);
				}
			}
			let disable = flags.disable.as_deref().map(split_list).unwrap_or_default();
			current.retain(|f| !disable.contains(f));
		}
		if let Err(err) = manager.set_enabled_features(name, current) {
			return Ok(fail(format!("{err:#}")));
		}
		println!("{}", p.green(&format!("✔ Updated features for {name}")));
	}
	let updated = manager.enabled_features(name);
	let available = plugin.feature_names();
	if flags.json {
		print_json(&json!({"plugin": name, "enabledFeatures": updated, "availableFeatures": available}))?;
		return Ok(0);
	}
	println!("{}", p.bold(&format!("Features for {name}:\n")));
	let Some(defs) = plugin.manifest.get("features").and_then(Value::as_object).filter(|m| !m.is_empty()) else {
		println!("{}", p.dim("  No optional features available"));
		return Ok(0);
	};
	let enabled = updated.unwrap_or_default();
	for (feature, def) in defs {
		let icon = if enabled.contains(feature) { p.green("●") } else { p.dim("○") };
		let default_label = if def.get("default").and_then(Value::as_bool) == Some(true) {
			p.dim(" (default)")
		} else {
			String::new()
		};
		println!("{icon} {feature}{default_label}");
		if let Some(d) = def.get("description").and_then(Value::as_str) {
			println!("{}", p.dim(&format!("    {d}")));
		}
	}
	Ok(0)
}

fn display_value(value: Option<&Value>, secret: bool, unset: &str) -> String {
	match value {
		Some(v) if !v.is_null() => {
			if secret {
				"********".into()
			} else {
				v.as_str().map_or_else(|| v.to_string(), str::to_owned)
			}
		},
		_ => unset.to_owned(),
	}
}

fn config(manager: &mut PluginManager, args: &[String], flags: &Flags) -> Result<u8> {
	let Some(sub) = args.first().map(String::as_str) else {
		return Ok(fail(format!(
			"Usage: {APP_NAME} plugin config <list|get|set|delete|validate> <plugin> [key] [value]"
		)));
	};
	if sub == "validate" {
		return config_validate(manager, flags);
	}
	let Some(name) = args.get(1) else { return Ok(fail("Plugin name required".into())) };
	let Some(plugin) = manager.get_plugin(name, None)? else {
		return Ok(fail(format!("Plugin \"{name}\" not found")));
	};
	let schema = plugin.manifest.get("settings").cloned().unwrap_or_else(|| json!({}));
	let key = args.get(2);
	let p = Paint::stdout();
	match sub {
		"list" => {
			let settings = manager.settings(name);
			if flags.json {
				print_json(&json!({"settings": settings, "schema": schema}))?;
				return Ok(0);
			}
			println!("{}", p.bold(&format!("Settings for {name}:\n")));
			let Some(defs) = schema.as_object().filter(|m| !m.is_empty()) else {
				println!("{}", p.dim("  No settings defined"));
				return Ok(0);
			};
			for (k, s) in defs {
				let value = settings.get(k).or_else(|| s.get("default"));
				let secret = s.get("secret").and_then(Value::as_bool) == Some(true);
				println!("  {k}: {}", display_value(value, secret, &p.dim("(not set)")));
				if let Some(d) = s.get("description").and_then(Value::as_str) {
					println!("{}", p.dim(&format!("    {d}")));
				}
				if let Some(env) = s.get("env").and_then(Value::as_str) {
					println!("{}", p.dim(&format!("    env: {env}")));
				}
			}
		},
		"get" => {
			let Some(key) = key else { return Ok(fail("Key required".into())) };
			let settings = manager.settings(name);
			let s = schema.get(key);
			let value = settings.get(key).or_else(|| s.and_then(|s| s.get("default")));
			if flags.json {
				println!("{}", json!({ key: value }));
			} else {
				let secret = s.and_then(|s| s.get("secret")).and_then(Value::as_bool) == Some(true);
				println!("{}", display_value(value, secret, "(not set)"));
			}
		},
		"set" => {
			let Some(key) = key else { return Ok(fail("Key required".into())) };
			let text = args[3..].join(" ");
			let value = match schema.get(key) {
				Some(s) => {
					let value = parse_setting_value(&text, s);
					if let Err(error) = validate_setting(&value, s) {
						return Ok(fail(error));
					}
					value
				},
				None => json!(text),
			};
			manager.set_setting(name, key, value)?;
			println!("{}", p.green(&format!("✔ Set {key}")));
		},
		"delete" => {
			let Some(key) = key else { return Ok(fail("Key required".into())) };
			manager.delete_setting(name, key)?;
			println!("{}", p.green(&format!("✔ Deleted {key}")));
		},
		other => {
			eprintln!("{}", Paint::stderr().red(&format!("Unknown config subcommand: {other}")));
			eprintln!("{}", Paint::stderr().dim("Valid subcommands: list, get, set, delete, validate"));
			return Ok(1);
		},
	}
	Ok(0)
}

fn config_validate(manager: &PluginManager, flags: &Flags) -> Result<u8> {
	let mut errors = Vec::new();
	let mut plugins = manager.list()?;
	for summary in Marketplace::new().installed()? {
		let Some(path) = summary.entries.first().and_then(|e| e.get("installPath")).and_then(Value::as_str) else {
			continue;
		};
		let fallback = summary.id.rfind('@').map_or(summary.id.as_str(), |i| &summary.id[..i]);
		if let Some(resolved) = manager.get_plugin(fallback, Some(std::path::Path::new(path)))?
			&& !plugins.iter().any(|p| p.name == resolved.name)
		{
			plugins.push(resolved);
		}
	}
	for plugin in &plugins {
		let settings = manager.settings(&plugin.name);
		for (key, s) in plugin.manifest.get("settings").and_then(Value::as_object).into_iter().flatten() {
			if let Some(value) = settings.get(key)
				&& let Err(error) = validate_setting(value, s)
			{
				errors.push(json!({"plugin": plugin.name, "key": key, "error": error}));
			}
		}
	}
	if flags.json {
		print_json(&json!({"valid": errors.is_empty(), "errors": errors}))?;
		return Ok(0);
	}
	let p = Paint::stdout();
	if errors.is_empty() {
		println!("{}", p.green("✔ All settings valid"));
		return Ok(0);
	}
	for e in &errors {
		println!(
			"{}",
			p.red(&format!(
				"✘ {}.{}: {}",
				e["plugin"].as_str().unwrap_or(""),
				e["key"].as_str().unwrap_or(""),
				e["error"].as_str().unwrap_or("")
			))
		);
	}
	Ok(1)
}

fn set_enabled(manager: &mut PluginManager, plugins: &[String], flags: &Flags, enabled: bool) -> Result<u8> {
	let (action, past, key) = if enabled { ("enable", "Enabled", "enabled") } else { ("disable", "Disabled", "disabled") };
	if plugins.is_empty() {
		return Ok(fail(format!("Usage: {APP_NAME} plugin {action} <plugin> ...")));
	}
	let market = Marketplace::new();
	let ids: Vec<String> = market.installed()?.into_iter().map(|m| m.id).collect();
	let p = Paint::stdout();
	for name in plugins {
		let result = if ids.contains(name) {
			market.set_enabled(name, enabled, flags.scope.map(Scope::as_str))
		} else {
			manager.set_enabled(name, enabled)
		};
		match result {
			Ok(()) => {
				if flags.json {
					println!("{}", json!({ key: name }));
				} else {
					println!("{}", p.green(&format!("✔ {past} {name}")));
				}
			},
			Err(err) => return Ok(fail(format!("✘ Failed to {action} {name}: {err:#}"))),
		}
	}
	Ok(0)
}

fn marketplace(args: &[String]) -> Result<u8> {
	let sub = args.first().map_or("list", String::as_str);
	if sub != "list" {
		return Ok(fail(format!(
			"marketplace {sub} is not yet available in the native application; use the TypeScript CLI."
		)));
	}
	let p = Paint::stdout();
	let list = Marketplace::new().marketplaces()?;
	if list.is_empty() {
		println!("{}", p.dim("No marketplaces configured"));
		return Ok(0);
	}
	for m in &list {
		println!(
			"  {} {}",
			p.cyan(m.get("name").and_then(Value::as_str).unwrap_or("?")),
			p.dim(m.get("sourceUri").and_then(Value::as_str).unwrap_or(""))
		);
	}
	Ok(0)
}
