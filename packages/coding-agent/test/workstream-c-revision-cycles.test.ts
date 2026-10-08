/**
 * Workstream C (U4a/T13) — full operator cycles for preset and skill
 * revisions: history, inspect, evaluate, cancel, promote, rollback,
 * conflicts, and restrictions.
 *
 * Contracts (component surfaces only; services are owner-owned):
 * - Skill cycle runs end to end through SkillRevisionsComponent: draft →
 *   evaluate → promote → second draft → promote → rollback, with the live
 *   file tracking the active pointer byte-for-byte.
 * - Failed evaluations block promotion and unevaluated promotion requires
 *   the explicit disclosure contract, for both kinds.
 * - Stale expected-active pointers reject as revision conflicts with zero
 *   partial effects, for both kinds.
 * - Effective restrictions shown by inspect (preset) / the skill manager
 *   equal the restraints the production executor runs with (executed child
 *   toolset), verified through deterministic transport.
 */
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import * as ai from "@harvest/pi-ai";
import { getBundledModels } from "@harvest/pi-catalog/models";
import { KeybindingsManager } from "@harvest/pi-coding-agent/config/keybindings";
import type { ModelRegistry } from "@harvest/pi-coding-agent/config/model-registry";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import { installManagedEvalExecutors } from "@harvest/pi-coding-agent/autolearn/eval-executor";
import {
	createSkillDraft,
	getManagedSkillsDir,
	listSkillRevisions,
	promoteSkillRevision,
	setSkillEvalRunner,
} from "@harvest/pi-coding-agent/autolearn/managed-skills";
import { AgentsHubComponent } from "@harvest/pi-coding-agent/modes/components/agents-hub";
import { SkillRevisionsComponent } from "@harvest/pi-coding-agent/modes/components/skill-revisions";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { CreateAgentSessionResult } from "@harvest/pi-coding-agent/sdk";
import * as sdkModule from "@harvest/pi-coding-agent/sdk";
import * as discovery from "@harvest/pi-coding-agent/task/discovery";
import {
	createPresetDraft,
	listPresetRevisions,
	promotePresetRevision,
	setPresetEvalRunner,
} from "@harvest/pi-coding-agent/task/agents";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import type { TUI } from "@harvest/pi-tui";
import { setKeybindings, visibleWidth } from "@harvest/pi-tui";
import { removeWithRetries } from "@harvest/pi-utils";
import { getAgentDir, setAgentDir } from "@harvest/pi-utils/dirs";

