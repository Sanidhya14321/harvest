import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import {
	clearImprovementBudgetsForTests,
	clearSkillPinsForTests,
	createSkillDraft,
	evaluateSkillRevision,
	getManagedSkillsDir,
	getSkillPins,
	isSkillEvalRunActive,
	listSkillRevisions,
	noteParentTurnCompleted,
	pinSkillRevision,
	promoteSkillRevision,
	pruneSkillRevisions,
	setSkillEvalRunner,
	tryClaimImprovementCandidate,
	tryClaimImprovementEvaluation,
	unpinSkillRevision,
} from "@harvest/pi-coding-agent/autolearn/managed-skills";
import { createDraftRevision } from "@harvest/pi-coding-agent/autolearn/revisions";
import { type SettingPath, Settings } from "@harvest/pi-coding-agent/config/settings";
import type { MnemopiSessionState } from "@harvest/pi-coding-agent/mnemopi/state";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import { LearnTool } from "@harvest/pi-coding-agent/tools/learn";
import { ManageSkillTool } from "@harvest/pi-coding-agent/tools/manage-skill";
import { removeWithRetries } from "@harvest/pi-utils";
import { getAgentDir, setAgentDir } from "@harvest/pi-utils/dirs";

function makeSession(
	settingsOverrides: Partial<Record<SettingPath, unknown>> = {},
	extra: Partial<ToolSession> = {},
): ToolSession {
	return {
		cwd: "/tmp/test",
		hasUI: false,
		skipPythonPreflight: true,
		getSessionFile: () => null,
		getSessionSpawns: () => "*",
		settings: Settings.isolated(settingsOverrides),
		...extra,
	};
}

