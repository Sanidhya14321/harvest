//! Bundled provider/model routing for `provider/model` selectors.
//!
//! Generated from `packages/catalog/src/models.json` by
//! `bun run gen:rust-catalog`; only the wires the Rust runtime speaks
//! (`OpenAI` chat completions, `Anthropic` Messages) are listed.
use std::{collections::BTreeMap, sync::LazyLock};

use serde::Deserialize;

/// The request protocol a catalog model speaks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Wire {
	/// `POST {base}/chat/completions`.
	#[default]
	OpenAi,
	/// `POST {base}/v1/messages`.
	Anthropic,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
	pub base_url:       String,
	#[serde(default)]
	pub wire:           Wire,
	pub context_window: Option<u64>,
	pub max_tokens:     Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogProvider {
	pub env_vars: Vec<String>,
	pub models:   BTreeMap<String, CatalogModel>,
}

static CATALOG: LazyLock<BTreeMap<String, CatalogProvider>> = LazyLock::new(|| {
	serde_json::from_str(include_str!("catalog.json")).expect("generated model catalog")
});

/// Every routable `(provider, model id, row)`, sorted.
pub fn models() -> impl Iterator<Item = (&'static str, &'static str, &'static CatalogModel)> {
	CATALOG.iter().flat_map(|(provider, entry)| {
		entry
			.models
			.iter()
			.map(move |(id, model)| (provider.as_str(), id.as_str(), model))
	})
}

/// Every env var a catalog provider may read its key from.
pub fn env_vars() -> impl Iterator<Item = &'static str> {
	CATALOG
		.values()
		.flat_map(|provider| provider.env_vars.iter().map(String::as_str))
}

/// A resolved catalog selector.
#[derive(Debug, PartialEq, Eq)]
pub struct Route {
	pub provider:   &'static str,
	pub model:      &'static str,
	pub base_url:   &'static str,
	pub wire:       Wire,
	pub max_tokens: Option<u64>,
	pub env_vars:   &'static [String],
}

/// `parseModelString` + catalog lookup for a `provider/id` selector.
///
/// Splits at the first `/` (ids such as `openai/gpt-5.5` on `OpenRouter` keep
/// their slash). A trailing `:level` thinking suffix is dropped when the bare
/// id is listed.
pub fn route(selector: &str) -> Option<Route> {
	let (provider, id) = selector.split_once('/')?;
	if provider.is_empty() {
		return None;
	}
	let (provider, entry) = CATALOG.get_key_value(provider)?;
	let lookup = |id: &str| entry.models.get_key_value(id);
	let (model, row) =
		lookup(id).or_else(|| id.rsplit_once(':').and_then(|(base, _)| lookup(base)))?;
	Some(Route {
		provider,
		model,
		base_url: &row.base_url,
		wire: row.wire,
		max_tokens: row.max_tokens,
		env_vars: &entry.env_vars,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn selectors_route_to_the_provider_endpoint_and_key() {
		let groq = CATALOG.get("groq").expect("groq is bundled");
		let (id, row) = groq.models.iter().next().expect("groq has models");
		let route = route(&format!("groq/{id}")).expect("listed model routes");
		assert_eq!((route.model, route.base_url), (id.as_str(), row.base_url.as_str()));
		assert_eq!(route.env_vars, ["GROQ_API_KEY".to_owned()]);
		let suffixed = super::route(&format!("groq/{id}:high")).expect("thinking suffix");
		assert_eq!(suffixed.model, id);
		assert_eq!(route.wire, Wire::OpenAi);
		let claude = super::route("anthropic/claude-sonnet-4-5").expect("anthropic model");
		assert_eq!((claude.wire, claude.base_url), (Wire::Anthropic, "https://api.anthropic.com"));
		assert!(claude.max_tokens.is_some());
		assert!(super::route("groq/not-a-model").is_none());
		assert!(super::route("bare-model").is_none());
		assert!(super::route("/x").is_none());
	}
}

/// Environment variables a catalog provider reads its API key from.
pub fn provider_env_vars(provider: &str) -> &'static [String] {
	CATALOG.get(provider).map_or(&[], |entry| &entry.env_vars)
}
