/**
 * Fullscreen /agents hub, shown on the alternate screen like /models.
 *
 * Layout mirrors the model hub: a sidebar of scopes (All agents, per-source
 * groups, "+ New agent"), a body listing agents with type-to-filter search,
 * and a footer that turns into a chip strip while configuring. Enter on an
 * agent opens its property strip (enabled / model / prewalk / advisor); a
 * property opens a value strip whose "pick model…" chip dives into the real
 * ModelBrowser and whose "pattern…" chip opens an inline pattern input, so
 * every per-agent knob is picked instead of memorized.
 */

import * as fs from "node:fs/promises";
import * as path from "node:path";
import type { AgentMessage } from "@harvest/pi-agent-core";
import {
	type Component,
	Editor,
	fuzzyMatch,
	Input,
	matchesKey,
	replaceTabs,
	routeSgrMouseInput,
	ScrollView,
	type SgrMouseEvent,
	type TUI,
	visibleWidth,
	wrapTextWithAnsi,
} from "@harvest/pi-tui";
import { isEnoent, prompt, sanitizeText } from "@harvest/pi-utils";
import { YAML } from "bun";
import type { EffectiveExtensionRoots } from "../../capability/types";
import { getConfigDirs } from "../../config";
import type { ModelRegistry } from "../../config/model-registry";
import {
	resolveAgentAdvisorSelection,
	resolveAgentModelPatterns,
	resolveAgentPrewalkPattern,
	resolveConfiguredModelPatterns,
	resolveModelOverride,
} from "../../config/model-resolver";
import type { Settings } from "../../config/settings";
import agentCreationArchitectPrompt from "../../prompts/system/agent-creation-architect.md" with { type: "text" };
import agentCreationUserPrompt from "../../prompts/system/agent-creation-user.md" with { type: "text" };
import { createAgentSession } from "../../sdk";
import { EVAL_DEFAULT_MODEL_PATTERN } from "../../autolearn/eval-executor";
import { refreshAgentDiscovery } from "../../task";
import {
	createPresetDraft,
	evaluatePresetRevision,
	getManagedPresetsDir,
	isPresetNameClaimedByAuthored,
	isValidPresetName,
	listManagedPresetNames,
	listPresetRevisions,
	loadBundledAgents,
	parseAgent,
	promotePresetRevision,
	rollbackPresetRevision,
	PRESET_IDENTIFIER_PATTERN as IDENTIFIER_PATTERN,
} from "../../task/agents";
import { discoverAgents } from "../../task/discovery";
import { resolveAgentPrewalkDefault } from "../../task/prewalk";
import type { ToolSession } from "../../tools/index";
import type { AgentDefinition, AgentSource } from "../../task/types";
import { shortenPath } from "../../tools/render-utils";
import { getEditorTheme, getSymbolTheme, theme } from "../theme/theme";
import {
	matchesAppFollowUp,
	matchesSelectCancel,
	matchesSelectDown,
	matchesSelectUp,
} from "../utils/keybinding-matchers";
import { buildBrowserItems, ModelBrowser, type ModelBrowserItem, sortModelItems } from "./model-browser";
import { canSplitPane, dialogContentWidth, fit, renderDialog, row, splitBodyWidth, splitRow } from "./overlay-box";
import { formatEvaluationLines, formatPromotedNotice, revisionEvalStatus } from "./revision-views";

/** One agent with its per-agent settings overrides resolved for display. */
interface HubAgent extends AgentDefinition {
	disabled: boolean;
	/** `task.agentModelOverrides[name]` as a comma-joined pattern list. */
	overrideModel?: string;
	/** `task.agentPrewalk[name]`: "on", "off", or a model pattern. */
	prewalkOverride?: string;
	/** `task.agentAdvisor[name]`: "on", "off", or a model pattern. */
	advisorOverride?: string;
	/**
	 * True for rows synthesized from revision history for presets with no
	 * materialized file (unevaluated/inactive drafts). Spawn discovery is
	 * file-scan only and never lists these — the hub enumerates them through
	 * the revision service so the revision manager stays reachable. Drafts
	 * never execute: nothing outside the revision manager consumes these rows.
	 */
	managedDraftOnly?: boolean;
}

const SOURCE_LABEL: Record<AgentSource, string> = {
	project: "Project",
	user: "User",
	bundled: "Bundled",
};
const SOURCE_ORDER: Record<AgentSource, number> = { project: 0, user: 1, bundled: 2 };

interface SidebarEntry {
	id: string;
	kind: "all" | "source" | "new" | "separator";
	label: string;
	source?: AgentSource;
	annotation?: string;
}

/** A body row of the agent list: an agent or the trailing "+ New agent…". */
type ListRow = { kind: "agent"; agent: HubAgent } | { kind: "new" };

/** The per-agent knob a strip or the model browser is editing. */
type PropertyKind = "model" | "prewalk" | "advisor";

interface StripChip {
	label: string;
	styled: string;
	action:
		| { kind: "toggle" }
		| { kind: "property"; property: PropertyKind }
		| { kind: "set"; property: PropertyKind; value: string | undefined }
		| { kind: "pick"; property: PropertyKind }
		| { kind: "pattern"; property: PropertyKind }
		| { kind: "revisions" }
		| { kind: "rev-inspect"; revId: string }
		| { kind: "rev-evaluate"; revId: string }
		| { kind: "rev-cancel-eval"; revId: string }
		| { kind: "rev-promote"; revId: string }
		| { kind: "rev-rollback"; revId: string };
}

type StripState =
	| { kind: "chips"; agent: HubAgent; property?: PropertyKind; chips: StripChip[]; index: number }
	| { kind: "pattern"; agent: HubAgent; property: PropertyKind; input: Input };

/** Recorded chip hit-range on the footer row (columns relative to frame col 0). */
interface ChipRange {
	start: number;
	end: number;
	index: number;
}

interface GeneratedAgentSpec {
	identifier: string;
	whenToUse: string;
	systemPrompt: string;
}

/**
 * Override for the /agents hub creation architect (tests inject a
 * deterministic fixture; production runs the isolated session below).
 * The runner always receives runtime-only settings — a clone of the shared
 * instance with recursive capture disabled — never the shared instance, so
 * generation leaves persisted configuration bytes untouched on success and
 * failure alike.
 */
export type AgentsHubArchitectRunner = (input: { brief: string; settings: Settings }) => Promise<GeneratedAgentSpec>;

let agentsHubArchitectRunner: AgentsHubArchitectRunner | undefined;

/** Override the hub creation architect (tests inject deterministic fixtures). */
export function setAgentsHubArchitectRunner(runner: AgentsHubArchitectRunner | undefined): void {
	agentsHubArchitectRunner = runner;
}

/** One managed-preset revision as the hub revision manager displays it. */
export interface PresetRevisionView {
	id: string;
	state: string;
	active: boolean;
	evaluations: number;
	evalStatus: "unevaluated" | "passing" | "failing";
}

/** True for agents materialized from the isolated managed-presets store.
 * Authoritative containment against the managed-presets root the service
 * writes — never a path substring, so sibling directories whose names merely
 * contain "managed-presets" (backups, archives) are not mistaken for managed
 * artifacts, and synthetic draft-only rows (whose filePath is the managed
 * path the draft would materialize at) resolve identically. */
export function isManagedPresetAgent(agent: { filePath?: string }): boolean {
	const filePath = agent.filePath;
	if (!filePath) return false;
	try {
		const root = path.resolve(getManagedPresetsDir());
		const resolved = path.resolve(filePath);
		const rel = path.relative(root, resolved);
		return rel !== "" && !rel.startsWith("..") && !path.isAbsolute(rel);
	} catch {
		return false;
	}
}

/**
 * Managed presets with revision history but no materialized file
 * (unevaluated/inactive drafts): file-scan spawn discovery never lists them,
 * so the hub enumerates them here through the revision service and
 * synthesizes management rows. Names already present in spawn discovery
 * (`known`) and names without history are skipped — authored/bundled
 * definitions always win and legacy files without history stay on the
 * discovery path.
 */
async function enumerateDraftOnlyManagedPresets(
	known: Set<string>,
): Promise<Array<{ name: string; agent: AgentDefinition }>> {
	const candidates = new Set<string>();
	try {
		const files = await fs.readdir(getManagedPresetsDir()).catch((err: unknown) => {
			if (isEnoent(err)) return [];
			throw err;
		});
		for (const entry of files) {
			if (!entry.endsWith(".md") || entry.endsWith(".txn.json") || entry.endsWith(".tmp")) continue;
			const stem = entry.slice(0, -".md".length);
			if (isValidPresetName(stem) && !known.has(stem)) candidates.add(stem);
		}
	} catch {
		// Best-effort: an unreadable managed dir leaves discovery rows intact.
	}
	// Draft-only names come from the authoritative revision service (U1),
	// never a mirrored store path: per-name verification below still runs
	// through `listPresetRevisions`, so stray directories never surface.
	try {
		for (const name of await listManagedPresetNames()) {
			if (!known.has(name)) candidates.add(name);
		}
	} catch {
		// Best-effort: same as above.
	}
	const out: Array<{ name: string; agent: AgentDefinition }> = [];
	for (const name of [...candidates].sort()) {
		let revisions: Array<{ id: string; description: string; content: string }>;
		try {
			({ revisions } = await listPresetRevisions(name));
		} catch {
			continue;
		}
		if (revisions.length === 0) continue;
		const latest = revisions[revisions.length - 1];
		if (!latest) continue;
		let description = latest.description;
		let systemPrompt = "";
		try {
			const parsed = parseAgent(`managed:${name}`, latest.content, "user", "warn");
			description = parsed.description;
			systemPrompt = parsed.systemPrompt;
		} catch {
			systemPrompt = latest.content;
		}
		out.push({
			name,
			agent: {
				name,
				description,
				systemPrompt,
				source: "user",
				filePath: path.join(getManagedPresetsDir(), `${name}.md`),
			},
		});
	}
	return out;
}

/** Ambient model context for resolution previews and the creation architect. */
export interface AgentsHubModelContext {
	modelRegistry?: ModelRegistry;
	activeModelPattern?: string;
	defaultModelPattern?: string;
	/**
	 * Live provider for the owning session's extension roots (explicit + mode +
	 * configured). Supplied so `/agents` lists exactly the agents the session
	 * can spawn — including `--extension` roots and honoring `explicit-only`.
	 * Falls back to a settings-only merge struct when absent.
	 */
	extensionRoots?: () => EffectiveExtensionRoots;
}

export interface AgentsHubCallbacks {
	onCancel: () => void;
}

/**
 * Production evaluation context for managed-preset runs started from the hub.
 *
 * The revision service executes evaluations through real restricted task
 * execution and refuses to run without the owning session (model registry,
 * credential resolvers, cwd, settings, restrictions). The hub itself owns no
 * session, so the host must thread the owning production ToolSession (or an
 * equivalent trusted context) plus an AbortSignal here — see the patch
 * request on `showAgentsDashboard`: pass the live session's ToolSession and a
 * signal aborted when the hub closes. Without a parent, evaluations surface
 * as unavailable instead of running unscoped; tests inject a structural
 * parent plus deterministic transport.
 */
