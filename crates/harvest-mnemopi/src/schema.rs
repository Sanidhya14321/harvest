//! Schema bootstrap — exact port of `core/beam/schema.ts` (`initBeam`), the
//! `EpisodicGraph` tables, and the annotations migration target, so a database
//! created by either runtime is readable by the other.

use rusqlite::Connection;

use crate::{Result, util::now_iso};

fn add_column_if_missing(
	db: &Connection,
	table: &str,
	column: &str,
	definition: &str,
) -> Result<bool> {
	let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
	let exists = stmt
		.query_map([], |r| r.get::<_, String>(1))?
		.filter_map(|r| r.ok())
		.any(|name| name == column);
	if exists {
		return Ok(false);
	}
	db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"))?;
	Ok(true)
}

const WORKING_MEMORY: &str = "
CREATE TABLE IF NOT EXISTS working_memory (
	id TEXT PRIMARY KEY,
	content TEXT NOT NULL,
	embed_text TEXT DEFAULT NULL,
	source TEXT,
	timestamp TEXT,
	session_id TEXT DEFAULT 'default',
	importance REAL DEFAULT 0.5,
	metadata_json TEXT,
	veracity TEXT DEFAULT 'unknown',
	memory_type TEXT DEFAULT 'unknown',
	consolidated_at TEXT,
	recall_count INTEGER DEFAULT 0,
	last_recalled TIMESTAMP DEFAULT NULL,
	valid_until TIMESTAMP DEFAULT NULL,
	superseded_by TEXT DEFAULT NULL,
	scope TEXT DEFAULT 'global',
	author_id TEXT DEFAULT NULL,
	author_type TEXT DEFAULT NULL,
	channel_id TEXT DEFAULT NULL,
	trust_tier TEXT DEFAULT 'STATED',
	validator TEXT DEFAULT NULL,
	validated_at TIMESTAMP DEFAULT NULL,
	validation_count INTEGER DEFAULT 0,
	event_date TEXT DEFAULT NULL,
	event_date_precision TEXT DEFAULT 'unknown',
	temporal_tags TEXT DEFAULT '[]',
	corrected_by INTEGER DEFAULT NULL,
	created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);";

