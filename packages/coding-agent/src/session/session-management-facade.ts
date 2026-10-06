import * as path from "node:path";
import { getAgentDir, isEnoent, logger, Serial } from "@harvest/pi-utils";
import type { AgentSession } from "./agent-session";
import type { LiveSessionRegistry, LiveSessionSnapshot } from "./live-session-registry";
import { SessionManager } from "./session-manager";
import type { SessionTabs } from "./session-tabs";
import type { SessionStorage } from "./session-storage";
import type { SessionViewStateStore } from "./session-view-state";

const ARCHIVE_FILENAME = "archived-sessions.json";
const ARCHIVE_VERSION = 1;
const LEGACY_ARCHIVE_FILENAME = "session-archive.json";
const SETTLE_TIMEOUT_MS = 5000;
const SETTLE_POLL_MS = 100;

export type ManagedSessionStatus = "idle" | "running" | "waiting" | "completed" | "error" | "archived";

export interface ManagedSessionHandle {
	id: string;
	path: string | undefined;
	title: string | undefined;
	status: ManagedSessionStatus;
	selected: boolean;
	visible: boolean;
	archived: boolean;
}

export interface SessionCreateOptions {
	background?: boolean;
	cwd?: string;
	onCreated?: (session: AgentSession) => void;
}

export interface SessionFacadeDeps {
	live: LiveSessionRegistry;
	tabs: SessionTabs;
	viewState: SessionViewStateStore;
	storage: SessionStorage;
	createSession: (opts?: SessionCreateOptions) => Promise<AgentSession>;
	openSession: (path: string) => Promise<AgentSession>;
	directories: { cwd: string; sessionDir: string };
	/** Legacy archive location, migration input only. Default: getAgentDir(). */
	legacyAgentDir?: string;
}

/**
 * Typed management facade over the existing session owners.
 *
 * Ownership retained: LiveSessionRegistry owns live runtimes, SessionTabs owns
 * visibility + navigation history, SessionManager/storage own durable sessions,
 * InteractiveMode owns presentation. This facade only coordinates those owners
 * through stable session IDs; paths are persistence references.
 *
 * Close/Stop/Archive/Delete are separate: close hides the view and preserves
 * the runtime, stop aborts the run, archive sets reversible versioned
 * metadata through the session storage abstraction (no filesystem moves),
 * delete removes persistence after settlement and seals the writer so late
 * callbacks can never resurrect the file.
 */
export class SessionManagementFacade {
	readonly #serial = new Serial();
	#generation = 0;
	readonly #tombstones = new Set<string>();
	#archived: Set<string> | undefined;
	#archiveLoaded = false;

	readonly #live: LiveSessionRegistry;
	readonly #tabs: SessionTabs;
	readonly #viewState: SessionViewStateStore;
	readonly #storage: SessionStorage;
	readonly #createSession: (opts?: SessionCreateOptions) => Promise<AgentSession>;
	readonly #openSession: (path: string) => Promise<AgentSession>;
	readonly #directories: { cwd: string; sessionDir: string };
	readonly #legacyAgentDir: string;

	constructor(deps: SessionFacadeDeps) {
		this.#live = deps.live;
		this.#tabs = deps.tabs;
		this.#viewState = deps.viewState;
		this.#storage = deps.storage;
		this.#createSession = deps.createSession;
		this.#openSession = deps.openSession;
		this.#directories = deps.directories;
		this.#legacyAgentDir = deps.legacyAgentDir ?? getAgentDir();
		void this.#loadArchived().catch(error => logger.warn("Failed to load session archive", { error }));
	}

	/** True once a session's persistence was deleted; background callbacks must not write to it. */
	isDeleted(sessionId: string): boolean {
		return this.#tombstones.has(sessionId);
	}

	#archivePath(): string {
		return path.join(this.#directories.sessionDir, ARCHIVE_FILENAME);
	}

