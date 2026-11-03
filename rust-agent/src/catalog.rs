//! Bundled provider/model routing for `provider/model` selectors.
//!
//! Generated from `packages/catalog/src/models.json` by
//! `bun run gen:rust-catalog`; only OpenAI-compatible chat models are listed
//! because that is the wire the Rust runtime speaks.
use std::{collections::BTreeMap, sync::LazyLock};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
	pub base_url:       String,
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

/// Every env var a catalog provider may read its key from.
pub fn env_vars() -> impl Iterator<Item = &'static str> {
	CATALOG
		.values()
		.flat_map(|provider| provider.env_vars.iter().map(String::as_str))
}

/// A resolved catalog selector.
#[derive(Debug, PartialEq, Eq)]
pub struct Route {
	pub provider: &'static str,
	pub model:    &'static str,
	pub base_url: &'static str,
	pub env_vars: &'static [String],
}

/// `parseModelString` + catalog lookup for a `provider/id` selector.
///
/// Splits at the first `/` (ids such as `openai/gpt-5.5` on OpenRouter keep their slash). A
/// trailing `:level` thinking suffix is dropped when the bare id is listed.
pub fn route(selector: &str) -> Option<Route> {
	let (provider, id) = selector.split_once('/')?;
	if provider.is_empty() {
		return None;
	}
	let (provider, entry) = CATALOG.get_key_value(provider)?;
	let lookup = |id: &str| entry.models.get_key_value(id);
	let (model, row) =
		lookup(id).or_else(|| id.rsplit_once(':').and_then(|(base, _)| lookup(base)))?;
	Some(Route { provider, model, base_url: &row.base_url, env_vars: &entry.env_vars })
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
		assert!(super::route("groq/not-a-model").is_none());
		assert!(super::route("bare-model").is_none());
		assert!(super::route("/x").is_none());
	}
}
