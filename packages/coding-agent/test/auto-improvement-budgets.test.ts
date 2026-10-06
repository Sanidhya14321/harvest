import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import { ManageSkillTool } from "@harvest/pi-coding-agent/tools/manage-skill";
import { PresetsTool } from "@harvest/pi-coding-agent/tools/presets";
import {
	clearImprovementBudgetsForTests,
	noteParentTurnCompleted,
} from "@harvest/pi-coding-agent/autolearn/managed-skills";
import { clearPresetImprovementBudgetsForTests } from "@harvest/pi-coding-agent/task/agents";
import { getAgentDir, setAgentDir } from "@harvest/pi-utils/dirs";

function autoSession(uuid: string): ToolSession {
	return {
		cwd: "/proj",
		settings: Settings.isolated({ "autolearn.enabled": true, "autolearn.autoContinue": true }),
		getSessionId: () => uuid,
		refreshSkills: async () => {},
	} as unknown as ToolSession;
}

function manualSession(uuid: string): ToolSession {
	return {
		cwd: "/proj",
		settings: Settings.isolated({ "autolearn.enabled": true }),
		getSessionId: () => uuid,
		refreshSkills: async () => {},
	} as unknown as ToolSession;
}

describe("automatic improvement budgets (R6 lifecycle)", () => {
	const created: string[] = [];
	const unique = (stem: string) =>
		`${stem}-${Date.now().toString(36)}-${Math.floor(Math.random() * 1296).toString(36)}`;
	let originalAgentDir = "";
	let tempHome = "";
	beforeEach(async () => {
		originalAgentDir = getAgentDir();
		tempHome = await fs.mkdtemp(path.join(os.tmpdir(), "omp-budget-"));
		setAgentDir(path.join(tempHome, ".harvest", "agent"));
	});
	afterEach(async () => {
		clearImprovementBudgetsForTests();
		clearPresetImprovementBudgetsForTests();
		const { deleteManagedSkill } = await import("@harvest/pi-coding-agent/autolearn/managed-skills");
		for (const name of created.splice(0)) {
			await deleteManagedSkill(name).catch(() => {});
		}
		setAgentDir(originalAgentDir);
		await fs.rm(tempHome, { recursive: true, force: true });
	});

	it("bounds auto-loop skill candidates to one per parent turn", async () => {
		const one = unique("budget-skill-one");
		const two = unique("budget-skill-two");
		created.push(one, two);
		const tool = new ManageSkillTool(autoSession("sess-budget-1"));
		await tool.execute("1", {
			action: "create",
			name: one,
			description: "Budget probe",
			body: "Do the thing.",
		});
		await expect(
			tool.execute("2", {
				action: "create",
				name: two,
				description: "Budget probe",
				body: "Do the other thing.",
			}),
		).rejects.toThrow(/already used its candidate slot/);
		// Next completed parent turn resets the allowance.
		noteParentTurnCompleted("sess-budget-1");
		await tool.execute("3", {
			action: "create",
			name: two,
			description: "Budget probe",
			body: "Do the other thing.",
		});
	});

	it("bounds auto-loop evaluations to one per parent turn", async () => {
		const name = unique("budget-eval-skill");
		created.push(name);
		created.push(name);
		const tool = new ManageSkillTool(autoSession("sess-budget-2"));
		await tool.execute("1", {
			action: "create",
			name,
			description: "Budget probe",
			body: "Do the thing.",
		});
		noteParentTurnCompleted("sess-budget-2");
		// No executor is wired in unit context: the budget claim runs first
		// and the missing-executor error proves the slot was claimed, not bypassed.
		await expect(
			tool.execute("2", {
				action: "evaluate",
				name,
				task: "Do the thing",
				expectedOutcome: "The thing is done",
			}),
		).rejects.toThrow(/No evaluation executor is wired/);
		await expect(
			tool.execute("3", {
				action: "evaluate",
				name,
				task: "Do the thing",
				expectedOutcome: "The thing is done",
			}),
		).rejects.toThrow(/already used its evaluation slot/);
	});

	it("never gates ordinary sessions without autoContinue", async () => {
		const one = unique("manual-skill-one");
		const two = unique("manual-skill-two");
		created.push(one, two);
		const tool = new ManageSkillTool(manualSession("sess-manual-1"));
		await tool.execute("1", {
			action: "create",
			name: one,
			description: "Manual probe",
			body: "Do the thing.",
		});
		await tool.execute("2", {
			action: "create",
			name: two,
			description: "Manual probe",
			body: "Do the other thing.",
		});
	});

	it("bounds auto-loop preset candidates per parent turn", async () => {
		const one = unique("budget-preset-one");
		const two = unique("budget-preset-two");
		const tool = new PresetsTool(autoSession("sess-budget-3"));
		await tool.execute("1", {
			action: "create",
			name: one,
			description: "Use this agent when probing budgets.",
			systemPrompt: "Probe.",
		});
		await expect(
			tool.execute("2", {
				action: "create",
				name: two,
				description: "Use this agent when probing budgets.",
				systemPrompt: "Probe.",
			}),
		).rejects.toThrow(/already used its candidate slot/);
		const { deleteManagedPreset } = await import("@harvest/pi-coding-agent/task/agents");
		await deleteManagedPreset(one).catch(() => {});
	});

	it("pins active skill revisions per run and releases one run without touching the other", async () => {
		const name = unique("budget-pin-skill");
		created.push(name);
		const tool = new ManageSkillTool(manualSession("sess-pin-1"));
		await tool.execute("1", { action: "create", name, description: "Pin probe", body: "Pin me." });
		const { readActiveRevision } = await import("@harvest/pi-coding-agent/autolearn/revisions");
		const { pinActiveSkillsForRun, unpinActiveSkillsForRun, getSkillPins } =
			await import("@harvest/pi-coding-agent/autolearn/managed-skills");
		const active = await readActiveRevision("skill", name);
		if (!active) throw new Error("Active revision is missing");
		const first = await pinActiveSkillsForRun("run-one");
		const second = await pinActiveSkillsForRun("run-two");
		expect(first.some(pair => pair.revId === active.id)).toBe(true);
		expect(getSkillPins(name).has(active.id)).toBe(true);
		unpinActiveSkillsForRun(first, "run-one");
		expect(getSkillPins(name).has(active.id)).toBe(true);
		unpinActiveSkillsForRun(second, "run-two");
		expect(getSkillPins(name).has(active.id)).toBe(false);
	});
});
