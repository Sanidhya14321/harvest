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
import { getAgentDir, isEnoent, logger } from "@harvest/pi-utils";

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
export function withRevisionTransaction<T>(kind: RevisionKind, name: string, fn: () => Promise<T>): Promise<T> {
	const key = `${kind}:${assertArtifactName(name)}`;
	const prev = revisionTransactionChains.get(key) ?? Promise.resolve();
	const run = prev.then(fn, fn);
	const guarded = run.catch(() => {});
	revisionTransactionChains.set(key, guarded);
	void guarded.finally(() => {
		if (revisionTransactionChains.get(key) === guarded) revisionTransactionChains.delete(key);
	});
	return run;
}

export type RevisionState = "draft" | "active";
export type RevisionKind = "skill" | "preset";

export interface EvaluationRecord {
	id: string;
	task: string;
	expectedOutcome: string;
	passed: boolean;
	summary: string;
	runId?: string;
	sessionId?: string;
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
	revision.state = "active";
	await Bun.write(revisionFile(kind, name, cleanRev, opts?.agentDir), JSON.stringify(revision, null, "\t"));
	const pointer: RevisionPointer = { active: cleanRev, updatedAt: new Date().toISOString() };
	await writePointerAtomic(kind, name, pointer, opts?.agentDir);
	return pointer;
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
		revision.state = "active";
		await Bun.write(revisionFile(kind, name, cleanRev, opts?.agentDir), JSON.stringify(revision, null, "\t"));
		const pointer: RevisionPointer = { active: cleanRev, updatedAt: new Date().toISOString() };
		await writePointerAtomic(kind, name, pointer, opts?.agentDir);
		return pointer;
	} catch (err) {
		await materialize
			.rollback()
			.catch(rollbackErr => logger.warn("Materialization rollback failed", { error: rollbackErr }));
		throw err;
	}
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
 * Retain the latest twenty revisions; older revisions survive only while
 * pinned by live runs. Returns the surviving revisions.
 */
export async function pruneRevisions(
	kind: RevisionKind,
	name: string,
	pinned: ReadonlySet<string> = new Set(),
	agentDir?: string,
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
		try {
			await unlink(revisionFile(kind, name, revision.id, agentDir));
		} catch (err) {
			logger.warn("Failed to prune revision", { kind, name, revId: revision.id, error: err });
		}
	}
	return listRevisions(kind, name, agentDir);
}
