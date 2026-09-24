import { afterAll, beforeAll, describe, expect, it } from "bun:test";
import { resetSettingsForTest, Settings } from "../src/config/settings";
import { type SegmentContext, renderSegment } from "../src/modes/components/status-line/segments";
import { StatusLineComponent } from "../src/modes/components/status-line/component";
import { FooterComponent } from "../src/modes/components/footer";
import { initTheme, theme } from "../src/modes/theme/theme";
import type { AgentSession } from "../src/session/agent-session";

beforeAll(async () => {
	resetSettingsForTest();
	await Settings.init({ inMemory: true });
	await initTheme();
});

afterAll(() => {
	resetSettingsForTest();
});

function createMockSession(): AgentSession {
	return {
		state: { messages: [], model: { id: "test-model", name: "Test Model" } },
		messages: [],
		systemPrompt: [],
		agent: { state: { tools: [] } },
		skills: [],
		isStreaming: false,
		isFastModeActive: () => false,
		isFastModeEnabled: () => false,
		isAutoThinking: false,
		autoResolvedThinkingLevel: () => undefined,
		getGoalModeState: () => null,
		getAsyncJobSnapshot: () => ({ running: [] }),
		getContextUsage: () => null,
		modelRegistry: { isUsingOAuth: () => false },
		sessionManager: {
			getSessionName: () => "laya-test",
			getEntries: () => [],
			getUsageStatistics: () => ({
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
			}),
		},
	} as unknown as AgentSession;
}

function createMockContext(overrides?: Partial<SegmentContext>): SegmentContext {
	return {
		session: createMockSession(),
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
		...overrides,
	};
}

describe("status line laya segment", () => {
	it("renders dim off status when Laya is disabled", () => {
		const ctx = createMockContext({
			laya: { enabled: false, connected: false },
		});
		const result = renderSegment("laya", ctx);
		expect(result.visible).toBe(true);
		expect(result.content).toContain(theme.status.disabled);
		expect(result.content).toContain("Laya off");
		expect(result.content).toBe(theme.fg("dim", `${theme.status.disabled} Laya off`));
	});

	it("hides segment when Laya is disabled and hideWhenOff option is enabled", () => {
		const ctx = createMockContext({
			laya: { enabled: false, connected: false },
			options: { laya: { hideWhenOff: true } },
		});
		const result = renderSegment("laya", ctx);
		expect(result.visible).toBe(false);
		expect(result.content).toBe("");
	});

	it("renders success status when Laya is enabled and connected", () => {
		const ctx = createMockContext({
			laya: { enabled: true, connected: true },
		});
		const result = renderSegment("laya", ctx);
		expect(result.visible).toBe(true);
		expect(result.content).toContain(theme.status.success);
		expect(result.content).toContain("Laya");
		expect(result.content).not.toContain("disconnected");
		expect(result.content).toBe(theme.fg("success", `${theme.status.success} Laya`));
	});

	it("renders error status when Laya is enabled but disconnected", () => {
		const ctx = createMockContext({
			laya: { enabled: true, connected: false },
		});
		const result = renderSegment("laya", ctx);
		expect(result.visible).toBe(true);
		expect(result.content).toContain(theme.status.error);
		expect(result.content).toContain("Laya (disconnected)");
		expect(result.content).toBe(theme.fg("error", `${theme.status.error} Laya (disconnected)`));
	});

	it("renders startup placeholder properly", () => {
		const ctx = createMockContext({
			laya: { enabled: true, connected: true },
			startupPlaceholder: true,
		});
		const result = renderSegment("laya", ctx);
		expect(result.visible).toBe(true);
		expect(result.content).toContain(theme.status.success);
		expect(result.content).toContain("…");
	});
});

describe("StatusLineComponent Laya integration", () => {
	it("updates segment output when setLayaStatus is called", () => {
		const session = createMockSession();
		const component = new StatusLineComponent(session);

		component.setLayaStatus({ enabled: true, connected: true });
		let topBorder = component.getTopBorder(120).content;
		expect(topBorder).toContain("Laya");
		expect(topBorder).toContain(theme.status.success);

		component.setLayaStatus({ enabled: true, connected: false });
		topBorder = component.getTopBorder(120).content;
		expect(topBorder).toContain("Laya (disconnected)");
		expect(topBorder).toContain(theme.status.error);

		component.setLayaStatus({ enabled: false, connected: false });
		topBorder = component.getTopBorder(120).content;
		expect(topBorder).toContain("Laya off");
		expect(topBorder).toContain(theme.status.disabled);
	});
});

describe("FooterComponent Laya integration", () => {
	it("renders connected status in stats line when enabled", () => {
		const session = createMockSession();
		const footer = new FooterComponent(session);

		footer.setLayaStatus({ enabled: true, connected: true });
		const lines = footer.render(120);
		expect(lines.some(line => line.includes("Laya"))).toBe(true);
		expect(lines.some(line => line.includes(theme.status.success))).toBe(true);
	});

	it("renders disconnected status in stats line when enabled but not running", () => {
		const session = createMockSession();
		const footer = new FooterComponent(session);

		footer.setLayaStatus({ enabled: true, connected: false });
		const lines = footer.render(120);
		expect(lines.some(line => line.includes("Laya (disconnected)"))).toBe(true);
		expect(lines.some(line => line.includes(theme.status.error))).toBe(true);
	});

	it("renders off status when setShowLayaWhenOff is enabled", () => {
		const session = createMockSession();
		const footer = new FooterComponent(session);

		footer.setLayaStatus({ enabled: false, connected: false });
		footer.setShowLayaWhenOff(true);
		const lines = footer.render(120);
		expect(lines.some(line => line.includes("Laya off"))).toBe(true);
		expect(lines.some(line => line.includes(theme.status.disabled))).toBe(true);
	});
});
