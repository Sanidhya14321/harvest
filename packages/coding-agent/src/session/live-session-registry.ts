import { normalizePathForComparison } from "@harvest/pi-utils";
import type { AgentSession } from "./agent-session";

export type LiveSessionStatus = "idle" | "running" | "waiting" | "completed" | "error";

export interface LiveSessionSnapshot {
	id: string;
	path: string;
	title: string | undefined;
	status: LiveSessionStatus;
	unread: boolean;
	selected: boolean;
}

export interface LiveSessionSelection {
	session: AgentSession;
	/** False when a newer navigation request superseded this one while it loaded. */
	selected: boolean;
}

interface LiveSessionEntry {
	session: AgentSession;
	path: string | undefined;
	lastUsed: number;
	status: LiveSessionStatus;
	unread: boolean;
	unsubscribe: () => void;
	unsubscribeTitle: () => void;
}

/** Owns independent agent runtimes for one project; tab visibility never controls execution. */
export class LiveSessionRegistry {
	readonly #entries = new Map<string, LiveSessionEntry>();
	readonly #idsByPath = new Map<string, string>();
	readonly #dormant = new Map<string, LiveSessionSnapshot>();
	readonly #opening = new Map<string, Promise<AgentSession>>();
	readonly #listeners = new Set<() => void>();
	readonly #project: string;
	#selectedId: string;
	#selectionGeneration = 0;
	#usageOrder = 0;
	#disposed = false;

	constructor(
		initial: AgentSession,
		private readonly openSession: (path: string) => Promise<AgentSession>,
	) {
		this.#project = normalizePathForComparison(initial.sessionManager.getCwd());
		this.#selectedId = initial.sessionManager.getSessionId();
		this.#add(initial);
	}

	get selected(): AgentSession {
		return this.#entries.get(this.#selectedId)!.session;
	}

	get sessions(): readonly AgentSession[] {
		return [...this.#entries.values()].map(entry => entry.session);
	}

	/** Runs and approval requests that need an explicit decision before process exit. */
	get busySessions(): readonly LiveSessionSnapshot[] {
		return this.snapshots.filter(snapshot => snapshot.status === "running" || snapshot.status === "waiting");
	}

	get snapshots(): readonly LiveSessionSnapshot[] {
		const live = [...this.#entries.entries()].map(([id, entry]) => ({
			id,
			path: entry.session.sessionManager.getSessionFile() ?? entry.path ?? "",
			title: entry.session.sessionManager.getSessionName(),
			status: entry.status,
			unread: entry.unread,
			selected: id === this.#selectedId,
		}));
		return [...live, ...this.#dormant.values()];
	}

	onChange(listener: () => void): () => void {
		this.#listeners.add(listener);
		return () => this.#listeners.delete(listener);
	}

