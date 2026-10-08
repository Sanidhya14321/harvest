import { type } from "@harvest/omptype";
import type { AgentTool, AgentToolResult } from "@harvest/pi-agent-core";
import agentCreationArchitectPrompt from "../prompts/system/agent-creation-architect.md" with { type: "text" };
import agentCreationUserPrompt from "../prompts/system/agent-creation-user.md" with { type: "text" };
import {
	deleteManagedPreset,
	discoverManagedPresets,
	evaluatePresetRevision,
	getPresetPins,
	isPresetNameClaimedByAuthored,
	listPresetRevisions,
	loadBundledAgents,
	promotePresetRevision,
	readActivePresetRevision,
	requirePresetAutoImprovementBudget,
	rollbackPresetRevision,
	sanitizePresetName,
	validatePresetRevisionId,
	writeManagedPreset,
} from "../task/agents";
import { discoverAgents } from "../task/discovery";
import { resolveSpawnPolicy } from "../task/spawn-policy";
import { createToolEvalSignal } from "../autolearn/revisions";
import presetsDescription from "../prompts/tools/presets.md" with { type: "text" };
import type { ToolSession } from ".";

const presetsSchema = type({
	action: "'list' | 'inspect' | 'create' | 'update' | 'evaluate' | 'promote' | 'rollback' | 'delete'",
	"name?": type("string").describe("preset identifier (required except for list)"),
	"description?": type("string").describe(
		"one-line trigger starting with 'Use this agent when…' (required for create/update)",
	),
	"systemPrompt?": type("string").describe("complete agent instructions (required for create/update)"),
	"brief?": type("string").describe("short goal for architect-generated presets (alternative to systemPrompt)"),
	"tools?": "string[]",
	"spawns?": type("string").describe("spawn policy for the preset (frontmatter spawns:)"),
	"model?": type("string").describe("model pattern for the preset (frontmatter model:)"),
	"revisionId?": type("string").describe(
		"target revision for evaluate/promote/rollback (defaults to the latest draft)",
	),
	"expectedActive?": type("string").describe("expected current active revision for update; rejects on conflict"),
	"task?": type("string").describe("explicit evaluation task (required for evaluate)"),
	"expectedOutcome?": type("string").describe("explicit expected observable outcome (required for evaluate)"),
	"discloseUnevaluated?": type("boolean").describe("explicitly activate unevaluated content on promote"),
}).narrow(
	(p, ctx) =>
		p.action === "list" || p.name !== undefined || ctx.mustBe('used with "name" for every action except "list"'),
);

export type PresetsParams = typeof presetsSchema.infer;

export interface GeneratedPresetSpec {
	identifier: string;
	whenToUse: string;
	systemPrompt: string;
}

function extractJsonObject(raw: string): string {
	const fenceMatch = raw.match(/```(?:json)?\s*([\s\S]*?)```/i);
	if (fenceMatch?.[1]) return fenceMatch[1].trim();
	const start = raw.indexOf("{");
	const end = raw.lastIndexOf("}");
	if (start >= 0 && end >= start) return raw.slice(start, end + 1).trim();
	return raw.trim();
}

function parseGeneratedPresetSpec(raw: string): GeneratedPresetSpec {
	const parsed = JSON.parse(extractJsonObject(raw)) as Partial<GeneratedPresetSpec>;
	if (!parsed || typeof parsed !== "object") throw new Error("Model output is not a JSON object");
	if (
		typeof parsed.identifier !== "string" ||
		typeof parsed.whenToUse !== "string" ||
		typeof parsed.systemPrompt !== "string"
	) {
		throw new Error("Model output is missing required fields (identifier, whenToUse, systemPrompt)");
	}
	const identifier = parsed.identifier.trim();
	const whenToUse = parsed.whenToUse.trim();
	const systemPrompt = parsed.systemPrompt.trim();
	const { PRESET_IDENTIFIER_PATTERN } = { PRESET_IDENTIFIER_PATTERN: /^[a-z0-9]+(?:-[a-z0-9]+){1,5}$/ };
	if (!PRESET_IDENTIFIER_PATTERN.test(identifier)) {
		throw new Error("Generated identifier is invalid (must be lowercase kebab-case, 2+ words)");
	}
	if (!whenToUse.toLowerCase().startsWith("use this agent when")) {
		throw new Error("Generated whenToUse must start with 'Use this agent when...'");
	}
	if (!systemPrompt) throw new Error("Generated systemPrompt is empty");
	return { identifier, whenToUse, systemPrompt };
}

