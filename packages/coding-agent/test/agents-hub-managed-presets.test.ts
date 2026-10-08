/**
 * Workstream C (C3) — /agents hub managed-preset lifecycle through the
 * isolated managed-presets service.
 *
 * Contracts (hub surface only; service internals owned by workstream B):
 * - The creation architect runs on runtime-only settings (a clone with
 *   recursive capture disabled), never mutating the shared Settings on
 *   success or failure — persisted config bytes stay identical.
 * - Managed-scope saves mint draft revisions via the managed-preset service
 *   (revision history, unevaluated + inactive) and never touch authored or
 *   bundled agent files; collisions are refused with an actionable error.
 * - The revision manager surfaces draft/eval state per revision and routes
 *   evaluate → promote → rollback through the existing service, with failing
 *   evaluations blocking promotion and every failure rendered actionably.
 * - Authored flows are preserved: project/user saves, strips, and toggles
 *   behave exactly as before.
 */
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { KeybindingsManager } from "@harvest/pi-coding-agent/config/keybindings";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import { AgentsHubComponent, setAgentsHubArchitectRunner } from "@harvest/pi-coding-agent/modes/components/agents-hub";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import * as discovery from "@harvest/pi-coding-agent/task/discovery";
import {
	createPresetDraft,
	evaluatePresetRevision,
	listPresetRevisions,
	setPresetEvalRunner,
} from "@harvest/pi-coding-agent/task/agents";
import type { TUI } from "@harvest/pi-tui";
import { setKeybindings } from "@harvest/pi-tui";
import { removeWithRetries } from "@harvest/pi-utils";
import { getAgentDir, setAgentDir } from "@harvest/pi-utils/dirs";

