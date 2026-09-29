import { describe, expect, it } from "bun:test";
import type { AssistantMessage } from "@harvest/pi-ai";
import type { AgentSessionEvent } from "../../src/session/agent-session-events";
import { RunDiagnosticsTracker } from "../../src/modes/run-diagnostics";

const IDLE = () => false;
const ACTIVE = () => true;

function assistantMessage(overrides?: Partial<AssistantMessage>): AssistantMessage {
	return {
		role: "assistant",
		content: [{ type: "text", text: "done" }],
		api: "openai-completions",
		provider: "test",
		model: "test-model",
		usage: {},
		stopReason: "stop",
		...overrides,
	} as unknown as AssistantMessage;
}

function agentEnd(messages: unknown, isTerminal?: boolean): AgentSessionEvent {
	return { type: "agent_end", messages: messages as never, isTerminal } as unknown as AgentSessionEvent;
}

describe("RunDiagnosticsTracker", () => {
	it("reads unknown sessions as idle with zero elapsed", () => {
		const tracker = new RunDiagnosticsTracker();
		expect(tracker.snapshot("missing")).toMatchObject({ stage: "idle", elapsedMs: 0, failure: undefined });
	});

	it("tracks a clean run from start through tools to idle", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent(
			"a",
			{ type: "tool_execution_start", toolCallId: "1", toolName: "read", args: {} } as AgentSessionEvent,
			ACTIVE,
			1500,
		);
		expect(tracker.snapshot("a", 2000)).toMatchObject({ stage: "tool", activeTool: "read", elapsedMs: 1000 });
		tracker.handleEvent(
			"a",
			{ type: "tool_execution_end", toolCallId: "1", toolName: "read", result: "ok" } as AgentSessionEvent,
			ACTIVE,
			2500,
		);
		expect(tracker.snapshot("a", 2600)).toMatchObject({ stage: "streaming", activeTool: undefined });
		tracker.handleEvent("a", agentEnd([], true), IDLE, 3000);
		expect(tracker.snapshot("a", 9000)).toMatchObject({
			stage: "idle",
			elapsedMs: 0,
			failure: undefined,
			lastEvent: "agent_end",
		});
	});

	it("ignores a superseded end while the runtime still streams", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent("a", agentEnd([], true), ACTIVE, 1500);
		expect(tracker.snapshot("a", 1600)).toMatchObject({ stage: "streaming", elapsedMs: 600 });
	});

	it("holds the working state on a scheduling pause until delivery re-wakes the loop", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent("a", agentEnd([], false), IDLE, 2000);
		expect(tracker.snapshot("a", 2500)).toMatchObject({ stage: "awaitingDelivery", elapsedMs: 1500 });
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 3000);
		expect(tracker.snapshot("a", 4000)).toMatchObject({ stage: "streaming", elapsedMs: 3000 });
	});

	it("records a failed retry saga under its final error", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent(
			"a",
			{ type: "auto_retry_start", attempt: 1, maxAttempts: 3, delayMs: 500, errorMessage: "socket reset" },
			ACTIVE,
			1500,
		);
		expect(tracker.snapshot("a")).toMatchObject({ stage: "retry" });
		tracker.handleEvent("a", agentEnd([], true), ACTIVE, 1600);
		expect(tracker.snapshot("a")).toMatchObject({ stage: "retry", failure: undefined });
		tracker.handleEvent(
			"a",
			{ type: "auto_retry_end", success: false, attempt: 3, finalError: "socket reset (3 attempts)" },
			IDLE,
			2000,
		);
		tracker.handleEvent("a", agentEnd([], true), IDLE, 2100);
		expect(tracker.snapshot("a")).toMatchObject({ stage: "idle", failure: "socket reset (3 attempts)" });
	});

	it("clears a recovered saga and starts the next run without stale failure", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent(
			"a",
			{ type: "auto_retry_start", attempt: 1, maxAttempts: 3, delayMs: 500, errorMessage: "blip" },
			ACTIVE,
			1500,
		);
		tracker.handleEvent("a", { type: "auto_retry_end", success: true, attempt: 1 }, ACTIVE, 1600);
		tracker.handleEvent("a", agentEnd([assistantMessage()], true), IDLE, 2000);
		expect(tracker.snapshot("a")).toMatchObject({ stage: "idle", failure: undefined });
	});

	it("preserves an error-stop failure until the next run starts", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent(
			"a",
			agentEnd([assistantMessage({ stopReason: "error", errorMessage: "provider refused" })], true),
			IDLE,
			2000,
		);
		expect(tracker.snapshot("a", 9999)).toMatchObject({ stage: "idle", failure: "provider refused" });
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 10000);
		expect(tracker.snapshot("a", 10001)).toMatchObject({ stage: "streaming", failure: undefined });
	});

	it("records error notices and errored tools without ending the run", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent("a", { type: "notice", level: "error", message: "hook blew up" }, ACTIVE, 1500);
		expect(tracker.snapshot("a")).toMatchObject({ stage: "streaming", failure: "hook blew up" });
		tracker.handleEvent(
			"a",
			{ type: "tool_execution_start", toolCallId: "1", toolName: "bash", args: {} } as AgentSessionEvent,
			ACTIVE,
			1600,
		);
		tracker.handleEvent(
			"a",
			{ type: "tool_execution_end", toolCallId: "1", toolName: "bash", result: "exit 1", isError: true },
			ACTIVE,
			1700,
		);
		expect(tracker.snapshot("a")).toMatchObject({ stage: "streaming", failure: "exit 1" });
	});

	it("tracks compaction windows and records only real compaction failures", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent(
			"a",
			{ type: "auto_compaction_start", reason: "threshold", action: "context-full" },
			ACTIVE,
			1500,
		);
		expect(tracker.snapshot("a")).toMatchObject({ stage: "compaction" });
		tracker.handleEvent(
			"a",
			{ type: "auto_compaction_end", action: "context-full", result: undefined, aborted: false, willRetry: false },
			ACTIVE,
			2000,
		);
		expect(tracker.snapshot("a")).toMatchObject({ stage: "streaming", failure: undefined });
		tracker.handleEvent(
			"a",
			{
				type: "auto_compaction_end",
				action: "context-full",
				result: undefined,
				aborted: false,
				willRetry: false,
				errorMessage: "snapshot write failed",
			},
			ACTIVE,
			2500,
		);
		expect(tracker.snapshot("a")).toMatchObject({ failure: "snapshot write failed" });
	});

	it("keeps each session's stage, tool, and failure independent", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent(
			"a",
			{ type: "tool_execution_start", toolCallId: "1", toolName: "read", args: {} } as AgentSessionEvent,
			ACTIVE,
			1100,
		);
		tracker.handleEvent("b", { type: "agent_start" } as AgentSessionEvent, IDLE, 1200);
		tracker.handleEvent("b", agentEnd([], true), IDLE, 1300);
		expect(tracker.snapshot("a")).toMatchObject({ stage: "tool", activeTool: "read" });
		expect(tracker.snapshot("b")).toMatchObject({ stage: "idle" });
		tracker.clear("a");
		expect(tracker.snapshot("a")).toMatchObject({ stage: "idle", lastEvent: undefined });
	});

	it("truncates an overlong failure instead of keeping the whole turn text", () => {
		const tracker = new RunDiagnosticsTracker();
		const longText = "x".repeat(500);
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent(
			"a",
			agentEnd([assistantMessage({ stopReason: "error", content: [{ type: "text", text: longText }] })], true),
			IDLE,
			2000,
		);
		const failure = tracker.snapshot("a").failure ?? "";
		expect(failure.length).toBeLessThanOrEqual(301);
		expect(failure.endsWith("…")).toBe(true);
	});

	it("measures prompt-to-first-token latency once per saga", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent("a", { type: "message_start", message: {} } as unknown as AgentSessionEvent, ACTIVE, 1100);
		expect(tracker.snapshot("a").firstTokenMs).toBeUndefined();
		tracker.handleEvent(
			"a",
			{ type: "message_update", message: {}, assistantMessageEvent: {} } as unknown as AgentSessionEvent,
			ACTIVE,
			1400,
		);
		expect(tracker.snapshot("a").firstTokenMs).toBe(300);
		tracker.handleEvent(
			"a",
			{ type: "message_update", message: {}, assistantMessageEvent: {} } as unknown as AgentSessionEvent,
			ACTIVE,
			2000,
		);
		expect(tracker.snapshot("a").firstTokenMs).toBe(300);
	});

	it("leaves first-token latency absent when a turn streams no tokens", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent("a", { type: "message_start", message: {} } as unknown as AgentSessionEvent, ACTIVE, 1100);
		tracker.handleEvent("a", { type: "message_end", message: {} } as unknown as AgentSessionEvent, ACTIVE, 1200);
		tracker.handleEvent("a", agentEnd([], true), IDLE, 1300);
		expect(tracker.snapshot("a").firstTokenMs).toBeUndefined();
	});

	it("records finished tool durations and counts assistant turns", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 1000);
		tracker.handleEvent(
			"a",
			{ type: "tool_execution_start", toolCallId: "1", toolName: "read", args: {} } as AgentSessionEvent,
			ACTIVE,
			1500,
		);
		tracker.handleEvent(
			"a",
			{ type: "tool_execution_end", toolCallId: "1", toolName: "read", result: "ok" } as AgentSessionEvent,
			ACTIVE,
			2500,
		);
		tracker.handleEvent(
			"a",
			{ type: "turn_end", message: {}, toolResults: [] } as unknown as AgentSessionEvent,
			ACTIVE,
			2600,
		);
		expect(tracker.snapshot("a")).toMatchObject({
			lastTool: { name: "read", durationMs: 1000 },
			turnCount: 1,
		});
		tracker.handleEvent("a", agentEnd([], true), IDLE, 3000);
		expect(tracker.snapshot("a").lastTool).toMatchObject({ name: "read", durationMs: 1000 });
		tracker.handleEvent("a", { type: "agent_start" } as AgentSessionEvent, IDLE, 4000);
		expect(tracker.snapshot("a")).toMatchObject({ lastTool: undefined, turnCount: 0, firstTokenMs: undefined });
	});
});
