import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import {
	clearSkillPinsForTests,
	createSkillDraft,
	evaluateSkillRevision,
	getManagedSkillsDir,
	getSkillPins,
	listSkillRevisions,
	pinSkillRevision,
	pinSkillRevisionForRun,
	pruneSkillRevisions,
	setSkillEvalRunner,
	unpinSkillRevision,
	unpinSkillRevisionForRun,
	validateSkillRevisionId,
} from "@harvest/pi-coding-agent/autolearn/managed-skills";
import { createDraftRevision, readActivePointer } from "@harvest/pi-coding-agent/autolearn/revisions";
import { type SettingPath, Settings } from "@harvest/pi-coding-agent/config/settings";
import { AgentRegistry } from "@harvest/pi-coding-agent/registry/agent-registry";
import type { AgentSession } from "@harvest/pi-coding-agent/session/agent-session";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import { LearnTool } from "@harvest/pi-coding-agent/tools/learn";
import { ManageSkillTool } from "@harvest/pi-coding-agent/tools/manage-skill";
import {
	clearPresetPinsForTests,
	createPresetDraft,
	evaluatePresetRevision,
	getManagedPresetsDir,
	listPresetRevisions,
	pinPresetRevisionForRun,
	prunePresetRevisions,
	setPresetEvalRunner,
	unpinPresetRevisionForRun,
	validatePresetRevisionId,
	writeManagedPreset,
} from "@harvest/pi-coding-agent/task/agents";
import { PresetsTool, setPresetArchitectRunner } from "@harvest/pi-coding-agent/tools/presets";
import {
	clearSessionToolDepsForTests,
	SessionsTool,
	type ManagedLiveSessionLike,
} from "@harvest/pi-coding-agent/tools/sessions";
import { removeWithRetries } from "@harvest/pi-utils";
import { getAgentDir, setAgentDir } from "@harvest/pi-utils/dirs";

function makeToolSession(
	settingsOverrides: Partial<Record<SettingPath, unknown>> = {},
	extra: Partial<ToolSession> = {},
): ToolSession {
	return {
		cwd: "/tmp/b-probe",
		hasUI: false,
		skipPythonPreflight: true,
		getSessionFile: () => null,
		getSessionSpawns: () => "*",
		settings: Settings.isolated(settingsOverrides),
		...extra,
	};
}

async function snapshotConfigBytes(agentDir: string): Promise<Map<string, string>> {
	const out = new Map<string, string>();
	async function walk(dir: string): Promise<void> {
		const entries = await fs.readdir(dir, { withFileTypes: true }).catch(() => []);
		for (const entry of entries as import("node:fs").Dirent[]) {
			const full = path.join(dir, entry.name);
			if (entry.isDirectory()) {
				if (
					entry.name === "managed-skills" ||
					entry.name === "managed-presets" ||
					entry.name === "managed-revisions"
				) {
					continue;
				}
				await walk(full);
			} else if (entry.isFile()) {
				const bytes = await Bun.file(full)
					.arrayBuffer()
					.catch(() => null);
				if (bytes) out.set(full, Buffer.from(bytes).toString("hex"));
			}
		}
	}
	await walk(agentDir);
	return out;
}

function expectSnapshotsEqual(a: Map<string, string>, b: Map<string, string>): void {
	expect([...a.keys()].sort()).toEqual([...b.keys()].sort());
	for (const [k, v] of a) expect(b.get(k)).toBe(v);
}

function fakeLive(id: string): ManagedLiveSessionLike {
	return {
		getSessionId: () => id,
		abort: async () => {},
		setSessionName: async () => {},
		prompt: async () => undefined,
	};
}

const SKILL_DESC = "When to use the probe skill.";
const PRESET_DESC = "Use this agent when reviewing managed deltas.";
const PRESET_BODY = "You are a delta reviewer. Check every hunk twice.";

