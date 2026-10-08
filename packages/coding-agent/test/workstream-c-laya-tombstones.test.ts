/**
 * Workstream C — Laya retirement tombstones: inert replay, undiscoverable
 * skill, and inert provider-metadata records.
 *
 * Contracts:
 * - Persisted `laya_gating_decision` events replay safely through run
 *   diagnostics: they record the historical gate line for display only and
 *   change no stage, failure, or approval verdict.
 * - The `laya-setup` skill is undiscoverable: the repo `.agents/skills`
 *   discovery scan (project walk-up candidates from a cwd inside the repo)
 *   never offers it.
 * - Legacy `type: "laya"` provider metadata never activates computer
 *   behavior: a full persisted-transcript replay carrying the legacy record
 *   ends idle with no failure and no approval verdict change.
 */
import { describe, expect, it } from "bun:test";
import * as os from "node:os";
import * as path from "node:path";
import { getProjectPathCandidates } from "@harvest/pi-coding-agent/discovery/agents";
import { scanSkillsFromDir } from "@harvest/pi-coding-agent/discovery/helpers";
import { RunDiagnosticsTracker } from "@harvest/pi-coding-agent/modes/run-diagnostics";
import type { AgentSessionEvent } from "@harvest/pi-coding-agent/session/agent-session-events";

const IDLE = () => false;

// Repo root: this test file lives at
// `<root>/packages/coding-agent/test/workstream-c-laya-tombstones.test.ts`.
const REPO_ROOT = path.resolve(import.meta.dir, "..", "..", "..");

describe("laya gating decision replays inertly", () => {
	it("records the historical gate line without changing stage or failure", () => {
		// Failure mode: replaying a pre-removal session either drops the
		// event (decode failure) or, worse, reactivates gating and stalls or
		// fails the run.
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent(
			"a",
			{ type: "tool_execution_start", toolCallId: "1", toolName: "read", args: {} } as AgentSessionEvent,
			IDLE,
			1100,
		);
		const before = tracker.snapshot("a", 1200);
		tracker.handleEvent(
			"a",
			{
				type: "laya_gating_decision",
				toolName: "write",
				latencyMs: 45,
				requireApproval: true,
				fallback: false,
				reason: "historical verdict",
			} as AgentSessionEvent,
			IDLE,
			1300,
		);
		const after = tracker.snapshot("a", 1400);
		expect(after.stage).toBe(before.stage);
		expect(after.activeTool).toBe(before.activeTool);
		expect(after.failure).toBe(before.failure);
		expect(after.lastGate).toEqual({ toolName: "write", latencyMs: 45, verdict: "approval-required" });
		expect(after.lastEvent).toBe("laya_gating_decision");
	});

	it("a fallback record replays as fallback without failing the run", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent(
			"a",
			{
				type: "laya_gating_decision",
				toolName: "bash",
				latencyMs: 300,
				requireApproval: true,
				fallback: true,
				reason: "timeout",
			} as AgentSessionEvent,
			IDLE,
			1500,
		);
		expect(tracker.snapshot("a")).toMatchObject({
			stage: "streaming",
			failure: undefined,
			lastGate: { toolName: "bash", latencyMs: 300, verdict: "fallback" },
		});
	});
});

describe("laya-setup skill is undiscoverable", () => {
	it("never surfaces laya-setup from the repo .agents/skills walk-up scan", async () => {
		// Failure mode: a retired install/launch skill keeps appearing in
		// skill discovery and offers a removed setup flow. Candidates come
		// from the real project walk-up starting at a cwd INSIDE the repo
		// (a nested package directory), so the repo `.agents/skills`
		// directory is reached by walking up — never from a fixture-relative
		// guess. The scan must prove itself by finding the live `brag`
		// skill; otherwise the laya-setup absence assertion is vacuous.
		const nestedCwd = path.join(REPO_ROOT, "packages", "coding-agent");
		const ctx = { cwd: nestedCwd, home: os.tmpdir(), repoRoot: REPO_ROOT } as never;
		const candidates = getProjectPathCandidates(ctx, "skills").filter(dir =>
			path.resolve(dir).startsWith(path.resolve(REPO_ROOT)),
		);
		// The walk-up must actually reach the repo skills directory; an empty
		// candidate list would make the absence assertion vacuous.
		expect(candidates.some(dir => path.resolve(dir) === path.join(REPO_ROOT, ".agents", "skills"))).toBe(true);
		const results = await Promise.all(
			candidates.map(dir => scanSkillsFromDir(ctx, { dir, providerId: "agents", level: "project" })),
		);
		const names = results.flatMap(result => result.items.map(item => item.name));
		const paths = results.flatMap(result => result.items.map(item => item.path));
		// Positive proof: the live repo skill is discovered through the same scan.
		expect(names).toContain("brag");
		expect(paths.some(skillPath => skillPath.includes(path.join(".agents", "skills", "brag")))).toBe(true);
		expect(names).not.toContain("laya-setup");
		expect(paths.some(skillPath => skillPath.includes("laya-setup"))).toBe(false);
	});
});

