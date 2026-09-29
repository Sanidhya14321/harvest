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

interface LiveSessionEntry {
	session: AgentSession;
	path: string;
	status: LiveSessionStatus;
	unread: boolean;
	unsubscribe: () => void;
	unsubscribeTitle: () => void;
}

/** Owns independent agent runtimes for one project; tab visibility never controls execution. */
export class LiveSessionRegistry {
	readonly #entries = new Map<string, LiveSessionEntry>();
	readonly #opening = new Map<string, Promise<AgentSession>>();
	readonly #listeners = new Set<() => void>();
	readonly #project: string;
	#selectedId: string;
	#selectionGeneration = 0;
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

	get snapshots(): readonly LiveSessionSnapshot[] {
		return [...this.#entries.entries()].map(([id, entry]) => ({
			id,
			path: entry.session.sessionManager.getSessionFile() ?? entry.path,
			title: entry.session.sessionManager.getSessionName(),
			status: entry.status,
			unread: entry.unread,
			selected: id === this.#selectedId,
		}));
	}

	onChange(listener: () => void): () => void {
		this.#listeners.add(listener);
		return () => this.#listeners.delete(listener);
	}

	/** Resolve a warm runtime without reading its session file; cold opens are deduplicated. */
	async select(path: string): Promise<AgentSession> {
		if (this.#disposed) throw new Error("Live session registry is closed");
		const generation = ++this.#selectionGeneration;
		const key = normalizePathForComparison(path);
		const warm = [...this.#entries.values()].find(
			entry => normalizePathForComparison(entry.session.sessionManager.getSessionFile() ?? entry.path) === key,
		);
		const session = warm?.session ?? (await this.#openOnce(path, key));
		if (this.#disposed) throw new Error("Live session registry is closed");
		if (generation !== this.#selectionGeneration) return session;
		const id = session.sessionManager.getSessionId();
		this.#selectedId = id;
		const entry = this.#entries.get(id)!;
		entry.unread = false;
		if (entry.status === "completed") entry.status = "idle";
		this.#emit();
		return session;
	}

	/** Attach a newly created session without replacing or aborting another runtime. */
	adopt(session: AgentSession): void {
		if (this.#disposed) throw new Error("Live session registry is closed");
		this.#selectionGeneration++;
		this.#add(session);
		this.#selectedId = session.sessionManager.getSessionId();
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

	async dispose(): Promise<void> {
		if (this.#disposed) return;
		this.#disposed = true;
		for (const entry of this.#entries.values()) {
			entry.unsubscribe();
			entry.unsubscribeTitle();
		}
		await Promise.all([...this.#entries.values()].map(entry => entry.session.dispose()));
		this.#entries.clear();
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
		const path = manager.getSessionFile();
		if (!path) throw new Error("A live session must have a session path");
		if (normalizePathForComparison(manager.getCwd()) !== this.#project) {
			throw new Error("Live tabs are limited to the current project");
		}
		const existing = this.#entries.get(id);
		if (existing) {
			if (existing.session !== session) throw new Error(`Session ${id} is already live`);
			return;
		}
		const entry: LiveSessionEntry = {
			session,
			path,
			status: session.isStreaming ? "running" : "idle",
			unread: false,
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
		this.#emit();
	}

	#emit(): void {
		for (const listener of this.#listeners) listener();
	}
}
