import { describe, expect, it, vi } from "bun:test";
import { KeybindingsManager } from "@harvest/pi-coding-agent/config/keybindings";
import { AGENT_MODE_ORDER, agentModeDef, getAgentMode, nextAgentMode } from "@harvest/pi-coding-agent/modes/agent-mode";
import { CustomEditor } from "@harvest/pi-coding-agent/modes/components/custom-editor";
import { renderSegment } from "@harvest/pi-coding-agent/modes/components/status-line/segments";
import type { SegmentContext } from "@harvest/pi-coding-agent/modes/components/status-line/segments";
import { getEditorTheme, initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";

describe("agent mode ladder", () => {
	it("walks plan -> build -> accept-edits -> auto -> plan", () => {
		expect(AGENT_MODE_ORDER).toEqual(["plan", "build", "accept-edits", "auto"]);
		expect(nextAgentMode("plan")).toBe("build");
		expect(nextAgentMode("build")).toBe("accept-edits");
		expect(nextAgentMode("accept-edits")).toBe("auto");
		expect(nextAgentMode("auto")).toBe("plan");
	});

	it("resolves plan state over any approval value", () => {
		expect(getAgentMode(true, "yolo")).toBe("plan");
		expect(getAgentMode(true, "always-ask")).toBe("plan");
		expect(getAgentMode(false, "always-ask")).toBe("build");
		expect(getAgentMode(false, "write")).toBe("accept-edits");
		expect(getAgentMode(false, "yolo")).toBe("auto");
		expect(getAgentMode(false, "bogus")).toBe("auto");
	});

	it("exposes a label and approval mapping per mode", () => {
		expect(agentModeDef("plan").approvalMode).toBeUndefined();
		expect(agentModeDef("build").approvalMode).toBe("always-ask");
		expect(agentModeDef("accept-edits").approvalMode).toBe("write");
		expect(agentModeDef("auto").approvalMode).toBe("yolo");
	});
});

describe("shift+tab binding", () => {
	it("binds shift+tab to mode cycling and leaves thinking unbound", () => {
		const keybindings = KeybindingsManager.inMemory();
		expect(keybindings.getKeys("app.mode.cycle")).toContain("shift+tab");
		expect(keybindings.getKeys("app.thinking.cycle")).toEqual([]);
	});

	it("routes the shipped shift+tab default to the mode handler", async () => {
		await initTheme();
		const editor = new CustomEditor(getEditorTheme());
		const onCycleAgentMode = vi.fn();
		const onCycleThinkingLevel = vi.fn();

		editor.onCycleAgentMode = onCycleAgentMode;
		editor.onCycleThinkingLevel = onCycleThinkingLevel;
		editor.handleInput("\x1b[Z");

		expect(onCycleAgentMode).toHaveBeenCalledTimes(1);
		expect(onCycleThinkingLevel).not.toHaveBeenCalled();
	});
});

function modeCtx(approvalMode?: string): SegmentContext {
	return {
		session: {} as SegmentContext["session"],
		width: 120,
		compactThinkingLevel: false,
		options: {},
		planMode: null,
		loopMode: null,
		prewalk: null,
		goalMode: null,
		vibeMode: null,
		collab: null,
		approvalMode,
		usageStats: {
			input: 0,
			output: 0,
			cacheRead: 0,
			cacheWrite: 0,
			totalTokens: 0,
			orchestrationInput: 0,
			orchestrationOutput: 0,
			orchestrationCacheRead: 0,
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
		usage: null,
	};
}

describe("status line agent mode badge", () => {
	it("shows the approval-derived mode when no exclusive mode is active", async () => {
		await initTheme();
		expect(Bun.stripANSI(renderSegment("mode", modeCtx("always-ask")).content)).toContain("Build");
		expect(Bun.stripANSI(renderSegment("mode", modeCtx("write")).content)).toContain("Accept Edits");
		expect(Bun.stripANSI(renderSegment("mode", modeCtx("yolo")).content)).toContain("Auto");
		expect(renderSegment("mode", modeCtx("yolo")).visible).toBe(true);
	});

	it("keeps plan above the approval fallback", async () => {
		await initTheme();
		const ctx = { ...modeCtx("yolo"), planMode: { enabled: true, paused: false } };
		expect(Bun.stripANSI(renderSegment("mode", ctx).content)).toContain("Plan");
	});
});