const ANSI_PATTERN = /\x1b\[[0-?]*[ -/]*[@-~]/g;
const tuiStub = { requestRender: () => {}, terminal: { rows: 30 } } as unknown as TUI;

const MODEL_A = getBundledModels("anthropic").find(m => m.id === "claude-3-haiku-20240307")!;
const SELECTOR_A = `${MODEL_A.provider}/${MODEL_A.id}`;

if (!MODEL_A) {
	throw new Error("Expected catalog fixture model to exist");
}

let tempCwd: string;
let tempRoot: string;
let testAgentDir: string;
let originalAgentDir: string;

function strip(component: { render(width: number): readonly string[] }): string {
	return component.render(120).join("\n").replace(ANSI_PATTERN, "");
}

function type(component: { handleInput(data: string): void }, text: string): void {
	for (const char of text) component.handleInput(char);
}

async function waitFor(label: string, predicate: () => boolean | Promise<boolean>, timeoutMs = 10000): Promise<void> {
	const start = Date.now();
	for (;;) {
		if (await predicate()) return;
		if (Date.now() - start > timeoutMs) throw new Error(`timed out waiting for ${label}`);
		await Bun.sleep(25);
	}
}

function makeParent(settings: Settings): ToolSession {
	return {
		cwd: tempCwd,
		hasUI: false,
		skipPythonPreflight: true,
		getSessionFile: () => null,
		getSessionSpawns: () => "*",
		settings,
		taskDepth: 0,
		getSessionId: () => "parent-session",
		getActiveModel: () => MODEL_A,
		getActiveModelString: () => SELECTOR_A,
		modelRegistry: {
			getAvailable: () => [MODEL_A],
			getApiKey: async () => "stored-key",
		} as unknown as ModelRegistry,
	};
}

function makeSettings(): { settings: Settings; setSpy: ReturnType<typeof vi.spyOn> } {
	const settings = Settings.isolated({ "autolearn.enabled": true });
	settings.override("modelRoles", { task: SELECTOR_A });
	return { settings, setSpy: vi.spyOn(settings, "set") };
}

interface StubChild {
	prompts: string[];
	disposed: number;
	createdOptions?: unknown;
}

function stubChild(output: string, child: StubChild): void {
	vi.spyOn(sdkModule, "createAgentSession").mockImplementation(async options => {
		child.createdOptions = options;
		return {
			session: {
				prompt: async (text: string) => {
					child.prompts.push(text);
				},
				state: { messages: [{ role: "assistant", content: [{ type: "text", text: output }] }] },
				sessionManager: { getSessionId: () => "child-run-1" },
				dispose: async () => {
					child.disposed++;
				},
			},
		} as unknown as CreateAgentSessionResult;
	});
}

function stubGrading(firstLine: string, basis: string): void {
	vi.spyOn(ai, "completeSimple").mockResolvedValue({
		stopReason: "stop",
		content: [{ type: "text", text: `${firstLine}\n${basis}` }],
	} as never);
}

/** Drive the skill manager from list through task/outcome submission. */
async function driveSkillEval(manager: SkillRevisionsComponent, task: string, outcome: string): Promise<void> {
	manager.handleInput("e");
	type(manager, task);
	manager.handleInput("\r");
	type(manager, outcome);
	manager.handleInput("\r");
}

beforeAll(async () => {
	await initTheme(false);
	tempCwd = await fs.mkdtemp(path.join(os.tmpdir(), "omp-u4a-cwd-"));
	tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "omp-u4a-agentdir-"));
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
	setPresetEvalRunner(undefined);
	setSkillEvalRunner(undefined);
	installManagedEvalExecutors();
});

afterEach(async () => {
	vi.restoreAllMocks();
	setPresetEvalRunner(undefined);
	setSkillEvalRunner(undefined);
	setAgentDir(originalAgentDir);
	await removeWithRetries(testAgentDir).catch(() => {});
});

