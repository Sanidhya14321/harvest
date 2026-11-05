//! Native memory-tool adapter. Database paths and scope are selected by the
//! host.
use std::{
	collections::HashSet,
	fmt,
	path::{Path, PathBuf},
	sync::Mutex,
};

use anyhow::{Context, Result, bail};
use harvest_mnemopi::beam::{
	BeamMemory, BeamOptions, consolidate,
	recall::{self, RecallOptions, RecallResult},
	store::{self, RememberOptions},
};
use serde_json::{Value, json};

use super::{MemoryBackend, RetentionCursor, RetentionEpisode};

struct ScopedBank {
	name:   String,
	memory: BeamMemory,
}

pub struct MnemopiBackend {
	banks:        Mutex<Vec<ScopedBank>>,
	session_id:   String,
	recall_limit: usize,
	query_chars:  usize,
}

impl fmt::Debug for MnemopiBackend {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("MnemopiBackend").finish_non_exhaustive()
	}
}

fn hash(text: &str) -> String {
	crate::managed::base36(u128::from(crate::commands::paths::wyhash(text.as_bytes(), 0)))
}

fn limit_bank(name: &str) -> String {
	if name.len() <= 64 {
		return name.to_owned();
	}
	let hash = hash(name);
	let prefix = name[..63 - hash.len()].trim_end_matches('-');
	format!("{}-{hash}", if prefix.is_empty() { "bank" } else { prefix })
}

fn sanitize_bank(name: &str) -> Option<String> {
	let mut clean = String::new();
	for character in name.trim().chars() {
		if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
			clean.push(character);
		} else if !clean.ends_with('-') {
			clean.push('-');
		}
	}
	let clean = clean.trim_matches('-');
	(!clean.is_empty()).then(|| limit_bank(clean))
}

impl MnemopiBackend {
	pub fn from_settings(
		settings: &crate::settings::Settings,
		cwd: &Path,
		session_id: &str,
	) -> Result<Self> {
		Self::from_settings_in(settings, cwd, session_id, None)
	}

	pub fn from_settings_in(
		settings: &crate::settings::Settings,
		cwd: &Path,
		session_id: &str,
		agent_dir: Option<&Path>,
	) -> Result<Self> {
		let cwd = std::path::absolute(cwd)?;
		let configured = settings
			.string("mnemopi.dbPath")
			.filter(|path| !path.trim().is_empty());
		let base_path = match configured {
			Some(path) => PathBuf::from(path),
			None => crate::config::agent_dir_from(agent_dir.map(Path::to_owned), |key| {
				std::env::var_os(key)
			})
			.context("cannot resolve memory agent directory")?
			.join("memories/mnemopi/mnemopi.db"),
		};
		let base_path = if base_path.is_absolute() {
			base_path
		} else {
			cwd.join(base_path)
		};
		let configured_bank = settings
			.string("mnemopi.bank")
			.and_then(|bank| sanitize_bank(&bank));
		let global = configured_bank.clone().unwrap_or_else(|| "default".into());
		let project_name = cwd
			.file_name()
			.and_then(|name| name.to_str())
			.and_then(sanitize_bank)
			.unwrap_or_else(|| "default".into());
		let project = limit_bank(&format!("{project_name}-{}", hash(&cwd.to_string_lossy())));
		let project = match configured_bank {
			Some(base) => limit_bank(&format!("{base}-{project}")),
			None => project,
		};
		let names = match settings.string("mnemopi.scoping").as_deref() {
			Some("global") => vec![global.clone()],
			Some("per-project-tagged") if project != global => vec![project, global.clone()],
			Some("per-project" | "per-project-tagged") | None => vec![project],
			Some(other) => bail!("unsupported Mnemopi scoping {other}"),
		};
		let directory = base_path
			.parent()
			.context("memory database path needs a parent directory")?;
		let mut banks = Vec::new();
		for name in names {
			let path = if name == global {
				base_path.clone()
			} else {
				harvest_mnemopi::banks::bank_db_path(directory, &name)?
			};
			let memory = BeamMemory::open(BeamOptions {
				db_path: Some(path),
				session_id: Some(name.clone()),
				author_id: Some("coding-agent".into()),
				author_type: Some("agent".into()),
				channel_id: Some(name.clone()),
				..Default::default()
			})?;
			// Protect age-eligible memories before the first write can TTL-trim them.
			consolidate::sleep(&memory, false)?;
			banks.push(ScopedBank { name, memory });
		}
		Ok(Self {
			banks:        Mutex::new(banks),
			session_id:   session_id.into(),
			recall_limit: settings
				.number("mnemopi.recallLimit")
				.unwrap_or(8.0)
				.clamp(1.0, 100.0) as usize,
			query_chars:  settings
				.number("mnemopi.recallMaxQueryChars")
				.unwrap_or(4000.0)
				.clamp(256.0, 65536.0) as usize,
		})
	}

