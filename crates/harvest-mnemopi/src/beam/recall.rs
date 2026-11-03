//! Hybrid recall (`core/beam/recall.ts`): FTS5 + dense cosine + lexical +
//! importance + recency.

use std::{
	collections::{HashMap, HashSet},
	sync::OnceLock,
};

use chrono::{DateTime, Utc};
use regex::Regex;
use rusqlite::types::Value as SqlValue;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{BeamMemory, store::query_rows};
use crate::{
	Result, config,
	schema::table_exists,
	util::{Row, clamp01, now_iso, parse_ts, placeholders, rnum, round4, rstr},
};

pub const RECALL_CONTENT_PREVIEW_CHARS: usize = 500;
const DEFAULT_LIMIT: i64 = 500;
const STOP_WORDS: &[&str] = &[
	"a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "how", "i", "in", "is", "it",
	"of", "on", "or", "that", "the", "this", "to", "was", "what", "when", "where", "who", "with",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecallOptions {
	pub temporal_weight:       Option<f64>,
	pub query_time:            Option<String>,
	pub temporal_halflife:     Option<f64>,
	pub vec_weight:            Option<f64>,
	pub fts_weight:            Option<f64>,
	pub importance_weight:     Option<f64>,
	pub author_id:             Option<String>,
	pub author_type:           Option<String>,
	pub channel_id:            Option<String>,
	pub source:                Option<String>,
	pub veracity:              Option<String>,
	pub memory_type:           Option<String>,
	pub from_date:             Option<String>,
	pub to_date:               Option<String>,
	#[serde(default)]
	pub ignore_session_scope:  bool,
	pub include_working:       Option<bool>,
	pub content_preview_chars: Option<usize>,
	pub update_recall_counts:  Option<bool>,
	/// Pre-computed query embedding; when `None` the host embedder (if any) is
	/// used.
	pub query_embedding:       Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResult {
	pub id:               String,
	pub content:          String,
	pub source:           Option<String>,
	pub timestamp:        Option<String>,
	pub importance:       f64,
	pub score:            f64,
	pub tier:             String,
	pub tier_label:       String,
	pub degradation_tier: Option<i64>,
	pub keyword_score:    f64,
	pub dense_score:      f64,
	pub fts_score:        f64,
	pub recency_score:    f64,
	pub temporal_score:   f64,
	pub recall_count:     i64,
	pub last_recalled:    Option<String>,
	pub explanation:      String,
	pub truncated:        bool,
	pub full_length:      usize,
	/// Remaining raw row columns.
	#[serde(flatten)]
	pub extra:            Row,
}

struct Candidate {
	row:         Row,
	working:     bool,
	fts:         f64,
	fts_matched: bool,
	dense:       f64,
}

fn tokenize(text: &str) -> Vec<String> {
	static WORD: OnceLock<Regex> = OnceLock::new();
	let re = WORD.get_or_init(|| Regex::new(r"[\p{L}\p{N}_]+").expect("regex"));
	let lower = text.to_lowercase();
	re.find_iter(&lower)
		.map(|m| m.as_str().to_string())
		.filter(|t| !STOP_WORDS.contains(&t.as_str()))
		.collect()
}

fn expanded_tokens(query: &str) -> Vec<String> {
	let mut seen = HashSet::new();
	tokenize(query)
		.into_iter()
		.filter(|t| seen.insert(t.clone()))
		.collect()
}

fn fts_phrase(t: &str) -> String {
	format!("\"{}\"", t.replace('"', "\"\""))
}

fn fts_query(query: &str) -> String {
	let tokens: Vec<String> = expanded_tokens(query).into_iter().take(12).collect();
	if tokens.is_empty() {
		return fts_phrase(query.trim());
	}
	tokens
		.iter()
		.map(|t| fts_phrase(t))
		.collect::<Vec<_>>()
		.join(" OR ")
}

fn count_occurrences(hay: &str, needle: &str) -> usize {
	if needle.is_empty() {
		0
	} else {
		hay.matches(needle).count()
	}
}

fn lexical_relevance(tokens: &[String], content: &str, normalized: &str) -> f64 {
	if tokens.is_empty() {
		return 0.0;
	}
	let lower = content.to_lowercase();
	if tokens.len() > 1 && !normalized.is_empty() && lower.contains(normalized) {
		return 1.0;
	}
	if tokens.len() == 1 {
		let t = &tokens[0];
		if t.is_empty() || !lower.contains(t.as_str()) {
			return 0.0;
		}
		let c = count_occurrences(&lower, t);
		return clamp01(0.7 + (c.saturating_sub(1).min(3) as f64) * 0.1);
	}
	let content_tokens: HashSet<String> = tokenize(&lower).into_iter().collect();
	let (mut exact, mut partial) = (0.0, 0.0);
	for t in tokens {
		if content_tokens.contains(t) || lower.contains(t.as_str()) {
			exact += 1.0;
			continue;
		}
		if content_tokens.iter().any(|c| {
			c.chars().count() >= 4
				&& t.chars().count() >= 4
				&& (c.contains(t.as_str()) || t.contains(c.as_str()))
		}) {
			partial += 1.0;
		}
	}
	clamp01((exact + partial * 0.5) / tokens.len() as f64)
}

fn minimum_relevance(n: usize) -> f64 {
	match n {
		0 | 1 => 0.08,
		2 => 0.18,
		3 => 0.34,
		_ => 0.22,
	}
}

fn query_asks_current(q: &str) -> bool {
	static R: OnceLock<Regex> = OnceLock::new();
	R.get_or_init(|| {
		Regex::new(r"(?i)\b(?:now|current|currently|latest|recent|today|active|present)\b")
			.expect("re")
	})
	.is_match(q)
}

fn current_content_adjustment(content: &str, sensitive: bool) -> f64 {
	if !sensitive {
		return 1.0;
	}
	static A: OnceLock<Regex> = OnceLock::new();
	static B: OnceLock<Regex> = OnceLock::new();
	let lower = content.to_lowercase();
	let mut f = 1.0;
	if A
		.get_or_init(|| {
			Regex::new(r"\b(?:current|currently|latest|now|active|present)\b").expect("re")
		})
		.is_match(&lower)
	{
		f *= 1.35;
	}
	if B
		.get_or_init(|| {
			Regex::new(r"\b(?:was|previous|previously|legacy|old|stale|former|deprecated)\b")
				.expect("re")
		})
		.is_match(&lower)
	{
		f *= 0.72;
	}
	f
}

fn veracity_weight(v: &str) -> f64 {
	match v {
		"stated" | "true" | "likely_true" => 1.0,
		"inferred" => 0.7,
		"imported" => 0.6,
		"tool" => 0.5,
		"false" => 0.0,
		_ => 0.8,
	}
}

fn decay_since(ts: Option<&str>, now: DateTime<Utc>, half_life: f64) -> f64 {
	let Some(t) = ts.and_then(parse_ts) else {
		return 0.0;
	};
	let hours = ((now - t).num_milliseconds().max(0) as f64) / 3_600_000.0;
	(-hours / half_life.max(0.001)).exp()
}

pub fn parse_query_time(v: Option<&str>) -> DateTime<Utc> {
	v.and_then(parse_ts).unwrap_or_else(Utc::now)
}

pub fn clip_recall_content(content: &str, limit: usize) -> (String, bool, usize) {
	let full = content.chars().count();
	if limit == 0 || full <= limit {
		return (content.to_string(), false, full);
	}
	let head: String = content.chars().take(limit - 1).collect();
	(format!("{head}…"), true, full)
}

fn build_where(beam: &BeamMemory, prefix: &str, o: &RecallOptions) -> (String, Vec<SqlValue>) {
	let p = if prefix.is_empty() {
		String::new()
	} else {
		format!("{prefix}.")
	};
	let mut c = vec![
		format!("({p}valid_until IS NULL OR {p}valid_until > ?)"),
		format!("{p}superseded_by IS NULL"),
	];
	let mut params = vec![SqlValue::Text(now_iso())];
	let channel = o.channel_id.as_deref().filter(|c| !c.is_empty());
	if o.ignore_session_scope
		|| (channel.is_none() && (o.author_id.is_some() || o.author_type.is_some()))
	{
	} else if let Some(ch) = channel {
		c.push(format!("({p}session_id = ? OR {p}scope = 'global' OR {p}channel_id = ?)"));
		params.push(SqlValue::Text(beam.session_id.clone()));
		params.push(SqlValue::Text(ch.into()));
	} else {
		c.push(format!("({p}session_id = ? OR {p}scope = 'global')"));
		params.push(SqlValue::Text(beam.session_id.clone()));
	}
	if let Some(d) = &o.from_date {
		c.push(format!("{p}timestamp >= ?"));
		params.push(SqlValue::Text(format!("{d}T00:00:00")));
	}
	if let Some(d) = &o.to_date {
		c.push(format!("{p}timestamp <= ?"));
		params.push(SqlValue::Text(format!("{d}T23:59:59")));
	}
	for (col, v) in [
		("source", &o.source),
		("veracity", &o.veracity),
		("memory_type", &o.memory_type),
		("author_id", &o.author_id),
		("author_type", &o.author_type),
	] {
		if let Some(v) = v.as_ref().filter(|v| !v.is_empty()) {
			c.push(format!("{p}{col} = ?"));
			params.push(SqlValue::Text(v.clone()));
		}
	}
	(c.join(" AND "), params)
}

const MEMORY_COLUMNS: &str = "id, content, source, timestamp, session_id, importance, \
                              metadata_json, veracity, memory_type, recall_count, last_recalled, \
                              valid_until, superseded_by, scope, author_id, author_type, \
                              channel_id, event_date, event_date_precision, temporal_tags";

fn columns(working: bool) -> String {
	if working {
		format!("{MEMORY_COLUMNS}, embed_text")
	} else {
		format!("{MEMORY_COLUMNS}, rowid, summary_of, tier")
	}
}

fn fts_rows(beam: &BeamMemory, working: bool, query: &str, limit: i64) -> Vec<Row> {
	let (table, sql) = if working {
		(
			"fts_working",
			"SELECT f.id, f.rank FROM fts_working f WHERE f.fts_working MATCH ?
			 AND EXISTS (SELECT 1 FROM working_memory w WHERE w.id = f.id AND w.superseded_by IS NULL
			     AND (w.valid_until IS NULL OR w.valid_until > ?))
			 ORDER BY f.rank, f.id LIMIT ?",
		)
	} else {
		(
			"fts_episodes",
			"SELECT f.rowid, f.rank FROM fts_episodes f WHERE f.fts_episodes MATCH ?
			 AND EXISTS (SELECT 1 FROM episodic_memory e WHERE e.rowid = f.rowid AND e.superseded_by IS NULL
			     AND (e.valid_until IS NULL OR e.valid_until > ?))
			 ORDER BY f.rank, f.rowid LIMIT ?",
		)
	};
	if !table_exists(&beam.db, table) {
		return Vec::new();
	}
	query_rows(beam, sql, &[
		SqlValue::Text(fts_query(query)),
		SqlValue::Text(now_iso()),
		SqlValue::Integer(limit),
	])
	.unwrap_or_default()
}

fn key_of(row: &Row, key: &str) -> Option<String> {
	rstr(row, key)
}

fn normalize_ranks(rows: &[Row], key: &str) -> HashMap<String, f64> {
	let mut out = HashMap::new();
	if rows.is_empty() {
		return out;
	}
	let ranks: Vec<f64> = rows.iter().map(|r| rnum(r, "rank", 0.0)).collect();
	let min = ranks.iter().copied().fold(f64::INFINITY, f64::min);
	let max = ranks.iter().copied().fold(f64::NEG_INFINITY, f64::max);
	let range = if max == min { 1.0 } else { max - min };
	for (row, rank) in rows.iter().zip(ranks) {
		if let Some(k) = key_of(row, key) {
			out.insert(k, 1.0 - (rank - min) / range);
		}
	}
	out
}

fn cosine(a: &[f32], b: &[f64]) -> f64 {
	if a.len() != b.len() || a.is_empty() {
		return 0.0;
	}
	let (mut dot, mut na, mut nb) = (0.0, 0.0, 0.0);
	for (x, y) in a.iter().zip(b) {
		let x = f64::from(*x);
		dot += x * y;
		na += x * x;
		nb += y * y;
	}
	if na == 0.0 || nb == 0.0 {
		0.0
	} else {
		dot / (na.sqrt() * nb.sqrt())
	}
}

fn vector_similarities(beam: &BeamMemory, ids: &[String], q: &[f32]) -> HashMap<String, f64> {
	let mut out = HashMap::new();
	if q.is_empty() || ids.is_empty() {
		return out;
	}
	for chunk in ids.chunks(500) {
		let p: Vec<SqlValue> = chunk.iter().map(|i| SqlValue::Text(i.clone())).collect();
		let rows = query_rows(
			beam,
			&format!(
				"SELECT memory_id, embedding_json FROM memory_embeddings WHERE memory_id IN ({})",
				placeholders(chunk.len())
			),
			&p,
		)
		.unwrap_or_default();
		for row in rows {
			let (Some(id), Some(raw)) = (rstr(&row, "memory_id"), rstr(&row, "embedding_json")) else {
				continue;
			};
			if let Ok(v) = serde_json::from_str::<Vec<f64>>(&raw) {
				out.insert(id, cosine(q, &v).max(0.0));
			}
		}
	}
	out
}

fn all_visible_ids(beam: &BeamMemory, table: &str, o: &RecallOptions) -> Vec<String> {
	let (w, mut p) = build_where(beam, "", o);
	p.push(SqlValue::Integer(DEFAULT_LIMIT));
	query_rows(
		beam,
		&format!("SELECT id FROM {table} WHERE {w} ORDER BY timestamp DESC LIMIT ?"),
		&p,
	)
	.unwrap_or_default()
	.iter()
	.filter_map(|r| rstr(r, "id"))
	.collect()
}

fn fetch_candidates(
	beam: &BeamMemory,
	working: bool,
	keys: &[String],
	fts: &HashMap<String, f64>,
	vec: &HashMap<String, f64>,
	o: &RecallOptions,
) -> Vec<Candidate> {
	if keys.is_empty() {
		return Vec::new();
	}
	let table = if working {
		"working_memory"
	} else {
		"episodic_memory"
	};
	let key_col = if working { "id" } else { "rowid" };
	let cols: String = columns(working)
		.split(", ")
		.map(|c| format!("m.{c}"))
		.collect::<Vec<_>>()
		.join(", ");
	let (w, wp) = build_where(beam, "m", o);
	let mut p: Vec<SqlValue> = keys
		.iter()
		.map(|k| {
			if working {
				SqlValue::Text(k.clone())
			} else {
				SqlValue::Integer(k.parse().unwrap_or(-1))
			}
		})
		.collect();
	p.extend(wp);
	let rows = query_rows(
		beam,
		&format!(
			"SELECT {cols} FROM {table} m WHERE m.{key_col} IN ({}) AND {w}",
			placeholders(keys.len())
		),
		&p,
	)
	.unwrap_or_default();
	rows
		.into_iter()
		.map(|row| {
			let k = key_of(&row, key_col).unwrap_or_default();
			let id = rstr(&row, "id").unwrap_or_default();
			Candidate {
				fts: fts.get(&k).copied().unwrap_or(0.0),
				fts_matched: fts.contains_key(&k),
				dense: vec.get(&id).copied().unwrap_or(0.0),
				row,
				working,
			}
		})
		.collect()
}

fn fallback_candidates(beam: &BeamMemory, working: bool, o: &RecallOptions) -> Vec<Candidate> {
	let table = if working {
		"working_memory"
	} else {
		"episodic_memory"
	};
	let (w, mut p) = build_where(beam, "", o);
	p.push(SqlValue::Integer(DEFAULT_LIMIT.min(2000)));
	query_rows(
		beam,
		&format!(
			"SELECT {} FROM {table} WHERE {w} ORDER BY timestamp DESC LIMIT ?",
			columns(working)
		),
		&p,
	)
	.unwrap_or_default()
	.into_iter()
	.map(|row| Candidate { row, working, fts: 0.0, fts_matched: false, dense: 0.0 })
	.collect()
}

fn collect_candidates(
	beam: &BeamMemory,
	query: &str,
	top_k: usize,
	o: &RecallOptions,
) -> Vec<Candidate> {
	let limit = (top_k as i64 * 3).max(50);
	let include_working = o.include_working != Some(false);
	let wm_rows = if include_working {
		fts_rows(beam, true, query, limit)
	} else {
		Vec::new()
	};
	let em_rows = fts_rows(beam, false, query, limit);
	let wm_fts = normalize_ranks(&wm_rows, "id");
	let em_fts = normalize_ranks(&em_rows, "rowid");
	let mut wm_ids: Vec<String> = wm_rows.iter().filter_map(|r| rstr(r, "id")).collect();
	let mut em_rowids: Vec<String> = em_rows.iter().filter_map(|r| rstr(r, "rowid")).collect();
	let mut wm_vec = HashMap::new();
	let mut em_vec = HashMap::new();
	if let Some(q) = o.query_embedding.as_deref() {
		let all_wm = if include_working {
			all_visible_ids(beam, "working_memory", o)
		} else {
			Vec::new()
		};
		let all_em = all_visible_ids(beam, "episodic_memory", o);
		wm_vec = vector_similarities(beam, &all_wm, q);
		em_vec = vector_similarities(beam, &all_em, q);
		let top = |m: &HashMap<String, f64>| {
			let mut v: Vec<(&String, &f64)> = m.iter().collect();
			v.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
			v.into_iter()
				.take(limit as usize)
				.map(|(k, _)| k.clone())
				.collect::<Vec<_>>()
		};
		let mut seen: HashSet<String> = wm_ids.iter().cloned().collect();
		for id in top(&wm_vec) {
			if seen.insert(id.clone()) {
				wm_ids.push(id);
			}
		}
		let em_ids = top(&em_vec);
		if !em_ids.is_empty() {
			let p: Vec<SqlValue> = em_ids.iter().map(|i| SqlValue::Text(i.clone())).collect();
			let rows = query_rows(
				beam,
				&format!(
					"SELECT rowid, id FROM episodic_memory WHERE id IN ({})",
					placeholders(em_ids.len())
				),
				&p,
			)
			.unwrap_or_default();
			let mut seen: HashSet<String> = em_rowids.iter().cloned().collect();
			for r in rows {
				if let Some(rid) = rstr(&r, "rowid").filter(|x| seen.insert(x.clone())) {
					em_rowids.push(rid);
				}
			}
		}
	}
	let mut out = Vec::new();
	if !wm_ids.is_empty() {
		out.extend(fetch_candidates(beam, true, &wm_ids, &wm_fts, &wm_vec, o));
	} else if include_working {
		out.extend(fallback_candidates(beam, true, o));
	}
	if !em_rowids.is_empty() {
		out.extend(fetch_candidates(beam, false, &em_rowids, &em_fts, &em_vec, o));
	} else {
		out.extend(fallback_candidates(beam, false, o));
	}
	out
}

#[allow(clippy::too_many_arguments)]
fn score(
	c: Candidate,
	tokens: &[String],
	normalized: &str,
	weights: (f64, f64, f64),
	o: &RecallOptions,
	current_sensitive: bool,
	query_time: Option<DateTime<Utc>>,
) -> Option<RecallResult> {
	let content = rstr(&c.row, "content").unwrap_or_default();
	let searchable = rstr(&c.row, "embed_text")
		.filter(|s| !s.is_empty())
		.unwrap_or_else(|| content.clone());
	let lexical = lexical_relevance(tokens, &searchable, normalized);
	if lexical < minimum_relevance(tokens.len()) && c.dense < 0.65 {
		return None;
	}
	let (vw, fw, iw) = weights;
	let importance = rnum(&c.row, "importance", 0.5);
	let ts = rstr(&c.row, "timestamp");
	let decay = decay_since(ts.as_deref(), query_time.unwrap_or_else(Utc::now), 72.0);
	let keyword = lexical.max(c.fts * 0.6);
	let base = if c.working {
		let kw_share = (1.0 - iw) * 0.6;
		let b = keyword * kw_share + importance * iw + keyword * keyword * 0.08;
		if c.dense > 0.0 {
			b * 0.8 + c.dense * 0.2
		} else {
			b
		}
	} else {
		(c.dense * vw + c.fts * fw + importance * iw).max(lexical * 0.8)
	};
	let mut s = base * (0.7 + 0.3 * decay);
	let tw = o.temporal_weight.unwrap_or(0.0);
	let mut temporal = 0.0;
	if tw > 0.0 {
		let half = o
			.temporal_halflife
			.unwrap_or_else(config::temporal_halflife_hours);
		let qt = query_time.unwrap_or_else(Utc::now);
		temporal = decay_since(ts.as_deref(), qt, half).max(decay_since(
			rstr(&c.row, "event_date").as_deref(),
			qt,
			half * 2.0,
		));
		s *= 1.0 + tw * temporal;
	}
	let degradation_tier = (!c.working).then(|| rnum(&c.row, "tier", 1.0) as i64);
	if let Some(t) = degradation_tier {
		s *= match t {
			1 => 1.0,
			2 => 0.85,
			_ => 0.7,
		};
	}
	s *= veracity_weight(&rstr(&c.row, "veracity").unwrap_or_else(|| "unknown".into()))
		* current_content_adjustment(&searchable, current_sensitive);
	let (preview, truncated, full_length) = clip_recall_content(
		&content,
		o.content_preview_chars
			.unwrap_or(RECALL_CONTENT_PREVIEW_CHARS),
	);
	let tier = if c.working { "working" } else { "episodic" };
	let source_kind = if c.fts_matched {
		"fts"
	} else if c.dense > 0.0 {
		"vec"
	} else {
		"fallback"
	};
	let mut expl = vec![tier.to_string(), source_kind.to_string()];
	if lexical > 0.0 {
		expl.push(format!("keyword={}", round4(lexical)));
	}
	if c.dense > 0.0 {
		expl.push(format!("dense={}", round4(c.dense)));
	}
	if temporal > 0.0 {
		expl.push(format!("temporal={}", round4(temporal)));
	}
	let mut extra = c.row.clone();
	for k in [
		"id",
		"content",
		"source",
		"timestamp",
		"importance",
		"tier",
		"recall_count",
		"last_recalled",
	] {
		extra.remove(k);
	}
	extra.insert(
		"voice_scores".into(),
		json!({"vec": round4(c.dense), "fts": round4(c.fts), "keyword": round4(lexical),
			"importance": round4(importance), "recency_decay": round4(decay), "temporal": round4(temporal)}),
	);
	Some(RecallResult {
		id: rstr(&c.row, "id").unwrap_or_default(),
		content: preview,
		source: rstr(&c.row, "source"),
		timestamp: ts,
		importance,
		score: round4(s),
		tier: tier.into(),
		tier_label: tier.into(),
		degradation_tier,
		keyword_score: round4(lexical),
		dense_score: round4(c.dense),
		fts_score: round4(c.fts),
		recency_score: round4(decay),
		temporal_score: round4(temporal),
		recall_count: rnum(&c.row, "recall_count", 0.0) as i64,
		last_recalled: rstr(&c.row, "last_recalled"),
		explanation: expl.join(" "),
		truncated,
		full_length,
		extra,
	})
}

fn dedup_cross_tier(beam: &BeamMemory, results: Vec<RecallResult>) -> Vec<RecallResult> {
	let working: HashSet<String> = results
		.iter()
		.filter(|r| r.tier == "working")
		.map(|r| r.id.clone())
		.collect();
	let episodic: Vec<String> = results
		.iter()
		.filter(|r| r.tier == "episodic")
		.map(|r| r.id.clone())
		.collect();
	if working.is_empty() || episodic.is_empty() {
		return results;
	}
	let p: Vec<SqlValue> = episodic.iter().map(|i| SqlValue::Text(i.clone())).collect();
	let rows = query_rows(
		beam,
		&format!(
			"SELECT id, summary_of FROM episodic_memory WHERE id IN ({})",
			placeholders(episodic.len())
		),
		&p,
	)
	.unwrap_or_default();
	let drop: HashSet<String> = rows
		.iter()
		.filter(|r| {
			rstr(r, "summary_of")
				.unwrap_or_default()
				.split(',')
				.map(str::trim)
				.any(|id| !id.is_empty() && working.contains(id))
		})
		.filter_map(|r| rstr(r, "id"))
		.collect();
	results
		.into_iter()
		.filter(|r| !(r.tier == "episodic" && drop.contains(&r.id)))
		.collect()
}

fn update_recall_counts(
	beam: &BeamMemory,
	results: &[RecallResult],
	o: &RecallOptions,
) -> Result<()> {
	let ts = now_iso();
	for (tier, table) in [("working", "working_memory"), ("episodic", "episodic_memory")] {
		let ids: Vec<&String> = results
			.iter()
			.filter(|r| r.tier == tier)
			.map(|r| &r.id)
			.collect();
		if ids.is_empty() {
			continue;
		}
		let (w, wp) = build_where(beam, "", o);
		let mut p = vec![SqlValue::Text(ts.clone())];
		p.extend(ids.iter().map(|i| SqlValue::Text((*i).clone())));
		p.extend(wp);
		beam.db.execute(
			&format!(
				"UPDATE {table} SET recall_count = COALESCE(recall_count, 0) + 1, last_recalled = ? \
				 WHERE id IN ({}) AND {w}",
				placeholders(ids.len())
			),
			rusqlite::params_from_iter(p.iter()),
		)?;
	}
	Ok(())
}

fn normalized_weights(v: f64, f: f64, i: f64) -> (f64, f64, f64) {
	let (v, f, i) = (v.max(0.0), f.max(0.0), i.max(0.0));
	let sum = v + f + i;
	if sum <= 0.0 {
		(0.5, 0.3, 0.2)
	} else {
		(v / sum, f / sum, i / sum)
	}
}

/// Hybrid recall across working + episodic tiers.
pub fn recall(
	beam: &BeamMemory,
	query: &str,
	top_k: usize,
	options: &RecallOptions,
) -> Result<Vec<RecallResult>> {
	crate::limits::check_text(query, crate::limits::MAX_QUERY_BYTES, "recall query")?;
	if top_k > crate::limits::MAX_RECALL_RESULTS {
		return Err(crate::MnemopiError::Invalid("recall limit exceeds 100 results".into()));
	}
	for value in [
		options.temporal_weight,
		options.temporal_halflife,
		options.vec_weight,
		options.fts_weight,
		options.importance_weight,
	]
	.into_iter()
	.flatten()
	{
		if !value.is_finite() || value < 0.0 {
			return Err(crate::MnemopiError::Invalid("invalid recall scoring option".into()));
		}
	}
	if top_k == 0 {
		return Ok(Vec::new());
	}
	let mut o = options.clone();
	let current = query_asks_current(query);
	if current {
		o.query_time.get_or_insert_with(now_iso);
		o.temporal_weight.get_or_insert(0.45);
	}
	if o.query_embedding.is_none() && !query.is_empty() && !config::embeddings_disabled() {
		if let Some(e) = beam.embedder.as_ref() {
			o.query_embedding = e.embed(&[query.to_string()]).ok().and_then(|mut v| v.pop());
		}
	}
	if o.query_embedding.as_ref().is_some_and(|v| {
		v.len() > crate::limits::MAX_VECTOR_DIMENSIONS || v.iter().any(|n| !n.is_finite())
	}) {
		return Err(crate::MnemopiError::Invalid("invalid query embedding".into()));
	}
	let weights = normalized_weights(
		o.vec_weight.unwrap_or(beam.config.vec_weight),
		o.fts_weight.unwrap_or(beam.config.fts_weight),
		o.importance_weight.unwrap_or(beam.config.importance_weight),
	);
	let tokens = expanded_tokens(query);
	let normalized = query
		.split_whitespace()
		.collect::<Vec<_>>()
		.join(" ")
		.to_lowercase();
	let query_time = o.query_time.as_deref().map(|q| parse_query_time(Some(q)));
	let mut scored: Vec<RecallResult> = collect_candidates(beam, query, top_k, &o)
		.into_iter()
		.filter_map(|c| score(c, &tokens, &normalized, weights, &o, current, query_time))
		.collect();
	scored.sort_by(|a, b| {
		b.score
			.partial_cmp(&a.score)
			.unwrap_or(std::cmp::Ordering::Equal)
	});
	let mut seen = HashSet::new();
	scored.retain(|r| seen.insert(format!("{}:{}", r.tier, r.id)));
	let mut results = dedup_cross_tier(beam, scored);
	results.truncate(top_k);
	if o.update_recall_counts != Some(false) {
		update_recall_counts(beam, &results, &o)?;
	}
	Ok(results)
}

/// Format results as a bullet list for prompt injection.
pub fn format_context(results: &[RecallResult]) -> String {
	results
		.iter()
		.map(|r| format!("- {}", r.content))
		.collect::<Vec<_>>()
		.join("\n")
}

#[allow(dead_code)]
fn _value_unused(_: Value) {}