export type PresetArchitectRunner = (input: {
	brief: string;
	systemPrompt: string[];
	userPrompt: (request: string) => string;
}) => Promise<string>;

let presetArchitectRunner: PresetArchitectRunner | undefined;

/** Override the preset architect executor (tests inject deterministic fixtures). */
export function setPresetArchitectRunner(runner: PresetArchitectRunner | undefined): void {
	presetArchitectRunner = runner;
}

/**
 * Managed agent presets: versioned task-agent definitions in an isolated
 * managed location. Generated presets never touch authored/bundled dirs;
 * name collisions — including authored/bundled claims — are refused.
 *
 * Creation runs through an isolated architect session (hasUI:false, minimal
 * tool surface, detached prompt, disposed afterwards) mirroring the /agents
 * hub save flow; the tool-level flow stays here so the hub owner keeps the
 * interactive surface. After every save, agent discovery is invalidated via
 * `refreshAgentDiscovery` (owner-owned; imported, clearing both the
 * create-time memo and published snapshots).
 */
export class PresetsTool implements AgentTool<typeof presetsSchema> {
	readonly name = "presets";
	readonly approval = (args: unknown): "read" | "write" | "exec" => {
		const action = (args as Partial<PresetsParams>).action;
		if (action === "list" || action === "inspect") return "read";
		if (action === "create" || action === "evaluate") return "exec";
		return "write";
	};
	readonly label = "Presets";
	readonly description = presetsDescription;
	readonly parameters = presetsSchema;
	readonly strict = true;
	readonly loadMode = "essential" as const;
	readonly summary = "Manage versioned agent presets in an isolated managed location";

	constructor(private readonly session: ToolSession) {}

	static createIf(session: ToolSession): PresetsTool | null {
		if (!session.settings.get("autolearn.enabled")) return null;
		return new PresetsTool(session);
	}

	async execute(_id: string, params: PresetsParams, signal?: AbortSignal): Promise<AgentToolResult> {
		switch (params.action) {
			case "list":
				return this.listPresets();
			case "inspect":
				return this.inspectPreset(params);
			case "create":
				return this.createPreset(params);
			case "update":
				return this.updatePreset(params);
			case "evaluate":
				return this.evaluatePreset(params, signal);
			case "promote":
				return this.promotePreset(params);
			case "rollback":
				return this.rollbackPreset(params);
			case "delete":
				return this.deletePreset(params);
			default:
				throw new Error(`Unknown presets action "${(params as { action: string }).action}".`);
		}
	}

	private requireName(params: PresetsParams): string {
		if (!params.name) throw new Error(`"${params.action}" requires "name".`);
		return sanitizePresetName(params.name);
	}

