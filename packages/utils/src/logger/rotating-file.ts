/** Behavior-compatible reimplementation of winston-daily-rotate-file's used surface with async buffering. */
import * as crypto from "node:crypto";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

interface AuditEntry {
	readonly date: number;
	readonly name: string;
	readonly hash: string;
}

interface AuditState {
	readonly keep: { readonly days: false; readonly amount: number };
	readonly auditLog: string;
	readonly files: AuditEntry[];
	readonly hashType: "sha256";
}

/** Configuration for a process-local rotating file sink. */
export interface RotatingFileOptions {
	readonly directory: string;
	readonly filenamePrefix: string;
	readonly filenameSuffix: string;
	readonly auditFile: string;
	readonly maxBytes: number;
	readonly maxFiles: number;
	readonly flushIntervalMs?: number;
	readonly bufferThresholdBytes?: number;
}

interface QueuedRecord {
	readonly record: string;
	readonly date: Date;
	readonly bytes: number;
}

function isAuditEntry(value: unknown): value is AuditEntry {
	if (value === null || typeof value !== "object") return false;
	const entry = value as Record<string, unknown>;
	return typeof entry.date === "number" && typeof entry.name === "string" && typeof entry.hash === "string";
}

/** Buffered append sink with local-day and size rotation plus bounded retention. */
export class RotatingFileSink {
	readonly #directory: string;
	readonly #filenamePrefix: string;
	readonly #filenameSuffix: string;
	readonly #auditFile: string;
	readonly #maxBytes: number;
	readonly #maxFiles: number;
	readonly #flushIntervalMs: number;
	readonly #bufferThresholdBytes: number;

	#files: AuditEntry[];
	#activeDay: string | undefined;
	#activeIndex = 0;
	#activePath: string | undefined;
	#activeBytes = 0;
	#closed = false;

	#queue: QueuedRecord[] = [];
	#bufferedBytes = 0;
	#timer: NodeJS.Timeout | undefined;
	#flushing = false;
	#flushPromise: Promise<void> | undefined;
	#drainScheduled = false;
	#cleanupExitHooks: (() => void) | undefined;

	constructor(options: RotatingFileOptions) {
		this.#directory = options.directory;
		this.#filenamePrefix = options.filenamePrefix;
		this.#filenameSuffix = options.filenameSuffix;
		this.#auditFile = options.auditFile;
		this.#maxBytes = options.maxBytes;
		this.#maxFiles = options.maxFiles;
		this.#flushIntervalMs = options.flushIntervalMs ?? 250;
		this.#bufferThresholdBytes = options.bufferThresholdBytes ?? 64 * 1024;
		this.#files = this.#readAudit();
		const now = new Date();
		this.#selectFile(this.#localDay(now));
		const activePath = this.#activePath;
		if (activePath) {
			this.#registerFile(activePath, now.getTime());
			fs.closeSync(fs.openSync(activePath, "a"));
		}

		if (this.#flushIntervalMs > 0) {
			this.#timer = setInterval(() => {
				void this.flush();
			}, this.#flushIntervalMs);
			this.#timer.unref();
		}

