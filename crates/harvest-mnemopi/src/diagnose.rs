//! Read-only schema/integrity inspection of an already-open bank.

use serde::Serialize;

use crate::{BeamMemory, Result, schema, util};

#[derive(Debug, Serialize)]
pub struct DiagnosticEntry {
	pub ts:       String,
	pub category: &'static str,
	pub check:    String,
	pub status:   &'static str,
	pub detail:   String,
}

#[derive(Debug, Serialize)]
pub struct DiagnosticSummary {
	pub checks_total:  usize,
	pub checks_passed: usize,
	pub checks_failed: usize,
	pub key_findings:  Vec<String>,
	pub entries:       Vec<DiagnosticEntry>,
	pub database:      String,
}

/// Does not initialize tables, reconcile embeddings, or open another database.
pub fn inspect_database(beam: &BeamMemory) -> Result<DiagnosticSummary> {
	let mut entries = Vec::new();
	let mut integrity = beam.db.prepare("PRAGMA quick_check(10)")?;
	for detail in integrity.query_map([], |r| r.get::<_, String>(0))? {
		let detail = detail?;
		entries.push(DiagnosticEntry {
			ts: util::now_iso(),
			category: "db",
			check: "integrity_check".into(),
			status: if detail == "ok" { "OK" } else { "FAIL" },
			detail,
		});
	}
	for table in [
		"working_memory",
		"episodic_memory",
		"scratchpad",
		"fts_working",
		"fts_episodes",
		"memoria_facts",
		"memoria_timelines",
		"memoria_kg",
		"memoria_instructions",
		"memoria_preferences",
		"consolidation_log",
		"annotations",
		"triples",
	] {
		let exists = schema::table_exists(&beam.db, table);
		entries.push(DiagnosticEntry {
			ts:       util::now_iso(),
			category: "schema",
			check:    format!("table:{table}"),
			status:   if exists { "OK" } else { "MISSING" },
			detail:   String::new(),
		});
	}
	let key_findings: Vec<_> = entries
		.iter()
		.filter(|e| e.status != "OK")
		.map(|e| format!("{}: {} {}", e.check, e.status, e.detail))
		.collect();
	Ok(DiagnosticSummary {
		checks_total: entries.len(),
		checks_passed: entries.len() - key_findings.len(),
		checks_failed: key_findings.len(),
		key_findings,
		entries,
		database: beam
			.db_path
			.as_ref()
			.map_or_else(|| ":memory:".into(), |p| p.display().to_string()),
	})
}