	private async listPresets(): Promise<AgentToolResult> {
		const { agents } = await discoverAgents(this.session.cwd).catch(() => ({ agents: [] }));
		const managed = await discoverManagedPresets();
		const seen = new Set(agents.map(agent => agent.name));
		const rows: string[] = agents.map(agent => `- ${agent.name} (${agent.source})`);
		const details: Array<{
			name: string;
			source: string;
			active: string | null;
			revisions: Array<{
				id: string;
				state: string;
				active: boolean;
				evaluations: number;
				evalStatus: string;
				pinned: boolean;
			}>;
		}> = [];
		for (const agent of managed.filter(a => !seen.has(a.name))) {
			const { active, revisions } = await listPresetRevisions(agent.name).catch(() => ({
				active: null as string | null,
				revisions: [],
			}));
			const pins = getPresetPins(agent.name);
			rows.push(
				`- ${agent.name} (managed, active ${active ?? "none"}, ${revisions.length} revision(s)` +
					`${revisions.some(r => pins.has(r.id)) ? ", pinned" : ""})`,
			);
			details.push({
				name: agent.name,
				source: "managed",
				active,
				revisions: revisions.map(rev => ({
					id: rev.id,
					state: rev.state,
					active: rev.id === active,
					evaluations: rev.evaluations.length,
					evalStatus:
						rev.evaluations.length === 0
							? "unevaluated"
							: rev.evaluations.every(e => e.passed)
								? "passing"
								: "failing",
					pinned: pins.has(rev.id),
				})),
			});
		}
		return {
			content: [
				{
					type: "text",
					text:
						`Agent presets (${rows.length}):\n${rows.join("\n")}\n` +
						`Draft stages without touching the live file; evaluate records pass/fail; promote activates (failed blocks, unevaluated needs discloseUnevaluated); rollback reactivates a prior active revision.`,
				},
			],
			details: { action: "list", count: rows.length, managed: details },
		};
	}

	private async inspectPreset(params: PresetsParams): Promise<AgentToolResult> {
		const name = this.requireName(params);
		const { agents } = await discoverAgents(this.session.cwd).catch(() => ({ agents: [] }));
		const known = agents.find(agent => agent.name === name);
		const managed = (await discoverManagedPresets()).find(agent => agent.name === name);
		const { active, revisions } = await listPresetRevisions(name).catch(() => ({
			active: null,
			revisions: [],
		}));
		const pins = getPresetPins(name);
		const definition = managed ?? known;
		if (!definition && revisions.length === 0) {
			return {
				content: [{ type: "text", text: `Preset "${name}" not found.` }],
				isError: true,
				details: { action: "inspect", name },
			};
		}
		const lines = revisions.map(rev => {
			const status =
				rev.evaluations.length === 0 ? "unevaluated" : rev.evaluations.every(e => e.passed) ? "passing" : "failing";
			return `- ${rev.id} (${rev.state}${rev.id === active ? ", active" : ""}${pins.has(rev.id) ? ", pinned" : ""}, eval:${status}): ${rev.evaluations.length} evaluation(s)`;
		});
		return {
			content: [
				{
					type: "text",
					text:
						`Preset "${name}": ${definition?.description ?? "(no live definition)"}\n` +
						`Active revision: ${active ?? "none"}; ${revisions.length} revision(s).\n${lines.join("\n")}`,
				},
			],
			details: {
				action: "inspect",
				name,
				active,
				source: managed ? "managed" : (known?.source ?? null),
				revisions: revisions.map(rev => ({
					id: rev.id,
					state: rev.state,
					active: rev.id === active,
					evaluations: rev.evaluations.length,
					evalStatus:
						rev.evaluations.length === 0
							? "unevaluated"
							: rev.evaluations.every(e => e.passed)
								? "passing"
								: "failing",
					pinned: pins.has(rev.id),
				})),
			},
		};
	}

	private async claimedNames(): Promise<Set<string>> {
		const { agents } = await discoverAgents(this.session.cwd).catch(() => ({ agents: [] }));
		return new Set([...agents, ...loadBundledAgents()].map(agent => agent.name));
	}

