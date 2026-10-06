/**
 * Bundled agent definitions.
 *
 * Agents are embedded at build time via Bun's import with { type: "text" }.
 */
import * as path from "node:path";
import * as fs from "node:fs/promises";
import type { Stats } from "node:fs";
import { Effort } from "@harvest/pi-ai";
import { getAgentDir, isEnoent, parseFrontmatter, prompt } from "@harvest/pi-utils";
import { YAML } from "bun";
import { parseAgentFields } from "../discovery/helpers";
import { SecuritySandbox } from "../core/harvest/security";
// Embed agent markdown files at build time
import agentFrontmatterTemplate from "../prompts/agents/frontmatter.md" with { type: "text" };
import reviewerMd from "../prompts/agents/reviewer.md" with { type: "text" };
import scoutMd from "../prompts/agents/scout.md" with { type: "text" };
import securityReviewerMd from "../prompts/agents/security-reviewer.md" with { type: "text" };
import taskMd from "../prompts/agents/task.md" with { type: "text" };
import { AUTO_THINKING } from "../thinking";
import type { ToolSession } from "../tools/index";

import type { AgentDefinition, AgentSource } from "./types";

interface AgentFrontmatter {
	name: string;
	description: string;
	tools?: string[];
	spawns?: string;
	model?: string | string[];
	thinkingLevel?: string;
	blocking?: boolean;
	prewalk?: boolean | string;
	advisor?: boolean | string;
}

interface EmbeddedAgentDef {
	fileName: string;
	frontmatter?: AgentFrontmatter;
	template: string;
}

function buildAgentContent(def: EmbeddedAgentDef): string {
	const body = prompt.render(def.template);
	if (!def.frontmatter) return body;
	return prompt.render(agentFrontmatterTemplate, { ...def.frontmatter, body });
}

const EMBEDDED_AGENT_DEFS: EmbeddedAgentDef[] = [
	{ fileName: "scout.md", template: scoutMd },
	{ fileName: "reviewer.md", template: reviewerMd },
	{ fileName: "security-reviewer.md", template: securityReviewerMd },
	{
		fileName: "task.md",
		frontmatter: {
			name: "task",
			description: "General-purpose subagent with full capabilities for delegated multi-step tasks",
			spawns: "*",
			model: "@task",
			thinkingLevel: AUTO_THINKING,
			// No `prewalk` frontmatter: the generic task hand-off (strong model
			// plans, then hands off to the smol role) is armed by the
			// `task.prewalk` setting (default off) or per agent via /agents
			// (task.agentPrewalk).
		},
		template: taskMd,
	},
	{
		fileName: "sonic.md",
		frontmatter: {
			name: "sonic",
			description: "Low-reasoning agent for strictly mechanical updates or data collection only",
			model: "@smol",
			thinkingLevel: Effort.Medium,
		},
		template: taskMd,
	},
];

// Computed lazily on first loadBundledAgents() call to avoid eager prompt.render at module load.

export class AgentParsingError extends Error {
	constructor(
		error: Error,
		readonly source?: unknown,
	) {
		super(`Failed to parse agent: ${error.message}`, { cause: error });
		this.name = "AgentParsingError";
	}

	override toString(): string {
		const details: string[] = [this.message];
		if (this.source !== undefined) {
			details.push(`Source: ${JSON.stringify(this.source)}`);
		}
		if (this.cause && typeof this.cause === "object" && "stack" in this.cause && this.cause.stack) {
			details.push(`Stack:\n${this.cause.stack}`);
		} else if (this.stack) {
			details.push(`Stack:\n${this.stack}`);
		}
		return details.join("\n\n");
	}
}

/**
 * Parse an agent from embedded content.
 */
export function parseAgent(
	filePath: string,
	content: string,
	source: AgentSource,
	level: "fatal" | "warn" | "off" = "fatal",
): AgentDefinition {
	const { frontmatter, body } = parseFrontmatter(content, {
		location: filePath,
		level,
	});
	const fields = parseAgentFields(frontmatter);
	if (!fields) {
		throw new AgentParsingError(new Error(`Invalid agent field: ${filePath}\n${content}`), filePath);
	}
	return {
		...fields,
		systemPrompt: body,
		source,
		filePath,
	};
}

/** Cache for bundled agents */
let bundledAgentsCache: AgentDefinition[] | null = null;

/**
 * Load all bundled agents from embedded content.
 * Results are cached after first load.
 */
export function loadBundledAgents(): AgentDefinition[] {
	if (bundledAgentsCache !== null) {
		return bundledAgentsCache;
	}
	bundledAgentsCache = EMBEDDED_AGENT_DEFS.map(def =>
		parseAgent(`embedded:${def.fileName}`, buildAgentContent(def), "bundled"),
	);
	return bundledAgentsCache;
}

/**
 * Get a bundled agent by name.
 */
export function getBundledAgent(name: string): AgentDefinition | undefined {
	return loadBundledAgents().find(a => a.name === name);
}

/**
 * Get all bundled agents as a map keyed by name.
 */
export function getBundledAgentsMap(): Map<string, AgentDefinition> {
	const map = new Map<string, AgentDefinition>();
	for (const agent of loadBundledAgents()) {
		map.set(agent.name, agent);
	}
	return map;
}

/**
 * Clear the bundled agents cache (for testing).
 */
export function clearBundledAgentsCache(): void {
	bundledAgentsCache = null;
}

// Re-export for backward compatibility
export const BUNDLED_AGENTS = loadBundledAgents;

// ═══════════════════════════════════════════════════════════════════════════
// Managed agent presets (workstream B)
//
// Generated presets live in an isolated managed location
// (`<agentDir>/managed-presets`, never authored/bundled dirs) and route
// through the shared revision store (`kind: "preset"`). Authored/bundled
// definitions are never overwritten: a managed preset whose name is claimed
// by authored/bundled/discovered definitions is refused at write time, and
// authored always wins at read time.
// ═══════════════════════════════════════════════════════════════════════════

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
} from "../autolearn/revisions";