describe("managed skill revisions", () => {
	let tempHome: string;
	let originalAgentDir: string;
	let refreshed = 0;

	beforeEach(async () => {
		originalAgentDir = getAgentDir();
		tempHome = await fs.mkdtemp(path.join(os.tmpdir(), "omp-skill-rev-"));
		spyOn(os, "homedir").mockReturnValue(tempHome);
		setAgentDir(path.join(tempHome, ".omp", "agent"));
		refreshed = 0;
		setSkillEvalRunner(undefined);
		clearSkillPinsForTests();
	});

	afterEach(async () => {
		spyOn(os, "homedir").mockRestore();
		setAgentDir(originalAgentDir);
		setSkillEvalRunner(undefined);
		clearSkillPinsForTests();
		await removeWithRetries(tempHome);
	});

	const tool = () =>
		new ManageSkillTool(async () => {
			refreshed++;
		});
	const file = (name: string) => path.join(getManagedSkillsDir(), name, "SKILL.md");

	it("records history on create/update and rejects expected-revision conflicts", async () => {
		await tool().execute("1", { action: "create", name: "reved", description: "When to rev.", body: "v1" });
		await tool().execute("2", { action: "update", name: "reved", description: "When to rev.", body: "v2" });
		const { active, revisions } = await listSkillRevisions("reved");
		expect(revisions).toHaveLength(2);
		expect(active).toBe(revisions[revisions.length - 1]!.id);
		expect(refreshed).toBe(2);

		await expect(
			tool().execute("3", {
				action: "update",
				name: "reved",
				description: "When to rev.",
				body: "v3",
				expectedActive: "rev-stale",
			}),
		).rejects.toThrow(/Revision conflict/);
		// The conflict rejected before any file write.
		expect(await Bun.file(file("reved")).text()).toContain("v2");
	});

	it("draft stages a revision without touching the live file; promote activates it", async () => {
		setSkillEvalRunner(async () => ({ passed: true, summary: "observed expected outcome" }));
		await tool().execute("1", { action: "create", name: "gated", description: "When gated.", body: "live" });
		const drafted = await tool().execute("2", {
			action: "draft",
			name: "gated",
			description: "When gated.",
			body: "candidate",
		});
		const draftId = (drafted.details as { revisionId: string }).revisionId;
		expect(await Bun.file(file("gated")).text()).toContain("live");

		const evaluated = await tool().execute("3", {
			action: "evaluate",
			name: "gated",
			revisionId: draftId,
			task: "exercise the skill",
			expectedOutcome: "expected observable outcome",
		});
		expect((evaluated.details as { passed: boolean }).passed).toBe(true);

		const promoted = await tool().execute("4", {
			action: "promote",
			name: "gated",
			revisionId: draftId,
		});
		expect((promoted.details as { disclosedUnevaluated: boolean }).disclosedUnevaluated).toBe(false);
		expect(await Bun.file(file("gated")).text()).toContain("candidate");
		expect(refreshed).toBe(2);
	});

	it("failed evaluations block promotion and never auto-promote", async () => {
		setSkillEvalRunner(async () => ({ passed: false, summary: "outcome diverged" }));
		await tool().execute("1", { action: "create", name: "failcase", description: "d.", body: "live" });
		const draft = await createSkillDraft({ name: "failcase", description: "d.", body: "candidate" });
		const evaluated = await tool().execute("2", {
			action: "evaluate",
			name: "failcase",
			revisionId: draft.id,
			task: "exercise the skill",
			expectedOutcome: "expected observable outcome",
		});
		expect((evaluated.details as { passed: boolean }).passed).toBe(false);
		await expect(tool().execute("3", { action: "promote", name: "failcase", revisionId: draft.id })).rejects.toThrow(
			/failing evaluations/,
		);
		// The live file still carries the last good content.
		expect(await Bun.file(file("failcase")).text()).toContain("live");
	});

	it("unevaluated promotion requires discloseUnevaluated and surfaces the status", async () => {
		await tool().execute("1", { action: "create", name: "raw", description: "d.", body: "live" });
		const draft = await createSkillDraft({ name: "raw", description: "d.", body: "candidate" });
		await expect(promoteSkillRevision("raw", draft.id)).rejects.toThrow(/discloseUnevaluated/);
		const result = await promoteSkillRevision("raw", draft.id, { discloseUnevaluated: true });
		expect(result.disclosedUnevaluated).toBe(true);
	});

	it("evaluation requires an explicit task and expected outcome", async () => {
		await expect(evaluateSkillRevision("nope", "rev-x", { task: "", expectedOutcome: "o" })).rejects.toThrow(
			/explicit task/,
		);
		await expect(evaluateSkillRevision("nope", "rev-x", { task: "t", expectedOutcome: "" })).rejects.toThrow(
			/explicit expected outcome/,
		);
	});

	it("pinning survives prune while unpinned old revisions are dropped", async () => {
		const first = await createSkillDraft({ name: "pinned", description: "d.", body: "v0" });
		pinSkillRevision("pinned", first.id);
		for (let i = 1; i <= 21; i++) {
			await createDraftRevision({
				kind: "skill",
				name: "pinned",
				content: `content ${i}`,
				description: "d.",
			});
		}
		const surviving = await pruneSkillRevisions("pinned");
		expect(surviving.some(rev => rev.id === first.id)).toBe(true);
		expect(surviving.length).toBeLessThanOrEqual(21);

		unpinSkillRevision("pinned", first.id);
		expect(getSkillPins("pinned").size).toBe(0);
		const survivingAfter = await pruneSkillRevisions("pinned");
		expect(survivingAfter.some(rev => rev.id === first.id)).toBe(false);
	});

	it("learn(skill) writes call refreshSkills", async () => {
		const remembered: string[] = [];
		const session = makeSession(
			{ "autolearn.enabled": true, "memory.backend": "mnemopi" },
			{
				getMnemopiSessionState: () =>
					({
						sessionId: "sess-1",
						session: { sessionManager: { getCwd: () => "/tmp/work" } },
						rememberScoped: (memory: string) => {
							remembered.push(memory);
							return "mem-id";
						},
					}) as unknown as MnemopiSessionState,
				refreshSkills: async () => {
					refreshed++;
				},
			},
		);
		const result = await new LearnTool(session).execute("1", {
			memory: "Prefer Bun.file over readFileSync.",
			skill: { action: "create", name: "refresh-probe", description: "When to probe.", body: "# probe" },
		});
		expect(remembered).toHaveLength(1);
		expect(refreshed).toBe(1);
		expect(result.details).toMatchObject({ skill: "refresh-probe", status: "unevaluated-active" });
	});

	it("learn(skill) honors expectedActive conflicts", async () => {
		const remembered: string[] = [];
		const session = makeSession(
			{ "autolearn.enabled": true, "memory.backend": "mnemopi" },
			{
				getMnemopiSessionState: () =>
					({
						sessionId: "sess-1",
						session: { sessionManager: { getCwd: () => "/tmp/work" } },
						rememberScoped: (memory: string) => {
							remembered.push(memory);
							return "mem-id";
						},
					}) as unknown as MnemopiSessionState,
			},
		);
		await new LearnTool(session).execute("1", {
			memory: "lesson one",
			skill: { action: "create", name: "conflict-probe", description: "d.", body: "v1" },
		});
		await expect(
			new LearnTool(session).execute("2", {
				memory: "lesson two",
				skill: { action: "update", name: "conflict-probe", description: "d.", body: "v2", expectedActive: "stale" },
			}),
		).rejects.toThrow(/Revision conflict/);
		expect(remembered).toHaveLength(2);
	});

	it("bounds auto-improvement to one candidate + one evaluation per parent turn", () => {
		const key = "parent-turn";
		clearImprovementBudgetsForTests();
		noteParentTurnCompleted(key);
		expect(tryClaimImprovementCandidate(key)).toBe(true);
		expect(tryClaimImprovementCandidate(key)).toBe(false);
		expect(tryClaimImprovementEvaluation(key)).toBe(true);
		expect(tryClaimImprovementEvaluation(key)).toBe(false);
		noteParentTurnCompleted(key);
		expect(tryClaimImprovementCandidate(key)).toBe(true);
		expect(tryClaimImprovementEvaluation(key)).toBe(true);
	});

	it("eval runs neither claim budget nor trigger auto-learning", async () => {
		expect(isSkillEvalRunActive()).toBe(false);
		setSkillEvalRunner(async () => {
			expect(isSkillEvalRunActive()).toBe(true);
			expect(tryClaimImprovementCandidate("parent-turn")).toBe(false);
			return { passed: true, summary: "ok" };
		});
		await tool().execute("1", { action: "create", name: "guarded", description: "d.", body: "live" });
		const draft = await createSkillDraft({ name: "guarded", description: "d.", body: "candidate" });
		await evaluateSkillRevision("guarded", draft.id, { task: "t", expectedOutcome: "o" });
		expect(isSkillEvalRunActive()).toBe(false);
	});
});
