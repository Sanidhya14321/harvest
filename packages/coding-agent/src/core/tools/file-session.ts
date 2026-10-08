/**
 * File Session Turn Checkpoint Stack & Windows-Resilient Atomic Mutator.
 *
 * Maintains turn-level snapshots for atomic rollbacks and provides
 * crash-safe, lock-resilient atomic file writing with backoff.
 */

import * as crypto from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";

export interface TurnSnapshot {
	readonly path: string;
	readonly before: string | null;
	readonly after: string | null;
	readonly turnId: string;
	readonly timestamp: number;
}

export interface RestoreResult {
	readonly path: string;
	readonly restored: boolean;
	readonly action: "restored_content" | "deleted_new_file" | "failed";
	readonly error?: string;
}

/**
 * Windows-resilient atomic write with exponential backoff on EBUSY/EPERM.
 */
export async function atomicWriteFile(targetPath: string, content: string): Promise<void> {
	const dir = path.dirname(targetPath);
	await fs.promises.mkdir(dir, { recursive: true });

	const pid = process.pid;
	let tmpPath = "";
	for (let i = 0; i < 5; i++) {
		const timestamp = Date.now();
		const rand = crypto.randomBytes(4).toString("hex");
		tmpPath = path.join(dir, `.harvest-${pid}-${timestamp}-${rand}.tmp`);
		try {
			// Write content to temporary file with wx / exclusive creation (per Section 4.3)
			await fs.promises.writeFile(tmpPath, content, { encoding: "utf8", flag: "wx" });
			break;
		} catch (err: unknown) {
			if ((err as { code?: string })?.code === "EEXIST" && i < 4) {
				continue;
			}
			throw err;
		}
	}

	let attempts = 0;
	const maxAttempts = 10;
	let delayMs = 10;

	while (attempts < maxAttempts) {
		try {
			await fs.promises.rename(tmpPath, targetPath);
			return;
		} catch (err: unknown) {
			attempts++;
			const code = (err as { code?: string })?.code;
			if ((code === "EBUSY" || code === "EPERM" || code === "EACCES") && attempts < maxAttempts) {
				await new Promise(resolve => setTimeout(resolve, delayMs));
				delayMs = Math.min(delayMs * 2, 500);
				continue;
			}
			try {
				await fs.promises.unlink(tmpPath);
			} catch {}
			throw err;
		}
	}
}

/** Synchronous variant for atomic writes */
export function atomicWriteFileSync(targetPath: string, content: string): void {
	const dir = path.dirname(targetPath);
	fs.mkdirSync(dir, { recursive: true });

	const pid = process.pid;
	let tmpPath = "";
	for (let i = 0; i < 5; i++) {
		const timestamp = Date.now();
		const rand = crypto.randomBytes(4).toString("hex");
		tmpPath = path.join(dir, `.harvest-${pid}-${timestamp}-${rand}.tmp`);
		try {
			fs.writeFileSync(tmpPath, content, { encoding: "utf8", flag: "wx" });
			break;
		} catch (err: unknown) {
			if ((err as { code?: string })?.code === "EEXIST" && i < 4) {
				continue;
			}
			throw err;
		}
	}

	let attempts = 0;
	const maxAttempts = 10;
	let delayMs = 10;

	while (attempts < maxAttempts) {
		try {
			fs.renameSync(tmpPath, targetPath);
			return;
		} catch (err: unknown) {
			attempts++;
			const code = (err as { code?: string })?.code;
			if ((code === "EBUSY" || code === "EPERM" || code === "EACCES") && attempts < maxAttempts) {
				// Busy wait or small sync spin
				const start = Date.now();
				while (Date.now() - start < delayMs) {
					// sync pause
				}
				delayMs = Math.min(delayMs * 2, 500);
				continue;
			}
			try {
				fs.unlinkSync(tmpPath);
			} catch {}
			throw err;
		}
	}
}

export class FileSession {
	readonly #snapshots: TurnSnapshot[] = [];
	readonly #workspaceRoot: string;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#workspaceRoot = path.resolve(workspaceRoot);
	}

	#canonicalize(filePath: string): string {
		const absolute = path.isAbsolute(filePath) ? filePath : path.resolve(this.#workspaceRoot, filePath);
		try {
			return fs.realpathSync.native ? fs.realpathSync.native(absolute) : fs.realpathSync(absolute);
		} catch {
			return path.normalize(absolute);
		}
	}

	/** Record file mutation in the active turn's rollback stack */
	recordMutation(turnId: string, filePath: string, beforeContent: string | null, afterContent: string | null): void {
		const canonical = this.#canonicalize(filePath);
		this.#snapshots.push({
			path: canonical,
			before: beforeContent,
			after: afterContent,
			turnId,
			timestamp: Date.now(),
		});
	}

	/** Get snapshots for a specific turn */
	getTurnSnapshots(turnId: string): readonly TurnSnapshot[] {
		return this.#snapshots.filter(s => s.turnId === turnId);
	}

	/**
	 * Roll back all file mutations made during a specific turn.
	 * Restores files to their exact `before` state atomically.
	 */
	async undoTurn(turnId: string): Promise<RestoreResult[]> {
		const turnSnapshots = this.#snapshots.filter(s => s.turnId === turnId);
		const results: RestoreResult[] = [];

		// Roll back in reverse order of mutation
		for (let i = turnSnapshots.length - 1; i >= 0; i--) {
			const snap = turnSnapshots[i];
			try {
				if (snap.before === null) {
					// File was created in this turn; remove it
					if (fs.existsSync(snap.path)) {
						await fs.promises.unlink(snap.path);
						results.push({ path: snap.path, restored: true, action: "deleted_new_file" });
					} else {
						results.push({ path: snap.path, restored: true, action: "deleted_new_file" });
					}
				} else {
					// File existed; restore prior content
					await atomicWriteFile(snap.path, snap.before);
					results.push({ path: snap.path, restored: true, action: "restored_content" });
				}
			} catch (err: unknown) {
				const msg = err instanceof Error ? err.message : String(err);
				results.push({ path: snap.path, restored: false, action: "failed", error: msg });
			}
		}

		// Remove the undone snapshots from memory
		for (let i = this.#snapshots.length - 1; i >= 0; i--) {
			if (this.#snapshots[i].turnId === turnId) {
				this.#snapshots.splice(i, 1);
			}
		}

		return results;
	}

	/** Undo the most recent turn on the stack */
	async undoLastTurn(): Promise<{ turnId: string; results: RestoreResult[] } | null> {
		if (this.#snapshots.length === 0) return null;
		const lastTurnId = this.#snapshots[this.#snapshots.length - 1].turnId;
		const results = await this.undoTurn(lastTurnId);
		return { turnId: lastTurnId, results };
	}

	get snapshotCount(): number {
		return this.#snapshots.length;
	}

	clear(): void {
		this.#snapshots.length = 0;
	}
}
