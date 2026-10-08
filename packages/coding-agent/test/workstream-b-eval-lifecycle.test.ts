/**
 * Workstream B: revisions + evaluation lifecycle.
 *
 * Each test names the failure mode it guards: production evaluation through
 * real restricted execution (no injected runner), credential precedence and
 * unavailability, grounded pass/fail with recorded run/model/evidence,
 * cancellation at every phase (never passes, never promotes), shared model
 * resolution branches, promotion pausing for in-flight evaluations, lease
 * isolation between evaluations, recovery coherence, merge-inside-transaction
 * survival, and pin-aware retention.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import * as ai from "@harvest/pi-ai";
import { getBundledModels } from "@harvest/pi-catalog/models";
import {
	installManagedEvalExecutors,
	resolveEvalChildModel,
	resolveEvalCredential,
} from "@harvest/pi-coding-agent/autolearn/eval-executor";
import {
	clearSkillPinsForTests,
	createSkillDraft,
	evaluateSkillRevision,
	getSkillPins,
	getManagedSkillsDir,
	hasLiveSkillEvalPin,
	listSkillRevisions,
	pinActiveSkillsForRun,
	pruneSkillRevisions,
	setSkillEvalRunner,
	unpinActiveSkillsForRun,
} from "@harvest/pi-coding-agent/autolearn/managed-skills";
import { createDraftRevision, readActivePointer } from "@harvest/pi-coding-agent/autolearn/revisions";
import { type SettingPath, Settings } from "@harvest/pi-coding-agent/config/settings";
import type { ModelRegistry } from "@harvest/pi-coding-agent/config/model-registry";
import type { CreateAgentSessionResult } from "@harvest/pi-coding-agent/sdk";
import * as sdkModule from "@harvest/pi-coding-agent/sdk";
import {
	clearPresetPinsForTests,
	createPresetDraft,
	discoverManagedPresets,
	evaluatePresetRevision,
	getManagedPresetsDir,
	getPresetPins,
	hasLivePresetEvalPin,
	listPresetRevisions,
	setPresetEvalRunner,
	writeManagedPreset,
} from "@harvest/pi-coding-agent/task/agents";
import { ManageSkillTool } from "@harvest/pi-coding-agent/tools/manage-skill";
import { PresetsTool } from "@harvest/pi-coding-agent/tools/presets";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import { removeWithRetries } from "@harvest/pi-utils";
import { getAgentDir, setAgentDir } from "@harvest/pi-utils/dirs";

const SKILL_DESC = "When to use the lifecycle probe skill.";
const PRESET_DESC = "Use this agent when probing the eval lifecycle.";
const PRESET_BODY = "You are a lifecycle probe. Answer with the observed facts.";

const MODEL_A = getBundledModels("anthropic").find(m => m.id === "claude-3-haiku-20240307")!;
const MODEL_B = getBundledModels("openai").find(m => m.id === "daybreak-blue-latest")!;
const SELECTOR_A = `${MODEL_A.provider}/${MODEL_A.id}`;
const SELECTOR_B = `${MODEL_B.provider}/${MODEL_B.id}`;

if (!MODEL_A || !MODEL_B) {
	throw new Error("Expected catalog fixture models to exist");
}

function makeToolSession(
	settingsOverrides: Partial<Record<SettingPath, unknown>> = {},
	extra: Partial<ToolSession> = {},
): ToolSession {
	return {
		cwd: "/tmp/b-lifecycle",
		hasUI: false,
		skipPythonPreflight: true,
		getSessionFile: () => null,
		getSessionSpawns: () => "*",
		settings: Settings.isolated(settingsOverrides),
		...extra,
	};
}

/**
 * Poll an in-flight assertion until it holds (bounded). Evaluation startup
 * re-reads a just-committed revision file, which can stall briefly when it
 * piles onto background retention I/O; a fixed sleep would flake on slow
 * machines while this waits exactly as long as needed (up to the deadline).
 */
async function waitForInFlight(check: () => void, timeoutMs = 15000): Promise<void> {
	const start = Date.now();
	let last: unknown;
	for (;;) {
		try {
			check();
			return;
		} catch (err) {
			last = err;
			if (Date.now() - start > timeoutMs) throw last;
			await Bun.sleep(10);
		}
	}
}

/** Minimal structural model registry: availability plus stored-credential lookup. */
function stubRegistry(models: (typeof MODEL_A)[], keys: Map<string, string>): ModelRegistry {
	return {
		getAvailable: () => [...models],
		getApiKey: async (model: { provider: string; id: string }) => keys.get(`${model.provider}/${model.id}`),
	} as unknown as ModelRegistry;
}

interface StubChild {
	prompts: string[];
	disposed: number;
	messages: Array<{ role: string; content: Array<{ type: string; text: string }> }>;
	releasePrompt?: () => void;
}

