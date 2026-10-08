import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { emptySidebarSnapshot, WorkspaceSidebar } from "../../src/modes/components/workspace-sidebar";
import { createTheme, getBuiltinThemes } from "../../src/modes/theme/loader";
import { setThemeInstance, theme, type Theme } from "../../src/modes/theme/theme";
import { VirtualTerminal } from "../../../tui/test/virtual-terminal";

let previousTheme: Theme | undefined;
beforeEach(() => {
	previousTheme = theme;
	setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "truecolor", symbolPresetOverride: "ascii" }));
});
afterEach(() => {
	if (previousTheme) setThemeInstance(previousTheme);
});

describe("workspace sidebar surface contracts", () => {
	it("keeps the selected section visible after scrolling past multi-row sections", () => {
		const sidebar = new WorkspaceSidebar();
		sidebar.setSnapshot({
			...emptySidebarSnapshot(),
			title: "session",
			mcpServers: [
				{ name: "files", connected: true },
				{ name: "search", connected: false },
			],
			todos: [{ label: "remaining task", done: false }],
		});
		sidebar.setFocused(true);
		sidebar.render(30, 3);
		sidebar.handleInput("down");
		sidebar.handleInput("down");
		const selected = sidebar.render(30, 3).map(row => Bun.stripANSI(row));
		expect(selected.some(row => row.includes("MCP"))).toBe(true);
		const scrollBeforeCollapse = sidebar.scrollOffset;
		sidebar.handleInput("enter");
		expect(sidebar.isCollapsed("MCP")).toBe(true);
		sidebar.handleInput("down");
		expect(sidebar.render(30, 3).some(row => Bun.stripANSI(row).includes("Todo"))).toBe(true);
		expect(sidebar.scrollOffset).toBeGreaterThanOrEqual(scrollBeforeCollapse);
	});

	it("paints the entire allocated sidebar including trailing empty rows and uses ASCII chrome", async () => {
		const sidebar = new WorkspaceSidebar();
		sidebar.setSnapshot({
			...emptySidebarSnapshot(),
			title: "session",
			mcpServers: [{ name: "files", connected: true }],
			todos: [{ label: "completed", done: true }],
		});
		const lines = sidebar.render(30, 24);
		expect(lines).toHaveLength(24);
		const text = Bun.stripANSI(lines.join("\n"));
		expect(text).toContain("[ok] completed");
		expect(text).not.toMatch(/[●○✓…│▎]/);
		const terminal = new VirtualTerminal(30, 24);
		terminal.write("\x1b[H" + lines.join("\r\n"));
		await terminal.waitForRender();
		const cells = terminal.getViewportCellRows();
		const expectedBackground = cells[0]![4];
		expect(expectedBackground).not.toBe(0);
		for (const row of cells) {
			for (let index = 4; index < row.length; index += 8) expect(row[index]).toBe(expectedBackground);
		}
		for (const width of [1, 2, 3]) {
			for (const line of sidebar.render(width, 3)) expect(Bun.stringWidth(line)).toBe(width);
		}
	});
});
