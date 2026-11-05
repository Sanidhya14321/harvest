//! Version/fingerprint-bound deferred diagnostics and per-session
//! deduplication. The owning Client retains the sole diagnostic payload cache.

use std::{
	collections::{BTreeMap, BTreeSet},
	path::PathBuf,
	time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use harvest_runtime::{JailedFs, MAX_FILE_BYTES, content_hash};
use serde_json::{Value, json};

use super::edits::Range;

const MESSAGE_LIMIT: usize = 50;
const MAX_DIAGNOSTICS: usize = 10_000;

pub(super) fn validate(items: &Value) -> Result<()> {
	let items = items.as_array().context("invalid LSP diagnostics array")?;
	ensure!(items.len() <= MAX_DIAGNOSTICS, "too many LSP diagnostics");
	for item in items {
		let range: Range =
			serde_json::from_value(item["range"].clone()).context("invalid diagnostic range")?;
		ensure!(range.start <= range.end, "reversed diagnostic range");
		ensure!(
			item["message"]
				.as_str()
				.is_some_and(|text| text.len() <= 16 * 1024),
			"invalid diagnostic message"
		);
		if let Some(severity) = item.get("severity") {
			ensure!(
				severity.as_u64().is_some_and(|v| (1..=4).contains(&v)),
				"invalid diagnostic severity"
			);
		}
	}
	Ok(())
}

fn identity(item: &Value) -> String {
	// Location moves do not make the same semantic warning new.
	content_hash(json!({"message":item["message"],"severity":item.get("severity"),"source":item.get("source"),"code":item.get("code")}).to_string().as_bytes())
}

pub(super) fn report(
	uri: &str,
	version: Option<i64>,
	current: Option<bool>,
	items: &[Value],
) -> Value {
	let mut items: Vec<_> = items.iter().collect();
	items.sort_by(|a, b| {
		let key = |item: &Value| {
			(
				item["severity"].as_u64().unwrap_or(1),
				item["range"]["start"]["line"].as_u64().unwrap_or(0),
				item["range"]["start"]["character"].as_u64().unwrap_or(0),
			)
		};
		key(a)
			.cmp(&key(b))
			.then_with(|| a["message"].as_str().cmp(&b["message"].as_str()))
	});
	let errors = items
		.iter()
		.filter(|item| item["severity"].as_u64().unwrap_or(1) == 1)
		.count();
	let warnings = items.iter().filter(|item| item["severity"] == 2).count();
	let total = items.len();
	let mut kept = Vec::new();
	let mut bytes = 0usize;
	for item in items.into_iter().take(MESSAGE_LIMIT) {
		let size = item.to_string().len();
		if bytes + size > harvest_runtime::MAX_TOOL_BYTES / 2 {
			break;
		}
		bytes += size;
		kept.push(item);
	}
	json!({"uri":uri,"available":true,"version":version,"current":current,"items":kept,
		"total":total,"omitted":total-kept.len(),"errors":errors,"warnings":warnings,
		"summary":format!("{errors} error(s), {warnings} warning(s)")})
}

struct Pending {
	path:    PathBuf,
	hash:    String,
	version: i64,
	expires: Instant,
}

#[derive(Default)]
pub(super) struct Deferred {
	pending: BTreeMap<String, Pending>,
	seen:    BTreeMap<String, BTreeSet<String>>,
}
impl Deferred {
	pub fn expect(&mut self, uri: String, path: PathBuf, text: &str, version: i64) {
		if self.pending.contains_key(&uri) || self.pending.len() < 128 {
			self.pending.insert(uri, Pending {
				path,
				hash: content_hash(text.as_bytes()),
				version,
				expires: Instant::now() + Duration::from_secs(12),
			});
		}
	}

	pub fn invalidate(&mut self, uri: &str) {
		self.pending.remove(uri);
	}

	pub fn forget(&mut self, uri: &str) {
		self.pending.remove(uri);
		self.seen.remove(uri);
	}

	pub fn acknowledge(&mut self, uri: &str, items: &[Value]) {
		self.pending.remove(uri);
		let identities = items
			.iter()
			.take(MESSAGE_LIMIT)
			.map(identity)
			.collect::<BTreeSet<_>>();
		if identities.is_empty() {
			self.seen.remove(uri);
		} else {
			if self.seen.len() >= 256 && !self.seen.contains_key(uri) {
				self.seen.pop_first();
			}
			self.seen.insert(uri.to_owned(), identities);
		}
	}

	pub fn clear(&mut self) {
		self.pending.clear();
		self.seen.clear();
	}

	pub fn collect(
		&mut self,
		fs: &JailedFs,
		cache: &BTreeMap<String, Value>,
		max_bytes: usize,
	) -> Result<Vec<Value>> {
		let mut ready = Vec::new();
		let mut finished = Vec::new();
		let mut budget = max_bytes.min(harvest_runtime::MAX_TOOL_BYTES / 2);
		for (uri, pending) in &self.pending {
			if pending.expires <= Instant::now() {
				finished.push(uri.clone());
				continue;
			}
			let Some(bytes) = fs.read_optional(&pending.path, MAX_FILE_BYTES)? else {
				finished.push(uri.clone());
				continue;
			};
			if content_hash(&bytes) != pending.hash {
				finished.push(uri.clone());
				continue;
			}
			let Some(published) = cache.get(uri) else {
				continue;
			};
			// Unversioned pushes remain available to an explicit diagnostics
			// query, but cannot establish freshness for automatic late delivery.
			if published["version"].as_i64() != Some(pending.version) {
				continue;
			}
			let raw = published["diagnostics"]
				.as_array()
				.context("invalid cached diagnostics")?;
			let selected = report(uri, Some(pending.version), Some(true), raw);
			let items = selected["items"]
				.as_array()
				.context("invalid bounded diagnostic selection")?;
			let identities: BTreeSet<_> = items.iter().map(identity).collect();
			let previous = self.seen.get(uri);
			let fresh: Vec<_> = items
				.iter()
				.filter(|item| previous.is_none_or(|seen| !seen.contains(&identity(item))))
				.cloned()
				.collect();
			if !fresh.is_empty() {
				let value = report(uri, Some(pending.version), Some(true), &fresh);
				let size = value.to_string().len();
				if size > budget {
					continue;
				}
				budget -= size;
				ready.push(value);
			}
			if identities.is_empty() {
				self.seen.remove(uri);
			} else {
				if self.seen.len() >= 256 && !self.seen.contains_key(uri) {
					self.seen.pop_first();
				}
				self.seen.insert(uri.clone(), identities);
			}
			finished.push(uri.clone());
		}
		for uri in finished {
			self.pending.remove(&uri);
		}
		Ok(ready)
	}
}