	/** Resolve a warm runtime without reading its session file; cold opens are deduplicated. */
	async select(path: string): Promise<LiveSessionSelection> {
		if (this.#disposed) throw new Error("Live session registry is closed");
		const generation = ++this.#selectionGeneration;
		const key = normalizePathForComparison(path);
		const warmId = this.#idsByPath.get(key);
		const warm = warmId ? this.#entries.get(warmId) : undefined;
		const session = warm?.session ?? (await this.#openOnce(path, key));
		if (this.#disposed) throw new Error("Live session registry is closed");
		if (generation !== this.#selectionGeneration) return { session, selected: false };
		const id = session.sessionManager.getSessionId();
		this.#selectedId = id;
		const entry = this.#entries.get(id)!;
		entry.lastUsed = ++this.#usageOrder;
		entry.unread = false;
		if (entry.status === "completed") entry.status = "idle";
		this.#emit();
		return { session, selected: true };
	}

	/** Resolve a warm runtime by stable session ID without reading its session file. */
	selectById(sessionId: string): LiveSessionSelection {
		if (this.#disposed) throw new Error("Live session registry is closed");
		const entry = this.#entries.get(sessionId);
		if (!entry) throw new Error(`Session ${sessionId} is not live`);
		this.#selectionGeneration++;
		this.#selectedId = sessionId;
		entry.lastUsed = ++this.#usageOrder;
		entry.unread = false;
		if (entry.status === "completed") entry.status = "idle";
		this.#emit();
		return { session: entry.session, selected: true };
	}

	/** Attach a newly created session without replacing or aborting another runtime. */
	adopt(session: AgentSession): void {
		if (this.#disposed) throw new Error("Live session registry is closed");
		this.#selectionGeneration++;
		this.#add(session);
		this.#selectedId = session.sessionManager.getSessionId();
		this.#entries.get(this.#selectedId)!.lastUsed = ++this.#usageOrder;
		this.#emit();
	}

	/** Record the persistence path for a previously path-less live session. */
	notePath(sessionId: string, path: string): void {
		if (this.#disposed) throw new Error("Live session registry is closed");
		const entry = this.#entries.get(sessionId);
		if (!entry) throw new Error(`Session ${sessionId} is not live`);
		const key = normalizePathForComparison(path);
		const existingId = this.#idsByPath.get(key);
		if (existingId && existingId !== sessionId) throw new Error(`Session path ${path} is already live`);
		if (entry.path) this.#idsByPath.delete(normalizePathForComparison(entry.path));
		entry.path = path;
		this.#idsByPath.set(key, sessionId);
		this.#dormant.delete(key);
		this.#emit();
	}

	/** Release least-recently-used idle runtimes after a view has detached from them. */
	async releaseIdleRuntimes(limit = 6): Promise<void> {
		if (this.#disposed) return;
		const idle = [...this.#entries.entries()]
			.filter(
				([, entry]) =>
					entry.path !== undefined &&
					entry.status !== "running" &&
					entry.status !== "waiting" &&
					!entry.session.isStreaming &&
					!entry.session.isBashRunning &&
					!entry.session.isEvalRunning &&
					!entry.session.hasPendingAsyncWork(),
			)
			.sort((left, right) => left[1].lastUsed - right[1].lastUsed);
		let toRelease = Math.max(0, idle.length - Math.max(1, limit));
		for (const [id, entry] of idle) {
			if (toRelease === 0) break;
			if (id === this.#selectedId) continue;
			if (!entry.path) continue;
			const snapshot = {
				id,
				path: entry.path,
				title: entry.session.sessionManager.getSessionName(),
				status: entry.status,
				unread: entry.unread,
				selected: false,
			} satisfies LiveSessionSnapshot;
			await entry.session.dispose();
			entry.unsubscribe();
			entry.unsubscribeTitle();
			this.#entries.delete(id);
			this.#idsByPath.delete(normalizePathForComparison(entry.path));
			this.#dormant.set(normalizePathForComparison(entry.path), snapshot);
			toRelease--;
		}
		this.#emit();
	}

	markWaiting(sessionId: string, waiting: boolean): void {
		const entry = this.#entries.get(sessionId);
		if (!entry) return;
		entry.status = waiting ? "waiting" : entry.session.isStreaming ? "running" : "idle";
		if (waiting && sessionId !== this.#selectedId) entry.unread = true;
		this.#emit();
	}

	markError(sessionId: string): void {
		const entry = this.#entries.get(sessionId);
		if (!entry) return;
		entry.status = "error";
		if (sessionId !== this.#selectedId) entry.unread = true;
		this.#emit();
	}

	/** Stop only the requested run; hiding or selecting tabs never calls this. */
	async stop(sessionId: string): Promise<void> {
		const entry = this.#entries.get(sessionId);
		if (!entry) throw new Error(`Session ${sessionId} is not live`);
		await entry.session.abort();
	}

	async dispose(): Promise<void> {
		if (this.#disposed) return;
		this.#disposed = true;
		await Promise.allSettled(this.#opening.values());
		for (const entry of this.#entries.values()) {
			entry.unsubscribe();
			entry.unsubscribeTitle();
		}
		await Promise.all([...this.#entries.values()].map(entry => entry.session.dispose()));
		this.#entries.clear();
		this.#idsByPath.clear();
		this.#dormant.clear();
		this.#listeners.clear();
	}

	async #openOnce(path: string, key: string): Promise<AgentSession> {
		let pending = this.#opening.get(key);
		if (!pending) {
			pending = this.openSession(path).then(async session => {
				try {
					if (this.#disposed) throw new Error("Live session registry is closed");
					this.#add(session);
					return session;
				} catch (error) {
					await session.dispose();
					throw error;
				}
			});
			this.#opening.set(key, pending);
			void pending.finally(() => this.#opening.delete(key)).catch(() => {});
		}
		return pending;
	}

	#add(session: AgentSession): void {
		const manager = session.sessionManager;
		const id = manager.getSessionId();
		const path = manager.getSessionFile() ?? undefined;
		if (normalizePathForComparison(manager.getCwd()) !== this.#project) {
			throw new Error("Live tabs are limited to the current project");
		}
		const existing = this.#entries.get(id);
		if (existing) {
			if (existing.session !== session) throw new Error(`Session ${id} is already live`);
			return;
		}
		const pathKey = path ? normalizePathForComparison(path) : undefined;
		if (pathKey) {
			const existingPathId = this.#idsByPath.get(pathKey);
			if (existingPathId && existingPathId !== id) throw new Error(`Session path ${path} is already live`);
		}
		const entry: LiveSessionEntry = {
			session,
			path,
			lastUsed: ++this.#usageOrder,
			status: session.isStreaming ? "running" : pathKey ? (this.#dormant.get(pathKey)?.status ?? "idle") : "idle",
			unread: pathKey ? (this.#dormant.get(pathKey)?.unread ?? false) : false,
			unsubscribe: () => {},
			unsubscribeTitle: () => {},
		};
		entry.unsubscribe = session.subscribe(event => {
			if (event.type === "agent_start") entry.status = "running";
			else if (event.type === "agent_end" && event.isTerminal !== false) {
				entry.status = id === this.#selectedId ? "idle" : "completed";
				entry.unread = id !== this.#selectedId;
			} else if (event.type === "notice" && event.level === "error") {
				entry.status = "error";
				entry.unread = id !== this.#selectedId;
			} else return;
			this.#emit();
		});
		entry.unsubscribeTitle = manager.onSessionNameChanged(() => this.#emit());
		this.#entries.set(id, entry);
		if (pathKey) {
			this.#idsByPath.set(pathKey, id);
			this.#dormant.delete(pathKey);
		}
		this.#emit();
	}

	#emit(): void {
		for (const listener of this.#listeners) listener();
	}
}
