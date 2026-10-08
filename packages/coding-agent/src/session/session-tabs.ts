import { normalizePathForComparison } from "@harvest/pi-utils";

/** One open tab: a persistence path plus the stable session ID the owner routes by. */
export interface SessionTabEntry {
	readonly path: string;
	readonly label?: string;
	readonly sessionId?: string;
}

/** Closed-tab entry retained for reopen, carrying its session ID when known. */
export interface ClosedSessionTabEntry {
	readonly path: string;
	readonly label?: string;
	readonly sessionId?: string;
}

/**
 * Warm-runtime-first navigation target shared by keyboard (next/prev/reopen)
 * and mouse (strip click) paths. Both resolve through this helper before any
 * disk-file check: a stable session ID match wins, then a live path match,
 * and only then does the caller fall back to a cold open from disk.
 */
export interface WarmSessionRef {
	readonly id: string;
	readonly path: string;
}

export type NavigationTargetKind = "warm-id" | "warm-path" | "cold";

export interface NavigationTarget {
	readonly kind: NavigationTargetKind;
	/** Stable session ID when the target resolved warm; undefined for cold opens. */
	readonly sessionId: string | undefined;
	readonly path: string;
}

/**
 * Resolve one navigation target warm-runtime-first. `warm` is a registry
 * snapshot (stable IDs); `fileExists` is consulted only when no warm entry
 * matches, so a live runtime is never re-read from disk. Returns undefined
 * when the target is neither warm nor on disk (caller drops the tab).
 */
export function resolveNavigationTarget(
	target: { id?: string; path: string },
	warm: readonly WarmSessionRef[],
	fileExists: (path: string) => boolean,
): NavigationTarget | undefined {
	if (target.id) {
		const byId = warm.find(snapshot => snapshot.id === target.id);
		if (byId) return { kind: "warm-id", sessionId: byId.id, path: byId.path };
	}
	const byPath = warm.find(
		snapshot => normalizePathForComparison(snapshot.path) === normalizePathForComparison(target.path),
	);
	if (byPath) return { kind: "warm-path", sessionId: byPath.id, path: byPath.path };
	if (fileExists(target.path)) return { kind: "cold", sessionId: target.id, path: target.path };
	return undefined;
}

/** Open session files in one terminal. The active runtime remains the source of truth. */
export class SessionTabs {
	#paths: string[] = [];
	#labels = new Map<string, string>();
	#ids = new Map<string, string>();
	#closed: ClosedSessionTabEntry[] = [];
	#history: string[] = [];
	#historyIndex = -1;
	readonly #capacity: number;

	constructor(capacity = Number.POSITIVE_INFINITY) {
		this.#capacity = capacity;
	}

	get paths(): readonly string[] {
		return this.#paths;
	}

