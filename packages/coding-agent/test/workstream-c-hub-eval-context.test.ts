/**
 * Workstream C (U2/T12) — hub evaluations run through the installed
 * production evaluator with the owning session context.
 *
 * Contracts (hub surface only; executor internals are owner-owned):
 * - The hub threads its eval context (production ToolSession parent +
 *   AbortSignal) into every `evaluatePresetRevision` call: with a parent the
 *   production path runs and the "needs a parent session context" error never
 *   appears; without one the hub renders an unavailable state instead of
 *   running unscoped.
 * - Deterministic transport only at the seams (child session creation +
 *   grading completion, mirroring workstream B): the installed production
 *   runner is never substituted, so pass/fail/no-cred/no-model/cancel all
 *   exercise the real executor, restriction, provenance, and disposal paths.
 * - Running, passed, failed, cancelled, and unavailable states all render;
 *   operator cancel records nothing; late callbacks after close/dispose are
 *   fenced; settings bytes are unchanged on every outcome.
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
import { AgentsHubComponent } from "@harvest/pi-coding-agent/modes/components/agents-hub";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { CreateAgentSessionResult } from "@harvest/pi-coding-agent/sdk";
import * as sdkModule from "@harvest/pi-coding-agent/sdk";
import * as discovery from "@harvest/pi-coding-agent/task/discovery";
import { createPresetDraft, listPresetRevisions, setPresetEvalRunner } from "@harvest/pi-coding-agent/task/agents";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import type { TUI } from "@harvest/pi-tui";
import { setKeybindings } from "@harvest/pi-tui";
import { removeWithRetries } from "@harvest/pi-utils";
import { getAgentDir, setAgentDir } from "@harvest/pi-utils/dirs";

const ANSI_PATTERN = /\x1b\[[0-?]*[ -/]*[@-~]/g;
const tuiStub = { requestRender: () => {}, terminal: { rows: 30 } } as unknown as TUI;

const MODEL_A = getBundledModels("anthropic").find(m => m.id === "claude-3-haiku-20240307")!;
const MODEL_B = getBundledModels("openai").find(m => m.id === "daybreak-blue-latest")!;
const SELECTOR_A = `${MODEL_A.provider}/${MODEL_A.id}`;

if (!MODEL_A || !MODEL_B) {
	throw new Error("Expected catalog fixture models to exist");
}

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

/** Structural parent session: cwd/settings/registry/credentials/model source. */
function makeParent(settings: Settings, keys: Map<string, string>, extra: Partial<ToolSession> = {}): ToolSession {
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
			getAvailable: () => [MODEL_A, MODEL_B],
			getApiKey: async (model: { provider: string; id: string }) => keys.get(`${model.provider}/${model.id}`),
		} as unknown as ModelRegistry,
		...extra,
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
	messages: Array<{ role: string; content: Array<{ type: string; text: string }> }>;
	releasePrompt?: () => void;
	createdOptions?: unknown;
}

function assistantMessages(text: string): StubChild["messages"] {
	return [{ role: "assistant", content: [{ type: "text", text }] }];
}

/** Immediate child: records the task, answers with canned output. */
function stubImmediateChild(child: StubChild): void {
	vi.spyOn(sdkModule, "createAgentSession").mockImplementation(async options => {
		child.createdOptions = options;
		return {
			session: {
				prompt: async (text: string) => {
					child.prompts.push(text);
				},
				state: { messages: child.messages },
				sessionManager: { getSessionId: () => "child-run-1" },
				dispose: async () => {
					child.disposed++;
				},
			},
		} as unknown as CreateAgentSessionResult;
	});
}

/** Blocking child: the prompt stays in flight until the test releases it. */
function stubBlockingChild(child: StubChild): void {
	vi.spyOn(sdkModule, "createAgentSession").mockImplementation(async options => {
		child.createdOptions = options;
		return {
			session: {
				prompt: async (text: string) => {
					child.prompts.push(text);
					await new Promise<void>(resolve => {
						child.releasePrompt = resolve;
					});
				},
				state: { messages: child.messages },
				sessionManager: { getSessionId: () => "child-run-1" },
				dispose: async () => {
					child.disposed++;
					child.releasePrompt?.();
				},
			},
		} as unknown as CreateAgentSessionResult;
	});
}

function stubGrading(firstLine: string, basis: string): ReturnType<typeof vi.spyOn> {
	return vi.spyOn(ai, "completeSimple").mockResolvedValue({
		stopReason: "stop",
		content: [{ type: "text", text: `${firstLine}\n${basis}` }],
	} as never);
}

