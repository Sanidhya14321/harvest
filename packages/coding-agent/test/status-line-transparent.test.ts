import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import { StatusLineComponent } from "@harvest/pi-coding-agent/modes/components/status-line";
import { createTheme, getBuiltinThemes } from "@harvest/pi-coding-agent/modes/theme/loader";
import { setThemeInstance, theme, type Theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { beginSettingsTest, restoreSettingsTestState, type SettingsTestState } from "./helpers/settings-test-state";

let settingsState: SettingsTestState | undefined;
let previousTheme: Theme | undefined;
beforeEach(async () => {
	previousTheme = theme;
	settingsState = beginSettingsTest();
	await Settings.init({ inMemory: true });
	setThemeInstance(createTheme(getBuiltinThemes().dark!, { mode: "truecolor" }));
});

afterEach(() => {
	restoreSettingsTestState(settingsState);
	if (previousTheme) setThemeInstance(previousTheme);
});

function makeSession() {
	return {
		state: { messages: [], model: undefined },
		messages: [],
		model: undefined,
		systemPrompt: [],
		agent: { state: { tools: [] } },
		skills: [],
		isStreaming: false,
		isAutoThinking: false,
		autoResolvedThinkingLevel: () => undefined,
		isFastModeActive: () => false,
		isFastModeEnabled: () => false,
		getGoalModeState: () => null,
		getAsyncJobSnapshot: () => ({ running: [] }),
		modelRegistry: { isUsingOAuth: () => false },
		sessionManager: {
			getSessionName: () => "transparent test",
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
	} as unknown as ConstructorParameters<typeof StatusLineComponent>[0];
}

function buildComponent(transparent: boolean) {
	const component = new StatusLineComponent(makeSession());
	component.updateSettings({
		preset: "custom",
		leftSegments: ["pi"],
		rightSegments: ["session_name"],
		separator: "powerline-thin",
		sessionAccent: false,
		transparent,
	});
	return component;
}

describe("status line transparent background", () => {
	it("paints the theme's statusLineBg when disabled (default)", () => {
		const themeBg = theme.getBgAnsi("statusLineBg");
		// Sanity check the test fixture: the default `dark` theme paints a real bg color,
		// otherwise the negative case below would be vacuous.
		expect(themeBg).toMatch(/\x1b\[48;/);

		const border = buildComponent(false).getTopBorder(80).content;
		expect(border).toContain(themeBg);
	});

	it("drops the theme bg fill and powerline caps when enabled", () => {
		const border = buildComponent(true).getTopBorder(80).content;
		const themeBg = theme.getBgAnsi("statusLineBg");

		// No 48; (background) ANSI escape anywhere in the rendered bar — every bg is
		// the terminal default (`\x1b[49m`).
		expect(border).not.toContain(themeBg);
		expect(border).not.toMatch(/\x1b\[48;/);
		expect(border).toContain("\x1b[49m");

		// Powerline-thin endcap glyphs are sourced from theme.sep.powerlineLeft/Right and
		// rely on the bg color as fg to visually bridge the bar; skipped under transparency.
		const leftCap = theme.sep.powerlineRight; // cap on the left side of right group
		const rightCap = theme.sep.powerlineLeft; // cap on the right side of left group
		expect(border).not.toContain(leftCap);
		expect(border).not.toContain(rightCap);
	});
});