describe("workstream B hardening", () => {
	let tempHome: string;
	let tempCwd: string;
	let originalAgentDir: string;

	beforeEach(async () => {
		originalAgentDir = getAgentDir();
		tempHome = await fs.mkdtemp(path.join(os.tmpdir(), "omp-b-"));
		tempCwd = await fs.mkdtemp(path.join(os.tmpdir(), "omp-b-cwd-"));
		spyOn(os, "homedir").mockReturnValue(tempHome);
		setAgentDir(path.join(tempHome, ".omp", "agent"));
		setSkillEvalRunner(undefined);
		setPresetEvalRunner(undefined);
		setPresetArchitectRunner(undefined);
		clearSkillPinsForTests();
		clearPresetPinsForTests();
		clearSessionToolDepsForTests();
	});

	afterEach(async () => {
		spyOn(os, "homedir").mockRestore();
		setAgentDir(originalAgentDir);
		setSkillEvalRunner(undefined);
		setPresetEvalRunner(undefined);
		setPresetArchitectRunner(undefined);
		clearSkillPinsForTests();
		clearPresetPinsForTests();
		clearSessionToolDepsForTests();
		await removeWithRetries(tempHome);
		await removeWithRetries(tempCwd);
	});

	it("prompt-only preset update preserves policy fields", async () => {
		await writeManagedPreset({
			action: "create",
			name: "policy-keeper",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
			tools: ["read", "grep"],
			spawns: "reviewer-*",
			model: "@task",
		});
		const tool = new PresetsTool(makeToolSession({ "autolearn.enabled": true }, { cwd: tempCwd }));
		const updated = await tool.execute("1", {
			action: "update",
			name: "policy-keeper",
			systemPrompt: "You are a delta reviewer. Check every hunk three times.",
		});
		expect((updated.details as { name: string }).name).toBe("policy-keeper");
		const file = await Bun.file(path.join(getManagedPresetsDir(), "policy-keeper.md")).text();
		expect(file).toContain("Check every hunk three times");
		expect(file).toContain("read");
		expect(file).toContain("reviewer-*");
		expect(file).toContain(PRESET_DESC);
	});

	it("skill body-only update preserves description", async () => {
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "mergeprobe", description: SKILL_DESC, body: "v1 body" });
		await tool.execute("2", { action: "update", name: "mergeprobe", body: "v2 body" });
		const content = await Bun.file(path.join(getManagedSkillsDir(), "mergeprobe", "SKILL.md")).text();
		expect(content).toContain("v2 body");
		expect(content).toContain(SKILL_DESC);
	});

	it("architect success and failure leave settings files byte-identical", async () => {
		const agentDir = getAgentDir();
		await fs.mkdir(agentDir, { recursive: true });
		await Bun.write(path.join(agentDir, "config.yml"), "autolearn:\n  enabled: true\n");
		const before = await snapshotConfigBytes(agentDir);
		setPresetArchitectRunner(async () =>
			JSON.stringify({ identifier: "arch-probe", whenToUse: PRESET_DESC, systemPrompt: PRESET_BODY }),
		);
		const tool = new PresetsTool(makeToolSession({ "autolearn.enabled": true }, { cwd: tempCwd }));
		await tool.execute("1", { action: "create", name: "arch-probe", brief: "review deltas" });
		const afterSuccess = await snapshotConfigBytes(agentDir);
		expectSnapshotsEqual(before, afterSuccess);

		setPresetArchitectRunner(async () => {
			throw new Error("architect boom");
		});
		await expect(tool.execute("2", { action: "create", name: "arch-fail", brief: "review" })).rejects.toThrow(
			/architect boom/,
		);
		const afterFailure = await snapshotConfigBytes(agentDir);
		expectSnapshotsEqual(before, afterFailure);
	});

	it("eval success and failure leave settings files byte-identical", async () => {
		const agentDir = getAgentDir();
		await fs.mkdir(agentDir, { recursive: true });
		await Bun.write(path.join(agentDir, "config.yml"), "autolearn:\n  enabled: true\n");
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "evalbytes", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "evalbytes", description: SKILL_DESC, body: "candidate" });
		const before = await snapshotConfigBytes(agentDir);
		setSkillEvalRunner(async () => ({ passed: true, summary: "ok" }));
		await tool.execute("2", {
			action: "evaluate",
			name: "evalbytes",
			revisionId: draft.id,
			task: "t",
			expectedOutcome: "o",
		});
		expectSnapshotsEqual(before, await snapshotConfigBytes(agentDir));
		setSkillEvalRunner(async () => ({ passed: false, summary: "bad" }));
		const draft2 = await createSkillDraft({ name: "evalbytes", description: SKILL_DESC, body: "candidate2" });
		await tool.execute("3", {
			action: "evaluate",
			name: "evalbytes",
			revisionId: draft2.id,
			task: "t",
			expectedOutcome: "o",
		});
		expectSnapshotsEqual(before, await snapshotConfigBytes(agentDir));
	});

	it("rejects traversal, absolute, and separator revision ids everywhere", async () => {
		for (const bad of ["../escape", "/abs/rev-1", "rev-a/b", "rev-a\\b", "rev-..", "not-a-rev!", ""]) {
			expect(() => validateSkillRevisionId(bad)).toThrow(/Invalid revision id/);
			expect(() => validatePresetRevisionId(bad)).toThrow(/Invalid revision id/);
		}
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "travprobe", description: SKILL_DESC, body: "v1" });
		await expect(
			tool.execute("2", {
				action: "promote",
				name: "travprobe",
				revisionId: "../escape",
			}),
		).rejects.toThrow(/Invalid revision id/);
		expect(await Bun.file(path.join(getManagedSkillsDir(), "travprobe", "SKILL.md")).text()).toContain("v1");
	});

	it("symlink and hardlink fixtures are refused and left unmodified", async () => {
		await writeManagedPreset({
			action: "create",
			name: "link-guard",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
		});
		const file = path.join(getManagedPresetsDir(), "link-guard.md");
		const outside = path.join(tempCwd, "outside.md");
		await Bun.write(outside, "outside content");
		const original = await Bun.file(file).text();
		try {
			await fs.rm(file);
			await fs.symlink(outside, file);
		} catch {
			return;
		}
		await expect(
			writeManagedPreset({ action: "update", name: "link-guard", systemPrompt: "new prompt" }),
		).rejects.toThrow(/symlink/);
		expect(await Bun.file(outside).text()).toBe("outside content");
		expect(await fs.lstat(file).then(s => s.isSymbolicLink())).toBe(true);
		await fs.rm(file);
		await Bun.write(file, original);
	});

	it("materialization failure leaves old active published", async () => {
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "atomprobe", description: SKILL_DESC, body: "v1" });
		const draft = await createSkillDraft({ name: "atomprobe", description: SKILL_DESC, body: "v2" });
		setSkillEvalRunner(async () => ({ passed: true, summary: "ok" }));
		await tool.execute("2", {
			action: "evaluate",
			name: "atomprobe",
			revisionId: draft.id,
			task: "t",
			expectedOutcome: "o",
		});
		const beforePointer = await readActivePointer("skill", "atomprobe");
		const beforeContent = await Bun.file(path.join(getManagedSkillsDir(), "atomprobe", "SKILL.md")).text();
		const renameSpy = spyOn(fs, "rename").mockRejectedValueOnce(new Error("disk boom"));
		await expect(
			(await import("@harvest/pi-coding-agent/autolearn/managed-skills")).promoteSkillRevision(
				"atomprobe",
				draft.id,
			),
		).rejects.toThrow(/disk boom/);
		renameSpy.mockRestore();
		const afterPointer = await readActivePointer("skill", "atomprobe");
		expect(afterPointer.active).toBe(beforePointer.active);
		expect(await Bun.file(path.join(getManagedSkillsDir(), "atomprobe", "SKILL.md")).text()).toBe(beforeContent);
	});

	it("interrupted transaction recovers to the published pointer", async () => {
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "txnprobe", description: SKILL_DESC, body: "v1" });
		const draft = await createSkillDraft({ name: "txnprobe", description: SKILL_DESC, body: "v2" });
		const root = getManagedSkillsDir();
		const file = path.join(root, "txnprobe", "SKILL.md");
		const pointerBefore = await readActivePointer("skill", "txnprobe");
		await Bun.write(
			path.join(root, "txnprobe.txn.json"),
			JSON.stringify({ revId: draft.id, previousActive: pointerBefore.active }),
		);
		await Bun.write(`${file}.tmp`, "v2-staged");
		await fs.rename(`${file}.tmp`, file);
		expect(await Bun.file(file).text()).toContain("v2");
		const { active } = await listSkillRevisions("txnprobe");
		expect(active).toBe(pointerBefore.active);
		expect(await Bun.file(file).text()).toContain("v1");
	});

	it("concurrent eval appends all survive", async () => {
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "concprobe", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "concprobe", description: SKILL_DESC, body: "cand" });
		let n = 0;
		setSkillEvalRunner(async () => {
			n++;
			await Bun.sleep(5);
			return { passed: true, summary: `run ${n}` };
		});
		await Promise.all(
			[1, 2, 3, 4, 5].map(i =>
				evaluateSkillRevision("concprobe", draft.id, { task: `task ${i}`, expectedOutcome: `out ${i}` }),
			),
		);
		const { revisions } = await listSkillRevisions("concprobe");
		const target = revisions.find(r => r.id === draft.id);
		expect(target?.evaluations.length).toBe(5);
	});

	it("concurrent preset eval appends all survive", async () => {
		await writeManagedPreset({
			action: "create",
			name: "conc-preset",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
		});
		const draft = await createPresetDraft({ name: "conc-preset", description: PRESET_DESC, systemPrompt: "cand" });
		setPresetEvalRunner(async () => {
			await Bun.sleep(5);
			return { passed: true, summary: "ok" };
		});
		await Promise.all(
			[1, 2, 3, 4].map(i =>
				evaluatePresetRevision("conc-preset", draft.id, { task: `t${i}`, expectedOutcome: `o${i}` }),
			),
		);
		const { revisions } = await listPresetRevisions("conc-preset");
		expect(revisions.find(r => r.id === draft.id)?.evaluations.length).toBe(4);
	});

	it("prune beyond twenty keeps active and pinned", async () => {
		const first = await createSkillDraft({ name: "pruneprobe", description: SKILL_DESC, body: "v0" });
		pinSkillRevision("pruneprobe", first.id);
		for (let i = 1; i <= 22; i++) {
			await createDraftRevision({
				kind: "skill",
				name: "pruneprobe",
				content: `content ${i}`,
				description: SKILL_DESC,
			});
		}
		const surviving = await pruneSkillRevisions("pruneprobe");
		expect(surviving.some(r => r.id === first.id)).toBe(true);
		expect(surviving.length).toBeLessThanOrEqual(21);
		unpinSkillRevision("pruneprobe", first.id);
	});

	it("stale expectedActive rejects with zero partial effects", async () => {
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "staleprobe", description: SKILL_DESC, body: "v1" });
		const beforeRevs = await listSkillRevisions("staleprobe");
		const beforeFile = await Bun.file(path.join(getManagedSkillsDir(), "staleprobe", "SKILL.md")).text();
		await expect(
			tool.execute("2", {
				action: "update",
				name: "staleprobe",
				description: SKILL_DESC,
				body: "v2",
				expectedActive: "rev-doesnotexist123",
			}),
		).rejects.toThrow(/Revision conflict/);
		const afterRevs = await listSkillRevisions("staleprobe");
		expect(afterRevs.active).toBe(beforeRevs.active);
		expect(afterRevs.revisions.length).toBe(beforeRevs.revisions.length);
		expect(await Bun.file(path.join(getManagedSkillsDir(), "staleprobe", "SKILL.md")).text()).toBe(beforeFile);
	});

	it("two runs pinning the same revision protect it until both settle", async () => {
		const first = await createSkillDraft({ name: "doublepin", description: SKILL_DESC, body: "v0" });
		pinSkillRevisionForRun("doublepin", first.id, "run-a");
		pinSkillRevisionForRun("doublepin", first.id, "run-b");
		for (let i = 1; i <= 22; i++) {
			await createDraftRevision({ kind: "skill", name: "doublepin", content: `c${i}`, description: SKILL_DESC });
		}
		let surviving = await pruneSkillRevisions("doublepin");
		expect(surviving.some(r => r.id === first.id)).toBe(true);
		unpinSkillRevisionForRun("doublepin", first.id, "run-a");
		expect(getSkillPins("doublepin").has(first.id)).toBe(true);
		surviving = await pruneSkillRevisions("doublepin");
		expect(surviving.some(r => r.id === first.id)).toBe(true);
		unpinSkillRevisionForRun("doublepin", first.id, "run-b");
		expect(getSkillPins("doublepin").has(first.id)).toBe(false);
		surviving = await pruneSkillRevisions("doublepin");
		expect(surviving.some(r => r.id === first.id)).toBe(false);
	});

	it("two preset runs pinning the same revision protect it until both settle", async () => {
		const first = await createPresetDraft({ name: "double-preset", description: PRESET_DESC, systemPrompt: "v0" });
		pinPresetRevisionForRun("double-preset", first.id, "run-a");
		pinPresetRevisionForRun("double-preset", first.id, "run-b");
		for (let i = 1; i <= 22; i++) {
			await createDraftRevision({
				kind: "preset",
				name: "double-preset",
				content: `c${i}`,
				description: PRESET_DESC,
			});
		}
		let surviving = await prunePresetRevisions("double-preset");
		expect(surviving.some(r => r.id === first.id)).toBe(true);
		unpinPresetRevisionForRun("double-preset", first.id, "run-a");
		surviving = await prunePresetRevisions("double-preset");
		expect(surviving.some(r => r.id === first.id)).toBe(true);
		unpinPresetRevisionForRun("double-preset", first.id, "run-b");
		surviving = await prunePresetRevisions("double-preset");
		expect(surviving.some(r => r.id === first.id)).toBe(false);
	});

	it("rollback refuses never-active drafts", async () => {
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "rbprobe", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "rbprobe", description: SKILL_DESC, body: "cand" });
		await expect(tool.execute("2", { action: "rollback", name: "rbprobe", revisionId: draft.id })).rejects.toThrow(
			/never active/,
		);
		expect(await Bun.file(path.join(getManagedSkillsDir(), "rbprobe", "SKILL.md")).text()).toContain("live");
	});

	it("learn(skill) update merges omitted fields", async () => {
		const session = makeToolSession(
			{ "autolearn.enabled": true, "memory.backend": "mnemopi" },
			{
				getMnemopiSessionState: () =>
					({
						sessionId: "sess-1",
						session: { sessionManager: { getCwd: () => "/tmp/work" } },
						rememberScoped: () => "mem-id",
					}) as unknown as import("@harvest/pi-coding-agent/mnemopi/state").MnemopiSessionState,
			},
		);
		await new LearnTool(session).execute("1", {
			memory: "lesson one",
			skill: { action: "create", name: "learnmerge", description: SKILL_DESC, body: "v1" },
		});
		await new LearnTool(session).execute("2", {
			memory: "lesson two",
			skill: { action: "update", name: "learnmerge", body: "v2" },
		});
		const content = await Bun.file(path.join(getManagedSkillsDir(), "learnmerge", "SKILL.md")).text();
		expect(content).toContain("v2");
		expect(content).toContain(SKILL_DESC);
	});

	it("expired and forged grants are denied", async () => {
		const registry = new AgentRegistry();
		const live: ManagedLiveSessionLike = fakeLive("foreign");
		registry.register({
			id: "foreign",
			displayName: "foreign",
			kind: "sub",
			parentId: "someone-else",
			session: live as unknown as AgentSession,
			status: "idle",
		});
		const expired = new SessionsTool({
			...makeToolSession({ "autolearn.enabled": true }),
			agentRegistry: registry,
			getSessionId: () => "caller",
			managedSessionGrant: { scope: "sessions:manage", expiresAt: Date.now() - 1_000 },
		});
		await expect(expired.execute("1", { action: "stop", sessionId: "foreign" })).rejects.toThrow(
			/host-provided grant or the existing approval path/,
		);
		const wrongScope = new SessionsTool({
			...makeToolSession({ "autolearn.enabled": true }),
			agentRegistry: registry,
			getSessionId: () => "caller",
			managedSessionGrant: { scope: "other:scope", expiresAt: Date.now() + 60_000 } as unknown as {
				scope: string;
				expiresAt: number;
			},
		});
		await expect(wrongScope.execute("2", { action: "stop", sessionId: "foreign" })).rejects.toThrow(
			/host-provided grant or the existing approval path/,
		);
	});

	it("list outputs expose draft, eval status, and pinned markers", async () => {
		const skillTool = new ManageSkillTool(async () => {});
		await skillTool.execute("1", { action: "create", name: "ui-probe", description: SKILL_DESC, body: "live" });
		const draft = await createSkillDraft({ name: "ui-probe", description: SKILL_DESC, body: "cand" });
		setSkillEvalRunner(async () => ({ passed: true, summary: "ok" }));
		await skillTool.execute("2", {
			action: "evaluate",
			name: "ui-probe",
			revisionId: draft.id,
			task: "t",
			expectedOutcome: "o",
		});
		const listed = await skillTool.execute("3", { action: "list", name: "ui-probe" });
		const details = listed.details as {
			revisions: Array<{ id: string; state: string; evalStatus: string; pinned: boolean; active: boolean }>;
		};
		expect(details.revisions.some(r => r.evalStatus === "passing")).toBe(true);
		expect(details.revisions.some(r => r.state === "draft")).toBe(true);
		expect(JSON.stringify(listed.content)).toMatch(/eval:/);

		const presetTool = new PresetsTool(makeToolSession({ "autolearn.enabled": true }, { cwd: tempCwd }));
		await presetTool.execute("1", {
			action: "create",
			name: "ui-preset",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
		});
		const listedPresets = await presetTool.execute("2", { action: "list" });
		expect(JSON.stringify(listedPresets.content)).toMatch(/draft|evaluate|promote|rollback/);
		const inspected = await presetTool.execute("3", { action: "inspect", name: "ui-preset" });
		expect((inspected.details as { revisions: unknown[] }).revisions.length).toBeGreaterThan(0);
	});

	it("preset rollback refuses never-active drafts", async () => {
		await writeManagedPreset({
			action: "create",
			name: "rb-preset",
			description: PRESET_DESC,
			systemPrompt: "original prompt",
		});
		const draft = await createPresetDraft({ name: "rb-preset", description: PRESET_DESC, systemPrompt: "cand" });
		const tool = new PresetsTool(makeToolSession({ "autolearn.enabled": true }, { cwd: tempCwd }));
		await expect(tool.execute("1", { action: "rollback", name: "rb-preset", revisionId: draft.id })).rejects.toThrow(
			/never active/,
		);
	});

	it("preset materialization failure leaves old active published", async () => {
		await writeManagedPreset({
			action: "create",
			name: "atom-preset",
			description: PRESET_DESC,
			systemPrompt: "v1 prompt",
		});
		const draft = await createPresetDraft({
			name: "atom-preset",
			description: PRESET_DESC,
			systemPrompt: "v2 prompt",
		});
		setPresetEvalRunner(async () => ({ passed: true, summary: "ok" }));
		const beforePointer = await readActivePointer("preset", "atom-preset");
		const beforeContent = await Bun.file(path.join(getManagedPresetsDir(), "atom-preset.md")).text();
		const renameSpy = spyOn(fs, "rename").mockRejectedValueOnce(new Error("disk boom"));
		const { promotePresetRevision } = await import("@harvest/pi-coding-agent/task/agents");
		await expect(promotePresetRevision("atom-preset", draft.id)).rejects.toThrow(/disk boom/);
		renameSpy.mockRestore();
		expect((await readActivePointer("preset", "atom-preset")).active).toBe(beforePointer.active);
		expect(await Bun.file(path.join(getManagedPresetsDir(), "atom-preset.md")).text()).toBe(beforeContent);
	});

	it("preset stale expectedActive rejects with zero partial effects", async () => {
		await writeManagedPreset({
			action: "create",
			name: "stale-preset",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
		});
		const before = await listPresetRevisions("stale-preset");
		const beforeFile = await Bun.file(path.join(getManagedPresetsDir(), "stale-preset.md")).text();
		await expect(
			writeManagedPreset({
				action: "update",
				name: "stale-preset",
				systemPrompt: "new prompt",
				expectedActive: "rev-doesnotexist123",
			}),
		).rejects.toThrow(/Revision conflict/);
		const after = await listPresetRevisions("stale-preset");
		expect(after.active).toBe(before.active);
		expect(after.revisions.length).toBe(before.revisions.length);
		expect(await Bun.file(path.join(getManagedPresetsDir(), "stale-preset.md")).text()).toBe(beforeFile);
	});
});
