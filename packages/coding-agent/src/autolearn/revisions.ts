/**
 * Revision service for managed skills and managed agent presets.
 *
 * One store for both artifact kinds: immutable revisions with parent links,
 * provenance, draft/active state, evaluation records, and an atomic
 * active-revision pointer. Retains the latest twenty revisions per artifact;
 * older revisions pinned by live runs survive until unpinned.
 *
 * "Improvement" here means versioned skill/preset iteration, not model-weight
 * training.
 */
import * as path from "node:path";
import { AsyncLocalStorage } from "node:async_hooks";
import { getAgentDir, isEnoent, logger } from "@harvest/pi-utils";
import type { ToolSession } from "../tools/index";

export const MAX_REVISIONS_PER_ARTIFACT = 20;
export const MAX_MANAGED_ARTIFACT_BYTES = 64_000;

/** Minted revision ID grammar: `rev-` plus hyphen-joined base36 segments. */
const REVISION_ID_PATTERN = /^rev-[a-z0-9]+(?:-[a-z0-9]+)*$/;

/**
 * Reject path separators, absolute paths, and traversal in revision IDs.
 * Matches the minted grammar (see `newRevisionId`); aligned with the
 * managed skill/preset validators.
 */
export function assertRevisionId(revId: string): string {
	const trimmed = revId.trim();
	if (
		!REVISION_ID_PATTERN.test(trimmed) ||
		path.isAbsolute(trimmed) ||
		trimmed.includes("/") ||
		trimmed.includes("\\") ||
		trimmed.includes("..")
	) {
		throw new Error(
			`Invalid revision ID "${revId}": expected rev-[a-z0-9]+(-[a-z0-9]+)*, no separators, absolute paths, or traversal.`,
		);
	}
	return trimmed;
}

/** Reject path separators, absolute paths, and traversal in artifact names. */
export function assertArtifactName(name: string): string {
	if (!name || name.includes("/") || name.includes("\\") || name.includes("..") || path.isAbsolute(name)) {
		throw new Error(`Invalid artifact name "${name}": no path separators, absolute paths, or traversal.`);
	}
	return name;
}

/**
 * Single per-artifact serialized transaction boundary for ALL mutation
 * verbs (create/update/delete/draft/materialize/promote/rollback/
 * evaluate-append). Same-artifact callers run in submission order; different
 * artifacts proceed in parallel. In-process only; cross-process races are
 * out of scope and must additionally rely on atomic file replacement.
 */
const revisionTransactionChains = new Map<string, Promise<unknown>>();
/**
 * Async-context tokens currently executing inside {@link withRevisionTransaction}.
 * Re-entrancy is scoped per async context AND per holder generation (not a
 * global set): a nested call made while holding this artifact's lock runs
 * directly instead of chaining behind itself (which would deadlock), while
 * concurrent top-level callers — each in their own context — still serialize
 * behind the holder in submission order.
 *
 * Authority expires with the owning transaction: each holder mints a fresh
 * token, registered in {@link revisionTransactionActive} for the key's
 * lifetime. A nested call is re-entrant only when its inherited token still
 * matches the live holder. Fire-and-forget work created inside the lock but
 * settling after release (inherited delayed callbacks) carries a stale token
 * and therefore queues behind the next holder instead of sharing dead
 * authority.
 */
const revisionTransactionScope = new AsyncLocalStorage<Map<string, symbol>>();
const revisionTransactionActive = new Map<string, symbol>();

function revisionTransactionKey(kind: RevisionKind, name: string, agentDir?: string): string {
	const dir = path.resolve(agentDir ?? getAgentDir());
	return `${dir}|${kind}:${assertArtifactName(name)}`;
}

/**
 * Run `fn` without any inherited revision-transaction authority. Auto-prune
 * triggers fired from inside a mutation must queue behind the holder instead
 * of running concurrently on stale re-entrancy: wrapping the trigger keeps
 * unlink ownership through completion.
 */
export function withoutRevisionTransactionScope<T>(fn: () => T): T {
	return revisionTransactionScope.run(new Map(), fn);
}

