import { afterEach, beforeEach, expect, test } from "bun:test";
import { BorderedLoader } from "../../../src/modes/components/bordered-loader";
import { BtwPanelComponent } from "../../../src/modes/components/btw-panel";
import { OmfgPanelComponent } from "../../../src/modes/components/omfg-panel";
import { createTheme, getBuiltinThemes } from "../../../src/modes/theme/loader";
import { setThemeInstance, theme } from "../../../src/modes/theme/theme";
import { type Component, type TUI, visibleWidth } from "@harvest/pi-tui";

let previousTheme = theme;
const host = { requestRender() {}, requestComponentRender() {} } as unknown as TUI;
beforeEach(() => {
	previousTheme = theme;
	setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "none", symbolPresetOverride: "ascii" }));
});
afterEach(() => {
	setThemeInstance(previousTheme);
});

function checkFrame(component: Component, width: number, height: number): string {
	component.setMaxHeight?.(height);
	const lines = component.render(width);
	expect(lines.length).toBeLessThanOrEqual(height);
	for (const line of lines) {
		expect(visibleWidth(line)).toBeLessThanOrEqual(width);
		expect(line).not.toMatch(/\x1b\[[\d;]*m/);
		expect(Bun.stripANSI(line)).not.toMatch(/[^\x00-\x7f]/);
	}
	return Bun.stripANSI(lines.join("\n"));
}

test("completed side answers keep copy/branch controls at short heights and preserve the full copy payload", () => {
	const panel = new BtwPanelComponent({ tui: host, question: "Question" });
	const answer = `${"Full answer\n".repeat(20)}Final answer`;
	panel.setAnswer(answer);
	panel.markComplete();
	for (const height of [1, 2, 4]) {
		const text = checkFrame(panel, 40, height);
		expect(text).toContain("c copy");
		expect(text).toContain("b branch");
	}
	expect(panel.getCopyText()).toBe(answer);
	panel.markError("Provider\tfailed");
	expect(checkFrame(panel, 24, 4).replace(/\s+/g, " ")).toContain("Provider failed");
	expect(panel.getCopyText()).toBeUndefined();
});

test("rule confirmation and failures retain their control or cause before optional status rows", () => {
	const panel = new OmfgPanelComponent({ tui: host, complaint: "Problem" });
	panel.setRule("Rule content");
	panel.setStatus("confirming", "Check the proposed rule");
	expect(checkFrame(panel, 40, 2)).toContain("Rule content");
	panel.markError("Validation\tfailed");
	expect(checkFrame(panel, 24, 3).replace(/\s+/g, " ")).toContain("Validation failed");
	for (const width of [1, 2, 3, 4]) checkFrame(panel, width, 1);
});

test("a one-row loader exposes Escape and cancellation aborts its signal", () => {
	const loader = new BorderedLoader(host, theme, "Working on a long task");
	try {
		expect(checkFrame(loader, 24, 1)).toContain("Esc cancel");
		loader.handleInput("\x1b");
		expect(loader.signal.aborted).toBe(true);
	} finally {
		loader.dispose();
	}
});