const EPISODIC_MEMORY: &str = "
CREATE TABLE IF NOT EXISTS episodic_memory (
	rowid INTEGER PRIMARY KEY AUTOINCREMENT,
	id TEXT UNIQUE NOT NULL,
	content TEXT NOT NULL,
	source TEXT,
	timestamp TEXT,
	session_id TEXT DEFAULT 'default',
	importance REAL DEFAULT 0.5,
	metadata_json TEXT,
	summary_of TEXT DEFAULT '',
	veracity TEXT DEFAULT 'unknown',
	tier INTEGER DEFAULT 1,
	degraded_at TEXT,
	memory_type TEXT DEFAULT 'unknown',
	binary_vector BLOB,
	recall_count INTEGER DEFAULT 0,
	last_recalled TIMESTAMP DEFAULT NULL,
	valid_until TIMESTAMP DEFAULT NULL,
	superseded_by TEXT DEFAULT NULL,
	scope TEXT DEFAULT 'global',
	author_id TEXT DEFAULT NULL,
	author_type TEXT DEFAULT NULL,
	channel_id TEXT DEFAULT NULL,
	trust_tier TEXT DEFAULT 'STATED',
	validator TEXT DEFAULT NULL,
	validated_at TIMESTAMP DEFAULT NULL,
	validation_count INTEGER DEFAULT 0,
	event_date TEXT DEFAULT NULL,
	event_date_precision TEXT DEFAULT 'unknown',
	temporal_tags TEXT DEFAULT '[]',
	corrected_by INTEGER DEFAULT NULL,
	created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_wm_session ON working_memory(session_id);
CREATE INDEX IF NOT EXISTS idx_wm_timestamp ON working_memory(timestamp);
CREATE INDEX IF NOT EXISTS idx_wm_source ON working_memory(source);
CREATE INDEX IF NOT EXISTS idx_em_session ON episodic_memory(session_id);
CREATE INDEX IF NOT EXISTS idx_em_timestamp ON episodic_memory(timestamp);
CREATE INDEX IF NOT EXISTS idx_em_source ON episodic_memory(source);";

const SCRATCHPAD_AND_FTS: &str = "
CREATE TABLE IF NOT EXISTS scratchpad (
	id TEXT PRIMARY KEY,
	content TEXT NOT NULL,
	session_id TEXT DEFAULT 'default',
	created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
	updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_sp_session ON scratchpad(session_id);
CREATE VIRTUAL TABLE IF NOT EXISTS fts_episodes USING fts5(
	content,
	content='episodic_memory',
	content_rowid='rowid'
);
CREATE VIRTUAL TABLE IF NOT EXISTS fts_working USING fts5(
	id UNINDEXED,
	content
);
CREATE TRIGGER IF NOT EXISTS em_ai AFTER INSERT ON episodic_memory BEGIN
	INSERT INTO fts_episodes(rowid, content) VALUES (new.rowid, new.content);
END;
CREATE TRIGGER IF NOT EXISTS em_ad AFTER DELETE ON episodic_memory BEGIN
	INSERT INTO fts_episodes(fts_episodes, rowid, content) VALUES ('delete', old.rowid, old.content);
END;
CREATE TRIGGER IF NOT EXISTS em_au AFTER UPDATE ON episodic_memory BEGIN
	INSERT INTO fts_episodes(fts_episodes, rowid, content) VALUES ('delete', old.rowid, old.content);
	INSERT INTO fts_episodes(rowid, content) VALUES (new.rowid, new.content);
END;
DROP TRIGGER IF EXISTS wm_ai;
CREATE TRIGGER IF NOT EXISTS wm_ai AFTER INSERT ON working_memory BEGIN
	INSERT INTO fts_working(id, content) VALUES (new.id, COALESCE(new.embed_text, new.content));
END;
CREATE TRIGGER IF NOT EXISTS wm_ad AFTER DELETE ON working_memory BEGIN
	DELETE FROM fts_working WHERE id = old.id;
END;
DROP TRIGGER IF EXISTS wm_au;
CREATE TRIGGER IF NOT EXISTS wm_au AFTER UPDATE OF content, embed_text ON working_memory BEGIN
	DELETE FROM fts_working WHERE id = old.id;
	INSERT INTO fts_working(id, content) VALUES (new.id, COALESCE(new.embed_text, new.content));
END;
CREATE TABLE IF NOT EXISTS memoria_facts (
	id INTEGER PRIMARY KEY AUTOINCREMENT,
	session_id TEXT DEFAULT 'default',
	message_idx INTEGER,
	fact_type TEXT,
	key TEXT,
	value TEXT,
	context_snippet TEXT,
	importance REAL DEFAULT 0.5,
	timestamp TEXT,
	version_id INTEGER DEFAULT 0,
	previous_value TEXT,
	updated_msg_idx INTEGER,
	valid_from_msg_idx INTEGER,
	valid_to_msg_idx INTEGER,
	source_memory_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_facts_key ON memoria_facts(key);
CREATE INDEX IF NOT EXISTS idx_facts_type ON memoria_facts(fact_type);
CREATE INDEX IF NOT EXISTS idx_facts_session ON memoria_facts(session_id);";

const MEMORIA: &str = "
CREATE TABLE IF NOT EXISTS memoria_timelines (
	event_id INTEGER PRIMARY KEY AUTOINCREMENT,
	session_id TEXT DEFAULT 'default',
	date TEXT,
	message_idx INTEGER,
	description TEXT,
	source TEXT,
	source_memory_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_timelines_date ON memoria_timelines(date);
CREATE INDEX IF NOT EXISTS idx_timelines_session ON memoria_timelines(session_id);
CREATE TABLE IF NOT EXISTS memoria_instructions (
	id INTEGER PRIMARY KEY AUTOINCREMENT,
	session_id TEXT DEFAULT 'default',
	message_idx INTEGER,
	instruction TEXT,
	active INTEGER DEFAULT 1,
	topic TEXT,
	context_snippet TEXT,
	source_memory_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_instr_session ON memoria_instructions(session_id);
CREATE INDEX IF NOT EXISTS idx_instr_active ON memoria_instructions(active);
CREATE TABLE IF NOT EXISTS memoria_preferences (
	id INTEGER PRIMARY KEY AUTOINCREMENT,
	session_id TEXT DEFAULT 'default',
	message_idx INTEGER,
	preference TEXT,
	topic TEXT,
	evolution TEXT,
	context_snippet TEXT,
	source_memory_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_pref_session ON memoria_preferences(session_id);
CREATE TABLE IF NOT EXISTS memoria_kg (
	id INTEGER PRIMARY KEY AUTOINCREMENT,
	session_id TEXT DEFAULT 'default',
	subject TEXT,
	predicate TEXT,
	object TEXT,
	message_idx INTEGER,
	confidence REAL DEFAULT 0.7,
	source_memory_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_kg_subject ON memoria_kg(subject);
CREATE INDEX IF NOT EXISTS idx_kg_predicate ON memoria_kg(predicate);
CREATE INDEX IF NOT EXISTS idx_kg_session ON memoria_kg(session_id);";

const LOG_AND_EMBEDDINGS: &str = "
CREATE TABLE IF NOT EXISTS consolidation_log (
	id INTEGER PRIMARY KEY AUTOINCREMENT,
	session_id TEXT,
	items_consolidated INTEGER,
	summary_preview TEXT,
	created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS memory_embeddings (
	memory_id TEXT PRIMARY KEY,
	embedding_json TEXT NOT NULL,
	model TEXT,
	created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);";

const VALIDATIONS_FACTS_ANNOTATIONS_TRIPLES: &str = "
CREATE TABLE IF NOT EXISTS memory_validations (
	validation_id INTEGER PRIMARY KEY AUTOINCREMENT,
	memory_id TEXT NOT NULL,
	validator TEXT NOT NULL,
	validated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
	action TEXT NOT NULL,
	new_content TEXT,
	note TEXT
);
CREATE INDEX IF NOT EXISTS idx_validations_memory ON memory_validations(memory_id);
CREATE INDEX IF NOT EXISTS idx_validations_validator ON memory_validations(validator);
CREATE TRIGGER IF NOT EXISTS trim_validations_to_3
AFTER INSERT ON memory_validations
BEGIN
	DELETE FROM memory_validations
	WHERE memory_id = NEW.memory_id
	  AND validation_id NOT IN (
		SELECT validation_id FROM memory_validations
		WHERE memory_id = NEW.memory_id
		ORDER BY validation_id DESC
		LIMIT 3
	  );
END;
CREATE TABLE IF NOT EXISTS facts (
	fact_id TEXT PRIMARY KEY,
	session_id TEXT NOT NULL,
	subject TEXT NOT NULL,
	predicate TEXT NOT NULL,
	object TEXT NOT NULL,
	timestamp TEXT,
	source_msg_id TEXT,
	confidence REAL DEFAULT 1.0,
	created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_facts_session ON facts(session_id);
CREATE INDEX IF NOT EXISTS idx_facts_subject ON facts(subject);
CREATE INDEX IF NOT EXISTS idx_facts_source ON facts(source_msg_id);
CREATE VIRTUAL TABLE IF NOT EXISTS fts_facts USING fts5(
	subject, predicate, object, content='facts'
);
CREATE TRIGGER IF NOT EXISTS facts_ai AFTER INSERT ON facts BEGIN
	INSERT INTO fts_facts(rowid, subject, predicate, object)
	VALUES (new.rowid, new.subject, new.predicate, new.object);
END;
CREATE TRIGGER IF NOT EXISTS facts_ad AFTER DELETE ON facts BEGIN
	INSERT INTO fts_facts(fts_facts, rowid, subject, predicate, object)
	VALUES ('delete', old.rowid, old.subject, old.predicate, old.object);
END;";

const ANNOTATIONS_TRIPLES: &str = "
CREATE TABLE IF NOT EXISTS annotations (
	id INTEGER PRIMARY KEY AUTOINCREMENT,
	memory_id TEXT NOT NULL,
	kind TEXT NOT NULL,
	value TEXT NOT NULL,
	source TEXT,
	confidence REAL DEFAULT 1.0,
	created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_annot_memory_kind ON annotations(memory_id, kind);
CREATE INDEX IF NOT EXISTS idx_annot_kind_value ON annotations(kind, value);
CREATE UNIQUE INDEX IF NOT EXISTS idx_annot_unique ON annotations(memory_id, kind, value);
CREATE TABLE IF NOT EXISTS triples (
	id INTEGER PRIMARY KEY AUTOINCREMENT,
	subject TEXT NOT NULL,
	predicate TEXT NOT NULL,
	object TEXT NOT NULL,
	valid_from TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
	valid_until TEXT,
	source TEXT,
	confidence REAL DEFAULT 1.0,
	created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_triples_subject ON triples(subject);
CREATE INDEX IF NOT EXISTS idx_triples_predicate ON triples(predicate);
CREATE INDEX IF NOT EXISTS idx_triples_object ON triples(object);
CREATE INDEX IF NOT EXISTS idx_triples_valid_from ON triples(valid_from);";

/// `EpisodicGraph#initTables`.
pub const GRAPH_TABLES: &str = "
CREATE TABLE IF NOT EXISTS gists (
	id TEXT PRIMARY KEY,
	text TEXT NOT NULL,
	timestamp TEXT,
	participants_json TEXT,
	location TEXT,
	emotion TEXT,
	time_scope TEXT,
	memory_id TEXT,
	created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS facts (
	fact_id TEXT PRIMARY KEY,
	session_id TEXT DEFAULT 'default',
	subject TEXT NOT NULL,
	predicate TEXT NOT NULL,
	object TEXT NOT NULL,
	timestamp TEXT,
	source_msg_id TEXT,
	confidence REAL DEFAULT 0.5,
	created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_facts_subject ON facts(subject);
CREATE INDEX IF NOT EXISTS idx_facts_predicate ON facts(predicate);
CREATE INDEX IF NOT EXISTS idx_facts_object ON facts(object);
CREATE INDEX IF NOT EXISTS idx_facts_source_msg ON facts(source_msg_id);
CREATE TABLE IF NOT EXISTS graph_edges (
	id INTEGER PRIMARY KEY AUTOINCREMENT,
	source TEXT NOT NULL,
	target TEXT NOT NULL,
	edge_type TEXT NOT NULL,
	weight REAL DEFAULT 1.0,
	timestamp TEXT,
	created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
	UNIQUE(source, target, edge_type)
);
CREATE INDEX IF NOT EXISTS idx_edges_source ON graph_edges(source);
CREATE INDEX IF NOT EXISTS idx_edges_target ON graph_edges(target);
CREATE INDEX IF NOT EXISTS idx_edges_type ON graph_edges(edge_type);";

/// Port of `initBeam(db)`; idempotent.
pub fn init_beam(db: &Connection) -> Result<()> {
	db.execute_batch(WORKING_MEMORY)?;
	db.execute_batch(EPISODIC_MEMORY)?;

	add_column_if_missing(db, "episodic_memory", "tier", "INTEGER DEFAULT 1")?;
	add_column_if_missing(db, "episodic_memory", "degraded_at", "TEXT")?;
	db.execute_batch("CREATE INDEX IF NOT EXISTS idx_em_tier ON episodic_memory(tier)")?;
	add_column_if_missing(db, "working_memory", "veracity", "TEXT DEFAULT 'unknown'")?;
	add_column_if_missing(db, "episodic_memory", "veracity", "TEXT DEFAULT 'unknown'")?;
	add_column_if_missing(db, "working_memory", "memory_type", "TEXT DEFAULT 'unknown'")?;
	add_column_if_missing(db, "working_memory", "embed_text", "TEXT DEFAULT NULL")?;
	add_column_if_missing(db, "episodic_memory", "memory_type", "TEXT DEFAULT 'unknown'")?;
	add_column_if_missing(db, "episodic_memory", "binary_vector", "BLOB")?;
	if add_column_if_missing(db, "working_memory", "consolidated_at", "TEXT")? {
		db.execute("UPDATE working_memory SET consolidated_at = ? WHERE consolidated_at IS NULL", [
			now_iso(),
		])?;
	}
	db.execute_batch(
		"CREATE INDEX IF NOT EXISTS idx_wm_unconsolidated ON working_memory(session_id, timestamp) \
		 WHERE consolidated_at IS NULL",
	)?;

	db.execute_batch(SCRATCHPAD_AND_FTS)?;
	for (col, def) in [
		("version_id", "INTEGER DEFAULT 0"),
		("previous_value", "TEXT"),
		("updated_msg_idx", "INTEGER"),
		("valid_from_msg_idx", "INTEGER"),
		("valid_to_msg_idx", "INTEGER"),
		("source_memory_id", "TEXT"),
	] {
		add_column_if_missing(db, "memoria_facts", col, def)?;
	}
	db.execute_batch(MEMORIA)?;
	for table in ["memoria_timelines", "memoria_instructions", "memoria_preferences", "memoria_kg"] {
		add_column_if_missing(db, table, "source_memory_id", "TEXT")?;
	}
	db.execute_batch(LOG_AND_EMBEDDINGS)?;

	for (table, col, def) in [
		("working_memory", "recall_count", "INTEGER DEFAULT 0"),
		("working_memory", "last_recalled", "TIMESTAMP DEFAULT NULL"),
		("episodic_memory", "recall_count", "INTEGER DEFAULT 0"),
		("episodic_memory", "last_recalled", "TIMESTAMP DEFAULT NULL"),
		("working_memory", "valid_until", "TIMESTAMP DEFAULT NULL"),
		("working_memory", "superseded_by", "TEXT DEFAULT NULL"),
		("working_memory", "scope", "TEXT DEFAULT 'global'"),
		("episodic_memory", "valid_until", "TIMESTAMP DEFAULT NULL"),
		("episodic_memory", "superseded_by", "TEXT DEFAULT NULL"),
		("episodic_memory", "scope", "TEXT DEFAULT 'global'"),
	] {
		add_column_if_missing(db, table, col, def)?;
	}
	db.execute_batch(
		"CREATE INDEX IF NOT EXISTS idx_em_scope_imp ON episodic_memory(scope, importance) WHERE \
		 superseded_by IS NULL;
		 CREATE INDEX IF NOT EXISTS idx_wm_session_recall ON working_memory(session_id, last_recalled) \
		 WHERE valid_until IS NULL;
		 CREATE INDEX IF NOT EXISTS idx_mem_emb_type ON memory_embeddings(memory_id, model);",
	)?;

	for table in ["working_memory", "episodic_memory"] {
		for (col, def) in [
			("author_id", "TEXT DEFAULT NULL"),
			("author_type", "TEXT DEFAULT NULL"),
			("channel_id", "TEXT DEFAULT NULL"),
			("trust_tier", "TEXT DEFAULT 'STATED'"),
			("validator", "TEXT DEFAULT NULL"),
			("validated_at", "TIMESTAMP DEFAULT NULL"),
			("validation_count", "INTEGER DEFAULT 0"),
		] {
			add_column_if_missing(db, table, col, def)?;
		}
	}
	db.execute_batch(
		"CREATE INDEX IF NOT EXISTS idx_wm_author ON working_memory(author_id);
		 CREATE INDEX IF NOT EXISTS idx_wm_channel ON working_memory(channel_id);
		 CREATE INDEX IF NOT EXISTS idx_em_author ON episodic_memory(author_id);
		 CREATE INDEX IF NOT EXISTS idx_em_channel ON episodic_memory(channel_id);
		 CREATE INDEX IF NOT EXISTS idx_wm_validator ON working_memory(validator);
		 CREATE INDEX IF NOT EXISTS idx_wm_validated_at ON working_memory(validated_at);",
	)?;
	db.execute_batch(VALIDATIONS_FACTS_ANNOTATIONS_TRIPLES)?;

	for table in ["working_memory", "episodic_memory"] {
		for (col, def) in [
			("event_date", "TEXT DEFAULT NULL"),
			("event_date_precision", "TEXT DEFAULT 'unknown'"),
			("temporal_tags", "TEXT DEFAULT '[]'"),
			("corrected_by", "INTEGER DEFAULT NULL"),
		] {
			add_column_if_missing(db, table, col, def)?;
		}
	}
	db.execute_batch(
		"CREATE INDEX IF NOT EXISTS idx_wm_event_date ON working_memory(event_date);
		 CREATE INDEX IF NOT EXISTS idx_em_event_date ON episodic_memory(event_date);",
	)?;
	db.execute_batch(ANNOTATIONS_TRIPLES)?;
	Ok(())
}

/// Full bootstrap performed by `new BeamMemory(...)`: beam tables + graph
/// tables.
pub fn init_all(db: &Connection) -> Result<()> {
	init_beam(db)?;
	db.execute_batch(GRAPH_TABLES)?;
	Ok(())
}

pub fn table_exists(db: &Connection, table: &str) -> bool {
	db.query_row(
		"SELECT 1 FROM sqlite_master WHERE type IN ('table','virtual table') AND name = ? LIMIT 1",
		[table],
		|_| Ok(()),
	)
	.is_ok()
}
