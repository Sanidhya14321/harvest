/**
 * Managed-skills primitives for the experimental auto-learn feature.
 *
 * Managed skills are auto-generated/enhanced `SKILL.md` files kept in an
 * isolated directory (`~/.omp/agent/managed-skills`) separate from
 * user-authored skills (`~/.omp/agent/skills`). They are discovered and
 * surfaced like normal skills, but every write here is confined to
 * `getManagedSkillsDir()` — auto-management can never touch authored skills.
 */
import type { Stats } from "node:fs";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import { getAgentDir, isEnoent } from "@harvest/pi-utils";
import { YAML } from "bun";
import { SecuritySandbox } from "../core/harvest/security";
import type { ToolSession } from "../tools/index";

/** Provider id stamped on discovered managed skills (distinguishes them from authored). */
export const MANAGED_SKILLS_PROVIDER_ID = "omp-managed";

/** Hard cap on a managed SKILL.md body to keep generated skills bounded. */
export const MAX_MANAGED_SKILL_BYTES = 64_000;

const SKILL_NAME_PATTERN = /^[a-z0-9][a-z0-9-]{0,63}$/;

/**
 * Minted revision-id grammar. Production ids are `rev-<base36>-<base36>`
 * (see revisions.ts `newRevisionId`); accept one or more hyphen-joined
 * base36 segments after the prefix and reject everything else (separators,
 * absolute paths, traversal) before the id ever reaches the filesystem.
 */
export const REVISION_ID_PATTERN = /^rev-[a-z0-9]+(?:-[a-z0-9]+)*$/;

/** Validate a revision id against the minted grammar; throws on traversal/separators. */
export function validateSkillRevisionId(revId: string): string {
	const trimmed = revId.trim();
	if (
		!REVISION_ID_PATTERN.test(trimmed) ||
		path.isAbsolute(trimmed) ||
		trimmed.includes("/") ||
		trimmed.includes("\\") ||
		trimmed.includes("..")
	) {
		throw new Error(
			`Invalid revision id "${revId}". Expected grammar rev-[a-z0-9]+(-[a-z0-9]+)* (no separators, absolute paths, or traversal).`,
		);
	}
	return trimmed;
}

/** Validate an expectedActive pointer value (revision id or explicit null). */
export function validateSkillExpectedActive(value: string | null | undefined): string | null | undefined {
	if (value === undefined || value === null) return value;
	const trimmed = value.trim();
	// Traversal/separators/absolute are always refused as invalid (security).
	if (path.isAbsolute(trimmed) || trimmed.includes("/") || trimmed.includes("\\") || trimmed.includes("..")) {
		throw new Error(
			`Invalid revision id "${value}". Expected grammar rev-[a-z0-9]+(-[a-z0-9]+)* (no separators, absolute paths, or traversal).`,
		);
	}
	// Non-grammar but otherwise safe values (e.g. legacy "stale" fixtures) flow
	// through to the pointer comparison, which rejects them as Revision
	// conflict with zero partial effects (checked INSIDE the transaction).
	return trimmed;
}

/** Resolve the isolated managed-skills directory (`~/.omp/agent/managed-skills`). */
export function getManagedSkillsDir(agentDir: string = getAgentDir()): string {
	return path.join(agentDir, "managed-skills");
}

/**
 * Validate + normalize a managed-skill name. Throws on anything outside the
 * strict allowlist so a bad name can never escape `getManagedSkillsDir()`
 * (blocks `..`, slashes, empty, and uppercase).
 */
export function sanitizeSkillName(raw: string): string {
	const name = raw.trim().toLowerCase();
	if (!SKILL_NAME_PATTERN.test(name)) {
		throw new Error(
			`Invalid skill name "${raw}". Use lowercase letters, digits, and hyphens (1-64 chars, starting with a letter or digit).`,
		);
	}
	return name;
}

/**
 * Whether `name` is a safe managed-skill name (the exact post-sanitize shape).
 * Used to validate names read from disk at discovery time — a managed
 * `SKILL.md` whose `frontmatter.name` was not produced by `sanitizeSkillName`
 * (e.g. hand-placed) must not render unescaped into the system prompt.
 */
export function isValidManagedSkillName(name: string): boolean {
	return SKILL_NAME_PATTERN.test(name);
}

/**
 * Neutralize a machine-generated managed-skill description so it cannot break
 * out of the system prompt's `<skills>` listing. Managed descriptions are
 * generated from prior task content and persist across sessions, so this is a
 * trust boundary: strip control/format chars, angle brackets (`<system-directive>`
 * / `</skills>`), and Markdown fence delimiters (backticks, `~~~`), then collapse
 * to a single line. Applied on BOTH write and read so existing files are safe too.
 */