/** Resolve the isolated managed-presets directory (`~/.omp/agent/managed-presets`). */
export function getManagedPresetsDir(agentDir: string = getAgentDir()): string {
	return path.join(agentDir, "managed-presets");
}

/**
 * Preset identifier shape (mirrors the /agents hub rule): lowercase
 * kebab-case, 2–6 words. Shared here so the presets tool and the hub
 * validate identically (owner patch: hub imports this instead of its local).
 */
export const PRESET_IDENTIFIER_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+){1,5}$/;

/** Validate + normalize a managed-preset name. Throws outside the allowlist. */
export function sanitizePresetName(raw: string): string {
	const name = raw.trim().toLowerCase();
	if (!PRESET_IDENTIFIER_PATTERN.test(name)) {
		throw new Error(
			`Invalid preset name "${raw}". Use lowercase kebab-case with 2-6 words (e.g. "api-docs-writer").`,
		);
	}
	return name;
}

/** Whether `name` has the exact post-sanitize preset shape. */
export function isValidPresetName(name: string): boolean {
	return PRESET_IDENTIFIER_PATTERN.test(name);
}

export interface ManagedPresetSpec {
	name: string;
	description: string;
	systemPrompt: string;
	tools?: string[];
	spawns?: string;
	model?: string | string[];
}

export interface WriteManagedPresetInput extends Partial<ManagedPresetSpec> {
	name: string;
	action: "create" | "update";
	/** Expected current active revision; rejects on conflict. */
	expectedActive?: string | null;
}

/**
 * Minted revision-id grammar shared with managed skills. Production ids are
 * `rev-<base36>-<base36>`; accept hyphen-joined segments and reject
 * separators/absolute/traversal before filesystem use.
 */
export const PRESET_REVISION_ID_PATTERN = /^rev-[a-z0-9]+(?:-[a-z0-9]+)*$/;

/** Validate a preset revision id; throws on traversal/separators. */
export function validatePresetRevisionId(revId: string): string {
	const trimmed = revId.trim();
	if (
		!PRESET_REVISION_ID_PATTERN.test(trimmed) ||
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

export function validatePresetExpectedActive(value: string | null | undefined): string | null | undefined {
	if (value === undefined || value === null) return value;
	const trimmed = value.trim();
	if (path.isAbsolute(trimmed) || trimmed.includes("/") || trimmed.includes("\\") || trimmed.includes("..")) {
		throw new Error(
			`Invalid revision id "${value}". Expected grammar rev-[a-z0-9]+(-[a-z0-9]+)* (no separators, absolute paths, or traversal).`,
		);
	}
	return trimmed;
}

/**
 * Single per-artifact serialized transaction for ALL preset mutation verbs.
 * Delegates to the shared revision-service boundary. Expected-revision
 * checks run INSIDE the boundary; null = explicit no-active.
 */
export function withArtifactTransaction<T>(kind: string, name: string, fn: () => Promise<T>): Promise<T> {
	if (kind !== "preset") throw new Error(`withArtifactTransaction(preset): unsupported kind "${kind}"`);
	return withRevisionTransaction("preset", sanitizePresetName(name), fn);
}

/** Per-revision evaluation-append chains so concurrent eval appends all survive. */
const presetEvalChains = new Map<string, Promise<unknown>>();
function serializePresetEvalAppend<T>(name: string, revId: string, op: () => Promise<T>): Promise<T> {
	const key = `${sanitizePresetName(name)}:${validatePresetRevisionId(revId)}`;
	const prev = presetEvalChains.get(key) ?? Promise.resolve();
	const run = prev.then(op, op);
	const guarded = run.catch(() => {});
	presetEvalChains.set(key, guarded);
	void guarded.finally(() => {
		if (presetEvalChains.get(key) === guarded) presetEvalChains.delete(key);
	});
	return run;
}

function assertManagedPresetFileSafeForUpdate(name: string, fileStat: Stats): void {
	if (!fileStat.isFile()) {
		throw new Error(`Managed preset "${name}" is not a regular file; refusing to overwrite it.`);
	}
	if (fileStat.nlink > 1) {
		throw new Error(
			`Managed preset "${name}" has ${fileStat.nlink} hard links; refusing to overwrite a file that may be user-authored elsewhere.`,
		);
	}
}

function assertPresetPathJailed(root: string, target: string, label: string): string {
	const sandbox = new SecuritySandbox(root);
	const check = sandbox.assertPathJailed(target);
	if (!check.jailed) throw new Error(`${label}: ${check.error}`);
	return check.resolvedPath;
}

function recheckPresetPathJailed(root: string, resolved: string, label: string): string {
	const sandbox = new SecuritySandbox(root);
	const check = sandbox.recheckJailed(resolved);
	if (!check.jailed) throw new Error(`${label}: ${check.error}`);
	return check.resolvedPath;
}

function assertPresetRevisionIdentity(safe: string, revision: { kind: string; name: string; id: string }): void {
	if (revision.kind !== "preset" || revision.name !== safe) {
		throw new Error(
			`Revision identity mismatch for preset "${safe}": got kind "${revision.kind}" name "${revision.name}" (${revision.id}).`,
		);
	}
}

function prevalidatePresetRevisionContent(safe: string, content: string): void {
	if (Buffer.byteLength(content, "utf8") > 64_000) {
		throw new Error(`Managed preset "${safe}" revision is over the byte limit.`);
	}
	try {
		const parsed = parseAgent(`managed:${safe}`, content, "user", "fatal");
		if (parsed.name !== safe) {
			throw new Error(`Revision for preset "${safe}" names "${parsed.name}" (identity mismatch).`);
		}
	} catch (err) {
		if (err instanceof Error && /identity mismatch/.test(err.message)) throw err;
		throw new Error(`Revision for preset "${safe}" fails the agent contract: ${(err as Error).message}`);
	}
}

function presetTxnFile(root: string, safe: string): string {
	return path.join(root, `${safe}.txn.json`);
}

async function writePresetTxn(root: string, safe: string, revId: string, previousActive: string | null): Promise<void> {
	await Bun.write(
		presetTxnFile(root, safe),
		JSON.stringify({ kind: "preset", name: safe, revId, previousActive, startedAt: new Date().toISOString() }),
	);
}

async function clearPresetTxn(root: string, safe: string): Promise<void> {
	try {
		await fs.rm(presetTxnFile(root, safe));
	} catch (err) {
		if (!isEnoent(err)) throw err;
	}
}

/**
 * Recover an interrupted preset promote/rollback before discovery exposes
 * artifacts: the file ahead of the pointer rolls back to the pointer's
 * content (old active stays published).
 */
export async function recoverInterruptedPresetTransaction(name: string, opts?: { agentDir?: string }): Promise<void> {
	const safe = sanitizePresetName(name);
	const root = opts?.agentDir ? path.join(opts.agentDir, "managed-presets") : getManagedPresetsDir();
	const txnPath = presetTxnFile(root, safe);
	let txn: { revId: string } | undefined;
	try {
		txn = (await Bun.file(txnPath).json()) as { revId: string };
	} catch (err) {
		if (isEnoent(err)) return;
		throw err;
	}
	if (!txn) return;
	const pointer = await readActivePointer("preset", safe, opts?.agentDir);
	const filePath = path.join(root, `${safe}.md`);
	const fileContent = await Bun.file(filePath)
		.text()
		.catch(err => {
			if (isEnoent(err)) return undefined;
			throw err;
		});
	const activeRev = pointer.active
		? await readRevision("preset", safe, pointer.active, opts?.agentDir).catch(() => undefined)
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
			const tmp = `${filePath}.recover.tmp`;
			await Bun.write(tmp, expected);
			await fs.rename(tmp, filePath);
		}
	}
	await clearPresetTxn(root, safe);
}