export function withRevisionTransaction<T>(
	kind: RevisionKind,
	name: string,
	fn: () => Promise<T>,
	agentDir?: string,
): Promise<T> {
	const key = revisionTransactionKey(kind, name, agentDir);
	// Re-entrant only while the owning holder is still live in this context.
	// A stale inherited token (delayed callback outliving its transaction, or
	// a new generation holding the same key) forces a queue instead of a
	// direct run.
	const inherited = revisionTransactionScope.getStore()?.get(key);
	if (inherited !== undefined && revisionTransactionActive.get(key) === inherited) return fn();
	const prev = revisionTransactionChains.get(key) ?? Promise.resolve();
	const runInner = async (): Promise<T> => {
		const token = Symbol(key);
		revisionTransactionActive.set(key, token);
		const nested = new Map(revisionTransactionScope.getStore());
		nested.set(key, token);
		try {
			return await revisionTransactionScope.run(nested, fn);
		} finally {
			// Expire authority: delayed callbacks inheriting this token no
			// longer match, so they serialize instead of sharing the lock.
			if (revisionTransactionActive.get(key) === token) revisionTransactionActive.delete(key);
		}
	};
	const run = prev.then(runInner, runInner);
	const guarded = run.catch(() => {});
	revisionTransactionChains.set(key, guarded);
	void guarded.finally(() => {
		if (revisionTransactionChains.get(key) === guarded) revisionTransactionChains.delete(key);
	});
	return run;
}

export type RevisionState = "draft" | "active";
export type RevisionKind = "skill" | "preset";

/**
 * Race long-lived evaluation work against the caller's AbortSignal so
 * cancellation stays prompt even when the runner ignores the signal (a hung
 * provider call must not wedge promotion or pin release). `work` starts
 * lazily — a pre-aborted signal rejects without starting it, so no stray
 * execution (and no unhandled rejection) escapes. `Promise.race` subscribes
 * to the started work, so a runner that settles later can neither resolve
 * the race nor surface an unhandled rejection. The abort listener is always
 * removed before this returns.
 */
export function raceWithAbortSignal<T>(
	signal: AbortSignal | undefined,
	work: () => Promise<T>,
	makeAbortError: () => Error,
): Promise<T> {
	if (!signal) return work();
	if (signal.aborted) return Promise.reject(makeAbortError());
	let onAbort: (() => void) | undefined;
	const abortWait = new Promise<never>((_, reject) => {
		onAbort = () => reject(makeAbortError());
		signal.addEventListener("abort", onAbort, { once: true });
	});
	return Promise.race([work(), abortWait]).finally(() => {
		if (onAbort) signal.removeEventListener("abort", onAbort);
	});
}

/** Scope opened by {@link createToolEvalSignal}. */
export interface ToolEvalSignalScope {
	/** Signal fused from the tool call plus parent-disposal linkage. */
	signal: AbortSignal;
	/**
	 * Register evaluation work so parent disposal aborts it (via the fused
	 * controller) and awaits its settlement. Falls back to the raw promise
	 * when the parent exposes no tracker (detached tool instances in tests).
	 */
	track<T>(execution: Promise<T>): Promise<T>;
	/** Remove the tool-call listener. Call in a finally after settlement. */
	release(): void;
}

/**
 * Fuse a tool call's AbortSignal with a disposal-linked controller for one
 * evaluation run: the tool signal aborts the controller, and registering the
 * run via {@link ToolEvalSignalScope.track} lets parent session disposal
 * abort it too. Cancellation never touches settings (runtime signal only).
 */
export function createToolEvalSignal(
	parent: Pick<ToolSession, "trackEvalExecution"> | undefined,
	signal?: AbortSignal,
): ToolEvalSignalScope {
	const controller = new AbortController();
	let onAbort: (() => void) | undefined;
	if (signal) {
		if (signal.aborted) {
			controller.abort(signal.reason);
		} else {
			onAbort = () => controller.abort(signal.reason);
			signal.addEventListener("abort", onAbort, { once: true });
		}
	}
	return {
		signal: controller.signal,
		track<T>(execution: Promise<T>): Promise<T> {
			return parent?.trackEvalExecution?.(execution, controller) ?? execution;
		},
		release(): void {
			if (onAbort && signal) signal.removeEventListener("abort", onAbort);
		},
	};
}

export interface EvaluationRecord {
	id: string;
	task: string;
	expectedOutcome: string;
	passed: boolean;
	summary: string;
	runId?: string;
	sessionId?: string;
	/** Resolved evaluation model selector (e.g. "provider/id"); recorded for audit. */
	model?: string;
	/** How the evaluation model was resolved (role/pattern/fallback basis). */
	modelBasis?: string;
	createdAt: string;
}

export interface ArtifactRevision {
	id: string;
	kind: RevisionKind;
	name: string;
	parent: string | null;
	content: string;
	description: string;
	state: RevisionState;
	provenance: { sessionId?: string; runId?: string; actor: string };
	evaluations: EvaluationRecord[];
	createdAt: string;
}

export interface RevisionPointer {
	active: string | null;
	updatedAt: string;
}

