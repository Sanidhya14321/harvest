/**
 * File Freshness Tracking.
 *
 * Tracks mtimeMs, size, and SHA-256 hash when files are read.
 * Asserts freshness before any mutation is applied.
 */

import * as crypto from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";

export interface FreshnessRecord {
	readonly path: string;
	readonly mtimeMs: number;
	readonly size: number;
	readonly sha256: string;
	readonly timestamp: number;
}

export class FreshnessTracker {
	readonly #records: Map<string, FreshnessRecord> = new Map<string, FreshnessRecord>();
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

	#computeSha256(filePath: string): string {
		const buffer = fs.readFileSync(filePath);
		return crypto.createHash("sha256").update(buffer).digest("hex");
	}

	#computeStringSha256(content: string): string {
		return crypto.createHash("sha256").update(content, "utf8").digest("hex");
	}

	/** Record file state from disk or supplied content. */
	recordRead(filePath: string, content?: string): FreshnessRecord | null {
		const canonical = this.#canonicalize(filePath);
		try {
			const stat = fs.statSync(canonical);
			const sha256 = content !== undefined ? this.#computeStringSha256(content) : this.#computeSha256(canonical);
			const record: FreshnessRecord = {
				path: canonical,
				mtimeMs: stat.mtimeMs,
				size: stat.size,
				sha256,
				timestamp: Date.now(),
			};
			this.#records.set(canonical, record);
			return record;
		} catch {
			return null;
		}
	}

	/**
	 * Assert that the file on disk has not been modified since it was last read.
	 * Throws an actionable error if stale.
	 */
	assertCurrent(filePath: string): void {
		const canonical = this.#canonicalize(filePath);
		const lastRecord = this.#records.get(canonical);

		if (!lastRecord) {
			// If not recorded in freshness tracker, verify file exists
			if (!fs.existsSync(canonical)) {
				throw new Error(`File '${filePath}' does not exist on disk.`);
			}
			return;
		}

		let stat: fs.Stats;
		try {
			stat = fs.statSync(canonical);
		} catch (err: unknown) {
			const msg = err instanceof Error ? err.message : String(err);
			throw new Error(`Cannot verify freshness for '${filePath}': ${msg}`);
		}

		// Fast path: mtime and size match exactly
		if (stat.mtimeMs === lastRecord.mtimeMs && stat.size === lastRecord.size) {
			return;
		}

		// If mtime or size differs, verify SHA-256 hash
		const currentHash = this.#computeSha256(canonical);
		if (currentHash !== lastRecord.sha256) {
			const rel = path.relative(this.#workspaceRoot, canonical).replace(/\\/g, "/");
			throw new Error(
				`${rel || filePath} was modified since it was read (by another tool, a formatter, or another process). Re-read the file and re-apply your edit against the current content.`,
			);
		}

		// If content hash matches despite mtime bump (e.g. touch or formatting no-op), refresh record
		this.#records.set(canonical, {
			...lastRecord,
			mtimeMs: stat.mtimeMs,
			size: stat.size,
		});
	}

	/** Update freshness record after a successful mutation. */
	recordMutation(filePath: string, newContent?: string): void {
		const canonical = this.#canonicalize(filePath);
		try {
			const stat = fs.statSync(canonical);
			const sha256 = newContent !== undefined ? this.#computeStringSha256(newContent) : this.#computeSha256(canonical);
			this.#records.set(canonical, {
				path: canonical,
				mtimeMs: stat.mtimeMs,
				size: stat.size,
				sha256,
				timestamp: Date.now(),
			});
		} catch {}
	}

	getRecord(filePath: string): FreshnessRecord | undefined {
		const canonical = this.#canonicalize(filePath);
		return this.#records.get(canonical);
	}

	clear(): void {
		this.#records.clear();
	}
}
