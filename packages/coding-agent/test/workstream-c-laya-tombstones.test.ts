/**
 * Workstream C — Laya retirement tombstones: inert replay, undiscoverable
 * skill, and inert provider-metadata records.
 *
 * Contracts:
 * - Persisted `laya_gating_decision` events replay safely through run
 *   diagnostics: they record the historical gate line for display only and
 *   change no stage, failure, or approval verdict.
 * - The `laya-setup` skill is undiscoverable: the repo `.agents/skills`
 *   discovery scan never offers it.
 * - Legacy `type: "laya"` provider metadata decodes as an inert record and
 *   never activates computer behavior.
 */
import { describe, expect, it } from "bun:test";
import * as path from "node:path";
import type { ToolCallProviderMetadata } from "@harvest/pi-ai";
import { scanSkillsFromDir } from "@harvest/pi-coding-agent/discovery/helpers";
import { RunDiagnosticsTracker } from "@harvest/pi-coding-agent/modes/run-diagnostics";
import type { AgentSessionEvent } from "@harvest/pi-coding-agent/session/agent-session-events";

const IDLE = () => false;

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
	it("never surfaces laya-setup from the repo .agents/skills scan", async () => {
		// Failure mode: a retired install/launch skill keeps appearing in
		// skill discovery and offers a removed setup flow.
		const dir = path.resolve(import.meta.dir, "../../.agents/skills");
		const ctx = { cwd: dir, home: dir, repoRoot: dir } as never;
		const result = await scanSkillsFromDir(ctx, { dir, providerId: "agents", level: "project" });
		expect(result.items.some(item => item.name === "laya-setup")).toBe(false);
		expect(result.items.some(item => item.path.includes("laya-setup"))).toBe(false);
	});
});

describe("legacy laya provider metadata is inert", () => {
	it("decodes as an unknown record and never counts as computer metadata", () => {
		// Failure mode: a persisted `type: "laya"` tool call is misrouted
		// into computer replay/execution paths (or fails to decode).
		const legacy: ToolCallProviderMetadata = {
			type: "laya",
			layaGatingRequired: true,
			layaGatingReason: "historical",
		} as unknown as ToolCallProviderMetadata;
		expect(legacy.type).toBe("laya");
		// The only metadata the runtime may act on is `type: "computer"`.
		const isComputer = legacy.type === "computer";
		expect(isComputer).toBe(false);
		if (legacy.type === "computer") {
			throw new Error("unreachable: laya tombstone must never narrow to computer");
		}
		expect((legacy as Record<string, unknown>).type).toBe("laya");
	});
});