const ANSI_PATTERN = /\x1b\[[0-?]*[ -/]*[@-~]/g;

let tempCwd: string;
let tempAgentDir: string;
let testAgentDir: string;
let originalAgentDir: string;

const tuiStub = { requestRender: () => {}, terminal: { rows: 30 } } as unknown as TUI;

const SCOUT_SPEC = {
	identifier: "hub-drafted-scout",
	whenToUse: "Use this agent when scouting an unfamiliar repository for entry points.",
	systemPrompt: "You scout repositories and report entry points concisely.",
};

const AUTHORED_SPEC = {
	identifier: "hub-authored-helper",
	whenToUse: "Use this agent when helping with authored-save coverage.",
	systemPrompt: "You help with authored saves.",
};

let nextSpec = SCOUT_SPEC;
let capturedRuntimeSettings: Settings | undefined;

function mockAgents(
	agents: Array<{ name: string; description: string; source: "project" | "user" | "bundled"; filePath?: string }>,
): void {
	vi.spyOn(discovery, "discoverAgents").mockResolvedValue({
		projectAgentsDir: null,
		agents: agents.map(agent => ({ systemPrompt: "", ...agent })),
	});
}

async function createHub(settings: Settings): Promise<AgentsHubComponent> {
	return AgentsHubComponent.create(tuiStub, tempCwd, settings, {}, { onCancel: () => {} });
}

function strip(hub: AgentsHubComponent): string {
	return hub.render(120).join("\n").replace(ANSI_PATTERN, "");
}

function type(hub: AgentsHubComponent, text: string): void {
	for (const char of text) hub.handleInput(char);
}

async function waitFor(label: string, predicate: () => boolean | Promise<boolean>, timeoutMs = 10000): Promise<void> {
	const start = Date.now();
	for (;;) {
		if (await predicate()) return;
		if (Date.now() - start > timeoutMs) throw new Error(`timed out waiting for ${label}`);
		await Bun.sleep(25);
	}
}

async function walkFiles(root: string): Promise<string[]> {
	const out: string[] = [];
	async function walk(dir: string): Promise<void> {
		const entries = await fs.readdir(dir, { withFileTypes: true }).catch(() => []);
		for (const entry of entries) {
			const full = path.join(dir, entry.name);
			if (entry.isDirectory()) await walk(full);
			else out.push(path.relative(root, full));
		}
	}
	await walk(root);
	return out.sort();
}

beforeAll(async () => {
	await initTheme(false);
	tempCwd = await fs.mkdtemp(path.join(os.tmpdir(), "omp-agents-hub-managed-"));
	tempAgentDir = await fs.mkdtemp(path.join(os.tmpdir(), "omp-agents-hub-agentdir-"));
	originalAgentDir = getAgentDir();
});

afterAll(async () => {
	await removeWithRetries(tempCwd);
	await removeWithRetries(tempAgentDir);
});

beforeEach(async () => {
	// Per-test agent-dir isolation: drafts are now visible as hub rows in
	// the same store (U1), so a shared store would shift row navigation
	// between tests. Each test gets a fresh store.
	testAgentDir = await fs.mkdtemp(path.join(tempAgentDir, "t-"));
	setAgentDir(path.join(testAgentDir, "agent"));
	setKeybindings(KeybindingsManager.inMemory());
	nextSpec = SCOUT_SPEC;
	capturedRuntimeSettings = undefined;
	setAgentsHubArchitectRunner(async ({ brief, settings }) => {
		capturedRuntimeSettings = settings;
		expect(brief.trim().length).toBeGreaterThan(0);
		return { ...nextSpec };
	});
	setPresetEvalRunner(async () => ({ passed: true, summary: "lists entry points" }));
});

afterEach(async () => {
	vi.restoreAllMocks();
	setAgentsHubArchitectRunner(undefined);
	setPresetEvalRunner(undefined);
	setAgentDir(originalAgentDir);
	await removeWithRetries(testAgentDir).catch(() => {});
});

describe("hub managed-preset operator lifecycle (C3)", () => {
	it("generate → draft → inspect → evaluate → promote → rollback with authored intact and settings untouched", async () => {
		// Failure mode: the hub generated on shared Settings and wrote
		// authored files directly, bypassing revision history, eval state,
		// and restrictions — failures were silent and config bytes drifted.
		mockAgents([{ name: "dev", description: "Development agent", source: "project" }]);
		const settings = Settings.isolated();
		const setSpy = vi.spyOn(settings, "set");
		const filesBefore = await walkFiles(tempCwd);

		// Generate through the hub create flow into the managed scope.
		const hub = await createHub(settings);
		hub.handleInput("\x1b[B"); // dev → + New agent…
		hub.handleInput("\r");
		type(hub, "a tireless code scout");
		hub.handleInput("\t"); // project → user
		hub.handleInput("\t"); // user → managed
		hub.handleInput("\x11"); // Ctrl+Q generate
		await waitFor("architect review", () => strip(hub).includes("Review generated agent"));
		expect(strip(hub)).toContain("Scope: managed");

		// Architect isolation: runtime-only clone, shared settings untouched.
		expect(capturedRuntimeSettings).toBeDefined();
		expect(capturedRuntimeSettings).not.toBe(settings);
		expect(capturedRuntimeSettings?.get("autolearn.enabled")).toBe(false);
		expect(setSpy).not.toHaveBeenCalled();

		// Save mints a managed draft; authored tree is byte-identical.
		hub.handleInput("\r");
		await waitFor("managed draft notice", () => strip(hub).includes("Drafted managed preset"));
		expect(await walkFiles(tempCwd)).toEqual(filesBefore);
		const inspected = await listPresetRevisions(SCOUT_SPEC.identifier);
		expect(inspected.active).toBeNull();
		expect(inspected.revisions).toHaveLength(1);
		const draftId = inspected.revisions[0]?.id;
		expect(draftId).toMatch(/^rev-/);
		if (!draftId) throw new Error("managed draft revision missing");
		expect(inspected.revisions[0]?.evaluations).toHaveLength(0);

		// Inspect + evaluate + promote + rollback through the revision manager.
		mockAgents([
			{ name: "dev", description: "Development agent", source: "project" },
			{
				name: SCOUT_SPEC.identifier,
				description: SCOUT_SPEC.whenToUse,
				source: "user",
				filePath: path.join(testAgentDir, "agent", "managed-presets", `${SCOUT_SPEC.identifier}.md`),
			},
		]);
		const hub2 = await createHub(settings);
		await waitFor("managed agent listed", () => strip(hub2).includes(SCOUT_SPEC.identifier));
		hub2.handleInput("\x1b[B"); // dev → managed agent
		hub2.handleInput("\r"); // agent strip
		expect(strip(hub2)).toContain("revisions…");
		hub2.handleInput("\x1b[C");
		hub2.handleInput("\x1b[C");
		hub2.handleInput("\x1b[C"); // model → prewalk → advisor → revisions…
		hub2.handleInput("\r"); // revision manager
		await waitFor("revision listed", () => strip(hub2).includes(draftId ?? "no-such-rev"));
		expect(strip(hub2)).toContain("unevaluated");

		hub2.handleInput("\r"); // revision actions
		expect(strip(hub2)).toContain("evaluate…");
		hub2.handleInput("\r"); // evaluate… → task input
		type(hub2, "list the repo root");
		hub2.handleInput("\r"); // → outcome input
		type(hub2, "a file listing appears");
		hub2.handleInput("\r"); // run evaluation
		await waitFor("evaluation recorded", () => strip(hub2).includes("passing"));
		const evaluated = await listPresetRevisions(SCOUT_SPEC.identifier);
		expect(evaluated.revisions[0]?.evaluations).toHaveLength(1);

		hub2.handleInput("\r"); // revision actions
		hub2.handleInput("\x1b[C"); // evaluate… → promote
		hub2.handleInput("\r");
		await waitFor("promoted active", async () => (await listPresetRevisions(SCOUT_SPEC.identifier)).active !== null);
		const promoted = await listPresetRevisions(SCOUT_SPEC.identifier);
		expect(promoted.active).toBe(draftId);
		// The manager reopens asynchronously after promote (refresh +
		// revision reload): await the visible active marker, not just the
		// committed pointer.
		await waitFor("manager shows active", () => strip(hub2).includes("[active]"));

		// A second draft sets up rollback to the previously-active revision.
		const second = await createPresetDraft({
			name: SCOUT_SPEC.identifier,
			description: SCOUT_SPEC.whenToUse,
			systemPrompt: `${SCOUT_SPEC.systemPrompt} Second pass.`,
		});
		hub2.handleInput("\x1b"); // close manager
		hub2.handleInput("\r"); // agent strip
		hub2.handleInput("\x1b[C");
		hub2.handleInput("\x1b[C");
		hub2.handleInput("\x1b[C"); // → revisions…
		hub2.handleInput("\r");
		await waitFor("second draft listed", () => strip(hub2).includes(second.id));
		const orderForSecond = (await listPresetRevisions(SCOUT_SPEC.identifier)).revisions.map(rev => rev.id);
		for (let i = 0; i < orderForSecond.indexOf(second.id); i++) hub2.handleInput("\x1b[B");
		hub2.handleInput("\r"); // actions for the second draft
		hub2.handleInput("\x1b[C"); // → promote
		hub2.handleInput("\r");
		await waitFor(
			"second promoted",
			async () => (await listPresetRevisions(SCOUT_SPEC.identifier)).active === second.id,
		);
		// The manager reopens asynchronously after promote: proceed only
		// once it shows the second revision as active, so later navigation
		// targets settled rows instead of a mid-reopen list.
		await waitFor("manager shows second active", () => strip(hub2).includes(`active ${second.id}`));
		// The manager reopened fresh at index 0: move to the first revision, roll back.
		const orderForRollback = (await listPresetRevisions(SCOUT_SPEC.identifier)).revisions.map(rev => rev.id);
		for (let i = 0; i < orderForRollback.indexOf(draftId ?? ""); i++) hub2.handleInput("\x1b[B");
		hub2.handleInput("\r"); // actions
		hub2.handleInput("\x1b[C");
		hub2.handleInput("\x1b[C"); // → rollback
		hub2.handleInput("\r");
		await waitFor("rolled back", async () => (await listPresetRevisions(SCOUT_SPEC.identifier)).active === draftId);
		const rolledBack = await listPresetRevisions(SCOUT_SPEC.identifier);
		expect(rolledBack.active).toBe(draftId);
		const liveFile = await Bun.file(
			path.join(testAgentDir, "agent", "managed-presets", `${SCOUT_SPEC.identifier}.md`),
		).text();
		const firstContent = evaluated.revisions.find(rev => rev.id === draftId)?.content;
		if (firstContent === undefined) throw new Error("first revision content missing");
		expect(liveFile).toBe(firstContent);

		// Authored tree still untouched; shared settings still unmutated.
		expect(await walkFiles(tempCwd)).toEqual(filesBefore);
		expect(setSpy).not.toHaveBeenCalled();
	}, 60000);

	it("failing evaluations block promotion with an actionable error and settings still untouched", async () => {
		// Failure mode: a failing revision promotes silently, or the
		// restriction surfaces as a dead control with no reason.
		mockAgents([{ name: "dev", description: "Development agent", source: "project" }]);
		const settings = Settings.isolated();
		const setSpy = vi.spyOn(settings, "set");
		const draft = await createPresetDraft({
			name: "hub-failing-case",
			description: "Use this agent when exercising the failing-evaluation guard.",
			systemPrompt: "You always fail gracefully.",
		});
		setPresetEvalRunner(async () => ({ passed: false, summary: "wrong files listed" }));
		await evaluatePresetRevision("hub-failing-case", draft.id, {
			task: "list the repo root",
			expectedOutcome: "a file listing appears",
		});
		mockAgents([
			{ name: "dev", description: "Development agent", source: "project" },
			{
				name: "hub-failing-case",
				description: "Use this agent when exercising the failing-evaluation guard.",
				source: "user",
				filePath: path.join(testAgentDir, "agent", "managed-presets", "hub-failing-case.md"),
			},
		]);
		const hub = await createHub(settings);
		hub.handleInput("\x1b[B");
		hub.handleInput("\r"); // agent strip for hub-failing-case
		hub.handleInput("\x1b[C");
		hub.handleInput("\x1b[C");
		hub.handleInput("\x1b[C"); // → revisions…
		hub.handleInput("\r");
		await waitFor("failing revision listed", () => strip(hub).includes("failing (1)"));
		hub.handleInput("\r"); // actions
		hub.handleInput("\x1b[C"); // → promote
		hub.handleInput("\r");
		await waitFor("promotion refused", () => strip(hub).includes("failing evaluations"));
		const state = await listPresetRevisions("hub-failing-case");
		expect(state.active).toBeNull();
		expect(setSpy).not.toHaveBeenCalled();
	}, 60000);

	it("keeps authored project/user saves byte-identical in behavior (no managed writes)", async () => {
		// Failure mode: the managed reroute changes the authored save path
		// (different bytes, moved files, lost scope selection).
		nextSpec = AUTHORED_SPEC;
		mockAgents([{ name: "dev", description: "Development agent", source: "project" }]);
		const settings = Settings.isolated();
		const hub = await createHub(settings);
		hub.handleInput("\x1b[B"); // + New agent…
		hub.handleInput("\r");
		type(hub, "an authored helper");
		hub.handleInput("\x11");
		await waitFor("architect review", () => strip(hub).includes("Review generated agent"));
		expect(strip(hub)).toContain("Scope: project");
		hub.handleInput("\r");
		await waitFor("authored save", () => strip(hub).includes(`Created agent ${AUTHORED_SPEC.identifier} at`));
		const managed = await fs.readdir(path.join(testAgentDir, "agent", "managed-presets")).catch(() => []);
		expect(managed.filter(name => name.includes(AUTHORED_SPEC.identifier))).toEqual([]);
	}, 60000);
});
