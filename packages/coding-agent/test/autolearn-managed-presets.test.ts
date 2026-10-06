import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import {
	clearPresetPinsForTests,
	createPresetDraft,
	deleteManagedPreset,
	discoverManagedPresets,
	getManagedPresetsDir,
	isPresetNameClaimedByAuthored,
	listPresetRevisions,
	loadBundledAgents,
	parseAgent,
	pinPresetRevision,
	promotePresetRevision,
	prunePresetRevisions,
	sanitizePresetName,
	setPresetEvalRunner,
	unpinPresetRevision,
	writeManagedPreset,
} from "@harvest/pi-coding-agent/task/agents";
import { type SettingPath, Settings } from "@harvest/pi-coding-agent/config/settings";
import type { ToolSession } from "@harvest/pi-coding-agent/tools";
import { PresetsTool, setPresetArchitectRunner } from "@harvest/pi-coding-agent/tools/presets";
import { removeWithRetries } from "@harvest/pi-utils";
import { getAgentDir, setAgentDir } from "@harvest/pi-utils/dirs";

function makeSession(
	settingsOverrides: Partial<Record<SettingPath, unknown>> = {},
	extra: Partial<ToolSession> = {},
): ToolSession {
	return {
		cwd: "/tmp/preset-probe",
		hasUI: false,
		skipPythonPreflight: true,
		getSessionFile: () => null,
		getSessionSpawns: () => "*",
		settings: Settings.isolated(settingsOverrides),
		...extra,
	};
}

const VALID_DESC = "Use this agent when reviewing managed deltas.";
const VALID_BODY = "You are a delta reviewer. Check every hunk twice.";

