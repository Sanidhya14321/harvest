//! Instance-owned facade over the shared BEAM engines; no process singleton.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

pub use crate::beam::{
	recall::{RecallOptions, RecallResult},
	store::{RememberBatchItem, RememberBatchOptions, RememberOptions},
};
use crate::{
	Result,
	beam::{BeamMemory, BeamOptions, consolidate, recall, store},
	util::Row,
};

/// A host-selected bank/database and session. Hosts serialize access to an
/// instance with a mutex when exposing it through a Send + Sync tool backend.
pub struct Mnemopi {
	beam: BeamMemory,
}

#[derive(Debug, Serialize)]
pub struct MemoryStats {
	pub total_memories: i64,
	pub total_sessions: i64,
	pub sources:        BTreeMap<String, i64>,
	pub last_memory:    Option<String>,
	pub database:       String,
	pub mode:           &'static str,
	pub beam:           Value,
}

impl Mnemopi {
	pub fn open(options: BeamOptions) -> Result<Self> {
		Ok(Self { beam: BeamMemory::open(options)? })
	}

	pub fn beam(&self) -> &BeamMemory {
		&self.beam
	}

	pub fn remember(&self, content: &str, options: &RememberOptions) -> Result<String> {
		store::remember(&self.beam, content, options)
	}

	pub fn remember_batch(
		&self,
		items: &[RememberBatchItem],
		options: &RememberBatchOptions,
	) -> Result<Vec<String>> {
		store::remember_batch(&self.beam, items, options)
	}

	pub fn recall(
		&self,
		query: &str,
		top_k: usize,
		options: &RecallOptions,
	) -> Result<Vec<RecallResult>> {
		recall::recall(&self.beam, query, top_k, options)
	}

	pub fn get(&self, memory_id: &str) -> Result<Option<Row>> {
		store::get(&self.beam, memory_id)
	}

	pub fn update(
		&self,
		memory_id: &str,
		content: Option<&str>,
		importance: Option<f64>,
	) -> Result<bool> {
		store::update_working(&self.beam, memory_id, content, importance)
	}

	pub fn forget(&self, memory_id: &str) -> Result<bool> {
		store::forget_working(&self.beam, memory_id)
	}

	pub fn invalidate(&self, memory_id: &str, replacement_id: Option<&str>) -> Result<bool> {
		store::invalidate(&self.beam, memory_id, replacement_id)
	}

	pub fn get_context(&self, limit: i64) -> Result<Vec<Row>> {
		store::get_context(&self.beam, limit)
	}

	pub fn scratchpad_write(&self, content: &str) -> Result<String> {
		store::scratchpad_write(&self.beam, content)
	}

	pub fn scratchpad_read(&self) -> Result<Vec<Row>> {
		store::scratchpad_read(&self.beam)
	}

	pub fn scratchpad_clear(&self) -> Result<()> {
		store::scratchpad_clear(&self.beam)
	}

	pub fn sleep(&self, dry_run: bool) -> Result<Value> {
		consolidate::sleep(&self.beam, dry_run)
	}

	pub fn sleep_all_sessions(&mut self, dry_run: bool) -> Result<Value> {
		consolidate::sleep_all_sessions(&mut self.beam, dry_run)
	}

	pub fn export(&self) -> Result<Value> {
		store::export_to_dict(&self.beam)
	}

	pub fn import(&self, data: &Value, force: bool) -> Result<store::ImportStats> {
		store::import_from_dict(&self.beam, data, force)
	}

	pub fn stats(&self) -> Result<MemoryStats> {
		let count = |sql: &str| self.beam.db.query_row(sql, [], |r| r.get::<_, i64>(0));
		let mut statement = self.beam.db.prepare(
			"SELECT COALESCE(source, 'unknown'), COUNT(*) FROM working_memory GROUP BY source ORDER \
			 BY source LIMIT 1000",
		)?;
		let sources = statement
			.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
			.collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
		let last_memory: Option<String> =
			self
				.beam
				.db
				.query_row("SELECT MAX(timestamp) FROM working_memory", [], |r| r.get(0))?;
		Ok(MemoryStats {
			total_memories: count("SELECT COUNT(*) FROM working_memory")?,
			total_sessions: count("SELECT COUNT(DISTINCT session_id) FROM working_memory")?,
			sources,
			last_memory,
			database: self
				.beam
				.db_path
				.as_ref()
				.map_or_else(|| ":memory:".into(), |p| p.display().to_string()),
			mode: "beam",
			beam: json!({
				"working_memory": store::get_working_stats(&self.beam, None, None, None)?,
				"episodic_memory": store::get_episodic_stats(&self.beam, None, None, None)?,
				"triples": { "total": count("SELECT COUNT(*) FROM triples")? },
			}),
		})
	}
}
