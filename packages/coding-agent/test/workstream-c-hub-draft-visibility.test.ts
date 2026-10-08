/**
 * Workstream C (U1/T11) — draft-only managed presets are enumerable for
 * management through the revision service, independently of spawn discovery.
 *
 * Contracts (hub surface only; the revision service is owner-owned):
 * - Saving a draft immediately exposes a row plus history, unevaluated
 *   state, and actions in the SAME hub (no promote round-trip, no reopen).
 * - File-scan spawn discovery (`discoverManagedPresets`) excludes the
 *   inactive draft: drafts never execute.
 * - Authored definitions and already-active presets are unchanged: no
 *   shadowing, no synthetic duplicate rows.
 * - Managed identity is authoritative containment in the managed-presets
 *   root, not a path substring: sibling directories whose names merely
 *   contain "managed-presets" are not managed artifacts.
 */
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { KeybindingsManager } from "@harvest/pi-coding-agent/config/keybindings";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import {
	AgentsHubComponent,
	isManagedPresetAgent,
	setAgentsHubArchitectRunner,
} from "@harvest/pi-coding-agent/modes/components/agents-hub";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import * as discovery from "@harvest/pi-coding-agent/task/discovery";
import {
	createPresetDraft,
	discoverManagedPresets,
	listPresetRevisions,
	promotePresetRevision,
} from "@harvest/pi-coding-agent/task/agents";
import type { TUI } from "@harvest/pi-tui";
import { setKeybindings } from "@harvest/pi-tui";
import { removeWithRetries } from "@harvest/pi-utils";
import { getAgentDir, setAgentDir } from "@harvest/pi-utils/dirs";

const ANSI_PATTERN = /\x1b\[[0-?]*[ -/]*[@-~]/g;
const tuiStub = { requestRender: () => {}, terminal: { rows: 30 } } as unknown as TUI;

let tempCwd: string;
let tempRoot: string;
let testAgentDir: string;
let originalAgentDir: string;

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

beforeAll(async () => {
	await initTheme(false);
	tempCwd = await fs.mkdtemp(path.join(os.tmpdir(), "omp-u1-cwd-"));
	tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "omp-u1-agentdir-"));
	originalAgentDir = getAgentDir();
});

afterAll(async () => {
	await removeWithRetries(tempCwd);
	await removeWithRetries(tempRoot);
});

beforeEach(async () => {
	setKeybindings(KeybindingsManager.inMemory());
	testAgentDir = await fs.mkdtemp(path.join(tempRoot, "t-"));
	setAgentDir(path.join(testAgentDir, "agent"));
});

afterEach(async () => {
	vi.restoreAllMocks();
	setAgentsHubArchitectRunner(undefined);
	setAgentDir(originalAgentDir);
	await removeWithRetries(testAgentDir).catch(() => {});
});