export interface AgentsHubEvalContext {
	/** Owning production session: model/registry/credentials/restrictions source. */
	parent?: ToolSession;
	/** Caller abort: aborts the run, records nothing, never passes/promotes. */
	signal?: AbortSignal;
}

const SIDEBAR_MIN_WIDTH = 16;
const SIDEBAR_MAX_WIDTH = 24;

function extractAssistantText(messages: AgentMessage[]): string | null {
	for (let i = messages.length - 1; i >= 0; i--) {
		const message = messages[i];
		if (message?.role !== "assistant") continue;
		const blocks = message.content;
		if (!Array.isArray(blocks)) continue;
		const text = blocks
			.map(block => {
				if (!block || typeof block !== "object") return "";
				if (!("type" in block) || block.type !== "text" || !("text" in block)) return "";
				const value = block.text;
				return typeof value === "string" ? value : "";
			})
			.join("\n")
			.trim();
		if (text.length > 0) return text;
	}
	return null;
}

function extractJsonObject(raw: string): string {
	const fenceMatch = raw.match(/```(?:json)?\s*([\s\S]*?)```/i);
	if (fenceMatch?.[1]) return fenceMatch[1].trim();
	const start = raw.indexOf("{");
	const end = raw.lastIndexOf("}");
	if (start >= 0 && end >= start) return raw.slice(start, end + 1).trim();
	return raw.trim();
}

function parseGeneratedAgentSpec(raw: string): GeneratedAgentSpec {
	const parsed = JSON.parse(extractJsonObject(raw)) as Partial<GeneratedAgentSpec>;
	if (!parsed || typeof parsed !== "object") {
		throw new Error("Model output is not a JSON object");
	}
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
	if (!IDENTIFIER_PATTERN.test(identifier)) {
		throw new Error("Generated identifier is invalid (must be lowercase kebab-case, 2+ words)");
	}
	if (!whenToUse.toLowerCase().startsWith("use this agent when")) {
		throw new Error("Generated whenToUse must start with 'Use this agent when...'");
	}
	if (!systemPrompt) {
		throw new Error("Generated systemPrompt is empty");
	}
	return { identifier, whenToUse, systemPrompt };
}

function matchAgent(agent: HubAgent, query: string): boolean {
	const text = `${agent.name} ${agent.description} ${SOURCE_LABEL[agent.source]} ${agent.overrideModel ?? ""}`;
	return query
		.trim()
		.split(/\s+/)
		.every(token => fuzzyMatch(token, text).matches);
}

/**
 * The fullscreen agents hub component. Hosted via
 * `ui.showOverlay(..., { fullscreen: true })`; the host must call
 * {@link AgentsHubComponent.dispose} when the overlay closes.
 */
export class AgentsHubComponent implements Component {
	#tui: TUI;
	#cwd: string;
	#settings: Settings;
	#modelContext: AgentsHubModelContext;
	#callbacks: AgentsHubCallbacks;
	#evalContext: AgentsHubEvalContext;

	#allAgents: HubAgent[] = [];
	#entries: SidebarEntry[] = [];
	#activeEntryId = "all";
	#sidebarScroll = 0;
	#focus: "scope" | "list" = "list";

	#rows: ListRow[] = [];
	#rowIndex = 0;
	#rowHover: number | null = null;
	#listScroll = 0;
	#searchQuery = "";
	#notice: string | null = null;
	#loadError: string | null = null;

	#strip: StripState | null = null;
	#chipRanges: ChipRange[] = [];
	/** Non-null while the body shows the model browser for one agent property. */
	#assigning: { agent: HubAgent; property: PropertyKind } | null = null;
	#browser: ModelBrowser;