async function stageAndReplacePresetFile(file: string, content: string): Promise<void> {
	const tmp = `${file}.tmp`;
	await Bun.write(tmp, content);
	await fs.rename(tmp, file);
}

/** Whether `name` is claimed by a non-managed definition (authored/bundled/discovered). */
export function isPresetNameClaimedByAuthored(name: string, agents: readonly AgentDefinition[]): boolean {
	return agents.some(agent => agent.name === name && !agent.filePath?.includes("managed-presets"));
}

/** Serialize the preset file: YAML frontmatter (parseAgent-compatible) + system prompt body. */
export function buildPresetFileContent(spec: ManagedPresetSpec): string {
	const frontmatter: Record<string, unknown> = { name: spec.name, description: spec.description };
	if (spec.tools !== undefined) frontmatter.tools = spec.tools;
	if (spec.spawns !== undefined) frontmatter.spawns = spec.spawns;
	if (spec.model !== undefined) frontmatter.model = spec.model;
	const rendered = YAML.stringify(frontmatter, null, 2).trimEnd();
	return `---\n${rendered}\n---\n\n${spec.systemPrompt.trim()}\n`;
}

/** Read the currently materialized managed preset file, if any. */
export async function readManagedPresetFile(
	name: string,
	opts?: { agentDir?: string },
): Promise<{ content: string } | undefined> {
	const safe = sanitizePresetName(name);
	await recoverInterruptedPresetTransaction(safe, opts).catch(() => {});
	try {
		const root = opts?.agentDir ? path.join(opts.agentDir, "managed-presets") : getManagedPresetsDir();
		const file = path.join(root, `${safe}.md`);
		assertPresetPathJailed(root, file, `Managed preset "${safe}"`);
		return { content: await Bun.file(file).text() };
	} catch (err) {
		if (isEnoent(err)) return undefined;
		throw err;
	}
}

/** Load the current preset spec for merge (active revision preferred, file fallback). */
async function loadCurrentPresetSpec(safe: string, agentDir?: string): Promise<ManagedPresetSpec | undefined> {
	const active = await readActiveRevision("preset", safe, agentDir).catch(() => undefined);
	const content = active?.content ?? (await readManagedPresetFile(safe, { agentDir }).catch(() => undefined))?.content;
	if (!content) return undefined;
	try {
		const parsed = parseAgent(`managed:${safe}`, content, "user", "warn");
		const raw = parsed as AgentDefinition & { tools?: unknown; spawns?: unknown; model?: unknown };
		const spec: ManagedPresetSpec = {
			name: safe,
			description: parsed.description,
			systemPrompt: parsed.systemPrompt,
		};
		if (Array.isArray(raw.tools)) spec.tools = raw.tools as string[];
		// parseAgent normalizes model to string[]; preserve the stored shape when present.
		if (raw.model !== undefined) spec.model = raw.model as string | string[];
		try {
			const { parseFrontmatter: parseFm } = await import("@harvest/pi-utils");
			const fm = parseFm(content, { location: `managed:${safe}`, level: "warn" }).frontmatter as {
				spawns?: unknown;
			};
			if (typeof fm.spawns === "string") spec.spawns = fm.spawns;
		} catch {
			// spawns stays undefined when frontmatter unreadable; validation below decides.
		}
		return spec;
	} catch {
		return undefined;
	}
}

