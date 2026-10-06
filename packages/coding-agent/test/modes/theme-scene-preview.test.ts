import { beforeAll, describe, expect, it } from "bun:test";
import { Settings, settings } from "@harvest/pi-coding-agent/config/settings";
import { renderComposerShapePreview } from "@harvest/pi-coding-agent/modes/components/composer-shape-preview";
import { StatusLineComponent } from "@harvest/pi-coding-agent/modes/components/status-line/component";
import { themeSetupScene } from "@harvest/pi-coding-agent/modes/setup-wizard/scenes/theme";
import type { SetupSceneHost } from "@harvest/pi-coding-agent/modes/setup-wizard/scenes/types";
import { initThemeSync } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { AgentSession } from "@harvest/pi-coding-agent/session/agent-session";

function makeSession() {
	return {
		messages: [],
		model: { name: "Preview Model", contextWindow: 128000 },
		contextUsageRevision: 0,
		systemPrompt: [],
		agent: { state: { tools: [] } },
		skills: [],
		getContextUsage: () => ({ tokens: 42, contextWindow: 128000 }),
		state: { messages: [] },
		sessionManager: {
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
				tokensPerSecond: null,
			}),
			getSessionName: () => "theme-preview-session",
		},
		getPrewalkState: () => undefined,
		getAsyncJobSnapshot: () => undefined,
		isAdvisorActive: () => false,
		getAdvisorStatusOverview: () => ({ configured: false, advisors: [] }),
		getAdvisorCost: () => 0,
		isAdvisorUsingSubscription: () => false,
		isFastModeActive: () => false,
		configuredThinkingLevel: () => undefined,
		modelRegistry: { isUsingOAuth: () => false },
	};
}

function mount(width: number): { lines: string[]; statusLine: StatusLineComponent } {
	const statusLine = new StatusLineComponent(makeSession() as unknown as AgentSession);
	const host = {
		ctx: { settings, statusLine },
		requestRender: () => {},
		finish: () => {},
		setFocus: () => {},
		restoreFocus: () => {},
	} as unknown as SetupSceneHost;
	const controller = themeSetupScene.mount(host);
	return { lines: [...controller.render(width)], statusLine };
}

beforeAll(async () => {
	await Settings.init({ inMemory: true });
	initThemeSync();
	settings.set("composer.shape", "band");
});

describe("theme setup scene preview", () => {
	it("renders the live status and composer chrome instead of mock rows", () => {
		const width = 100;
		const { lines, statusLine } = mount(width);
		const stripped = Bun.stripANSI(lines.join("\n"));

		// The real status pipeline's rows appear verbatim in the scene.
		const previewWidth = Math.max(24, Math.min(width, 88));
		const statusPreview = statusLine.getPreviewLines(previewWidth).map(line => Bun.stripANSI(line));
		for (const line of statusPreview) {
			if (line.trim()) expect(stripped).toContain(line);
		}

		// The editor block is the real composer-shape render for the
		// configured shape — the same getComposerStyle path runtime uses.
		const composerPreview = renderComposerShapePreview("band", width, statusLine).map(line => Bun.stripANSI(line));
		const chrome = composerPreview.filter(line => line.trim());
		expect(chrome.length).toBeGreaterThanOrEqual(2);
		for (const line of chrome) expect(stripped).toContain(line);

		// No mock residue: the old hand-drawn rows carried a fake session path
		// and a fake editor hint line the real pipeline never emits.
		expect(stripped).not.toContain("~/project");
		expect(stripped).not.toContain("shift+enter newline");
	});

	it("follows the configured composer shape through the real style path", () => {
		const first = Bun.stripANSI(mount(100).lines.join("\n"));
		settings.set("composer.shape", "box");
		try {
			const second = Bun.stripANSI(mount(100).lines.join("\n"));
			expect(second).not.toBe(first);
		} finally {
			settings.set("composer.shape", "band");
		}
	});
});
