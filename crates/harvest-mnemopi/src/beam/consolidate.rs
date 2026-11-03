//! Working-to-episodic consolidation and category-preserving fact storage.
//!
//! Selection, claims, summaries and the log commit together: failed writes do
//! not strand working rows as permanently consolidated.

use std::collections::BTreeMap;

use chrono::{Duration, Utc};
use rusqlite::{params, types::Value as SqlValue};
use serde_json::{Value, json};

use super::{BeamMemory, store};
use crate::{
	Result, aaak, config,
	db::transaction,
	extraction::{self, ExtractedFactCategories},
	limits::{MAX_BATCH_ITEMS, MAX_CONTENT_BYTES},
	util::{Row, generate_id, iso, now_iso, prefix_chars, rstr, stable_memory_id},
};

fn insert_fact(
	beam: &BeamMemory,
	text: &str,
	index: i64,
	source: Option<&str>,
	importance: f64,
) -> Result<()> {
	let timestamp = now_iso();
	beam.db.execute(
		"INSERT INTO memoria_facts \
		 (session_id,message_idx,fact_type,key,value,context_snippet,importance,timestamp,\
		 source_memory_id)
		 VALUES (?,?,'entity','fact',?,?,?,?,?)",
		params![beam.source_session(), index, text, text, importance, timestamp, source],
	)?;
	let id = stable_memory_id(
		&format!("{}\0entity\0fact\0{text}", beam.source_session()),
		source.unwrap_or(""),
	);
	beam.db.execute(
		"INSERT OR IGNORE INTO facts \
		 (fact_id,session_id,subject,predicate,object,timestamp,source_msg_id,confidence)
		 VALUES (?,?,'fact','entity',?,?,?,?)",
		params![id, beam.source_session(), text, timestamp, source, importance],
	)?;
	Ok(())
}

