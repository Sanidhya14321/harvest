import { expect, it, vi } from "bun:test";
import type { InteractiveModeContext } from "@harvest/pi-coding-agent/modes/types";
import { executeBuiltinSlashCommand } from "@harvest/pi-coding-agent/slash-commands/builtin-registry";
import { RunDiagnosticsTracker } from "@harvest/pi-coding-agent/modes/run-diagnostics";
import type { AgentSessionEvent } from "@harvest/pi-coding-agent/session/agent-session-events";

function contextWithTracker(tracker: RunDiagnosticsTracker | undefined): InteractiveModeContext {
	const showSessionInfo = vi.fn();
	const ctx = {
		sessionManager: { getSessionId: () => "session-one" },
		showSessionInfo,
		editor: { setText: vi.fn() },
		runDiagnostics: tracker,
	} as unknown as InteractiveModeContext;
	return ctx;
}

it("routes /diagnostics to the session run snapshot", async () => {
	const tracker = new RunDiagnosticsTracker();
	tracker.handleEvent("session-one", { type: "agent_start" } as AgentSessionEvent, () => false, 1000);
	tracker.handleEvent(
		"session-one",
		{ type: "tool_execution_start", toolCallId: "1", toolName: "read", args: {} } as AgentSessionEvent,
		() => true,
		1500,
	);
	const ctx = contextWithTracker(tracker);
	expect(await executeBuiltinSlashCommand("/diagnostics", { ctx })).toBe(true);
	const showSessionInfo = ctx.showSessionInfo as unknown as ReturnType<typeof vi.fn>;
	expect(showSessionInfo).toHaveBeenCalledTimes(1);
	const text = showSessionInfo.mock.calls[0]?.[0] as string;
	expect(text).toContain("Stage: tool");
	expect(text).toContain("Active tool: read");
});

it("reports unavailable when the context carries no tracker", async () => {
	const ctx = contextWithTracker(undefined);
	expect(await executeBuiltinSlashCommand("/diagnostics", { ctx })).toBe(true);
	const showSessionInfo = ctx.showSessionInfo as unknown as ReturnType<typeof vi.fn>;
	expect(showSessionInfo).toHaveBeenCalledWith("Run diagnostics are unavailable in this context.");
});