	fn lock(&self) -> Result<std::sync::MutexGuard<'_, Vec<ScopedBank>>> {
		self
			.banks
			.lock()
			.map_err(|_| anyhow::anyhow!("memory backend lock was poisoned"))
	}

	fn remember(
		&self,
		bank: &ScopedBank,
		request: &Value,
		content: &str,
		context: Option<&Value>,
	) -> Result<String> {
		if content.trim().is_empty() {
			bail!("memory content must not be empty");
		}
		let metadata = json!({ "session_id": self.session_id, "cwd": request["cwd"], "context": context, "tool": request["tool"] });
		let options = RememberOptions {
			source: request["source"].as_str().map(str::to_owned),
			importance: request["importance"].as_f64(),
			metadata: metadata.as_object().cloned(),
			scope: Some("bank".into()),
			veracity: Some("tool".into()),
			memory_type: Some("fact".into()),
			extract: true,
			extract_entities: true,
			..Default::default()
		};
		Ok(store::remember(&bank.memory, content, &options)?)
	}

	fn recall_results(&self, query: &str) -> Result<Vec<RecallResult>> {
		let query: String = query.chars().take(self.query_chars).collect();
		let banks = self.lock()?;
		let mut results = Vec::new();
		let mut ids = HashSet::new();
		let mut contents = HashSet::new();
		let mut failures = Vec::new();
		let mut successes = 0;
		for bank in banks.iter() {
			match recall::recall(&bank.memory, &query, self.recall_limit, &RecallOptions::default()) {
				Ok(hits) => {
					successes += 1;
					results.extend(hits);
				},
				Err(error) => failures.push(format!("{}: {error}", bank.name)),
			}
		}
		if successes == 0 {
			bail!("Mnemopi recall failed for all scoped targets: {}", failures.join("; "));
		}
		results.sort_by(|a, b| b.score.total_cmp(&a.score));
		results
			.retain(|row| ids.insert(row.id.clone()) && contents.insert(row.content.to_lowercase()));
		results.truncate(self.recall_limit);
		Ok(results)
	}
}

impl MemoryBackend for MnemopiBackend {
	fn configure_llm(
		&self,
		llm: Option<std::sync::Arc<dyn harvest_mnemopi::providers::Llm>>,
	) -> Result<()> {
		let mut banks = self.lock()?;
		for bank in banks.iter_mut() {
			bank.memory.llm = llm.clone();
		}
		Ok(())
	}

	fn id(&self) -> &str {
		"mnemopi"
	}

	fn retention_cursors(&self, session_id: &str) -> Result<Vec<RetentionCursor>> {
		let banks = self.lock()?;
		let bank = banks.first().context("memory has no retain bank")?;
		let mut statement = bank.memory.db.prepare(
			"SELECT value FROM annotations WHERE kind='coding-agent-retention-v1' AND source=? ORDER \
			 BY id DESC LIMIT 1024",
		)?;
		let rows = statement.query_map([session_id], |row| row.get::<_, String>(0))?;
		let mut cursors = Vec::new();
		for row in rows {
			let text = row?;
			if text.len() > 1024 {
				bail!("automatic memory cursor exceeds byte limit");
			}
			let cursor: RetentionCursor = serde_json::from_str(&text)?;
			cursors.push(cursor);
		}
		Ok(cursors)
	}

	fn retain_episode(
		&self,
		episode: &RetentionEpisode,
		cancellation: &harvest_runtime::cancel::CancelToken,
	) -> Result<()> {
		cancellation.heartbeat()?;
		if episode.session_id != self.session_id {
			bail!("memory retention belongs to another session");
		}
		if episode.transcript.is_empty() {
			bail!("automatic retention requires meaningful committed text");
		}
		let banks = self.lock()?;
		let bank = banks.first().context("memory has no retain bank")?;
		let cursor = serde_json::to_string(&episode.cursor)?;
		let metadata = json!({"session_id":episode.session_id,"cwd":episode.cwd,
			"source_id":format!("{}-native-{}",episode.session_id,episode.cursor.digest),
			"retained_through_user_turn":episode.cursor.user_turns,"native_retained_messages":episode.cursor.messages,
			"native_prefix_digest":episode.cursor.digest,"outcome":episode.outcome});
		let options = RememberOptions {
			source: Some("coding-agent-transcript".into()),
			importance: Some(0.65),
			scope: Some("bank".into()),
			veracity: Some("unknown".into()),
			memory_type: Some("episode".into()),
			metadata: metadata.as_object().cloned(),
			embed_text: Some(episode.embed_text.clone()),
			extract_text: episode.extract_text.clone(),
			extract: episode.extract_text.is_some(),
			extract_entities: episode.extract_text.is_some(),
			..Default::default()
		};
		harvest_mnemopi::db::transaction(&bank.memory.db, || {
			let id = store::remember(&bank.memory, &episode.transcript, &options)?;
			if store::get(&bank.memory, &id)?.is_none() {
				return Err(harvest_mnemopi::MnemopiError::Invalid(
					"automatic episode did not remain stored".into(),
				));
			}
			// Existing annotations preserve per-host cursors even when content dedupes
			// against a transcript retained by another session in this bank.
			bank.memory.db.execute(
				"INSERT OR IGNORE INTO annotations(memory_id,kind,value,source,confidence) VALUES \
				 (?,'coding-agent-retention-v1',?,?,1)",
				rusqlite::params![id, cursor, episode.session_id],
			)?;
			bank.memory.db.execute(
				"DELETE FROM annotations WHERE kind='coding-agent-retention-v1' AND source=? AND id \
				 NOT IN (SELECT id FROM annotations WHERE kind='coding-agent-retention-v1' AND \
				 source=? ORDER BY id DESC LIMIT 1024)",
				rusqlite::params![episode.session_id, episode.session_id],
			)?;
			Ok(())
		})?;
		Ok(())
	}

