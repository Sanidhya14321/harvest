//! Working-memory write path (`core/beam/store.ts`).

use std::collections::HashSet;

use chrono::Utc;
use rusqlite::{OptionalExtension, params_from_iter, types::Value as SqlValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{BeamMemory, consolidate};
use crate::{
	Result, config,
	db::transaction,
	extraction,
	limits::{
		self, MAX_BATCH_ITEMS, MAX_CONTENT_BYTES, MAX_METADATA_BYTES, MAX_RESULT_BYTES, MAX_ROWS,
	},
	schema::table_exists,
	util::{Row, generate_id, iso, json_to_sql, now_iso, placeholders, row_to_map},
};

const CANONICAL_VERACITY: &[&str] =
	&["true", "false", "stated", "inferred", "tool", "imported", "unknown"];
const TRUST_TIERS: &[&str] = &["STATED", "DERIVED", "EXTERNAL_WRITE", "IMPORTED"];
const MEMORIA_SOURCE_TABLES: &[&str] = &[
	"memoria_facts",
	"memoria_instructions",
	"memoria_kg",
	"memoria_preferences",
	"memoria_timelines",
];
const EMBED_REBUILD_BATCH: usize = 128;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RememberOptions {
	pub source:           Option<String>,
	pub importance:       Option<f64>,
	pub timestamp:        Option<String>,
	pub scope:            Option<String>,
	pub veracity:         Option<String>,
	pub trust_tier:       Option<String>,
	pub memory_type:      Option<String>,
	pub valid_until:      Option<String>,
	pub author_id:        Option<String>,
	pub author_type:      Option<String>,
	pub channel_id:       Option<String>,
	pub metadata:         Option<Map<String, Value>>,
	pub memory_id:        Option<String>,
	/// Text embedded/indexed instead of `content` (stored in `embed_text`).
	pub embed_text:       Option<String>,
	/// Text mined for facts instead of `content`.
	pub extract_text:     Option<String>,
	#[serde(default)]
	pub extract_entities: bool,
	/// Run fact extraction (synchronously, via the host `Llm` or heuristics).
	#[serde(default)]
	pub extract:          bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RememberBatchItem {
	pub content:     String,
	pub source:      Option<String>,
	pub importance:  Option<f64>,
	pub timestamp:   Option<String>,
	pub scope:       Option<String>,
	pub veracity:    Option<String>,
	pub memory_type: Option<String>,
	pub metadata:    Option<Map<String, Value>>,
	pub embed_text:  Option<String>,
	#[serde(default)]
	pub extract:     bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RememberBatchOptions {
	pub veracity:       Option<String>,
	pub scope:          Option<String>,
	pub trust_tier:     Option<String>,
	pub memory_type:    Option<String>,
	#[serde(default)]
	pub force_veracity: bool,
	#[serde(default)]
	pub extract:        bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
	pub total: i64,
	pub count: i64,
	pub last:  Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TierImportStats {
	pub inserted:            i64,
	pub skipped:             i64,
	pub overwritten:         i64,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub embeddings_inserted: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScratchImportStats {
	pub inserted: i64,
	pub updated:  i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogImportStats {
	pub inserted: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportStats {
	pub working_memory:    TierImportStats,
	pub episodic_memory:   TierImportStats,
	pub scratchpad:        ScratchImportStats,
	pub consolidation_log: LogImportStats,
}

pub fn clamp_veracity(value: Option<&str>) -> String {
	let n = value.unwrap_or("").trim().to_lowercase();
	if CANONICAL_VERACITY.contains(&n.as_str()) {
		n
	} else {
		"unknown".into()
	}
}

fn source_to_trust_tier(source: &str) -> &'static str {
	match source.to_lowercase().as_str() {
		"tool" | "api" | "system" => "EXTERNAL_WRITE",
		"import" | "imported" | "backup" => "IMPORTED",
		_ => "STATED",
	}
}

fn normalize_trust_tier(value: Option<&str>, source: &str) -> String {
	match value {
		None => source_to_trust_tier(source).into(),
		Some(v) if TRUST_TIERS.contains(&v) => v.into(),
		Some(_) => "STATED".into(),
	}
}

fn metadata_json(m: Option<&Map<String, Value>>) -> Option<String> {
	m.map(|m| Value::Object(m.clone()).to_string())
}

pub(crate) fn query_rows(beam: &BeamMemory, sql: &str, params: &[SqlValue]) -> Result<Vec<Row>> {
	let mut stmt = beam.db.prepare(sql)?;
	let mut rows = Vec::new();
	let mut remaining = MAX_RESULT_BYTES;
	for row in stmt.query_map(params_from_iter(params.iter()), row_to_map)? {
		if rows.len() == MAX_ROWS {
			return Err(crate::MnemopiError::Invalid("query exceeds row limit".into()));
		}
		let row = row?;
		remaining -= limits::serialized_size(&row, remaining)?;
		rows.push(row);
	}
	Ok(rows)
}

fn text(s: &str) -> SqlValue {
	SqlValue::Text(s.to_string())
}

fn opt_text(s: Option<&str>) -> SqlValue {
	s.map_or(SqlValue::Null, text)
}

/// Delete every artifact linked to working-memory ids (no orphans).
pub(crate) fn purge_working_memory_artifacts(beam: &BeamMemory, ids: &[String]) -> Result<()> {
	if ids.is_empty() {
		return Ok(());
	}
	let ph = placeholders(ids.len());
	let id_params: Vec<SqlValue> = ids.iter().map(|i| text(i)).collect();
	let mut graph_refs: Vec<String> = ids.to_vec();
	graph_refs.extend(ids.iter().map(|i| format!("gist_{i}")));
	if table_exists(&beam.db, "facts") {
		for row in query_rows(
			beam,
			&format!("SELECT fact_id FROM facts WHERE source_msg_id IN ({ph})"),
			&id_params,
		)? {
			if let Some(Value::String(f)) = row.get("fact_id") {
				graph_refs.push(f.clone());
			}
		}
		beam.db.execute(
			&format!("DELETE FROM facts WHERE source_msg_id IN ({ph})"),
			params_from_iter(id_params.iter()),
		)?;
	}
	beam.db.execute(
		&format!("DELETE FROM annotations WHERE memory_id IN ({ph})"),
		params_from_iter(id_params.iter()),
	)?;
	beam.db.execute(
		&format!("DELETE FROM memory_embeddings WHERE memory_id IN ({ph})"),
		params_from_iter(id_params.iter()),
	)?;
	for table in MEMORIA_SOURCE_TABLES {
		beam.db.execute(
			&format!("DELETE FROM {table} WHERE source_memory_id IN ({ph})"),
			params_from_iter(id_params.iter()),
		)?;
	}
	if table_exists(&beam.db, "gists") {
		beam.db.execute(
			&format!("DELETE FROM gists WHERE memory_id IN ({ph})"),
			params_from_iter(id_params.iter()),
		)?;
	}
	if table_exists(&beam.db, "graph_edges") {
		let mut seen = HashSet::new();
		graph_refs.retain(|r| seen.insert(r.clone()));
		let rph = placeholders(graph_refs.len());
		let mut p: Vec<SqlValue> = graph_refs.iter().map(|r| text(r)).collect();
		p.extend(graph_refs.iter().map(|r| text(r)));
		beam.db.execute(
			&format!("DELETE FROM graph_edges WHERE source IN ({rph}) OR target IN ({rph})"),
			params_from_iter(p.iter()),
		)?;
	}
	Ok(())
}

fn trim_working_memory(beam: &BeamMemory) -> Result<()> {
	let limit = beam.config.working_memory_limit;
	if limit <= 0 {
		return Ok(());
	}
	let cutoff = iso(Utc::now() - chrono::Duration::hours(beam.config.working_memory_ttl_hours));
	transaction(&beam.db, || {
		let ids: Vec<String> = {
			let mut stmt = beam.db.prepare(
				"SELECT id FROM working_memory
				 WHERE session_id = ?
				   AND consolidated_at IS NULL
				   AND trust_tier IS NOT 'IMPORTED'
				   AND (
					timestamp < ? OR
					id NOT IN (
						SELECT id FROM working_memory
						WHERE session_id = ? AND consolidated_at IS NULL AND trust_tier IS NOT 'IMPORTED'
						ORDER BY timestamp DESC
						LIMIT ?
					)
				   )",
			)?;
			stmt
				.query_map((&beam.session_id, &cutoff, &beam.session_id, limit), |r| r.get(0))?
				.collect::<rusqlite::Result<_>>()?
		};
		if ids.is_empty() {
			return Ok(());
		}
		let mut p: Vec<SqlValue> = ids.iter().map(|i| text(i)).collect();
		p.push(text(&beam.session_id));
		beam.db.execute(
			&format!(
				"DELETE FROM working_memory WHERE id IN ({}) AND session_id = ?",
				placeholders(ids.len())
			),
			params_from_iter(p.iter()),
		)?;
		purge_working_memory_artifacts(beam, &ids)
	})
}

fn add_annotation(beam: &BeamMemory, memory_id: &str, kind: &str, value: &str) {
	let _ = beam.db.execute(
		"INSERT OR IGNORE INTO annotations (memory_id, kind, value, source, confidence) VALUES (?, \
		 ?, ?, ?, ?)",
		(memory_id, kind, value, "", 1.0),
	);
}

fn add_temporal_annotations(beam: &BeamMemory, memory_id: &str, timestamp: &str, source: &str) {
	add_annotation(beam, memory_id, "occurred_on", &crate::util::prefix_chars(timestamp, 10));
	if !source.is_empty() && !matches!(source, "conversation" | "user" | "assistant") {
		add_annotation(beam, memory_id, "has_source", source);
	}
}

/// Embed `(memory_id, text)` pairs through the host embedder (best-effort).
pub(crate) fn run_embedding(beam: &BeamMemory, items: &[(String, String)]) {
	if config::embeddings_disabled() {
		return;
	}
	let Some(embedder) = beam.embedder.as_ref() else {
		return;
	};
	let max_chars = config::embedding_max_input_chars();
	let items: Vec<&(String, String)> = items.iter().filter(|(_, c)| !c.trim().is_empty()).collect();
	if items.is_empty() {
		return;
	}
	let texts: Vec<String> = items
		.iter()
		.map(|(_, c)| {
			if max_chars > 0 {
				crate::util::prefix_chars(c, max_chars)
			} else {
				c.clone()
			}
		})
		.collect();
	let Ok(matrix) = embedder.embed(&texts) else {
		return;
	};
	if matrix.len() != items.len()
		|| matrix.iter().any(|v| {
			v.is_empty() || v.len() > limits::MAX_VECTOR_DIMENSIONS || v.iter().any(|n| !n.is_finite())
		}) {
		return;
	}
	let model = embedder.model();
	let _ = transaction(&beam.db, || {
		for ((id, _), vector) in items.iter().zip(matrix.iter()) {
			let encoded = Value::Array(vector.iter().map(|v| json!(*v as f64)).collect()).to_string();
			beam.db.execute(
				"INSERT OR REPLACE INTO memory_embeddings(memory_id, embedding_json, model) VALUES \
				 (?, ?, ?)",
				(id, &encoded, &model),
			)?;
		}
		Ok(())
	});
}

/// Wipe and rebuild stored vectors when the active embedding model changed.
pub fn reconcile_embedding_model(beam: &BeamMemory) -> Result<()> {
	if config::embeddings_disabled() {
		return Ok(());
	}
	let Some(embedder) = beam.embedder.as_ref() else {
		return Ok(());
	};
	let active = embedder.model().trim().to_string();
	if active.is_empty() {
		return Ok(());
	}
	let live_sql = "SELECT id AS memoryId, COALESCE(embed_text, content) AS content FROM \
	                working_memory WHERE superseded_by IS NULL
		UNION ALL SELECT id AS memoryId, content FROM episodic_memory WHERE superseded_by IS NULL";
	let mismatch: bool = beam
		.db
		.query_row("SELECT 1 FROM memory_embeddings WHERE model IS NOT ? LIMIT 1", [&active], |_| {
			Ok(())
		})
		.optional()?
		.is_some();
	let to_embed: Vec<(String, String)> = if mismatch {
		let live = collect_pairs(beam, live_sql, &[])?;
		transaction(&beam.db, || {
			beam.db.execute_batch(
				"DELETE FROM memory_embeddings; UPDATE episodic_memory SET binary_vector = NULL;",
			)?;
			Ok(())
		})?;
		live
	} else {
		collect_pairs(
			beam,
			"SELECT id AS memoryId, COALESCE(embed_text, content) AS content FROM working_memory
			 WHERE superseded_by IS NULL AND id NOT IN (SELECT memory_id FROM memory_embeddings WHERE model \
			 = ?)
			 UNION ALL
			 SELECT id AS memoryId, content FROM episodic_memory
			 WHERE superseded_by IS NULL AND id NOT IN (SELECT memory_id FROM memory_embeddings WHERE model \
			 = ?)",
			&[text(&active), text(&active)],
		)?
	};
	for chunk in to_embed.chunks(EMBED_REBUILD_BATCH) {
		run_embedding(beam, chunk);
	}
	Ok(())
}

fn collect_pairs(
	beam: &BeamMemory,
	sql: &str,
	params: &[SqlValue],
) -> Result<Vec<(String, String)>> {
	Ok(query_rows(beam, sql, params)?
		.into_iter()
		.filter_map(|row| {
			Some((crate::util::rstr(&row, "memoryId")?, crate::util::rstr(&row, "content")?))
		})
		.collect())
}

fn run_fact_extraction(beam: &BeamMemory, memory_id: &str, content: &str) {
	if content.trim().is_empty() {
		return;
	}
	let extracted =
		extraction::extract_fact_categories(beam.llm.as_deref(), content, config::llm_max_tokens());
	if extracted.count() == 0 {
		return;
	}
	let _ = consolidate::store_extracted_fact_categories(beam, &extracted, 0, Some(memory_id), 0.7);
}

/// Store one memory; returns its id (an existing id when `content` is a
/// duplicate in this session).
pub fn remember(beam: &BeamMemory, content: &str, o: &RememberOptions) -> Result<String> {
	limits::check_text(content, MAX_CONTENT_BYTES, "memory content")?;
	limits::importance(o.importance)?;
	limits::serialized_size(o, MAX_CONTENT_BYTES + MAX_METADATA_BYTES)?;
	if let Some(meta) = &o.metadata {
		limits::serialized_size(meta, MAX_METADATA_BYTES)?;
	}
	let source = o.source.clone().unwrap_or_else(|| "conversation".into());
	let importance = o.importance.unwrap_or(0.5);
	let timestamp = o.timestamp.clone().unwrap_or_else(now_iso);
	let scope = o.scope.clone().unwrap_or_else(|| "session".into());
	let veracity = clamp_veracity(o.veracity.as_deref());
	let trust_tier = normalize_trust_tier(o.trust_tier.as_deref(), &source);
	let memory_type = o.memory_type.clone().unwrap_or_else(|| "unknown".into());
	let author_id = o.author_id.clone().or_else(|| beam.author_id.clone());
	let author_type = o.author_type.clone().or_else(|| beam.author_type.clone());
	let channel_id = o
		.channel_id
		.clone()
		.unwrap_or_else(|| beam.channel_id.clone());
	let embed_text = o.embed_text.clone().unwrap_or_else(|| content.to_string());
	let stored_embed = (embed_text != content).then(|| embed_text.clone());

	let existing: Option<String> = beam
		.db
		.query_row(
			"SELECT id FROM working_memory WHERE content = ? AND session_id = ? LIMIT 1",
			(content, &beam.session_id),
			|r| r.get(0),
		)
		.optional()?;
	if let Some(existing_id) = existing {
		beam.db.execute(
			"UPDATE working_memory
			 SET importance = MAX(importance, ?), timestamp = ?, source = ?,
				valid_until = COALESCE(?, valid_until),
				scope = COALESCE(?, scope),
				author_id = COALESCE(?, author_id),
				author_type = COALESCE(?, author_type),
				channel_id = COALESCE(?, channel_id),
				memory_type = COALESCE(?, memory_type),
				veracity = CASE WHEN ? != 'unknown' THEN ? ELSE veracity END,
				trust_tier = COALESCE(?, trust_tier),
				embed_text = COALESCE(?, embed_text),
				consolidated_at = NULL
			 WHERE id = ? AND session_id = ?",
			rusqlite::params![
				importance,
				timestamp,
				source,
				o.valid_until,
				scope,
				author_id,
				author_type,
				channel_id,
				memory_type,
				veracity,
				veracity,
				trust_tier,
				stored_embed,
				existing_id,
				beam.session_id
			],
		)?;
		if embed_text != content {
			run_embedding(beam, &[(existing_id.clone(), embed_text)]);
		}
		return Ok(existing_id);
	}

	let parsed_ts = crate::util::parse_ts(&timestamp).unwrap_or_else(Utc::now);
	let memory_id = o
		.memory_id
		.clone()
		.unwrap_or_else(|| generate_id(content, parsed_ts));
	beam.db.execute(
		"INSERT INTO working_memory
		 (id, content, embed_text, source, timestamp, session_id, importance, metadata_json, \
		 valid_until, scope,
		  author_id, author_type, channel_id, veracity, memory_type, trust_tier)
		 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
		rusqlite::params![
			memory_id,
			content,
			stored_embed,
			source,
			timestamp,
			beam.session_id,
			importance,
			metadata_json(o.metadata.as_ref()),
			o.valid_until,
			scope,
			author_id,
			author_type,
			channel_id,
			veracity,
			memory_type,
			trust_tier
		],
	)?;
	add_temporal_annotations(beam, &memory_id, &timestamp, &source);
	trim_working_memory(beam)?;
	run_embedding(beam, &[(memory_id.clone(), embed_text)]);
	if o.extract {
		let src = o
			.extract_text
			.clone()
			.unwrap_or_else(|| content.to_string());
		run_fact_extraction(beam, &memory_id, &src);
	}
	Ok(memory_id)
}

pub fn remember_batch(
	beam: &BeamMemory,
	items: &[RememberBatchItem],
	o: &RememberBatchOptions,
) -> Result<Vec<String>> {
	if items.len() > MAX_BATCH_ITEMS {
		return Err(crate::MnemopiError::Invalid("batch exceeds item limit".into()));
	}
	limits::serialized_size(&items, MAX_RESULT_BYTES)?;
	for item in items {
		limits::check_text(&item.content, MAX_CONTENT_BYTES, "memory content")?;
		limits::importance(item.importance)?;
		if let Some(meta) = &item.metadata {
			limits::serialized_size(meta, MAX_METADATA_BYTES)?;
		}
	}
	let timestamp = now_iso();
	let default_veracity = clamp_veracity(o.veracity.as_deref());
	let default_scope = o.scope.clone().unwrap_or_else(|| "session".into());
	let trust_tier =
		normalize_trust_tier(Some(o.trust_tier.as_deref().unwrap_or("IMPORTED")), "imported");
	let mut ids = Vec::with_capacity(items.len());
	transaction(&beam.db, || {
		let mut stmt = beam.db.prepare(
			"INSERT INTO working_memory
			 (id, content, embed_text, source, timestamp, session_id, importance, metadata_json,
			  author_id, author_type, channel_id, memory_type, veracity, trust_tier, scope)
			 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
		)?;
		for item in items {
			let ts = item.timestamp.clone().unwrap_or_else(|| timestamp.clone());
			let id = generate_id(&item.content, crate::util::parse_ts(&ts).unwrap_or_else(Utc::now));
			let source = item.source.clone().unwrap_or_else(|| "conversation".into());
			let embed = item.embed_text.clone().filter(|e| e != &item.content);
			let veracity = if o.force_veracity {
				default_veracity.clone()
			} else {
				item
					.veracity
					.as_deref()
					.map_or_else(|| default_veracity.clone(), |v| clamp_veracity(Some(v)))
			};
			stmt.execute(rusqlite::params![
				id,
				item.content,
				embed,
				source,
				ts,
				beam.session_id,
				item.importance.unwrap_or(0.5),
				metadata_json(item.metadata.as_ref()),
				beam.author_id,
				beam.author_type,
				beam.channel_id,
				item
					.memory_type
					.clone()
					.or_else(|| o.memory_type.clone())
					.unwrap_or_else(|| "unknown".into()),
				veracity,
				trust_tier,
				item.scope.clone().unwrap_or_else(|| default_scope.clone())
			])?;
			add_temporal_annotations(beam, &id, &ts, &source);
			ids.push(id);
		}
		drop(stmt);
		trim_working_memory(beam)
	})?;
	let pairs: Vec<(String, String)> = items
		.iter()
		.zip(ids.iter())
		.map(|(i, id)| (id.clone(), i.embed_text.clone().unwrap_or_else(|| i.content.clone())))
		.collect();
	run_embedding(beam, &pairs);
	for (item, id) in items.iter().zip(ids.iter()) {
		if item.extract || o.extract {
			run_fact_extraction(beam, id, &item.content);
		}
	}
	Ok(ids)
}

pub fn get_context(beam: &BeamMemory, limit: i64) -> Result<Vec<Row>> {
	if !(0..=MAX_ROWS as i64).contains(&limit) {
		return Err(crate::MnemopiError::Invalid("context limit out of bounds".into()));
	}
	query_rows(
		beam,
		"SELECT id, content, source, timestamp, importance, scope
		 FROM working_memory
		 WHERE (session_id = ? OR scope = 'global')
		   AND (valid_until IS NULL OR valid_until > ?)
		   AND superseded_by IS NULL
		 ORDER BY CASE WHEN scope = 'global' THEN 0 ELSE 1 END, importance DESC, timestamp DESC
		 LIMIT ?",
		&[text(&beam.session_id), text(&now_iso()), SqlValue::Integer(limit)],
	)
}

pub fn invalidate(
	beam: &BeamMemory,
	memory_id: &str,
	replacement_id: Option<&str>,
) -> Result<bool> {
	limits::check_text(memory_id, 1024, "memory id")?;
	if let Some(id) = replacement_id {
		limits::check_text(id, 1024, "replacement id")?;
	}
	let now = now_iso();
	for table in ["working_memory", "episodic_memory"] {
		let changed = beam.db.execute(
			&format!(
				"UPDATE {table} SET valid_until = ?, superseded_by = ? WHERE id = ? AND (session_id = \
				 ? OR scope = 'global')"
			),
			(&now, replacement_id, memory_id, &beam.session_id),
		)?;
		if changed > 0 {
			return Ok(true);
		}
	}
	Ok(false)
}

fn filtered_stats(
	beam: &BeamMemory,
	table: &str,
	author_id: Option<&str>,
	author_type: Option<&str>,
	channel_id: Option<&str>,
) -> Result<Stats> {
	let mut clauses = Vec::new();
	let mut params = Vec::new();
	for (col, v) in
		[("author_id", author_id), ("author_type", author_type), ("channel_id", channel_id)]
	{
		if let Some(v) = v.filter(|v| !v.is_empty()) {
			clauses.push(format!("{col} = ?"));
			params.push(text(v));
		}
	}
	let where_ = if clauses.is_empty() {
		String::new()
	} else {
		format!(" WHERE {}", clauses.join(" AND "))
	};
	let total: i64 = beam.db.query_row(
		&format!("SELECT COUNT(*) FROM {table}{where_}"),
		params_from_iter(params.iter()),
		|r| r.get(0),
	)?;
	let last: Option<String> = beam
		.db
		.query_row(
			&format!("SELECT timestamp FROM {table}{where_} ORDER BY timestamp DESC LIMIT 1"),
			params_from_iter(params.iter()),
			|r| r.get(0),
		)
		.optional()?
		.flatten();
	Ok(Stats { total, count: total, last })
}

pub fn get_working_stats(
	beam: &BeamMemory,
	author_id: Option<&str>,
	author_type: Option<&str>,
	channel_id: Option<&str>,
) -> Result<Stats> {
	filtered_stats(beam, "working_memory", author_id, author_type, channel_id)
}

pub fn get_episodic_stats(
	beam: &BeamMemory,
	author_id: Option<&str>,
	author_type: Option<&str>,
	channel_id: Option<&str>,
) -> Result<Stats> {
	filtered_stats(beam, "episodic_memory", author_id, author_type, channel_id)
}

pub fn update_working(
	beam: &BeamMemory,
	memory_id: &str,
	content: Option<&str>,
	importance: Option<f64>,
) -> Result<bool> {
	limits::check_text(memory_id, 1024, "memory id")?;
	if let Some(content) = content {
		limits::check_text(content, MAX_CONTENT_BYTES, "memory content")?;
	}
	limits::importance(importance)?;
	let mut assignments = Vec::new();
	let mut params = Vec::new();
	if let Some(c) = content {
		assignments.push("content = ?, embed_text = NULL");
		params.push(text(c));
	}
	if let Some(i) = importance {
		assignments.push("importance = ?");
		params.push(SqlValue::Real(i));
	}
	if assignments.is_empty() {
		return Ok(false);
	}
	params.push(text(memory_id));
	params.push(text(&beam.session_id));
	let changed = beam.db.execute(
		&format!(
			"UPDATE working_memory SET {} WHERE id = ? AND session_id = ?",
			assignments.join(", ")
		),
		params_from_iter(params.iter()),
	)?;
	if changed > 0 {
		if let Some(c) = content {
			run_embedding(beam, &[(memory_id.to_string(), c.to_string())]);
		}
	}
	Ok(changed > 0)
}

pub fn get(beam: &BeamMemory, memory_id: &str) -> Result<Option<Row>> {
	limits::check_text(memory_id, 1024, "memory id")?;
	let cols =
		"id, content, source, timestamp, session_id, importance, metadata_json, veracity, created_at";
	let working = query_rows(
		beam,
		&format!(
			"SELECT {cols} FROM working_memory WHERE id = ? AND (session_id = ? OR scope = 'global')"
		),
		&[text(memory_id), text(&beam.session_id)],
	)?;
	if let Some(mut row) = working.into_iter().next() {
		let meta = row.get("metadata_json").cloned().unwrap_or(Value::Null);
		row.insert("metadata".into(), meta);
		row.insert("memory_store".into(), json!("working"));
		return Ok(Some(row));
	}
	let episodic = query_rows(
		beam,
		&format!(
			"SELECT {cols} FROM episodic_memory WHERE id = ? AND (session_id = ? OR scope = 'global')"
		),
		&[text(memory_id), text(&beam.session_id)],
	)?;
	if let Some(mut row) = episodic.into_iter().next() {
		let meta = row.get("metadata_json").cloned().unwrap_or(Value::Null);
		row.insert("metadata".into(), meta);
		row.insert("memory_store".into(), json!("episodic"));
		return Ok(Some(row));
	}
	get_fact(beam, memory_id)
}

fn get_fact(beam: &BeamMemory, memory_id: &str) -> Result<Option<Row>> {
	let Some(fact) = query_rows(beam, "SELECT * FROM facts WHERE fact_id = ?", &[text(memory_id)])?
		.into_iter()
		.next()
	else {
		return Ok(None);
	};
	let same_session =
		fact.get("session_id").and_then(Value::as_str) == Some(beam.session_id.as_str());
	if !same_session && fact.get("scope").and_then(Value::as_str) != Some("global") {
		return Ok(None);
	}
	let s = |k: &str| {
		fact
			.get(k)
			.and_then(Value::as_str)
			.unwrap_or("")
			.to_string()
	};
	let (subject, predicate, object) = (s("subject"), s("predicate"), s("object"));
	let content = [subject.as_str(), predicate.as_str(), object.as_str()]
		.into_iter()
		.filter(|p| !p.is_empty())
		.collect::<Vec<_>>()
		.join(" ");
	let null = Value::Null;
	let mut out = Map::new();
	out.insert("id".into(), fact.get("fact_id").cloned().unwrap_or(Value::Null));
	out.insert("content".into(), json!(content));
	out.insert("source".into(), json!("facts"));
	out.insert("timestamp".into(), fact.get("timestamp").cloned().unwrap_or(Value::Null));
	out.insert("session_id".into(), fact.get("session_id").cloned().unwrap_or(Value::Null));
	out.insert("importance".into(), fact.get("confidence").cloned().unwrap_or(Value::Null));
	out.insert(
		"metadata".into(),
		json!(
			json!({
				"subject": subject, "predicate": predicate, "object": object,
				"source_msg_id": fact.get("source_msg_id").unwrap_or(&null),
			})
			.to_string()
		),
	);
	out.insert("created_at".into(), fact.get("created_at").cloned().unwrap_or(Value::Null));
	out.insert("memory_store".into(), json!("fact"));
	Ok(Some(out))
}

pub fn forget_working(beam: &BeamMemory, memory_id: &str) -> Result<bool> {
	limits::check_text(memory_id, 1024, "memory id")?;
	transaction(&beam.db, || {
		let deleted = beam.db.execute(
			"DELETE FROM working_memory WHERE id = ? AND session_id = ?",
			(memory_id, &beam.session_id),
		)?;
		if deleted > 0 {
			purge_working_memory_artifacts(beam, &[memory_id.to_string()])?;
		}
		Ok(deleted > 0)
	})
}

pub fn scratchpad_write(beam: &BeamMemory, content: &str) -> Result<String> {
	limits::check_text(content, MAX_CONTENT_BYTES, "scratchpad content")?;
	let id = generate_id(content, Utc::now());
	let ts = now_iso();
	beam.db.execute(
		"INSERT INTO scratchpad (id, content, session_id, created_at, updated_at) VALUES (?, ?, ?, \
		 ?, ?)
		 ON CONFLICT(id) DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
		(&id, content, &beam.session_id, &ts, &ts),
	)?;
	Ok(id)
}

pub fn scratchpad_read(beam: &BeamMemory) -> Result<Vec<Row>> {
	query_rows(
		beam,
		"SELECT id, content, created_at, updated_at FROM scratchpad WHERE session_id = ? ORDER BY \
		 updated_at DESC LIMIT ?",
		&[
			text(&beam.session_id),
			SqlValue::Integer(config::scratchpad_max_items().clamp(0, MAX_ROWS as i64)),
		],
	)
}

pub fn scratchpad_clear(beam: &BeamMemory) -> Result<()> {
	beam
		.db
		.execute("DELETE FROM scratchpad WHERE session_id = ?", [&beam.session_id])?;
	Ok(())
}

pub fn export_to_dict(beam: &BeamMemory) -> Result<Value> {
	let working = query_rows(
		beam,
		"SELECT id, content, source, timestamp, session_id, importance, embed_text,
			metadata_json, valid_until, superseded_by, scope,
			recall_count, last_recalled, created_at, veracity, consolidated_at,
			memory_type, author_id, author_type, channel_id, trust_tier,
			event_date, event_date_precision, temporal_tags
		 FROM working_memory ORDER BY session_id, timestamp",
		&[],
	)?;
	let episodic = query_rows(
		beam,
		"SELECT rowid, id, content, source, timestamp, session_id, importance,
			metadata_json, summary_of, valid_until, superseded_by, scope,
			recall_count, last_recalled, created_at, veracity, memory_type,
			author_id, author_type, channel_id, trust_tier,
			event_date, event_date_precision, temporal_tags
		 FROM episodic_memory ORDER BY session_id, timestamp",
		&[],
	)?;
	let scratch = query_rows(
		beam,
		"SELECT id, content, session_id, created_at, updated_at FROM scratchpad ORDER BY \
		 session_id, updated_at",
		&[],
	)?;
	let log = query_rows(
		beam,
		"SELECT id, session_id, items_consolidated, summary_preview, created_at FROM \
		 consolidation_log ORDER BY session_id, created_at",
		&[],
	)?;
	Ok(json!({
		"mnemopi_export": {
			"version": "1.0",
			"export_date": now_iso(),
			"source_db": beam.db_path.as_ref().map_or_else(|| ":memory:".to_string(), |p| p.display().to_string()),
			"component": "beam",
		},
		"working_memory": working,
		"episodic_memory": episodic,
		"episodic_embeddings": [],
		"scratchpad": scratch,
		"consolidation_log": log,
	}))
}

fn rows_of<'a>(data: &'a Value, key: &str) -> impl Iterator<Item = &'a Map<String, Value>> {
	data
		.get(key)
		.and_then(Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(Value::as_object)
}

fn row_exists(beam: &BeamMemory, table: &str, id: &str) -> Result<bool> {
	Ok(beam
		.db
		.query_row(&format!("SELECT 1 FROM {table} WHERE id = ?"), [id], |_| Ok(()))
		.optional()?
		.is_some())
}

pub fn import_from_dict(beam: &BeamMemory, data: &Value, force: bool) -> Result<ImportStats> {
	limits::serialized_size(data, MAX_RESULT_BYTES)?;
	for key in ["working_memory", "episodic_memory", "scratchpad", "consolidation_log"] {
		if data
			.get(key)
			.and_then(Value::as_array)
			.is_some_and(|rows| rows.len() > MAX_BATCH_ITEMS)
		{
			return Err(crate::MnemopiError::Invalid("import exceeds item limit".into()));
		}
	}
	let mut stats = ImportStats::default();
	stats.episodic_memory.embeddings_inserted = Some(0);
	let imported_at = now_iso();
	let g = |item: &Map<String, Value>, k: &str, fb: SqlValue| json_to_sql(item.get(k), fb);
	let t = |s: &str| SqlValue::Text(s.into());
	transaction(&beam.db, || {
		for item in rows_of(data, "working_memory") {
			let id = item
				.get("id")
				.map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string))
				.unwrap_or_default();
			if id.is_empty() {
				continue;
			}
			let exists = row_exists(beam, "working_memory", &id)?;
			if exists && !force {
				stats.working_memory.skipped += 1;
				continue;
			}
			if exists {
				beam
					.db
					.execute("DELETE FROM working_memory WHERE id = ?", [&id])?;
				purge_working_memory_artifacts(beam, std::slice::from_ref(&id))?;
				stats.working_memory.overwritten += 1;
			} else {
				stats.working_memory.inserted += 1;
			}
			let consolidated = match item.get("consolidated_at") {
				None | Some(Value::Null) => t(&imported_at),
				v => json_to_sql(v, t(&imported_at)),
			};
			let p = vec![
				t(&id),
				g(item, "content", t("")),
				g(item, "source", SqlValue::Null),
				g(item, "timestamp", SqlValue::Null),
				g(item, "session_id", t("default")),
				g(item, "importance", SqlValue::Real(0.5)),
				g(item, "metadata_json", t("{}")),
				g(item, "valid_until", SqlValue::Null),
				g(item, "superseded_by", SqlValue::Null),
				g(item, "scope", t("session")),
				g(item, "recall_count", SqlValue::Integer(0)),
				g(item, "last_recalled", SqlValue::Null),
				g(item, "created_at", SqlValue::Null),
				t(&clamp_veracity(item.get("veracity").and_then(Value::as_str))),
				consolidated,
				g(item, "memory_type", t("unknown")),
				g(item, "embed_text", SqlValue::Null),
				g(item, "author_id", SqlValue::Null),
				g(item, "author_type", SqlValue::Null),
				g(item, "channel_id", SqlValue::Null),
				g(item, "trust_tier", t("STATED")),
				g(item, "event_date", SqlValue::Null),
				g(item, "event_date_precision", t("unknown")),
				g(item, "temporal_tags", t("[]")),
			];
			beam.db.execute(
				&format!(
					"INSERT INTO working_memory
					 (id, content, source, timestamp, session_id, importance, metadata_json,
					  valid_until, superseded_by, scope, recall_count, last_recalled, created_at,
					  veracity, consolidated_at, memory_type, embed_text, author_id, author_type, channel_id,
					  trust_tier, event_date, event_date_precision, temporal_tags)
					 VALUES ({})",
					placeholders(24)
				),
				params_from_iter(p.iter()),
			)?;
		}

		for item in rows_of(data, "episodic_memory") {
			let id = item
				.get("id")
				.map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string))
				.unwrap_or_default();
			if id.is_empty() {
				continue;
			}
			let exists = row_exists(beam, "episodic_memory", &id)?;
			if exists && !force {
				stats.episodic_memory.skipped += 1;
				continue;
			}
			if exists {
				beam
					.db
					.execute("DELETE FROM episodic_memory WHERE id = ?", [&id])?;
				stats.episodic_memory.overwritten += 1;
			} else {
				stats.episodic_memory.inserted += 1;
			}
			let p = vec![
				t(&id),
				g(item, "content", t("")),
				g(item, "source", SqlValue::Null),
				g(item, "timestamp", SqlValue::Null),
				g(item, "session_id", t("default")),
				g(item, "importance", SqlValue::Real(0.5)),
				g(item, "metadata_json", t("{}")),
				g(item, "summary_of", t("")),
				g(item, "valid_until", SqlValue::Null),
				g(item, "superseded_by", SqlValue::Null),
				g(item, "scope", t("session")),
				g(item, "recall_count", SqlValue::Integer(0)),
				g(item, "last_recalled", SqlValue::Null),
				g(item, "created_at", SqlValue::Null),
				t(&clamp_veracity(item.get("veracity").and_then(Value::as_str))),
				g(item, "memory_type", t("unknown")),
				g(item, "author_id", SqlValue::Null),
				g(item, "author_type", SqlValue::Null),
				g(item, "channel_id", SqlValue::Null),
				g(item, "trust_tier", t("STATED")),
				g(item, "event_date", SqlValue::Null),
				g(item, "event_date_precision", t("unknown")),
				g(item, "temporal_tags", t("[]")),
			];
			beam.db.execute(
				&format!(
					"INSERT INTO episodic_memory
					 (id, content, source, timestamp, session_id, importance, metadata_json,
					  summary_of, valid_until, superseded_by, scope, recall_count, last_recalled, created_at,
					  veracity, memory_type, author_id, author_type, channel_id, trust_tier,
					  event_date, event_date_precision, temporal_tags)
					 VALUES ({})",
					placeholders(23)
				),
				params_from_iter(p.iter()),
			)?;
		}

		for item in rows_of(data, "scratchpad") {
			let id = item
				.get("id")
				.and_then(Value::as_str)
				.unwrap_or("")
				.to_string();
			if id.is_empty() {
				continue;
			}
			let p = [
				g(item, "content", t("")),
				g(item, "session_id", t("default")),
				g(item, "created_at", SqlValue::Null),
				g(item, "updated_at", SqlValue::Null),
				t(&id),
			];
			if row_exists(beam, "scratchpad", &id)? {
				beam.db.execute(
					"UPDATE scratchpad SET content = ?, session_id = ?, created_at = ?, updated_at = ? \
					 WHERE id = ?",
					params_from_iter(p.iter()),
				)?;
				stats.scratchpad.updated += 1;
			} else {
				beam.db.execute(
					"INSERT INTO scratchpad (content, session_id, created_at, updated_at, id) VALUES \
					 (?, ?, ?, ?, ?)",
					params_from_iter(p.iter()),
				)?;
				stats.scratchpad.inserted += 1;
			}
		}

		for item in rows_of(data, "consolidation_log") {
			let p = [
				g(item, "session_id", t("default")),
				g(item, "items_consolidated", SqlValue::Integer(0)),
				g(item, "summary_preview", t("")),
				g(item, "created_at", SqlValue::Null),
			];
			beam.db.execute(
				"INSERT INTO consolidation_log (session_id, items_consolidated, summary_preview, \
				 created_at) VALUES (?, ?, ?, ?)",
				params_from_iter(p.iter()),
			)?;
			stats.consolidation_log.inserted += 1;
		}
		Ok(())
	})?;
	Ok(stats)
}