export function sanitizeManagedDescription(raw: string): string {
	return raw
		.replace(/[\p{Cc}\p{Cf}]/gu, " ")
		.replace(/[<>`]/g, "")
		.replace(/~{2,}/g, "~")
		.replace(/\s+/g, " ")
		.trim();
}

/**
 * Serialize the minimal `name`/`description` frontmatter block via the repo's
 * YAML helper (round-trips through `parseFrontmatter`).
 */
export function toSkillFrontmatter(name: string, description: string): string {
	const frontmatter = YAML.stringify(
		{ name, description: sanitizeManagedDescription(description) },
		null,
		2,
	).trimEnd();
	return `---\n${frontmatter}\n---\n`;
}

export interface WriteManagedSkillInput {
	action: "create" | "update";
	name: string;
	/** Required for create; optional for update (omitted fields merge from current). */
	description?: string;
	/** Required for create; optional for update (omitted fields merge from current). */
	body?: string;
	/**
	 * Expected current active revision; rejects on conflict before any file
	 * write. Undefined skips the check (legacy immediate path).
	 */
	expectedActive?: string | null;
}

/**
 * Per-name mutation entry point. The shared revision-service boundary
 * serializes same-artifact callers; per-revision evaluation appends have
 * their own chains below so concurrent evaluations all survive.
 */

/**
 * Single per-artifact serialized transaction for ALL skill mutation verbs
 * (create/update/delete/draft/materialize/promote/rollback/evaluate-append).
 * Delegates to the shared revision-service boundary; `kind` is accepted for
 * a uniform cross-artifact signature. Expected-revision checks run INSIDE
 * the boundary; null means explicit no-active.
 */
export function withArtifactTransaction<T>(kind: string, name: string, fn: () => Promise<T>): Promise<T> {
	if (kind !== "skill") throw new Error(`withArtifactTransaction(skill): unsupported kind "${kind}"`);
	return withRevisionTransaction("skill", sanitizeSkillName(name), fn);
}

/** Per-revision evaluation-append chains so concurrent eval appends all survive. */
const skillEvalChains = new Map<string, Promise<unknown>>();
function serializeSkillEvalAppend<T>(name: string, revId: string, op: () => Promise<T>): Promise<T> {
	const key = `${sanitizeSkillName(name)}:${validateSkillRevisionId(revId)}`;
	const prev = skillEvalChains.get(key) ?? Promise.resolve();
	const run = prev.then(op, op);
	const guarded = run.catch(() => {});
	skillEvalChains.set(key, guarded);
	void guarded.finally(() => {
		if (skillEvalChains.get(key) === guarded) skillEvalChains.delete(key);
	});
	return run;
}

/**
 * Reject when the managed-skills root itself is a symlink. lstat on a child
 * follows intermediate components, so a symlinked root would let an otherwise
 * valid name write/delete outside the isolated directory (e.g. onto authored
 * skills). Checked before composing any child path.
 */
async function assertManagedRootSafe(agentDir?: string): Promise<string> {
	const root = agentDir ? path.join(agentDir, "managed-skills") : getManagedSkillsDir();
	const rootStat = await fs.lstat(root).catch(err => {
		if (isEnoent(err)) return null;
		throw err;
	});
	if (rootStat?.isSymbolicLink()) {
		throw new Error("The managed-skills root is a symlink; refusing to operate outside the managed directory.");
	}
	return root;
}

/**
 * Central jail guard for managed-skill targets: lexical + realpath
 * containment inside the managed root (ancestor + leaf link boundaries) via
 * SecuritySandbox, then re-validated after open/write via recheckJailed to
 * close the check→use window. Extends the central helper; the lstat/nlink/
 * O_NOFOLLOW checks below stay as the leaf hardening.
 */
function assertSkillPathJailed(root: string, target: string, label: string): string {
	const sandbox = new SecuritySandbox(root);
	const check = sandbox.assertPathJailed(target);
	if (!check.jailed) throw new Error(`${label}: ${check.error}`);
	return check.resolvedPath;
}

function recheckSkillPathJailed(root: string, resolved: string, label: string): string {
	const sandbox = new SecuritySandbox(root);
	const check = sandbox.recheckJailed(resolved);
	if (!check.jailed) throw new Error(`${label}: ${check.error}`);
	return check.resolvedPath;
}

function assertManagedSkillFileSafeForUpdate(name: string, fileStat: Stats): void {
	if (!fileStat.isFile()) {
		throw new Error(`Managed skill "${name}" SKILL.md is not a regular file; refusing to overwrite it.`);
	}
	if (fileStat.nlink > 1) {
		throw new Error(
			`Managed skill "${name}" SKILL.md has ${fileStat.nlink} hard links; refusing to overwrite a file that may be user-authored elsewhere.`,
		);
	}
}

/** Verify a loaded revision really belongs to this artifact (kind/name match). */
function assertSkillRevisionIdentity(safe: string, revision: { kind: string; name: string; id: string }): void {
	if (revision.kind !== "skill" || revision.name !== safe) {
		throw new Error(
			`Revision identity mismatch for skill "${safe}": got kind "${revision.kind}" name "${revision.name}" (${revision.id}).`,
		);
	}
}

/** Prevalidate revision content before it ever reaches the live file. */
function prevalidateSkillRevisionContent(safe: string, content: string): void {
	if (Buffer.byteLength(content, "utf8") > MAX_MANAGED_SKILL_BYTES) {
		throw new Error(`Managed skill "${safe}" revision is over the ${MAX_MANAGED_SKILL_BYTES} byte limit.`);
	}
	if (!content.includes("---")) throw new Error(`Revision for skill "${safe}" is missing frontmatter.`);
}

/** Transaction marker path for interrupted-promote recovery. */
function skillTxnFile(root: string, safe: string): string {
	return path.join(root, `${safe}.txn.json`);
}

async function writeSkillTxn(root: string, safe: string, revId: string, previousActive: string | null): Promise<void> {
	await Bun.write(
		skillTxnFile(root, safe),
		JSON.stringify({ kind: "skill", name: safe, revId, previousActive, startedAt: new Date().toISOString() }),
	);
}

async function clearSkillTxn(root: string, safe: string): Promise<void> {
	try {
		await fs.rm(skillTxnFile(root, safe));
	} catch (err) {
		if (!isEnoent(err)) throw err;
	}
}

/**
 * Recover an interrupted promote/rollback before discovery exposes artifacts:
 * when a txn marker exists but the pointer never published, the file is ahead
 * of the pointer — roll the file back to the pointer's content (old active
 * stays published; pointer+content consistent) and drop the marker.
 */
export async function recoverInterruptedSkillTransaction(name: string, opts?: { agentDir?: string }): Promise<void> {
	const safe = sanitizeSkillName(name);
	const root = opts?.agentDir ? path.join(opts.agentDir, "managed-skills") : getManagedSkillsDir();
	const txnPath = skillTxnFile(root, safe);
	let txn: { revId: string; previousActive: string | null } | undefined;
	try {
		txn = (await Bun.file(txnPath).json()) as { revId: string; previousActive: string | null };
	} catch (err) {
		if (isEnoent(err)) return;
		throw err;
	}
	if (!txn) return;
	const pointer = await readActivePointer("skill", safe, opts?.agentDir);
	const filePath = path.join(root, safe, "SKILL.md");
	const fileContent = await Bun.file(filePath)
		.text()
		.catch(err => {
			if (isEnoent(err)) return undefined;
			throw err;
		});
	const activeRev = pointer.active
		? await readRevision("skill", safe, pointer.active, opts?.agentDir).catch(() => undefined)
		: undefined;
	const expected = activeRev?.content;
	if (fileContent !== expected) {
		if (expected === undefined) {
			try {
				await fs.rm(filePath);
			} catch (err) {
				if (!isEnoent(err)) throw err;
			}
		} else {
			await fs.mkdir(path.dirname(filePath), { recursive: true });
			const tmp = `${filePath}.recover.tmp`;
			await Bun.write(tmp, expected);
			await fs.rename(tmp, filePath);
		}
	}
	await clearSkillTxn(root, safe);
}

/** Staged atomic replace: write to tmp in the same dir, then rename over the target. */
async function stageAndReplaceSkillFile(file: string, content: string): Promise<void> {
	const tmp = `${file}.tmp`;
	await Bun.write(tmp, content);
	await fs.rename(tmp, file);
}

/** Serialize a managed SKILL.md file body from its validated parts. */
export function buildSkillFileContent(name: string, description: string, body: string): string {
	return `${toSkillFrontmatter(name, description)}\n${body.trim()}\n`;
}

/** Create or update a managed `SKILL.md`. Returns the resolved file path. */
export async function writeManagedSkill(input: WriteManagedSkillInput): Promise<{ path: string }> {
	const name = sanitizeSkillName(input.name);
	const expectedActive = validateSkillExpectedActive(input.expectedActive);
	// R3 merge: update with omitted description/body preserves existing values
	// against the current file/revision; explicit values are validated replacements.
	let descriptionRaw = input.description;
	let bodyRaw = input.body;
	if (input.action === "update" && (descriptionRaw === undefined || bodyRaw === undefined)) {
		const current = await readActiveRevision("skill", name).catch(() => undefined);
		const currentFile = current ? undefined : await readManagedSkillFile(name).catch(() => undefined);
		const currentContent = current?.content ?? currentFile?.content;
		if (currentContent) {
			try {
				const { parseFrontmatter } = await import("@harvest/pi-utils");
				const parsed = parseFrontmatter(currentContent, { location: `managed:${name}`, level: "warn" });
				const fm = parsed.frontmatter as { description?: unknown };
				const existingDesc = typeof fm.description === "string" ? fm.description : "";
				if (descriptionRaw === undefined) descriptionRaw = existingDesc;
				if (bodyRaw === undefined) bodyRaw = parsed.body;
			} catch {
				// Fall through to required-field errors below.
			}
		}
	}
	if (descriptionRaw === undefined || bodyRaw === undefined) {
		throw new Error(
			`"${input.action}" requires both "description" and "body" (or an existing skill to merge against).`,
		);
	}
	const description = sanitizeManagedDescription(descriptionRaw);
	const body = bodyRaw.trim();
	// Reject empty content: an all-whitespace/control description sanitizes to ""
	// and the `requireDescription` discovery scan then silently drops the skill,
	// so the tool would report success for a skill that never appears.
	if (!description) {
		throw new Error(`Managed skill "${name}" needs a non-empty description.`);
	}
	if (!body) {
		throw new Error(`Managed skill "${name}" needs a non-empty body.`);
	}
	const content = buildSkillFileContent(name, description, body);
	// Cap the UTF-8 byte size of the FINAL file (body + description + frontmatter),
	// not the UTF-16 code-unit length of the body alone.
	const bytes = Buffer.byteLength(content, "utf8");
	if (bytes > MAX_MANAGED_SKILL_BYTES) {
		throw new Error(
			`Managed skill is ${bytes} bytes; the limit is ${MAX_MANAGED_SKILL_BYTES}. Trim the body or description.`,
		);
	}
	return withArtifactTransaction("skill", name, async () => {
		const root = await assertManagedRootSafe();
		await recoverInterruptedSkillTransaction(name).catch(() => {});
		// Route through the revision store first: seed history from any
		// pre-revision file, then enforce the expected-revision conflict check
		// BEFORE the file write so a stale caller never overwrites newer work.
		// Compared INSIDE the boundary; null = explicit no-active.
		await ensureSkillHistorySeeded(name);
		if (expectedActive !== undefined) {
			const pointer = await readActivePointer("skill", name);
			const want = expectedActive ?? null;
			if (pointer.active !== want) {
				throw new Error(
					`Revision conflict for skill "${name}": expected active ${want ?? "none"}, found ${pointer.active ?? "none"}. Re-list revisions and retry.`,
				);
			}
		}
		const dir = path.join(root, name);
		const file = path.join(dir, "SKILL.md");
		assertSkillPathJailed(root, file, `Managed skill "${name}"`);
		// Reject a symlinked skill directory: an intermediate symlink would let the
		// write escape the isolated managed root. lstat does not follow the final
		// component, so a symlinked `dir` is caught here.
		const dirStat = await fs.lstat(dir).catch(err => {
			if (isEnoent(err)) return null;
			throw err;
		});
		if (dirStat?.isSymbolicLink()) {
			throw new Error(
				`Managed skill "${name}" resolves through a symlink; refusing to write outside the managed directory.`,
			);
		}
		if (input.action === "create") {
			await fs.mkdir(dir, { recursive: true });
			// O_CREAT|O_EXCL ("wx"): atomic create that fails if the file already
			// exists (closing the check-then-write race) and refuses a symlinked SKILL.md.
			try {
				await fs.writeFile(file, content, { flag: "wx" });
			} catch (err) {
				if ((err as { code?: string }).code === "EEXIST") {
					throw new Error(`Managed skill "${name}" already exists. Use action "update" to change it.`);
				}
				throw err;
			}
			recheckSkillPathJailed(root, file, `Managed skill "${name}"`);
			try {
				await recordSkillFileWrite(name, content, description);
			} catch (err) {
				try {
					await fs.rm(file);
				} catch {
					// Best-effort rollback of a file whose pointer never published.
				}
				throw err;
			}
			void pruneSkillRevisions(name).catch(() => {});
			return { path: file };
		}
		// update: the file must already exist, be a plain managed file, and must
		// not share an inode with a user-authored file via hard link. Stage to a
		// tmp file in the same dir + atomic rename (never truncate-in-place), so
		// a materialization failure leaves the old active published.
		const fileStat = await fs.lstat(file).catch(err => {
			if (isEnoent(err)) return null;
			throw err;
		});
		if (fileStat === null) {
			throw new Error(`Managed skill "${name}" does not exist. Use action "create" to add it.`);
		}
		if (fileStat.isSymbolicLink()) {
			throw new Error(`Managed skill "${name}" SKILL.md is a symlink; refusing to overwrite it.`);
		}
		assertManagedSkillFileSafeForUpdate(name, fileStat);
		const oldContent = await Bun.file(file)
			.text()
			.catch(() => undefined);
		await stageAndReplaceSkillFile(file, content);
		recheckSkillPathJailed(root, file, `Managed skill "${name}"`);
		try {
			await recordSkillFileWrite(name, content, description);
		} catch (err) {
			try {
				if (oldContent !== undefined) await stageAndReplaceSkillFile(file, oldContent);
				else await fs.rm(file);
			} catch {
				// Best-effort rollback; surface the original pointer failure.
			}
			throw err;
		}
		void pruneSkillRevisions(name).catch(() => {});
		return { path: file };
	});
}

/**
 * Record an immediate create/update file write in revision history: mint a
 * draft for the new content and activate it. The write itself is the manual
 * activation, so unevaluated content is disclosed by the caller surfacing the
 * status (the gated draft/evaluate/promote path stays available for
 * evaluation-first flows). Runs inside the per-name mutation chain.
 */
async function recordSkillFileWrite(name: string, content: string, description: string): Promise<void> {
	const draft = await createDraftRevision({
		kind: "skill",
		name,
		content,
		description,
		provenance: { actor: "model" },
	});
	await promoteRevision("skill", name, draft.id, { discloseUnevaluated: true });
}

/** Delete a managed skill directory. Throws when it does not exist. */
export async function deleteManagedSkill(name: string): Promise<void> {
	const safe = sanitizeSkillName(name);
	await withArtifactTransaction("skill", safe, async () => {
		const root = await assertManagedRootSafe();
		const dir = path.join(root, safe);
		assertSkillPathJailed(root, dir, `Managed skill "${safe}"`);
		// Refuse to follow a symlinked skill directory (rm would delete the target).
		const dirStat = await fs.lstat(dir).catch(err => {
			if (isEnoent(err)) return null;
			throw err;
		});
		if (dirStat === null) throw new Error(`Managed skill "${safe}" does not exist.`);
		if (dirStat?.isSymbolicLink()) {
			throw new Error(`Managed skill "${safe}" is a symlink; refusing to delete outside the managed directory.`);
		}
		if (!dirStat.isDirectory()) {
			throw new Error(`Managed skill "${safe}" is not a directory; refusing to delete.`);
		}
		try {
			await fs.rm(dir, { recursive: true });
		} catch (err) {
			if (isEnoent(err)) {
				throw new Error(`Managed skill "${safe}" does not exist.`);
			}
			throw err;
		}
		// History retained for audit; prune keeps the store bounded.
		void pruneSkillRevisions(safe).catch(() => {});
	});
}

import {
	createDraftRevision,
	isEvaluatedPassing,
	listRevisions,
	promoteRevision,
	pruneRevisions,
	readActivePointer,
	readActiveRevision,
	readRevision,
	recordEvaluation,
	rollbackRevision,
	withRevisionTransaction,
	type ArtifactRevision,
} from "./revisions";

// ═══════════════════════════════════════════════════════════════════════════
// Revision bridging (workstream B)
//
// Routes managed-skill mutations through the shared revision store
// (`./revisions.ts`, owner-owned — imported, never edited). The materialized
// `SKILL.md` file under `getManagedSkillsDir()` is the *active* revision
// rendered for discovery; active revisions are immutable — every mutation
// mints a new draft revision and moves the active pointer (promote), never
// overwriting a revision file in place.
// ═══════════════════════════════════════════════════════════════════════════

/** Provenance recorded on revisions minted through managed-skill flows. */
export interface SkillRevisionProvenance {
	sessionId?: string;
	runId?: string;
	actor?: string;
}

export interface SkillDraftInput {
	name: string;
	/** Optional for merge flows; omitted fields preserve current values. */
	description?: string;
	/** Optional for merge flows; omitted fields preserve current values. */
	body?: string;
	/** Expected current active revision; rejects on conflict. */
	expectedActive?: string | null;
	provenance?: SkillRevisionProvenance;
	agentDir?: string;
}

/** Read the currently materialized SKILL.md file, if any. */
export async function readManagedSkillFile(
	name: string,
	opts?: { agentDir?: string },
): Promise<{ content: string } | undefined> {
	const safe = sanitizeSkillName(name);
	await recoverInterruptedSkillTransaction(safe, opts).catch(() => {});
	try {
		const root = opts?.agentDir ? path.join(opts.agentDir, "managed-skills") : getManagedSkillsDir();
		const file = path.join(root, safe, "SKILL.md");
		assertSkillPathJailed(root, file, `Managed skill "${safe}"`);
		const content = await Bun.file(file).text();
		return { content };
	} catch (err) {
		if (isEnoent(err)) return undefined;
		throw err;
	}
}

/**
 * Seed revision history from the materialized file the first time a managed
 * skill with no history is mutated. Legacy files written before revisions
 * existed enter history as the active baseline (explicitly disclosed as
 * unevaluated) so no content is silently dropped.
 */
export async function ensureSkillHistorySeeded(name: string, opts?: { agentDir?: string }): Promise<void> {
	const safe = sanitizeSkillName(name);
	const existing = await listRevisions("skill", safe, opts?.agentDir);
	if (existing.length > 0) return;
	const file = await readManagedSkillFile(safe);
	if (!file) return;
	const seeded = await createDraftRevision({
		kind: "skill",
		name: safe,
		content: file.content,
		description: "seeded from pre-revision managed SKILL.md",
		parent: null,
		provenance: { actor: "seed" },
		agentDir: opts?.agentDir,
	});
	await promoteRevision("skill", safe, seeded.id, { discloseUnevaluated: true, agentDir: opts?.agentDir });
}

/**
 * Mint a draft revision for a managed skill without touching the
 * materialized file. The caller decides when (and whether) to evaluate and
 * promote it. Expected-revision conflicts reject INSIDE the transaction.
 * Omitted description/body merge against the current active revision/file.
 */
export async function createSkillDraft(input: SkillDraftInput): Promise<ArtifactRevision> {
	const name = sanitizeSkillName(input.name);
	const expectedActive = validateSkillExpectedActive(input.expectedActive);
	return withArtifactTransaction("skill", name, async () => {
		await ensureSkillHistorySeeded(name, { agentDir: input.agentDir });
		let descRaw = input.description;
		let bodyRaw = input.body;
		if (descRaw === undefined || bodyRaw === undefined) {
			const current = await readActiveRevision("skill", name, input.agentDir).catch(() => undefined);
			if (current) {
				try {
					const { parseFrontmatter } = await import("@harvest/pi-utils");
					const parsed = parseFrontmatter(current.content, { location: `managed:${name}`, level: "warn" });
					const fm = parsed.frontmatter as { description?: unknown };
					if (descRaw === undefined && typeof fm.description === "string") descRaw = fm.description;
					if (bodyRaw === undefined) bodyRaw = parsed.body;
				} catch {
					// Fall through to required-field errors.
				}
			}
		}
		if (descRaw === undefined || bodyRaw === undefined) {
			throw new Error(
				`Draft for skill "${name}" requires both "description" and "body" (or an existing revision to merge against).`,
			);
		}
		const description = sanitizeManagedDescription(descRaw);
		if (!description) throw new Error(`Managed skill "${name}" needs a non-empty description.`);
		if (!bodyRaw.trim()) throw new Error(`Managed skill "${name}" needs a non-empty body.`);
		const content = buildSkillFileContent(name, description, bodyRaw);
		if (Buffer.byteLength(content, "utf8") > MAX_MANAGED_SKILL_BYTES) {
			throw new Error(
				`Managed skill is over the ${MAX_MANAGED_SKILL_BYTES} byte limit. Trim the body or description.`,
			);
		}
		return createDraftRevision({
			kind: "skill",
			name,
			content,
			description,
			expectedActive,
			provenance: {
				sessionId: input.provenance?.sessionId,
				runId: input.provenance?.runId,
				actor: input.provenance?.actor ?? "model",
			},
			agentDir: input.agentDir,
		});
	});
}

/** Internal materialize without serialization (caller holds the transaction). */
async function materializeSkillRevisionInner(
	safe: string,
	revId: string,
	root: string,
	agentDir?: string,
): Promise<{ path: string }> {
	const revision = await readRevision("skill", safe, revId, agentDir);
	if (!revision) throw new Error(`Revision ${revId} for skill "${safe}" not found`);
	assertSkillRevisionIdentity(safe, revision);
	validateSkillRevisionId(revision.id);
	prevalidateSkillRevisionContent(safe, revision.content);
	const dir = path.join(root, safe);
	const file = path.join(dir, "SKILL.md");
	assertSkillPathJailed(root, file, `Managed skill "${safe}"`);
	const dirStat = await fs.lstat(dir).catch(err => {
		if (isEnoent(err)) return null;
		throw err;
	});
	if (dirStat?.isSymbolicLink()) {
		throw new Error(
			`Managed skill "${safe}" resolves through a symlink; refusing to write outside the managed directory.`,
		);
	}
	await fs.mkdir(dir, { recursive: true });
	const fileStat = await fs.lstat(file).catch(err => {
		if (isEnoent(err)) return null;
		throw err;
	});
	if (fileStat === null) {
		try {
			await fs.writeFile(file, revision.content, { flag: "wx" });
		} catch (err) {
			if ((err as { code?: string }).code !== "EEXIST") throw err;
			throw new Error(`Managed skill "${safe}" appeared mid-materialize; retry the promotion.`);
		}
		recheckSkillPathJailed(root, file, `Managed skill "${safe}"`);
		return { path: file };
	}
	if (fileStat.isSymbolicLink()) {
		throw new Error(`Managed skill "${safe}" SKILL.md is a symlink; refusing to overwrite it.`);
	}
	assertManagedSkillFileSafeForUpdate(safe, fileStat);
	await stageAndReplaceSkillFile(file, revision.content);
	recheckSkillPathJailed(root, file, `Managed skill "${safe}"`);
	return { path: file };
}

/**
 * Materialize a revision as the live SKILL.md file with the same
 * symlink/hard-link guards as the direct write path (never follows a link
 * out of the isolated managed root). Serialized per skill name.
 */
export async function materializeSkillRevision(
	name: string,
	revId: string,
	opts?: { agentDir?: string },
): Promise<{ path: string }> {
	const safe = sanitizeSkillName(name);
	const cleanRev = validateSkillRevisionId(revId);
	return withArtifactTransaction("skill", safe, async () => {
		const root = await assertManagedRootSafe(opts?.agentDir);
		await recoverInterruptedSkillTransaction(safe, opts).catch(() => {});
		return materializeSkillRevisionInner(safe, cleanRev, root, opts?.agentDir);
	});
}

export interface PromoteSkillOptions {
	/** Required to activate unevaluated content; surfaces the status to the caller. */
	discloseUnevaluated?: boolean;
	agentDir?: string;
}

export interface PromoteSkillResult {
	path: string;
	revId: string;
	/** True when the activated revision was explicitly disclosed as unevaluated. */
	disclosedUnevaluated: boolean;
}

/**
 * Promote a draft revision to active and materialize it. Failed
 * evaluations block promotion (never auto-promote a failing revision);
 * unevaluated content activates only with explicit `discloseUnevaluated`.
 * Order: prevalidate + stage/atomic materialize FIRST, publish the pointer
 * ONLY after materialization succeeds; on pointer failure roll the file back
 * to the previous active content. Interrupted transactions recover via the
 * txn marker before discovery exposes artifacts.
 */
export async function promoteSkillRevision(
	name: string,
	revId: string,
	opts?: PromoteSkillOptions,
): Promise<PromoteSkillResult> {
	const safe = sanitizeSkillName(name);
	const cleanRev = validateSkillRevisionId(revId);
	return withArtifactTransaction("skill", safe, async () => {
		const root = await assertManagedRootSafe(opts?.agentDir);
		await recoverInterruptedSkillTransaction(safe, { agentDir: opts?.agentDir }).catch(() => {});
		const revision = await readRevision("skill", safe, cleanRev, opts?.agentDir);
		if (!revision) throw new Error(`Revision ${cleanRev} for skill "${safe}" not found`);
		assertSkillRevisionIdentity(safe, revision);
		prevalidateSkillRevisionContent(safe, revision.content);
		if (revision.evaluations.length > 0 && !isEvaluatedPassing(revision)) {
			throw new Error(
				`Revision ${cleanRev} for skill "${safe}" has failing evaluations; it cannot be promoted. Fix the content and evaluate a new draft.`,
			);
		}
		const unevaluated = revision.evaluations.length === 0;
		const before = await readActivePointer("skill", safe, opts?.agentDir);
		const filePath = path.join(root, safe, "SKILL.md");
		const oldContent = await Bun.file(filePath)
			.text()
			.catch(err => {
				if (isEnoent(err)) return undefined;
				throw err;
			});
		await writeSkillTxn(root, safe, cleanRev, before.active);
		await materializeSkillRevisionInner(safe, cleanRev, root, opts?.agentDir);
		try {
			await promoteRevision("skill", safe, cleanRev, {
				discloseUnevaluated: opts?.discloseUnevaluated,
				agentDir: opts?.agentDir,
			});
		} catch (err) {
			try {
				if (oldContent !== undefined) await stageAndReplaceSkillFile(filePath, oldContent);
				else await fs.rm(filePath);
			} catch {
				// Best-effort rollback; surface the pointer failure.
			}
			await clearSkillTxn(root, safe).catch(() => {});
			throw err;
		}
		await clearSkillTxn(root, safe).catch(() => {});
		void pruneSkillRevisions(safe, { agentDir: opts?.agentDir }).catch(() => {});
		return { path: filePath, revId: cleanRev, disclosedUnevaluated: unevaluated };
	});
}

/** Roll the active pointer back to a prior revision and materialize it. */
export async function rollbackSkillRevision(
	name: string,
	revId: string,
	opts?: { agentDir?: string },
): Promise<{ path: string }> {
	const safe = sanitizeSkillName(name);
	const cleanRev = validateSkillRevisionId(revId);
	return withArtifactTransaction("skill", safe, async () => {
		const root = await assertManagedRootSafe(opts?.agentDir);
		await recoverInterruptedSkillTransaction(safe, opts).catch(() => {});
		const revision = await readRevision("skill", safe, cleanRev, opts?.agentDir);
		if (!revision) throw new Error(`Revision ${cleanRev} for skill "${safe}" not found`);
		assertSkillRevisionIdentity(safe, revision);
		// Rollback only to previously-activated revisions: drafts that never
		// published can never be rolled back to (they were never live).
		if (revision.state !== "active") {
			throw new Error(
				`Cannot rollback skill "${safe}" to revision ${cleanRev}: it was never active (state ${revision.state}). Rollback targets a previously-activated revision.`,
			);
		}
		prevalidateSkillRevisionContent(safe, revision.content);
		const before = await readActivePointer("skill", safe, opts?.agentDir);
		const filePath = path.join(root, safe, "SKILL.md");
		const oldContent = await Bun.file(filePath)
			.text()
			.catch(err => {
				if (isEnoent(err)) return undefined;
				throw err;
			});
		await writeSkillTxn(root, safe, cleanRev, before.active);
		await materializeSkillRevisionInner(safe, cleanRev, root, opts?.agentDir);
		try {
			await rollbackRevision("skill", safe, cleanRev, opts?.agentDir);
		} catch (err) {
			try {
				if (oldContent !== undefined) await stageAndReplaceSkillFile(filePath, oldContent);
				else await fs.rm(filePath);
			} catch {
				// Best-effort rollback.
			}
			await clearSkillTxn(root, safe).catch(() => {});
			throw err;
		}
		await clearSkillTxn(root, safe).catch(() => {});
		void pruneSkillRevisions(safe, { agentDir: opts?.agentDir }).catch(() => {});
		return { path: filePath };
	});
}

/** List revision history with the active pointer for one managed skill. */
export async function listSkillRevisions(
	name: string,
	opts?: { agentDir?: string },
): Promise<{ active: string | null; revisions: ArtifactRevision[] }> {
	const safe = sanitizeSkillName(name);
	await recoverInterruptedSkillTransaction(safe, opts).catch(() => {});
	const [pointer, revisions] = await Promise.all([
		readActivePointer("skill", safe, opts?.agentDir),
		listRevisions("skill", safe, opts?.agentDir),
	]);
	return { active: pointer.active, revisions };
}

/** Read the active revision for one managed skill, if any. */
export async function readActiveSkillRevision(
	name: string,
	opts?: { agentDir?: string },
): Promise<ArtifactRevision | undefined> {
	const safe = sanitizeSkillName(name);
	await recoverInterruptedSkillTransaction(safe, opts).catch(() => {});
	const rev = await readActiveRevision("skill", safe, opts?.agentDir);
	if (rev) assertSkillRevisionIdentity(safe, rev);
	return rev;
}

// ── Pinning ──────────────────────────────────────────────────────────────
// Running agents pin the revision they selected so pruning retains it;
// promotion affects subsequent work only. Owner-aware: each run holds its own
// owner entry, so one run's finish never releases another run's pin.

const skillPinOwners = new Map<string, Set<string>>();
const skillPinKey = (name: string, revId: string): string =>
	`skill:${sanitizeSkillName(name)}:${validateSkillRevisionId(revId)}`;

/** Pin a revision so pruning retains it while a run is live. */
export function pinSkillRevision(name: string, revId: string): void {
	const key = skillPinKey(name, revId);
	let owners = skillPinOwners.get(key);
	if (!owners) {
		owners = new Set();
		skillPinOwners.set(key, owners);
	}
	owners.add("__manual__");
}

/** Owner-aware pin for a live run (runId- or session-scoped). */
export function pinSkillRevisionForRun(name: string, revId: string, runId: string): void {
	if (!runId.trim()) throw new Error("pinSkillRevisionForRun needs a non-empty run id");
	const key = skillPinKey(name, revId);
	let owners = skillPinOwners.get(key);
	if (!owners) {
		owners = new Set();
		skillPinOwners.set(key, owners);
	}
	owners.add(`run:${runId}`);
}

/** Release a run's pin on a revision. */
export function unpinSkillRevision(name: string, revId: string): void {
	const key = skillPinKey(name, revId);
	const owners = skillPinOwners.get(key);
	if (!owners) return;
	owners.delete("__manual__");
	if (owners.size === 0) skillPinOwners.delete(key);
}

/** Release one run's pin; other runs' pins on the same revision survive. */
export function unpinSkillRevisionForRun(name: string, revId: string, runId: string): void {
	const key = skillPinKey(name, revId);
	const owners = skillPinOwners.get(key);
	if (!owners) return;
	owners.delete(`run:${runId}`);
	if (owners.size === 0) skillPinOwners.delete(key);
}

/** Pins for one skill, for prune integration. Test seam: clear all pins. */
export function getSkillPins(name: string): Set<string> {
	const safe = sanitizeSkillName(name);
	const prefix = `skill:${safe}:`;
	const out = new Set<string>();
	for (const key of skillPinOwners.keys()) {
		if (key.startsWith(prefix)) out.add(key.slice(prefix.length));
	}
	return out;
}

/** Test-only: release every skill pin. */
export function clearSkillPinsForTests(): void {
	skillPinOwners.clear();
}

/**
 * Pin every currently-active managed skill revision for a live run, so
 * retention cannot drop one mid-run; promotion still affects subsequent
 * runs only. Returns the pinned pairs for release. Best-effort per skill:
 * one unreadable skill never blocks the run.
 */
export async function pinActiveSkillsForRun(
	runId: string,
	agentDir?: string,
): Promise<Array<{ name: string; revId: string }>> {
	if (!runId.trim()) throw new Error("pinActiveSkillsForRun needs a non-empty run id");
	const root = agentDir ? path.join(agentDir, "managed-skills") : getManagedSkillsDir();
	const entries = await fs.readdir(root).catch(() => []);
	const pinned: Array<{ name: string; revId: string }> = [];
	for (const entry of entries.sort()) {
		if (!isValidManagedSkillName(entry)) continue;
		try {
			const active = await readActiveRevision("skill", entry, agentDir);
			if (!active) continue;
			pinSkillRevisionForRun(entry, active.id, runId);
			pinned.push({ name: entry, revId: active.id });
		} catch {
			// One unreadable skill never blocks the run.
		}
	}
	return pinned;
}

/** Release one run's active-skill pins; other runs' pins survive. */
export function unpinActiveSkillsForRun(pinned: ReadonlyArray<{ name: string; revId: string }>, runId: string): void {
	for (const { name, revId } of pinned) {
		try {
			unpinSkillRevisionForRun(name, revId, runId);
		} catch {
			// Best-effort release.
		}
	}
}

/** Prune history to the latest twenty, retaining pins and the active revision. */
export async function pruneSkillRevisions(name: string, opts?: { agentDir?: string }): Promise<ArtifactRevision[]> {
	return pruneRevisions("skill", sanitizeSkillName(name), getSkillPins(sanitizeSkillName(name)), opts?.agentDir);
}

// ── Evaluation ─────────────────────────────────────────────────────────────
// Evaluations take an explicit task plus the expected observable outcome and
// run through restricted task execution. Missing cases stay unevaluated
// (never passed); a failed evaluation never auto-promotes.

export interface SkillEvalRequest {
	task: string;
	expectedOutcome: string;
	sessionId?: string;
	runId?: string;
	agentDir?: string;
	/** Parent session context for the production executor (real task execution). */
	parent?: ToolSession;
}

export interface SkillEvalOutcome {
	passed: boolean;
	summary: string;
	runId?: string;
	sessionId?: string;
}

export type SkillEvalRunner = (
	input: SkillEvalRequest & { name: string; revId: string; content: string },
) => Promise<SkillEvalOutcome>;

let skillEvalRunner: SkillEvalRunner | undefined;

/** Override the evaluation executor (tests inject deterministic fixtures). */
export function setSkillEvalRunner(runner: SkillEvalRunner | undefined): void {
	skillEvalRunner = runner;
}

/**
 * Guard marking evaluation runs so auto-learning convenience logic cannot
 * widen their permissions or trigger recursive capture. Eval children run at
 * taskDepth > 0 (no controller there), and this flag covers direct runs.
 */
let evalRunDepth = 0;

/** True while inside an evaluation executor. */
export function isSkillEvalRunActive(): boolean {
	return evalRunDepth > 0;
}

function enterSkillEvalRun(): void {
	evalRunDepth++;
}

function exitSkillEvalRun(): void {
	evalRunDepth = Math.max(0, evalRunDepth - 1);
}

/**
 * Evaluate a draft revision against an explicit task + expected observable
 * outcome. Records the result on the revision; never promotes. The runner
 * executes outside the transaction (it is long-lived); the append runs
 * serialized per revision so concurrent eval appends all survive.
 */
export async function evaluateSkillRevision(
	name: string,
	revId: string,
	request: SkillEvalRequest,
): Promise<{ passed: boolean; summary: string }> {
	const safe = sanitizeSkillName(name);
	const cleanRev = validateSkillRevisionId(revId);
	const task = request.task.trim();
	const expectedOutcome = request.expectedOutcome.trim();
	if (!task) throw new Error(`Evaluation of skill "${safe}" needs an explicit task.`);
	if (!expectedOutcome) throw new Error(`Evaluation of skill "${safe}" needs an explicit expected outcome.`);
	const revision = await readRevision("skill", safe, cleanRev, request.agentDir);
	if (!revision) throw new Error(`Revision ${cleanRev} for skill "${safe}" not found`);
	assertSkillRevisionIdentity(safe, revision);
	if (!skillEvalRunner) {
		throw new Error(
			`No evaluation executor is wired for skill "${safe}" (owner patch: task-execution seam). ` +
				`The revision stays unevaluated — unevaluated content is never treated as passed.`,
		);
	}
	enterSkillEvalRun();
	// Pin the evaluated revision for this run so retention cannot drop it
	// mid-evaluation; released in the finally below (other runs' pins survive).
	const evalOwner = request.runId?.trim() || `eval:${safe}:${cleanRev}`;
	try {
		pinSkillRevisionForRun(safe, cleanRev, evalOwner);
	} catch {
		// Pinning is retention hygiene, never evaluation fate.
	}
	try {
		const outcome = await skillEvalRunner({
			...request,
			task,
			expectedOutcome,
			name: safe,
			revId: cleanRev,
			content: revision.content,
		});
		await serializeSkillEvalAppend(safe, cleanRev, async () =>
			recordEvaluation(
				"skill",
				safe,
				cleanRev,
				{
					task,
					expectedOutcome,
					passed: outcome.passed,
					summary: outcome.summary,
					runId: outcome.runId ?? request.runId,
					sessionId: outcome.sessionId ?? request.sessionId,
				},
				request.agentDir,
			),
		);
		return { passed: outcome.passed, summary: outcome.summary };
	} finally {
		try {
			unpinSkillRevisionForRun(safe, cleanRev, evalOwner);
		} catch {
			// Best-effort release.
		}
		exitSkillEvalRun();
	}
}

// ── Auto-improvement bound ─────────────────────────────────────────────────
// At most one candidate plus one evaluation cycle per completed parent turn.
// Eval runs never consume (or trigger) budget: they run under the eval guard.

interface ImprovementBudget {
	turns: number;
	candidates: number;
	evaluations: number;
}

const improvementBudgets = new WeakMap<object, ImprovementBudget>();
const improvementBudgetByKey = new Map<string, ImprovementBudget>();

function budgetFor(key: object | string): ImprovementBudget {
	if (typeof key === "string") {
		let budget = improvementBudgetByKey.get(key);
		if (!budget) {
			budget = { turns: 0, candidates: 0, evaluations: 0 };
			improvementBudgetByKey.set(key, budget);
		}
		return budget;
	}
	let budget = improvementBudgets.get(key);
	if (!budget) {
		budget = { turns: 0, candidates: 0, evaluations: 0 };
		improvementBudgets.set(key, budget);
	}
	return budget;
}

/** Mark a parent turn completed; resets the per-turn candidate/eval allowance. */
export function noteParentTurnCompleted(key: object | string): void {
	const budget = budgetFor(key);
	budget.turns++;
	budget.candidates = 0;
	budget.evaluations = 0;
}

/** Claim the single candidate slot for the current parent turn. False when spent. */
export function tryClaimImprovementCandidate(key: object | string): boolean {
	if (isSkillEvalRunActive()) return false;
	const budget = budgetFor(key);
	if (budget.candidates >= 1) return false;
	budget.candidates++;
	return true;
}

/** Claim the single evaluation slot for the current parent turn. False when spent. */
export function tryClaimImprovementEvaluation(key: object | string): boolean {
	if (isSkillEvalRunActive()) return false;
	const budget = budgetFor(key);
	if (budget.evaluations >= 1) return false;
	budget.evaluations++;
	return true;
}

/** Test-only: drop string-keyed improvement budgets. */
export function clearImprovementBudgetsForTests(): void {
	improvementBudgetByKey.clear();
}

/**
 * Enforce the automatic-improvement bound for the auto-continue loop: one
 * candidate plus one evaluation per parent turn. Scoped to sessions running
 * with `autolearn.autoContinue` (the automatic loop); ordinary sessions —
 * including explicit operator flows — are never gated. No-op without a
 * session identity. Throws when this turn's slot is already spent.
 */
export function requireAutoImprovementBudget(
	session: Pick<ToolSession, "getSessionId" | "settings">,
	slot: "candidate" | "evaluation",
): void {
	if (session.settings.get("autolearn.autoContinue") !== true) return;
	const id = session.getSessionId?.() ?? null;
	if (!id) return;
	const claimed = slot === "candidate" ? tryClaimImprovementCandidate(id) : tryClaimImprovementEvaluation(id);
	if (!claimed) {
		throw new Error(
			`Automatic skill improvement already used its ${slot} slot this turn; further automatic ${slot}s resume next turn.`,
		);
	}
}
