//! Native prompt history v1. The legacy SQLite history remains untouched.
//! Host-approved filesystem capabilities own bounded reads, locks and atomic
//! publication; prompts never select a storage root or filename.

use std::{
	collections::BTreeSet,
	path::{Path, PathBuf},
	time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use harvest_runtime::{BoundedWriter, JailedFs};
use harvest_tui::editor::MAX_EDITOR_BYTES;
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
const NAME: &str = "history-native-v1.json";
const MAX_STORE_BYTES: usize = 4 * 1024 * 1024;
const MAX_PROMPT_BYTES: usize = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 1000;
const MAX_METADATA_BYTES: usize = 16 * 1024;
const MAX_QUERY_BYTES: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
	pub id:         u64,
	pub prompt:     String,
	pub created_at: u64,
	pub cwd:        String,
	pub session_id: String,
	used:           u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
	version:       u32,
	next_sequence: u64,
	entries:       Vec<Entry>,
}

impl Default for Document {
	fn default() -> Self {
		Self { version: VERSION, next_sequence: 1, entries: Vec::new() }
	}
}

pub(super) struct History {
	fs:   JailedFs,
	path: PathBuf,
}

impl History {
	pub fn open(workspace: &Path, user_directory: Option<&Path>) -> Result<Self> {
		let (fs, relative) = match user_directory {
			Some(directory) => (JailedFs::open_or_create(directory)?, PathBuf::from(NAME)),
			None => (JailedFs::open(workspace)?, Path::new(".omp").join(NAME)),
		};
		let path = fs.resource_path(&relative)?;
		let history = Self { fs, path };
		// Unknown versions or malformed data are retained for recovery. They
		// cannot be silently replaced by the next successful prompt submission.
		history.read()?;
		Ok(history)
	}

	fn read(&self) -> Result<Document> {
		let Some(bytes) = self.fs.read_optional(&self.path, MAX_STORE_BYTES)? else {
			return Ok(Document::default());
		};
		let mut document: Document = serde_json::from_slice(&bytes)
			.context("native prompt history is invalid; original file preserved")?;
		ensure!(
			document.version == VERSION,
			"unsupported native prompt history version; original file preserved"
		);
		ensure!(
			document.entries.len() <= MAX_ENTRIES,
			"native prompt history exceeds its entry limit"
		);
		let mut ids = BTreeSet::new();
		let mut used = BTreeSet::new();
		let mut prompts = BTreeSet::new();
		let mut total = 0usize;
		for entry in &document.entries {
			ensure!(
				entry.id > 0
					&& entry.used >= entry.id
					&& entry.used < document.next_sequence
					&& ids.insert(entry.id)
					&& used.insert(entry.used),
				"native prompt history has invalid sequence identities"
			);
			ensure!(
				entry.prompt.len() <= MAX_EDITOR_BYTES && !entry.prompt.is_empty(),
				"native prompt history has an oversized or empty prompt"
			);
			ensure!(
				normalize(&entry.prompt).as_deref() == Some(entry.prompt.as_str())
					&& prompts.insert(entry.prompt.as_str()),
				"native prompt history has noncanonical or duplicate prompts"
			);
			ensure!(
				entry.cwd.len() <= MAX_METADATA_BYTES && entry.session_id.len() <= 512,
				"native prompt history metadata exceeds its limit"
			);
			total = total
				.checked_add(entry.prompt.len())
				.context("native prompt history size overflow")?;
		}
		ensure!(
			total <= MAX_PROMPT_BYTES && document.next_sequence > 0,
			"native prompt history exceeds its byte limit"
		);
		document.entries.sort_by_key(|entry| entry.used);
		Ok(document)
	}

	pub fn recent(&self, limit: usize) -> Result<Vec<Entry>> {
		Ok(self
			.read()?
			.entries
			.into_iter()
			.rev()
			.take(limit.min(MAX_ENTRIES))
			.collect())
	}

	/// Token-AND substring search of the bounded native history. Legacy FTS
	/// indexing/import is a separate pending compatibility connection.
	pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Entry>> {
		ensure!(query.len() <= MAX_QUERY_BYTES, "history query exceeds its byte limit");
		let tokens: BTreeSet<_> = query.split_whitespace().map(str::to_lowercase).collect();
		ensure!(tokens.len() <= 64, "history query exceeds its token limit");
		Ok(self
			.read()?
			.entries
			.into_iter()
			.rev()
			.filter(|entry| {
				let text = entry.prompt.to_lowercase();
				tokens.iter().all(|token| text.contains(token))
			})
			.take(limit.min(MAX_ENTRIES))
			.collect())
	}

	pub fn record(&self, prompt: &str, workspace: &Path, session_id: &str) -> Result<()> {
		let Some(prompt) = normalize(prompt) else {
			return Ok(());
		};
		let cwd = workspace.to_string_lossy().into_owned();
		ensure!(
			cwd.len() <= MAX_METADATA_BYTES && session_id.len() <= 512,
			"prompt history metadata exceeds its limit"
		);
		let _ownership = self.fs.resource_lock(&self.path)?;
		let mut document = self.read()?;
		let used = document.next_sequence;
		document.next_sequence = used
			.checked_add(1)
			.context("native prompt history sequence exhausted")?;
		let id = document
			.entries
			.iter()
			.find(|entry| entry.prompt == prompt)
			.map_or(used, |entry| entry.id);
		document.entries.retain(|entry| entry.prompt != prompt);
		document.entries.push(Entry {
			id,
			prompt,
			used,
			cwd,
			session_id: session_id.to_owned(),
			created_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
		});
		let mut total = document
			.entries
			.iter()
			.map(|entry| entry.prompt.len())
			.sum::<usize>();
		while document.entries.len() > MAX_ENTRIES || total > MAX_PROMPT_BYTES {
			total = total.saturating_sub(document.entries.remove(0).prompt.len());
		}
		let mut bytes = BoundedWriter::new(MAX_STORE_BYTES);
		serde_json::to_writer(&mut bytes, &document)?;
		self.fs.atomic_write(&self.path, &bytes.into_inner())
	}
}

fn normalize(prompt: &str) -> Option<String> {
	if prompt.len() > MAX_EDITOR_BYTES {
		return None;
	}
	let normalized = prompt.replace("\r\n", "\n").replace('\r', "\n");
	let normalized = normalized
		.split('\n')
		.map(str::trim_end)
		.collect::<Vec<_>>()
		.join("\n");
	let normalized = normalized.trim();
	(!normalized.is_empty()).then(|| normalized.to_owned())
}
