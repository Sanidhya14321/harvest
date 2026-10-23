//! Layered native configuration and named profiles. Legacy YAML/schema
//! migration remains tracked separately; this module never rewrites existing
//! settings.
use std::{
	collections::{BTreeMap, BTreeSet},
	path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use crate::{
	config::{self, Environment, FileSettings, Overrides, Settings},
	plugins::Configuration,
};

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Api {
	#[default]
	OpenaiChat,
	AnthropicMessages,
}

#[derive(Debug, Default)]
pub struct LoadOptions {
	pub user_directory:      Option<PathBuf>,
	pub profile:             Option<String>,
	pub environment_profile: Option<String>,
	pub overlays:            Vec<PathBuf>,
	pub api:                 Option<Api>,
	pub max_output_tokens:   Option<u32>,
}
#[derive(Debug)]
pub struct Resolved {
	pub settings:          Settings,
	pub api:               Api,
	pub max_output_tokens: u32,
	pub plugins:           Vec<Configuration>,
}

#[derive(Clone, Default, Deserialize)]
struct Layer {
	#[serde(flatten)]
	settings:          FileSettings,
	api:               Option<Api>,
	max_output_tokens: Option<u32>,
	plugins:           Option<Vec<Configuration>>,
}
impl Layer {
	fn merge(&mut self, newer: Self) {
		macro_rules! field {
			($target:expr, $source:expr) => {
				if $source.is_some() {
					$target = $source;
				}
			};
		}
		field!(self.settings.model, newer.settings.model);
		field!(self.settings.base_url, newer.settings.base_url);
		field!(self.settings.api_key, newer.settings.api_key);
		field!(self.settings.max_steps, newer.settings.max_steps);
		field!(self.api, newer.api);
		field!(self.max_output_tokens, newer.max_output_tokens);
		field!(self.plugins, newer.plugins);
	}
}
#[derive(Clone, Default, Deserialize)]
struct Profile {
	extends: Option<String>,
	#[serde(flatten)]
	layer:   Layer,
}
#[derive(Default, Deserialize)]
struct Document {
	#[serde(flatten)]
	layer:    Layer,
	profile:  Option<String>,
	#[serde(default)]
	profiles: BTreeMap<String, Profile>,
}
fn read(root: &Path, path: &Path, required: bool) -> Result<Option<Document>> {
	let filesystem = harvest_runtime::JailedFs::open(root)?;
	let Some(bytes) = filesystem.read_optional(path, 256 * 1024)? else {
		if required {
			bail!("explicit configuration overlay is missing");
		}
		return Ok(None);
	};
	let source = std::str::from_utf8(&bytes).context("configuration is not UTF-8")?;
	// Keep legacy diagnostic identity without retaining TOML source excerpts.
	let label = if path == Path::new(".omp/config.toml") {
		".omp/config.toml"
	} else {
		"native configuration"
	};
	toml::from_str(source)
		.map(Some)
		.map_err(|error: toml::de::Error| {
			if let Some(span) = error.span() {
				anyhow::anyhow!("failed to parse {label} at byte {}", span.start)
			} else {
				anyhow::anyhow!("failed to parse {label}")
			}
		})
}
fn merge_document(target: &mut Document, newer: Document) {
	target.layer.merge(newer.layer);
	if newer.profile.is_some() {
		target.profile = newer.profile;
	}
	for (name, profile) in newer.profiles {
		let previous = target.profiles.entry(name).or_default();
		previous.layer.merge(profile.layer);
		if profile.extends.is_some() {
			previous.extends = profile.extends;
		}
	}
}

/// Precedence: defaults < user < project < explicit overlays < selected profile
/// < environment < flags. Profile inheritance is bounded and cycle-checked.
pub fn load(
	root: &Path,
	flags: &Overrides,
	environment: &Environment,
	options: &LoadOptions,
) -> Result<Resolved> {
	ensure!(options.overlays.len() <= 16, "too many explicit configuration overlays");
	let mut document = Document::default();
	if let Some(user) = &options.user_directory
		&& user.is_dir()
		&& let Some(layer) = read(user, Path::new("config.toml"), false)?
	{
		merge_document(&mut document, layer);
	}
	// Preserve the current Rust config and let the canonical project directory
	// override it when both native files are explicitly present.
	for path in [".omp/config.toml", ".harvest/config.toml"] {
		if let Some(layer) = read(root, Path::new(path), false)? {
			merge_document(&mut document, layer);
		}
	}
	for path in &options.overlays {
		let overlay = if path.is_absolute() {
			let parent = path
				.parent()
				.context("configuration overlay has no parent")?;
			read(
				parent,
				Path::new(
					path
						.file_name()
						.context("configuration overlay has no filename")?,
				),
				true,
			)?
		} else {
			read(root, path, true)?
		};
		merge_document(&mut document, overlay.context("configuration overlay is missing")?);
	}
	let selected = options
		.profile
		.as_ref()
		.or(options.environment_profile.as_ref())
		.or(document.profile.as_ref());
	if let Some(name) = selected {
		let mut visited = BTreeSet::new();
		let mut layers = Vec::new();
		let mut current = Some(name.as_str());
		while let Some(name) = current {
			ensure!(
				visited.len() < 32 && visited.insert(name),
				"configuration profile inheritance cycle or depth limit"
			);
			let profile = document
				.profiles
				.get(name)
				.context("selected configuration profile does not exist")?;
			layers.push(profile.layer.clone());
			current = profile.extends.as_deref();
		}
		for layer in layers.into_iter().rev() {
			document.layer.merge(layer);
		}
	}
	let api = options.api.or(document.layer.api).unwrap_or_default();
	if api == Api::AnthropicMessages {
		ensure!(
			(flags.model.is_some()
				|| environment.model.is_some()
				|| document.layer.settings.model.is_some())
				&& (flags.base_url.is_some()
					|| environment.base_url.is_some()
					|| document.layer.settings.base_url.is_some()),
			"Messages API requires --model and --base-url, or equivalent explicit native \
			 configuration"
		);
	}
	let max_output_tokens = options
		.max_output_tokens
		.or(document.layer.max_output_tokens)
		.unwrap_or(4096);
	ensure!(max_output_tokens > 0, "max output tokens must be at least 1");
	let plugins = document.layer.plugins.unwrap_or_default();
	ensure!(plugins.len() <= 16, "too many configured plugins");
	Ok(Resolved {
		settings: config::resolve_file_values(flags, environment, document.layer.settings)?,
		api,
		max_output_tokens,
		plugins,
	})
}