	private async createPreset(params: PresetsParams): Promise<AgentToolResult> {
		const requested = params.name ? sanitizePresetName(params.name) : null;
		requirePresetAutoImprovementBudget(this.session, "candidate");
		let spec: { name: string; description: string; systemPrompt: string };
		if (params.systemPrompt) {
			if (!params.description) throw new Error(`"create" with "systemPrompt" requires "description".`);
			if (!requested) throw new Error(`"create" with "systemPrompt" requires "name".`);
			spec = { name: requested, description: params.description, systemPrompt: params.systemPrompt };
		} else {
			if (!params.brief?.trim()) throw new Error(`"create" requires "systemPrompt" or "brief".`);
			const generated = await this.runArchitect(params.brief.trim());
			if (requested && generated.identifier !== requested) {
				throw new Error(
					`Architect proposed "${generated.identifier}" but "${requested}" was requested; retry with a matching brief or omit "name".`,
				);
			}
			spec = { name: generated.identifier, description: generated.whenToUse, systemPrompt: generated.systemPrompt };
		}
		const claimed = await this.claimedNames();
		// Reject name collisions, including authored/bundled claims: a managed
		// preset under a claimed name would never surface (authored wins) or
		// would shadow a bundled definition.
		if (claimed.has(spec.name) || loadBundledAgents().some(agent => agent.name === spec.name)) {
			return {
				content: [
					{
						type: "text",
						text: `Cannot create preset "${spec.name}": that name is already claimed by an authored, discovered, or bundled agent. Managed presets cannot override them. Choose a different name.`,
					},
				],
				isError: true,
				details: { action: "create", name: spec.name, collision: true },
			};
		}
		const { agents } = await discoverAgents(this.session.cwd).catch(() => ({ agents: [] as never[] }));
		if (isPresetNameClaimedByAuthored(spec.name, agents)) {
			return {
				content: [
					{
						type: "text",
						text: `Cannot create preset "${spec.name}": an authored or discovered agent already claims it. Choose a different name.`,
					},
				],
				isError: true,
				details: { action: "create", name: spec.name, collision: true },
			};
		}
		const { path: file } = await writeManagedPreset({
			action: "create",
			name: spec.name,
			description: spec.description,
			systemPrompt: spec.systemPrompt,
			...(params.tools !== undefined ? { tools: params.tools } : {}),
			...(params.spawns !== undefined ? { spawns: params.spawns } : {}),
			...(params.model !== undefined ? { model: params.model } : {}),
			expectedActive: params.expectedActive,
		});
		const discovery = await this.invalidateDiscovery();
		return {
			content: [{ type: "text", text: `Created managed preset "${spec.name}" (${file}).` }],
			details: {
				action: "create",
				name: spec.name,
				evaluated: false,
				status: "unevaluated-active",
				discoveryInvalidated: discovery.invalidated,
			},
		};
	}

	private async updatePreset(params: PresetsParams): Promise<AgentToolResult> {
		const name = this.requireName(params);
		if (
			params.description === undefined &&
			params.systemPrompt === undefined &&
			params.tools === undefined &&
			params.spawns === undefined &&
			params.model === undefined
		) {
			throw new Error(
				`"update" needs at least one of "description", "systemPrompt", "tools", "spawns", "model" (omitted policy fields merge from current).`,
			);
		}
		requirePresetAutoImprovementBudget(this.session, "candidate");
		const { agents } = await discoverAgents(this.session.cwd).catch(() => ({ agents: [] as never[] }));
		if (isPresetNameClaimedByAuthored(name, agents)) {
			return {
				content: [
					{
						type: "text",
						text: `Cannot update preset "${name}": it is claimed by an authored or discovered agent, which managed presets cannot override.`,
					},
				],
				isError: true,
				details: { action: "update", name, collision: true },
			};
		}
		const { path: file } = await writeManagedPreset({
			action: "update",
			name,
			description: params.description,
			systemPrompt: params.systemPrompt,
			...(params.tools !== undefined ? { tools: params.tools } : {}),
			...(params.spawns !== undefined ? { spawns: params.spawns } : {}),
			...(params.model !== undefined ? { model: params.model } : {}),
			expectedActive: params.expectedActive,
		});
		const discovery = await this.invalidateDiscovery();
		return {
			content: [{ type: "text", text: `Updated managed preset "${name}" (${file}).` }],
			details: {
				action: "update",
				name,
				evaluated: false,
				status: "unevaluated-active",
				discoveryInvalidated: discovery.invalidated,
			},
		};
	}

