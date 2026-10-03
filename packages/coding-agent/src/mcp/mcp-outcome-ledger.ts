/**
 * In-memory MCP outcome ledger (Stream-F F5 slice).
 *
 * The bridge reports uncertain deliveries as a `outcomeUnknown: true` flag on
 * the result details with "verify the remote state" guidance and never
 * replays (`tool-bridge.ts`, read-only). That flag alone cannot answer "what
 * was the last outcome for this idempotency key" — this ledger keeps the
 * last N outcomes keyed by the stable per-call idempotency key so callers
 * and diagnostics can surface them in result details.
 *
 * Observe-only: recording here never triggers a replay, retry, or state
 * change on the server. Replay policy stays exactly as implemented in the
 * bridge (pre-dispatch reconnect, read-only once, write only with a
 * server-provided key).
 */

export type McpOutcomeStatus = "committed" | "unknown" | "failed";

export interface McpOutcomeInput {
	readonly key: string;
	readonly status: McpOutcomeStatus;
	readonly serverName: string;
	readonly toolName: string;
	readonly failure?: string;
}

export interface McpOutcomeEntry {
	readonly key: string;
	readonly status: McpOutcomeStatus;
	readonly serverName: string;
	readonly toolName: string;
	readonly failure?: string;
	readonly at: number;
}

/** Details fragment surfaced alongside `MCPToolDetails` (additive, optional). */
export interface McpOutcomeDetails {
	readonly outcome?: McpOutcomeStatus;
	readonly outcomeKey?: string;
	readonly outcomeAt?: number;
}

/** Default ring size: enough for recent-call diagnostics, bounded for memory. */
export const DEFAULT_OUTCOME_LEDGER_CAPACITY = 100;
/** Hard ceiling so a misconfigured capacity cannot grow without bound. */
export const MAX_OUTCOME_LEDGER_CAPACITY = 1000;

const OUTCOME_STATUSES: ReadonlySet<McpOutcomeStatus> = new Set(["committed", "unknown", "failed"]);
const KEY_PATTERN = /^[A-Za-z0-9._:/-]+$/;
const MAX_KEY_CHARS = 128;
const MAX_NAME_CHARS = 256;
const MAX_FAILURE_CHARS = 1000;

function sanitizeKey(value: unknown): string | undefined {
	if (typeof value !== "string") return undefined;
	const trimmed = value.trim();
	if (trimmed.length === 0 || trimmed.length > MAX_KEY_CHARS) return undefined;
	return KEY_PATTERN.test(trimmed) ? trimmed : undefined;
}

function sanitizeName(value: unknown): string | undefined {
	if (typeof value !== "string") return undefined;
	const trimmed = value.trim();
	if (trimmed.length === 0 || trimmed.length > MAX_NAME_CHARS) return undefined;
	return trimmed;
}

function sanitizeFailure(value: unknown): string | undefined {
	if (value === undefined) return undefined;
	if (typeof value !== "string") return undefined;
	const trimmed = value.trim();
	if (trimmed.length === 0) return undefined;
	return trimmed.slice(0, MAX_FAILURE_CHARS);
}

function normalizeCapacity(value: unknown): number {
	if (typeof value !== "number" || !Number.isFinite(value)) return DEFAULT_OUTCOME_LEDGER_CAPACITY;
	const floored = Math.floor(value);
	if (floored <= 0) return DEFAULT_OUTCOME_LEDGER_CAPACITY;
	return Math.min(floored, MAX_OUTCOME_LEDGER_CAPACITY);
}

/**
 * Bounded FIFO ledger of recent MCP outcomes. Re-recording an existing key
 * refreshes its recency; the oldest entry is evicted past capacity.
 */
export class McpOutcomeLedger {
	readonly capacity: number;
	readonly #entries = new Map<string, McpOutcomeEntry>();

	constructor(capacity: number = DEFAULT_OUTCOME_LEDGER_CAPACITY) {
		this.capacity = normalizeCapacity(capacity);
	}

	get size(): number {
		return this.#entries.size;
	}

	/**
	 * Record one outcome. Returns the stored entry, or `undefined` when the
	 * input is invalid (fail-closed: invalid keys/names/statuses are dropped,
	 * never recorded under a fallback key).
	 */
	record(input: McpOutcomeInput): McpOutcomeEntry | undefined {
		const key = sanitizeKey(input.key);
		const serverName = sanitizeName(input.serverName);
		const toolName = sanitizeName(input.toolName);
		if (key === undefined || serverName === undefined || toolName === undefined) return undefined;
		if (!OUTCOME_STATUSES.has(input.status)) return undefined;
		const failure = sanitizeFailure(input.failure);
		const entry: McpOutcomeEntry = {
			key,
			status: input.status,
			serverName,
			toolName,
			...(failure !== undefined ? { failure } : {}),
			at: Date.now(),
		};
		if (this.#entries.has(key)) this.#entries.delete(key);
		this.#entries.set(key, entry);
		while (this.#entries.size > this.capacity) {
			const oldest = this.#entries.keys().next();
			if (oldest.done === true) break;
			this.#entries.delete(oldest.value);
		}
		return entry;
	}

	/** Latest entry for `key`, or `undefined` for unknown/invalid keys. */
	lookup(key: unknown): McpOutcomeEntry | undefined {
		const clean = sanitizeKey(key);
		if (clean === undefined) return undefined;
		const entry = this.#entries.get(clean);
		return entry === undefined ? undefined : { ...entry };
	}

	/** Snapshot in oldest-first order. */
	entries(): readonly McpOutcomeEntry[] {
		return Array.from(this.#entries.values()).map(entry => ({ ...entry }));
	}

	clear(): void {
		this.#entries.clear();
	}
}

/**
 * Build the additive details fragment for one idempotency key. Returns an
 * empty object when the key has no recorded outcome — callers spread this
 * onto result details so absent ledgers add no fields.
 */
export function outcomeDetailsFor(ledger: McpOutcomeLedger, key: unknown): McpOutcomeDetails {
	const entry = ledger.lookup(key);
	if (entry === undefined) return {};
	return { outcome: entry.status, outcomeKey: entry.key, outcomeAt: entry.at };
}

/**
 * Return `details` with the ledger outcome merged in. The input is never
 * mutated; replay behavior is untouched — this only annotates.
 */
export function annotateMcpDetails<T extends Record<string, unknown>>(
	details: T,
	ledger: McpOutcomeLedger,
	key: unknown,
): T & McpOutcomeDetails {
	return { ...details, ...outcomeDetailsFor(ledger, key) };
}