/// Keep fact categories in their persisted tables instead of flattening all
/// extraction output into undifferentiated text.
pub fn store_extracted_fact_categories(
	beam: &BeamMemory,
	extracted: &ExtractedFactCategories,
	message_index: i64,
	source_memory_id: Option<&str>,
	importance: f64,
) -> Result<usize> {
	crate::limits::importance(Some(importance))?;
	if extracted.count() > 100 {
		return Err(crate::MnemopiError::Invalid("extraction exceeds 100 facts".into()));
	}
	transaction(&beam.db, || {
		let mut stored = 0;
		for fact in extracted
			.facts
			.iter()
			.chain(&extracted.instructions)
			.chain(&extracted.preferences)
			.chain(&extracted.timelines)
		{
			crate::limits::check_text(fact, MAX_CONTENT_BYTES, "fact")?;
			insert_fact(beam, fact, message_index, source_memory_id, importance)?;
			stored += 1;
		}
		for instruction in &extracted.instructions {
			beam.db.execute(
				"INSERT INTO memoria_instructions
				(session_id,message_idx,instruction,active,context_snippet,source_memory_id) VALUES \
				 (?,?,?,1,?,?)",
				params![
					beam.source_session(),
					message_index,
					instruction,
					instruction,
					source_memory_id
				],
			)?;
		}
		for preference in &extracted.preferences {
			beam.db.execute(
				"INSERT INTO memoria_preferences
				(session_id,message_idx,preference,context_snippet,source_memory_id) VALUES (?,?,?,?,?)",
				params![beam.source_session(), message_index, preference, preference, source_memory_id],
			)?;
		}
		for timeline in &extracted.timelines {
			let date = timeline
				.get(..10)
				.filter(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").is_ok());
			beam.db.execute(
				"INSERT INTO memoria_timelines
				(session_id,date,message_idx,description,source,source_memory_id) VALUES \
				 (?,?,?,?,'extraction',?)",
				params![beam.source_session(), date, message_index, timeline, source_memory_id],
			)?;
		}
		for triple in &extracted.kg {
			for text in [&triple.subject, &triple.predicate, &triple.object] {
				crate::limits::check_text(text, MAX_CONTENT_BYTES, "triple")?;
			}
			beam.db.execute(
				"INSERT INTO memoria_kg
				(session_id,subject,predicate,object,message_idx,confidence,source_memory_id) VALUES \
				 (?,?,?,?,?,0.65,?)",
				params![
					beam.source_session(),
					triple.subject,
					triple.predicate,
					triple.object,
					message_index,
					source_memory_id
				],
			)?;
			beam.db.execute(
				"INSERT INTO triples (subject,predicate,object,valid_from,source,confidence) VALUES \
				 (?,?,?,?,?,0.65)",
				params![
					triple.subject,
					triple.predicate,
					triple.object,
					now_iso(),
					source_memory_id.unwrap_or("extraction")
				],
			)?;
		}
		Ok(stored)
	})
}

fn veracity_weight(value: &str) -> f64 {
	match value {
		"true" | "stated" => 1.0,
		"inferred" => 0.7,
		"imported" => 0.6,
		"tool" => 0.5,
		"false" => 0.0,
		_ => 0.8,
	}
}

fn aggregate_veracity(rows: &[Row]) -> String {
	let mut counts = BTreeMap::new();
	let mut winner = "unknown".to_string();
	let mut max = 0;
	for row in rows {
		let value = store::clamp_veracity(rstr(row, "veracity").as_deref());
		if value == "unknown" {
			continue;
		}
		let count = counts.entry(value.clone()).or_insert(0);
		*count += 1;
		if *count > max || (*count == max && veracity_weight(&value) < veracity_weight(&winner)) {
			winner = value;
			max = *count;
		}
	}
	winner
}

fn chunks(rows: Vec<Row>, source: &str, max: usize) -> Vec<Vec<Row>> {
	let budget = max.saturating_sub(source.chars().count() + 3);
	let mut chunks = Vec::new();
	let mut current = Vec::new();
	let mut size = 0;
	for row in rows {
		let length = rstr(&row, "content").unwrap_or_default().chars().count();
		if !current.is_empty() && size + 3 + length > budget {
			chunks.push(std::mem::take(&mut current));
			size = 0;
		}
		size += usize::from(!current.is_empty()) * 3 + length;
		current.push(row);
	}
	if !current.is_empty() {
		chunks.push(current);
	}
	chunks
}

fn store_episode(beam: &BeamMemory, source: &str, rows: &[Row], max: usize) -> Result<Vec<String>> {
	let text = rows
		.iter()
		.map(|r| rstr(r, "content").unwrap_or_default())
		.collect::<Vec<_>>()
		.join(" | ");
	let uncapped = format!("[{source}] {}", aaak::encode(&text));
	let truncated = uncapped.chars().count() > max;
	let summary = if truncated {
		const MARKER: &str = "\n[... sleep_consolidation episode truncated by maxEpisodeChars ...]";
		if max > MARKER.len() {
			format!("{}{MARKER}", prefix_chars(&uncapped, max - MARKER.len()).trim_end())
		} else {
			prefix_chars(&uncapped, max)
		}
	} else {
		uncapped
	};
	let ids: Vec<_> = rows.iter().filter_map(|r| rstr(r, "id")).collect();
	let scope = if rows
		.iter()
		.any(|r| rstr(r, "scope").as_deref() == Some("global"))
	{
		"global"
	} else {
		"session"
	};
	let expiry = rows.iter().filter_map(|r| rstr(r, "valid_until")).min();
	let now = Utc::now();
	let id = generate_id(&summary, now);
	let metadata = json!({ "original_count": rows.len(), "source": source, "llm_used": false,
		"truncated": truncated, "original_chars": text.chars().count(), "max_chars": max })
	.to_string();
	beam.db.execute(
		"INSERT INTO episodic_memory
		(id,content,source,timestamp,session_id,importance,metadata_json,summary_of,valid_until,scope,
		 author_id,author_type,channel_id,memory_type,veracity,created_at)
		VALUES (?,?,'sleep_consolidation',?,?,0.6,?,?,?,?,?,?,?,'unknown',?,?)",
		params![
			id,
			summary,
			iso(now),
			beam.source_session(),
			metadata,
			ids.join(","),
			expiry,
			scope,
			beam.author_id,
			beam.author_type,
			beam.channel_id,
			aggregate_veracity(rows),
			iso(now)
		],
	)?;
	let extracted =
		extraction::extract_fact_categories(beam.llm.as_deref(), &summary, config::llm_max_tokens());
	store_extracted_fact_categories(beam, &extracted, 0, Some(&id), 0.7)?;
	store::run_embedding(beam, &[(id, summary)]);
	Ok(ids)
}

pub fn sleep(beam: &BeamMemory, dry_run: bool) -> Result<Value> {
	transaction(&beam.db, || {
		let cutoff =
			iso(Utc::now() - Duration::hours(beam.config.working_memory_ttl_hours.max(0) / 2));
		let batch = config::sleep_batch_size().clamp(1, MAX_BATCH_ITEMS as i64);
		let rows = store::query_rows(
			beam,
			"SELECT id,COALESCE(embed_text,content) AS content,source,timestamp,
			importance,metadata_json,scope,valid_until,veracity FROM working_memory
			WHERE COALESCE(session_id,'default')=? AND timestamp<? AND consolidated_at IS NULL
			ORDER BY timestamp ASC LIMIT ?",
			&[
				SqlValue::Text(beam.source_session().into()),
				SqlValue::Text(cutoff),
				SqlValue::Integer(batch),
			],
		)?;
		if rows.is_empty() {
			return Ok(
				json!({ "dry_run": dry_run, "status": "no_op", "message": "No old working memories to consolidate" }),
			);
		}
		let mut groups: BTreeMap<String, Vec<Row>> = BTreeMap::new();
		for row in rows {
			groups
				.entry(rstr(&row, "source").unwrap_or_else(|| "unknown".into()))
				.or_default()
				.push(row);
		}
		let max = beam
			.config
			.max_episode_chars
			.clamp(1, (MAX_CONTENT_BYTES / 4) as i64) as usize;
		let mut ids = Vec::new();
		let mut summaries = 0;
		for (source, rows) in groups {
			for chunk in chunks(rows, &source, max) {
				let current: Vec<_> = if dry_run {
					chunk.iter().filter_map(|r| rstr(r, "id")).collect()
				} else {
					store_episode(beam, &source, &chunk, max)?
				};
				if !dry_run {
					let timestamp = now_iso();
					for id in &current {
						beam.db.execute(
							"UPDATE working_memory SET consolidated_at=? WHERE id=? AND session_id=? AND \
							 consolidated_at IS NULL",
							params![timestamp, id, beam.source_session()],
						)?;
					}
				}
				ids.extend(current);
				summaries += 1;
			}
		}
		if !dry_run {
			beam.db.execute(
				"INSERT INTO \
				 consolidation_log(session_id,items_consolidated,summary_preview,created_at) VALUES \
				 (?,?,?,?)",
				params![
					beam.source_session(),
					ids.len(),
					format!("{summaries} summaries (aaak) from {} items", ids.len()),
					now_iso()
				],
			)?;
		}
		Ok(json!({ "dry_run": dry_run, "status": if dry_run { "dry_run" } else { "consolidated" },
			"items_consolidated": ids.len(), "summaries_created": summaries, "conflicts_resolved": 0,
			"llm_used": 0, "method": "aaak", "consolidated_ids": ids, "degradation": degrade_episodic(beam, dry_run)? }))
	})
}

pub fn sleep_all_sessions(beam: &mut BeamMemory, dry_run: bool) -> Result<Value> {
	let cutoff = iso(Utc::now() - Duration::hours(beam.config.working_memory_ttl_hours.max(0) / 2));
	let sessions = store::query_rows(
		beam,
		"SELECT COALESCE(session_id,'default') AS session_id, COUNT(*) AS eligible
		FROM working_memory WHERE timestamp<? AND consolidated_at IS NULL GROUP BY session_id ORDER BY \
		 MIN(timestamp)",
		&[SqlValue::Text(cutoff)],
	)?;
	let mut results = Vec::new();
	let mut items = 0;
	let mut summaries = 0;
	for session in &sessions {
		let session_id = rstr(session, "session_id").unwrap_or_else(|| "default".into());
		let mut result = beam.with_session(&session_id, |scoped| sleep(scoped, dry_run))?;
		items += result["items_consolidated"].as_u64().unwrap_or(0);
		summaries += result["summaries_created"].as_u64().unwrap_or(0);
		result["session_id"] = json!(session_id);
		result["eligible"] = session.get("eligible").cloned().unwrap_or(Value::Null);
		results.push(result);
	}
	Ok(
		json!({ "dry_run": dry_run, "status": if items == 0 { "no_op" } else if dry_run { "dry_run" } else { "consolidated" },
		"sessions_scanned": sessions.len(), "sessions_consolidated": results.iter().filter(|r| r["status"] != "no_op").count(),
		"items_consolidated": items, "summaries_created": summaries, "llm_used": 0, "errors": 0,
		"error_details": [], "session_results": results, "original_session": beam.session_id }),
	)
}

pub fn degrade_episodic(beam: &BeamMemory, dry_run: bool) -> Result<Value> {
	let batch = config::degrade_batch_size().clamp(1, MAX_BATCH_ITEMS as i64);
	let mut counts = [0usize; 2];
	let mut stages = Vec::new();
	for (tier, days, max_chars, limit) in [
		(1, config::tier2_days(), 800usize, batch),
		(
			2,
			config::tier3_days(),
			config::tier3_max_chars().min(MAX_CONTENT_BYTES / 4),
			(batch / 2).max(1),
		),
	] {
		let cutoff = iso(Utc::now() - Duration::days(days.clamp(0, 365_000)));
		let rows = store::query_rows(
			beam,
			"SELECT id,content FROM episodic_memory WHERE tier=? AND created_at<?
			AND (session_id=? OR scope='global') ORDER BY created_at ASC LIMIT ?",
			&[
				SqlValue::Integer(tier),
				SqlValue::Text(cutoff),
				SqlValue::Text(beam.source_session().into()),
				SqlValue::Integer(limit),
			],
		)?;
		stages.push((tier, max_chars, rows));
	}
	// Both tiers are selected before any update so one call never ages the
	// same episode twice.
	for (index, (tier, max_chars, rows)) in stages.into_iter().enumerate() {
		counts[index] = rows.len();
		if dry_run {
			continue;
		}
		for row in rows {
			let Some(id) = rstr(&row, "id") else { continue };
			let content = rstr(&row, "content").unwrap_or_default();
			let compressed = prefix_chars(&content, max_chars);
			beam.db.execute(
				"UPDATE episodic_memory SET content=?,tier=?,degraded_at=? WHERE id=?",
				params![compressed, tier + 1, now_iso(), id],
			)?;
			if compressed != content {
				beam
					.db
					.execute("DELETE FROM memory_embeddings WHERE memory_id=?", [&id])?;
				beam
					.db
					.execute("UPDATE episodic_memory SET binary_vector=NULL WHERE id=?", [&id])?;
			}
		}
	}
	Ok(json!({ "status": if dry_run { "dry_run" } else { "degraded" },
		"tier1_to_tier2": counts[0], "tier2_to_tier3": counts[1] }))
}
