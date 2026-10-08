import { afterEach, beforeAll, beforeEach, describe, expect, it } from "bun:test";
import { agentPauseGate } from "@harvest/pi-agent-core";
import { type Component, visibleWidth } from "@harvest/pi-tui";
import { VirtualTerminal } from "../../../../tui/test/virtual-terminal";
import { resetSettingsForTest, Settings } from "../../../src/config/settings";
import {
	PauseScreenComponent,
	type PauseScreenHost,
	renderPauseScreen,
	runPauseScreen,
} from "../../../src/modes/components/pause-screen";
import { createTheme, getBuiltinThemes } from "../../../src/modes/theme/loader";
import { initTheme, setThemeInstance, type Theme, theme } from "../../../src/modes/theme/theme";

// Strip SGR colors so assertions see visible text only.
const stripAnsi = (text: string): string => text.replace(/\x1b\[[0-9;]*m/g, "");

interface FakeHost {
	host: PauseScreenHost;
	shown: Component[];
	statuses: string[];
	hiddenCount(): number;
}

function makeHost(rows = 24): FakeHost {
	const shown: Component[] = [];
	const statuses: string[] = [];
	let hidden = 0;
	const host: PauseScreenHost = {
		ui: {
			showOverlay(component) {
				shown.push(component);
				return {
					hide: () => {
						hidden++;
					},
					setHidden() {},
					isHidden: () => false,
				};
			},
			setFocus() {},
			requestRender() {},
			terminal: { rows },
		},
		showStatus(message) {
			statuses.push(message);
		},
	};
	return { host, shown, statuses, hiddenCount: () => hidden };
}

describe("pause screen", () => {
	let previousTheme: Theme;
	beforeAll(async () => {
		await initTheme(false);
	});
	beforeEach(async () => {
		await Settings.init({ inMemory: true });
		previousTheme = theme;
		setThemeInstance(createTheme(getBuiltinThemes().harvest, { mode: "truecolor", symbolPresetOverride: "unicode" }));
	});

	afterEach(() => {
		// The gate is process-global: never leak an engaged pause into other files.
		agentPauseGate.resume();
		setThemeInstance(previousTheme);
		resetSettingsForTest();
	});

	describe("renderPauseScreen", () => {
		it("paints exactly the requested rows with title, explainer, clock, and hint", () => {
			const lines = renderPauseScreen(80, 24, 65_000);
			expect(lines.length).toBe(24);
			const text = lines.map(stripAnsi).join("\n");
			expect(text).toContain("P A U S E D");
			expect(text).toContain("Main agent, subagents, and advisor");
			expect(text).toContain("paused for 1:05");
			expect(text).toContain("esc · enter · space — resume");
			expect(text).toContain("█".repeat(5));
		});

		it("drops to the compact card on small terminals", () => {
			const lines = renderPauseScreen(40, 10, 3_000);
			expect(lines.length).toBe(10);
			const text = lines.map(stripAnsi).join("\n");
			expect(text).toContain("▌▌ P A U S E D");
			expect(text).toContain("paused for 0:03");
			expect(text).toContain("esc to resume");
			expect(text).not.toContain("█".repeat(5)); // no room for the big glyph
		});

		it("rolls the clock into hours past 60 minutes", () => {
			const text = renderPauseScreen(80, 24, 3_725_000).map(stripAnsi).join("\n");
			expect(text).toContain("paused for 1:02:05");
		});

		it("displays the session name when provided in full mode", () => {
			const lines = renderPauseScreen(80, 24, 65_000, "My Awesome Session");
			const text = lines.map(stripAnsi).join("\n");
			expect(text).toContain("My Awesome Session");
			expect(text).toContain("P A U S E D");
		});

		it("displays the session name when provided in compact mode", () => {
			const lines = renderPauseScreen(40, 10, 3_000, "Compact Session Title");
			const text = lines.map(stripAnsi).join("\n");
			expect(text).toContain("Compact Session Title");
			expect(text).toContain("▌▌ P A U S E D");
		});

		it("keeps resume reachable when the allocation cannot also show the session and clock", () => {
			const one = renderPauseScreen(24, 1, 3_000, "Very long session title").map(stripAnsi);
			expect(one).toHaveLength(1);
			expect(one[0]).toContain("esc to resume");
			const two = renderPauseScreen(24, 2, 3_000, "Very long session title").map(stripAnsi);
			expect(two.join("\n")).toContain("P A U S E D");
			expect(two[1]).toContain("esc to resume");
			const four = renderPauseScreen(24, 4, 3_000, "Very long session title").map(stripAnsi);
			expect(four).toHaveLength(4);
			expect(four.join("\n")).toContain("paused for 0:03");
			expect(four[3]).toContain("esc to resume");
			expect(four.every(line => visibleWidth(line) <= 24)).toBe(true);
		});

		it("uses ASCII chrome without colors and fills every row of the pause surface when colors are enabled", () => {
			setThemeInstance(createTheme(getBuiltinThemes().harvest, { mode: "none", symbolPresetOverride: "ascii" }));
			const full = renderPauseScreen(80, 24, 3_000);
			expect(full.join("\n")).not.toMatch(/[^\x00-\x7f]/);
			expect(full.join("\n")).toContain("#####");
			const narrow = renderPauseScreen(2, 4, 3_000, "Long\tSession\nTitle");
			expect(narrow.every(line => visibleWidth(line) <= 2)).toBe(true);
			expect(narrow.join("\n")).not.toContain("\x1b");
			setThemeInstance(createTheme(getBuiltinThemes().harvest, { mode: "truecolor" }));
			const colored = renderPauseScreen(24, 4, 3_000);
			const terminal = new VirtualTerminal(24, 5);
			terminal.write(colored.join("\r\n"));
			for (let row = 0; row < 4; row++)
				expect(terminal.getViewportRowBackgroundColumns(row)).toEqual(Array.from({ length: 24 }, (_, col) => col));
		});

		it("uses the overlay allocation during resize rather than the full terminal height", () => {
			const { host } = makeHost(24);
			const component = new PauseScreenComponent({ ...host, sessionName: "Selected session" });
			component.setMaxHeight(4);
			expect(component.render(24)).toHaveLength(4);
			expect(component.render(24).map(stripAnsi).join("\n")).toContain("esc to resume");
			component.setMaxHeight(24);
			expect(component.render(80)).toHaveLength(24);
			expect(component.render(80).map(stripAnsi).join("\n")).toContain("Selected session");
			component.dispose();
		});
	});

	describe("runPauseScreen", () => {
		it("engages the gate for the screen's lifetime and releases it on escape", async () => {
			const { host, shown, statuses, hiddenCount } = makeHost();
			expect(agentPauseGate.paused).toBe(false);

			const run = runPauseScreen(host);
			await Bun.sleep(1);
			expect(agentPauseGate.paused).toBe(true);
			expect(shown.length).toBe(1);

			const component = shown[0];
			expect(component).toBeInstanceOf(PauseScreenComponent);
			if (component instanceof PauseScreenComponent) {
				component.handleInput("\x1b"); // escape → resume
			}
			await run;

			expect(agentPauseGate.paused).toBe(false);
			expect(hiddenCount()).toBe(1);
			expect(statuses.some(message => message.includes("Resumed after"))).toBe(true);
		});

		it("treats ctrl+c as resume, never as abort-and-stay-paused", async () => {
			const { host, shown } = makeHost();
			const run = runPauseScreen(host);
			await Bun.sleep(1);

			const component = shown[0];
			if (component instanceof PauseScreenComponent) {
				component.handleInput("\x03"); // ctrl+c
			}
			await run;
			expect(agentPauseGate.paused).toBe(false);
		});

		it("is a no-op when the gate is already engaged elsewhere", async () => {
			agentPauseGate.pause();
			const { host, shown } = makeHost();
			await runPauseScreen(host); // must resolve immediately, not park
			expect(shown.length).toBe(0);
			expect(agentPauseGate.paused).toBe(true); // foreign pause not stolen
		});
	});
});
