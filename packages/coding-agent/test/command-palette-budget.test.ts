import { afterEach, beforeAll, describe, expect, it } from "bun:test";
import { KeybindingsManager } from "@harvest/pi-coding-agent/config/keybindings";
import {
	CommandPaletteComponent,
	describePaletteDispatchError,
	describePaletteDispatchResult,
	mergePaletteItems,
	paletteListBudget,
	TAB_MANAGEMENT_PALETTE_SOURCES,
} from "@harvest/pi-coding-agent/modes/components/command-palette";
import { initTheme, setThemeInstance, theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { createTheme, getBuiltinThemes } from "@harvest/pi-coding-agent/modes/theme/loader";
import { setKeybindings, visibleWidth } from "@harvest/pi-tui";

beforeAll(() => {
	initTheme();
});

afterEach(() => {
	setKeybindings(KeybindingsManager.inMemory());
});

function hintedItems(count: number) {
	return Array.from({ length: count }, (_, i) => ({
		id: `/cmd-${i}`,
		title: `/cmd-${i}`,
		hint: `Does thing ${i}`,
	}));
}

describe("command palette viewport budget", () => {
	it("keeps the selected action and full dispatch payload through one-cell allocation and expansion", () => {
		const previousTheme = theme;
		setThemeInstance(createTheme(getBuiltinThemes().harvest, { mode: "none", symbolPresetOverride: "ascii" }));
		try {
			const palette = new CommandPaletteComponent();
			palette.setItems(hintedItems(25));
			palette.moveSelection(24);
			palette.setMaxHeight(4);
			const short = palette.render(24);
			expect(short.length).toBeLessThanOrEqual(4);
			expect(short.join("\n")).toContain("> /cmd-24");
			palette.setMaxHeight(1);
			const oneCell = palette.render(1);
			expect(oneCell).toHaveLength(1);
			expect(visibleWidth(oneCell[0]!)).toBe(1);
			palette.setMaxHeight(14);
			const expanded = palette.render(80);
			expect(expanded.length).toBeLessThanOrEqual(14);
			expect(expanded.join("\n")).toContain("> /cmd-24");
			expect(expanded.every(line => visibleWidth(line) === 80 && !/[^\x20-\x7e]/u.test(line))).toBe(true);
			const selected: string[] = [];
			palette.onSelect = item => selected.push(item.id);
			palette.handleInput("\r");
			palette.handleInput("\r");
			expect(selected).toEqual(["/cmd-24"]);
		} finally {
			setThemeInstance(previousTheme);
		}
	});
	it("caps the rendered panel at a 24-row viewport instead of a fixed ten", () => {
		// Failure mode: a fixed PAGE_SIZE=10 window plus description rows and
		// chrome overflows an 80x24 terminal, pushing the search prompt and
		// selected row off-screen.
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems(hintedItems(25));
		// 60% overlay share of 24 rows, like the production overlay sizing.
		palette.setMaxVisible(paletteListBudget(14));

		for (let i = 0; i < 24; i++) palette.moveSelection(1);
		const rendered = palette.render(80);
		// Chrome (top/prompt/bottom) + budgeted list body.
		expect(rendered.length).toBeLessThanOrEqual(3 + paletteListBudget(14));
		const text = Bun.stripANSI(rendered.join("\n"));
		expect(text).toContain(`${theme.nav.cursor} /cmd-24`);
		expect(text).toContain("/cmd-24");
	});

	it("keeps the selected row visible after resize and filtering", () => {
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems(hintedItems(25));
		palette.setMaxVisible(paletteListBudget(14));
		for (let i = 0; i < 20; i++) palette.moveSelection(1);
		expect(palette.selectedItem()?.id).toBe("/cmd-20");

		palette.setMaxVisible(4);
		expect(palette.visibleItems().map(item => item.id)).toContain("/cmd-20");

		palette.setQuery("/cmd-2");
		expect(palette.selectedItem()?.id).toBe("/cmd-2");
		expect(Bun.stripANSI(palette.render(80).join("\n"))).toContain("/cmd-2");
	});

	it("pages by the visible window and clamps at the top with few items", () => {
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems(hintedItems(25));
		palette.setMaxVisible(paletteListBudget(14));
		palette.render(80);
		expect(palette.pageSize()).toBeGreaterThan(1);

		const few = new CommandPaletteComponent();
		few.setItems(hintedItems(3));
		few.moveSelection(-few.pageSize());
		expect(few.selectedItem()?.id).toBe("/cmd-0");
	});
});

describe("command palette file/template intent", () => {
	it("drafts file/template commands with unknown arg metadata instead of bare-executing", () => {
		// Failure mode: a file/template command with no recorded arg metadata
		// executes immediately with no arguments (incomplete command runs).
		const items = mergePaletteItems(
			[{ name: "status", description: "builtin status", group: "builtin" }],
			[
				{ name: "notes.md", description: "file command", group: "file" },
				{ name: "review", description: "prompt template", group: "template" },
				{ name: "deploy", description: "explicitly arg-free file", group: "file", allowArgs: false },
			],
		);
		const byId = new Map(items.map(item => [item.id, item]));
		expect(byId.get("/notes.md")?.intent).toBe("draft");
		expect(byId.get("/review")?.intent).toBe("draft");
		expect(byId.get("/deploy")?.intent ?? "run").toBe("run");
		expect(byId.get("/status")?.intent ?? "run").toBe("run");
	});
});

describe("command palette dispatch feedback", () => {
	it("maps every dispatch result to visible feedback", () => {
		const draft = { id: "/tab close", title: "/tab close", intent: "draft" as const };
		expect(describePaletteDispatchResult(draft, false)).toEqual({ kind: "draft", text: "/tab close " });

		const run = { id: "/tab reopen", title: "/tab reopen" };
		expect(describePaletteDispatchResult(run, true)).toEqual({ kind: "status", text: "/tab reopen done" });
		expect(describePaletteDispatchResult(run, false)).toMatchObject({ kind: "error" });
		expect(describePaletteDispatchResult(run, false).text).toContain("/tab reopen");
		expect(describePaletteDispatchError(run, new Error("boom")).text).toContain("boom");
	});

	it("exposes tab Close + Reopen through palette items with remap-aware hints", () => {
		const items = mergePaletteItems(
			[{ name: "status", description: "builtin", group: "builtin" }],
			[...TAB_MANAGEMENT_PALETTE_SOURCES],
		);
		const byId = new Map(items.map(item => [item.id, item]));
		expect(byId.get("/tab close")?.intent).toBe("draft");
		expect(byId.get("/tab close")?.argHint).toBe("[number]");
		expect(byId.get("/tab reopen")?.intent ?? "run").toBe("run");

		const palette = new CommandPaletteComponent();
		palette.setKeybindings(KeybindingsManager.inMemory());
		palette.setItems(items);
		const text = Bun.stripANSI(palette.render(80).join("\n"));
		expect(text).toContain("Alt+W");

		palette.setKeybindings(KeybindingsManager.inMemory({ "app.session.tab.close": "ctrl+k" }));
		const remapped = Bun.stripANSI(palette.render(80).join("\n"));
		expect(remapped).toContain("Ctrl+K");
	});
});
