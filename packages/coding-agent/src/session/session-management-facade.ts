import * as path from "node:path";
import { getAgentDir, isEnoent, logger, Serial } from "@harvest/pi-utils";
import { withFileLock } from "@harvest/pi-utils/file-lock";
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

/**
 * Marker carried on deletion-failure errors when the facade recovered a
 * usable durable runtime under the same UUID (S4): the storage removal
 * failed, but the session was reopened from its intact file and re-adopted,
 * so the caller can re-point its view instead of showing a dead tab.
 */
export interface SessionDeleteRecovery {
	readonly sessionRecovered: true;
	readonly sessionId: string;
}

export function isSessionDeleteRecovery(error: unknown): error is Error & SessionDeleteRecovery {
	return (
		error instanceof Error &&
		(error as { sessionRecovered?: unknown }).sessionRecovered === true &&
		typeof (error as { sessionId?: unknown }).sessionId === "string"
	);
}

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

/**
 * Process-wide archive coordinators, one per canonical archive path. Two
 * facades over the same project share one Serial, so concurrent archive /
 * restore / delete mutations in this process serialize instead of
 * interleaving read-modify-write cycles (instance-local serials clobbered
 * each other). Cross-process exclusion comes from the OS-backed file lock
 * held around each coordinated section.
 */
const archiveSerials = new Map<string, Serial>();