describe("legacy laya provider metadata is inert", () => {
	it("a persisted transcript carrying the legacy record replays to idle with no failure", () => {
		// Failure mode: a persisted `type: "laya"` tool call is misrouted
		// into computer replay/execution paths (or fails to decode), failing
		// or stalling the replayed run. The tracker must treat the whole
		// pre-removal fragment — gating decision plus surrounding tool
		// lifecycle — as history: the run drains to idle with no failure and
		// the gate line preserved for display only.
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent(
			"a",
			{ type: "tool_execution_start", toolCallId: "1", toolName: "write", args: {} } as AgentSessionEvent,
			IDLE,
			1100,
		);
		tracker.handleEvent(
			"a",
			{
				type: "laya_gating_decision",
				toolName: "write",
				latencyMs: 12,
				requireApproval: false,
				fallback: false,
				reason: "legacy laya record",
			} as AgentSessionEvent,
			IDLE,
			1150,
		);
		tracker.handleEvent(
			"a",
			{
				type: "tool_execution_end",
				toolCallId: "1",
				toolName: "write",
				result: { content: [{ type: "text", text: "ok" }] },
			} as unknown as AgentSessionEvent,
			IDLE,
			1200,
		);
		tracker.handleEvent("a", { type: "agent_end", messages: [] } as AgentSessionEvent, IDLE, 1300);
		const snapshot = tracker.snapshot("a", 1400);
		expect(snapshot.stage).toBe("idle");
		expect(snapshot.failure).toBeUndefined();
		expect(snapshot.activeTool).toBeUndefined();
		expect(snapshot.lastGate).toEqual({ toolName: "write", latencyMs: 12, verdict: "auto-cleared" });
	});

	it("unknown legacy metadata on tool events changes no lifecycle verdict", () => {
		// Failure mode: a persisted `type: "laya"` record attached to a tool
		// call is read as a computer safety gate and forces human approval
		// for a retired verdict. The diagnostics lifecycle must follow the
		// ordinary tool path — stage and active tool from the start event,
		// cleared by the end event — with the legacy record contributing
		// nothing but the historical gate line.
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent(
			"a",
			{
				type: "tool_execution_start",
				toolCallId: "1",
				toolName: "write",
				args: {},
				providerMetadata: { type: "laya", layaGatingRequired: true, layaGatingReason: "historical" },
			} as unknown as AgentSessionEvent,
			IDLE,
			1100,
		);
		expect(tracker.snapshot("a", 1150)).toMatchObject({ stage: "tool", activeTool: "write" });
		tracker.handleEvent(
			"a",
			{
				type: "laya_gating_decision",
				toolName: "write",
				latencyMs: 12,
				requireApproval: false,
				fallback: false,
				reason: "legacy laya record",
			} as AgentSessionEvent,
			IDLE,
			1160,
		);
		tracker.handleEvent(
			"a",
			{
				type: "tool_execution_end",
				toolCallId: "1",
				toolName: "write",
				result: { content: [{ type: "text", text: "ok" }] },
			} as unknown as AgentSessionEvent,
			IDLE,
			1200,
		);
		expect(tracker.snapshot("a", 1250)).toMatchObject({
			stage: "streaming",
			activeTool: undefined,
			failure: undefined,
			lastGate: { toolName: "write", latencyMs: 12, verdict: "auto-cleared" },
		});
	});
});
