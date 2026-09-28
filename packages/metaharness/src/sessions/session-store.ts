/**
 * Isolated SQLite mirror for harness sessions.
 *
 * Own file (`<jobs-dir>/_manager/sessions.sqlite`), own tables. Never touches
 * `metaharness.sqlite` (`runs`/`trials`). The benchmark job dir stays the
 * source of truth; this store is a queryable cache rebuilt by `syncRun`.
 */
import { Database } from "bun:sqlite";
import * as fs from "node:fs";
import * as path from "node:path";
import type { HarnessSession, HarnessSessionListFilter, HarnessSessionStatus } from "./types";

const SCHEMA = `
CREATE TABLE IF NOT EXISTS harness_sessions (
	id TEXT PRIMARY KEY,
	run TEXT NOT NULL,
	trial TEXT NOT NULL,
	task TEXT NOT NULL DEFAULT '',
	status TEXT NOT NULL DEFAULT 'running',
	trace_path TEXT,
	session_file TEXT,
	cost_usd REAL NOT NULL DEFAULT 0,
	duration_ms INTEGER NOT NULL DEFAULT 0,
	detail TEXT NOT NULL DEFAULT '',
	updated_at INTEGER NOT NULL,
	live INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_harness_sessions_run ON harness_sessions(run);
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

interface SessionRow {
	id: string;
	run: string;
	trial: string;
	task: string;
	status: string;
	trace_path: string | null;
	session_file: string | null;
	cost_usd: number;
	duration_ms: number;
	detail: string;
	updated_at: number;
	live: number;
}

function rowToSession(r: SessionRow): HarnessSession {
	return {
		id: String(r.id),
		run: String(r.run),
		trial: String(r.trial),
		task: String(r.task),
		status: String(r.status) as HarnessSessionStatus,
		tracePath: r.trace_path === null ? null : String(r.trace_path),
		sessionFile: r.session_file === null ? null : String(r.session_file),
		costUsd: Number(r.cost_usd),
		durationMs: Number(r.duration_ms),
		detail: String(r.detail),
		updatedAt: Number(r.updated_at),
		live: Number(r.live) === 1,
	};
}

export class SessionStore {
	#db: Database;
	readonly jobsDir: string;

	constructor(jobsDir: string, dbPath?: string) {
		this.jobsDir = jobsDir;
		fs.mkdirSync(path.join(jobsDir, "_manager"), { recursive: true });
		this.#db = new Database(dbPath ?? path.join(jobsDir, "_manager", "sessions.sqlite"));
		this.#db.run("PRAGMA busy_timeout = 5000");
		enableWal(this.#db);
		this.#db.run(SCHEMA);
	}

	close(): void {
		this.#db.close();
	}

	upsert(session: HarnessSession): void {
		this.#db
			.query(
				`INSERT INTO harness_sessions
				 (id, run, trial, task, status, trace_path, session_file, cost_usd, duration_ms, detail, updated_at, live)
				 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
				 ON CONFLICT(id) DO UPDATE SET
					run = excluded.run, trial = excluded.trial, task = excluded.task,
					status = excluded.status, trace_path = excluded.trace_path,
					session_file = excluded.session_file, cost_usd = excluded.cost_usd,
					duration_ms = excluded.duration_ms, detail = excluded.detail,
					updated_at = excluded.updated_at, live = excluded.live`,
			)
			.run(
				session.id,
				session.run,
				session.trial,
				session.task,
				session.status,
				session.tracePath,
				session.sessionFile,
				session.costUsd,
				session.durationMs,
				session.detail,
				session.updatedAt,
				session.live ? 1 : 0,
			);
	}

	syncRunSessions(run: string, sessions: HarnessSession[]): void {
		const tx = this.#db.transaction(() => {
			if (sessions.length > 0) {
				const ids = sessions.map(s => s.id);
				this.#db
					.query(`DELETE FROM harness_sessions WHERE run = ? AND id NOT IN (${ids.map(() => "?").join(",")})`)
					.run(run, ...ids);
			} else {
				this.#db.query(`DELETE FROM harness_sessions WHERE run = ?`).run(run);
			}
			for (const s of sessions) this.upsert(s);
		});
		tx();
	}

	deleteRun(run: string): void {
		this.#db.query(`DELETE FROM harness_sessions WHERE run = ?`).run(run);
	}

	get(id: string): HarnessSession | null {
		const row = this.#db.query(`SELECT * FROM harness_sessions WHERE id = ?`).get(id) as SessionRow | null;
		return row ? rowToSession(row) : null;
	}

	list(filter: HarnessSessionListFilter = {}): HarnessSession[] {
		let sql = `SELECT * FROM harness_sessions`;
		const clauses: string[] = [];
		const args: unknown[] = [];
		if (filter.run) {
			clauses.push(`run = ?`);
			args.push(filter.run);
		}
		if (filter.status) {
			clauses.push(`status = ?`);
			args.push(filter.status);
		}
		if (clauses.length > 0) sql += ` WHERE ${clauses.join(" AND ")}`;
		sql += ` ORDER BY updated_at DESC`;
		const rows = this.#db.query(sql).all(...args) as SessionRow[];
		return rows.map(rowToSession);
	}
}