	/** Open tabs as {id,path} entries for ID-routed navigation. Tabs remain usable while callers migrate to selectById. */
	get entries(): readonly SessionTabEntry[] {
		return this.#paths.map(tabPath => {
			const entry: { path: string; label?: string; sessionId?: string } = { path: tabPath };
			const label = this.label(tabPath);
			if (label) entry.label = label;
			const sessionId = this.idForPath(tabPath);
			if (sessionId) entry.sessionId = sessionId;
			return entry;
		});
	}

	indexOf(path: string): number {
		return this.#paths.findIndex(item => this.#same(item, path));
	}

	/** Stable session ID recorded for a tab path, if the owner has noted one. */
	idForPath(path: string): string | undefined {
		return this.#ids.get(normalizePathForComparison(path));
	}

	/** Tab path carrying a stable session ID, if one is open. */
	pathForId(sessionId: string): string | undefined {
		for (const tabPath of this.#paths) {
			if (this.#ids.get(normalizePathForComparison(tabPath)) === sessionId) return tabPath;
		}
		return undefined;
	}

	/** Record the stable session ID for an open tab without disturbing its order. */
	noteId(path: string, sessionId: string): void {
		if (this.indexOf(path) < 0) return;
		this.#ids.set(normalizePathForComparison(path), sessionId);
	}

	/**
	 * Ensure a tab is open for `path` and carries `sessionId`, without
	 * disturbing its order. Every session transition (create, warm select,
	 * cold reopen, tab open, model-created open) routes through this helper
	 * so the path→UUID binding can never go missing: a tab without a noted
	 * ID falls back to path matching, which collides after forks and moves.
	 */
	ensureTabId(path: string, sessionId: string, label?: string): void {
		if (label) this.#labels.set(normalizePathForComparison(path), label);
		if (this.indexOf(path) < 0) {
			if (this.#paths.length >= this.#capacity) {
				throw new Error(`At most ${this.#capacity} session tabs can be open`);
			}
			this.#paths.push(path);
		}
		this.#ids.set(normalizePathForComparison(path), sessionId);
	}

	#same(a: string, b: string): boolean {
		return normalizePathForComparison(a) === normalizePathForComparison(b);
	}

	label(path: string): string | undefined {
		return this.#labels.get(normalizePathForComparison(path));
	}

	open(path: string, label?: string, sessionId?: string): void {
		if (label) this.#labels.set(normalizePathForComparison(path), label);
		if (sessionId) this.#ids.set(normalizePathForComparison(path), sessionId);
		if (this.indexOf(path) >= 0) return;
		if (this.#paths.length >= this.#capacity) {
			throw new Error(`At most ${this.#capacity} session tabs can be open`);
		}
		this.#paths.push(path);
	}

	visit(path: string): void {
		this.open(path);
		if (this.#historyIndex >= 0 && this.#same(this.#history[this.#historyIndex] ?? "", path)) return;
		this.#history = this.#history.slice(0, this.#historyIndex + 1);
		this.#history.push(path);
		this.#historyIndex = this.#history.length - 1;
	}

	historyTarget(direction: 1 | -1): string | undefined {
		const index = this.#historyIndex + direction;
		return index >= 0 && index < this.#history.length ? this.#history[index] : undefined;
	}

	commitHistoryMove(direction: 1 | -1): void {
		if (this.historyTarget(direction)) this.#historyIndex += direction;
	}

	close(path: string, remember = true): boolean {
		const index = this.indexOf(path);
		if (index < 0) return false;
		const [closed] = this.#paths.splice(index, 1);
		if (closed) {
			const key = normalizePathForComparison(closed);
			if (remember) this.#closed.push({ path: closed, label: this.#labels.get(key), sessionId: this.#ids.get(key) });
			this.#labels.delete(key);
			this.#ids.delete(key);
			const before = this.#history.slice(0, this.#historyIndex + 1).filter(item => this.#same(item, closed)).length;
			this.#history = this.#history.filter(item => !this.#same(item, closed));
			this.#historyIndex = Math.min(this.#history.length - 1, this.#historyIndex - before);
		}
		return true;
	}

	reopen(): string | undefined {
		const closed = this.#closed.pop();
		if (!closed) return undefined;
		this.open(closed.path, closed.label, closed.sessionId);
		return closed.path;
	}

	/** Closed tabs available to reopen, most recent last. */
	get recentlyClosed(): readonly ClosedSessionTabEntry[] {
		return [...this.#closed];
	}

	/** Restore a previously persisted closed entry without disturbing the open order. */
	rememberClosed(path: string, label?: string, sessionId?: string): void {
		this.#closed.push({ path, label, sessionId });
	}

	/**
	 * Close the final tab into the Home empty state. Unlike `close`, this never
	 * refuses the last tab: every open tab is hidden (remembered for reopen)
	 * while its runtime keeps running, and the owner shows Home instead of an
	 * active session. Returns the hidden paths, most recent last.
	 *
	 * Owner wiring (selector-controller `handleSessionTabsCommand` close path):
	 * when no neighbor exists, call this helper, persist tabs, and render Home
	 * instead of returning "last tab cannot be closed".
	 */
	closeLastTabToHome(): { closed: string[]; home: true } {
		const closed: string[] = [];
		// Snapshot: close() mutates #paths during iteration.
		for (const tabPath of this.#paths.slice()) {
			if (this.close(tabPath, true)) closed.push(tabPath);
		}
		return { closed, home: true as const };
	}

	neighbor(current: string, direction: 1 | -1): string | undefined {
		if (this.#paths.length < 2) return undefined;
		const index = this.indexOf(current);
		if (index < 0) return undefined;
		return this.#paths[(index + direction + this.#paths.length) % this.#paths.length];
	}

	/**
	 * Close-then-select target for `current`: the right neighbor when one is
	 * open, else the left neighbor. Unlike {@link neighbor} (which wraps for
	 * keyboard next/prev cycling), this never wraps: closing the rightmost
	 * tab selects the tab to its left, never the first tab. Returns undefined
	 * when `current` is unknown or it is the only open tab (the owner enters
	 * detached Home instead of selecting anything).
	 *
	 * Owner wiring (selector-controller `handleSessionTabsCommand` close path,
	 * interactive-mode strip × close path): close-then-select must use this
	 * helper; keyboard next/prev keeps using {@link neighbor}.
	 */
	closeNeighbor(current: string): string | undefined {
		if (this.#paths.length < 2) return undefined;
		const index = this.indexOf(current);
		if (index < 0) return undefined;
		return this.#paths[index + 1] ?? this.#paths[index - 1];
	}
}

/**
 * Screen-space origin of the rendered tab strip, in terminal cells. The
 * composer layout owns this (it stacks header/transcript/strip rows); the
 * strip itself only ever sees strip-local rows.
 */
export interface TabStripScreenOrigin {
	readonly row: number;
	readonly col: number;
}

/**
 * Translate one screen-space mouse hit into strip-local coordinates. The
 * strip hit map (`closeTargetAt`, `tabAt`, hover/click/new-target zones) is
 * built from just-rendered rows, so every hit-test input must already be
 * local: owners translate with this helper BEFORE hit-testing and drop hits
 * that fall outside the strip (`undefined`). Hits above/left of the origin
 * are outside by construction — the strip never assumes screen row zero.
 *
 * Pure: no I/O, no component state. Owner wiring (interactive-mode mouse
 * path): resolve the strip origin from the composer layout for the current
 * frame, translate `event.row/event.col` through this helper, and only then
 * call the strip's hover/click/close-target entry points.
 */
export function translateStripHitToLocal(
	screenRow: number,
	screenCol: number,
	origin: TabStripScreenOrigin,
): { row: number; col: number } | undefined {
	if (!Number.isInteger(screenRow) || !Number.isInteger(screenCol)) return undefined;
	if (!Number.isInteger(origin.row) || !Number.isInteger(origin.col)) return undefined;
	const row = screenRow - origin.row;
	const col = screenCol - origin.col;
	if (row < 0 || col < 0) return undefined;
	return { row, col };
}
