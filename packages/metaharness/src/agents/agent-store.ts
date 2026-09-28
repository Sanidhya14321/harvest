/**
 * Isolated SQLite mirror for harness agents. Own file, own tables — never
 * touches `metaharness.sqlite`.
 */
import { Database } from "bun:sqlite";
import * as fs from "node:fs";
import * as path from "node:path";
import type { HarnessAgent, HarnessAgentListFilter } from "./types";

const SCHEMA = `
CREATE TABLE IF NOT EXISTS harness_agents (
	id TEXT PRIMARY KEY,
	display_name TEXT NOT NULL DEFAULT '',
	kind TEXT NOT NULL DEFAULT 'trial-worker',
	run TEXT NOT NULL DEFAULT '',
	trial TEXT,
	status TEXT NOT NULL DEFAULT 'running',
	pid INTEGER,
	session_id TEXT,
	session_file TEXT,
	created_at INTEGER NOT NULL,
	last_activity INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_harness_agents_run ON harness_agents(run);
`;

function isBusyLock(err: unknown): boolean {
	if (err && typeof err === "object" && "code" in err) {
		const code = (err as { code?: unknown }).code;
		return typeof code === "string" && code.startsWith("SQLITE_BUSY");
	}
	return false;
}

function enableWal(db: Database): void {
	for (let attempt = 1; attempt <= 10; attempt++) {
		try {
			db.run("PRAGMA journal_mode = WAL");
			return;
		} catch (err) {
			if (attempt < 10 && isBusyLock(err)) {
				Bun.sleepSync(100);
				continue;
			}
			throw err;
		}
	}
}

interface AgentRow {
	id: string;
	display_name: string;
	kind: string;
	run: string;
	trial: string | null;
	status: string;
	pid: number | null;
	session_id: string | null;
	session_file: string | null;
	created_at: number;
	last_activity: number;
}

function rowToAgent(r: AgentRow): HarnessAgent {
	return {
		id: String(r.id),
		displayName: String(r.display_name),
		kind: (String(r.kind) === "benchmark-runner" ? "benchmark-runner" : "trial-worker") as HarnessAgent["kind"],
		run: String(r.run),
		trial: r.trial === null ? null : String(r.trial),
		status: String(r.status) as HarnessAgent["status"],
		pid: r.pid === null ? null : Number(r.pid),
		sessionId: r.session_id === null ? null : String(r.session_id),
		sessionFile: r.session_file === null ? null : String(r.session_file),
		createdAt: Number(r.created_at),
		lastActivity: Number(r.last_activity),
	};
}

export class AgentStore {
	#db: Database;
	readonly jobsDir: string;

	constructor(jobsDir: string, dbPath?: string) {
		this.jobsDir = jobsDir;
		fs.mkdirSync(path.join(jobsDir, "_manager"), { recursive: true });
		this.#db = new Database(dbPath ?? path.join(jobsDir, "_manager", "agents.sqlite"));
		this.#db.run("PRAGMA busy_timeout = 5000");
		enableWal(this.#db);
		this.#db.run(SCHEMA);
	}

	close(): void {
		this.#db.close();
	}

	upsert(agent: HarnessAgent): void {
		this.#db
			.query(
				`INSERT INTO harness_agents
				 (id, display_name, kind, run, trial, status, pid, session_id, session_file, created_at, last_activity)
				 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
				 ON CONFLICT(id) DO UPDATE SET
					display_name = excluded.display_name, kind = excluded.kind, run = excluded.run,
					trial = excluded.trial, status = excluded.status, pid = excluded.pid,
					session_id = excluded.session_id, session_file = excluded.session_file,
					last_activity = excluded.last_activity`,
			)
			.run(
				agent.id,
				agent.displayName,
				agent.kind,
				agent.run,
				agent.trial,
				agent.status,
				agent.pid,
				agent.sessionId,
				agent.sessionFile,
				agent.createdAt,
				agent.lastActivity,
			);
	}

	remove(id: string): void {
		this.#db.query(`DELETE FROM harness_agents WHERE id = ?`).run(id);
	}

	get(id: string): HarnessAgent | null {
		const row = this.#db.query(`SELECT * FROM harness_agents WHERE id = ?`).get(id) as AgentRow | null;
		return row ? rowToAgent(row) : null;
	}

	list(filter: HarnessAgentListFilter = {}): HarnessAgent[] {
		let sql = `SELECT * FROM harness_agents`;
		const clauses: string[] = [];
		const args: string[] = [];
		if (filter.run) {
			clauses.push(`run = ?`);
			args.push(filter.run);
		}
		if (filter.status) {
			clauses.push(`status = ?`);
			args.push(filter.status);
		}
		if (clauses.length > 0) sql += ` WHERE ${clauses.join(" AND ")}`;
		sql += ` ORDER BY last_activity DESC`;
		const rows = this.#db.query(sql).all(...args) as AgentRow[];
		return rows.map(rowToAgent);
	}
}