	// Create flow (AI-generated agent definition).
	#createInput: Editor | null = null;
	#createDescription = "";
	#createScope: "project" | "user" | "managed" = "project";
	#createGenerating = false;
	#createSpec: GeneratedAgentSpec | null = null;
	#createError: string | null = null;
	#createStreamingText = "";
	#createReview = new ScrollView([], {
		height: 1,
		theme: { track: text => theme.fg("dim", text), thumb: text => theme.fg("accent", text) },
	});
	#createReviewLayout: { spec: GeneratedAgentSpec; width: number; scope: string; error: string | null } | undefined;

	// Managed-preset revision management (isolated managed-presets store only;
	// authored/bundled agents never enter this flow).
	#managingRevisions: {
		agent: HubAgent;
		items: PresetRevisionView[];
		active: string | null;
		index: number;
		scroll: number;
		error: string | null;
	} | null = null;
	#presetEvalInput: {
		agent: HubAgent;
		revId: string;
		step: "task" | "outcome";
		task: string;
		input: Input;
	} | null = null;
	/** In-flight preset evaluation, if any (single-flight per hub). */
	#presetEvalRunning: {
		agentName: string;
		revId: string;
		/** Fence token: late callbacks from a closed manager or a superseded run are dropped. */
		generation: number;
		cancel: () => void;
	} | null = null;
	/** Bumped on manager close/switch, hub dispose, and every new eval start. */
	#evalGeneration = 0;
	/** Revision under inspection (read-only detail: content, evals, restrictions). */
	#inspectingRevision: { agent: HubAgent; revId: string; error: string | null; body: string[] } | null = null;
	#inspectionView = new ScrollView([], {
		height: 1,
		theme: { track: text => theme.fg("dim", text), thumb: text => theme.fg("accent", text) },
	});
	#inspectionLayout: { body: readonly string[]; width: number } | undefined;
	/** Last fallible hub-action failure, rendered distinctly from notices. */
	#actionError: string | null = null;

	// Frame geometry from the last render, for mouse hit-testing.
	#contentRowStart = 1;
	#contentRowCount = 0;
	#sidebarWidthLast = SIDEBAR_MIN_WIDTH;
	#footerRow = 0;
	/** First agent-list row's offset in body-line coordinates (after the status row). */
	#listRowStart = 2;
	#maxHeight: number | undefined;
	#splitVisible = false;
	#scopeOnly = false;
	#bodyHeaderRows = 0;
	#contentInset = 2;

	private constructor(
		tui: TUI,
		cwd: string,
		settings: Settings,
		modelContext: AgentsHubModelContext,
		callbacks: AgentsHubCallbacks,
		evalContext: AgentsHubEvalContext = {},
	) {
		this.#tui = tui;
		this.#cwd = cwd;
		this.#settings = settings;
		this.#modelContext = modelContext;
		this.#callbacks = callbacks;
		this.#evalContext = evalContext;
		this.#browser = new ModelBrowser(settings, {
			emptyText: () => "  No models available — configure a provider in /models first.",
		});
		this.#browser.setShowProvider(true);
		this.#browser.onActivate = item => this.#commitPickedModel(item);
		this.#browser.onCancel = () => this.#cancelAssign();
	}

	static async create(
		tui: TUI,
		cwd: string,
		settings: Settings,
		modelContext: AgentsHubModelContext = {},
		callbacks: AgentsHubCallbacks = { onCancel: () => {} },
		evalContext: AgentsHubEvalContext = {},
	): Promise<AgentsHubComponent> {
		const hub = new AgentsHubComponent(tui, cwd, settings, modelContext, callbacks, evalContext);
		await hub.#reload();
		return hub;
	}

	dispose(): void {
		// Fence late evaluation callbacks: a run settling after close must
		// neither reopen the manager nor rewrite notices on a dead view.
		this.#evalGeneration++;
		this.#presetEvalRunning = null;
	}
	invalidate(): void {}
	setMaxHeight(height: number): void {
		this.#maxHeight = Math.max(1, Math.floor(height));
	}

	/** Live extension roots for the owning session; settings-only merge fallback when no provider. */
	#extensionRoots(): EffectiveExtensionRoots {
		return (
			this.#modelContext.extensionRoots?.() ?? {
				explicit: [],
				mode: "merge",
				configured: this.#settings.get("extensions") ?? [],
				configuredLevel: this.#settings.extensionsSourceLevel(),
			}
		);
	}

	// ═══════════════════════════════════════════════════════════════════════
	// Data pipeline
	// ═══════════════════════════════════════════════════════════════════════

	async #reload(): Promise<void> {
		this.#loadError = null;
		try {
			const selectedName = this.#selectedAgent()?.name;
			const { agents } = await discoverAgents(this.#cwd, undefined, this.#extensionRoots());
			const disabled = new Set(this.#settings.get("task.disabledAgents") ?? []);
			const overrides = this.#settings.get("task.agentModelOverrides") ?? {};
			const prewalkOverrides = this.#settings.get("task.agentPrewalk") ?? {};
			const advisorOverrides = this.#settings.get("task.agentAdvisor") ?? {};
			const toHubAgent = (agent: AgentDefinition): HubAgent => {
				const override = overrides[agent.name];
				const overrideModel = (Array.isArray(override) ? override.join(",") : (override ?? "")).trim();
				return {
					...agent,
					disabled: disabled.has(agent.name),
					overrideModel: overrideModel || undefined,
					prewalkOverride: prewalkOverrides[agent.name]?.trim() || undefined,
					advisorOverride: advisorOverrides[agent.name]?.trim() || undefined,
				};
			};
			const discovered = agents.map(toHubAgent);
			// Draft-only managed presets have revision history but no
			// materialized file, so file-scan spawn discovery never lists
			// them. Enumerate them through the revision service and synthesize
			// management rows — otherwise a freshly saved draft is unreachable
			// (no row, no revision manager) until it is promoted. Authored and
			// bundled definitions always win on collision and are never
			// shadowed here.
			const known = new Set(discovered.map(agent => agent.name));
			const bundledNames = new Set(loadBundledAgents().map(agent => agent.name));
			for (const draft of await enumerateDraftOnlyManagedPresets(known)) {
				if (bundledNames.has(draft.name)) continue;
				discovered.push({ ...toHubAgent(draft.agent), managedDraftOnly: true });
			}
			this.#allAgents = discovered.slice().sort((a, b) => {
				const sourceCmp = SOURCE_ORDER[a.source] - SOURCE_ORDER[b.source];
				if (sourceCmp !== 0) return sourceCmp;
				return a.name.localeCompare(b.name);
			});
			this.#buildSidebar();
			this.#buildRows();
			if (selectedName) {
				const index = this.#rows.findIndex(r => r.kind === "agent" && r.agent.name === selectedName);
				if (index >= 0) this.#rowIndex = index;
			}
			this.#clampRowIndex();
		} catch (error) {
			this.#allAgents = [];
			this.#buildSidebar();
			this.#buildRows();
			this.#loadError = error instanceof Error ? error.message : String(error);
		}
		this.#tui.requestRender();
	}

	#buildSidebar(): void {
		const counts: Record<AgentSource, number> = { project: 0, user: 0, bundled: 0 };
		for (const agent of this.#allAgents) counts[agent.source]++;
		const entries: SidebarEntry[] = [
			{ id: "all", kind: "all", label: "All agents", annotation: String(this.#allAgents.length) },
		];
		const sources = (["project", "user", "bundled"] as const).filter(source => counts[source] > 0);
		if (sources.length > 0) {
			entries.push({ id: "sep:sources", kind: "separator", label: "" });
			for (const source of sources) {
				entries.push({
					id: `source:${source}`,
					kind: "source",
					label: SOURCE_LABEL[source],
					source,
					annotation: String(counts[source]),
				});
			}
		}
		entries.push({ id: "sep:actions", kind: "separator", label: "" });
		entries.push({ id: "new", kind: "new", label: "New agent" });
		this.#entries = entries;
		if (!entries.some(entry => entry.id === this.#activeEntryId)) this.#activeEntryId = "all";
	}

	#activeEntry(): SidebarEntry {
		return this.#entries.find(entry => entry.id === this.#activeEntryId) ?? this.#entries[0];
	}

	#buildRows(): void {
		const entry = this.#activeEntry();
		const scoped =
			entry.kind === "source" ? this.#allAgents.filter(agent => agent.source === entry.source) : this.#allAgents;
		const filtered = this.#searchQuery ? scoped.filter(agent => matchAgent(agent, this.#searchQuery)) : scoped;
		this.#rows = [...filtered.map(agent => ({ kind: "agent", agent }) as ListRow), { kind: "new" }];
	}

	#clampRowIndex(): void {
		this.#rowIndex = Math.max(0, Math.min(this.#rowIndex, this.#rows.length - 1));
	}

	#selectedAgent(): HubAgent | undefined {
		const rowDef = this.#rows[this.#rowIndex];
		return rowDef?.kind === "agent" ? rowDef.agent : undefined;
	}

	// ═══════════════════════════════════════════════════════════════════════
	// Effective per-agent values
	// ═══════════════════════════════════════════════════════════════════════

	#effectiveModelPatterns(agent: HubAgent): string[] {
		return resolveAgentModelPatterns({
			settingsOverride: agent.overrideModel,
			agentModel: agent.model,
			settings: this.#settings,
			activeModelPattern: this.#modelContext.activeModelPattern,
			fallbackModelPattern: this.#modelContext.defaultModelPattern,
		});
	}

	#resolvePatterns(patterns: string[]): string | undefined {
		const registry = this.#modelContext.modelRegistry;
		if (!registry || patterns.length === 0) return undefined;
		const { model, thinkingLevel, explicitThinkingLevel } = resolveModelOverride(patterns, registry, this.#settings);
		if (!model) return undefined;
		const level = explicitThinkingLevel && thinkingLevel ? `:${thinkingLevel}` : "";
		return `${model.provider}/${model.id}${level}`;
	}

	#effectivePrewalkPattern(agent: HubAgent): string | undefined {
		return resolveAgentPrewalkPattern({
			settingsOverride: agent.prewalkOverride,
			agentPrewalk: resolveAgentPrewalkDefault(agent, this.#settings.get("task.prewalk") ?? false),
		});
	}

	#effectiveAdvisorPattern(agent: HubAgent): string | undefined {
		const selection = resolveAgentAdvisorSelection({
			settingsOverride: agent.advisorOverride,
			agentAdvisor: agent.advisor,
		});
		return selection ? (selection.model ?? "@advisor") : undefined;
	}

	// ═══════════════════════════════════════════════════════════════════════
	// Mutations
	// ═══════════════════════════════════════════════════════════════════════

	#toggleAgent(agent: HubAgent): void {
		agent.disabled = !agent.disabled;
		const disabled = this.#allAgents
			.filter(entry => entry.disabled)
			.map(entry => entry.name)
			.sort((a, b) => a.localeCompare(b));
		this.#settings.set("task.disabledAgents", disabled);
		this.#notice = `${agent.name} ${agent.disabled ? "disabled" : "enabled"}`;
		this.#tui.requestRender();
	}

	#persistRecord(property: PropertyKind): void {
		const overrides: Record<string, string> = {};
		for (const agent of this.#allAgents) {
			const value = this.#overrideFor(agent, property)?.trim();
			if (value) overrides[agent.name] = value;
		}
		const key =
			property === "model"
				? "task.agentModelOverrides"
				: property === "prewalk"
					? "task.agentPrewalk"
					: "task.agentAdvisor";
		this.#settings.set(key, overrides);
	}

	#overrideFor(agent: HubAgent, property: PropertyKind): string | undefined {
		switch (property) {
			case "model":
				return agent.overrideModel;
			case "prewalk":
				return agent.prewalkOverride;
			case "advisor":
				return agent.advisorOverride;
		}
	}

	#setOverride(agent: HubAgent, property: PropertyKind, value: string | undefined): void {
		const trimmed = value?.trim() || undefined;
		switch (property) {
			case "model":
				agent.overrideModel = trimmed;
				break;
			case "prewalk":
				agent.prewalkOverride = trimmed;
				break;
			case "advisor":
				agent.advisorOverride = trimmed;
				break;
		}
		this.#persistRecord(property);
		this.#notice = this.#describeProperty(agent, property);
		this.#tui.requestRender();
	}

	/** One-line effective description used for notices and the status row. */
	#describeProperty(agent: HubAgent, property: PropertyKind): string {
		switch (property) {
			case "model": {
				const patterns = this.#effectiveModelPatterns(agent);
				const resolved = this.#resolvePatterns(patterns);
				const base = agent.overrideModel ?? (patterns.length > 0 ? patterns.join(",") : "session model");
				return `${agent.name} model: ${base}${resolved ? ` ${theme.getSymbolPreset() === "ascii" ? "->" : "→"} ${resolved}` : ""}`;
			}
			case "prewalk": {
				const pattern = this.#effectivePrewalkPattern(agent);
				return `${agent.name} prewalk: ${pattern ? `on (${pattern})` : "off"}`;
			}
			case "advisor": {
				const pattern = this.#effectiveAdvisorPattern(agent);
				return `${agent.name} advisor: ${pattern ? `on (${pattern})` : "off"}`;
			}
		}
	}

	// ═══════════════════════════════════════════════════════════════════════
	// Strips
	// ═══════════════════════════════════════════════════════════════════════

	#propertySummary(agent: HubAgent, property: PropertyKind): string {
		switch (property) {
			case "model":
				return agent.overrideModel ?? "auto";
			case "prewalk": {
				const pattern = this.#effectivePrewalkPattern(agent);
				return pattern ?? "off";
			}
			case "advisor": {
				const pattern = this.#effectiveAdvisorPattern(agent);
				return pattern ?? "off";
			}
		}
	}

	/** Level-1 strip: pick which knob of `agent` to change. */
	#openAgentStrip(agent: HubAgent): void {
		const enabledChip: StripChip = {
			label: agent.disabled ? "enable" : "disable",
			styled: agent.disabled
				? theme.fg("success", `${theme.status.enabled} enable`)
				: theme.fg("dim", `${theme.status.disabled} disable`),
			action: { kind: "toggle" },
		};
		const propertyChip = (property: PropertyKind): StripChip => {
			const summary = this.#propertySummary(agent, property);
			return {
				label: property,
				styled: `${theme.fg("accent", property)}${theme.fg("dim", `: ${summary}`)}`,
				action: { kind: "property", property },
			};
		};
		this.#strip = {
			kind: "chips",
			agent,
			chips: [
				enabledChip,
				propertyChip("model"),
				propertyChip("prewalk"),
				propertyChip("advisor"),
				...(isManagedPresetAgent(agent)
					? [
							{
								label: `revisions${theme.symbol("sep.ellipsis")}`,
								styled: theme.fg("accent", `revisions${theme.symbol("sep.ellipsis")}`),
								action: { kind: "revisions" as const },
							} satisfies StripChip,
						]
					: []),
			],
			index: 1,
		};
	}

	/** Level-2 strip: value choices for one property of `agent`. */
	#openPropertyStrip(agent: HubAgent, property: PropertyKind): void {
		const current = this.#overrideFor(agent, property)?.toLowerCase();
		const chips: StripChip[] = [];
		const mark = (label: string, active: boolean, color: "accent" | "muted" = "muted"): string =>
			active ? theme.fg("accent", `${theme.status.enabled} ${label}`) : theme.fg(color, label);
		if (property === "model") {
			chips.push({
				label: `pick model${theme.symbol("sep.ellipsis")}`,
				styled: theme.fg("accent", `pick model${theme.symbol("sep.ellipsis")}`),
				action: { kind: "pick", property },
			});
			chips.push({
				label: `pattern${theme.symbol("sep.ellipsis")}`,
				styled: theme.fg("muted", `pattern${theme.symbol("sep.ellipsis")}`),
				action: { kind: "pattern", property },
			});
			if (agent.overrideModel) {
				chips.push({
					label: "clear override",
					styled: theme.fg("warning", "clear override"),
					action: { kind: "set", property, value: undefined },
				});
			}
		} else {
			chips.push({
				label: "agent default",
				styled: mark("agent default", current === undefined),
				action: { kind: "set", property, value: undefined },
			});
			chips.push({
				label: "on",
				styled: mark("on", current === "on"),
				action: { kind: "set", property, value: "on" },
			});
			chips.push({
				label: "off",
				styled: mark("off", current === "off"),
				action: { kind: "set", property, value: "off" },
			});
			chips.push({
				label: `pick model${theme.symbol("sep.ellipsis")}`,
				styled: theme.fg("accent", `pick model${theme.symbol("sep.ellipsis")}`),
				action: { kind: "pick", property },
			});
			chips.push({
				label: `pattern${theme.symbol("sep.ellipsis")}`,
				styled: theme.fg("muted", `pattern${theme.symbol("sep.ellipsis")}`),
				action: { kind: "pattern", property },
			});
		}
		this.#strip = { kind: "chips", agent, property, chips, index: 0 };
	}

	#openPatternStrip(agent: HubAgent, property: PropertyKind): void {
		const input = new Input();
		const current = this.#overrideFor(agent, property);
		if (current) input.setValue(current);
		this.#strip = { kind: "pattern", agent, property, input };
	}

	#closeStrip(): void {
		this.#strip = null;
		this.#chipRanges = [];
	}

	#activateStripChip(): void {
		const strip = this.#strip;
		if (strip?.kind !== "chips") return;
		const chip = strip.chips[strip.index];
		if (!chip) return;
		const action = chip.action;
		switch (action.kind) {
			case "toggle":
				this.#toggleAgent(strip.agent);
				this.#closeStrip();
				return;
			case "revisions":
				this.#closeStrip();
				void this.#openRevisionManager(strip.agent);
				return;
			case "rev-inspect":
				this.#closeStrip();
				void this.#inspectManagedRevision(strip.agent, action.revId);
				return;
			case "rev-evaluate":
				this.#closeStrip();
				this.#beginPresetEval(strip.agent, action.revId);
				return;
			case "rev-cancel-eval":
				this.#closeStrip();
				this.#cancelPresetEval(strip.agent, action.revId);
				return;
			case "rev-promote":
				this.#closeStrip();
				void this.#promoteManagedRevision(strip.agent, action.revId);
				return;
			case "rev-rollback":
				this.#closeStrip();
				void this.#rollbackManagedRevision(strip.agent, action.revId);
				return;
			case "property":
				this.#openPropertyStrip(strip.agent, action.property);
				return;
			case "set":
				this.#setOverride(strip.agent, action.property, action.value);
				this.#closeStrip();
				return;
			case "pick":
				this.#closeStrip();
				this.#startAssign(strip.agent, action.property);
				return;
			case "pattern":
				this.#openPatternStrip(strip.agent, action.property);
				return;
		}
	}

	#submitPattern(): void {
		const strip = this.#strip;
		if (strip?.kind !== "pattern") return;
		this.#setOverride(strip.agent, strip.property, strip.input.getValue());
		this.#closeStrip();
	}

	// ═══════════════════════════════════════════════════════════════════════
	// Model browser assign mode
	// ═══════════════════════════════════════════════════════════════════════

	#startAssign(agent: HubAgent, property: PropertyKind): void {
		const registry = this.#modelContext.modelRegistry;
		const models = registry?.getAvailable() ?? [];
		const items = buildBrowserItems(models);
		sortModelItems(items, { mruOrder: this.#settings.getStorage()?.getModelUsageOrder() ?? [] });
		this.#assigning = { agent, property };
		this.#browser.setItems(items);
		this.#browser.setQuery("");
		const current = this.#overrideFor(agent, property);
		if (current) this.#browser.selectSelector(current);
	}

	#commitPickedModel(item: ModelBrowserItem): void {
		const target = this.#assigning;
		if (!target) return;
		this.#assigning = null;
		this.#browser.setQuery("");
		this.#setOverride(target.agent, target.property, item.selector);
	}

	#cancelAssign(): void {
		this.#assigning = null;
		this.#browser.setQuery("");
		this.#tui.requestRender();
	}

	// ═══════════════════════════════════════════════════════════════════════
	// Create flow
	// ═══════════════════════════════════════════════════════════════════════

	get #createActive(): boolean {
		return this.#createInput !== null || this.#createGenerating || this.#createSpec !== null;
	}

	#beginCreateFlow(): void {
		if (this.#createGenerating) return;
		this.#createError = null;
		this.#createSpec = null;
		this.#createDescription = "";
		const editor = new Editor(getEditorTheme());
		editor.setBorderVisible(false);
		editor.setPromptGutter("> ");
		editor.setMaxHeight(Math.max(3, Math.min(8, this.#terminalRows() - 12)));
		editor.disableSubmit = true;
		editor.onChange = value => {
			this.#createDescription = value;
		};
		this.#createInput = editor;
		this.#tui.requestRender();
	}

	#clearCreateFlow(): void {
		this.#createInput = null;
		this.#createDescription = "";
		this.#createGenerating = false;
		this.#createSpec = null;
		this.#createError = null;
		this.#createStreamingText = "";
	}

	async #generateAgentFromDescription(rawDescription: string): Promise<void> {
		const description = rawDescription.trim();
		this.#createDescription = description;
		if (!description) {
			this.#createError = "Description is required.";
			this.#tui.requestRender();
			return;
		}
		this.#createGenerating = true;
		this.#createError = null;
		this.#createSpec = null;
		this.#createStreamingText = "";
		this.#tui.requestRender();
		try {
			// Runtime-only settings for the architect: a clone of the shared
			// instance with recursive capture disabled, so generation — success
			// or failure — never mutates persisted configuration.
			const runtimeSettings = await this.#settings.cloneForCwd(this.#cwd);
			runtimeSettings.override("autolearn.enabled", false);
			const spec = await this.#runAgentCreationArchitect(description, runtimeSettings);
			this.#createSpec = spec;
			this.#notice = null;
		} catch (error) {
			this.#createError = error instanceof Error ? error.message : String(error);
		} finally {
			this.#createGenerating = false;
			this.#tui.requestRender();
		}
	}

	async #runAgentCreationArchitect(description: string, runtimeSettings: Settings): Promise<GeneratedAgentSpec> {
		if (agentsHubArchitectRunner) {
			return agentsHubArchitectRunner({ brief: description, settings: runtimeSettings });
		}
		const modelRegistry = this.#modelContext.modelRegistry;
		if (!modelRegistry) {
			throw new Error("Model registry unavailable in current session.");
		}
		await modelRegistry.refresh();
		const modelPatterns = resolveConfiguredModelPatterns(
			this.#modelContext.activeModelPattern ??
				this.#modelContext.defaultModelPattern ??
				runtimeSettings.getModelRole("default"),
			runtimeSettings,
		);
		const { model } = resolveModelOverride(modelPatterns, modelRegistry, runtimeSettings);
		const selectedModel = model ?? modelRegistry.getAvailable()[0];
		if (!selectedModel) {
			throw new Error("No available model to generate agent specification.");
		}
		const systemPrompt = prompt.render(agentCreationArchitectPrompt, {});
		const userPrompt = prompt.render(agentCreationUserPrompt, { request: description });
		const { session } = await createAgentSession({
			cwd: this.#cwd,
			authStorage: modelRegistry.authStorage,
			modelRegistry,
			settings: runtimeSettings,
			model: selectedModel,
			systemPrompt: [systemPrompt],
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
		const unsubscribe = session.subscribe(event => {
			if (event.type === "message_update" && "assistantMessageEvent" in event) {
				const ame = event.assistantMessageEvent;
				if (ame.type === "text_delta") {
					this.#createStreamingText += ame.delta;
					this.#tui.requestRender();
				}
			}
		});
		try {
			await session.prompt(userPrompt, { expandPromptTemplates: false });
			const raw = extractAssistantText(session.state.messages);
			if (!raw) {
				throw new Error("No response returned by agent creation architect.");
			}
			return parseGeneratedAgentSpec(raw);
		} finally {
			unsubscribe();
			await session.dispose();
		}
	}

	async #saveGeneratedAgent(): Promise<void> {
		const spec = this.#createSpec;
		if (!spec) return;
		if (this.#createScope === "managed") {
			await this.#saveManagedPresetDraft(spec);
			return;
		}
		const dirs = getConfigDirs("agents", {
			user: this.#createScope === "user",
			project: this.#createScope === "project",
			cwd: this.#cwd,
		});
		const targetDir = dirs[0]?.path;
		if (!targetDir) {
			throw new Error(`Cannot resolve ${this.#createScope} agents directory.`);
		}
		const filePath = path.join(targetDir, `${spec.identifier}.md`);
		try {
			await fs.stat(filePath);
			throw new Error(`Agent file already exists: ${shortenPath(filePath)}`);
		} catch (error) {
			if (!isEnoent(error)) throw error;
		}
		const frontmatter = YAML.stringify({ name: spec.identifier, description: spec.whenToUse }, null, 2).trimEnd();
		const content = `---\n${frontmatter}\n---\n\n${spec.systemPrompt.trim()}\n`;
		await Bun.write(filePath, content);
		await refreshAgentDiscovery(this.#cwd, this.#extensionRoots());
		this.#clearCreateFlow();
		this.#notice = `Created agent ${spec.identifier} at ${shortenPath(filePath)}`;
		await this.#reload();
	}

	/**
	 * Save an architect-generated spec as a managed-preset draft revision.
	 * The draft is unevaluated and inactive until the operator evaluates and
	 * promotes it through the revision manager below — authored and bundled
	 * agents are never touched (name collisions are refused, not shadowed).
	 */
	async #saveManagedPresetDraft(spec: GeneratedAgentSpec): Promise<void> {
		const { agents } = await discoverAgents(this.#cwd, undefined, this.#extensionRoots()).catch(() => ({
			agents: [] as AgentDefinition[],
		}));
		if (
			isPresetNameClaimedByAuthored(spec.identifier, agents) ||
			loadBundledAgents().some(agent => agent.name === spec.identifier)
		) {
			throw new Error(
				`Cannot create managed preset "${spec.identifier}": that name is already claimed by an authored, discovered, or bundled agent. Managed presets cannot override them. Choose a different name.`,
			);
		}
		const draft = await createPresetDraft({
			name: spec.identifier,
			description: spec.whenToUse,
			systemPrompt: spec.systemPrompt,
		});
		await refreshAgentDiscovery(this.#cwd, this.#extensionRoots());
		this.#clearCreateFlow();
		this.#actionError = null;
		// Reload BEFORE announcing: the "Drafted" notice promises the draft
		// row is listed, so the merge must have completed first. Otherwise
		// operators (and tests) observe the notice while the row is still
		// missing and act on a stale list.
		await this.#reload();
		this.#notice =
			`Drafted managed preset ${spec.identifier} revision ${draft.id} ` +
			`(unevaluated — open revisions… to evaluate, then promote)`;
	}

	// ═══════════════════════════════════════════════════════════════════════
	// Managed-preset revision management
	// ═══════════════════════════════════════════════════════════════════════

	/** Open the revision manager for a managed-preset agent. */
	async #openRevisionManager(agent: HubAgent): Promise<void> {
		if (!isManagedPresetAgent(agent)) return;
		this.#closeStrip();
		this.#actionError = null;
		this.#managingRevisions = { agent, items: [], active: null, index: 0, scroll: 0, error: null };
		this.#tui.requestRender();
		try {
			const { active, revisions } = await listPresetRevisions(agent.name);
			const items: PresetRevisionView[] = revisions.map(rev => ({
				id: rev.id,
				state: rev.state,
				active: rev.id === active,
				evaluations: rev.evaluations.length,
				evalStatus: revisionEvalStatus(rev.evaluations),
			}));
			const current = this.#managingRevisions;
			if (current && current.agent.name === agent.name) {
				current.items = items;
				current.active = active;
				current.index = Math.max(0, Math.min(current.index, Math.max(0, items.length - 1)));
			}
		} catch (error) {
			const current = this.#managingRevisions;
			if (current && current.agent.name === agent.name) {
				current.error = error instanceof Error ? error.message : String(error);
			}
		}
		this.#tui.requestRender();
	}

	#closeRevisionManager(): void {
		// Fence late evaluation settlements: a run finishing after close
		// refreshes items only when this same manager is still open.
		this.#evalGeneration++;
		this.#managingRevisions = null;
		this.#presetEvalInput = null;
		this.#inspectingRevision = null;
	}

	#selectedRevision(): PresetRevisionView | undefined {
		return this.#managingRevisions?.items[this.#managingRevisions.index];
	}

	/** Level-1 revision actions for one managed-preset revision. */
	#openRevisionStrip(agent: HubAgent, rev: PresetRevisionView): void {
		const running = this.#presetEvalRunning?.agentName === agent.name && this.#presetEvalRunning.revId === rev.id;
		const chips: StripChip[] = running
			? [
					{
						label: "cancel evaluation",
						styled: theme.fg("warning", "cancel evaluation"),
						action: { kind: "rev-cancel-eval", revId: rev.id },
					},
				]
			: [
					{
						label: `evaluate${theme.symbol("sep.ellipsis")}`,
						styled: theme.fg("accent", `evaluate${theme.symbol("sep.ellipsis")}`),
						action: { kind: "rev-evaluate", revId: rev.id },
					},
					{
						label: "promote",
						styled: theme.fg("accent", "promote"),
						action: { kind: "rev-promote", revId: rev.id },
					},
					{
						label: "rollback",
						styled: theme.fg("warning", "rollback"),
						action: { kind: "rev-rollback", revId: rev.id },
					},
					{
						label: `inspect${theme.symbol("sep.ellipsis")}`,
						styled: theme.fg("muted", `inspect${theme.symbol("sep.ellipsis")}`),
						action: { kind: "rev-inspect", revId: rev.id },
					},
				];
		this.#strip = { kind: "chips", agent, chips, index: 0 };
	}

	async #promoteManagedRevision(agent: HubAgent, revId: string): Promise<void> {
		try {
			const result = await promotePresetRevision(agent.name, revId, { discloseUnevaluated: true });
			await refreshAgentDiscovery(this.#cwd, this.#extensionRoots());
			await this.#reload();
			this.#actionError = null;
			this.#notice = formatPromotedNotice("preset", agent.name, result.revId, result.disclosedUnevaluated);
			await this.#openRevisionManager(this.#findHubAgent(agent.name) ?? agent);
		} catch (error) {
			this.#actionError = error instanceof Error ? error.message : String(error);
			this.#tui.requestRender();
		}
	}

	async #rollbackManagedRevision(agent: HubAgent, revId: string): Promise<void> {
		try {
			await rollbackPresetRevision(agent.name, revId);
			await refreshAgentDiscovery(this.#cwd, this.#extensionRoots());
			await this.#reload();
			this.#actionError = null;
			this.#notice = `Rolled back managed preset ${agent.name} to revision ${revId}`;
			await this.#openRevisionManager(this.#findHubAgent(agent.name) ?? agent);
		} catch (error) {
			this.#actionError = error instanceof Error ? error.message : String(error);
			this.#tui.requestRender();
		}
	}

	#findHubAgent(name: string): HubAgent | undefined {
		return this.#allAgents.find(entry => entry.name === name);
	}

	#beginPresetEval(agent: HubAgent, revId: string): void {
		const input = new Input();
		this.#presetEvalInput = { agent, revId, step: "task", task: "", input };
	}

	#advancePresetEval(): void {
		const evalState = this.#presetEvalInput;
		if (!evalState) return;
		if (evalState.step === "task") {
			const task = evalState.input.getValue().trim();
			if (!task) {
				this.#actionError = "Evaluation needs an explicit task.";
				this.#tui.requestRender();
				return;
			}
			const input = new Input();
			this.#presetEvalInput = { ...evalState, step: "outcome", task, input };
			this.#actionError = null;
			this.#tui.requestRender();
			return;
		}
		const expectedOutcome = evalState.input.getValue().trim();
		if (!expectedOutcome) {
			this.#actionError = "Evaluation needs an explicit expected outcome.";
			this.#tui.requestRender();
			return;
		}
		const { agent, revId, task } = evalState;
		this.#presetEvalInput = null;
		this.#startPresetEval(agent, revId, task, expectedOutcome);
	}

	/**
	 * Run one preset evaluation through the installed (production) evaluator
	 * with the owning session context. The run executes outside the revision
	 * transaction; only the metadata append is serialized there. Late
	 * callbacks are fenced by generation: closing the manager, disposing the
	 * hub, or starting a newer run drops stale settlements instead of
	 * reopening views or rewriting notices.
	 */
	#startPresetEval(agent: HubAgent, revId: string, task: string, expectedOutcome: string): void {
		this.#evalGeneration++;
		const generation = this.#evalGeneration;
		const runController = new AbortController();
		const ownerSignal = this.#evalContext.signal;
		const forwardAbort =
			ownerSignal && !ownerSignal.aborted ? () => runController.abort(ownerSignal.reason) : undefined;
		if (ownerSignal?.aborted) {
			runController.abort(ownerSignal.reason);
		} else if (forwardAbort && ownerSignal) {
			ownerSignal.addEventListener("abort", forwardAbort, { once: true });
		}
		const parent = this.#evalContext.parent;
		const cleanup = (): void => {
			if (forwardAbort && ownerSignal) ownerSignal.removeEventListener("abort", forwardAbort);
		};
		this.#actionError = null;
		this.#notice = `Evaluating ${agent.name} revision ${revId}…`;
		this.#presetEvalRunning = {
			agentName: agent.name,
			revId,
			generation,
			cancel: () => runController.abort(),
		};
		this.#tui.requestRender();
		void evaluatePresetRevision(agent.name, revId, {
			task,
			expectedOutcome,
			parent,
			signal: runController.signal,
			sessionId: parent?.getSessionId?.() ?? undefined,
		})
			.then(async result => {
				cleanup();
				// The revision record changed even when this view is stale:
				// refresh in place (index preserved) without touching notices.
				await this.#refreshRevisionItems(agent.name);
				if (this.#evalGeneration !== generation) return;
				this.#presetEvalRunning = null;
				this.#actionError = null;
				this.#notice =
					`Evaluation of ${agent.name} revision ${revId} ` +
					`${result.passed ? "passed" : "FAILED"}: ${result.summary}`;
				this.#tui.requestRender();
			})
			.catch(async error => {
				cleanup();
				await this.#refreshRevisionItems(agent.name);
				const message = error instanceof Error ? error.message : String(error);
				if (/abort/i.test(message)) {
					// Cancellation records nothing and never passes/promotes.
					// Unlike pass/fail settlements this is terminal operator
					// intent, so the confirmation is never fenced (it never
					// reopens views — it only lands the status notice).
					if (this.#presetEvalRunning?.generation === generation) this.#presetEvalRunning = null;
					this.#actionError = null;
					this.#notice = `Evaluation of ${agent.name} revision ${revId} cancelled — nothing recorded.`;
					this.#tui.requestRender();
					return;
				}
				if (this.#evalGeneration !== generation) return;
				this.#presetEvalRunning = null;
				if (/parent session context/i.test(message)) {
					this.#actionError =
						`Evaluation unavailable: no parent session is wired for this hub. ` +
						`Production evaluations run the revision through restricted task execution, which needs ` +
						`the owning session's model, credentials, and restrictions — the revision stays unevaluated.`;
				} else {
					this.#actionError = message;
				}
				this.#tui.requestRender();
			});
	}

	/** Operator cancel for the in-flight evaluation of one revision, if any. */
	#cancelPresetEval(agent: HubAgent, revId: string): void {
		const running = this.#presetEvalRunning;
		if (running && running.agentName === agent.name && running.revId === revId) {
			running.cancel();
			return;
		}
		this.#notice = `No evaluation is running for ${agent.name} revision ${revId}.`;
		this.#tui.requestRender();
	}

	/** Re-list one manager's revisions in place (selection preserved). */
	async #refreshRevisionItems(agentName: string): Promise<void> {
		const manager = this.#managingRevisions;
		if (!manager || manager.agent.name !== agentName) return;
		try {
			const { active, revisions } = await listPresetRevisions(agentName);
			if (this.#managingRevisions !== manager) return;
			manager.items = revisions.map(rev => ({
				id: rev.id,
				state: rev.state,
				active: rev.id === active,
				evaluations: rev.evaluations.length,
				evalStatus: revisionEvalStatus(rev.evaluations),
			}));
			manager.active = active;
			manager.index = Math.max(0, Math.min(manager.index, Math.max(0, manager.items.length - 1)));
		} catch {
			// Best-effort: the next explicit reload repairs the list.
		}
		this.#tui.requestRender();
	}

	/**
	 * Read-only inspection of one managed-preset revision: identity, state,
	 * provenance, the effective execution restrictions the evaluator will run
	 * with (declared tools/model, else the production defaults), the
	 * evaluation history, and a content preview. Never mutates.
	 */
	async #inspectManagedRevision(agent: HubAgent, revId: string): Promise<void> {
		this.#inspectingRevision = { agent, revId, error: null, body: [] };
		this.#inspectionView.scrollToTop();
		this.#inspectionLayout = undefined;
		this.#tui.requestRender();
		try {
			const { active, revisions } = await listPresetRevisions(agent.name);
			const rev = revisions.find(entry => entry.id === revId);
			const current = this.#inspectingRevision;
			if (!current || current.agent.name !== agent.name || current.revId !== revId) return;
			if (!rev) {
				current.error = `Revision ${revId} for preset "${agent.name}" not found.`;
			} else {
				const lines: string[] = [];
				lines.push(`revision ${rev.id} (${rev.state})${rev.id === active ? " [active]" : ""}`);
				lines.push(`description: ${rev.description}`);
				const provenance = [`actor=${rev.provenance.actor}`];
				if (rev.provenance.sessionId) provenance.push(`session=${rev.provenance.sessionId}`);
				if (rev.provenance.runId) provenance.push(`run=${rev.provenance.runId}`);
				lines.push(`provenance: ${provenance.join(" ")}`);
				// Effective restrictions mirror the production evaluator: the
				// revision's declared tools/model, else the restricted
				// defaults the run executes with.
				let tools = "read, glob, grep (default)";
				let model = `${EVAL_DEFAULT_MODEL_PATTERN} (default)`;
				try {
					const parsed = parseAgent(`managed:${agent.name}`, rev.content, "user", "warn");
					if (parsed.tools && parsed.tools.length > 0) {
						tools = parsed.tools.join(", ");
					}
					if (parsed.model && parsed.model.length > 0) {
						model = parsed.model.join(", ");
					}
				} catch {
					// Unparseable content keeps the defaults above; the eval
					// path itself reports the contract failure.
				}
				lines.push(`effective restrictions: tools=[${tools}] model=${model}`);
				lines.push(`evaluations (${rev.evaluations.length}):`);
				for (const evaluation of rev.evaluations) {
					lines.push(...formatEvaluationLines(evaluation));
				}
				lines.push(`content (${Buffer.byteLength(rev.content, "utf8")} bytes):`);
				for (const contentLine of rev.content.split("\n")) {
					lines.push(`  ${contentLine}`);
				}
				current.body = lines;
			}
		} catch (error) {
			const current = this.#inspectingRevision;
			if (current && current.agent.name === agent.name && current.revId === revId) {
				current.error = error instanceof Error ? error.message : String(error);
			}
		}
		this.#tui.requestRender();
	}

	// ═══════════════════════════════════════════════════════════════════════
	// Input
	// ═══════════════════════════════════════════════════════════════════════

	handleInput(data: string): void {
		if (data.startsWith("\x1b[<")) {
			routeSgrMouseInput(data, event => this.#routeMouseEvent(event));
			this.#tui.requestRender();
			return;
		}

		if (this.#strip) {
			this.#handleStripInput(data);
			this.#tui.requestRender();
			return;
		}

		if (this.#presetEvalInput) {
			this.#handlePresetEvalInput(data);
			this.#tui.requestRender();
			return;
		}

		if (this.#createActive) {
			this.#handleCreateInput(data);
			this.#tui.requestRender();
			return;
		}

		if (this.#managingRevisions) {
			this.#handleRevisionManagerInput(data);
			this.#tui.requestRender();
			return;
		}

		if (matchesSelectCancel(data)) {
			if (this.#assigning) {
				this.#cancelAssign();
				return;
			}
			if (this.#searchQuery.length > 0) {
				this.#searchQuery = "";
				this.#buildRows();
				this.#clampRowIndex();
				this.#tui.requestRender();
				return;
			}
			this.#callbacks.onCancel();
			return;
		}

		if (this.#assigning) {
			this.#browser.handleInput(data);
			this.#tui.requestRender();
			return;
		}

		if (matchesKey(data, "ctrl+r")) {
			void this.#reload();
			return;
		}

		if (matchesKey(data, "tab") || matchesKey(data, "shift+tab")) {
			this.#focus = this.#focus === "scope" ? "list" : "scope";
			this.#tui.requestRender();
			return;
		}
		if (matchesKey(data, "left")) {
			this.#focus = "scope";
			this.#tui.requestRender();
			return;
		}
		if (matchesKey(data, "right")) {
			this.#focus = "list";
			this.#tui.requestRender();
			return;
		}

		if (this.#focus === "scope") {
			if (matchesSelectUp(data)) {
				this.#moveSidebar(-1);
				this.#tui.requestRender();
				return;
			}
			if (matchesSelectDown(data)) {
				this.#moveSidebar(1);
				this.#tui.requestRender();
				return;
			}
			if (matchesKey(data, "enter") || matchesKey(data, "return") || data === "\n") {
				if (this.#activeEntry().kind === "new") {
					this.#beginCreateFlow();
				} else {
					this.#focus = "list";
				}
				this.#tui.requestRender();
				return;
			}
		}

		if (matchesSelectUp(data)) {
			this.#rowIndex = Math.max(0, this.#rowIndex - 1);
			this.#tui.requestRender();
			return;
		}
		if (matchesSelectDown(data)) {
			this.#rowIndex = Math.min(this.#rows.length - 1, this.#rowIndex + 1);
			this.#tui.requestRender();
			return;
		}
		if (matchesKey(data, "enter") || matchesKey(data, "return") || data === "\n") {
			this.#activateRow(this.#rows[this.#rowIndex]);
			this.#tui.requestRender();
			return;
		}
		if (data === " " && this.#searchQuery.length === 0) {
			const agent = this.#selectedAgent();
			if (agent) this.#toggleAgent(agent);
			return;
		}
		if (matchesKey(data, "backspace")) {
			if (this.#searchQuery.length > 0) {
				this.#searchQuery = this.#searchQuery.slice(0, -1);
				this.#buildRows();
				this.#clampRowIndex();
				this.#tui.requestRender();
			}
			return;
		}
		// Type-to-filter: any printable character extends the query.
		if (data.length === 1 && data >= " " && data !== "\x7f") {
			this.#searchQuery += data;
			this.#focus = "list";
			this.#buildRows();
			this.#rowIndex = 0;
			this.#listScroll = 0;
			this.#tui.requestRender();
		}
	}

	#activateRow(rowDef: ListRow | undefined): void {
		if (!rowDef) return;
		if (rowDef.kind === "new") {
			this.#beginCreateFlow();
			return;
		}
		this.#openAgentStrip(rowDef.agent);
	}

	#handleStripInput(data: string): void {
		const strip = this.#strip;
		if (!strip) return;
		if (matchesSelectCancel(data)) {
			// A property strip steps back up to the agent strip instead of closing.
			if (strip.kind === "chips" && strip.property) {
				this.#openAgentStrip(strip.agent);
				return;
			}
			if (strip.kind === "pattern") {
				this.#openPropertyStrip(strip.agent, strip.property);
				return;
			}
			this.#closeStrip();
			return;
		}
		if (strip.kind === "pattern") {
			if (matchesKey(data, "enter") || matchesKey(data, "return") || data === "\n") {
				this.#submitPattern();
				return;
			}
			strip.input.handleInput(data);
			return;
		}
		if (matchesKey(data, "left") || matchesKey(data, "up") || matchesKey(data, "shift+tab")) {
			strip.index = (strip.index - 1 + strip.chips.length) % strip.chips.length;
			return;
		}
		if (matchesKey(data, "right") || matchesKey(data, "down") || matchesKey(data, "tab")) {
			strip.index = (strip.index + 1) % strip.chips.length;
			return;
		}
		if (matchesKey(data, "enter") || matchesKey(data, "return") || data === "\n") {
			this.#activateStripChip();
			return;
		}
	}

	#handleCreateInput(data: string): void {
		if (this.#createSpec) {
			if (matchesSelectCancel(data)) {
				this.#clearCreateFlow();
				return;
			}
			if (matchesKey(data, "tab") || matchesKey(data, "shift+tab")) {
				this.#cycleCreateScope();
				return;
			}
			if (data.toLowerCase() === "r") {
				void this.#generateAgentFromDescription(this.#createDescription);
				return;
			}
			if (matchesKey(data, "enter") || matchesKey(data, "return") || data === "\n") {
				void this.#saveGeneratedAgent().catch(error => {
					this.#createError = error instanceof Error ? error.message : String(error);
					this.#tui.requestRender();
				});
			} else this.#createReview.handleScrollKey(data);
			return;
		}
		if (matchesSelectCancel(data)) {
			if (!this.#createGenerating) this.#clearCreateFlow();
			return;
		}
		if (this.#createGenerating) return;
		if (matchesAppFollowUp(data)) {
			void this.#generateAgentFromDescription(this.#createInput?.getExpandedText() ?? this.#createDescription);
			return;
		}
		if (matchesKey(data, "tab") || matchesKey(data, "shift+tab")) {
			this.#cycleCreateScope();
			return;
		}
		if (matchesKey(data, "enter") || matchesKey(data, "return") || data === "\n") {
			this.#createInput?.handleInput("\n");
			this.#createDescription = this.#createInput?.getExpandedText() ?? "";
			return;
		}
		this.#createInput?.handleInput(data);
		this.#createDescription = this.#createInput?.getExpandedText() ?? "";
	}

	/** Cycle the create-flow save scope: project → user → managed. */
	#cycleCreateScope(): void {
		this.#createScope =
			this.#createScope === "project" ? "user" : this.#createScope === "user" ? "managed" : "project";
	}

	#handlePresetEvalInput(data: string): void {
		const evalState = this.#presetEvalInput;
		if (!evalState) return;
		if (matchesSelectCancel(data)) {
			this.#presetEvalInput = null;
			this.#actionError = null;
			return;
		}
		if (matchesKey(data, "enter") || matchesKey(data, "return") || data === "\n") {
			this.#advancePresetEval();
			return;
		}
		evalState.input.handleInput(data);
	}

	#handleRevisionManagerInput(data: string): void {
		const manager = this.#managingRevisions;
		if (!manager) return;
		if (this.#inspectingRevision) {
			if (matchesSelectCancel(data)) this.#inspectingRevision = null;
			else this.#inspectionView.handleScrollKey(data);
			return;
		}
		if (matchesSelectCancel(data)) {
			this.#closeRevisionManager();
			return;
		}
		if (matchesSelectUp(data)) {
			manager.index = Math.max(0, manager.index - 1);
			return;
		}
		if (matchesSelectDown(data)) {
			manager.index = Math.min(Math.max(0, manager.items.length - 1), manager.index + 1);
			return;
		}
		if (matchesKey(data, "enter") || matchesKey(data, "return") || data === "\n") {
			const rev = this.#selectedRevision();
			if (rev) this.#openRevisionStrip(manager.agent, rev);
		}
	}

	#moveSidebar(delta: number): void {
		const count = this.#entries.length;
		if (count === 0) return;
		let index = this.#entries.findIndex(entry => entry.id === this.#activeEntryId);
		if (index < 0) index = 0;
		for (let step = 0; step < count; step++) {
			index = (index + delta + count) % count;
			const entry = this.#entries[index];
			if (entry && entry.kind !== "separator") {
				this.#activeEntryId = entry.id;
				if (entry.kind !== "new") {
					this.#buildRows();
					this.#rowIndex = 0;
					this.#listScroll = 0;
				}
				return;
			}
		}
	}

	// ═══════════════════════════════════════════════════════════════════════
	// Mouse
	// ═══════════════════════════════════════════════════════════════════════

	#routeMouseEvent(event: SgrMouseEvent): boolean {
		const contentLine = event.row - this.#contentRowStart;
		const overContent = contentLine >= 0 && contentLine < this.#contentRowCount;
		const sidebarColEnd = this.#splitVisible ? 2 + this.#sidebarWidthLast : Infinity;
		const bodyColStart = this.#splitVisible ? this.#sidebarWidthLast + 5 : this.#contentInset;
		const overSidebar =
			overContent && (this.#splitVisible || this.#scopeOnly) && event.col >= 0 && event.col < sidebarColEnd;
		const overBody = overContent && !this.#scopeOnly && event.col >= bodyColStart;
		const bodyLine = contentLine - this.#bodyHeaderRows;

		if (event.row === this.#footerRow && this.#strip?.kind === "chips") {
			const strip = this.#strip;
			if (event.leftClick) {
				for (const range of this.#chipRanges) {
					if (event.col >= range.start && event.col < range.end) {
						strip.index = range.index;
						this.#activateStripChip();
						return true;
					}
				}
			}
			return true;
		}

		if (this.#assigning) {
			if (overBody) this.#browser.routeMouse(event, bodyLine);
			return true;
		}
		if (this.#inspectingRevision && overBody && event.wheel !== null) {
			this.#inspectionView.scroll(event.wheel);
			return true;
		}
		if (this.#createSpec && overBody && event.wheel !== null) {
			this.#createReview.scroll(event.wheel);
			return true;
		}
		if (this.#createActive || this.#strip || this.#managingRevisions || this.#presetEvalInput) return true;

		if (event.wheel !== null) {
			if (overSidebar) {
				const maxScroll = Math.max(0, this.#entries.length - this.#contentRowCount);
				this.#sidebarScroll = Math.max(0, Math.min(this.#sidebarScroll + event.wheel, maxScroll));
			} else if (overBody) {
				this.#rowIndex = Math.max(0, Math.min(this.#rows.length - 1, this.#rowIndex + event.wheel));
			}
			return true;
		}

		if (event.motion) {
			// Hover is stored as an absolute row index so paint and click agree.
			const hoverRow = bodyLine - this.#listRowStart + this.#listScroll;
			this.#rowHover = overBody && hoverRow >= 0 && hoverRow < this.#rows.length ? hoverRow : null;
			return true;
		}

		if (!event.leftClick) return true;

		if (overSidebar) {
			const index = this.#sidebarScroll + contentLine;
			const clicked = this.#entries[index];
			if (clicked && clicked.kind !== "separator") {
				if (clicked.kind === "new") {
					this.#beginCreateFlow();
				} else {
					this.#activeEntryId = clicked.id;
					this.#buildRows();
					this.#rowIndex = 0;
					this.#focus = "scope";
				}
			}
			return true;
		}
		if (overBody) {
			this.#focus = "list";
			const listLine = bodyLine - this.#listRowStart + this.#listScroll;
			if (listLine >= 0 && listLine < this.#rows.length) {
				if (listLine === this.#rowIndex) {
					this.#activateRow(this.#rows[listLine]);
				} else {
					this.#rowIndex = listLine;
				}
			}
		}
		return true;
	}

	// ═══════════════════════════════════════════════════════════════════════
	// Rendering
	// ═══════════════════════════════════════════════════════════════════════

	#terminalRows(): number {
		// Short-terminal budget: never render taller than the viewport itself.
		// The old Math.max(16, …) floor overflowed viewports under 16 rows and
		// pushed the search row, selected row, and footer into scrollback.
		const terminalRows = this.#tui.terminal?.rows || process.stdout.rows || 40;
		return Math.max(1, Math.min(this.#maxHeight ?? terminalRows, terminalRows));
	}

	#sidebarWidth(): number {
		let longest = 0;
		for (const entry of this.#entries) {
			longest = Math.max(longest, visibleWidth(entry.label) + visibleWidth(entry.annotation ?? "") + 5);
		}
		return Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, longest));
	}

	#renderSidebar(width: number, rows: number): string[] {
		const activeIndex = Math.max(
			0,
			this.#entries.findIndex(entry => entry.id === this.#activeEntryId),
		);
		if (activeIndex < this.#sidebarScroll) this.#sidebarScroll = activeIndex;
		else if (activeIndex >= this.#sidebarScroll + rows) this.#sidebarScroll = activeIndex - rows + 1;

		const lines: string[] = [];
		for (let i = this.#sidebarScroll; i < Math.min(this.#entries.length, this.#sidebarScroll + rows); i++) {
			const entry = this.#entries[i];
			if (!entry) continue;
			if (entry.kind === "separator") {
				lines.push(theme.fg("border", theme.symbol("boxRound.horizontal").repeat(width)));
				continue;
			}
			const active = entry.id === this.#activeEntryId;
			const cursor = active && this.#focus === "scope" ? theme.fg("accent", theme.nav.cursor) : " ";
			const icon = entry.kind === "all" ? theme.icon.model : entry.kind === "new" ? "+" : theme.status.enabled;
			const labelStyled = active ? theme.bold(theme.fg("accent", entry.label)) : entry.label;
			const left = `${cursor} ${theme.fg(entry.kind === "new" ? "dim" : "accent", icon)} ${labelStyled}`;
			const annotation = theme.fg("dim", entry.annotation ?? "");
			const leftWidth = visibleWidth(left);
			const annWidth = visibleWidth(annotation);
			let line: string;
			if (leftWidth + annWidth + 1 <= width) {
				line = `${left}${" ".repeat(width - leftWidth - annWidth)}${annotation}`;
			} else {
				line = fit(left, width);
			}
			lines.push(line);
		}
		return lines;
	}

	#statusRow(width: number): string {
		if (this.#loadError) return fit(theme.fg("error", ` ${this.#loadError}`), width);
		if (this.#assigning) {
			const { agent, property } = this.#assigning;
			const what = property === "model" ? "model override" : `${property} model`;
			return fit(
				theme.fg("accent", ` Picking ${what} for ${theme.bold(agent.name)}; Enter assigns, Esc cancels`),
				width,
			);
		}
		if (this.#createActive) {
			return fit(theme.fg("accent", " New agent: describe it and let the architect draft it"), width);
		}
		if (this.#managingRevisions) {
			const manager = this.#managingRevisions;
			const error = manager.error ?? this.#actionError;
			if (error) return fit(theme.fg("error", ` ${error}`), width);
			const active = manager.active ? `active ${manager.active}` : "no active revision";
			return fit(theme.fg("accent", ` Revisions for ${theme.bold(manager.agent.name)}; ${active}`), width);
		}
		if (this.#actionError) return fit(theme.fg("error", ` ${this.#actionError}`), width);
		if (this.#notice) return fit(theme.fg("success", ` ${this.#notice}`), width);
		const entry = this.#activeEntry();
		const scopeLabel = entry.kind === "source" ? `${entry.label} agents` : "All agents";
		const count = this.#rows.filter(rowDef => rowDef.kind === "agent").length;
		return fit(theme.fg("muted", ` ${scopeLabel}${theme.sep.dot}${count}`), width);
	}

	#renderList(width: number, rows: number): string[] {
		const lines: string[] = [];
		const searchText = this.#searchQuery ? theme.fg("accent", this.#searchQuery) : theme.fg("dim", "type to filter");
		if (rows >= 3) lines.push(fit(` ${theme.fg("muted", "search:")} ${searchText}`, width));
		if (rows >= 8) lines.push("");
		this.#listRowStart = lines.length;

		const detailRows = 4;
		// Short terminals collapse the detail block first: the search row and
		// the selected agent row keep their rows, the detail block is cut by
		// the trailing slice below.
		const visibleRows = Math.max(1, rows - lines.length - (rows >= 8 ? detailRows : 0));
		if (this.#rowIndex < this.#listScroll) this.#listScroll = this.#rowIndex;
		else if (this.#rowIndex >= this.#listScroll + visibleRows) this.#listScroll = this.#rowIndex - visibleRows + 1;
		this.#listScroll = Math.max(0, Math.min(this.#listScroll, Math.max(0, this.#rows.length - visibleRows)));

		let nameWidth = 0;
		for (const rowDef of this.#rows) {
			if (rowDef.kind === "agent") nameWidth = Math.max(nameWidth, visibleWidth(rowDef.agent.name));
		}

		const listFocused = this.#focus === "list";
		for (let i = this.#listScroll; i < Math.min(this.#rows.length, this.#listScroll + visibleRows); i++) {
			const rowDef = this.#rows[i];
			if (!rowDef) continue;
			const selected = i === this.#rowIndex;
			const hovered = i === this.#rowHover;
			const cursor = selected && listFocused ? theme.fg("accent", theme.nav.cursor) : " ";
			if (rowDef.kind === "new") {
				let line = ` ${cursor} ${theme.fg(selected ? "accent" : "dim", `+ New agent${theme.symbol("sep.ellipsis")}`)}`;
				if (selected || hovered) line = theme.bgFill("selectedBg", fit(line, width));
				lines.push(fit(line, width));
				continue;
			}
			const agent = rowDef.agent;
			const dot = agent.disabled
				? theme.fg("dim", theme.status.disabled)
				: theme.fg("success", theme.status.enabled);
			const name = replaceTabs(agent.name).padEnd(nameWidth);
			const nameStyled = agent.disabled
				? theme.fg("dim", name)
				: selected
					? theme.bold(theme.fg("accent", name))
					: name;
			const badges: string[] = [];
			if (agent.overrideModel) badges.push(theme.fg("warning", agent.overrideModel));
			if (agent.managedDraftOnly) badges.push(theme.fg("accent", `draft${theme.sep.dot}unevaluated`));
			const prewalk = this.#effectivePrewalkPattern(agent);
			if (prewalk) badges.push(theme.fg("dim", `pre:${prewalk}`));
			const advisor = this.#effectiveAdvisorPattern(agent);
			if (advisor) badges.push(theme.fg("dim", `adv:${advisor}`));
			const sourceTag = theme.fg("dim", SOURCE_LABEL[agent.source].toLowerCase());
			let line = ` ${cursor} ${dot} ${nameStyled}  ${sourceTag}`;
			const right = badges.join("  ");
			const rightWidth = visibleWidth(right);
			const lineWidth = visibleWidth(line);
			if (rightWidth > 0 && lineWidth + rightWidth + 2 <= width) {
				line = `${line}${" ".repeat(width - lineWidth - rightWidth - 1)}${right}`;
			}
			line = fit(line, width);
			if (selected || hovered) {
				const w = visibleWidth(line);
				if (w < width) line += " ".repeat(width - w);
				line = theme.bgFill("selectedBg", line);
			}
			lines.push(line);
		}

		// Selected-agent detail block pinned to the bottom of the body pane.
		while (lines.length < rows - detailRows) lines.push("");
		const agent = this.#selectedAgent();
		lines.push(theme.fg("border", theme.symbol("boxRound.horizontal").repeat(Math.max(1, width))));
		if (agent) {
			lines.push(fit(` ${theme.fg("dim", replaceTabs(agent.description))}`, width));
			const patterns = this.#effectiveModelPatterns(agent);
			const resolved = this.#resolvePatterns(patterns);
			const modelLine = `${theme.fg("muted", "model:")} ${patterns.length > 0 ? replaceTabs(patterns.join(",")) : theme.fg("dim", "(session model)")}${resolved ? ` ${theme.fg("dim", theme.getSymbolPreset() === "ascii" ? "->" : "→")} ${theme.fg("success", resolved)}` : ""}`;
			lines.push(fit(` ${modelLine}`, width));
			const prewalk = this.#effectivePrewalkPattern(agent);
			const advisor = this.#effectiveAdvisorPattern(agent);
			const flagLine = [
				`${theme.fg("muted", "prewalk:")} ${prewalk ? theme.fg("success", prewalk) : theme.fg("dim", "off")}`,
				`${theme.fg("muted", "advisor:")} ${advisor ? theme.fg("success", advisor) : theme.fg("dim", "off")}`,
				agent.filePath ? theme.fg("dim", shortenPath(agent.filePath)) : "",
			]
				.filter(Boolean)
				.join("   ");
			lines.push(fit(` ${flagLine}`, width));
		} else {
			lines.push(theme.fg("dim", " Select an agent to inspect"));
			lines.push("");
			lines.push("");
		}
		return lines.slice(0, rows);
	}

	#renderRevisions(width: number, rows: number): string[] {
		const manager = this.#managingRevisions;
		const lines: string[] = [];
		if (!manager) return lines;
		const inspecting = this.#inspectingRevision;
		if (inspecting && inspecting.agent.name === manager.agent.name) {
			if (inspecting.error) {
				lines.push(fit(theme.fg("error", ` ${replaceTabs(inspecting.error)}`), width));
			} else if (inspecting.body.length === 0) {
				lines.push(fit(theme.fg("dim", ` Loading revision${theme.symbol("sep.ellipsis")}`), width));
			} else {
				if (this.#inspectionLayout?.body !== inspecting.body || this.#inspectionLayout.width !== width) {
					const wrapWidth = Math.max(1, width - 1);
					const wrapped = inspecting.body.flatMap(bodyLine =>
						wrapTextWithAnsi(replaceTabs(sanitizeText(bodyLine)), wrapWidth),
					);
					this.#inspectionView.setLines(wrapped);
					this.#inspectionLayout = { body: inspecting.body, width };
				}
				this.#inspectionView.setSymbols(getSymbolTheme());
				this.#inspectionView.setHeight(rows);
				lines.push(...this.#inspectionView.render(width));
			}
			while (lines.length < rows) lines.push("");
			return lines.slice(0, rows);
		}
		if (manager.error) {
			lines.push(fit(theme.fg("error", ` ${replaceTabs(manager.error)}`), width));
		}
		if (manager.items.length === 0 && !manager.error) {
			lines.push(
				fit(theme.fg("dim", " No revisions yet — generate a draft from + New agent… (managed scope)"), width),
			);
		}
		const visibleRows = Math.max(1, rows - lines.length);
		if (manager.index < manager.scroll) manager.scroll = manager.index;
		else if (manager.index >= manager.scroll + visibleRows) manager.scroll = manager.index - visibleRows + 1;
		manager.scroll = Math.max(0, Math.min(manager.scroll, Math.max(0, manager.items.length - visibleRows)));
		for (let i = manager.scroll; i < Math.min(manager.items.length, manager.scroll + visibleRows); i++) {
			const rev = manager.items[i];
			if (!rev) continue;
			const selected = i === manager.index;
			const cursor = selected ? theme.fg("accent", theme.nav.cursor) : " ";
			const running =
				this.#presetEvalRunning?.agentName === manager.agent.name && this.#presetEvalRunning.revId === rev.id;
			const evalStyled = running
				? theme.fg("accent", `evaluating${theme.symbol("sep.ellipsis")}`)
				: rev.evalStatus === "passing"
					? theme.fg("success", `passing (${rev.evaluations})`)
					: rev.evalStatus === "failing"
						? theme.fg("error", `failing (${rev.evaluations})`)
						: theme.fg("dim", "unevaluated");
			const activeMark = rev.active ? theme.fg("success", " [active]") : "";
			const idStyled = rev.active ? theme.bold(theme.fg("accent", rev.id)) : rev.id;
			lines.push(fit(` ${cursor} ${idStyled}  ${theme.fg("muted", rev.state)}  ${evalStyled}${activeMark}`, width));
		}
		while (lines.length < rows) lines.push("");
		return lines.slice(0, rows);
	}

	#renderCreate(width: number, rows: number): string[] {
		if (!this.#createSpec && !this.#createGenerating && this.#createInput) {
			this.#createInput.setMaxHeight(Math.max(1, rows));
			return this.#createInput
				.render(Math.max(1, width))
				.slice(0, rows)
				.map(line => fit(line, width));
		}
		if (this.#createSpec) {
			const spec = this.#createSpec;
			const cache = this.#createReviewLayout;
			if (
				cache?.spec !== spec ||
				cache.width !== width ||
				cache.scope !== this.#createScope ||
				cache.error !== this.#createError
			) {
				if (cache?.spec !== spec) this.#createReview.scrollToTop();
				const content = [
					"Review generated agent",
					`Identifier: ${spec.identifier}`,
					`Scope: ${this.#createScope}`,
					...(this.#createError ? [`Error: ${this.#createError}`] : []),
					"",
					"whenToUse:",
					...spec.whenToUse.split("\n"),
					"",
					"systemPrompt:",
					...spec.systemPrompt.split("\n"),
				];
				this.#createReview.setLines(
					content.flatMap(line => wrapTextWithAnsi(replaceTabs(sanitizeText(line)), Math.max(1, width - 1))),
				);
				this.#createReviewLayout = { spec, width, scope: this.#createScope, error: this.#createError };
			}
			this.#createReview.setSymbols(getSymbolTheme());
			this.#createReview.setHeight(rows);
			return [...this.#createReview.render(width)];
		}
		const lines = [theme.fg("muted", `Generating${theme.symbol("sep.ellipsis")}`)];
		const wrapped = this.#createStreamingText
			.split("\n")
			.flatMap(line => wrapTextWithAnsi(replaceTabs(sanitizeText(line)), Math.max(1, width)));
		if (rows > 1) lines.push(...wrapped.slice(-(rows - 1)));
		if (this.#createError) {
			lines[0] = fit(theme.fg("error", replaceTabs(sanitizeText(this.#createError))), width);
		}
		while (lines.length < rows) lines.push("");
		return lines.slice(0, rows);
	}

	#footerHint(): string {
		const hint = (...parts: string[]): string => parts.join(theme.sep.dot);
		if (this.#strip) {
			if (this.#strip.kind === "pattern") {
				const property = this.#strip.property;
				const values = property === "model" ? "a model pattern" : '"on", "off", or a model pattern';
				return hint("Esc back", `Enter ${values} (@smol and :level suffixes work; empty clears)`);
			}
			return hint("Esc back", "Left/Right choose", this.#strip.property ? "Enter apply" : "Enter open");
		}
		if (this.#presetEvalInput) {
			return this.#presetEvalInput.step === "task"
				? hint("Esc back", "Enter task for this revision")
				: hint("Esc back", "Enter run evaluation", "expected observable outcome");
		}
		if (this.#assigning) {
			return hint("Esc cancel", "Enter pick", "Up/Down models", "type to search");
		}
		if (this.#createActive) {
			if (this.#createSpec)
				return hint(
					"Esc cancel",
					"Enter save",
					"PgUp/PgDn scroll",
					"Home/End limits",
					"Tab scope (project/user/managed)",
					"r regenerate",
				);
			if (this.#createGenerating) return `Generating${theme.symbol("sep.ellipsis")}`;
			return hint("Esc cancel", "Ctrl+Q/Ctrl+Enter generate", "Enter newline", "Tab scope");
		}
		if (this.#managingRevisions) {
			if (this.#inspectingRevision) return hint("Esc back", "Up/Down scroll", "PgUp/PgDn page", "Home/End limits");
			return hint("Esc back", "Up/Down revisions", "Enter actions (inspect/evaluate/promote/rollback)");
		}
		if (this.#focus === "scope") {
			return hint("Esc close", "Right/Enter agents", "Up/Down scopes");
		}
		return hint(
			"Esc close",
			"Enter configure",
			"Space enable/disable",
			"Up/Down rows",
			"Left scopes",
			"type to search",
			"Ctrl+R reload",
		);
	}

	#renderFooter(width: number): string {
		this.#chipRanges = [];
		const evalInput = this.#presetEvalInput;
		if (evalInput) {
			const what = evalInput.step === "task" ? "evaluation task" : "expected outcome";
			const label = theme.fg("accent", `${evalInput.agent.name} ${evalInput.revId} ${what}:`);
			const labelWidth = visibleWidth(`${evalInput.agent.name} ${evalInput.revId} ${what}:`);
			const showLabel = width >= labelWidth + 8;
			const inputWidth = Math.max(1, width - (showLabel ? labelWidth + 1 : 0));
			const inputLine = evalInput.input.render(inputWidth)[0] ?? "";
			return fit(`${showLabel ? `${label} ` : ""}${inputLine}`, width);
		}
		const strip = this.#strip;
		if (!strip) {
			return fit(theme.fg("dim", this.#footerHint()), width);
		}
		if (strip.kind === "pattern") {
			const label = theme.fg("accent", `${strip.agent.name} ${strip.property} pattern:`);
			const labelWidth = visibleWidth(`${strip.agent.name} ${strip.property} pattern:`);
			const showLabel = width >= labelWidth + 8;
			const inputWidth = Math.max(1, width - (showLabel ? labelWidth + 1 : 0));
			const inputLine = strip.input.render(inputWidth)[0] ?? "";
			return fit(`${showLabel ? `${label} ` : ""}${inputLine}`, width);
		}
		const arrow = theme.getSymbolPreset() === "ascii" ? "->" : "→";
		const prefix = strip.property
			? `${theme.fg("accent", strip.agent.name)}${theme.fg("dim", `${theme.sep.dot}${strip.property} ${arrow}`)} `
			: `${theme.fg("accent", strip.agent.name)}${theme.fg("dim", ` ${arrow}`)} `;
		const selectedChip = strip.chips[strip.index];
		const showPrefix = visibleWidth(prefix) + visibleWidth(selectedChip?.styled ?? "") + 4 <= width;
		let line = showPrefix ? prefix : "";
		let col = this.#contentInset + visibleWidth(line);
		let start = strip.index;
		let remaining = width - visibleWidth(line);
		for (let i = strip.index - 1; i >= 0; i--) {
			const cost = visibleWidth(strip.chips[i]?.styled ?? "") + 3;
			const selectedCost = visibleWidth(selectedChip?.styled ?? "") + 4;
			if (cost + selectedCost > remaining) break;
			remaining -= cost;
			start = i;
		}
		for (let i = start; i < strip.chips.length; i++) {
			const chip = strip.chips[i];
			if (!chip) continue;
			const selected = i === strip.index;
			const available = Math.max(0, width - visibleWidth(line));
			if (!selected && visibleWidth(chip.styled) + 2 > available) break;
			const body =
				selected && available < visibleWidth(chip.styled) + 4
					? fit(`${theme.nav.cursor} ${chip.styled}`, available)
					: ` ${chip.styled} `;
			const bracket = selected && available >= visibleWidth(chip.styled) + 4;
			const rendered = selected
				? theme.bgFill("selectedBg", bracket ? `${theme.fg("accent", "[")}${body}${theme.fg("accent", "]")}` : body)
				: body;
			const w = visibleWidth(rendered);
			this.#chipRanges.push({ start: col, end: col + w, index: i });
			line += rendered;
			col += w;
			line += " ";
			col += 1;
		}
		return fit(line, width);
	}

	render(width: number): string[] {
		const height = this.#terminalRows();
		const sidebarWidth = this.#sidebarWidth();
		this.#sidebarWidthLast = sidebarWidth;
		this.#contentInset = Math.floor((width - dialogContentWidth(width)) / 2);
		this.#splitVisible = canSplitPane(width, sidebarWidth);
		this.#scopeOnly =
			!this.#splitVisible &&
			this.#focus === "scope" &&
			!this.#createActive &&
			!this.#strip &&
			!this.#assigning &&
			!this.#managingRevisions &&
			!this.#presetEvalInput;
		const bodyWidth = this.#splitVisible ? splitBodyWidth(width, sidebarWidth) : dialogContentWidth(width);
		const footer = this.#renderFooter(dialogContentWidth(width));
		const allocation = renderDialog(
			"Agents",
			Array.from({ length: height }, () => ""),
			width,
			height,
			footer,
		);
		const contentRows = allocation.bodyRows;
		this.#contentRowCount = contentRows;
		this.#bodyHeaderRows = contentRows >= 4 ? 1 : 0;

		const bodyLines: string[] = this.#bodyHeaderRows ? [this.#statusRow(bodyWidth)] : [];
		const primaryRows = contentRows - this.#bodyHeaderRows;
		const footerAsControl = height === 1 && (this.#strip || this.#presetEvalInput);
		if (footerAsControl) {
			bodyLines.push(footer);
		} else if (this.#createActive) {
			bodyLines.push(...this.#renderCreate(bodyWidth, primaryRows));
		} else if (this.#assigning) {
			this.#browser.setMaxHeight(primaryRows);
			this.#browser.setFocused(true);
			bodyLines.push(...this.#browser.render(bodyWidth));
		} else if (this.#managingRevisions) {
			bodyLines.push(...this.#renderRevisions(bodyWidth, primaryRows));
		} else {
			bodyLines.push(...this.#renderList(bodyWidth, primaryRows));
		}

		const sidebarLines = this.#renderSidebar(this.#splitVisible ? sidebarWidth : bodyWidth, contentRows);
		const content = this.#scopeOnly ? sidebarLines : bodyLines;
		while (content.length < contentRows) content.push("");
		const layout = renderDialog("Agents", content.slice(0, contentRows), width, height, footer);
		this.#contentRowStart = layout.bodyRowStart;
		this.#footerRow = footerAsControl ? layout.bodyRowStart : height >= 2 ? layout.bodyRowStart + contentRows : -1;
		const activeSidebar = this.#entries.findIndex(entry => entry.id === this.#activeEntryId) - this.#sidebarScroll;
		for (let i = 0; i < contentRows; i++) {
			const activeBody =
				!this.#createActive &&
				!this.#assigning &&
				!this.#managingRevisions &&
				!footerAsControl &&
				i === this.#bodyHeaderRows + this.#listRowStart + this.#rowIndex - this.#listScroll;
			if (this.#splitVisible) {
				layout.lines[layout.bodyRowStart + i] = splitRow(
					sidebarLines[i] ?? "",
					bodyLines[i] ?? "",
					width,
					sidebarWidth,
					i === activeSidebar ? "selectedBg" : "panelBg",
				);
			} else {
				layout.lines[layout.bodyRowStart + i] = row(
					content[i] ?? "",
					width,
					undefined,
					this.#scopeOnly
						? i === activeSidebar
							? "selectedBg"
							: "modalBg"
						: activeBody
							? "selectedBg"
							: "modalBg",
				);
			}
		}
		return layout.lines;
	}
}
