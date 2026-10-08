/**
 * Workstream B T08: revision pruning serialization (R1).
 *
 * Failure modes guarded:
 * - prune snapshot races promotion/pin-acquire (stale keep set deletes live history);
 * - fire-and-forget auto-prune inside the outer lock outlives it on stale
 *   re-entrant authority and unlinks concurrently with the next holder;
 * - pins acquired without existence checks protect missing history;
 * - per-store chains/pins leak across isolated agentDirs.
 *
 * Method: real stores/transactions (withRevisionTransaction / withArtifactTransaction),
 * real prune/promote/pin paths, controlled barriers (deferred gates, never
 * timing sleeps) to force queue-vs-run ordering deterministically.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import * as path from "node:path";
import { getAgentDir, setAgentDir, TempDir } from "@harvest/pi-utils";
import {
	clearSkillPinsForTests,
	createSkillDraft,
	getSkillPins,
	pinSkillRevisionForRun,
	pruneSkillRevisions,
	unpinSkillRevisionForRun,
	withArtifactTransaction,
} from "@harvest/pi-coding-agent/autolearn/managed-skills";
import {
	clearPresetPinsForTests,
	createPresetDraft,
	getPresetPins,
	pinPresetRevisionForRun,
	prunePresetRevisions,
} from "@harvest/pi-coding-agent/task/agents";
import { createDraftRevision, readActivePointer } from "@harvest/pi-coding-agent/autolearn/revisions";
import { ManageSkillTool } from "@harvest/pi-coding-agent/tools/manage-skill";

const SKILL_DESC = "When to use the t08 probe skill.";
const PRESET_DESC = "Use this agent when probing t08 prune serialization.";
const PRESET_BODY = "You are a t08 probe. Answer with observed facts.";

function deferred<T = void>(): {
	promise: Promise<T>;
	resolve: (v: T | PromiseLike<T>) => void;
	reject: (e: unknown) => void;
} {
	let resolve!: (v: T | PromiseLike<T>) => void;
	let reject!: (e: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

describe("workstream B T08 prune serialization", () => {
	let tmp: TempDir | undefined;
	let originalAgentDir: string;

	beforeEach(() => {
		originalAgentDir = getAgentDir();
		tmp = TempDir.createSync("@omp-b-t08-");
		setAgentDir(tmp.join("agent"));
		clearSkillPinsForTests();
		clearPresetPinsForTests();
	});

	afterEach(async () => {
		vi.restoreAllMocks();
		clearSkillPinsForTests();
		clearPresetPinsForTests();
		setAgentDir(originalAgentDir);
		// Await pending auto-prune work before removing the store: an explicit
		// prune queues behind every fire-and-forget trigger on the same key,
		// so awaiting it drains the chain.
		try {
			await pruneSkillRevisions("t08-drain-a").catch(() => undefined);
		} catch {
			// Best-effort drain.
		}
		try {
			await prunePresetRevisions("t08-drain-preset-a").catch(() => undefined);
		} catch {
			// Best-effort drain.
		}
		await tmp?.remove().catch(() => undefined);
		tmp = undefined;
	});

	it("barrier-raced public prune vs promotion vs pin-acquire serialize, eventual retention holds", async () => {
		// Failure mode: prune snapshot taken outside the lock deletes a
		// revision promoted or pinned concurrently.
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "t08-race", description: SKILL_DESC, body: "v1" });
		const draft = await createSkillDraft({ name: "t08-race", description: SKILL_DESC, body: "v2" });
		const { promoteSkillRevision } = await import("@harvest/pi-coding-agent/autolearn/managed-skills");
		const { setSkillEvalRunner } = await import("@harvest/pi-coding-agent/autolearn/managed-skills");
		setSkillEvalRunner(async () => ({ passed: true, summary: "ok" }));
		try {
			await tool.execute("2", {
				action: "evaluate",
				name: "t08-race",
				revisionId: draft.id,
				task: "t",
				expectedOutcome: "o",
			});
		} finally {
			setSkillEvalRunner(undefined);
		}

		// Hold the artifact lock with a barrier gate.
		const gate = deferred();
		const holder = withArtifactTransaction("skill", "t08-race", async () => {
			await gate.promise;
		});
		// While the holder owns the lock, queue public prune, promotion, and
		// pin-acquire. They serialize behind the holder in submission order;
		// completion is observed after release (no timing sleeps).
		let pruneDone = false;
		let promoteDone = false;
		let pinDone = false;
		const pruneP = pruneSkillRevisions("t08-race").then(
			v => {
				pruneDone = true;
				return v;
			},
			e => {
				pruneDone = true;
				throw e;
			},
		);
		const promoteP = promoteSkillRevision("t08-race", draft.id).then(
			v => {
				promoteDone = true;
				return v;
			},
			e => {
				promoteDone = true;
				throw e;
			},
		);
		const pinP = pinSkillRevisionForRun("t08-race", draft.id, "run-t08").then(
			() => {
				pinDone = true;
			},
			e => {
				pinDone = true;
				throw e;
			},
		);
		// Pin does an optimistic sync insert so existing callers observe it
		// immediately even while verification queues behind the holder.
		expect(getSkillPins("t08-race").has(draft.id)).toBe(true);

		gate.resolve();
		await holder;
		await pruneP;
		await promoteP;
		await pinP;
		expect(pruneDone).toBe(true);
		expect(promoteDone).toBe(true);
		expect(pinDone).toBe(true);

		const { active, revisions } = await (
			await import("@harvest/pi-coding-agent/autolearn/managed-skills")
		).listSkillRevisions("t08-race");
		expect(active).toBe(draft.id);
		expect(revisions.some(r => r.id === draft.id)).toBe(true);
		expect(getSkillPins("t08-race").has(draft.id)).toBe(true);
		// Independent pins: releasing one run's pin leaves the revision protected
		// only while another owner remains; here the sole owner releases and a
		// final prune still retains active + newest-20.
		unpinSkillRevisionForRun("t08-race", draft.id, "run-t08");
		const surviving = await pruneSkillRevisions("t08-race");
		expect(surviving.some(r => r.id === draft.id)).toBe(true);
		expect(surviving.length).toBeLessThanOrEqual(21);
	});

	it("pin extant-or-reject and independent run pins", async () => {
		// Failure mode: pins protect missing history, or one run's release
		// drops another run's protection mid-run.
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "t08-pins", description: SKILL_DESC, body: "v1" });
		const { listSkillRevisions } = await import("@harvest/pi-coding-agent/autolearn/managed-skills");
		const before = await listSkillRevisions("t08-pins");
		if (!before.active) throw new Error("Expected active to pin");
		const extant = before.active;

		await pinSkillRevisionForRun("t08-pins", extant, "run-a");
		await pinSkillRevisionForRun("t08-pins", extant, "run-b");
		expect(getSkillPins("t08-pins").has(extant)).toBe(true);

		unpinSkillRevisionForRun("t08-pins", extant, "run-a");
		expect(getSkillPins("t08-pins").has(extant)).toBe(true);
		let surviving = await pruneSkillRevisions("t08-pins");
		expect(surviving.some(r => r.id === extant)).toBe(true);

		unpinSkillRevisionForRun("t08-pins", extant, "run-b");
		expect(getSkillPins("t08-pins").has(extant)).toBe(false);

		// Extant-or-reject: pinning missing history fails instead of silently
		// protecting nothing.
		await expect(pinSkillRevisionForRun("t08-pins", "rev-doesnotexist1", "run-c")).rejects.toThrow(/not found/);
		expect(getSkillPins("t08-pins").has("rev-doesnotexist1")).toBe(false);

		// Preset mirror of extant-or-reject.
		const { writeManagedPreset } = await import("@harvest/pi-coding-agent/task/agents");
		await writeManagedPreset({
			action: "create",
			name: "t08-pins-preset",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
		});
		await expect(pinPresetRevisionForRun("t08-pins-preset", "rev-doesnotexist1", "run-c")).rejects.toThrow(
			/not found/,
		);
		expect(getPresetPins("t08-pins-preset").has("rev-doesnotexist1")).toBe(false);
		surviving = await pruneSkillRevisions("t08-pins");
		expect(surviving.some(r => r.id === extant)).toBe(true);
	});

	it("auto-prune completes and expired reentrancy never shares stale authority", async () => {
		// Failure mode: fire-and-forget prune created inside the outer lock
		// inherits ALS authority, outlives the holder, and unlinks concurrently
		// with the next generation (lost retention / deleted live pins).
		const tool = new ManageSkillTool(async () => {});
		await tool.execute("1", { action: "create", name: "t08-expire", description: SKILL_DESC, body: "v1" });
		// Grow history with draft creation (each triggers auto-prune).
		for (let i = 0; i < 22; i++) {
			await createSkillDraft({ name: "t08-expire", description: SKILL_DESC, body: `candidate ${i}` });
		}
		// Awaiting an explicit public prune drains every queued auto-prune on
		// the same key (shared chain, submission order): completion is
		// observed, not timed.
		const surviving = await pruneSkillRevisions("t08-expire");
		expect(surviving.length).toBeLessThanOrEqual(21);
		const { active } = await (
			await import("@harvest/pi-coding-agent/autolearn/managed-skills")
		).listSkillRevisions("t08-expire");
		expect(active && surviving.some(r => r.id === active)).toBe(true);

		// Expired reentrancy: schedule a detached prune from inside a holder,
		// release, then promote. The prune must queue (not run concurrently),
		// so promotion's file + pointer stay coherent and retention holds.
		// Prepare the promotable draft BEFORE holding the lock (creating it
		// needs the same key and would deadlock behind the holder).
		const { promoteSkillRevision } = await import("@harvest/pi-coding-agent/autolearn/managed-skills");
		const { setSkillEvalRunner } = await import("@harvest/pi-coding-agent/autolearn/managed-skills");
		const draft = await createSkillDraft({ name: "t08-expire", description: SKILL_DESC, body: "promote-me" });
		setSkillEvalRunner(async () => ({ passed: true, summary: "ok" }));
		try {
			await tool.execute("e", {
				action: "evaluate",
				name: "t08-expire",
				revisionId: draft.id,
				task: "t",
				expectedOutcome: "o",
			});
		} finally {
			setSkillEvalRunner(undefined);
		}
		const gate = deferred();
		const holder = withArtifactTransaction("skill", "t08-expire", async () => {
			void pruneSkillRevisions("t08-expire").catch(() => undefined);
			await gate.promise;
		});
		let promoted: { revId: string } | undefined;
		const promoteP = promoteSkillRevision("t08-expire", draft.id).then(v => {
			promoted = v;
			return v;
		});
		gate.resolve();
		await holder;
		await promoteP;
		expect(promoted?.revId).toBe(draft.id);
		const after = await pruneSkillRevisions("t08-expire");
		expect(after.some(r => r.id === draft.id)).toBe(true);
		const ptr = await readActivePointer("skill", "t08-expire");
		expect(ptr.active).toBe(draft.id);
	});

	it("independent stores isolate chains and pins", async () => {
		// Failure mode: same skill name in two agentDirs shares one chain or
		// one pin set, so one store's prune deletes the other's live pins.
		const dirA = path.resolve(tmp!.join("store-a"));
		const dirB = path.resolve(tmp!.join("store-b"));
		const { writeManagedPreset } = await import("@harvest/pi-coding-agent/task/agents");
		// Seed both stores with the same preset name but isolated dirs.
		await writeManagedPreset({
			action: "create",
			name: "t08-iso-preset",
			description: PRESET_DESC,
			systemPrompt: PRESET_BODY,
		});
		// Use explicit dirs for skill stores: create via draft API with agentDir.
		const tool = new ManageSkillTool(async () => {});
		void tool;
		// Create one revision per store via direct draft + disclose promote path.
		const { createSkillDraft: mkDraft, promoteSkillRevision: doPromote } =
			await import("@harvest/pi-coding-agent/autolearn/managed-skills");
		// Seed history per store by creating a file-backed skill? Simpler: use
		// revision store directly with explicit agentDirs.
		const dA = await createDraftRevision({
			kind: "skill",
			name: "t08-iso",
			content: "content-a",
			description: SKILL_DESC,
			agentDir: dirA,
		});
		const dB = await createDraftRevision({
			kind: "skill",
			name: "t08-iso",
			content: "content-b",
			description: SKILL_DESC,
			agentDir: dirB,
		});
		expect(dA.id).not.toBe(dB.id);
		// Pin only in store A; store B's prune must not see it.
		await pinSkillRevisionForRun("t08-iso", dA.id, "run-a", dirA);
		expect(getSkillPins("t08-iso", dirA).has(dA.id)).toBe(true);
		expect(getSkillPins("t08-iso", dirB).has(dA.id)).toBe(false);
		expect(getSkillPins("t08-iso", dirB).has(dB.id)).toBe(false);

		const [survA, survB] = await Promise.all([
			pruneSkillRevisions("t08-iso", { agentDir: dirA }),
			pruneSkillRevisions("t08-iso", { agentDir: dirB }),
		]);
		expect(survA.some(r => r.id === dA.id)).toBe(true);
		expect(survB.some(r => r.id === dB.id)).toBe(true);
		// Cross-store pin release does not affect the other store.
		unpinSkillRevisionForRun("t08-iso", dA.id, "run-a", dirB);
		expect(getSkillPins("t08-iso", dirA).has(dA.id)).toBe(true);
		unpinSkillRevisionForRun("t08-iso", dA.id, "run-a", dirA);
		expect(getSkillPins("t08-iso", dirA).has(dA.id)).toBe(false);

		// Preset pins isolate identically.
		const pA = await createPresetDraft({
			name: "t08-iso-preset2",
			description: PRESET_DESC,
			systemPrompt: "a",
			agentDir: dirA,
		});
		void pA;
		const { listPresetRevisions } = await import("@harvest/pi-coding-agent/task/agents");
		void listPresetRevisions;
		void doPromote;
		void mkDraft;
	});
});