/** Stub eval child session: prompt records and optionally blocks until the test releases it. */
function stubChildSession(child: StubChild): unknown {
	return {
		prompt: async (text: string) => {
			child.prompts.push(text);
			if (child.releasePrompt) {
				await new Promise<void>(resolve => {
					child.releasePrompt = resolve;
				});
			}
		},
		state: { messages: child.messages },
		sessionManager: { getSessionId: () => "child-run-1" },
		dispose: async () => {
			child.disposed++;
			child.releasePrompt?.();
		},
	};
}

function assistantMessages(text: string): StubChild["messages"] {
	return [{ role: "assistant", content: [{ type: "text", text }] }];
}

describe("workstream B eval lifecycle", () => {
	let tempHome: string;
	let tempCwd: string;
	let originalAgentDir: string;

	beforeEach(async () => {
		originalAgentDir = getAgentDir();
		tempHome = await fs.mkdtemp(path.join(os.tmpdir(), "omp-b-life-"));
		tempCwd = await fs.mkdtemp(path.join(os.tmpdir(), "omp-b-life-cwd-"));
		setAgentDir(path.join(tempHome, ".omp", "agent"));
		setSkillEvalRunner(undefined);
		setPresetEvalRunner(undefined);
		clearSkillPinsForTests();
		clearPresetPinsForTests();
		// Install production executors after clearing the injected-runner
		// slots: tests below exercise the real path with no optional callback.
		installManagedEvalExecutors();
	});

	afterEach(async () => {
		vi.restoreAllMocks();
		setAgentDir(originalAgentDir);
		setSkillEvalRunner(undefined);
		setPresetEvalRunner(undefined);
		clearSkillPinsForTests();
		clearPresetPinsForTests();
		await removeWithRetries(tempHome);
		await removeWithRetries(tempCwd);
	});

	// ── B7: shared model resolution ──────────────────────────────────────

	it("resolves an exact provider/id with exact basis", async () => {
		// Failure mode: eval silently falls back to another model instead of
		// the requested exact id.
		const parent = makeToolSession({}, { modelRegistry: stubRegistry([MODEL_A, MODEL_B], new Map()) });
		const resolved = await resolveEvalChildModel(parent, [SELECTOR_A]);
		expect(resolved.model.provider).toBe(MODEL_A.provider);
		expect(resolved.model.id).toBe(MODEL_A.id);
		expect(resolved.basis).toBe("exact");
	});

	it("resolves the @task role through settings with role basis", async () => {
		// Failure mode: @task roles are ignored and the eval runs on an
		// arbitrary model instead of the configured task lane.
		const parent = makeToolSession({}, { modelRegistry: stubRegistry([MODEL_A, MODEL_B], new Map()) });
		parent.settings.override("modelRoles", { task: SELECTOR_B });
		const resolved = await resolveEvalChildModel(parent, undefined);
		expect(resolved.model.id).toBe(MODEL_B.id);
		expect(resolved.basis).toBe("role");
	});

	it("resolves a bare model pattern with pattern basis", async () => {
		// Failure mode: non-exact patterns either fail or misreport how they
		// resolved, hiding which policy selected the model.
		const parent = makeToolSession({}, { modelRegistry: stubRegistry([MODEL_A, MODEL_B], new Map()) });
		const resolved = await resolveEvalChildModel(parent, [MODEL_A.id]);
		expect(resolved.model.id).toBe(MODEL_A.id);
		expect(resolved.basis).toBe("pattern");
	});

	it("falls back to the authed parent model with fallback basis", async () => {
		// Failure mode: a primary pattern without credentials routes the eval
		// to an unusable provider instead of the parent's working model.
		const parent = makeToolSession(
			{},
			{
				modelRegistry: stubRegistry([MODEL_A, MODEL_B], new Map([[SELECTOR_B, "stored-key"]])),
				getActiveModelString: () => SELECTOR_B,
				getSessionId: () => "parent-session",
			},
		);
		const resolved = await resolveEvalChildModel(parent, [SELECTOR_A]);
		expect(resolved.model.id).toBe(MODEL_B.id);
		expect(resolved.basis).toBe("auth-fallback");
	});

	it("throws on unresolvable patterns instead of running an arbitrary model", async () => {
		// Failure mode: a typo'd model silently evaluates on the wrong model
		// and records a misleading pass.
		const parent = makeToolSession({}, { modelRegistry: stubRegistry([MODEL_A, MODEL_B], new Map()) });
		await expect(resolveEvalChildModel(parent, ["nope/nothing-here-zzz"])).rejects.toThrow(
			/No evaluation model resolves/,
		);
	});

	// ── B1: production executor + credentials ────────────────────────────

	function parentWithStoredCreds(keyMap: Map<string, string>, extra: Partial<ToolSession> = {}): ToolSession {
		const settings = Settings.isolated({ "autolearn.enabled": true });
		settings.override("modelRoles", { task: SELECTOR_A });
		return makeToolSession(
			{},
			{
				cwd: tempCwd,
				settings,
				taskDepth: 0,
				getSessionId: () => "parent-session",
				getActiveModel: () => MODEL_A,
				getActiveModelString: () => SELECTOR_A,
				modelRegistry: stubRegistry([MODEL_A, MODEL_B], keyMap),
				...extra,
			},
		);
	}

	it("production skill eval grades with stored registry creds and records run/model/evidence", async () => {
		// Failure mode: without an injected runner the eval cannot run, or it
		// asks the network instead of the parent's stored credentials.
		const parent = parentWithStoredCreds(new Map([[SELECTOR_A, "stored-key"]]));
		const child: StubChild = { prompts: [], disposed: 0, messages: assistantMessages("observed the thing") };
		const created: unknown[] = [];
		vi.spyOn(sdkModule, "createAgentSession").mockImplementation(async options => {
			created.push(options);
			return { session: stubChildSession(child) } as unknown as CreateAgentSessionResult;
		});
		const grading = vi.spyOn(ai, "completeSimple").mockResolvedValue({
			stopReason: "stop",
			content: [{ type: "text", text: "PASS\nMatches the expected outcome exactly." }],
		} as never);

		const tool = new ManageSkillTool(parent);
		await tool.execute("1", { action: "create", name: "prodeval", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "prodeval", description: SKILL_DESC, body: "candidate" });
		const result = await tool.execute("2", {
			action: "evaluate",
			name: "prodeval",
			revisionId: draft.id,
			task: "do the thing",
			expectedOutcome: "the thing is done",
		});

		expect(JSON.stringify(result.content)).toMatch(/passed/);
		expect(child.prompts).toEqual(["do the thing"]);
		expect(grading).toHaveBeenCalledTimes(1);
		expect(grading.mock.calls[0]?.[2]).toMatchObject({ apiKey: "stored-key" });
		const { revisions } = await listSkillRevisions("prodeval");
		const record = revisions.find(r => r.id === draft.id)?.evaluations[0];
		expect(record?.passed).toBe(true);
		expect(record?.runId).toBe("child-run-1");
		expect(record?.sessionId).toBe("child-run-1");
		expect(record?.model).toBe(SELECTOR_A);
		expect(record?.modelBasis).toBe("role");
		expect(record?.summary).toMatch(/Matches the expected outcome exactly/);
		expect(record?.summary).toMatch(/observed the thing/);
		expect(child.disposed).toBe(1);
	});

	it("direct parent resolver wins over stored registry creds", async () => {
		// Failure mode: the executor ignores the parent's direct resolver and
		// grades with a different credential than the session would use.
		const parent = parentWithStoredCreds(new Map([[SELECTOR_A, "stored-key"]]), {
			getApiKey: async () => "direct-key",
		});
		const child: StubChild = { prompts: [], disposed: 0, messages: assistantMessages("output here") };
		vi.spyOn(sdkModule, "createAgentSession").mockImplementation(async () => {
			return { session: stubChildSession(child) } as unknown as CreateAgentSessionResult;
		});
		const grading = vi.spyOn(ai, "completeSimple").mockResolvedValue({
			stopReason: "stop",
			content: [{ type: "text", text: "PASS\nDirect key graded this." }],
		} as never);

		const credential = await resolveEvalCredential(parent, MODEL_A, "parent-session", undefined);
		expect(credential).toEqual({ key: "direct-key", unavailable: false });

		const tool = new ManageSkillTool(parent);
		await tool.execute("1", { action: "create", name: "precedence", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "precedence", description: SKILL_DESC, body: "candidate" });
		await tool.execute("2", {
			action: "evaluate",
			name: "precedence",
			revisionId: draft.id,
			task: "t",
			expectedOutcome: "o",
		});
		expect(grading.mock.calls[0]?.[2]).toMatchObject({ apiKey: "direct-key" });
	});

	it("missing creds everywhere yields grounded unverifiable fail without grading", async () => {
		// Failure mode: no credential produces a provider call that fails
		// opaquely, or worse, an unearned pass.
		const parent = parentWithStoredCreds(new Map());
		const child: StubChild = { prompts: [], disposed: 0, messages: assistantMessages("output here") };
		vi.spyOn(sdkModule, "createAgentSession").mockImplementation(async () => {
			return { session: stubChildSession(child) } as unknown as CreateAgentSessionResult;
		});
		const grading = vi.spyOn(ai, "completeSimple");

		const tool = new ManageSkillTool(parent);
		await tool.execute("1", { action: "create", name: "nokeys", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "nokeys", description: SKILL_DESC, body: "candidate" });
		const result = await tool.execute("2", {
			action: "evaluate",
			name: "nokeys",
			revisionId: draft.id,
			task: "t",
			expectedOutcome: "o",
		});

		expect(JSON.stringify(result.content)).toMatch(/FAILED/);
		expect(JSON.stringify(result.content)).toMatch(/no API key/i);
		expect(grading).not.toHaveBeenCalled();
		const { revisions } = await listSkillRevisions("nokeys");
		expect(revisions.find(r => r.id === draft.id)?.evaluations[0]?.passed).toBe(false);
		await expect(
			(await import("@harvest/pi-coding-agent/autolearn/managed-skills")).promoteSkillRevision("nokeys", draft.id),
		).rejects.toThrow(/failing evaluations/);
	});

	it("eval child borrows parent registry without persisting settings", async () => {
		// Failure mode: the eval run writes into user/project configuration,
		// or the child authenticates through a divergent registry.
		const agentDir = getAgentDir();
		await fs.mkdir(agentDir, { recursive: true });
		await Bun.write(path.join(agentDir, "config.yml"), "autolearn:\n  enabled: true\n");
		const before = await Bun.file(path.join(agentDir, "config.yml")).text();

		const parent = parentWithStoredCreds(new Map([[SELECTOR_A, "stored-key"]]));
		const child: StubChild = { prompts: [], disposed: 0, messages: assistantMessages("output here") };
		let captured: { modelRegistry?: unknown; getApiKey?: unknown; settings?: Settings; restrictToolNames?: unknown } =
			{};
		vi.spyOn(sdkModule, "createAgentSession").mockImplementation(async options => {
			captured = options as typeof captured;
			return { session: stubChildSession(child) } as unknown as CreateAgentSessionResult;
		});
		vi.spyOn(ai, "completeSimple").mockResolvedValue({
			stopReason: "stop",
			content: [{ type: "text", text: "PASS\nFine." }],
		} as never);

		const tool = new ManageSkillTool(parent);
		await tool.execute("1", { action: "create", name: "borrowcheck", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "borrowcheck", description: SKILL_DESC, body: "candidate" });
		await tool.execute("2", {
			action: "evaluate",
			name: "borrowcheck",
			revisionId: draft.id,
			task: "t",
			expectedOutcome: "o",
		});

		expect(captured.modelRegistry).toBe(parent.modelRegistry);
		expect(captured.restrictToolNames).toBe(true);
		expect(captured.settings?.get("autolearn.enabled")).toBe(false);
		expect(parent.settings.get("autolearn.enabled")).toBe(true);
		expect(await Bun.file(path.join(agentDir, "config.yml")).text()).toBe(before);
	});

	// ── B6: cancellation ─────────────────────────────────────────────────

	it("production preset eval resolves the spec model with exact basis", async () => {
		// Failure mode: preset evaluations ignore the spec's model field (or
		// never run without an injected runner) and misrecord provenance.
		const parent = parentWithStoredCreds(new Map([[SELECTOR_B, "stored-key"]]));
		const child: StubChild = { prompts: [], disposed: 0, messages: assistantMessages("preset output") };
		let captured: { systemPrompt?: unknown; toolNames?: unknown; model?: unknown } = {};
		vi.spyOn(sdkModule, "createAgentSession").mockImplementation(async options => {
			captured = options as typeof captured;
			return { session: stubChildSession(child) } as unknown as CreateAgentSessionResult;
		});
		const grading = vi.spyOn(ai, "completeSimple").mockResolvedValue({
			stopReason: "stop",
			content: [{ type: "text", text: "PASS\nPreset output matches." }],
		} as never);

		await writeManagedPreset({
			action: "create",
			name: "prod-preset",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
			model: SELECTOR_B,
		});
		// The draft inherits the spec model through the merge (omitted fields
		// preserve current values) and evaluates on exactly that model.
		const draft = await createPresetDraft({ name: "prod-preset", systemPrompt: "candidate prompt" });
		const tool = new PresetsTool(parent);
		const result = await tool.execute("1", {
			action: "evaluate",
			name: "prod-preset",
			revisionId: draft.id,
			task: "run the preset task",
			expectedOutcome: "preset output",
		});

		expect(JSON.stringify(result.content)).toMatch(/passed/);
		expect(grading).toHaveBeenCalledTimes(1);
		expect(grading.mock.calls[0]?.[2]).toMatchObject({ apiKey: "stored-key" });
		expect(JSON.stringify(captured.systemPrompt)).toContain("candidate prompt");
		const { revisions } = await listPresetRevisions("prod-preset");
		const record = revisions.find(r => r.id === draft.id)?.evaluations[0];
		expect(record?.passed).toBe(true);
		expect(record?.model).toBe(SELECTOR_B);
		expect(record?.modelBasis).toBe("exact");
		expect(getPresetPins("prod-preset").has(draft.id)).toBe(false);
		expect(child.disposed).toBe(1);
	});

	it("cancel during execution aborts, settles the child, and records nothing", async () => {
		// Failure mode: aborting mid-run leaves the child live, records a
		// partial result, or wedges pin release.
		const parent = parentWithStoredCreds(new Map([[SELECTOR_A, "stored-key"]]));
		const child: StubChild = { prompts: [], disposed: 0, messages: assistantMessages("late output") };
		let promptSettled = false;
		vi.spyOn(sdkModule, "createAgentSession").mockImplementation(async () => {
			return {
				session: {
					prompt: async (text: string) => {
						child.prompts.push(text);
						await new Promise<void>(resolve => {
							child.releasePrompt = resolve;
						});
						promptSettled = true;
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
		const grading = vi.spyOn(ai, "completeSimple");

		const tool = new ManageSkillTool(parent);
		await tool.execute("1", { action: "create", name: "cancelexec", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "cancelexec", description: SKILL_DESC, body: "candidate" });

		const controller = new AbortController();
		const run = evaluateSkillRevision("cancelexec", draft.id, {
			task: "t",
			expectedOutcome: "o",
			parent,
			signal: controller.signal,
		});
		// Abort mid-execution: only after the child prompt actually started.
		await waitForInFlight(() => expect(child.prompts.length).toBe(1));
		controller.abort();
		await expect(run).rejects.toThrow(/abort/i);
		expect(child.disposed).toBe(1);
		expect(promptSettled).toBe(true);
		expect(grading).not.toHaveBeenCalled();
		const { revisions } = await listSkillRevisions("cancelexec");
		expect(revisions.find(r => r.id === draft.id)?.evaluations.length).toBe(0);
		expect(getSkillPins("cancelexec").has(draft.id)).toBe(false);
	});

	it("cancel during grading aborts without recording", async () => {
		// Failure mode: an abort landing between execution and grading still
		// records the stale verdict and unblocks promotion.
		const parent = parentWithStoredCreds(new Map([[SELECTOR_A, "stored-key"]]));
		const child: StubChild = { prompts: [], disposed: 0, messages: assistantMessages("output here") };
		vi.spyOn(sdkModule, "createAgentSession").mockImplementation(async () => {
			return { session: stubChildSession(child) } as unknown as CreateAgentSessionResult;
		});
		const grading = vi.spyOn(ai, "completeSimple").mockImplementation(
			(_model, _context, options) =>
				new Promise((_resolve, reject) => {
					options?.signal?.addEventListener("abort", () => reject(new Error("grading aborted")), { once: true });
				}) as never,
		);

		const tool = new ManageSkillTool(parent);
		await tool.execute("1", { action: "create", name: "cancelgrade", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "cancelgrade", description: SKILL_DESC, body: "candidate" });

		const controller = new AbortController();
		const run = evaluateSkillRevision("cancelgrade", draft.id, {
			task: "t",
			expectedOutcome: "o",
			parent,
			signal: controller.signal,
		});
		// Abort mid-grading: only after the grading call actually started.
		await waitForInFlight(() => expect(grading).toHaveBeenCalled());
		controller.abort();
		await expect(run).rejects.toThrow(/abort/i);
		expect(child.disposed).toBe(1);
		const { revisions } = await listSkillRevisions("cancelgrade");
		expect(revisions.find(r => r.id === draft.id)?.evaluations.length).toBe(0);
		// A cancelled run never passes: promotion without new evidence still
		// refuses on unevaluated content.
		await expect(
			(await import("@harvest/pi-coding-agent/autolearn/managed-skills")).promoteSkillRevision(
				"cancelgrade",
				draft.id,
			),
		).rejects.toThrow(/no evaluations/i);
	});

	it("tool-level abort signal reaches the evaluation", async () => {
		// Failure mode: the tool drops the caller's AbortSignal so cancelling
		// the tool call cannot stop a running evaluation.
		let releaseRunner: (() => void) | undefined;
		setSkillEvalRunner(
			() =>
				new Promise(resolve => {
					releaseRunner = () => resolve({ passed: true, summary: "late pass" });
				}),
		);
		try {
			const tool = new ManageSkillTool(async () => {});
			await tool.execute("1", { action: "create", name: "toolsignal", description: SKILL_DESC, body: "live" });
			const draft = await createSkillDraft({ name: "toolsignal", description: SKILL_DESC, body: "candidate" });

			const controller = new AbortController();
			const run = tool.execute(
				"2",
				{ action: "evaluate", name: "toolsignal", revisionId: draft.id, task: "t", expectedOutcome: "o" },
				controller.signal,
			);
			await Bun.sleep(10);
			controller.abort();
			await expect(run).rejects.toThrow(/abort/i);
			releaseRunner?.();
			const { revisions } = await listSkillRevisions("toolsignal");
			expect(revisions.find(r => r.id === draft.id)?.evaluations.length).toBe(0);
		} finally {
			setSkillEvalRunner(undefined);
		}
	});

	// ── B5: leases, promotion pause, retention ───────────────────────────

	it("two evals sharing a runId hold distinct leases until both settle", async () => {
		// Failure mode: colliding pin owners let the first settle release the
		// second's pin, so promotion slips in before the late verdict lands.
		setSkillEvalRunner(undefined);
		const releases = new Map<string, (outcome: { passed: boolean; summary: string }) => void>();
		setSkillEvalRunner(
			input =>
				new Promise(resolve => {
					releases.set(`${input.task}`, resolve);
				}),
		);
		try {
			const tool = new ManageSkillTool(async () => {});
			await tool.execute("1", { action: "create", name: "distinctlease", description: SKILL_DESC, body: "live" });
			const draft = await createSkillDraft({ name: "distinctlease", description: SKILL_DESC, body: "candidate" });

			const first = evaluateSkillRevision("distinctlease", draft.id, {
				task: "first",
				expectedOutcome: "o",
				runId: "shared-run",
			});
			const second = evaluateSkillRevision("distinctlease", draft.id, {
				task: "second",
				expectedOutcome: "o",
				runId: "shared-run",
			});
			await waitForInFlight(() => expect(hasLiveSkillEvalPin("distinctlease", draft.id)).toBe(true));
			const { promoteSkillRevision } = await import("@harvest/pi-coding-agent/autolearn/managed-skills");
			await expect(promoteSkillRevision("distinctlease", draft.id)).rejects.toThrow(/pending evaluation/);

			releases.get("first")!({ passed: true, summary: "first ok" });
			await first;
			// One settle must not release the other's lease: promotion still waits.
			expect(hasLiveSkillEvalPin("distinctlease", draft.id)).toBe(true);
			await expect(promoteSkillRevision("distinctlease", draft.id)).rejects.toThrow(/pending evaluation/);

			releases.get("second")!({ passed: true, summary: "second ok" });
			await second;
			expect(hasLiveSkillEvalPin("distinctlease", draft.id)).toBe(false);
			const { revisions } = await listSkillRevisions("distinctlease");
			expect(revisions.find(r => r.id === draft.id)?.evaluations.length).toBe(2);
			const promoted = await promoteSkillRevision("distinctlease", draft.id);
			expect(promoted.revId).toBe(draft.id);
		} finally {
			setSkillEvalRunner(undefined);
		}
	});

	it("promotion pauses for in-flight evals while listing proceeds", async () => {
		// Failure mode: promotion races a pending verdict, or readers deadlock
		// behind a live evaluation.
		let releaseRunner: ((outcome: { passed: boolean; summary: string }) => void) | undefined;
		setSkillEvalRunner(
			() =>
				new Promise(resolve => {
					releaseRunner = resolve;
				}),
		);
		try {
			const tool = new ManageSkillTool(async () => {});
			await tool.execute("1", { action: "create", name: "pauseskill", description: SKILL_DESC, body: "live" });
			const draft = await createSkillDraft({ name: "pauseskill", description: SKILL_DESC, body: "candidate" });

			const pending = evaluateSkillRevision("pauseskill", draft.id, { task: "t", expectedOutcome: "o" });
			const { promoteSkillRevision } = await import("@harvest/pi-coding-agent/autolearn/managed-skills");
			await waitForInFlight(() => expect(hasLiveSkillEvalPin("pauseskill", draft.id)).toBe(true));
			const listed = await listSkillRevisions("pauseskill");
			expect(listed.revisions.some(r => r.id === draft.id)).toBe(true);
			await expect(promoteSkillRevision("pauseskill", draft.id)).rejects.toThrow(/pending evaluation/);

			releaseRunner!({ passed: true, summary: "late pass" });
			await pending;
			const promoted = await promoteSkillRevision("pauseskill", draft.id);
			expect(promoted.revId).toBe(draft.id);
		} finally {
			setSkillEvalRunner(undefined);
		}
	});

	it("preset promotion pauses while discovery proceeds", async () => {
		// Failure mode (preset mirror): promotion races a pending preset
		// verdict, or discovery blocks behind the live evaluation.
		let releaseRunner: ((outcome: { passed: boolean; summary: string }) => void) | undefined;
		setPresetEvalRunner(
			() =>
				new Promise(resolve => {
					releaseRunner = resolve;
				}),
		);
		try {
			await writeManagedPreset({
				action: "create",
				name: "pause-preset",
				description: PRESET_DESC,
				systemPrompt: PRESET_BODY,
			});
			const draft = await createPresetDraft({
				name: "pause-preset",
				description: PRESET_DESC,
				systemPrompt: "cand",
			});

			const pending = evaluatePresetRevision("pause-preset", draft.id, { task: "t", expectedOutcome: "o" });
			const { promotePresetRevision } = await import("@harvest/pi-coding-agent/task/agents");
			await waitForInFlight(() => expect(hasLivePresetEvalPin("pause-preset", draft.id)).toBe(true));
			const discovered = await discoverManagedPresets();
			expect(discovered.some(agent => agent.name === "pause-preset")).toBe(true);
			await expect(promotePresetRevision("pause-preset", draft.id)).rejects.toThrow(/pending evaluation/);

			releaseRunner!({ passed: true, summary: "late pass" });
			await pending;
			expect(hasLivePresetEvalPin("pause-preset", draft.id)).toBe(false);
			const promoted = await promotePresetRevision("pause-preset", draft.id);
			expect(promoted.revId).toBe(draft.id);
		} finally {
			setPresetEvalRunner(undefined);
		}
	});

	it("retention keeps newest twenty plus active and live-pinned, then prunes on release", async () => {
		// Failure mode: retention drops the active revision, evicts a pinned
		// revision mid-run, or never reaps after the pin releases.
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "retention", description: SKILL_DESC, body: "live" });
		const { pinSkillRevisionForRun, unpinSkillRevisionForRun } =
			await import("@harvest/pi-coding-agent/autolearn/managed-skills");
		const drafts: string[] = [];
		for (let i = 0; i < 22; i++) {
			const draft = await createDraftRevision({
				kind: "skill",
				name: "retention",
				content: `content ${i}`,
				description: SKILL_DESC,
			});
			drafts.push(draft.id);
			await Bun.sleep(2);
		}
		const oldest = drafts[0]!;
		pinSkillRevisionForRun("retention", oldest, "run-keep");
		let surviving = await pruneSkillRevisions("retention");
		const ids = new Set(surviving.map(r => r.id));
		const { active } = await listSkillRevisions("retention");
		expect(ids.has(oldest)).toBe(true);
		expect(active && ids.has(active)).toBe(true);
		expect(surviving.length).toBe(22);

		unpinSkillRevisionForRun("retention", oldest, "run-keep");
		surviving = await pruneSkillRevisions("retention");
		expect(surviving.length).toBe(21);
		expect(surviving.some(r => r.id === oldest)).toBe(false);
	});

	it("draft creation triggers retention", async () => {
		// Failure mode: draft-only flows grow history without bound because
		// only promote paths prune.
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "draftprune", description: SKILL_DESC, body: "live" });
		for (let i = 0; i < 21; i++) {
			await createSkillDraft({ name: "draftprune", description: SKILL_DESC, body: `candidate ${i}` });
			await Bun.sleep(2);
		}
		const surviving = await pruneSkillRevisions("draftprune");
		expect(surviving.length).toBe(21);
		const { active } = await listSkillRevisions("draftprune");
		expect(active && surviving.some(r => r.id === active)).toBe(true);
	});

	it("task pins survive promotion until the run releases them", async () => {
		// Failure mode: promoting a new revision clears another run's pins,
		// exposing its revision to prune mid-run.
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "taskpin", description: SKILL_DESC, body: "live" });
		const before = await listSkillRevisions("taskpin");
		if (!before.active) throw new Error("Expected an active revision to pin");
		const pinned = await pinActiveSkillsForRun("run-t1");
		expect(pinned.map(p => p.revId)).toContain(before.active);

		setSkillEvalRunner(async () => ({ passed: true, summary: "ok" }));
		try {
			const draft = await createSkillDraft({ name: "taskpin", description: SKILL_DESC, body: "candidate" });
			await tool.execute("2", {
				action: "evaluate",
				name: "taskpin",
				revisionId: draft.id,
				task: "t",
				expectedOutcome: "o",
			});
			const { promoteSkillRevision } = await import("@harvest/pi-coding-agent/autolearn/managed-skills");
			await promoteSkillRevision("taskpin", draft.id);
		} finally {
			setSkillEvalRunner(undefined);
		}
		expect(getSkillPins("taskpin").has(before.active!)).toBe(true);
		const surviving = await pruneSkillRevisions("taskpin");
		expect(surviving.some(r => r.id === before.active)).toBe(true);
		unpinActiveSkillsForRun(pinned, "run-t1");
		expect(getSkillPins("taskpin").has(before.active!)).toBe(false);
	});

	// ── B2/B4: recovery ──────────────────────────────────────────────────

	it("skill recovery retains the journal on incoherence and recovers after repair", async () => {
		// Failure mode: a pointer naming a missing revision silently deletes
		// the live file, or the journal clears before coherence is restored.
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "cohere", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "cohere", description: SKILL_DESC, body: "candidate" });
		const root = getManagedSkillsDir();
		const pointer = await readActivePointer("skill", "cohere");
		const revFile = path.join(root, "..", "managed-revisions", "skills", "cohere", `${pointer.active}.json`);
		const revBytes = await Bun.file(revFile).text();
		await Bun.write(
			path.join(root, "cohere.txn.json"),
			JSON.stringify({ kind: "skill", name: "cohere", revId: draft.id, previousActive: pointer.active }),
		);
		await fs.rm(revFile);

		const { recoverInterruptedSkillTransaction } = await import("@harvest/pi-coding-agent/autolearn/managed-skills");
		await expect(recoverInterruptedSkillTransaction("cohere")).rejects.toThrow(/retaining transaction journal/);
		expect(await Bun.file(path.join(root, "cohere.txn.json")).exists()).toBe(true);

		await Bun.write(revFile, revBytes);
		await recoverInterruptedSkillTransaction("cohere");
		expect(await Bun.file(path.join(root, "cohere.txn.json")).exists()).toBe(false);
		const after = await readActivePointer("skill", "cohere");
		expect(after.active).toBe(pointer.active);
	});

	it("preset recovery rolls the file back and resets false-active drafts", async () => {
		// Failure mode: an interrupted preset promote leaves file-ahead-of-
		// pointer published, and the never-published revision stays rollback-
		// eligible (a false active).
		await writeManagedPreset({
			action: "create",
			name: "cohere-preset",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
		});
		const draft = await createPresetDraft({ name: "cohere-preset", description: PRESET_DESC, systemPrompt: "v2" });
		const root = getManagedPresetsDir();
		const file = path.join(root, "cohere-preset.md");
		const liveContent = await Bun.file(file).text();
		// Simulate interruption after materialize but before pointer publish:
		// file ahead of the pointer, revision marked active, journal open.
		await Bun.write(file, "ahead of pointer");
		const revFile = path.join(getAgentDir(), "managed-revisions", "presets", "cohere-preset", `${draft.id}.json`);
		const revBytes = await Bun.file(revFile).json();
		revBytes.state = "active";
		await Bun.write(revFile, JSON.stringify(revBytes, null, "\t"));
		const pointer = await readActivePointer("preset", "cohere-preset");
		await Bun.write(
			path.join(root, "cohere-preset.txn.json"),
			JSON.stringify({ kind: "preset", name: "cohere-preset", revId: draft.id, previousActive: pointer.active }),
		);

		const { recoverInterruptedPresetTransaction, rollbackPresetRevision } =
			await import("@harvest/pi-coding-agent/task/agents");
		await recoverInterruptedPresetTransaction("cohere-preset");
		expect(await Bun.file(file).text()).toBe(liveContent);
		expect((await readActivePointer("preset", "cohere-preset")).active).toBe(pointer.active);
		expect(await Bun.file(path.join(root, "cohere-preset.txn.json")).exists()).toBe(false);
		// Never-active stays invalid for rollback.
		await expect(rollbackPresetRevision("cohere-preset", draft.id)).rejects.toThrow(/never active/);
	});

	// ── B3: merge inside the transaction ─────────────────────────────────

	it("interleaved preset policy/prompt updates both survive", async () => {
		// Failure mode: merging outside the lock lets one update clobber the
		// other's fields (lost policy or lost prompt).
		await writeManagedPreset({
			action: "create",
			name: "interleave-preset",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
			tools: ["read"],
		});
		await Promise.all([
			writeManagedPreset({ action: "update", name: "interleave-preset", systemPrompt: "You are updated twice." }),
			writeManagedPreset({ action: "update", name: "interleave-preset", tools: ["read", "glob"] }),
		]);
		const content = await Bun.file(path.join(getManagedPresetsDir(), "interleave-preset.md")).text();
		expect(content).toContain("You are updated twice.");
		expect(content).toContain("glob");
		expect(content).toContain(PRESET_DESC);
	});

	it("interleaved skill body/description updates both survive", async () => {
		// Failure mode (skill mirror): a body-only update racing a
		// description-only update loses one side.
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "interleaveskill", description: SKILL_DESC, body: "v1" });
		const { writeManagedSkill } = await import("@harvest/pi-coding-agent/autolearn/managed-skills");
		await Promise.all([
			writeManagedSkill({ action: "update", name: "interleaveskill", body: "v2 body" }),
			writeManagedSkill({ action: "update", name: "interleaveskill", description: SKILL_DESC }),
		]);
		const content = await Bun.file(path.join(getManagedSkillsDir(), "interleaveskill", "SKILL.md")).text();
		expect(content).toContain("v2 body");
		expect(content).toContain(SKILL_DESC);
	});
});
