//! Migration adapter: discovery, scope filtering, BM25 and graph traversal are
//! owned by harvest-brain. No learned content changes on this read-only path.
use std::{
	collections::{BTreeMap, BTreeSet},
	path::PathBuf,
	sync::{Arc, Mutex},
	time::{Duration, Instant},
};

use anyhow::{Result, bail};
use harvest_brain::{BrainIndex, BrainStore, RetrievalContext, Scope};
use napi_derive::napi;
use serde::{Deserialize, Serialize};

use crate::task;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceConfig {
	directory: PathBuf,
	scope:     Scope,
	jail_root: Option<PathBuf>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
	#[serde(default)]
	scopes:    BTreeSet<Scope>,
	#[serde(default)]
	tags:      BTreeSet<String>,
	max_chars: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Page {
	id:          String,
	document_id: String,
	file_path:   String,
	role:        Scope,
	scope:       Scope,
	title:       String,
	heading:     String,
	content:     String,
	tokens:      Vec<String>,
}

#[derive(Default)]
struct State {
	index:     BrainIndex,
	refreshed: Option<Instant>,
}

#[napi]
pub struct NativeMarkdownBrain {
	roots: Arc<Vec<SourceConfig>>,
	state: Arc<Mutex<State>>,
}

fn napi_error(error: impl std::fmt::Display) -> napi::Error {
	napi::Error::from_reason(error.to_string())
}

impl NativeMarkdownBrain {
	fn refresh_state(roots: &[SourceConfig], state: &mut State) -> Result<()> {
		let mut groups: BTreeMap<(Scope, PathBuf), Vec<PathBuf>> = BTreeMap::new();
		for root in roots {
			let jail = root.jail_root.as_ref().unwrap_or(&root.directory);
			match std::fs::symlink_metadata(jail) {
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
				Err(error) => return Err(error.into()),
				Ok(metadata) if metadata.file_type().is_symlink() => {
					bail!("brain jail root is a symlink")
				},
				Ok(_) => {},
			}
			groups
				.entry((root.scope, jail.clone()))
				.or_default()
				.push(root.directory.clone());
		}
		let mut stores = Vec::new();
		for ((scope, jail), sources) in groups {
			let mut store = match scope {
				Scope::Project => BrainStore::project(&jail)?,
				Scope::User => BrainStore::user(&jail)?,
			};
			store.select_sources(sources)?;
			stores.push(store);
		}
		let mut index = BrainIndex::default();
		index.refresh(&stores.iter().collect::<Vec<_>>())?;
		state.index = index;
		state.refreshed = Some(Instant::now());
		Ok(())
	}
}

#[napi]
impl NativeMarkdownBrain {
	#[napi(constructor)]
	pub fn new(roots_json: String) -> napi::Result<Self> {
		if roots_json.len() > 64 * 1024 {
			return Err(napi_error("brain roots exceed input limit"));
		}
		let roots: Vec<SourceConfig> = serde_json::from_str(&roots_json).map_err(napi_error)?;
		if roots.len() > 32 {
			return Err(napi_error("too many brain roots"));
		}
		Ok(Self { roots: Arc::new(roots), state: Arc::new(Mutex::new(State::default())) })
	}

	#[napi]
	pub fn refresh(&self) -> task::Promise<()> {
		let roots = self.roots.clone();
		let state = self.state.clone();
		task::blocking("brain.refresh", (), move |_| {
			let mut state = state.lock().map_err(napi_error)?;
			Self::refresh_state(&roots, &mut state).map_err(napi_error)
		})
	}

	#[napi]
	pub fn retrieve(&self, query: String, options_json: String) -> task::Promise<String> {
		let roots = self.roots.clone();
		let state = self.state.clone();
		task::blocking("brain.retrieve", (), move |_| {
			if query.len() > 16 * 1024 || options_json.len() > 64 * 1024 {
				return Err(napi_error("brain request exceeds input limit"));
			}
			let options: Options = serde_json::from_str(&options_json).map_err(napi_error)?;
			let mut state = state.lock().map_err(napi_error)?;
			if state
				.refreshed
				.is_none_or(|time| time.elapsed() > Duration::from_secs(2))
				|| state
					.index
					.documents()
					.iter()
					.any(|document| document.metadata.ownership == harvest_brain::Ownership::Generated)
			{
				Self::refresh_state(&roots, &mut state).map_err(napi_error)?;
			}
			let pages = state.index.retrieve(&query, &RetrievalContext {
				scopes:    options.scopes,
				tags:      options.tags,
				max_chars: options.max_chars,
			});
			let pages: Vec<_> = pages
				.into_iter()
				.map(|page| Page {
					id:          format!("{}#{}", page.id, page.section_index),
					tokens:      harvest_brain::tokenize(&format!(
						"{} {} {}",
						page.title, page.heading, page.content
					)),
					document_id: page.id,
					file_path:   page.path,
					role:        page.scope,
					scope:       page.scope,
					title:       page.title,
					heading:     page.heading,
					content:     page.content,
				})
				.collect();
			serde_json::to_string(&pages).map_err(napi_error)
		})
	}
}
