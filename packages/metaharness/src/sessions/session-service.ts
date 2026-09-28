/**
 * Harness session service: mirror-first, live-opt-in.
 *
 * - `off`: no-ops (zero I/O, zero coding-agent imports).
 * - `mirror` (default when enabled): `syncRun` indexes normalized benchmark
 *   snapshots into `SessionStore`.
 * - `live`: mirror plus a coding-agent `SessionManager` sidecar journal per
 *   trial under `<jobs-dir>/_manager/sessions/<run>/`. Live journals are
 *   created lazily via dynamic import so mirror mode never loads the agent.
 */
import * as path from "node:path";
import { readBenchmarkSnapshot } from "../benchmarks";
import type { RunStore } from "../store";
import { mirrorSnapshotToSessions } from "./session-mirror";
import { SessionStore } from "./session-store";
import type { HarnessSession, HarnessSessionListFilter } from "./types";

export type SessionsMode = "off" | "mirror" | "live";

export function sessionsModeFromEnv(env: NodeJS.ProcessEnv = process.env): SessionsMode {
	const raw = (env.META_SESSIONS ?? "off").toLowerCase();
	if (raw === "live" || raw === "1" || raw === "on") return "live";
	if (raw === "mirror") return "mirror";
	return "off";
}

export function assertSafeSessionId(id: string): void {
	if (!id || id === "." || id === ".." || /[/\\]/.test(id) && id.includes("..")) {
		throw new Error(`invalid session id: ${id}`);
	}
	if (id.length > 512) throw new Error(`invalid session id: too long`);
}

function sidecarDir(jobsDir: string, run: string): string {
	return path.join(jobsDir, "_manager", "sessions", run);
}

export class SessionService {
	readonly mode: SessionsMode;
	readonly jobsDir: string;
	readonly store: SessionStore;
	#liveFiles = new Map<string, string>();

	constructor(jobsDir: string, mode: SessionsMode = sessionsModeFromEnv(), dbPath?: string) {
		this.jobsDir = jobsDir;
		this.mode = mode;
		this.store = new SessionStore(jobsDir, dbPath);
	}

	get enabled(): boolean {
		return this.mode !== "off";
	}

	get live(): boolean {
		return this.mode === "live";
	}

	close(): void {
		this.store.close();
	}

	/** Mirror one run's latest snapshot into the session store. No-op when off. */
	syncRun(runStore: RunStore, jobName: string): void {
		if (!this.enabled) return;
		const run = runStore.getRun(jobName);
		if (!run) {
			this.store.deleteRun(jobName);
			return;
		}
		const jobDir = path.join(this.jobsDir, jobName);
		const snapshot = readBenchmarkSnapshot(run.benchmark, jobDir);
		const sessions = mirrorSnapshotToSessions(jobName, snapshot).map(s => {
			const liveFile = this.#liveFiles.get(s.id);
			return liveFile ? { ...s, sessionFile: liveFile, live: this.live } : s;
		});
		this.store.syncRunSessions(jobName, sessions);
	}

	syncAll(runStore: RunStore): void {
		if (!this.enabled) return;
		for (const run of runStore.listRuns()) this.syncRun(runStore, run.jobName);
	}

	list(filter: HarnessSessionListFilter = {}): HarnessSession[] {
		return this.store.list(filter);
	}

	get(id: string): HarnessSession | null {
		return this.store.get(id);
	}

	/** Ensure a live sidecar journal exists for a mirrored session. Mirror mode throws. */
	async ensureLive(sessionId: string): Promise<HarnessSession> {
		if (!this.live) throw new Error("live sessions are disabled (set META_SESSIONS=live)");
		const existing = this.store.get(sessionId);
		if (!existing) throw new Error(`session ${sessionId} not found`);
		const cached = this.#liveFiles.get(sessionId);
		if (cached) return { ...existing, sessionFile: cached, live: true };
		const { SessionManager } = await import("@harvest/pi-coding-agent/session/session-manager");
		const { FileSessionStorage } = await import("@harvest/pi-coding-agent/session/session-storage");
		const storage = new FileSessionStorage();
		const manager = SessionManager.create(existing.run, sidecarDir(this.jobsDir, existing.run), storage);
		const file = manager.getSessionFile();
		if (!file) throw new Error("live session manager produced no session file");
		await manager.flush();
		await manager.close();
		this.#liveFiles.set(sessionId, file);
		const updated: HarnessSession = { ...existing, sessionFile: file, live: true, updatedAt: Date.now() };
		this.store.upsert(updated);
		return updated;
	}

	/** Fork a session journal into a new sidecar file. Requires live mode. */
	async fork(sessionId: string): Promise<HarnessSession> {
		if (!this.live) throw new Error("live sessions are disabled (set META_SESSIONS=live)");
		const source = this.store.get(sessionId);
		if (!source) throw new Error(`session ${sessionId} not found`);
		const live = await this.ensureLive(sessionId);
		if (!live.sessionFile) throw new Error(`session ${sessionId} has no live journal`);
		const { SessionManager } = await import("@harvest/pi-coding-agent/session/session-manager");
		const { FileSessionStorage } = await import("@harvest/pi-coding-agent/session/session-storage");
		const storage = new FileSessionStorage();
		const forked = await SessionManager.forkFrom(
			live.sessionFile,
			live.run,
			sidecarDir(this.jobsDir, live.run),
			storage,
			{ copyArtifacts: false, suppressBreadcrumb: true, resetInheritedCost: true },
		);
		const file = forked.getSessionFile();
		if (!file) throw new Error("fork produced no session file");
		await forked.flush();
		await forked.close();
		const forkedId = `${live.id}:fork-${Date.now()}`;
		const record: HarnessSession = { ...live, id: forkedId, sessionFile: file, live: true, updatedAt: Date.now() };
		this.store.upsert(record);
		return record;
	}

	/** Re-open a live journal so a fresh `SessionManager` sees the same transcript. */
	async resume(sessionId: string): Promise<HarnessSession> {
		if (!this.live) throw new Error("live sessions are disabled (set META_SESSIONS=live)");
		const record = this.store.get(sessionId);
		if (!record) throw new Error(`session ${sessionId} not found`);
		if (!record.sessionFile) return this.ensureLive(sessionId);
		const { SessionManager } = await import("@harvest/pi-coding-agent/session/session-manager");
		const { FileSessionStorage } = await import("@harvest/pi-coding-agent/session/session-storage");
		const storage = new FileSessionStorage();
		const manager = SessionManager.create(record.run, sidecarDir(this.jobsDir, record.run), storage);
		await manager.setSessionFile(record.sessionFile);
		await manager.flush();
		await manager.close();
		return { ...record, live: true, updatedAt: Date.now() };
	}
}