	async #loadArchived(): Promise<Set<string>> {
		if (this.#archiveLoaded && this.#archived) return this.#archived;
		const ids = new Set<string>();
		try {
			const raw: unknown = JSON.parse(await this.#storage.readText(this.#archivePath()));
			if (typeof raw === "object" && raw !== null && (raw as { version?: unknown }).version === ARCHIVE_VERSION) {
				for (const id of (raw as { ids?: unknown }).ids as unknown[]) {
					if (typeof id === "string") ids.add(id);
				}
			}
		} catch (err) {
			if (!isEnoent(err)) logger.warn("Failed to read session archive", { error: err });
		}
		// One-time migration input: legacy agentDir JSON merges in, then is
		// removed so it never becomes a parallel authoritative store.
		try {
			const legacy: unknown = await Bun.file(path.join(this.#legacyAgentDir, LEGACY_ARCHIVE_FILENAME)).json();
			if (Array.isArray(legacy)) {
				for (const id of legacy) if (typeof id === "string") ids.add(id);
				await this.#storage.unlink(path.join(this.#legacyAgentDir, LEGACY_ARCHIVE_FILENAME)).catch(() => {});
			}
		} catch (err) {
			if (!isEnoent(err)) logger.warn("Failed to migrate legacy session archive", { error: err });
		}
		this.#archived = ids;
		this.#archiveLoaded = true;
		return ids;
	}

	async #saveArchived(): Promise<void> {
		const ids = this.#archived ?? new Set<string>();
		await this.#storage.writeTextAtomic(
			this.#archivePath(),
			JSON.stringify({ version: ARCHIVE_VERSION, ids: [...ids] }, null, "\t"),
		);
	}

	async archivedIds(): Promise<ReadonlySet<string>> {
		return this.#loadArchived();
	}

	/** Live snapshots projected as handles (sync; archived flags come from the cached set). */
	list(): ManagedSessionHandle[] {
		const archived = this.#archived ?? new Set<string>();
		const visiblePaths = new Set(this.#tabs.paths.map(p => p));
		return this.#live.snapshots.map(snapshot => ({
			id: snapshot.id,
			path: snapshot.path || undefined,
			title: snapshot.title,
			status: archived.has(snapshot.id) ? "archived" : snapshot.status,
			selected: snapshot.selected,
			visible: snapshot.path ? visiblePaths.has(snapshot.path) : false,
			archived: archived.has(snapshot.id),
		}));
	}

	/** Live plus persisted cold sessions (async; loads archive metadata first). */
	async listAll(): Promise<ManagedSessionHandle[]> {
		const archived = await this.#loadArchived();
		const handles = this.list().map(handle => ({
			...handle,
			status: archived.has(handle.id) ? ("archived" as const) : handle.status,
			archived: archived.has(handle.id),
		}));
		const known = new Set(handles.map(handle => handle.id));
		let cold: { id: string; path: string; title?: string }[] = [];
		try {
			cold = await SessionManager.list(this.#directories.cwd, this.#directories.sessionDir, this.#storage);
		} catch (err) {
			logger.warn("Failed to list persisted sessions", { error: err });
		}
		const visiblePaths = new Set(this.#tabs.paths.map(p => p));
		for (const session of cold) {
			if (known.has(session.id)) continue;
			handles.push({
				id: session.id,
				path: session.path,
				title: session.title,
				status: archived.has(session.id) ? "archived" : "idle",
				selected: false,
				visible: visiblePaths.has(session.path),
				archived: archived.has(session.id),
			});
		}
		return handles;
	}

	inspect(sessionId: string): ManagedSessionHandle | undefined {
		return this.list().find(handle => handle.id === sessionId);
	}

	async create(opts?: SessionCreateOptions): Promise<ManagedSessionHandle> {
		return this.#serial.run(async () => {
			const session = await this.#createSession(opts);
			const id = session.sessionManager.getSessionId();
			const file = session.sessionManager.getSessionFile();
			if (file) {
				this.#tabs.open(file, session.sessionManager.getSessionName());
				this.#tabs.noteId(file, id);
				this.#live.notePath(id, file);
			}
			return (
				this.inspect(id) ?? {
					id,
					path: file ?? undefined,
					title: session.sessionManager.getSessionName(),
					status: "idle" as const,
					selected: false,
					visible: true,
					archived: false,
				}
			);
		});
	}

	/** Warm-runtime lookup first; disk-file checks only for cold opens. */
	async select(sessionId: string): Promise<{ switched: boolean }> {
		return this.#serial.run(async () => {
			const generation = ++this.#generation;
			const snapshots = this.#live.snapshots;
			const target = snapshots.find(snapshot => snapshot.id === sessionId);
			let session: AgentSession;
			if (target) {
				try {
					session = this.#live.selectById(sessionId).session;
				} catch {
					if (!target.path) return { switched: false };
					const selection = await this.#live.select(target.path);
					if (!selection.selected || generation !== this.#generation) return { switched: false };
					session = selection.session;
				}
			} else {
				return { switched: false };
			}
			if (generation !== this.#generation) return { switched: false };
			const file = session.sessionManager.getSessionFile();
			if (file) this.#tabs.visit(file);
			return { switched: true };
		});
	}

	async rename(sessionId: string, title: string): Promise<void> {
		const session = this.#live.sessions.find(s => s.sessionManager.getSessionId() === sessionId);
		if (!session) throw new Error(`Session ${sessionId} is not live`);
		await session.setSessionName(title, "user");
	}

	/** Hide the view only; never aborts, disposes, archives, or deletes the runtime. */
	async close(sessionId: string): Promise<{ lastTab: boolean }> {
		return this.#serial.run(async () => {
			const target = this.#live.snapshots.find(snapshot => snapshot.id === sessionId);
			if (target?.path) this.#tabs.close(target.path, true);
			return { lastTab: this.#tabs.paths.length === 0 };
		});
	}

	/**
	 * Reopen the exact requested session: warm runtime when live, otherwise a
	 * cold open of that session's persisted file adopted into the registry.
	 * Archived sessions must be restored first.
	 */
	async reopen(sessionId: string): Promise<{ switched: boolean }> {
		const archived = await this.#loadArchived();
		if (archived.has(sessionId)) throw new Error(`Session ${sessionId} is archived; restore it first`);
		if (this.#live.snapshots.some(snapshot => snapshot.id === sessionId)) {
			return this.select(sessionId);
		}
		let cold: { id: string; path: string }[] = [];
		try {
			cold = await SessionManager.list(this.#directories.cwd, this.#directories.sessionDir, this.#storage);
		} catch (err) {
			logger.warn("Failed to list persisted sessions for reopen", { error: err });
		}
		const match = cold.find(session => session.id === sessionId);
		if (!match) return { switched: false };
		const session = await this.#openSession(match.path);
		if (session.sessionManager.getSessionId() !== sessionId) {
			await session.dispose().catch(() => {});
			throw new Error(`Session file for ${sessionId} resolved to a different session`);
		}
		this.#live.adopt(session);
		this.#tabs.open(match.path, session.sessionManager.getSessionName());
		this.#tabs.noteId(match.path, sessionId);
		return { switched: true };
	}

	/** Reopen the most recently closed tab (bounded closed-tab history). */
	async reopenLast(): Promise<{ switched: boolean; sessionId?: string }> {
		const closed = this.#tabs.reopen();
		if (!closed) return { switched: false };
		const sessionId = this.#tabs.idForPath(closed);
		if (sessionId) return { ...(await this.reopen(sessionId)), sessionId };
		const selection = await this.#live.select(closed);
		return { switched: selection.selected };
	}

	/** Abort only the requested run through existing cancellation. */
	async stop(sessionId: string): Promise<void> {
		await this.#live.stop(sessionId);
	}

	#isBusySession(session: AgentSession | undefined, snapshot: LiveSessionSnapshot | undefined): boolean {
		if (!session) return snapshot?.status === "running" || snapshot?.status === "waiting";
		return (
			session.isStreaming ||
			session.isBashRunning ||
			session.isEvalRunning ||
			session.hasPendingAsyncWork() ||
			session.isCompacting ||
			snapshot?.status === "waiting"
		);
	}

	async archive(sessionId: string): Promise<void> {
		const snapshots = this.#live.snapshots;
		const session = this.#live.sessions.find(s => s.sessionManager.getSessionId() === sessionId);
		if (
			this.#isBusySession(
				session,
				snapshots.find(snapshot => snapshot.id === sessionId),
			)
		) {
			throw new Error(`Session ${sessionId} is busy; stop it before archiving`);
		}
		const archived = await this.#loadArchived();
		archived.add(sessionId);
		await this.#saveArchived();
		const target = snapshots.find(snapshot => snapshot.id === sessionId);
		if (target?.path) this.#tabs.close(target.path, false);
	}

	async restore(sessionId: string): Promise<void> {
		const archived = await this.#loadArchived();
		if (!archived.delete(sessionId)) return;
		await this.#saveArchived();
	}

	/**
	 * Target-aware delete. Ordinary delete rejects busy targets (streaming,
	 * shell/Eval, pending async work, approvals, compaction). With stopFirst,
	 * stops through existing cancellation, waits for settlement up to a
	 * timeout (rejection on timeout — never proceeds unsettled), seals the
	 * writer, deletes storage/artifacts, tombstones the ID, then disposes and
	 * detaches the runtime. Deletion failure retains discoverability.
	 */
	async delete(sessionId: string, opts?: { stopFirst?: boolean }): Promise<void> {
		const snapshot = this.#live.snapshots.find(entry => entry.id === sessionId);
		const session = this.#live.sessions.find(s => s.sessionManager.getSessionId() === sessionId);
		if (this.#isBusySession(session, snapshot) && !opts?.stopFirst) {
			throw new Error(`Session ${sessionId} is busy; stop it first or delete with stop-and-delete`);
		}
		await this.#serial.run(async () => {
			if (session && opts?.stopFirst) {
				await this.#live.stop(sessionId);
				const deadline = Date.now() + SETTLE_TIMEOUT_MS;
				for (;;) {
					if (!this.#isBusySession(session, undefined)) break;
					if (Date.now() >= deadline) {
						throw new Error(
							`Session ${sessionId} did not settle within ${SETTLE_TIMEOUT_MS}ms; deletion refused, session remains discoverable`,
						);
					}
					await Bun.sleep(SETTLE_POLL_MS);
				}
			}
			const target = this.#live.snapshots.find(entry => entry.id === sessionId);
			const file = target?.path ?? session?.sessionManager.getSessionFile() ?? undefined;
			if (target?.path) this.#tabs.close(target.path, false);
			this.#viewState.clearDraft(sessionId);
			this.#viewState.saveReadingAnchor(sessionId, undefined);
			// Seal the writer BEFORE unlinking so concurrent appends during
			// deletion cannot recreate the file after it is gone.
			session?.sessionManager.seal();
			try {
				if (file) {
					await this.#storage.deleteSessionWithArtifacts(file);
				}
			} catch (error) {
				throw new Error(
					`Failed to delete session ${sessionId}: ${error instanceof Error ? error.message : String(error)}. Session remains discoverable.`,
				);
			}
			this.#tombstones.add(sessionId);
			const archived = await this.#loadArchived();
			if (archived.delete(sessionId)) await this.#saveArchived();
			if (session) {
				try {
					await session.dispose();
				} catch (error) {
					logger.warn("Failed to dispose deleted session runtime", { sessionId, error });
				}
				this.#live.detach(sessionId);
			}
			await this.#storage.drain();
		});
	}

	async serializeTabs(projectKey: string, active?: { path?: string; sessionId?: string }): Promise<unknown> {
		const { snapshotSessionTabs } = await import("./session-tab-persistence");
		return snapshotSessionTabs(this.#tabs, {
			projectKey,
			activePath: active?.path,
			activeSessionId: active?.sessionId,
		});
	}
}

export type { LiveSessionSnapshot };