function mockDiscovery(): void {
	vi.spyOn(discovery, "discoverAgents").mockResolvedValue({
		projectAgentsDir: null,
		agents: [{ name: "dev", description: "Development agent", source: "project", systemPrompt: "" }],
	});
}

/** Drive the hub from a fresh draft row through task/outcome submission. */
async function driveHubEval(hub: AgentsHubComponent, name: string, task: string, outcome: string): Promise<void> {
	type(hub, name);
	await waitFor("filtered to draft", () => strip(hub).includes(name));
	hub.handleInput("\r"); // agent strip
	await waitFor("agent strip", () => strip(hub).includes("revisions…"));
	hub.handleInput("\x1b[C");
	hub.handleInput("\x1b[C");
	hub.handleInput("\x1b[C"); // → revisions…
	hub.handleInput("\r");
	const { revisions } = await listPresetRevisions(name);
	const revId = revisions[0]?.id;
	if (!revId) throw new Error("draft revision missing");
	await waitFor("revision listed", () => strip(hub).includes(revId));
	hub.handleInput("\r"); // revision actions
	await waitFor("evaluate action", () => strip(hub).includes("evaluate…"));
	hub.handleInput("\r"); // evaluate… → task input
	type(hub, task);
	hub.handleInput("\r"); // → outcome input
	type(hub, outcome);
	hub.handleInput("\r"); // run evaluation
}

beforeAll(async () => {
	await initTheme(false);
	tempCwd = await fs.mkdtemp(path.join(os.tmpdir(), "omp-u2-cwd-"));
	tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "omp-u2-agentdir-"));
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
	installManagedEvalExecutors();
});

afterEach(async () => {
	vi.restoreAllMocks();
	setPresetEvalRunner(undefined);
	setAgentDir(originalAgentDir);
	await removeWithRetries(testAgentDir).catch(() => {});
});