function validatePresetPolicyFields(
	safe: string,
	spec: { tools?: unknown; spawns?: unknown; model?: unknown },
): { tools?: string[]; spawns?: string; model?: string | string[] } {
	let tools: string[] | undefined;
	let spawns: string | undefined;
	let model: string | string[] | undefined;
	if (spec.tools !== undefined) {
		if (!Array.isArray(spec.tools) || !spec.tools.every(t => typeof t === "string" && t.trim())) {
			throw new Error(`Managed preset "${safe}" tools must be an array of non-empty strings.`);
		}
		tools = [...(spec.tools as string[])];
	}
	if (spec.spawns !== undefined) {
		if (typeof spec.spawns !== "string" || !spec.spawns.trim()) {
			throw new Error(`Managed preset "${safe}" spawns must be a non-empty string.`);
		}
		spawns = (spec.spawns as string).trim();
	}
	if (spec.model !== undefined) {
		if (typeof spec.model === "string") {
			if (!spec.model.trim()) throw new Error(`Managed preset "${safe}" model must be non-empty.`);
			model = spec.model.trim();
		} else if (Array.isArray(spec.model) && spec.model.every(m => typeof m === "string" && (m as string).trim())) {
			model = [...(spec.model as string[])];
		} else {
			throw new Error(`Managed preset "${safe}" model must be a string or array of non-empty strings.`);
		}
	}
	return { tools, spawns, model };
}

/**
 * Write a managed preset file into the isolated managed location. Refuses
 * names claimed by authored/bundled definitions and never writes outside
 * `getManagedPresetsDir()` (names are allowlist-validated; the root symlink
 * check mirrors the managed-skills guard). Update with omitted
 * tools/spawns/model/description/systemPrompt preserves existing values
 * against the current file/revision; explicit values are validated
 * replacements.
 */
export async function writeManagedPreset(input: WriteManagedPresetInput): Promise<{ path: string }> {
	const name = sanitizePresetName(input.name);
	const expectedActive = validatePresetExpectedActive(input.expectedActive);
	let description = input.description?.trim();
	let systemPrompt = input.systemPrompt?.trim();
	let tools = input.tools;
	let spawns = input.spawns;
	let model = input.model;
	if (input.action === "update") {
		const current = await loadCurrentPresetSpec(name);
		if (!current && (description === undefined || systemPrompt === undefined)) {
			throw new Error(`Managed preset "${name}" does not exist. Use action "create" to add it.`);
		}
		if (description === undefined) description = current?.description;
		if (systemPrompt === undefined) systemPrompt = current?.systemPrompt;
		if (tools === undefined) tools = current?.tools;
		if (spawns === undefined) spawns = current?.spawns;
		if (model === undefined) model = current?.model;
	}
	if (!description) throw new Error(`Managed preset "${name}" needs a non-empty description.`);
	if (!systemPrompt) throw new Error(`Managed preset "${name}" needs a non-empty system prompt.`);
	if (description.toLowerCase().startsWith("use this agent when") === false) {
		throw new Error(
			`Managed preset "${name}" description must start with 'Use this agent when…' (agent discovery trigger).`,
		);
	}
	const policy = validatePresetPolicyFields(name, { tools, spawns, model });
	const content = buildPresetFileContent({ name, description, systemPrompt, ...policy });
	prevalidatePresetRevisionContent(name, content);
	return withArtifactTransaction("preset", name, async () => {
		const root = await (async () => {
			const dir = getManagedPresetsDir();
			const rootStat = await fs.lstat(dir).catch(err => {
				if (isEnoent(err)) return null;
				throw err;
			});
			if (rootStat?.isSymbolicLink()) {
				throw new Error(
					"The managed-presets root is a symlink; refusing to operate outside the managed directory.",
				);
			}
			return dir;
		})();
		await recoverInterruptedPresetTransaction(name).catch(() => {});
		await ensurePresetHistorySeeded(name);
		if (expectedActive !== undefined) {
			const pointer = await readActivePointer("preset", name);
			const want = expectedActive ?? null;
			if (pointer.active !== want) {
				throw new Error(
					`Revision conflict for preset "${name}": expected active ${want ?? "none"}, found ${pointer.active ?? "none"}. Re-list revisions and retry.`,
				);
			}
		}
		const file = path.join(root, `${name}.md`);
		assertPresetPathJailed(root, file, `Managed preset "${name}"`);
		if (input.action === "create") {
			await fs.mkdir(root, { recursive: true });
			try {
				await fs.writeFile(file, content, { flag: "wx" });
			} catch (err) {
				if ((err as { code?: string }).code === "EEXIST") {
					throw new Error(`Managed preset "${name}" already exists. Use action "update" to change it.`);
				}
				throw err;
			}
			recheckPresetPathJailed(root, file, `Managed preset "${name}"`);
			try {
				const draft = await createDraftRevision({
					kind: "preset",
					name,
					content,
					description,
					provenance: { actor: "model" },
				});
				await promoteRevision("preset", name, draft.id, { discloseUnevaluated: true });
			} catch (err) {
				try {
					await fs.rm(file);
				} catch {
					// Best-effort rollback of a file whose pointer never published.
				}
				throw err;
			}
			void prunePresetRevisions(name).catch(() => {});
			return { path: file };
		}
		const existingStat = await fs.lstat(file).catch(err => {
			if (isEnoent(err)) return null;
			throw err;
		});
		if (existingStat === null)
			throw new Error(`Managed preset "${name}" does not exist. Use action "create" to add it.`);
		if (existingStat.isSymbolicLink()) {
			throw new Error(`Managed preset "${name}" is a symlink; refusing to overwrite it.`);
		}
		assertManagedPresetFileSafeForUpdate(name, existingStat);
		const oldContent = await Bun.file(file)
			.text()
			.catch(() => undefined);
		await stageAndReplacePresetFile(file, content);
		recheckPresetPathJailed(root, file, `Managed preset "${name}"`);
		try {
			const draft = await createDraftRevision({
				kind: "preset",
				name,
				content,
				description,
				provenance: { actor: "model" },
			});
			await promoteRevision("preset", name, draft.id, { discloseUnevaluated: true });
		} catch (err) {
			try {
				if (oldContent !== undefined) await stageAndReplacePresetFile(file, oldContent);
				else await fs.rm(file);
			} catch {
				// Best-effort rollback.
			}
			throw err;
		}
		void prunePresetRevisions(name).catch(() => {});
		return { path: file };
	});
}

