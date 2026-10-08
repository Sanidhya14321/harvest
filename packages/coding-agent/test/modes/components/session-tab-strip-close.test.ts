import { afterEach, beforeAll, beforeEach, describe, expect, it } from "bun:test";
import { SessionTabStrip, TAB_CLOSE_GLYPH } from "../../../src/modes/components/session-tab-strip";
import { initTheme, setThemeInstance, type Theme, theme } from "../../../src/modes/theme/theme";
import { createTheme, getBuiltinThemes } from "../../../src/modes/theme/loader";
import { SessionTabs } from "../../../src/session/session-tabs";

beforeAll(async () => {
	await initTheme(false);
});
let previousTheme: Theme;
beforeEach(() => {
	previousTheme = theme;
	setThemeInstance(createTheme(getBuiltinThemes().harvest, { mode: "256color", symbolPresetOverride: "unicode" }));
});
afterEach(() => setThemeInstance(previousTheme));

function twoTabStrip(callbacks: { selected?: string[]; closed?: string[]; targets?: unknown[] } = {}) {
	const tabs = new SessionTabs();
	tabs.open("/work/first.jsonl", "First task");
	tabs.open("/work/second.jsonl", "Second task");
	let selected = "/work/first.jsonl";
	const strip = new SessionTabStrip(
		tabs,
		() => selected,
		() => "First task",
		async path => {
			callbacks.selected?.push(path);
			selected = path;
		},
	);
	strip.setOnClose(path => {
		callbacks.closed?.push(path);
	});
	strip.setOnSelectTarget(async target => {
		callbacks.targets?.push(target);
		selected = target.path;
	});
	return { tabs, strip, getSelected: () => selected };
}

/** Screen column of the close (`x`) cell for the tab showing `title`. */
function closeColumn(strip: SessionTabStrip, strippedRow: string, title: string): number {
	const marker = `${title} ${TAB_CLOSE_GLYPH}`;
	const col = strippedRow.indexOf(marker);
	expect(col).toBeGreaterThan(-1);
	return col + marker.length - 1;
}

describe("session tab close column", () => {
	it("renders an ASCII x close column on active and inactive tabs", () => {
		const { strip } = twoTabStrip();
		const text = Bun.stripANSI(strip.renderWorkspace(100, true).join("\n"));
		expect(text).toContain(`First task ${TAB_CLOSE_GLYPH}`);
		expect(text).toContain(`Second task ${TAB_CLOSE_GLYPH}`);
		expect(text).not.toContain("×");
	});

	it("closes the inactive tab without activating it first", async () => {
		// Failure mode: the close hit-test falls through to the tab target,
		// so closing a background tab selects it (and its runtime) first.
		const callbacks: { selected: string[]; closed: string[] } = { selected: [], closed: [] };
		const { strip } = twoTabStrip(callbacks);
		const row = Bun.stripANSI(strip.renderWorkspace(100, true)[0] ?? "");

		const col = closeColumn(strip, row, "Second task");
		expect(strip.closeTargetAt(0, col)).toBe("/work/second.jsonl");
		expect(strip.clickWorkspace(0, col)).toBe(true);
		await Bun.sleep(0);

		expect(callbacks.closed).toEqual(["/work/second.jsonl"]);
		expect(callbacks.selected).toEqual([]);
	});

	it("keeps title clicks on the select path (close and select hit-tests are independent)", async () => {
		const callbacks: { selected: string[]; closed: string[] } = { selected: [], closed: [] };
		const { strip } = twoTabStrip(callbacks);
		const row = Bun.stripANSI(strip.renderWorkspace(100, true)[0] ?? "");

		const titleCol = row.indexOf("Second task") + 2;
		expect(strip.closeTargetAt(0, titleCol)).toBeUndefined();
		expect(strip.clickWorkspace(0, titleCol)).toBe(true);
		await Bun.sleep(0);

		expect(callbacks.closed).toEqual([]);
	});

	it("clears the open flash on keyboard selection changes, not only mouse clicks", () => {
		// Failure mode: keyboard next/prev never fires onTabChange, so the
		// flash highlight sticks on the wrong tab across keyboard navigation.
		const tabs = new SessionTabs();
		tabs.open("/work/first.jsonl", "First task");
		tabs.open("/work/second.jsonl", "Second task");
		let selected = "/work/first.jsonl";
		const strip = new SessionTabStrip(
			tabs,
			() => selected,
			() => "task",
			async () => {},
		);
		strip.renderWorkspace(100, true);
		tabs.open("/work/third.jsonl", "Third task");
		const flashed = strip.renderWorkspace(100, true).join("\n");
		const selectedBg = theme.getBgAnsi("selectedBg");
		expect(flashed.split(selectedBg).length - 1).toBe(2);

		// Keyboard selection change: current path moves with no click.
		selected = "/work/third.jsonl";
		const after = strip.renderWorkspace(100, true).join("\n");
		expect(after.split(selectedBg).length - 1).toBe(1);
	});

	it("keeps mouse hover independent of the flash state", () => {
		// Failure mode: the flash rewrites the stored hover target, so a real
		// pointer hover is lost (or the flash is cleared) on the next render.
		const { strip } = twoTabStrip();
		strip.renderWorkspace(100, true);
		const row = Bun.stripANSI(strip.renderWorkspace(100, true)[0] ?? "");
		// Hover the first tab while no flash is pending: hover sticks.
		expect(strip.hoverWorkspace(0, row.indexOf("First task"))).toBe(true);
		expect(strip.hoverWorkspace(0, row.indexOf("First task"))).toBe(false);
	});

	it("renders the Home empty state after the last tab closes", () => {
		const { tabs, strip } = twoTabStrip();
		const outcome = tabs.closeLastTabToHome();
		expect(outcome.home).toBe(true);
		const home = strip
			.renderHome(80)
			.map(line => Bun.stripANSI(line))
			.join("\n");
		expect(home).toContain("Home");
		expect(home).toContain("Reopen");
	});
});
