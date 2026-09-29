import { afterAll, beforeAll, describe, expect, it } from "bun:test";
import { stripVTControlCharacters } from "node:util";
import { resetSettingsForTest, Settings } from "../src/config/settings";
import { type SegmentContext, renderSegment } from "../src/modes/components/status-line/segments";
import { initTheme } from "../src/modes/theme/theme";
import { RunDiagnosticsTracker } from "../src/modes/run-diagnostics";
import type { AgentSession } from "../src/session/agent-session";
import type { AgentSessionEvent } from "../src/session/agent-session-events";

beforeAll(async () => {
	resetSettingsForTest();
	await Settings.init({ inMemory: true });
	await initTheme();
});

afterAll(() => {
	resetSettingsForTest();
});

function createMockContext(run: SegmentContext["run"]): SegmentContext {
	return {
		session: {} as unknown as AgentSession,
		width: 120,
		compactThinkingLevel: false,
		options: {},
		planMode: null,
		loopMode: null,
		prewalk: null,
		goalMode: null,
		vibeMode: null,
		collab: null,
		usageStats: {
			input: 0,
			output: 0,
			cacheRead: 0,
			cacheWrite: 0,
			totalTokens: 0,
			orchestrationInput: 0,
			orchestrationOutput: 0,
			orchestrationCacheRead: 0,
			orchestrationCacheWrite: 0,
			premiumRequests: 0,
			cost: 0,
			tokensPerSecond: null,
		},
		contextPercent: 0,
		contextTokens: 0,
		contextWindow: 0,
		autoCompactEnabled: false,
		compactionSpeculation: "idle",
		speculationBlinkOn: true,
		subagentCount: 0,
		activeMs: 0,
		turnElapsedMs: null,
		activeRepo: null,
		worktree: null,
		git: { branch: null, status: null, pr: null },
		run,
	} as unknown as SegmentContext;
}

function streamingRun(): SegmentContext["run"] {
	const tracker = new RunDiagnosticsTracker();
	tracker.handleEvent("s", { type: "agent_start" } as AgentSessionEvent, () => false, 1000);
	tracker.handleEvent(
		"s",
		{ type: "tool_execution_start", toolCallId: "1", toolName: "read", args: {} } as AgentSessionEvent,
		() => true,
		1500,
	);
	return tracker.snapshot("s", 13_000);
}

describe("run status-line segment", () => {
	it("stays hidden without a record or for a clean idle session", () => {
		expect(renderSegment("run", createMockContext(undefined))).toMatchObject({ visible: false });
		const tracker = new RunDiagnosticsTracker();
		expect(renderSegment("run", createMockContext(tracker.snapshot("s")))).toMatchObject({ visible: false });
	});

	it("shows the active tool with elapsed time while running", () => {
		const rendered = renderSegment("run", createMockContext(streamingRun()));
		expect(rendered.visible).toBe(true);
		const text = stripVTControlCharacters(rendered.content);
		expect(text).toContain("working");
		expect(text).toContain("read");
		expect(text).toContain("12");
	});

	it("labels retry, compaction, and delivery waits distinctly", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("s", { type: "agent_start" } as AgentSessionEvent, () => false, 1000);
		tracker.handleEvent(
			"s",
			{ type: "auto_retry_start", attempt: 1, maxAttempts: 3, delayMs: 500, errorMessage: "blip" },
			() => true,
			1500,
		);
		expect(
			stripVTControlCharacters(renderSegment("run", createMockContext(tracker.snapshot("s"))).content),
		).toContain("retry");
		tracker.handleEvent(
			"s",
			{ type: "auto_compaction_start", reason: "threshold", action: "context-full" },
			() => true,
			2000,
		);
		expect(
			stripVTControlCharacters(renderSegment("run", createMockContext(tracker.snapshot("s"))).content),
		).toContain("compacting");
	});

	it("surfaces the preserved failure with control characters stripped", () => {
		const tracker = new RunDiagnosticsTracker();
		tracker.handleEvent("s", { type: "agent_start" } as AgentSessionEvent, () => false, 1000);
		tracker.handleEvent(
			"s",
			{ type: "notice", level: "error", message: "socket reset\ninjected\tescape" },
			() => true,
			1500,
		);
		tracker.handleEvent("s", { type: "agent_end", messages: [] } as unknown as AgentSessionEvent, () => false, 2000);
		const rendered = renderSegment("run", createMockContext(tracker.snapshot("s", 9000)));
		expect(rendered.visible).toBe(true);
		const text = stripVTControlCharacters(rendered.content);
		expect(text).toContain("socket reset");
		expect(text).not.toContain("\n");
		expect(text).not.toContain("\t");
	});
});