describe("U2 hub evaluation through the production executor", () => {
	it("passes with run/model/provenance recorded and settings untouched", async () => {
		// Failure mode: the hub fires evaluatePresetRevision with no parent,
		// so the production executor throws "needs a parent session context"
		// and zero evaluations ever record.
		mockDiscovery();
		const { settings, setSpy } = makeSettings();
		const parent = makeParent(settings, new Map([[SELECTOR_A, "stored-key"]]));
		const child: StubChild = { prompts: [], disposed: 0, messages: assistantMessages("observed the thing") };
		stubImmediateChild(child);
		const grading = stubGrading("PASS", "Matches the expected outcome exactly.");
		const draft = await createPresetDraft({
			name: "u2-eval-pass",
			description: "Use this agent when passing U2.",
			systemPrompt: "You pass.",
		});
		const hub = await AgentsHubComponent.create(tuiStub, tempCwd, settings, {}, { onCancel: () => {} }, { parent });
		await driveHubEval(hub, "u2-eval-pass", "do the thing", "the thing is done");
		await waitFor("evaluation recorded", () => strip(hub).includes("passing"));

		expect(child.prompts).toEqual(["do the thing"]);
		expect(grading).toHaveBeenCalledTimes(1);
		expect(grading.mock.calls[0]?.[2]).toMatchObject({ apiKey: "stored-key" });
		const { revisions } = await listPresetRevisions("u2-eval-pass");
		const record = revisions.find(r => r.id === draft.id)?.evaluations[0];
		expect(record?.passed).toBe(true);
		expect(record?.runId).toBe("child-run-1");
		expect(record?.sessionId).toBe("child-run-1");
		expect(record?.model).toBe(SELECTOR_A);
		expect(record?.modelBasis).toBe("role");
		expect(record?.summary).toMatch(/Matches the expected outcome exactly/);
		expect(record?.summary).toMatch(/observed the thing/);
		expect(child.disposed).toBe(1);
		// The restricted child inherits the revision's restraints: default
		// read/glob/grep toolset, isolated session, borrowed registry.
		expect(child.createdOptions).toMatchObject({ toolNames: ["read", "glob", "grep"], restrictToolNames: true });
		// Neither success nor failure may persist into configuration.
		expect(setSpy).not.toHaveBeenCalled();
		expect(settings.get("autolearn.enabled")).toBe(true);
	});

	it("fails with the grader basis recorded and settings still untouched", async () => {
		// Failure mode: a failing run is misrendered as passed, or its
		// failing record never lands and the revision stays promotable.
		mockDiscovery();
		const { settings, setSpy } = makeSettings();
		const parent = makeParent(settings, new Map([[SELECTOR_A, "stored-key"]]));
		const child: StubChild = { prompts: [], disposed: 0, messages: assistantMessages("observed other") };
		stubImmediateChild(child);
		stubGrading("FAIL", "The output lists the wrong files.");
		await createPresetDraft({
			name: "u2-eval-fail",
			description: "Use this agent when failing U2.",
			systemPrompt: "You fail.",
		});
		const hub = await AgentsHubComponent.create(tuiStub, tempCwd, settings, {}, { onCancel: () => {} }, { parent });
		await driveHubEval(hub, "u2-eval-fail", "do the thing", "the thing is done");
		await waitFor("evaluation failed", () => strip(hub).includes("failing"));
		hub.handleInput("\x1b"); // close manager → status shows the notice
		await waitFor("failure notice", () => strip(hub).includes("FAILED"));
		const { revisions } = await listPresetRevisions("u2-eval-fail");
		expect(revisions[0]?.evaluations[0]?.passed).toBe(false);
		expect(child.disposed).toBe(1);
		expect(setSpy).not.toHaveBeenCalled();
	});

	it("records unverifiable instead of passing when no credential resolves", async () => {
		// Failure mode: without a bearer the run is graded against nothing
		// and either throws mid-flight or records a misleading pass.
		mockDiscovery();
		const { settings, setSpy } = makeSettings();
		const parent = makeParent(settings, new Map());
		const child: StubChild = { prompts: [], disposed: 0, messages: assistantMessages("output without creds") };
		stubImmediateChild(child);
		const grading = stubGrading("PASS", "unreachable");
		await createPresetDraft({
			name: "u2-eval-nocred",
			description: "Use this agent when missing credentials.",
			systemPrompt: "You lack creds.",
		});
		const hub = await AgentsHubComponent.create(tuiStub, tempCwd, settings, {}, { onCancel: () => {} }, { parent });
		await driveHubEval(hub, "u2-eval-nocred", "do the thing", "the thing is done");
		await waitFor("evaluation recorded", () => strip(hub).includes("failing"));
		expect(grading).not.toHaveBeenCalled();
		const { revisions } = await listPresetRevisions("u2-eval-nocred");
		const record = revisions[0]?.evaluations[0];
		expect(record?.passed).toBe(false);
		expect(record?.summary).toMatch(/no API key/i);
		expect(child.disposed).toBe(1);
		expect(setSpy).not.toHaveBeenCalled();
	});

	it("surfaces an actionable error and records nothing when no model resolves", async () => {
		// Failure mode: an unresolvable model runs the revision on an
		// arbitrary fallback instead of staying unevaluated with a reason.
		mockDiscovery();
		const { settings, setSpy } = makeSettings();
		const parent = makeParent(settings, new Map(), {
			modelRegistry: { getAvailable: () => [] } as unknown as ModelRegistry,
			getActiveModel: () => undefined,
			getActiveModelString: () => undefined,
		});
		const created = vi.spyOn(sdkModule, "createAgentSession");
		await createPresetDraft({
			name: "u2-eval-nomodel",
			description: "Use this agent when missing models.",
			systemPrompt: "You lack models.",
		});
		const hub = await AgentsHubComponent.create(tuiStub, tempCwd, settings, {}, { onCancel: () => {} }, { parent });
		await driveHubEval(hub, "u2-eval-nomodel", "do the thing", "the thing is done");
		await waitFor("model error", () => strip(hub).includes("No evaluation model resolves"));
		expect(created).not.toHaveBeenCalled();
		const { revisions } = await listPresetRevisions("u2-eval-nomodel");
		expect(revisions[0]?.evaluations).toHaveLength(0);
		expect(setSpy).not.toHaveBeenCalled();
	});

	it("operator cancel records nothing and renders the cancelled state", async () => {
		// Failure mode: cancelling mid-run records a partial result (which
		// could then promote) or leaves the child live.
		mockDiscovery();
		const { settings, setSpy } = makeSettings();
		const parent = makeParent(settings, new Map([[SELECTOR_A, "stored-key"]]));
		const child: StubChild = { prompts: [], disposed: 0, messages: assistantMessages("late output") };
		stubBlockingChild(child);
		const grading = vi.spyOn(ai, "completeSimple");
		await createPresetDraft({
			name: "u2-eval-cancel",
			description: "Use this agent when cancelling U2.",
			systemPrompt: "You cancel.",
		});
		const hub = await AgentsHubComponent.create(tuiStub, tempCwd, settings, {}, { onCancel: () => {} }, { parent });
		type(hub, "u2-eval-cancel");
		await waitFor("filtered", () => strip(hub).includes("u2-eval-cancel"));
		hub.handleInput("\r");
		hub.handleInput("\x1b[C");
		hub.handleInput("\x1b[C");
		hub.handleInput("\x1b[C");
		hub.handleInput("\r");
		const { revisions } = await listPresetRevisions("u2-eval-cancel");
		const revId = revisions[0]?.id;
		if (!revId) throw new Error("draft revision missing");
		await waitFor("revision listed", () => strip(hub).includes(revId));
		hub.handleInput("\r");
		hub.handleInput("\r");
		type(hub, "do the thing");
		hub.handleInput("\r");
		type(hub, "the thing is done");
		hub.handleInput("\r");
		await waitFor("running state", () => strip(hub).includes("evaluating…"));
		await waitFor("child started", () => child.prompts.length === 1);

		hub.handleInput("\r"); // actions for the running revision
		await waitFor("cancel action", () => strip(hub).includes("cancel evaluation"));
		hub.handleInput("\r"); // cancel
		hub.handleInput("\x1b"); // close manager → status shows the notice
		await waitFor("cancelled notice", () => strip(hub).includes("cancelled"));

		expect(grading).not.toHaveBeenCalled();
		expect(child.disposed).toBe(1);
		const after = await listPresetRevisions("u2-eval-cancel");
		expect(after.revisions.find(r => r.id === revId)?.evaluations).toHaveLength(0);
		expect(setSpy).not.toHaveBeenCalled();
	});

	it("renders unavailable instead of running unscoped with no parent wired", async () => {
		// Failure mode: without a threaded parent the hub fires the
		// production evaluator anyway and surfaces a raw "needs a parent
		// session context" throw — or worse, runs unscoped.
		mockDiscovery();
		const { settings, setSpy } = makeSettings();
		const created = vi.spyOn(sdkModule, "createAgentSession");
		await createPresetDraft({
			name: "u2-eval-noparent",
			description: "Use this agent when parentless.",
			systemPrompt: "You lack parents.",
		});
		const hub = await AgentsHubComponent.create(tuiStub, tempCwd, settings, {}, { onCancel: () => {} });
		await driveHubEval(hub, "u2-eval-noparent", "do the thing", "the thing is done");
		await waitFor("unavailable state", () => strip(hub).includes("Evaluation unavailable"));
		expect(created).not.toHaveBeenCalled();
		const { revisions } = await listPresetRevisions("u2-eval-noparent");
		expect(revisions[0]?.evaluations).toHaveLength(0);
		expect(setSpy).not.toHaveBeenCalled();
	});

	it("fences late settlements on close and dispose", async () => {
		// Failure mode: a run settling after the manager closed (or the hub
		// disposed) reopens views or rewrites notices on a dead view.
		mockDiscovery();
		const { settings, setSpy } = makeSettings();
		const parent = makeParent(settings, new Map([[SELECTOR_A, "stored-key"]]));
		const child: StubChild = { prompts: [], disposed: 0, messages: assistantMessages("late but recorded") };
		stubBlockingChild(child);
		stubGrading("PASS", "Settles after close.");
		await createPresetDraft({
			name: "u2-eval-fence",
			description: "Use this agent when fencing U2.",
			systemPrompt: "You fence.",
		});
		const hub = await AgentsHubComponent.create(tuiStub, tempCwd, settings, {}, { onCancel: () => {} }, { parent });
		await driveHubEval(hub, "u2-eval-fence", "do the thing", "the thing is done");
		await waitFor("running state", () => strip(hub).includes("evaluating…"));
		await waitFor("child started", () => child.prompts.length === 1);

		hub.handleInput("\x1b"); // close manager mid-run (fences UI updates)
		hub.dispose();
		child.releasePrompt?.();
		await waitFor("run recorded", async () => {
			const { revisions } = await listPresetRevisions("u2-eval-fence");
			return (revisions[0]?.evaluations.length ?? 0) === 1;
		});
		await Bun.sleep(100);
		// The run completed and recorded at the service level, but the dead
		// view was never reopened and no notice was rewritten onto it.
		expect(strip(hub)).not.toContain("Evaluation of");
		expect(strip(hub)).not.toContain("Revisions for");
		expect(child.disposed).toBe(1);
		expect(setSpy).not.toHaveBeenCalled();
	});
});