describe("U4a skill revision operator cycle", () => {
	it("shows evaluation input at one row and keeps the full task/outcome while inspection reaches the last content line", async () => {
		const name = "visible-revision-input";
		await createSkillDraft({
			name,
			description: "Display regression",
			body: `${"detail\n".repeat(20)}Final inspection sentinel`,
		});
		const observed: { task?: string; expectedOutcome?: string } = {};
		setSkillEvalRunner(async context => {
			observed.task = context.task;
			observed.expectedOutcome = context.expectedOutcome;
			return { passed: true, summary: "Expected output observed" };
		});
		const manager = await SkillRevisionsComponent.create(tuiStub, name);
		try {
			manager.setMaxHeight(1);
			manager.handleInput("e");
			type(manager, "full evaluation task Z");
			const tiny = manager.render(1);
			expect(tiny).toHaveLength(1);
			expect(visibleWidth(tiny[0]!)).toBeLessThanOrEqual(1);
			manager.setMaxHeight(4);
			expect(strip(manager)).toContain("full evaluation task Z");
			manager.handleInput("\r");
			type(manager, "complete expected outcome Y");
			expect(strip(manager)).toContain("complete expected outcome Y");
			manager.handleInput("\r");
			await waitFor("evaluation receives untruncated inputs", () => observed.expectedOutcome !== undefined);
			expect(observed).toEqual({ task: "full evaluation task Z", expectedOutcome: "complete expected outcome Y" });
			await waitFor("evaluation completed", () => strip(manager).includes("passing"));
			manager.handleInput("i");
			await waitFor("inspection loaded", () => strip(manager).includes("description:"));
			manager.handleInput("\x1b[F");
			expect(strip(manager)).toContain("Final inspection sentinel");
			manager.handleInput("\x1b");
			expect(strip(manager)).toContain("passing");
		} finally {
			manager.dispose();
		}
	});

	it("closing a running revision view aborts execution, records no result and fences late redraws", async () => {
		const name = "closed-revision-run";
		const draft = await createSkillDraft({ name, description: "Cancellation regression", body: "Pending output" });
		let signal: AbortSignal | undefined;
		const outcome = Promise.withResolvers<{ passed: boolean; summary: string }>();
		setSkillEvalRunner(context => {
			signal = context.signal;
			return outcome.promise;
		});
		let redraws = 0;
		const host = {
			terminal: { rows: 12 },
			requestRender: () => {
				redraws++;
			},
		} as unknown as TUI;
		const manager = await SkillRevisionsComponent.create(host, name);
		await driveSkillEval(manager, "run task", "observe output");
		await waitFor("executor begins", () => signal !== undefined);
		manager.dispose();
		const countAtClose = redraws;
		expect(signal?.aborted).toBe(true);
		outcome.resolve({ passed: true, summary: "Late output" });
		const state = await listSkillRevisions(name);
		expect(state.revisions.find(revision => revision.id === draft.id)?.evaluations).toHaveLength(0);
		await Bun.sleep(10);
		expect(redraws).toBe(countAtClose);
		expect(state.active).toBeNull();
	});
	it("draft → evaluate → promote → second draft → promote → rollback tracks the live file", async () => {
		// Failure mode: the skill surface offers model-callable verbs but no
		// operable management UI — history, eval state, and rollback are
		// reachable only by hand-rolled service calls with no rendered states.
		const { settings, setSpy } = makeSettings();
		const parent = makeParent(settings);
		const child: StubChild = { prompts: [], disposed: 0 };
		stubChild("observed skill output", child);
		stubGrading("PASS", "Output matches the expected outcome.");

		const first = await createSkillDraft({
			name: "u4-skill-cycle",
			description: "When to use the U4a cycle skill.",
			body: "First body.",
		});
		const manager = await SkillRevisionsComponent.create(
			tuiStub,
			"u4-skill-cycle",
			{ parent },
			{ onClose: () => {} },
		);
		await waitFor("draft listed", () => strip(manager).includes(first.id));
		expect(strip(manager)).toContain("unevaluated");

		await driveSkillEval(manager, "do the thing", "the thing is done");
		await waitFor("evaluation recorded", () => strip(manager).includes("passing"));

		manager.handleInput("p");
		await waitFor("promoted", () => strip(manager).includes("[active]"));
		let state = await listSkillRevisions("u4-skill-cycle");
		expect(state.active).toBe(first.id);

		// Restrictions shown by inspect equal the executed restraints.
		manager.handleInput("i");
		await waitFor("inspect restrictions", () => strip(manager).includes("effective restrictions"));
		expect(strip(manager)).toContain("tools=[read, glob, grep]");
		expect(strip(manager)).toContain("model=@task");
		expect(child.createdOptions).toMatchObject({ toolNames: ["read", "glob", "grep"], restrictToolNames: true });
		manager.handleInput("\x1b"); // back to list

		// Second draft, evaluated and promoted; then roll back to the first.
		const second = await createSkillDraft({
			name: "u4-skill-cycle",
			description: "When to use the U4a cycle skill.",
			body: "Second body.",
		});
		manager.handleInput("r");
		await waitFor("second listed", () => strip(manager).includes(second.id));
		manager.handleInput("\x1b[B"); // → second draft (oldest-first)
		await driveSkillEval(manager, "do the other thing", "the other thing is done");
		// Wait on the second revision's service record, not the rendered
		// "passing (1)" string: the first revision already renders that text,
		// so a string wait would fire before the second run settles and the
		// promotion below would hit the pending-evaluation guard.
		await waitFor("second passing", async () => {
			const { revisions } = await listSkillRevisions("u4-skill-cycle");
			return (revisions.find(rev => rev.id === second.id)?.evaluations.length ?? 0) === 1;
		});
		manager.handleInput("p");
		await waitFor("second active", async () => (await listSkillRevisions("u4-skill-cycle")).active === second.id);

		manager.handleInput("\x1b[A"); // → first revision
		manager.handleInput("b");
		await waitFor("rolled back", async () => (await listSkillRevisions("u4-skill-cycle")).active === first.id);
		state = await listSkillRevisions("u4-skill-cycle");
		expect(state.active).toBe(first.id);
		const liveFile = await Bun.file(path.join(getManagedSkillsDir(), "u4-skill-cycle", "SKILL.md")).text();
		const firstContent = state.revisions.find(rev => rev.id === first.id)?.content;
		if (firstContent === undefined) throw new Error("first revision content missing");
		expect(liveFile).toBe(firstContent);
		expect(setSpy).not.toHaveBeenCalled();
	});

	it("failing evaluations block promotion and unevaluated promotion discloses explicitly", async () => {
		// Failure mode: a failing skill revision promotes silently, or an
		// unevaluated promotion is indistinguishable from an evaluated one.
		const { settings, setSpy } = makeSettings();
		const parent = makeParent(settings);
		const child: StubChild = { prompts: [], disposed: 0 };
		stubChild("wrong output", child);
		stubGrading("FAIL", "The output does not match.");
		const failing = await createSkillDraft({
			name: "u4-skill-guard",
			description: "When to use the U4a guard skill.",
			body: "Failing body.",
		});
		const manager = await SkillRevisionsComponent.create(
			tuiStub,
			"u4-skill-guard",
			{ parent },
			{ onClose: () => {} },
		);
		await waitFor("draft listed", () => strip(manager).includes(failing.id));
		await driveSkillEval(manager, "do the thing", "the thing is done");
		await waitFor("failing state", () => strip(manager).includes("failing"));

		manager.handleInput("p");
		await waitFor("promotion refused", () => strip(manager).includes("failing evaluations"));
		let state = await listSkillRevisions("u4-skill-guard");
		expect(state.active).toBeNull();

		// An unevaluated draft promotes only under the disclosure contract.
		const fresh = await createSkillDraft({
			name: "u4-skill-guard",
			description: "When to use the U4a guard skill.",
			body: "Fresh body.",
		});
		manager.handleInput("r");
		await waitFor("fresh listed", () => strip(manager).includes(fresh.id));
		manager.handleInput("\x1b[B");
		manager.handleInput("p");
		await waitFor(
			"disclosed promotion",
			async () => (await listSkillRevisions("u4-skill-guard")).active === fresh.id,
		);
		await waitFor("disclosure notice", () => strip(manager).includes("promoted unevaluated — disclosed"));
		state = await listSkillRevisions("u4-skill-guard");
		expect(state.active).toBe(fresh.id);
		expect(setSpy).not.toHaveBeenCalled();
	});

	it("stale expected-active pointers reject as conflicts with zero partial effects", async () => {
		// Failure mode: a draft minted against a stale active pointer
		// silently overwrites concurrent work instead of conflicting.
		const first = await createSkillDraft({
			name: "u4-skill-stale",
			description: "When to use the stale skill.",
			body: "First.",
		});
		await promoteSkillRevision("u4-skill-stale", first.id, { discloseUnevaluated: true });
		await expect(
			createSkillDraft({
				name: "u4-skill-stale",
				description: "When to use the stale skill.",
				body: "Concurrent.",
				expectedActive: "rev-stale-pointer",
			}),
		).rejects.toThrow(/Revision conflict/);
		await expect(
			createSkillDraft({
				name: "u4-skill-stale",
				description: "When to use the stale skill.",
				body: "Concurrent.",
				expectedActive: null,
			}),
		).rejects.toThrow(/Revision conflict/);
		// Zero partial effects: no draft leaked, pointer untouched.
		const state = await listSkillRevisions("u4-skill-stale");
		expect(state.active).toBe(first.id);
		expect(state.revisions).toHaveLength(1);

		const presetFirst = await createPresetDraft({
			name: "u4-stale-preset-probe",
			description: "Use this agent when probing stale pointers.",
			systemPrompt: "First.",
		});
		await promotePresetRevision("u4-stale-preset-probe", presetFirst.id, { discloseUnevaluated: true });
		await expect(
			createPresetDraft({
				name: "u4-stale-preset-probe",
				description: "Use this agent when probing stale pointers.",
				systemPrompt: "Concurrent.",
				expectedActive: "rev-stale-pointer",
			}),
		).rejects.toThrow(/Revision conflict/);
		const presetState = await listPresetRevisions("u4-stale-preset-probe");
		expect(presetState.active).toBe(presetFirst.id);
		expect(presetState.revisions).toHaveLength(1);
	});
});