function revisionRoot(kind: RevisionKind, agentDir: string = getAgentDir()): string {
	return path.join(agentDir, "managed-revisions", kind === "skill" ? "skills" : "presets");
}

function artifactDir(kind: RevisionKind, name: string, agentDir?: string): string {
	return path.join(revisionRoot(kind, agentDir), assertArtifactName(name));
}

function revisionFile(kind: RevisionKind, name: string, revId: string, agentDir?: string): string {
	return path.join(artifactDir(kind, name, agentDir), `${revId}.json`);
}

function pointerFile(kind: RevisionKind, name: string, agentDir?: string): string {
	return path.join(artifactDir(kind, name, agentDir), "active.json");
}

function newRevisionId(): string {
	return `rev-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

async function listRevisionFiles(kind: RevisionKind, name: string, agentDir?: string): Promise<string[]> {
	const dir = artifactDir(kind, name, agentDir);
	const entries = await Array.fromAsync(new Bun.Glob("rev-*.json").scan({ cwd: dir })).catch(() => []);
	return entries.sort();
}

export async function readRevision(
	kind: RevisionKind,
	name: string,
	revId: string,
	agentDir?: string,
): Promise<ArtifactRevision | undefined> {
	try {
		return (await Bun.file(revisionFile(kind, name, assertRevisionId(revId), agentDir)).json()) as ArtifactRevision;
	} catch (err) {
		if (isEnoent(err)) return undefined;
		throw err;
	}
}

export async function readActivePointer(kind: RevisionKind, name: string, agentDir?: string): Promise<RevisionPointer> {
	try {
		return (await Bun.file(pointerFile(kind, name, agentDir)).json()) as RevisionPointer;
	} catch (err) {
		if (isEnoent(err)) return { active: null, updatedAt: new Date().toISOString() };
		throw err;
	}
}

async function writePointerAtomic(
	kind: RevisionKind,
	name: string,
	pointer: RevisionPointer,
	agentDir?: string,
): Promise<void> {
	const file = pointerFile(kind, name, agentDir);
	await Bun.write(`${file}.tmp`, JSON.stringify(pointer, null, "\t"));
	const { rename } = await import("node:fs/promises");
	await rename(`${file}.tmp`, file);
}

export async function readActiveRevision(
	kind: RevisionKind,
	name: string,
	agentDir?: string,
): Promise<ArtifactRevision | undefined> {
	const pointer = await readActivePointer(kind, name, agentDir);
	if (!pointer.active) return undefined;
	return readRevision(kind, name, pointer.active, agentDir);
}

export interface CreateRevisionOptions {
	kind: RevisionKind;
	name: string;
	content: string;
	description?: string;
	parent?: string | null;
	/** Expected current active revision; rejects on conflict. */
	expectedActive?: string | null;
	provenance?: { sessionId?: string; runId?: string; actor?: string };
	agentDir?: string;
}

export async function createDraftRevision(opts: CreateRevisionOptions): Promise<ArtifactRevision> {
	if (opts.content.length > MAX_MANAGED_ARTIFACT_BYTES) {
		throw new Error(`Revision content exceeds ${MAX_MANAGED_ARTIFACT_BYTES} bytes`);
	}
	const pointer = await readActivePointer(opts.kind, opts.name, opts.agentDir);
	if (opts.expectedActive !== undefined && pointer.active !== opts.expectedActive) {
		throw new Error(
			`Revision conflict for ${opts.kind} "${opts.name}": expected active ${opts.expectedActive ?? "none"}, found ${pointer.active ?? "none"}`,
		);
	}
	const revision: ArtifactRevision = {
		id: newRevisionId(),
		kind: opts.kind,
		name: opts.name,
		parent: opts.parent ?? pointer.active,
		content: opts.content,
		description: opts.description ?? "",
		state: "draft",
		provenance: {
			sessionId: opts.provenance?.sessionId,
			runId: opts.provenance?.runId,
			actor: opts.provenance?.actor ?? "model",
		},
		evaluations: [],
		createdAt: new Date().toISOString(),
	};
	await Bun.write(
		revisionFile(opts.kind, opts.name, revision.id, opts.agentDir),
		JSON.stringify(revision, null, "\t"),
	);
	return revision;
}

export async function recordEvaluation(
	kind: RevisionKind,
	name: string,
	revId: string,
	evalRecord: Omit<EvaluationRecord, "id" | "createdAt">,
	agentDir?: string,
): Promise<ArtifactRevision> {
	const cleanRev = assertRevisionId(revId);
	const revision = await readRevision(kind, name, cleanRev, agentDir);
	if (!revision) throw new Error(`Revision ${cleanRev} for ${kind} "${name}" not found`);
	const record: EvaluationRecord = {
		...evalRecord,
		id: `eval-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`,
		createdAt: new Date().toISOString(),
	};
	revision.evaluations.push(record);
	await Bun.write(revisionFile(kind, name, cleanRev, agentDir), JSON.stringify(revision, null, "\t"));
	return revision;
}

export function isEvaluatedPassing(revision: ArtifactRevision): boolean {
	return revision.evaluations.length > 0 && revision.evaluations.every(record => record.passed);
}

/** Promote a draft to active. Unevaluated content promotes only with explicit discloseUnevaluated. */
export async function promoteRevision(
	kind: RevisionKind,
	name: string,
	revId: string,
	opts?: { discloseUnevaluated?: boolean; agentDir?: string },
): Promise<RevisionPointer> {
	const cleanRev = assertRevisionId(revId);
	const revision = await readRevision(kind, name, cleanRev, opts?.agentDir);
	if (!revision) throw new Error(`Revision ${cleanRev} for ${kind} "${name}" not found`);
	if (revision.evaluations.length === 0 && !opts?.discloseUnevaluated) {
		throw new Error(
			`Revision ${cleanRev} for ${kind} "${name}" has no evaluations; pass discloseUnevaluated to activate unevaluated content explicitly`,
		);
	}
	const originalBytes = JSON.stringify(revision, null, "\t");
	revision.state = "active";
	await Bun.write(revisionFile(kind, name, cleanRev, opts?.agentDir), JSON.stringify(revision, null, "\t"));
	try {
		const pointer: RevisionPointer = { active: cleanRev, updatedAt: new Date().toISOString() };
		await writePointerAtomic(kind, name, pointer, opts?.agentDir);
		return pointer;
	} catch (err) {
		// Pointer never published: the revision was never active. Restore the
		// original history bytes so a later rollback cannot target this
		// false-active revision.
		try {
			await Bun.write(revisionFile(kind, name, cleanRev, opts?.agentDir), originalBytes);
		} catch (restoreErr) {
			logger.warn("Failed to restore revision state after pointer failure", { error: restoreErr });
		}
		throw err;
	}
}

/**
 * Roll back the active pointer to a prior revision. Only previously
 * activated revisions are valid targets: activating a draft that never
 * published (or a failing revision) as a rollback would bypass evaluation.
 */
export async function rollbackRevision(
	kind: RevisionKind,
	name: string,
	revId: string,
	agentDir?: string,
): Promise<RevisionPointer> {
	const cleanRev = assertRevisionId(revId);
	const revision = await readRevision(kind, name, cleanRev, agentDir);
	if (!revision) throw new Error(`Revision ${cleanRev} for ${kind} "${name}" not found`);
	if (revision.state !== "active") {
		throw new Error(
			`Cannot rollback ${kind} "${name}" to revision ${cleanRev}: it was never active (state ${revision.state}).`,
		);
	}
	const pointer: RevisionPointer = { active: cleanRev, updatedAt: new Date().toISOString() };
	await writePointerAtomic(kind, name, pointer, agentDir);
	return pointer;
}

/**
 * Atomic promotion combining materialization and pointer publication:
 * prevalidated content is materialized first, the pointer publishes only on
 * success, and a failed publication rolls the materialization back — the
 * previously active revision stays published throughout. Callers run this
 * inside withRevisionTransaction; interrupted runs recover by re-reading
 * the pointer (file-ahead content rolls back on next mutation).
 */
export async function promoteRevisionAtomic(
	kind: RevisionKind,
	name: string,
	revId: string,
	materialize: { run: () => Promise<void>; rollback: () => Promise<void> },
	opts?: { discloseUnevaluated?: boolean; agentDir?: string },
): Promise<RevisionPointer> {
	const cleanRev = assertRevisionId(revId);
	const revision = await readRevision(kind, name, cleanRev, opts?.agentDir);
	if (!revision) throw new Error(`Revision ${cleanRev} for ${kind} "${name}" not found`);
	if (revision.evaluations.length > 0 && !isEvaluatedPassing(revision)) {
		throw new Error(`Revision ${cleanRev} for ${kind} "${name}" has failing evaluations; it cannot be promoted.`);
	}
	if (revision.evaluations.length === 0 && !opts?.discloseUnevaluated) {
		throw new Error(
			`Revision ${cleanRev} for ${kind} "${name}" has no evaluations; pass discloseUnevaluated to activate unevaluated content explicitly`,
		);
	}
	await materialize.run();
	try {
		const originalBytes = JSON.stringify(revision, null, "\t");
		revision.state = "active";
		await Bun.write(revisionFile(kind, name, cleanRev, opts?.agentDir), JSON.stringify(revision, null, "\t"));
		try {
			const pointer: RevisionPointer = { active: cleanRev, updatedAt: new Date().toISOString() };
			await writePointerAtomic(kind, name, pointer, opts?.agentDir);
			return pointer;
		} catch (err) {
			try {
				await Bun.write(revisionFile(kind, name, cleanRev, opts?.agentDir), originalBytes);
			} catch (restoreErr) {
				logger.warn("Failed to restore revision state after pointer failure", { error: restoreErr });
			}
			throw err;
		}
	} catch (err) {
		await materialize
			.rollback()
			.catch(rollbackErr => logger.warn("Materialization rollback failed", { error: rollbackErr }));
		throw err;
	}
}

/**
 * Reset a false-active revision to draft when the pointer never published it.
 * Never-active stays invalid for rollback.
 */
export async function resetFalseActiveRevisionToDraft(
	kind: RevisionKind,
	name: string,
	revId: string,
	agentDir?: string,
): Promise<boolean> {
	const cleanRev = assertRevisionId(revId);
	const pointer = await readActivePointer(kind, name, agentDir);
	if (pointer.active === cleanRev) return false;
	const revision = await readRevision(kind, name, cleanRev, agentDir).catch(() => undefined);
	if (!revision || revision.state !== "active") return false;
	revision.state = "draft";
	await Bun.write(revisionFile(kind, name, cleanRev, agentDir), JSON.stringify(revision, null, "\t"));
	return true;
}

export async function listRevisions(kind: RevisionKind, name: string, agentDir?: string): Promise<ArtifactRevision[]> {
	const files = await listRevisionFiles(kind, name, agentDir);
	const revisions: ArtifactRevision[] = [];
	for (const file of files) {
		const revId = file.replace(/\.json$/, "");
		const revision = await readRevision(kind, name, revId, agentDir);
		if (revision) revisions.push(revision);
	}
	return revisions.sort((a, b) => (a.createdAt < b.createdAt ? -1 : 1));
}

/**
 * Locked prune body: assumes the caller already holds the shared artifact
 * transaction for (agentDir,kind,name). Lists history + pointer, retains
 * newest-20 + active + live-pinned, then unlinks the rest while still
 * holding ownership (no stale snapshots). `isPinnedLive` is consulted
 * synchronously immediately before each unlink so a pin added during the
 * loop still protects a not-yet-unlinked revision.
 */
export async function pruneRevisionsLocked(
	kind: RevisionKind,
	name: string,
	pinned: ReadonlySet<string> = new Set(),
	agentDir?: string,
	isPinnedLive?: (revId: string) => boolean,
): Promise<ArtifactRevision[]> {
	const revisions = await listRevisions(kind, name, agentDir);
	const pointer = await readActivePointer(kind, name, agentDir);
	const keep = new Set<string>(pinned);
	if (pointer.active) keep.add(pointer.active);
	const ordered = [...revisions].sort((a, b) => (a.createdAt > b.createdAt ? -1 : 1));
	for (const revision of ordered.slice(0, MAX_REVISIONS_PER_ARTIFACT)) keep.add(revision.id);
	const { unlink } = await import("node:fs/promises");
	for (const revision of revisions) {
		if (keep.has(revision.id)) continue;
		if (isPinnedLive?.(revision.id)) continue;
		try {
			await unlink(revisionFile(kind, name, revision.id, agentDir));
		} catch (err) {
			logger.warn("Failed to prune revision", { kind, name, revId: revision.id, error: err });
		}
	}
	return listRevisions(kind, name, agentDir);
}

/**
 * Retain the latest twenty revisions; older revisions survive only while
 * pinned by live runs. Returns the surviving revisions.
 *
 * Serialized through the shared artifact transaction keyed by (resolved
 * agentDir,kind,name) and holding ownership through unlink completion.
 * Per-artifact wrappers snapshot their pin sets INSIDE the same transaction
 * and delegate to {@link pruneRevisionsLocked} so the snapshot is never stale.
 */
export async function pruneRevisions(
	kind: RevisionKind,
	name: string,
	pinned: ReadonlySet<string> = new Set(),
	agentDir?: string,
): Promise<ArtifactRevision[]> {
	return withRevisionTransaction(kind, name, async () => pruneRevisionsLocked(kind, name, pinned, agentDir), agentDir);
}