/** Delete a managed preset file. Revision history is retained for audit. */
export async function deleteManagedPreset(name: string, opts?: { agentDir?: string }): Promise<void> {
	const safe = sanitizePresetName(name);
	await withArtifactTransaction("preset", safe, async () => {
		const root = opts?.agentDir ? path.join(opts.agentDir, "managed-presets") : getManagedPresetsDir();
		const file = path.join(root, `${safe}.md`);
		assertPresetPathJailed(root, file, `Managed preset "${safe}"`);
		const stat = await fs.lstat(file).catch(err => {
			if (isEnoent(err)) return null;
			throw err;
		});
		if (stat === null) throw new Error(`Managed preset "${safe}" does not exist.`);
		if (stat.isSymbolicLink()) {
			throw new Error(`Managed preset "${safe}" is a symlink; refusing to delete outside the managed directory.`);
		}
		try {
			await fs.rm(file);
		} catch (err) {
			if (isEnoent(err)) throw new Error(`Managed preset "${safe}" does not exist.`);
			throw err;
		}
		await clearPresetTxn(root, safe).catch(() => {});
	});
}

/** Seed preset history from the materialized file on first managed mutation. */
export async function ensurePresetHistorySeeded(name: string, opts?: { agentDir?: string }): Promise<void> {
	const safe = sanitizePresetName(name);
	const existing = await listRevisions("preset", safe, opts?.agentDir);
	if (existing.length > 0) return;
	const file = await readManagedPresetFile(safe, opts);
	if (!file) return;
	const seeded = await createDraftRevision({
		kind: "preset",
		name: safe,
		content: file.content,
		description: "seeded from pre-revision managed preset",
		parent: null,
		provenance: { actor: "seed" },
		agentDir: opts?.agentDir,
	});
	await promoteRevision("preset", safe, seeded.id, { discloseUnevaluated: true, agentDir: opts?.agentDir });
}

export interface PresetDraftInput extends Partial<ManagedPresetSpec> {
	name: string;
	expectedActive?: string | null;
	provenance?: { sessionId?: string; runId?: string; actor?: string };
	agentDir?: string;
}