describe("U4a preset restrictions equal executed behavior", () => {
	it("hub inspect shows the declared tools and the executor runs exactly them", async () => {
		// Failure mode: the operator sees one restraint set while the run
		// executes another (displayed restrictions ≠ executed behavior).
		vi.spyOn(discovery, "discoverAgents").mockResolvedValue({
			projectAgentsDir: null,
			agents: [{ name: "dev", description: "Development agent", source: "project", systemPrompt: "" }],
		});
		const { settings, setSpy } = makeSettings();
		const parent = makeParent(settings);
		const child: StubChild = { prompts: [], disposed: 0 };
		stubChild("restricted output", child);
		stubGrading("PASS", "Matches under restrictions.");
		await createPresetDraft({
			name: "u4-restriction-probe",
			description: "Use this agent when probing restrictions.",
			systemPrompt: "You stay restricted.",
			tools: ["read", "grep"],
		});
		const hub = await AgentsHubComponent.create(tuiStub, tempCwd, settings, {}, { onCancel: () => {} }, { parent });
		type(hub, "u4-restriction-probe");
		await waitFor("filtered", () => strip(hub).includes("u4-restriction-probe"));
		hub.handleInput("\r");
		hub.handleInput("\x1b[C");
		hub.handleInput("\x1b[C");
		hub.handleInput("\x1b[C");
		hub.handleInput("\r");
		const { revisions } = await listPresetRevisions("u4-restriction-probe");
		const revId = revisions[0]?.id;
		if (!revId) throw new Error("draft revision missing");
		await waitFor("revision listed", () => strip(hub).includes(revId));

		// Inspect shows the declared restraints before anything runs.
		hub.handleInput("\r"); // actions
		hub.handleInput("\x1b[C");
		hub.handleInput("\x1b[C");
		hub.handleInput("\x1b[C"); // evaluate → promote → rollback → inspect…
		hub.handleInput("\r");
		await waitFor("inspect restrictions", () => strip(hub).includes("effective restrictions"));
		// The agent contract normalizes explicit tool lists with "yield"
		// (discovery/helpers parseAgentFields); inspect shows the normalized
		// set — the same normalization the executor runs.
		expect(strip(hub)).toContain("tools=[read, grep, yield]");
		hub.handleInput("\x1b"); // back to list

		// Evaluate through the hub; the executed child carries exactly them.
		hub.handleInput("\r"); // actions
		hub.handleInput("\r"); // evaluate…
		type(hub, "stay restricted");
		hub.handleInput("\r");
		type(hub, "restriction holds");
		hub.handleInput("\r");
		await waitFor("evaluation recorded", () => strip(hub).includes("passing"));
		expect(child.createdOptions).toMatchObject({ toolNames: ["read", "grep", "yield"], restrictToolNames: true });
		expect(setSpy).not.toHaveBeenCalled();
	});
});
