//! Episodic graph edges (`core/episodic-graph.ts` `addEdge` /
//! `findRelatedMemories`).

use std::collections::HashSet;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
	pub source:    String,
	pub target:    String,
	pub edge_type: String,
	pub weight:    f64,
	pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedMemory {
	pub memory_id: String,
	pub edge_type: String,
	pub weight:    f64,
	pub depth:     u32,
}

fn clamp_weight(w: f64) -> f64 {
	if w.is_finite() {
		w.clamp(0.0, 1.0)
	} else {
		0.0
	}
}

pub fn add_edge(db: &Connection, edge: &GraphEdge) -> Result<()> {
	db.execute(
		"INSERT INTO graph_edges (source, target, edge_type, weight, timestamp)
		 VALUES (?, ?, ?, ?, ?)
		 ON CONFLICT(source, target, edge_type) DO UPDATE SET
			weight = excluded.weight,
			timestamp = excluded.timestamp",
		(&edge.source, &edge.target, &edge.edge_type, clamp_weight(edge.weight), &edge.timestamp),
	)?;
	Ok(())
}

pub fn find_related_memories(
	db: &Connection,
	memory_id: &str,
	depth: u32,
	edge_type: &str,
	min_weight: f64,
) -> Result<Vec<RelatedMemory>> {
	if depth > 8 {
		return Err(crate::MnemopiError::Invalid("graph depth exceeds 8".into()));
	}
	let threshold = clamp_weight(min_weight);
	let mut results = Vec::new();
	let mut seen: HashSet<String> = HashSet::from([memory_id.to_string()]);
	let mut current: Vec<String> = vec![memory_id.to_string()];
	for hop in 1..=depth {
		let mut next = Vec::new();
		for mem in &current {
			let rows: Vec<(String, String, String, f64)> = if edge_type.is_empty() {
				let mut s = db.prepare(
					"SELECT source, target, edge_type, weight FROM graph_edges
					 WHERE (source = ? OR target = ?) AND weight >= ? ORDER BY weight DESC, id LIMIT 1001",
				)?;
				s.query_map((mem, mem, threshold), |r| {
					Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
				})?
				.collect::<rusqlite::Result<_>>()?
			} else {
				let mut s = db.prepare(
					"SELECT source, target, edge_type, weight FROM graph_edges
					 WHERE (source = ? OR target = ?) AND edge_type = ? AND weight >= ? ORDER BY weight DESC, id \
					 LIMIT 1001",
				)?;
				s.query_map((mem, mem, edge_type, threshold), |r| {
					Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
				})?
				.collect::<rusqlite::Result<_>>()?
			};
			if rows.len() > 1000 {
				return Err(crate::MnemopiError::Invalid("graph degree exceeds 1000".into()));
			}
			for (source, target, et, weight) in rows {
				let neighbor = if &source == mem { target } else { source };
				if !seen.insert(neighbor.clone()) {
					continue;
				}
				next.push(neighbor.clone());
				results.push(RelatedMemory { memory_id: neighbor, edge_type: et, weight, depth: hop });
				if results.len() >= crate::limits::MAX_ROWS {
					return Err(crate::MnemopiError::Invalid(
						"graph traversal exceeds node limit".into(),
					));
				}
			}
		}
		current = next;
	}
	Ok(results)
}