describe("managed agent presets", () => {
	let tempHome: string;
	let tempCwd: string;
	let originalAgentDir: string;

	beforeEach(async () => {
		originalAgentDir = getAgentDir();
		tempHome = await fs.mkdtemp(path.join(os.tmpdir(), "omp-preset-"));
		tempCwd = await fs.mkdtemp(path.join(os.tmpdir(), "omp-preset-cwd-"));
		spyOn(os, "homedir").mockReturnValue(tempHome);
		setAgentDir(path.join(tempHome, ".omp", "agent"));
		setPresetEvalRunner(undefined);
		setPresetArchitectRunner(undefined);
		clearPresetPinsForTests();
	});

	afterEach(async () => {
		spyOn(os, "homedir").mockRestore();
		setAgentDir(originalAgentDir);
		setPresetEvalRunner(undefined);
		setPresetArchitectRunner(undefined);
		clearPresetPinsForTests();
		await removeWithRetries(tempHome);
		await removeWithRetries(tempCwd);
	});

	const tool = () => new PresetsTool(makeSession({ "autolearn.enabled": true }, { cwd: tempCwd }));

	it("validates preset names and descriptions", () => {
		expect(sanitizePresetName("  Delta-Reviewer ")).toBe("delta-reviewer");
		expect(() => sanitizePresetName("scout")).toThrow();
		expect(() => sanitizePresetName("UPPER")).toThrow();
		expect(() => sanitizePresetName("../escape")).toThrow();
	});

	it("saves a preset that parses under the agent contract and is discovered", async () => {
		const { path: file } = await writeManagedPreset({
			action: "create",
			name: "delta-reviewer",
			description: VALID_DESC,
			systemPrompt: VALID_BODY,
		});
		expect(file.startsWith(getManagedPresetsDir())).toBe(true);
		expect(file.startsWith(path.join(tempHome, ".omp", "agent", "managed-presets"))).toBe(true);

		// The materialized file parses under the same contract as discovery.
		const parsed = parseAgent(file, await Bun.file(file).text(), "user", "fatal");
		expect(parsed.name).toBe("delta-reviewer");
		expect(parsed.description).toBe(VALID_DESC);

		const managed = await discoverManagedPresets();
		expect(managed.map(agent => agent.name)).toContain("delta-reviewer");
	});

	it("supports a save → discover → spawn-routability round-trip", async () => {
		await tool().execute("1", {
			action: "create",
			name: "delta-reviewer",
			description: VALID_DESC,
			systemPrompt: VALID_BODY,
		});
		const managed = await discoverManagedPresets();
		expect(managed.some(agent => agent.name === "delta-reviewer")).toBe(true);

		const routability = await tool().checkSpawnRoutability("delta-reviewer");
		expect(routability.routable).toBe(true);
	});

	it("rejects name collisions, including authored and bundled claims", async () => {
		// Bundled claim: security-reviewer ships bundled and matches the pattern.
		expect(loadBundledAgents().some(agent => agent.name === "security-reviewer")).toBe(true);
		const bundled = await tool().execute("1", {
			action: "create",
			name: "security-reviewer",
			description: VALID_DESC,
			systemPrompt: VALID_BODY,
		});
		expect(bundled.isError).toBe(true);
		expect(JSON.stringify(bundled.content)).toMatch(/already claimed/);

		// Authored/discovered claim helper treats managed files as non-authored.
		await writeManagedPreset({
			action: "create",
			name: "delta-reviewer",
			description: VALID_DESC,
			systemPrompt: VALID_BODY,
		});
		const managed = await discoverManagedPresets();
		expect(isPresetNameClaimedByAuthored("delta-reviewer", managed)).toBe(false);
		expect(
			isPresetNameClaimedByAuthored("delta-reviewer", [
				...managed,
				{
					name: "delta-reviewer",
					description: "authored",
					systemPrompt: "authored",
					source: "user" as const,
					filePath: path.join(tempCwd, "agents", "delta-reviewer.md"),
				},
			]),
		).toBe(true);

		// Duplicate managed create fails instead of overwriting.
		await expect(
			writeManagedPreset({
				action: "create",
				name: "delta-reviewer",
				description: VALID_DESC,
				systemPrompt: VALID_BODY,
			}),
		).rejects.toThrow(/already exists/);
	});

	it("refuses to overwrite authored/bundled definitions and stays in the managed dir", async () => {
		const authored = await tool().execute("1", {
			action: "update",
			name: "security-reviewer",
			description: VALID_DESC,
			systemPrompt: VALID_BODY,
		});
		// security-reviewer is bundled-claimed, so the update is refused before any write.
		expect(authored.isError).toBe(true);
		expect(await Bun.file(path.join(getManagedPresetsDir(), "security-reviewer.md")).exists()).toBe(false);
	});

	it("rejects expected-revision conflicts on update", async () => {
		await writeManagedPreset({
			action: "create",
			name: "delta-reviewer",
			description: VALID_DESC,
			systemPrompt: VALID_BODY,
		});
		await expect(
			writeManagedPreset({
				action: "update",
				name: "delta-reviewer",
				description: VALID_DESC,
				systemPrompt: "changed",
				expectedActive: "rev-stale",
			}),
		).rejects.toThrow(/Revision conflict/);
	});

	it("failed preset evaluations block promotion", async () => {
		setPresetEvalRunner(async () => ({ passed: false, summary: "outcome diverged" }));
		await writeManagedPreset({
			action: "create",
			name: "delta-reviewer",
			description: VALID_DESC,
			systemPrompt: VALID_BODY,
		});
		const draft = await createPresetDraft({
			name: "delta-reviewer",
			description: VALID_DESC,
			systemPrompt: "candidate prompt",
		});
		const result = await tool().execute("1", {
			action: "evaluate",
			name: "delta-reviewer",
			revisionId: draft.id,
			task: "review a delta",
			expectedOutcome: "two findings reported",
		});
		expect((result.details as { passed: boolean }).passed).toBe(false);
		await expect(
			tool().execute("2", { action: "promote", name: "delta-reviewer", revisionId: draft.id }),
		).rejects.toThrow(/failing evaluations/);
	});

	it("unevaluated preset promotion discloses explicitly", async () => {
		await writeManagedPreset({
			action: "create",
			name: "delta-reviewer",
			description: VALID_DESC,
			systemPrompt: VALID_BODY,
		});
		const draft = await createPresetDraft({
			name: "delta-reviewer",
			description: VALID_DESC,
			systemPrompt: "candidate prompt",
		});
		await expect(promotePresetRevision("delta-reviewer", draft.id)).rejects.toThrow(/discloseUnevaluated/);
		const promoted = await tool().execute("1", {
			action: "promote",
			name: "delta-reviewer",
			revisionId: draft.id,
			discloseUnevaluated: true,
		});
		expect((promoted.details as { disclosedUnevaluated: boolean }).disclosedUnevaluated).toBe(true);
		expect(JSON.stringify(promoted.content)).toMatch(/unevaluated/);
	});

	it("pins survive preset prune", async () => {
		const first = await createPresetDraft({
			name: "pinned-preset",
			description: VALID_DESC,
			systemPrompt: VALID_BODY,
		});
		pinPresetRevision("pinned-preset", first.id);
		const { createDraftRevision } = await import("@harvest/pi-coding-agent/autolearn/revisions");
		for (let i = 0; i < 21; i++) {
			await createDraftRevision({
				kind: "preset",
				name: "pinned-preset",
				content: `c${i}`,
				description: VALID_DESC,
			});
		}
		const surviving = await prunePresetRevisions("pinned-preset");
		expect(surviving.some(rev => rev.id === first.id)).toBe(true);
		unpinPresetRevision("pinned-preset", first.id);
	});

	it("creates generated presets through the isolated architect pattern", async () => {
		setPresetArchitectRunner(async () =>
			JSON.stringify({
				identifier: "delta-reviewer",
				whenToUse: VALID_DESC,
				systemPrompt: VALID_BODY,
			}),
		);
		const result = await tool().execute("1", { action: "create", name: "delta-reviewer", brief: "review deltas" });
		expect((result.details as { name: string }).name).toBe("delta-reviewer");
		expect(await Bun.file(path.join(getManagedPresetsDir(), "delta-reviewer.md")).exists()).toBe(true);
		const { revisions } = await listPresetRevisions("delta-reviewer");
		expect(revisions.length).toBeGreaterThan(0);
	});

	it("rolls back to a prior preset revision", async () => {
		await writeManagedPreset({
			action: "create",
			name: "delta-reviewer",
			description: VALID_DESC,
			systemPrompt: "original prompt",
		});
		await writeManagedPreset({
			action: "update",
			name: "delta-reviewer",
			description: VALID_DESC,
			systemPrompt: "revised prompt",
		});
		const { revisions } = await listPresetRevisions("delta-reviewer");
		expect(revisions).toHaveLength(2);
		await tool().execute("1", { action: "rollback", name: "delta-reviewer", revisionId: revisions[0]!.id });
		expect(await Bun.file(path.join(getManagedPresetsDir(), "delta-reviewer.md")).text()).toContain(
			"original prompt",
		);
	});

	it("deletes the managed file but keeps history", async () => {
		await writeManagedPreset({
			action: "create",
			name: "delta-reviewer",
			description: VALID_DESC,
			systemPrompt: VALID_BODY,
		});
		await deleteManagedPreset("delta-reviewer");
		expect(await Bun.file(path.join(getManagedPresetsDir(), "delta-reviewer.md")).exists()).toBe(false);
		const { revisions } = await listPresetRevisions("delta-reviewer");
		expect(revisions.length).toBeGreaterThan(0);
	});
});