		this.#setupExitHooks();
	}

	get queueLength(): number {
		return this.#queue.length;
	}

	get bufferedBytes(): number {
		return this.#bufferedBytes;
	}

	/** Append one already-formatted log record to the in-memory buffer. */
	write(line: string): void {
		if (this.#closed) return;
		const record = `${line}${os.EOL}`;
		const bytes = Buffer.byteLength(record);
		this.#queue.push({ record, date: new Date(), bytes });
		this.#bufferedBytes += bytes;

		if (this.#bufferedBytes >= this.#bufferThresholdBytes && !this.#drainScheduled) {
			this.#drainScheduled = true;
			setImmediate(() => {
				this.#drainScheduled = false;
				void this.flush();
			});
		}
	}

	/** Asynchronously flush buffered records to disk. */
	async flush(): Promise<void> {
		if (this.#closed && this.#queue.length === 0) return;
		if (this.#flushing) {
			await this.#flushPromise;
			if (this.#queue.length > 0) {
				return this.flush();
			}
			return;
		}
		if (this.#queue.length === 0) return;

		this.#flushing = true;
		const records = this.#queue.splice(0);
		this.#bufferedBytes = 0;

		this.#flushPromise = (async () => {
			try {
				let currentBatchPath: string | undefined;
				let currentBatchText = "";

				for (const record of records) {
					const day = this.#localDay(record.date);
					this.#selectFile(day);
					const activePath = this.#activePath;
					if (!activePath) continue;

					if (activePath !== currentBatchPath) {
						if (currentBatchPath && currentBatchText.length > 0) {
							await fs.promises.appendFile(currentBatchPath, currentBatchText, "utf8");
							currentBatchText = "";
						}
						this.#registerFile(activePath, record.date.getTime());
						currentBatchPath = activePath;
					}
					currentBatchText += record.record;
					this.#activeBytes += record.bytes;
				}

				if (currentBatchPath && currentBatchText.length > 0) {
					await fs.promises.appendFile(currentBatchPath, currentBatchText, "utf8");
				}
			} catch {
				// Writing is best effort
			} finally {
				this.#flushing = false;
			}
		})();

		await this.#flushPromise;
		if (this.#queue.length > 0) {
			await this.flush();
		}
	}

	/** Synchronously flush any buffered records immediately. */
	flushSync(): void {
		if (this.#queue.length === 0) return;
		const records = this.#queue.splice(0);
		this.#bufferedBytes = 0;

		let currentBatchPath: string | undefined;
		let currentBatchText = "";

		for (const record of records) {
			const day = this.#localDay(record.date);
			this.#selectFile(day);
			const activePath = this.#activePath;
			if (!activePath) continue;

			if (activePath !== currentBatchPath) {
				if (currentBatchPath && currentBatchText.length > 0) {
					try {
						fs.appendFileSync(currentBatchPath, currentBatchText, "utf8");
					} catch {
						// Writing is best effort
					}
					currentBatchText = "";
				}
				this.#registerFile(activePath, record.date.getTime());
				currentBatchPath = activePath;
			}
			currentBatchText += record.record;
			this.#activeBytes += record.bytes;
		}

		if (currentBatchPath && currentBatchText.length > 0) {
			try {
				fs.appendFileSync(currentBatchPath, currentBatchText, "utf8");
			} catch {
				// Writing is best effort
			}
		}
	}

	/** Stop accepting records, clear timers, and flush remaining records synchronously. */
	close(): void {
		if (this.#closed) return;
		if (this.#timer) {
			clearInterval(this.#timer);
			this.#timer = undefined;
		}
		this.#cleanupExitHooks?.();
		this.#cleanupExitHooks = undefined;
		this.flushSync();
		this.#closed = true;
	}

	#setupExitHooks(): void {
		const onExit = () => {
			this.flushSync();
		};
		const onSignal = (signal: NodeJS.Signals) => {
			this.flushSync();
			if (process.listenerCount(signal) <= 1) {
				process.exit(signal === "SIGINT" ? 130 : 143);
			}
		};

		process.on("exit", onExit);
		process.on("beforeExit", onExit);
		process.on("SIGINT", onSignal);
		process.on("SIGTERM", onSignal);

		this.#cleanupExitHooks = () => {
			process.off("exit", onExit);
			process.off("beforeExit", onExit);
			process.off("SIGINT", onSignal);
			process.off("SIGTERM", onSignal);
		};
	}

	#localDay(date: Date): string {
		return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
	}

	#selectFile(day: string): void {
		if (day !== this.#activeDay) {
			this.#activeDay = day;
			this.#activeIndex = 0;
			this.#setActivePath(day, 0);
		}
		while (this.#activeBytes > this.#maxBytes) {
			this.#activeIndex++;
			this.#setActivePath(day, this.#activeIndex);
		}
	}

	#setActivePath(day: string, index: number): void {
		const suffix = index === 0 ? "" : `.${index}`;
		this.#activePath = path.join(
			this.#directory,
			`${this.#filenamePrefix}.${day}.${this.#filenameSuffix}.log${suffix}`,
		);
		try {
			this.#activeBytes = fs.statSync(this.#activePath).size;
		} catch {
			this.#activeBytes = 0;
		}
	}

	#registerFile(filePath: string, date: number): void {
		if (this.#files.some(file => file.name === filePath)) return;
		const hash = crypto.createHash("sha256").update(`${filePath}LOG_FILE${date}`).digest("hex");
		this.#files.push({ date, name: filePath, hash });
		while (this.#files.length > this.#maxFiles) {
			const removed = this.#files.shift();
			if (!removed) break;
			try {
				fs.rmSync(removed.name, { force: true });
			} catch {
				// Retention is best-effort; the current record must still be written.
			}
		}
		this.#writeAudit();
	}

	#readAudit(): AuditEntry[] {
		try {
			const parsed = JSON.parse(fs.readFileSync(this.#auditFile, "utf8")) as { files?: unknown };
			return Array.isArray(parsed.files) ? parsed.files.filter(isAuditEntry) : [];
		} catch {
			return [];
		}
	}

	#writeAudit(): void {
		const state: AuditState = {
			keep: { days: false, amount: this.#maxFiles },
			auditLog: this.#auditFile,
			files: this.#files,
			hashType: "sha256",
		};
		fs.writeFileSync(this.#auditFile, JSON.stringify(state, undefined, 4), "utf8");
	}
}
