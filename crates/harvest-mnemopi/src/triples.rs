//! Temporal triple store (`core/triples.ts`, `TripleStore.add`/`query`).

use rusqlite::{Connection, params_from_iter};
use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TripleRow {
	pub id:          i64,
	pub subject:     String,
	pub predicate:   String,
	pub object:      String,
	pub valid_from:  String,
	pub valid_until: Option<String>,
	pub source:      Option<String>,
	pub confidence:  Option<f64>,
	pub created_at:  Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TripleQuery {
	pub subject:   Option<String>,
	pub predicate: Option<String>,
	pub object:    Option<String>,
	pub as_of:     Option<String>,
}

fn today() -> String {
	chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Insert a triple, closing any open `(subject, predicate)` interval first.
pub fn add_triple(
	db: &Connection,
	subject: &str,
	predicate: &str,
	object: &str,
	valid_from: Option<&str>,
	source: Option<&str>,
	confidence: Option<f64>,
) -> Result<i64> {
	for value in [subject, predicate, object] {
		crate::limits::check_text(value, crate::limits::MAX_CONTENT_BYTES, "triple")?;
	}
	crate::limits::importance(confidence)?;
	let valid_from = valid_from.map(str::to_string).unwrap_or_else(today);
	crate::db::transaction(db, || {
		db.execute(
			"UPDATE triples SET valid_until = ? WHERE subject = ? AND predicate = ? AND valid_until \
			 IS NULL",
			(&valid_from, subject, predicate),
		)?;
		db.execute(
			"INSERT INTO triples (subject, predicate, object, valid_from, source, confidence) VALUES \
			 (?, ?, ?, ?, ?, ?)",
			(
				subject,
				predicate,
				object,
				&valid_from,
				source.unwrap_or("inferred"),
				confidence.unwrap_or(1.0),
			),
		)?;
		Ok(db.last_insert_rowid())
	})
}

pub fn query_triples(db: &Connection, q: &TripleQuery) -> Result<Vec<TripleRow>> {
	let mut conds = Vec::new();
	let mut params: Vec<String> = Vec::new();
	for (col, v) in [("subject", &q.subject), ("predicate", &q.predicate), ("object", &q.object)] {
		if let Some(v) = v.as_ref().filter(|v| !v.is_empty()) {
			conds.push(format!("{col} = ?"));
			params.push(v.clone());
		}
	}
	let as_of = q
		.as_of
		.clone()
		.filter(|v| !v.is_empty())
		.unwrap_or_else(today);
	conds.push("valid_from <= ?".into());
	params.push(as_of.clone());
	conds.push("(valid_until IS NULL OR valid_until > ?)".into());
	params.push(as_of);
	let sql = format!(
		"SELECT id, subject, predicate, object, valid_from, valid_until, source, confidence, \
		 created_at FROM triples WHERE {} ORDER BY valid_from DESC LIMIT 10001",
		conds.join(" AND ")
	);
	let mut stmt = db.prepare(&sql)?;
	let rows = stmt
		.query_map(params_from_iter(params.iter()), |r| {
			Ok(TripleRow {
				id:          r.get(0)?,
				subject:     r.get(1)?,
				predicate:   r.get(2)?,
				object:      r.get(3)?,
				valid_from:  r.get(4)?,
				valid_until: r.get(5)?,
				source:      r.get(6)?,
				confidence:  r.get(7)?,
				created_at:  r.get(8)?,
			})
		})?
		.collect::<rusqlite::Result<Vec<_>>>()?;
	if rows.len() > crate::limits::MAX_ROWS {
		return Err(crate::MnemopiError::Invalid("triple query exceeds row limit".into()));
	}
	Ok(rows)
}