	/**
	 * Run the architect pattern for a generated preset: an isolated session
	 * (hasUI:false, minimal tool surface, detached prompt) that is disposed
	 * afterwards. Auto-learn stays off inside so generation never triggers
	 * recursive capture.
	 */
	private async runArchitect(brief: string): Promise<GeneratedPresetSpec> {
		if (presetArchitectRunner) {
			const raw = await presetArchitectRunner({
				brief,
				systemPrompt: [agentCreationArchitectPrompt],
				userPrompt: (request: string) => request,
			});
			return parseGeneratedPresetSpec(raw);
		}
		const { createAgentSession } = await import("../sdk");
		const settings = await this.session.settings.cloneForCwd(this.session.cwd);
		settings.override("autolearn.enabled", false);
		const model = this.session.getActiveModel?.();
		if (!model) throw new Error("No active model is available to generate a preset.");
		const { session } = await createAgentSession({
			cwd: this.session.cwd,
			settings,
			model,
			systemPrompt: [agentCreationArchitectPrompt],
			hasUI: false,
			enableLsp: false,
			enableMCP: false,
			disableExtensionDiscovery: true,
			toolNames: ["__none__"],
			restrictToolNames: true,
			customTools: [],
			skills: [],
			contextFiles: [],
			promptTemplates: [],
			slashCommands: [],
		});
		try {
			const { prompt } = await import("@harvest/pi-utils");
			const userPrompt = prompt.render(agentCreationUserPrompt, { request: brief });
			await session.prompt(userPrompt, { expandPromptTemplates: false });
			const messages = session.state.messages;
			for (let i = messages.length - 1; i >= 0; i--) {
				const message = messages[i];
				if (message?.role !== "assistant" || !Array.isArray(message.content)) continue;
				const text = message.content
					.map(block =>
						block && typeof block === "object" && "type" in block && block.type === "text" && "text" in block
							? String(block.text)
							: "",
					)
					.join("\n")
					.trim();
				if (text) return parseGeneratedPresetSpec(text);
			}
			throw new Error("The preset architect returned no response.");
		} finally {
			await session.dispose();
		}
	}

	private async evaluatePreset(params: PresetsParams, signal?: AbortSignal): Promise<AgentToolResult> {
		const name = this.requireName(params);
		if (!params.task || !params.expectedOutcome) {
			throw new Error(`"evaluate" requires both "task" and "expectedOutcome".`);
		}
		requirePresetAutoImprovementBudget(this.session, "evaluation");
		const revId = await this.resolveTargetRevision(name, params.revisionId);
		// Fuse the tool call signal with parent-disposal linkage and register
		// before the run starts (mirror of the skill side). Cancellation
		// records nothing and never promotes.
		const evalSignal = createToolEvalSignal(this.session, signal);
		try {
			const run = evaluatePresetRevision(name, revId, {
				task: params.task,
				expectedOutcome: params.expectedOutcome,
				parent: this.session,
				signal: evalSignal.signal,
			});
			const { passed, summary } = await evalSignal.track(run);
			return {
				content: [
					{
						type: "text",
						text:
							`Evaluation ${passed ? "passed" : "FAILED"} for preset "${name}" revision ${revId}: ${summary}` +
							(passed
								? ` Promote to activate.`
								: ` Failed evaluations block promotion; fix the content and draft a new revision.`),
					},
				],
				details: { action: "evaluate", name, revisionId: revId, passed },
			};
		} finally {
			evalSignal.release();
		}
	}

	private async promotePreset(params: PresetsParams): Promise<AgentToolResult> {
		const name = this.requireName(params);
		const revId = await this.resolveTargetRevision(name, params.revisionId);
		const { path: file, disclosedUnevaluated } = await promotePresetRevision(name, revId, {
			discloseUnevaluated: params.discloseUnevaluated,
		});
		const discovery = await this.invalidateDiscovery();
		const suffix = disclosedUnevaluated
			? ` Activated unevaluated content explicitly (no evaluations recorded yet).`
			: ``;
		return {
			content: [
				{ type: "text", text: `Promoted managed preset "${name}" to revision ${revId} (${file}).${suffix}` },
			],
			details: {
				action: "promote",
				name,
				revisionId: revId,
				disclosedUnevaluated,
				discoveryInvalidated: discovery.invalidated,
			},
		};
	}

