import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { KeybindingsManager } from "../../../src/config/keybindings";
import { HookInputComponent } from "../../../src/modes/components/hook-input";
import { OverlayPanel, renderDialog, splitRow } from "../../../src/modes/components/overlay-box";
import { QueueModeSelectorComponent } from "../../../src/modes/components/queue-mode-selector";
import { loadThemeSync } from "../../../src/modes/theme/loader";
import { setThemeInstance, theme, type Theme } from "../../../src/modes/theme/theme";
import { CURSOR_MARKER, getKeybindings, parseSgrMouse, setKeybindings, visibleWidth } from "@harvest/pi-tui";

let previousTheme: Theme | undefined;
let previousKeybindings = getKeybindings();
beforeEach(() => {
	previousTheme = theme;
	previousKeybindings = getKeybindings();
});
afterEach(() => {
	setKeybindings(previousKeybindings);
	if (previousTheme) setThemeInstance(previousTheme);
});

describe("dialog allocation contracts", () => {
	test("one to three columns retain the active body row instead of overflowing split chrome", () => {
		setThemeInstance(loadThemeSync("harvest", { mode: "none", symbolPresetOverride: "ascii" }));
		for (const width of [1, 2, 3]) {
			const layout = renderDialog("Title", ["a", "b", "z"], width, 1, "cancel", 2);
			expect(layout.lines.map(visibleWidth)).toEqual([width]);
			expect(layout.lines[0]?.trim()).toBe("z");
			const split = splitRow("category", "z", width, 16);
			expect(visibleWidth(split)).toBe(width);
			expect(split.trim()).toBe("z");
		}
	});

	test("stable allocation reuses filled rows while a theme change repaints their background", () => {
		setThemeInstance(loadThemeSync("harvest", { mode: "truecolor" }));
		const panel = new OverlayPanel("Details");
		const body = ["body"];
		panel.addChild({ render: () => body });
		panel.setMaxHeight(8);
		const first = panel.render(24);
		panel.setMaxHeight(8);
		expect(panel.render(24)).toBe(first);
		setThemeInstance(loadThemeSync("harvest-light", { mode: "truecolor" }));
		const light = panel.render(24);
		expect(light).not.toBe(first);
		expect(light[1]).not.toBe(first[1]);
		expect(light.map(visibleWidth)).toEqual(Array(light.length).fill(24));
	});

	test("a compact selected row remains clickable after the title and list window are reallocated", () => {
		setThemeInstance(loadThemeSync("harvest", { mode: "none", symbolPresetOverride: "ascii" }));
		const picks: string[] = [];
		const selector = new QueueModeSelectorComponent(
			"all",
			value => picks.push(value),
			() => {},
		);
		selector.setMaxHeight(4);
		const lines = selector.render(24);
		expect(lines.length).toBeLessThanOrEqual(4);
		const selectedLine = lines.findIndex(line => /\ball\b/.test(line));
		expect(selectedLine).toBeGreaterThanOrEqual(0);
		const mouse = parseSgrMouse(`\x1b[<0;5;${selectedLine + 1}M`);
		if (!mouse) throw new Error("invalid mouse fixture");
		selector.routeMouse(mouse, selectedLine, 4);
		expect(picks).toEqual(["all"]);
	});

	test("a two-row hook prompt preserves the focused pasted value and its remapped cancel action", () => {
		setThemeInstance(loadThemeSync("harvest", { mode: "none", symbolPresetOverride: "ascii" }));
		setKeybindings(KeybindingsManager.inMemory({ "app.interrupt": "ctrl+g" }));
		const values: string[] = [];
		let cancelled = false;
		const input = new HookInputComponent(
			"Question",
			undefined,
			value => values.push(value),
			() => {
				cancelled = true;
			},
		);
		input.focused = true;
		input.setMaxHeight(2);
		input.pasteText("mañana");
		const lines = input.render(24);
		expect(lines.length).toBeLessThanOrEqual(2);
		expect(lines.some(line => line.includes(CURSOR_MARKER) && line.includes("mañana"))).toBe(true);
		input.handleInput("\r");
		expect(values).toEqual(["mañana"]);
		input.handleInput("\x07");
		expect(cancelled).toBe(true);
	});
});