	fn retain(&self, request: Value) -> Result<Value> {
		let items = request["items"]
			.as_array()
			.context("retain items must be an array")?;
		let content: Vec<&str> = items
			.iter()
			.map(|item| {
				item["content"]
					.as_str()
					.filter(|s| !s.trim().is_empty())
					.context("every retained item needs non-empty content")
			})
			.collect::<Result<_>>()?;
		let banks = self.lock()?;
		let bank = banks.first().context("memory has no retain bank")?;
		for (index, text) in content.iter().enumerate() {
			self
				.remember(bank, &request, text, items[index].get("context"))
				.with_context(|| {
					format!("stored {index} of {} memories before retention failed", items.len())
				})?;
		}
		Ok(json!({ "count": items.len() }))
	}

	fn recall(&self, request: Value) -> Result<Value> {
		let hits = self.recall_results(
			request["query"]
				.as_str()
				.context("query must be a string")?,
		)?;
		let text = hits
			.iter()
			.map(|hit| {
				let source = hit
					.source
					.as_deref()
					.map(|source| format!(" [{source}]"))
					.unwrap_or_default();
				let date = hit
					.timestamp
					.as_deref()
					.map(|date| format!(" ({})", date.chars().take(10).collect::<String>()))
					.unwrap_or_default();
				format!("- {} (id: {}){source}{date} c:{:.1}", hit.content, hit.id, hit.score)
			})
			.collect::<Vec<_>>()
			.join("\n\n");
		Ok(json!({ "count": hits.len(), "text": text }))
	}

	fn reflect(&self, request: Value) -> Result<Value> {
		let query = request["query"]
			.as_str()
			.context("query must be a string")?;
		let query = match request["context"]
			.as_str()
			.map(str::trim)
			.filter(|s| !s.is_empty())
		{
			Some(context) => format!("{}\n\nAdditional context:\n{context}", query.trim()),
			None => query.into(),
		};
		let hits = self.recall_results(&query)?;
		Ok(json!({ "text": recall::format_context(&hits) }))
	}

	fn learn(&self, request: Value) -> Result<Value> {
		let content = request["memory"]
			.as_str()
			.context("memory must be a string")?;
		let banks = self.lock()?;
		let bank = banks.first().context("memory has no retain bank")?;
		let id = self.remember(bank, &request, content, request.get("context"))?;
		Ok(json!({ "stored": true, "id": id }))
	}

	fn edit(&self, request: Value) -> Result<Value> {
		let id = request["id"]
			.as_str()
			.context("memory id must be a string")?;
		let op = request["op"]
			.as_str()
			.context("memory op must be a string")?;
		let banks = self.lock()?;
		let mut ineligible = None;
		for bank in banks.iter() {
			let Some(row) = store::get(&bank.memory, id)? else {
				continue;
			};
			let tier = row
				.get("memory_store")
				.and_then(Value::as_str)
				.unwrap_or("working");
			if tier == "fact" {
				ineligible =
					Some(json!({ "status": "not_editable", "bank": bank.name, "store": tier }));
				continue;
			}
			if matches!(op, "update" | "forget") && tier != "working" {
				ineligible
					.get_or_insert(json!({ "status": "not_found", "bank": bank.name, "store": tier }));
				continue;
			}
			let (changed, status) = match op {
				"update" => (
					store::update_working(
						&bank.memory,
						id,
						request["content"].as_str(),
						request["importance"].as_f64(),
					)?,
					"updated",
				),
				"forget" => (store::forget_working(&bank.memory, id)?, "deleted"),
				"invalidate" => (
					store::invalidate(&bank.memory, id, request["replacementId"].as_str())?,
					"invalidated",
				),
				_ => bail!("unsupported memory edit operation"),
			};
			if changed {
				return Ok(json!({ "status": status, "bank": bank.name, "store": tier }));
			}
		}
		Ok(ineligible.unwrap_or_else(|| json!({ "status": "not_found" })))
	}

	fn read(&self, id: &str) -> Result<Option<Value>> {
		let banks = self.lock()?;
		for bank in banks.iter() {
			if let Some(mut row) = store::get(&bank.memory, id)? {
				row.insert("bank".into(), json!(bank.name));
				return Ok(Some(json!(row)));
			}
		}
		Ok(None)
	}
}
