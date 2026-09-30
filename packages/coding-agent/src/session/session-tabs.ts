import { normalizePathForComparison } from "@harvest/pi-utils";

/** Open session files in one terminal. The active runtime remains the source of truth. */
export class SessionTabs {
	#paths: string[] = [];
	#labels = new Map<string, string>();
	#closed: { path: string; label?: string }[] = [];
	#history: string[] = [];
	#historyIndex = -1;
	readonly #capacity: number;

	constructor(capacity = Number.POSITIVE_INFINITY) {
		this.#capacity = capacity;
	}

	get paths(): readonly string[] {
		return this.#paths;
	}

	indexOf(path: string): number {
		return this.#paths.findIndex(item => this.#same(item, path));
	}

	#same(a: string, b: string): boolean {
		return normalizePathForComparison(a) === normalizePathForComparison(b);
	}

	label(path: string): string | undefined {
		return this.#labels.get(normalizePathForComparison(path));
	}

	open(path: string, label?: string): void {
		if (label) this.#labels.set(normalizePathForComparison(path), label);
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
			if (remember) this.#closed.push({ path: closed, label: this.label(closed) });
			this.#labels.delete(normalizePathForComparison(closed));
			const before = this.#history.slice(0, this.#historyIndex + 1).filter(item => this.#same(item, closed)).length;
			this.#history = this.#history.filter(item => !this.#same(item, closed));
			this.#historyIndex = Math.min(this.#history.length - 1, this.#historyIndex - before);
		}
		return true;
	}

	reopen(): string | undefined {
		const closed = this.#closed.pop();
		if (!closed) return undefined;
		this.open(closed.path, closed.label);
		return closed.path;
	}

	/** Closed tabs available to reopen, most recent last. */
	get recentlyClosed(): readonly { path: string; label?: string }[] {
		return [...this.#closed];
	}

	/** Restore a previously persisted closed entry without disturbing the open order. */
	rememberClosed(path: string, label?: string): void {
		this.#closed.push({ path, label });
	}

	neighbor(current: string, direction: 1 | -1): string | undefined {
		if (this.#paths.length < 2) return undefined;
		const index = this.indexOf(current);
		if (index < 0) return undefined;
		return this.#paths[(index + direction + this.#paths.length) % this.#paths.length];
	}
}
