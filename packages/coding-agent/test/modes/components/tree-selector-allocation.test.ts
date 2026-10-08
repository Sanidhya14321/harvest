import { afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import { stripVTControlCharacters } from "node:util";
import { visibleWidth } from "@harvest/pi-tui";
import { TreeSelectorComponent } from "../../../src/modes/components/tree-selector";
import { createTheme, getBuiltinThemes } from "../../../src/modes/theme/loader";
import { setThemeInstance, theme, type Theme } from "../../../src/modes/theme/theme";
import type { SessionTreeNode } from "../../../src/session/session-entries";

let previousTheme: Theme;
beforeEach(() => {
	previousTheme = theme;
	setThemeInstance(createTheme(getBuiltinThemes().harvest, { mode: "none", symbolPresetOverride: "ascii" }));
});
afterEach(() => setThemeInstance(previousTheme));

function tree(): SessionTreeNode[] {
	const nodes: SessionTreeNode[] = Array.from({ length: 20 }, (_, index) => ({
		entry: {
			type: "message",
			id: `item-${index}`,
			parentId: index ? `item-${index - 1}` : null,
			timestamp: "2026-10-08T00:00:00.000Z",
			message: { role: "user", content: `entry ${index} complete text`, timestamp: index },
		},
		children: [],
	}));
	for (let i = 1; i < nodes.length; i++) nodes[i - 1]!.children.push(nodes[i]!);
	return [nodes[0]!];
}

describe("session tree allocation", () => {
	it("keeps the selected entry visible after shrinking and switches to its complete identifier", () => {
		const select = vi.fn();
		const selector = new TreeSelectorComponent(tree(), "item-0", 24, select, () => {});
		selector.render(80);
		selector.handleInput("\x1b[F");
		for (const height of [4, 3, 2, 1]) {
			selector.setMaxHeight(height);
			const lines = selector.render(40);
			expect(lines.length).toBeLessThanOrEqual(height);
			expect(lines.every(line => visibleWidth(line) === 40)).toBe(true);
			const text = stripVTControlCharacters(lines.join("\n"));
			expect(text).toContain("entry 19");
			expect(text).not.toMatch(/[^\x20-\x7e\n]/);
		}
		selector.handleInput("\r");
		expect(select).toHaveBeenCalledWith("item-19", { summarize: false });
	});

	it("keeps label input editable at one cell and saves the full label after expansion", () => {
		const save = vi.fn();
		const selector = new TreeSelectorComponent(
			tree(),
			"item-0",
			24,
			() => {},
			() => {},
			save,
		);
		selector.handleInput("L");
		const label = "a complete label that exceeds the tiny viewport";
		for (const character of label) selector.handleInput(character);
		selector.setMaxHeight(1);
		const tiny = selector.render(1);
		expect(tiny).toHaveLength(1);
		expect(visibleWidth(tiny[0]!)).toBe(1);
		selector.setMaxHeight(8);
		expect(stripVTControlCharacters(selector.render(80).join("\n"))).toContain(label);
		selector.handleInput("\r");
		expect(save).toHaveBeenCalledWith("item-0", label);
	});
});