	private async rollbackPreset(params: PresetsParams): Promise<AgentToolResult> {
		const name = this.requireName(params);
		if (!params.revisionId) {
			throw new Error(`"rollback" requires "revisionId". Use action "inspect" to see candidates.`);
		}
		const cleanRev = validatePresetRevisionId(params.revisionId);
		const { path: file } = await rollbackPresetRevision(name, cleanRev);
		const discovery = await this.invalidateDiscovery();
		return {
			content: [{ type: "text", text: `Rolled back managed preset "${name}" to revision ${cleanRev} (${file}).` }],
			details: {
				action: "rollback",
				name,
				revisionId: cleanRev,
				discoveryInvalidated: discovery.invalidated,
			},
		};
	}

	private async deletePreset(params: PresetsParams): Promise<AgentToolResult> {
		const name = this.requireName(params);
		await deleteManagedPreset(name);
		const discovery = await this.invalidateDiscovery();
		return {
			content: [{ type: "text", text: `Deleted managed preset "${name}".` }],
			details: { action: "delete", name, discoveryInvalidated: discovery.invalidated },
		};
	}

	private async resolveTargetRevision(name: string, revisionId?: string): Promise<string> {
		if (revisionId) return validatePresetRevisionId(revisionId);
		const { active, revisions } = await listPresetRevisions(name);
		const drafts = revisions.filter(rev => rev.state === "draft" && rev.id !== active);
		const latest = drafts.length > 0 ? drafts[drafts.length - 1] : revisions[revisions.length - 1];
		if (!latest) throw new Error(`Preset "${name}" has no revisions. Create one first.`);
		return latest.id;
	}

	/**
	 * Invalidate agent discovery after a save so the new definition is
	 * picked up: clears both the create-time memo and published snapshots
	 * (via the owner-owned `refreshAgentDiscovery`). Best-effort: the file
	 * save already succeeded, and catalog pickup retries on the next load —
	 * a discovery failure is surfaced, never fatal.
	 */
	private async invalidateDiscovery(): Promise<{ invalidated: boolean; reason?: string }> {
		try {
			const { refreshAgentDiscovery } = await import("../task/index");
			await refreshAgentDiscovery(this.session.cwd, this.session.effectiveExtensionRoots?.());
			return { invalidated: true };
		} catch (err) {
			const reason = err instanceof Error ? err.message : String(err);
			return { invalidated: false, reason };
		}
	}

	/**
	 * Save → discover → spawn round-trip check for a managed preset: the
	 * materialized file parses under the agent contract, merges into
	 * discovery (managed loses to authored on collision), and satisfies the
	 * session spawn policy. Execution itself still goes through task/Eval
	 * policy; this validates routability, it does not spawn.
	 */
	async checkSpawnRoutability(name: string): Promise<{ routable: boolean; reason: string }> {
		const safe = sanitizePresetName(name);
		const managed = await discoverManagedPresets();
		const definition = managed.find(agent => agent.name === safe);
		if (!definition) return { routable: false, reason: `preset "${safe}" has no materialized managed file` };
		const { agents } = await discoverAgents(this.session.cwd).catch(() => ({ agents: [] as never[] }));
		if (isPresetNameClaimedByAuthored(safe, agents)) {
			return { routable: false, reason: `preset "${safe}" is shadowed by an authored/discovered agent` };
		}
		const spawnPolicy = resolveSpawnPolicy(this.session.getSessionSpawns());
		if (!spawnPolicy.enabled) {
			return { routable: false, reason: "spawning is disabled for this session" };
		}
		if (spawnPolicy.allowedAgents !== null && !spawnPolicy.allowedAgents.includes(safe)) {
			return {
				routable: false,
				reason: `spawn policy allows ${spawnPolicy.allowedErrorText}; "${safe}" is not included`,
			};
		}
		const active = await readActivePresetRevision(safe).catch(() => undefined);
		return {
			routable: true,
			reason: `preset "${safe}" parses, is undisputed, and satisfies the spawn policy${active ? ` (active ${active.id})` : ""}`,
		};
	}
}
