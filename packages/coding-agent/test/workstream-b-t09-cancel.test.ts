/**
 * Workstream B T09: evaluation cancellation (R2).
 *
 * Covers committed-then-cancelled (no unrecorded-commit lie) for both kinds,
 * plus holder-based queued-abort: the append queues behind a held artifact
 * transaction, aborts while queued, appends nothing, and stays retryable.
 * 1-of-2 lease isolation under a shared caller run ID is covered by the
 * eval-lifecycle lease suites.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import { getAgentDir, setAgentDir, TempDir } from "@harvest/pi-utils";
import {
	clearSkillPinsForTests,
	createSkillDraft,
	evaluateSkillRevision,
	listSkillRevisions,
	setSkillEvalRunner,
	withArtifactTransaction,
} from "@harvest/pi-coding-agent/autolearn/managed-skills";
import {
	clearPresetPinsForTests,
	createPresetDraft,
	evaluatePresetRevision,
	listPresetRevisions,
	setPresetEvalRunner,
	writeManagedPreset,
} from "@harvest/pi-coding-agent/task/agents";
import { ManageSkillTool } from "@harvest/pi-coding-agent/tools/manage-skill";

const SKILL_DESC = "When to use the t09 probe skill.";
const PRESET_DESC = "Use this agent when probing t09 cancellation.";
const PRESET_BODY = "You are a t09 probe. Answer with observed facts.";

describe("workstream B T09 cancel between pass and append", () => {
	let tmp: TempDir | undefined;
	let originalAgentDir: string;

	beforeEach(() => {
		originalAgentDir = getAgentDir();
		tmp = TempDir.createSync("@omp-b-t09-");
		setAgentDir(tmp.join("agent"));
		setSkillEvalRunner(undefined);
		setPresetEvalRunner(undefined);
		clearSkillPinsForTests();
		clearPresetPinsForTests();
	});

	afterEach(async () => {
		vi.restoreAllMocks();
		setSkillEvalRunner(undefined);
		setPresetEvalRunner(undefined);
		clearSkillPinsForTests();
		clearPresetPinsForTests();
		setAgentDir(originalAgentDir);
		await tmp?.remove().catch(() => undefined);
		tmp = undefined;
	});

	it("committed-then-cancelled still reports committed", async () => {
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "t09-commit", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "t09-commit", description: SKILL_DESC, body: "candidate" });
		setSkillEvalRunner(async () => ({ passed: true, summary: "committed pass" }));
		const controller = new AbortController();
		const result = await evaluateSkillRevision("t09-commit", draft.id, {
			task: "t",
			expectedOutcome: "o",
			signal: controller.signal,
		});
		expect(result.passed).toBe(true);
		controller.abort();
		const { revisions } = await listSkillRevisions("t09-commit");
		expect(revisions.find(r => r.id === draft.id)?.evaluations.length).toBe(1);
	});

	it("preset committed-then-cancelled still reports committed", async () => {
		await writeManagedPreset({
			action: "create",
			name: "t09-preset-commit",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
		});
		const draft = await createPresetDraft({
			name: "t09-preset-commit",
			description: PRESET_DESC,
			systemPrompt: "cand",
		});
		setPresetEvalRunner(async () => ({ passed: true, summary: "committed pass" }));
		const controller = new AbortController();
		const result = await evaluatePresetRevision("t09-preset-commit", draft.id, {
			task: "t",
			expectedOutcome: "o",
			signal: controller.signal,
		});
		expect(result.passed).toBe(true);
		controller.abort();
		const { revisions } = await listPresetRevisions("t09-preset-commit");
		expect(revisions.find(r => r.id === draft.id)?.evaluations.length).toBe(1);
	});

	it("queued-abort appends nothing and stays retryable", async () => {
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "t09-queued", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "t09-queued", description: SKILL_DESC, body: "candidate" });
		setSkillEvalRunner(async () => ({ passed: true, summary: "late pass" }));
		// Hold the artifact transaction so the append queues behind us.
		let release!: () => void;
		const gate = new Promise<void>(resolve => {
			release = resolve;
		});
		const holder = withArtifactTransaction("skill", "t09-queued", () => gate);
	 const controller = new AbortController();
		const pending = evaluateSkillRevision("t09-queued", draft.id, {
			task: "t",
			expectedOutcome: "o",
			signal: controller.signal,
		});
		// Let the runner finish and the append queue; then abort while queued.
		await Bun.sleep(50);
		controller.abort();
		release();
		await holder;
		await expect(pending).rejects.toThrow(/abort/i);
		const { revisions } = await listSkillRevisions("t09-queued");
		expect(revisions.find(r => r.id === draft.id)?.evaluations.length ?? 0).toBe(0);
		// Still retryable afterwards: a fresh evaluation records normally.
		setSkillEvalRunner(async () => ({ passed: true, summary: "retry pass" }));
		const retry = await evaluateSkillRevision("t09-queued", draft.id, { task: "t", expectedOutcome: "o" });
		expect(retry.passed).toBe(true);
		const after = await listSkillRevisions("t09-queued");
		expect(after.revisions.find(r => r.id === draft.id)?.evaluations.length).toBe(1);
	});
});
