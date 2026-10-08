import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { resetSettingsForTest, Settings } from "../src/config/settings";
import { FooterComponent } from "../src/modes/components/footer";
import { StatusLineComponent } from "../src/modes/components/status-line";
import { renderSegment } from "../src/modes/components/status-line/segments";
import type { SegmentContext } from "../src/modes/components/status-line/types";
import { createTheme, getBuiltinThemes } from "../src/modes/theme/loader";
import { setThemeInstance, theme, type Theme } from "../src/modes/theme/theme";
import type { AgentSession } from "../src/session/agent-session";
import { getProjectDir, setProjectDir } from "@harvest/pi-utils";

let previousTheme: Theme | undefined;
let previousProjectDir: string;
beforeEach(async () => {
	previousTheme = theme;
	previousProjectDir = getProjectDir();
	resetSettingsForTest();
	const settings = await Settings.init({ inMemory: true });
	settings.override("git.enabled", false);
	setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "none", symbolPresetOverride: "ascii" }));
});
afterEach(() => {
	setProjectDir(previousProjectDir);
	resetSettingsForTest();
	if (previousTheme) setThemeInstance(previousTheme);
});

function session(): AgentSession {
	return {
		state: { messages: [], model: undefined },
		messages: [],
		model: { contextWindow: 128000 },
		contextUsageRevision: 0,
		systemPrompt: [],
		agent: { state: { tools: [] } },
		skills: [],
		isStreaming: false,
		isAutoThinking: false,
		isFastModeActive: () => false,
		getAsyncJobSnapshot: () => ({ running: [] }),
		getCurrentModel: () => undefined,
		getContextUsage: () => ({ tokens: 0, contextWindow: 128000 }),
		getGoalModeState: () => null,
		modelRegistry: { isUsingOAuth: () => false },
		sessionManager: {
			getSessionName: () => "Named session",
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
			getEntries: () => [
				{
					type: "message",
					message: {
						role: "assistant",
						usage: {
							input: 10,
							output: 20,
							cacheRead: 0,
							cacheWrite: 0,
							cost: { total: 0.1 },
							premiumRequests: 1,
						},
					},
				},
			],
		},
	} as unknown as AgentSession;
}

describe("status surface fallback contracts", () => {
	it("keeps live, focused-agent, and startup status chrome ASCII and free of color escapes", () => {
		const value = session();
		const status = new StatusLineComponent(value);
		status.updateSettings({
			preset: "custom",
			leftSegments: ["pi", "model", "mode"],
			rightSegments: ["session_name"],
			separator: "powerline-thin",
			sessionAccent: true,
		});
		status.setSession(value, "worker-1");
		try {
			const lines = [
				status.getTopBorder(80).content,
				status.getBandTopBorder(80).content,
				status.renderBottomBar(80, "full"),
				status.renderStartupPlaceholder(80, "box"),
			];
			for (const line of lines) expect(line).not.toMatch(/[\u0080-\uFFFF]|\x1b/);
			expect(lines[0]).toContain("worker-1");
			expect(lines[3]).toContain("...");
			for (const width of [1, 2, 3, 4]) {
				for (const row of status.getPreviewLines(width, { statusAttachment: "top-rule-chip", bottomBar: "left" }))
					expect(Bun.stringWidth(row)).toBeLessThanOrEqual(width);
			}
			setThemeInstance(
				createTheme(getBuiltinThemes().harvest!, { mode: "truecolor", symbolPresetOverride: "ascii" }),
			);
			expect(status.getTopBorder(80).content).toMatch(/\x1b\[38;2;/);
			setThemeInstance(
				createTheme(getBuiltinThemes().harvest!, { mode: "256color", symbolPresetOverride: "ascii" }),
			);
			const limited = status.getTopBorder(80).content;
			expect(limited).toMatch(/\x1b\[38;5;/);
			expect(limited).not.toMatch(/\x1b\[38;2;/);
		} finally {
			status.dispose();
		}
	});

	it("left-truncates wide path labels in terminal cells instead of UTF-16 code units", () => {
		const context = {
			startupPlaceholder: false,
			options: { path: { maxLength: 4 } },
			git: { branch: "other" },
			worktree: { projectName: "界界界", worktreeName: "tail" },
		} as unknown as SegmentContext;
		const label = renderSegment("path", context).content;
		expect(Bun.stripANSI(label)).toContain("...l");
		expect(Bun.stripANSI(label)).not.toContain("界");
		expect(label).not.toContain("\x1b[38;");
	});

	it("keeps legacy footer paths, usage glyphs, and extension statuses inside tiny widths", () => {
		const footer = new FooterComponent(session());
		footer.setExtensionStatus("extension", "unsafe\t界界\nstatus");
		try {
			for (const width of [1, 2, 3, 4])
				for (const row of footer.render(width)) expect(Bun.stringWidth(row)).toBeLessThanOrEqual(width);
			const wide = footer.render(80).join("\n");
			expect(wide).toContain("in:10");
			expect(wide).toContain("out:20");
			expect(wide).toContain("* 1");
			expect(wide).not.toMatch(/[↑↓★…]|\t|\x1b/);
		} finally {
			footer.dispose();
		}
	});
});