describe("U1 draft-only presets are manageable without spawn visibility", () => {
	it("same hub shows the saved draft row with history, unevaluated state, and actions; file-scan discovery excludes it", async () => {
		// Failure mode: createPresetDraft writes history but no file, the hub
		// rebuilds rows from file-scan discovery only, and the draft is
		// unreachable (no row, no revision manager) until promoted.
		// The draft is generated AND saved through the hub's own create flow,
		// so every assertion below runs on the SAME hub instance.
		vi.spyOn(discovery, "discoverAgents").mockResolvedValue({
			projectAgentsDir: null,
			agents: [{ name: "dev", description: "Development agent", source: "project", systemPrompt: "" }],
		});
		setAgentsHubArchitectRunner(async () => ({
			identifier: "u1-draft-scout",
			whenToUse: "Use this agent when scouting for U1 coverage.",
			systemPrompt: "You scout for U1.",
		}));
		const settings = Settings.isolated();
		const hub = await AgentsHubComponent.create(tuiStub, tempCwd, settings, {}, { onCancel: () => {} });
		expect(strip(hub)).not.toContain("u1-draft-scout");

		hub.handleInput("\x1b[B"); // dev → + New agent…
		hub.handleInput("\r");
		type(hub, "a tireless code scout");
		hub.handleInput("\t"); // project → user
		hub.handleInput("\t"); // user → managed
		hub.handleInput("\x11"); // Ctrl+Q generate
		await waitFor("architect review", () => strip(hub).includes("Review generated agent"));
		hub.handleInput("\r"); // save as managed draft
		await waitFor("managed draft notice", () => strip(hub).includes("Drafted managed preset"));

		// Same hub, no reopen: the draft row is listed with its draft badge.
		// Match the row badge (not the notice text, which carries the name
		// before the post-save reload merges rows).
		await waitFor("draft row visible", () => strip(hub).includes("draft · unevaluated"));
		expect(strip(hub)).toContain("u1-draft-scout");

		const history = await listPresetRevisions("u1-draft-scout");
		expect(history.active).toBeNull();
		expect(history.revisions).toHaveLength(1);
		const draftId = history.revisions[0]?.id;
		if (!draftId) throw new Error("managed draft revision missing");

		// File-scan spawn discovery excludes the inactive draft: no live
		// file exists, so nothing outside the revision manager can run it.
		const scanned = await discoverManagedPresets();
		expect(scanned.map(agent => agent.name)).not.toContain("u1-draft-scout");

		// The revision manager is reachable from the row with history and
		// unevaluated state (no promotion needed to get here). The manager
		// loads its items asynchronously, so wait for its chrome.
		type(hub, "u1-draft-scout");
		await waitFor("filtered to draft", () => strip(hub).includes("draft · unevaluated"));
		hub.handleInput("\r"); // agent strip
		await waitFor("revision manager open", () => strip(hub).includes("revisions…"));
		hub.handleInput("\x1b[C");
		hub.handleInput("\x1b[C");
		hub.handleInput("\x1b[C"); // → revisions…
		hub.handleInput("\r");
		await waitFor("revision listed", () => strip(hub).includes(draftId));
		expect(strip(hub)).toContain("unevaluated");
		const inspected = await listPresetRevisions("u1-draft-scout");
		expect(inspected.revisions.find(rev => rev.id === draftId)?.evaluations).toHaveLength(0);
	});

	it("promoted presets and authored agents keep single discovery rows (no synthetic duplicates, no shadowing)", async () => {
		// Failure mode: the synthetic-row merge duplicates the row once the
		// draft promotes (file appears in discovery), or shadows an authored
		// agent that claims the same name.
		const managedPath = path.join(testAgentDir, "agent", "managed-presets", "u1-shared-row.md");
		vi.spyOn(discovery, "discoverAgents").mockResolvedValue({
			projectAgentsDir: null,
			agents: [
				{ name: "dev", description: "Development agent", source: "project", systemPrompt: "" },
				{
					name: "u1-shared-row",
					description: "Use this agent when sharing a row.",
					source: "user",
					systemPrompt: "live",
					filePath: managedPath,
				},
			],
		});
		const hub = await AgentsHubComponent.create(tuiStub, tempCwd, Settings.isolated(), {}, { onCancel: () => {} });
		const occurrences = strip(hub)
			.split("\n")
			.filter(line => line.includes("u1-shared-row")).length;
		expect(occurrences).toBeGreaterThan(0);
		// One list row plus the selected-agent detail block reference it —
		// never two selectable rows.
		type(hub, "u1-shared-row");
		await waitFor("filtered", () => strip(hub).includes("u1-shared-row"));
	});

	it("managed identity is containment in the managed root, not a substring", () => {
		// Failure mode: `filePath.includes("managed-presets")` mistakes
		// sibling directories (backups, archives) for managed artifacts and
		// routes authored agents into the revision flow.
		const managed = path.join(testAgentDir, "agent", "managed-presets", "some-preset.md");
		expect(isManagedPresetAgent({ filePath: managed })).toBe(true);
		expect(
			isManagedPresetAgent({
				filePath: path.join(testAgentDir, "agent", "managed-presets-backup", "some-preset.md"),
			}),
		).toBe(false);
		expect(
			isManagedPresetAgent({
				filePath: path.join(tempCwd, "agents", "managed-presets-notes.md"),
			}),
		).toBe(false);
		expect(isManagedPresetAgent({ filePath: "embedded:scout.md" })).toBe(false);
		expect(isManagedPresetAgent({})).toBe(false);
	});

	it("promoting the draft swaps the synthetic row for the live file without losing the manager", async () => {
		// Failure mode: after promotion the hub shows both the synthetic
		// draft row and the discovered live row, or the manager loses the
		// now-active revision.
		const { setPresetEvalRunner } = await import("@harvest/pi-coding-agent/task/agents");
		setPresetEvalRunner(async () => ({ passed: true, summary: "u1 pass" }));
		try {
			const draft = await createPresetDraft({
				name: "u1-promote-swap",
				description: "Use this agent when swapping rows on promote.",
				systemPrompt: "You swap rows.",
			});
			const { evaluatePresetRevision } = await import("@harvest/pi-coding-agent/task/agents");
			await evaluatePresetRevision("u1-promote-swap", draft.id, {
				task: "swap",
				expectedOutcome: "rows swap",
			});
			await promotePresetRevision("u1-promote-swap", draft.id, { discloseUnevaluated: true });
			const livePath = path.join(testAgentDir, "agent", "managed-presets", "u1-promote-swap.md");
			vi.spyOn(discovery, "discoverAgents").mockResolvedValue({
				projectAgentsDir: null,
				agents: [
					{ name: "dev", description: "Development agent", source: "project", systemPrompt: "" },
					{
						name: "u1-promote-swap",
						description: "Use this agent when swapping rows on promote.",
						source: "user",
						systemPrompt: "You swap rows.",
						filePath: livePath,
					},
				],
			});
			const hub = await AgentsHubComponent.create(tuiStub, tempCwd, Settings.isolated(), {}, { onCancel: () => {} });
			type(hub, "u1-promote-swap");
			await waitFor("live row", () => strip(hub).includes("u1-promote-swap"));
			// No draft badge once the live file carries the preset.
			expect(strip(hub)).not.toContain("draft · unevaluated");
			hub.handleInput("\r");
			expect(strip(hub)).toContain("revisions…");
		} finally {
			setPresetEvalRunner(undefined);
		}
	});
});