function archiveSerialFor(archivePath: string): Serial {
	const key = path.resolve(archivePath);
	let serial = archiveSerials.get(key);
	if (!serial) {
		serial = new Serial();
		archiveSerials.set(key, serial);
	}
	return serial;
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
	/** Single-flight for concurrent archive loads racing at startup. */
	#archivePending: Promise<void> | undefined;
	#generation = 0;
	readonly #tombstones = new Set<string>();
	/**
	 * Last-known archive set for the synchronous {@link list} projection.
	 * The on-disk file is the canonical authority: every async path re-reads
	 * it fresh (mutations under the shared serial + file lock), so a sibling
	 * facade's publish is never masked by this cache.
	 */
	#archived: Set<string> | undefined;
	/** Legacy migration input consumed (attempted) by this facade. */
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
		return this.#authoritativeArchived();
	}

	/** Raw on-disk archive set. No cache, no migration: the building block for coordinated sections. */
	async #readArchiveFromDisk(): Promise<Set<string>> {
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
		return ids;
	}

	/**
	 * Consume the legacy migration input exactly once per facade, coordinated
	 * across facades sharing this project. Lock ordering is fixed everywhere:
	 * the archive file lock first, legacy-file work second — never the
	 * reverse — so concurrent loads cannot interleave read-merge-write
	 * cycles. A racing sibling consumes the legacy source first; this load
	 * then sees it already gone and keeps the sibling's published set.
	 */
	async #ensureMigrationConsumed(): Promise<void> {
		if (this.#archiveLoaded) return;
		if (!this.#archivePending) {
			const archivePath = this.#archivePath();
			const pending = archiveSerialFor(archivePath)
				.run(() => withFileLock(archivePath, () => this.#migrateOnceLocked()))
				.then(() => {});
			this.#archivePending = pending;
			void pending
				.finally(() => {
					if (this.#archivePending === pending) this.#archivePending = undefined;
				})
				.catch(() => {});
		}
		await this.#archivePending;
	}

	async #migrateOnceLocked(): Promise<void> {
		if (this.#archiveLoaded) return;
		const ids = await this.#readArchiveFromDisk();
		await this.#migrateLegacyArchiveLocked(ids);
		this.#archived = ids;
		this.#archiveLoaded = true;
	}

	/**
	 * Canonical authoritative read: migration input first, then fresh on-disk
	 * state. The cache mirrors the result but never masks it — sibling
	 * facades (same process or another) publish straight to the file this
	 * reads, so concurrent archive/restore cycles always converge.
	 */
	async #authoritativeArchived(): Promise<Set<string>> {
		await this.#ensureMigrationConsumed();
		const ids = await this.#readArchiveFromDisk();
		this.#archived = ids;
		return ids;
	}

	/**
	 * One-time migration input: the legacy agentDir JSON merges into this
	 * project's per-project store. Only IDs present in this project's session
	 * store migrate here — everything else stays in the legacy file so other
	 * projects never lose their archived IDs. The merged archive is published
	 * atomically BEFORE the legacy source is touched; on any write failure
	 * the legacy source is preserved verbatim and migration retries next load.
	 */
	async #migrateLegacyArchiveLocked(ids: Set<string>): Promise<void> {
		const legacyPath = path.join(this.#legacyAgentDir, LEGACY_ARCHIVE_FILENAME);
		let legacy: unknown;
		try {
			legacy = await Bun.file(legacyPath).json();
		} catch (err) {
			if (!isEnoent(err)) logger.warn("Failed to migrate legacy session archive", { error: err });
			return;
		}
		if (!Array.isArray(legacy)) return;
		const legacyIds = legacy.filter((id): id is string => typeof id === "string");
		if (legacyIds.length === 0) {
			await this.#storage.unlink(legacyPath).catch(() => {});
			return;
		}
		// Attribute legacy IDs to this project. On lookup failure the legacy
		// source is preserved untouched — migration retries on the next load.
		let localIds: Set<string>;
		try {
			const local = await SessionManager.list(this.#directories.cwd, this.#directories.sessionDir, this.#storage);
			localIds = new Set(local.map(session => session.id));
		} catch (err) {
			logger.warn("Failed to attribute legacy session archive; source preserved", { error: err });
			return;
		}
		const mine = legacyIds.filter(id => localIds.has(id));
		if (mine.length === 0) return;
		for (const id of mine) ids.add(id);
		this.#archived = ids;
		try {
			await this.#storage.writeTextAtomic(
				this.#archivePath(),
				JSON.stringify({ version: ARCHIVE_VERSION, ids: [...ids] }, null, "\t"),
			);
		} catch (err) {
			for (const id of mine) ids.delete(id);
			logger.warn("Failed to publish migrated session archive; legacy source preserved", { error: err });
			return;
		}
		// Publish succeeded: consume only our IDs. Other-project IDs stay in
		// the legacy file; the file is removed only when fully consumed.
		const remainder = legacyIds.filter(id => !localIds.has(id));
		try {
			if (remainder.length === 0) await this.#storage.unlink(legacyPath);
			else await this.#storage.writeTextAtomic(legacyPath, JSON.stringify(remainder));
		} catch (err) {
			logger.warn("Failed to consume migrated legacy session archive", { error: err });
		}
	}

	/**
	 * Coordinated archive mutation: shared-serial → OS file lock → fresh
	 * read → mutate → atomic publish → update cache. Additions merge onto
	 * current disk state; removals delete exactly the target from current
	 * disk state and report absence without publishing (a no-op remove must
	 * never clobber a concurrent add). Because every mutation starts from a
	 * fresh read inside the exclusion zone, a removal can never resurrect an
	 * entry a sibling just restored, and an addition can never drop one a
	 * sibling just archived. Failed publishes preserve their input (the
	 * on-disk authority and the legacy source are untouched) and leave a
	 * truthful cache: the last-known-good disk state, never the unpublished
	 * mutation.
	 */
	async #mutateArchived(kind: "add" | "remove", sessionId: string): Promise<boolean> {
		await this.#ensureMigrationConsumed();
		const archivePath = this.#archivePath();
		return archiveSerialFor(archivePath).run(() =>
			withFileLock(archivePath, async () => {
				const ids = await this.#readArchiveFromDisk();
				if (kind === "add") {
					if (ids.has(sessionId)) {
						this.#archived = ids;
						return true;
					}
					ids.add(sessionId);
				} else {
					if (!ids.delete(sessionId)) {
						this.#archived = ids;
						return false;
					}
				}
				try {
					await this.#storage.writeTextAtomic(
						archivePath,
						JSON.stringify({ version: ARCHIVE_VERSION, ids: [...ids] }, null, "\t"),
					);
				} catch (err) {
					const authoritative = await this.#readArchiveFromDisk().catch(() => undefined);
					if (authoritative) this.#archived = authoritative;
					throw err;
				}
				this.#archived = ids;
				return true;
			}),
		);
	}

	/** Union-add; idempotent when already present. */
	async #addArchived(sessionId: string): Promise<void> {
		await this.#mutateArchived("add", sessionId);
	}

	/**
	 * Removal wins over concurrent adds of other IDs. Returns false when the
	 * ID was absent everywhere (nothing published, nothing clobbered).
	 */
	async #removeArchived(sessionId: string): Promise<boolean> {
		return this.#mutateArchived("remove", sessionId);
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
				this.#tabs.ensureTabId(file, id, session.sessionManager.getSessionName());
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
			if (file) {
				this.#tabs.ensureTabId(
					file,
					session.sessionManager.getSessionId(),
					session.sessionManager.getSessionName(),
				);
				this.#tabs.visit(file);
			}
			return { switched: true };
		});
	}

	/**
	 * Deadlock-free close-then-select for owners whose navigation queue must
	 * never nest (A1): hides `closedSessionId`'s tab and warm-selects
	 * `nextSessionId` inside a single Serial turn, without awaiting the public
	 * queue-backed `handleResumeSession`. The logic mirrors {@link select}
	 * inline — it never calls `close`/`select`, so no Serial turn nests.
	 * Never aborts, disposes, archives, or deletes the runtime.
	 */
	async closeAndSelect(
		closedSessionId: string,
		nextSessionId?: string,
	): Promise<{ switched: boolean; lastTab: boolean }> {
		return this.#serial.run(async () => {
			const generation = ++this.#generation;
			const closed = this.#live.snapshots.find(snapshot => snapshot.id === closedSessionId);
			if (closed?.path) this.#tabs.close(closed.path, true);
			const lastTab = (): boolean => this.#tabs.paths.length === 0;
			if (nextSessionId === undefined || nextSessionId === closedSessionId || lastTab()) {
				return { switched: false, lastTab: lastTab() };
			}
			const target = this.#live.snapshots.find(snapshot => snapshot.id === nextSessionId);
			if (!target) return { switched: false, lastTab: lastTab() };
			try {
				this.#live.selectById(nextSessionId);
			} catch {
				if (!target.path) return { switched: false, lastTab: lastTab() };
				const selection = await this.#live.select(target.path);
				if (!selection.selected || generation !== this.#generation) return { switched: false, lastTab: lastTab() };
			}
			if (generation !== this.#generation) return { switched: false, lastTab: lastTab() };
			const session = this.#live.sessions.find(s => s.sessionManager.getSessionId() === nextSessionId);
			const file = session?.sessionManager.getSessionFile() || target.path || undefined;
			if (file) {
				this.#tabs.ensureTabId(file, nextSessionId, session?.sessionManager.getSessionName());
				this.#tabs.visit(file);
			}
			return { switched: true, lastTab: false };
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
	 * Authoritative reopen gate: warm, cold, and unsaved reopens pass the
	 * SAME archive/ownership check — a warm registry hit is an optimization
	 * only and never authorizes the reopen. Owners must call this from every
	 * reopen entry (tab open/reopen/next/prev/switch, strip clicks, slash
	 * commands) BEFORE resolving the runtime: archived sessions reopen only
	 * after an explicit Restore.
	 */
	async requireRestoredForReopen(sessionId: string): Promise<void> {
		const archived = await this.#authoritativeArchived();
		if (archived.has(sessionId)) throw new Error(`Session ${sessionId} is archived; restore it first`);
	}

	/**
	 * Reopen the exact requested session: warm runtime when live, otherwise a
	 * cold open of that session's persisted file adopted into the registry.
	 * Archived sessions must be restored first.
	 */
	async reopen(sessionId: string): Promise<{ switched: boolean }> {
		await this.requireRestoredForReopen(sessionId);
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
		this.#tabs.ensureTabId(match.path, sessionId, session.sessionManager.getSessionName());
		return { switched: true };
	}

	/** Reopen the most recently closed tab (bounded closed-tab history). */
	async reopenLast(): Promise<{ switched: boolean; sessionId?: string }> {
		const closed = this.#tabs.reopen();
		if (!closed) return { switched: false };
		const sessionId =
			this.#tabs.idForPath(closed) ??
			this.#live.snapshotForPath(closed)?.id ??
			(await this.#resolveColdSessionIdByPath(closed));
		if (sessionId) {
			// Routed through reopen(), so the SAME authoritative gate applies:
			// an archived tab stays open (usable) and the error names Restore.
			// The tab was already re-added above, so a refused reopen preserves
			// it instead of dropping it.
			return { ...(await this.reopen(sessionId)), sessionId };
		}
		const selection = await this.#live.select(closed);
		const liveId = selection.session.sessionManager.getSessionId();
		this.#tabs.ensureTabId(closed, liveId, selection.session.sessionManager.getSessionName());
		return { switched: selection.selected };
	}

	/** Persisted-session UUID for a tab path, if this project's store knows it. */
	async #resolveColdSessionIdByPath(file: string): Promise<string | undefined> {
		let cold: { id: string; path: string }[] = [];
		try {
			cold = await SessionManager.list(this.#directories.cwd, this.#directories.sessionDir, this.#storage);
		} catch (err) {
			logger.warn("Failed to list persisted sessions for reopen", { error: err });
		}
		return cold.find(session => session.path === file)?.id;
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
		await this.#addArchived(sessionId);
		const target = snapshots.find(snapshot => snapshot.id === sessionId);
		if (target?.path) this.#tabs.close(target.path, false);
	}

	async restore(sessionId: string): Promise<void> {
		// Removal wins over concurrent adds of other IDs; no-op when absent.
		await this.#removeArchived(sessionId);
	}

	/**
	 * Target-aware delete as an owned lifecycle/storage transaction. Live
	 * runtimes resolve through the registry; cold UUIDs resolve through
	 * persisted-session metadata and are validated against this project's
	 * session store before anything is removed. Ordinary delete rejects busy
	 * targets (streaming, shell/Eval, pending async work, approvals,
	 * compaction). With stopFirst, stops through existing cancellation, waits
	 * for settlement up to a timeout (rejection on timeout — never proceeds
	 * unsettled), fences writers, deletes storage/artifacts, and only then —
	 * at success — tombstones the ID, clears view state, closes the tab, and
	 * disposes/detaches the runtime. Identity, draft, anchor, transcript,
	 * and config are preserved until that success point; unknown IDs are
	 * rejected with nothing tombstoned; an already-gone session file
	 * (never materialized, or removed concurrently) still completes the
	 * success path — delete is idempotent on ENOENT. Other deletion
	 * failures retain discoverability AND recover a usable durable runtime
	 * under the same UUID (the sealed pre-delete generation stays fenced
	 * and can no longer write), so the failure is retryable.
	 */
	async delete(sessionId: string, opts?: { stopFirst?: boolean }): Promise<void> {
		await this.#serial.run(async () => {
			// Resolve + busy-check inside the Serial: a run starting between
			// the caller's check and our execution must not be deleted
			// mid-flight, and stop/settle below may change liveness.
			const snapshot = this.#live.snapshots.find(entry => entry.id === sessionId);
			const session = this.#live.sessions.find(s => s.sessionManager.getSessionId() === sessionId);
			if (this.#isBusySession(session, snapshot) && !opts?.stopFirst) {
				throw new Error(`Session ${sessionId} is busy; stop it first or delete with stop-and-delete`);
			}
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
			// Re-resolve after stop/settle: the runtime may have transitioned.
			const target = this.#live.snapshots.find(entry => entry.id === sessionId);
			const liveSession = this.#live.sessions.find(s => s.sessionManager.getSessionId() === sessionId) ?? session;
			let file = target?.path || liveSession?.sessionManager.getSessionFile() || undefined;
			if (!file && !liveSession) {
				file = await this.#resolveColdSessionFile(sessionId);
				if (!file) {
					throw new Error(`Session ${sessionId} not found; nothing deleted.`);
				}
			}
			if (file) {
				this.#assertProjectSessionFile(file, sessionId);
			}
			// Preserve identity/transcript/config until success: snapshot the
			// raw persisted bytes so an artifact-boundary partial (session
			// file gone, artifacts failed) can be reconstructed under the
			// same UUID. View state (drafts, anchors) is likewise cleared
			// only on the success path below. A never-materialized file
			// (lazy persistence never wrote it: readText goes ENOENT) has
			// nothing durable to preserve; an existing-but-unreadable file
			// refuses deletion before anything is fenced, so the runtime
			// stays usable.
			let preservedBytes: string | undefined;
			if (file) {
				try {
					preservedBytes = await this.#storage.readText(file);
				} catch (error) {
					if (!isEnoent(error)) {
						throw new Error(
							`Session ${sessionId} could not be preserved for deletion (${error instanceof Error ? error.message : String(error)}); deletion refused, session remains usable.`,
						);
					}
				}
			}
			const wasSelected = target?.selected ?? false;
			// Flush buffered transcript bytes to durable storage BEFORE
			// sealing: recovery reopens the file, so everything worth
			// keeping must be readable from it. Best-effort — the seal
			// still fences writers on failure.
			try {
				await liveSession?.sessionManager.flush();
			} catch (error) {
				logger.warn("Failed to flush session before deletion; recovery may lag memory", {
					sessionId,
					error,
				});
			}
			// Seal the writer BEFORE unlinking so concurrent appends during
			// deletion cannot recreate the file after it is gone. The seal is
			// terminal: on storage failure the sealed generation is retired
			// (never unsealed) and recovery mints a fresh runtime.
			liveSession?.sessionManager.seal();
			try {
				if (file) {
					await this.#storage.deleteSessionWithArtifacts(file);
				}
			} catch (error) {
				// Idempotent delete: the session file was already gone (never
				// materialized, or removed concurrently) — the durable goal is
				// satisfied, so the success path below still runs. Any other
				// failure recovers a usable same-UUID runtime and stays
				// retryable.
				if (!file || !isEnoent(error)) {
					const recovery = await this.#recoverAfterFailedDelete({
						sessionId,
						file,
						preservedBytes,
						liveSession,
						wasSelected,
					});
					throw new Error(
						`Failed to delete session ${sessionId}: ${error instanceof Error ? error.message : String(error)}. ${recovery}`,
					);
				}
			}
			// Success-only cleanup, in order: archive removal (may throw while
			// state is still fully intact) → tombstone (late-write fence for
			// any background callback still holding the ID) → view-state
			// release → tab close → dispose/detach → drain.
			await this.#removeArchived(sessionId);
			this.#tombstones.add(sessionId);
			this.#viewState.clearDraft(sessionId);
			this.#viewState.saveReadingAnchor(sessionId, undefined);
			// Close the tab only after storage removal succeeds: a failed
			// delete keeps the view exactly where it was.
			if (file) {
				this.#tabs.close(file, false);
			}
			if (liveSession) {
				try {
					await liveSession.dispose();
				} catch (error) {
					logger.warn("Failed to dispose deleted session runtime", { sessionId, error });
				}
				this.#live.detach(sessionId);
			}
			await this.#storage.drain();
		});
	}

	/**
	 * Recover a USABLE durable runtime under the same UUID after a storage
	 * deletion failure. The sealed pre-delete generation is terminally fenced
	 * (never unsealed): it is disposed and detached, the persisted file is
	 * reconstructed from the preserved bytes when the unlink took it, and a
	 * fresh runtime is opened through the existing cold-open owner and
	 * adopted back into the registry with its tab binding. View state was
	 * never cleared, so drafts and anchors survive. Returns a truthful
	 * recovery note for the delete error; the failure stays retryable.
	 */
	async #recoverAfterFailedDelete(args: {
		sessionId: string;
		file: string | undefined;
		preservedBytes: string | undefined;
		liveSession: AgentSession | undefined;
		wasSelected: boolean;
	}): Promise<string> {
		const { sessionId, file, preservedBytes, liveSession, wasSelected } = args;
		if (!liveSession) {
			await this.#reconstructSessionFile(file, preservedBytes);
			return "Session remains discoverable; deletion can be retried.";
		}
		// Retire the sealed generation first: this releases its write lease
		// so the recovery open can claim the file, and nothing still
		// references the unwritable manager afterwards.
		try {
			await liveSession.dispose();
		} catch (error) {
			logger.warn("Failed to dispose sealed session generation during delete recovery", { sessionId, error });
		}
		this.#live.detach(sessionId);
		const reconstructed = await this.#reconstructSessionFile(file, preservedBytes);
		if (!file) {
			return "The live runtime was released; the session had no persisted file so there is nothing to recover.";
		}
		if (!reconstructed) {
			return "The persisted session file could not be reconstructed; the live runtime was released and deletion can be retried after manual recovery.";
		}
		let reopened: AgentSession;
		try {
			reopened = await this.#openSession(file);
		} catch (error) {
			logger.warn("Failed to reopen session during delete recovery", { sessionId, error });
			return "The persisted session file was preserved but the live runtime could not be reopened; deletion can be retried.";
		}
		if (reopened.sessionManager.getSessionId() !== sessionId) {
			await reopened.dispose().catch(() => {});
			return "The persisted session file resolved to a different session; the mismatched runtime was released and deletion can be retried.";
		}
		if (wasSelected) this.#live.adopt(reopened);
		else this.#live.adoptBackground(reopened);
		this.#tabs.ensureTabId(file, sessionId, reopened.sessionManager.getSessionName());
		return "Recovered a usable runtime under the same session identity; append/rename/flush work and deletion can be retried. The sealed pre-delete generation stays fenced and can no longer write.";
	}

	/**
	 * Restore the persisted session file from preserved bytes when the
	 * unlink took it (artifact-boundary partial: file gone, artifacts
	 * failed). Returns false when there is nothing to restore or the
	 * restore failed.
	 */
	async #reconstructSessionFile(file: string | undefined, preservedBytes: string | undefined): Promise<boolean> {
		if (!file || preservedBytes === undefined) return false;
		let exists = false;
		try {
			exists = await this.#storage.exists(file);
		} catch {
			exists = false;
		}
		if (exists) return true;
		try {
			await this.#storage.writeTextAtomic(file, preservedBytes);
			return true;
		} catch (error) {
			logger.warn("Failed to reconstruct session file during delete recovery", { file, error });
			return false;
		}
	}

	/**
	 * Resolve a cold (non-live) session UUID to its persisted file through
	 * session metadata. Returns undefined for unknown IDs — the caller rejects
	 * them with nothing tombstoned so a typo can never seal a live writer or
	 * hide a session from discovery.
	 */
	async #resolveColdSessionFile(sessionId: string): Promise<string | undefined> {
		let cold: { id: string; path: string }[] = [];
		try {
			cold = await SessionManager.list(this.#directories.cwd, this.#directories.sessionDir, this.#storage);
		} catch (err) {
			logger.warn("Failed to list persisted sessions for delete", { error: err });
		}
		return cold.find(session => session.id === sessionId)?.path;
	}

	/** Refuse files outside this project's session store, even when metadata names them. */
	#assertProjectSessionFile(file: string, sessionId: string): void {
		const dir = path.resolve(this.#directories.sessionDir);
		const resolved = path.resolve(file);
		if (resolved !== dir && !resolved.startsWith(dir + path.sep)) {
			throw new Error(`Session ${sessionId} resolved outside the project session store; deletion refused.`);
		}
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
