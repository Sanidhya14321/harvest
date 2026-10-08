/**
 * Workstream B T10: interrupted-transaction recovery (R3).
 *
 * Failure mode: interrupted rollback-to-V1 (pre-publish) makes recovery reset
 * historically-published V1 to draft (resetFalseActive matches any non-current
 * active).
 *
 * Method: real stores/transactions, versioned journals (op + published/state
 * facts), manual crash simulation (file-ahead + open journal, no pointer
 * publish) with deterministic sequential steps (no timing races). Covers
 * V1/V2/interrupt/recover/rollback-to-V1, never-active ineligibility,
 * post-publish preservation, legacy + missing-history retention, for BOTH kinds.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import { getAgentDir, setAgentDir, TempDir } from "@harvest/pi-utils";
import {
	clearSkillPinsForTests,
	createSkillDraft,
	listSkillRevisions,
	promoteSkillRevision,
	readActiveSkillRevision,
	recoverInterruptedSkillTransaction,
	rollbackSkillRevision,
	setSkillEvalRunner,
	getManagedSkillsDir,
} from "@harvest/pi-coding-agent/autolearn/managed-skills";
import {
	clearPresetPinsForTests,
	createPresetDraft,
	listPresetRevisions,
	promotePresetRevision,
	recoverInterruptedPresetTransaction,
	rollbackPresetRevision,
	setPresetEvalRunner,
	writeManagedPreset,
	getManagedPresetsDir,
} from "@harvest/pi-coding-agent/task/agents";
import { readActivePointer, readRevision } from "@harvest/pi-coding-agent/autolearn/revisions";
import { ManageSkillTool } from "@harvest/pi-coding-agent/tools/manage-skill";

const SKILL_DESC = "When to use the t10 probe skill.";
const PRESET_DESC = "Use this agent when probing t10 recovery.";
const PRESET_BODY = "You are a t10 probe. Answer with observed facts.";

describe("workstream B T10 interrupted recovery", () => {
	let tmp: TempDir | undefined;
	let originalAgentDir: string;

	beforeEach(() => {
		originalAgentDir = getAgentDir();
		tmp = TempDir.createSync("@omp-b-t10-");
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

	it("skill V1/V2 interrupt recover rollback-to-V1 succeeds and preserves history", async () => {
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "t10-skill", description: SKILL_DESC, body: "v1-live" });
		const before = await listSkillRevisions("t10-skill");
		const v1 = before.active!;
		expect(v1).toBeDefined();

		setSkillEvalRunner(async () => ({ passed: true, summary: "ok" }));
		try {
			const d2 = await createSkillDraft({ name: "t10-skill", description: SKILL_DESC, body: "v2-candidate" });
			await tool.execute("2", {
				action: "evaluate",
				name: "t10-skill",
				revisionId: d2.id,
				task: "t",
				expectedOutcome: "o",
			});
			await promoteSkillRevision("t10-skill", d2.id);
		} finally {
			setSkillEvalRunner(undefined);
		}
		const afterV2 = await listSkillRevisions("t10-skill");
		const v2 = afterV2.active!;
		expect(v2).not.toBe(v1);
		// Both V1 and V2 are historically published (state active).
		const revV1 = await readRevision("skill", "t10-skill", v1);
		expect(revV1?.state).toBe("active");

		// Simulate interrupted rollback-to-V1 pre-publish with a versioned
		// journal (op=rollback, targetWasActiveBefore=true): file ahead (V1
		// content), pointer still V2, journal open.
		const root = getManagedSkillsDir();
		const filePath = path.join(root, "t10-skill", "SKILL.md");
		const v1Rev = await readRevision("skill", "t10-skill", v1);
		const v2Rev = await readRevision("skill", "t10-skill", v2);
		expect(v1Rev).toBeDefined();
		expect(v2Rev).toBeDefined();
		const v1Content = v1Rev?.content ?? "";
		const v2Content = v2Rev?.content ?? "";
		expect(v1Content).not.toBe(v2Content);
		await Bun.write(filePath, v1Content);
		await Bun.write(
			path.join(root, "t10-skill.txn.json"),
			JSON.stringify({
				version: 2,
				kind: "skill",
				name: "t10-skill",
				op: "rollback",
				revId: v1,
				previousActive: v2,
				targetStateBefore: "active",
				targetWasActiveBefore: true,
				startedAt: new Date().toISOString(),
			}),
		);

		await recoverInterruptedSkillTransaction("t10-skill");
		// Recovery rolls the file back to the still-published V2 and preserves
		// V1 as legitimately active (never resets historically-published to draft).
		expect(await Bun.file(filePath).text()).toBe(v2Content);
		expect((await readActivePointer("skill", "t10-skill")).active).toBe(v2);
		expect((await readRevision("skill", "t10-skill", v1))?.state).toBe("active");
		expect(await Bun.file(path.join(root, "t10-skill.txn.json")).exists()).toBe(false);

		// Retry the rollback for real: it must succeed.
		const rolled = await rollbackSkillRevision("t10-skill", v1);
		expect(rolled.path).toBe(filePath);
		expect((await readActivePointer("skill", "t10-skill")).active).toBe(v1);
		expect(await Bun.file(filePath).text()).toBe(v1Content);

		// Never-active stays ineligible: a fresh draft cannot be a rollback target.
		const d3 = await createSkillDraft({ name: "t10-skill", description: SKILL_DESC, body: "v3-never-active" });
		await expect(rollbackSkillRevision("t10-skill", d3.id)).rejects.toThrow(/never active/);
	});

	it("skill post-publish interrupt preserves the commit", async () => {
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "t10-post", description: SKILL_DESC, body: "v1" });
		setSkillEvalRunner(async () => ({ passed: true, summary: "ok" }));
		let v2 = "";
		try {
			const d2 = await createSkillDraft({ name: "t10-post", description: SKILL_DESC, body: "v2" });
			await tool.execute("2", {
				action: "evaluate",
				name: "t10-post",
				revisionId: d2.id,
				task: "t",
				expectedOutcome: "o",
			});
			await promoteSkillRevision("t10-post", d2.id);
			v2 = d2.id;
		} finally {
			setSkillEvalRunner(undefined);
		}
		const agentDir = getAgentDir();
		void agentDir;
		const root = getManagedSkillsDir();
		const filePath = path.join(root, "t10-post", "SKILL.md");
		const liveContent = await Bun.file(filePath).text();
		const ptr = await readActivePointer("skill", "t10-post");
		expect(ptr.active).toBe(v2);
		// Simulate crash AFTER pointer publish but BEFORE journal clear: file
		// already matches the published pointer, journal still open.
		await Bun.write(
			path.join(root, "t10-post.txn.json"),
			JSON.stringify({
				version: 2,
				kind: "skill",
				name: "t10-post",
				op: "promote",
				revId: v2,
				previousActive: ptr.active,
				targetStateBefore: "draft",
				targetWasActiveBefore: false,
				startedAt: new Date().toISOString(),
			}),
		);
		await recoverInterruptedSkillTransaction("t10-post");
		expect((await readActivePointer("skill", "t10-post")).active).toBe(v2);
		expect(await Bun.file(filePath).text()).toBe(liveContent);
		expect((await readRevision("skill", "t10-post", v2))?.state).toBe("active");
		expect(await Bun.file(path.join(root, "t10-post.txn.json")).exists()).toBe(false);
	});

	it("skill legacy and missing-history journals are retained, never erased", async () => {
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "t10-legacy", description: SKILL_DESC, body: "live" });
		const root = getManagedSkillsDir();
		const txnPath = path.join(root, "t10-legacy.txn.json");
		const filePath = path.join(root, "t10-legacy", "SKILL.md");
		const liveContent = await Bun.file(filePath).text();

		// Legacy journal with insufficient evidence (missing revId): preserve + fail, never guess.
		await Bun.write(txnPath, JSON.stringify({ kind: "skill", name: "t10-legacy", previousActive: null }));
		await expect(recoverInterruptedSkillTransaction("t10-legacy")).rejects.toThrow(/insufficient evidence/);
		expect(await Bun.file(txnPath).exists()).toBe(true);
		expect(await Bun.file(filePath).text()).toBe(liveContent);
		await fs.rm(txnPath);

		// Missing-history: pointer names a revision whose file is gone.
		// Recovery must retain the journal and preserve the live file (never
		// silently delete it), surfacing for retry.
		const ptr = await readActivePointer("skill", "t10-legacy");
		const revFile = path.join(getAgentDir(), "managed-revisions", "skills", "t10-legacy", `${ptr.active}.json`);
		const revBytes = await Bun.file(revFile).text();
		const draft = await createSkillDraft({ name: "t10-legacy", description: SKILL_DESC, body: "cand" });
		await Bun.write(
			txnPath,
			JSON.stringify({
				version: 2,
				kind: "skill",
				name: "t10-legacy",
				op: "promote",
				revId: draft.id,
				previousActive: ptr.active,
				targetStateBefore: "draft",
				targetWasActiveBefore: false,
				startedAt: new Date().toISOString(),
			}),
		);
		await fs.rm(revFile);
		await expect(recoverInterruptedSkillTransaction("t10-legacy")).rejects.toThrow(/retaining transaction journal/);
		expect(await Bun.file(txnPath).exists()).toBe(true);
		expect(await Bun.file(filePath).text()).toBe(liveContent);
		await Bun.write(revFile, revBytes);
		await recoverInterruptedSkillTransaction("t10-legacy");
		expect(await Bun.file(txnPath).exists()).toBe(false);

		// Mutation callers surface unresolved recovery instead of swallowing it.
		await fs.rm(revFile);
		await Bun.write(
			txnPath,
			JSON.stringify({
				version: 2,
				kind: "skill",
				name: "t10-legacy",
				op: "promote",
				revId: draft.id,
				previousActive: ptr.active,
				targetStateBefore: "draft",
				targetWasActiveBefore: false,
				startedAt: new Date().toISOString(),
			}),
		);
		await expect(createSkillDraft({ name: "t10-legacy", description: SKILL_DESC, body: "another" })).rejects.toThrow(
			/retaining transaction journal/,
		);
		expect(await Bun.file(txnPath).exists()).toBe(true);
		await Bun.write(revFile, revBytes);
		await recoverInterruptedSkillTransaction("t10-legacy");
		expect(await Bun.file(txnPath).exists()).toBe(false);
	});

	it("preset V1/V2 interrupt recover rollback-to-V1 succeeds and preserves history", async () => {
		await writeManagedPreset({
			action: "create",
			name: "t10-preset",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
		});
		const before = await listPresetRevisions("t10-preset");
		const v1 = before.active!;
		setPresetEvalRunner(async () => ({ passed: true, summary: "ok" }));
		let v2 = "";
		try {
			const d2 = await createPresetDraft({
				name: "t10-preset",
				description: PRESET_DESC,
				systemPrompt: "v2 prompt",
			});
			const { evaluatePresetRevision: evalP } = await import("@harvest/pi-coding-agent/task/agents");
			await evalP("t10-preset", d2.id, { task: "t", expectedOutcome: "o" });
			await promotePresetRevision("t10-preset", d2.id);
			v2 = d2.id;
		} finally {
			setPresetEvalRunner(undefined);
		}
		expect(v2).not.toBe(v1);
		expect((await readRevision("preset", "t10-preset", v1))?.state).toBe("active");

		const root = getManagedPresetsDir();
		const filePath = path.join(root, "t10-preset.md");
		const v1Rev = await readRevision("preset", "t10-preset", v1);
		const v2Rev = await readRevision("preset", "t10-preset", v2);
		expect(v1Rev).toBeDefined();
		expect(v2Rev).toBeDefined();
		const v1Content = v1Rev?.content ?? "";
		const v2Content = v2Rev?.content ?? "";
		await Bun.write(filePath, v1Content);
		await Bun.write(
			path.join(root, "t10-preset.txn.json"),
			JSON.stringify({
				version: 2,
				kind: "preset",
				name: "t10-preset",
				op: "rollback",
				revId: v1,
				previousActive: v2,
				targetStateBefore: "active",
				targetWasActiveBefore: true,
				startedAt: new Date().toISOString(),
			}),
		);
		await recoverInterruptedPresetTransaction("t10-preset");
		expect(await Bun.file(filePath).text()).toBe(v2Content);
		expect((await readActivePointer("preset", "t10-preset")).active).toBe(v2);
		expect((await readRevision("preset", "t10-preset", v1))?.state).toBe("active");

		const rolled = await rollbackPresetRevision("t10-preset", v1);
		expect(rolled.path).toBe(filePath);
		expect((await readActivePointer("preset", "t10-preset")).active).toBe(v1);

		const d3 = await createPresetDraft({ name: "t10-preset", description: PRESET_DESC, systemPrompt: "v3 never" });
		await expect(rollbackPresetRevision("t10-preset", d3.id)).rejects.toThrow(/never active/);
	});

	it("preset post-publish preserves commit; legacy and missing-history retained", async () => {
		await writeManagedPreset({
			action: "create",
			name: "t10-post-preset",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
		});
		setPresetEvalRunner(async () => ({ passed: true, summary: "ok" }));
		let v2 = "";
		try {
			const d2 = await createPresetDraft({ name: "t10-post-preset", description: PRESET_DESC, systemPrompt: "v2" });
			const { evaluatePresetRevision: evalP } = await import("@harvest/pi-coding-agent/task/agents");
			await evalP("t10-post-preset", d2.id, { task: "t", expectedOutcome: "o" });
			await promotePresetRevision("t10-post-preset", d2.id);
			v2 = d2.id;
		} finally {
			setPresetEvalRunner(undefined);
		}
		const root = getManagedPresetsDir();
		const filePath = path.join(root, "t10-post-preset.md");
		const liveContent = await Bun.file(filePath).text();
		await Bun.write(
			path.join(root, "t10-post-preset.txn.json"),
			JSON.stringify({
				version: 2,
				kind: "preset",
				name: "t10-post-preset",
				op: "promote",
				revId: v2,
				previousActive: v2,
				targetStateBefore: "draft",
				targetWasActiveBefore: false,
				startedAt: new Date().toISOString(),
			}),
		);
		await recoverInterruptedPresetTransaction("t10-post-preset");
		expect((await readActivePointer("preset", "t10-post-preset")).active).toBe(v2);
		expect(await Bun.file(filePath).text()).toBe(liveContent);

		const txnPath = path.join(root, "t10-legacy-preset.txn.json");
		void txnPath;
		// Legacy insufficient: preserve + fail.
		await writeManagedPreset({
			action: "create",
			name: "t10-legacy-preset",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
		});
		const legacyRoot = getManagedPresetsDir();
		const legacyTxn = path.join(legacyRoot, "t10-legacy-preset.txn.json");
		const legacyFile = path.join(legacyRoot, "t10-legacy-preset.md");
		const legacyLive = await Bun.file(legacyFile).text();
		await Bun.write(legacyTxn, JSON.stringify({ kind: "preset", name: "t10-legacy-preset", previousActive: null }));
		await expect(recoverInterruptedPresetTransaction("t10-legacy-preset")).rejects.toThrow(/insufficient evidence/);
		expect(await Bun.file(legacyTxn).exists()).toBe(true);
		expect(await Bun.file(legacyFile).text()).toBe(legacyLive);
		await fs.rm(legacyTxn);

		// Missing-history retained.
		const ptr = await readActivePointer("preset", "t10-legacy-preset");
		const revFile = path.join(
			getAgentDir(),
			"managed-revisions",
			"presets",
			"t10-legacy-preset",
			`${ptr.active}.json`,
		);
		const revBytes = await Bun.file(revFile).text();
		const draft = await createPresetDraft({
			name: "t10-legacy-preset",
			description: PRESET_DESC,
			systemPrompt: "cand2",
		});
		await Bun.write(
			legacyTxn,
			JSON.stringify({
				version: 2,
				kind: "preset",
				name: "t10-legacy-preset",
				op: "promote",
				revId: draft.id,
				previousActive: ptr.active,
				targetStateBefore: "draft",
				targetWasActiveBefore: false,
				startedAt: new Date().toISOString(),
			}),
		);
		await fs.rm(revFile);
		await expect(recoverInterruptedPresetTransaction("t10-legacy-preset")).rejects.toThrow(
			/retaining transaction journal/,
		);
		expect(await Bun.file(legacyTxn).exists()).toBe(true);
		await Bun.write(revFile, revBytes);
		await recoverInterruptedPresetTransaction("t10-legacy-preset");
		expect(await Bun.file(legacyTxn).exists()).toBe(false);
		expect(await readActiveSkillRevision("t10-legacy-preset").catch(() => undefined)).toBeUndefined();
	});
});