/** Mint a draft preset revision without touching the materialized file (merge-aware). */
export async function createPresetDraft(input: PresetDraftInput): Promise<ArtifactRevision> {
	const name = sanitizePresetName(input.name);
	const expectedActive = validatePresetExpectedActive(input.expectedActive);
	return withArtifactTransaction("preset", name, async () => {
		await ensurePresetHistorySeeded(name, { agentDir: input.agentDir });
		let description = input.description?.trim();
		let systemPrompt = input.systemPrompt?.trim();
		let tools = input.tools;
		let spawns = input.spawns;
		let model = input.model;
		if (
			description === undefined ||
			systemPrompt === undefined ||
			tools === undefined ||
			spawns === undefined ||
			model === undefined
		) {
			const current = await loadCurrentPresetSpec(name, input.agentDir);
			if (description === undefined) description = current?.description;
			if (systemPrompt === undefined) systemPrompt = current?.systemPrompt;
			if (tools === undefined) tools = current?.tools;
			if (spawns === undefined) spawns = current?.spawns;
			if (model === undefined) model = current?.model;
		}
		if (!description) throw new Error(`Managed preset "${name}" needs a non-empty description.`);
		if (!systemPrompt) throw new Error(`Managed preset "${name}" needs a non-empty system prompt.`);
		if (description.toLowerCase().startsWith("use this agent when") === false) {
			throw new Error(
				`Managed preset "${name}" description must start with 'Use this agent when…' (agent discovery trigger).`,
			);
		}
		const policy = validatePresetPolicyFields(name, { tools, spawns, model });
		const content = buildPresetFileContent({ name, description, systemPrompt, ...policy });
		prevalidatePresetRevisionContent(name, content);
		return createDraftRevision({
			kind: "preset",
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

/** Internal preset materialize without serialization (caller holds the transaction). */
async function materializePresetRevisionInner(
	safe: string,
	revId: string,
	root: string,
	agentDir?: string,
): Promise<{ path: string }> {
	const revision = await readRevision("preset", safe, revId, agentDir);
	if (!revision) throw new Error(`Revision ${revId} for preset "${safe}" not found`);
	assertPresetRevisionIdentity(safe, revision);
	validatePresetRevisionId(revision.id);
	prevalidatePresetRevisionContent(safe, revision.content);
	const file = path.join(root, `${safe}.md`);
	assertPresetPathJailed(root, file, `Managed preset "${safe}"`);
	await fs.mkdir(root, { recursive: true });
	const fileStat = await fs.lstat(file).catch(err => {
		if (isEnoent(err)) return null;
		throw err;
	});
	if (fileStat === null) {
		try {
			await fs.writeFile(file, revision.content, { flag: "wx" });
		} catch (err) {
			if ((err as { code?: string }).code !== "EEXIST") throw err;
			throw new Error(`Managed preset "${safe}" appeared mid-materialize; retry the promotion.`);
		}
		recheckPresetPathJailed(root, file, `Managed preset "${safe}"`);
		return { path: file };
	}
	if (fileStat.isSymbolicLink()) {
		throw new Error(`Managed preset "${safe}" is a symlink; refusing to overwrite it.`);
	}
	assertManagedPresetFileSafeForUpdate(safe, fileStat);
	await stageAndReplacePresetFile(file, revision.content);
	recheckPresetPathJailed(root, file, `Managed preset "${safe}"`);
	return { path: file };
}

/** Materialize a preset revision as the live managed-preset file. */
export async function materializePresetRevision(
	name: string,
	revId: string,
	opts?: { agentDir?: string },
): Promise<{ path: string }> {
	const safe = sanitizePresetName(name);
	const cleanRev = validatePresetRevisionId(revId);
	return withArtifactTransaction("preset", safe, async () => {
		const root = opts?.agentDir ? path.join(opts.agentDir, "managed-presets") : getManagedPresetsDir();
		const rootStat = await fs.lstat(root).catch(err => {
			if (isEnoent(err)) return null;
			throw err;
		});
		if (rootStat?.isSymbolicLink()) {
			throw new Error("The managed-presets root is a symlink; refusing to operate outside the managed directory.");
		}
		await recoverInterruptedPresetTransaction(safe, opts).catch(() => {});
		return materializePresetRevisionInner(safe, cleanRev, root, opts?.agentDir);
	});
}

export interface PromotePresetResult {
	path: string;
	revId: string;
	disclosedUnevaluated: boolean;
}

/**
 * Promote a draft preset to active and materialize it. Failed evaluations
 * block promotion; unevaluated content needs explicit `discloseUnevaluated`.
 * Materialize-first, pointer-only-after; rollback file on pointer failure.
 */
export async function promotePresetRevision(
	name: string,
	revId: string,
	opts?: { discloseUnevaluated?: boolean; agentDir?: string },
): Promise<PromotePresetResult> {
	const safe = sanitizePresetName(name);
	const cleanRev = validatePresetRevisionId(revId);
	return withArtifactTransaction("preset", safe, async () => {
		const root = opts?.agentDir ? path.join(opts.agentDir, "managed-presets") : getManagedPresetsDir();
		await recoverInterruptedPresetTransaction(safe, opts).catch(() => {});
		const revision = await readRevision("preset", safe, cleanRev, opts?.agentDir);
		if (!revision) throw new Error(`Revision ${cleanRev} for preset "${safe}" not found`);
		assertPresetRevisionIdentity(safe, revision);
		prevalidatePresetRevisionContent(safe, revision.content);
		if (revision.evaluations.length > 0 && !isEvaluatedPassing(revision)) {
			throw new Error(
				`Revision ${cleanRev} for preset "${safe}" has failing evaluations; it cannot be promoted. Fix the content and evaluate a new draft.`,
			);
		}
		const unevaluated = revision.evaluations.length === 0;
		const before = await readActivePointer("preset", safe, opts?.agentDir);
		const filePath = path.join(root, `${safe}.md`);
		const oldContent = await Bun.file(filePath)
			.text()
			.catch(err => {
				if (isEnoent(err)) return undefined;
				throw err;
			});
		await writePresetTxn(root, safe, cleanRev, before.active);
		await materializePresetRevisionInner(safe, cleanRev, root, opts?.agentDir);
		try {
			await promoteRevision("preset", safe, cleanRev, {
				discloseUnevaluated: opts?.discloseUnevaluated,
				agentDir: opts?.agentDir,
			});
		} catch (err) {
			try {
				if (oldContent !== undefined) await stageAndReplacePresetFile(filePath, oldContent);
				else await fs.rm(filePath);
			} catch {
				// Best-effort rollback.
			}
			await clearPresetTxn(root, safe).catch(() => {});
			throw err;
		}
		await clearPresetTxn(root, safe).catch(() => {});
		void prunePresetRevisions(safe, { agentDir: opts?.agentDir }).catch(() => {});
		return { path: filePath, revId: cleanRev, disclosedUnevaluated: unevaluated };
	});
}

/** Roll the active preset pointer back to a prior revision and materialize it. */
export async function rollbackPresetRevision(
	name: string,
	revId: string,
	opts?: { agentDir?: string },
): Promise<{ path: string }> {
	const safe = sanitizePresetName(name);
	const cleanRev = validatePresetRevisionId(revId);
	return withArtifactTransaction("preset", safe, async () => {
		const root = opts?.agentDir ? path.join(opts.agentDir, "managed-presets") : getManagedPresetsDir();
		await recoverInterruptedPresetTransaction(safe, opts).catch(() => {});
		const revision = await readRevision("preset", safe, cleanRev, opts?.agentDir);
		if (!revision) throw new Error(`Revision ${cleanRev} for preset "${safe}" not found`);
		assertPresetRevisionIdentity(safe, revision);
		if (revision.state !== "active") {
			throw new Error(
				`Cannot rollback preset "${safe}" to revision ${cleanRev}: it was never active (state ${revision.state}). Rollback targets a previously-activated revision.`,
			);
		}
		prevalidatePresetRevisionContent(safe, revision.content);
		const before = await readActivePointer("preset", safe, opts?.agentDir);
		const filePath = path.join(root, `${safe}.md`);
		const oldContent = await Bun.file(filePath)
			.text()
			.catch(err => {
				if (isEnoent(err)) return undefined;
				throw err;
			});
		await writePresetTxn(root, safe, cleanRev, before.active);
		await materializePresetRevisionInner(safe, cleanRev, root, opts?.agentDir);
		try {
			await rollbackRevision("preset", safe, cleanRev, opts?.agentDir);
		} catch (err) {
			try {
				if (oldContent !== undefined) await stageAndReplacePresetFile(filePath, oldContent);
				else await fs.rm(filePath);
			} catch {
				// Best-effort rollback.
			}
			await clearPresetTxn(root, safe).catch(() => {});
			throw err;
		}
		await clearPresetTxn(root, safe).catch(() => {});
		void prunePresetRevisions(safe, { agentDir: opts?.agentDir }).catch(() => {});
		return { path: filePath };
	});
}

/** List preset revision history with the active pointer. */
export async function listPresetRevisions(
	name: string,
	opts?: { agentDir?: string },
): Promise<{ active: string | null; revisions: ArtifactRevision[] }> {
	const safe = sanitizePresetName(name);
	await recoverInterruptedPresetTransaction(safe, opts).catch(() => {});
	const [pointer, revisions] = await Promise.all([
		readActivePointer("preset", safe, opts?.agentDir),
		listRevisions("preset", safe, opts?.agentDir),
	]);
	return { active: pointer.active, revisions };
}

/** Read the active preset revision, if any. */
export async function readActivePresetRevision(
	name: string,
	opts?: { agentDir?: string },
): Promise<ArtifactRevision | undefined> {
	const safe = sanitizePresetName(name);
	await recoverInterruptedPresetTransaction(safe, opts).catch(() => {});
	const rev = await readActiveRevision("preset", safe, opts?.agentDir);
	if (rev) assertPresetRevisionIdentity(safe, rev);
	return rev;
}

// ── Preset pinning (promotion affects subsequent work only) ───────────────

const presetPinOwners = new Map<string, Set<string>>();
const presetPinKey = (name: string, revId: string): string =>
	`preset:${sanitizePresetName(name)}:${validatePresetRevisionId(revId)}`;

/** Pin a preset revision so pruning retains it while a run is live. */
export function pinPresetRevision(name: string, revId: string): void {
	const key = presetPinKey(name, revId);
	let owners = presetPinOwners.get(key);
	if (!owners) {
		owners = new Set();
		presetPinOwners.set(key, owners);
	}
	owners.add("__manual__");
}

/** Owner-aware pin for a live run. */
export function pinPresetRevisionForRun(name: string, revId: string, runId: string): void {
	if (!runId.trim()) throw new Error("pinPresetRevisionForRun needs a non-empty run id");
	const key = presetPinKey(name, revId);
	let owners = presetPinOwners.get(key);
	if (!owners) {
		owners = new Set();
		presetPinOwners.set(key, owners);
	}
	owners.add(`run:${runId}`);
}

/** Release a run's pin on a preset revision. */
export function unpinPresetRevision(name: string, revId: string): void {
	const key = presetPinKey(name, revId);
	const owners = presetPinOwners.get(key);
	if (!owners) return;
	owners.delete("__manual__");
	if (owners.size === 0) presetPinOwners.delete(key);
}

/** Release one run's pin; other runs' pins survive. */
export function unpinPresetRevisionForRun(name: string, revId: string, runId: string): void {
	const key = presetPinKey(name, revId);
	const owners = presetPinOwners.get(key);
	if (!owners) return;
	owners.delete(`run:${runId}`);
	if (owners.size === 0) presetPinOwners.delete(key);
}

/** Pins for one preset, for prune integration. */
export function getPresetPins(name: string): Set<string> {
	const safe = sanitizePresetName(name);
	const prefix = `preset:${safe}:`;
	const out = new Set<string>();
	for (const key of presetPinOwners.keys()) {
		if (key.startsWith(prefix)) out.add(key.slice(prefix.length));
	}
	return out;
}

/** Test-only: release every preset pin. */
export function clearPresetPinsForTests(): void {
	presetPinOwners.clear();
}

/** Prune preset history to the latest twenty, retaining pins and the active revision. */
export async function prunePresetRevisions(name: string, opts?: { agentDir?: string }): Promise<ArtifactRevision[]> {
	return pruneRevisions("preset", sanitizePresetName(name), getPresetPins(sanitizePresetName(name)), opts?.agentDir);
}

// ── Preset evaluation ─────────────────────────────────────────────────────

export interface PresetEvalRequest {
	task: string;
	expectedOutcome: string;
	sessionId?: string;
	runId?: string;
	agentDir?: string;
	/** Parent session context for the production executor (real task execution). */
	parent?: ToolSession;
}

export interface PresetEvalOutcome {
	passed: boolean;
	summary: string;
	runId?: string;
	sessionId?: string;
}

export type PresetEvalRunner = (
	input: PresetEvalRequest & { name: string; revId: string; content: string },
) => Promise<PresetEvalOutcome>;

let presetEvalRunner: PresetEvalRunner | undefined;

/** Override the preset evaluation executor (tests inject deterministic fixtures). */
export function setPresetEvalRunner(runner: PresetEvalRunner | undefined): void {
	presetEvalRunner = runner;
}

let presetEvalRunDepth = 0;

/** True while inside a preset evaluation executor (no recursive auto-learning). */
export function isPresetEvalRunActive(): boolean {
	return presetEvalRunDepth > 0;
}

/**
 * Evaluate a draft preset revision against an explicit task + expected
 * observable outcome. Records the result; never promotes. Model-role
 * resolution and task execution restrictions are preserved by routing the run
 * through the existing restricted task execution seam. Append serialized so
 * concurrent evals all survive.
 */
export async function evaluatePresetRevision(
	name: string,
	revId: string,
	request: PresetEvalRequest,
): Promise<{ passed: boolean; summary: string }> {
	const safe = sanitizePresetName(name);
	const cleanRev = validatePresetRevisionId(revId);
	const task = request.task.trim();
	const expectedOutcome = request.expectedOutcome.trim();
	if (!task) throw new Error(`Evaluation of preset "${safe}" needs an explicit task.`);
	if (!expectedOutcome) throw new Error(`Evaluation of preset "${safe}" needs an explicit expected outcome.`);
	const revision = await readRevision("preset", safe, cleanRev, request.agentDir);
	if (!revision) throw new Error(`Revision ${cleanRev} for preset "${safe}" not found`);
	assertPresetRevisionIdentity(safe, revision);
	if (!presetEvalRunner) {
		throw new Error(
			`No evaluation executor is wired for preset "${safe}" (owner patch: task-execution seam). ` +
				`The revision stays unevaluated — unevaluated content is never treated as passed.`,
		);
	}
	presetEvalRunDepth++;
	const evalOwner = request.runId?.trim() || `eval:${safe}:${cleanRev}`;
	try {
		pinPresetRevisionForRun(safe, cleanRev, evalOwner);
	} catch {
		// Pinning is retention hygiene, never evaluation fate.
	}
	try {
		const outcome = await presetEvalRunner({
			...request,
			task,
			expectedOutcome,
			name: safe,
			revId: cleanRev,
			content: revision.content,
		});
		await serializePresetEvalAppend(safe, cleanRev, async () =>
			recordEvaluation(
				"preset",
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
			unpinPresetRevisionForRun(safe, cleanRev, evalOwner);
		} catch {
			// Best-effort release.
		}
		presetEvalRunDepth = Math.max(0, presetEvalRunDepth - 1);
	}
}

// ── Preset auto-improvement bound (mirrors skills; eval runs never claim) ─

interface PresetImprovementBudget {
	turns: number;
	candidates: number;
	evaluations: number;
}

const presetImprovementBudgets = new WeakMap<object, PresetImprovementBudget>();
const presetImprovementBudgetByKey = new Map<string, PresetImprovementBudget>();

function presetBudgetFor(key: object | string): PresetImprovementBudget {
	if (typeof key === "string") {
		let budget = presetImprovementBudgetByKey.get(key);
		if (!budget) {
			budget = { turns: 0, candidates: 0, evaluations: 0 };
			presetImprovementBudgetByKey.set(key, budget);
		}
		return budget;
	}
	let budget = presetImprovementBudgets.get(key);
	if (!budget) {
		budget = { turns: 0, candidates: 0, evaluations: 0 };
		presetImprovementBudgets.set(key, budget);
	}
	return budget;
}

/** Mark a parent turn completed; resets the per-turn candidate/eval allowance. */
export function notePresetParentTurnCompleted(key: object | string): void {
	const budget = presetBudgetFor(key);
	budget.turns++;
	budget.candidates = 0;
	budget.evaluations = 0;
}

/** Claim the single preset candidate slot for the current parent turn. */
export function tryClaimPresetImprovementCandidate(key: object | string): boolean {
	if (isPresetEvalRunActive()) return false;
	const budget = presetBudgetFor(key);
	if (budget.candidates >= 1) return false;
	budget.candidates++;
	return true;
}

/** Claim the single preset evaluation slot for the current parent turn. */
export function tryClaimPresetImprovementEvaluation(key: object | string): boolean {
	if (isPresetEvalRunActive()) return false;
	const budget = presetBudgetFor(key);
	if (budget.evaluations >= 1) return false;
	budget.evaluations++;
	return true;
}

/** Test-only: drop string-keyed preset improvement budgets. */
export function clearPresetImprovementBudgetsForTests(): void {
	presetImprovementBudgetByKey.clear();
}

/**
 * Enforce the automatic-improvement bound for the auto-continue loop: one
 * preset candidate plus one evaluation per parent turn. Scoped to sessions
 * running with `autolearn.autoContinue`; ordinary sessions are never gated.
 * No-op without a session identity. Throws when this turn's slot is spent.
 */
export function requirePresetAutoImprovementBudget(
	session: Pick<ToolSession, "getSessionId" | "settings">,
	slot: "candidate" | "evaluation",
): void {
	if (session.settings.get("autolearn.autoContinue") !== true) return;
	const id = session.getSessionId?.() ?? null;
	if (!id) return;
	const claimed =
		slot === "candidate" ? tryClaimPresetImprovementCandidate(id) : tryClaimPresetImprovementEvaluation(id);
	if (!claimed) {
		throw new Error(
			`Automatic preset improvement already used its ${slot} slot this turn; further automatic ${slot}s resume next turn.`,
		);
	}
}

/**
 * Scan the isolated managed-presets directory and parse each file with the
 * same agent contract as discovery. Authored/bundled protection is enforced
 * at write time; at read time, callers merge with `discoverAgents` letting
 * non-managed definitions win on collision. Interrupted transactions recover
 * before artifacts are exposed; artifact identity (frontmatter name matches
 * the filename) is verified on load.
 */
export async function discoverManagedPresets(agentDir: string = getAgentDir()): Promise<AgentDefinition[]> {
	const dir = path.join(agentDir, "managed-presets");
	const entries = await fs.readdir(dir).catch(err => {
		if (isEnoent(err)) return [];
		throw err;
	});
	const found: AgentDefinition[] = [];
	for (const entry of entries.sort()) {
		if (!entry.endsWith(".md")) continue;
		if (entry.endsWith(".txn.json") || entry.endsWith(".tmp")) continue;
		const stem = entry.slice(0, -".md".length);
		if (!isValidPresetName(stem)) continue;
		await recoverInterruptedPresetTransaction(stem, { agentDir }).catch(() => {});
		const filePath = path.join(dir, entry);
		assertPresetPathJailed(dir, filePath, `Managed preset "${stem}"`);
		const stat = await fs.lstat(filePath).catch(() => null);
		if (!stat || stat.isSymbolicLink() || !stat.isFile() || stat.nlink > 1) continue;
		let content: string;
		try {
			content = await fs.readFile(filePath, "utf-8");
		} catch {
			continue;
		}
		try {
			const parsed = parseAgent(filePath, content, "user", "warn");
			if (parsed.name !== stem) continue;
			found.push(parsed);
		} catch {
			continue;
		}
	}
	return found;
}
