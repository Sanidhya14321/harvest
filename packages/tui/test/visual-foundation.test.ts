import { describe, expect, it } from "bun:test";
import {
	Box,
	BUILTIN_EDITOR_BORDER_STYLES,
	Editor,
	SelectList,
	SettingsList,
	type SettingsListTheme,
	Text,
} from "@harvest/pi-tui";
import { ScrollView } from "@harvest/pi-tui/components/scroll-view";
import { CURSOR_MARKER } from "@harvest/pi-tui/tui";
import {
	applyBackgroundToLine,
	Ellipsis,
	truncateToWidth,
	visibleWidth,
	wrapTextWithAnsi,
} from "@harvest/pi-tui/utils";
import { defaultEditorTheme } from "./test-themes";
import { VirtualTerminal } from "./virtual-terminal";

const asciiSymbols = defaultEditorTheme.symbols;
const selectedBackground = (text: string): string => `\x1b[48;2;30;40;50m${text}\x1b[49m`;
const settingsTheme: SettingsListTheme = {
	label: text => text,
	value: text => text,
	description: text => text,
	hint: text => text,
	cursor: "> ",
	selected: selectedBackground,
	symbols: asciiSymbols,
};

describe("visual foundation contracts", () => {
	it("keeps truncation within zero-width and sub-ellipsis ASCII allocations", () => {
		expect(truncateToWidth("abcdef", 0)).toBe("");
		expect(truncateToWidth("abcdef", 1, Ellipsis.Ascii)).toBe("a");
		expect(truncateToWidth("abcdef", 2, Ellipsis.Ascii)).toBe("ab");
		expect(truncateToWidth("abcdef", 3, Ellipsis.Ascii)).toBe("...");
		expect(visibleWidth(truncateToWidth("\x1b[31m界abcdef\x1b[0m", 1, Ellipsis.Ascii))).toBeLessThanOrEqual(1);
	});
	it("hard-wraps long text tokens without losing characters or foreground color across rows", () => {
		const rows = wrapTextWithAnsi("\x1b[31mABCDEFGHI\x1b[0m", 4, { hard: true });
		expect(rows.map(row => Bun.stripANSI(row))).toEqual(["ABCD", "EFGH", "I"]);
		expect(rows.every(row => visibleWidth(row) <= 4)).toBe(true);
		const terminal = new VirtualTerminal(4, 4);
		terminal.write(rows.join("\r\n"));
		expect(terminal.getViewportRowForegroundColumns(0)).toEqual([0, 1, 2, 3]);
		expect(terminal.getViewportRowForegroundColumns(1)).toEqual([0, 1, 2, 3]);
		expect(terminal.getViewportRowForegroundColumns(2)).toEqual([0]);
		terminal.write("\r\nx");
		expect(terminal.getViewportRowForegroundColumns(3)).toEqual([]);
	});
	it("restores a filled row after full resets while preserving inner backgrounds, OSC links, and cursor markers", () => {
		const link = "\x1b]8;;https://example.com\x07";
		const source = `a\x1b[0mb\x1b[48;2;80;90;100mc\x1b[49m${link}d\x1b]8;;\x07${CURSOR_MARKER}`;
		const row = applyBackgroundToLine(source, 8, selectedBackground);
		const terminal = new VirtualTerminal(8, 2);
		terminal.write(row);
		expect(terminal.getViewport()[0].trimEnd()).toBe("abcd");
		expect(terminal.getViewportRowBackgroundColumns(0)).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
		const cells = terminal.getViewportCellRows()[0];
		// The VT's bg word is the fifth word in each eight-word cell.
		expect(cells[2 * 8 + 4]).not.toBe(cells[4]);
		expect(cells[3 * 8 + 4]).toBe(cells[4]);
		expect(row).toContain(link);
		expect(row).toContain(CURSOR_MARKER);
		const sixel = "\x1bPq#0;2;0;0;0~\x1b\\";
		expect(applyBackgroundToLine(sixel, 0, selectedBackground)).toContain(sixel);
		terminal.write("\r\nnext");
		expect(terminal.getViewportRowBackgroundColumns(1)).toEqual([]);
	});

	it("keeps filled container render references stable until allocation, content, or palette changes", () => {
		let background = "\x1b[44m";
		let styles = 0;
		const text = new Text("body", 0, 0).setStyleFn(value => {
			styles += 1;
			return `\x1b[31m${value}\x1b[0m`;
		});
		const box = new Box(0, 0, row => applyBackgroundToLine(row, 0, value => `${background}${value}\x1b[49m`));
		box.addChild(text);
		const first = box.render(20);
		box.setPaddingX(0);
		box.setPaddingY(0);
		expect(box.render(20)).toBe(first);
		expect(styles).toBe(1);
		const terminal = new VirtualTerminal(20, 2);
		terminal.write(first[0]);
		const firstColor = terminal.getViewportCellRows()[0][4];
		background = "\x1b[45m";
		const recolored = box.render(20);
		expect(recolored).not.toBe(first);
		terminal.write(`\r${recolored[0]}`);
		expect(terminal.getViewportCellRows()[0][4]).not.toBe(firstColor);
		expect(terminal.getViewportRowBackgroundColumns(0)).toEqual(Array.from({ length: 20 }, (_, i) => i));
		expect(box.render(20)).toBe(recolored);
		const resized = box.render(24);
		expect(resized).not.toBe(recolored);
		expect(resized.every(row => visibleWidth(row) === 24)).toBe(true);
		text.setText("changed");
		expect(box.render(24)[0]).toContain("changed");
	});

	it("renders every built-in ASCII composer with bounded scrolling rows and a retained insertion marker", () => {
		for (const shape of BUILTIN_EDITOR_BORDER_STYLES) {
			const editor = new Editor(defaultEditorTheme);
			editor.setBorderStyle(shape);
			editor.setMaxHeight(5);
			editor.setScrollbarVisible(true);
			editor.focused = true;
			editor.setText(Array.from({ length: 20 }, (_, index) => `line ${index}`).join("\n"));
			const rows = editor.render(30);
			expect(
				rows.every(row => visibleWidth(row) <= 30),
				shape,
			).toBe(true);
			const plain = rows
				.map(row => Bun.stripANSI(row))
				.join("\n")
				.replaceAll(CURSOR_MARKER, "");
			expect(/[^\x00-\x7f]/.test(plain), shape).toBe(false);
			expect(plain, shape).toContain("line 19");
			expect(rows.join(""), shape).toContain(CURSOR_MARKER);
		}
	});

	it("uses symbol-policy scrollbar and ellipsis fallback at the overflow boundary", () => {
		const view = new ScrollView(["abcdefghijk", "second", "third"], { height: 2, symbols: asciiSymbols });
		const rows = view.render(7);
		expect(rows).toEqual(["abc...#", "second|"]);
		view.scrollToBottom();
		expect(view.render(7)).toEqual(["second|", "third #"]);
	});
	it("updates viewport symbols without moving the selected scroll window or overriding caller-owned glyphs", () => {
		const view = new ScrollView(["first", "secondlong", "thirdlong"], { height: 2 });
		view.scrollToBottom();
		expect(view.render(7)).toEqual(["secon…│", "third…█"]);
		view.setSymbols(asciiSymbols);
		expect(view.getScrollOffset()).toBe(1);
		expect(view.render(7)).toEqual(["sec...|", "thi...#"]);
		view.setSymbols(asciiSymbols);
		expect(view.render(7)).toEqual(["sec...|", "thi...#"]);
		const explicit = new ScrollView(["first", "secondlong", "thirdlong"], {
			height: 2,
			trackChar: ":",
			thumbChar: "=",
			ellipsis: Ellipsis.Omit,
		});
		explicit.scrollToBottom();
		explicit.setSymbols(asciiSymbols);
		expect(explicit.render(7)).toEqual(["second:", "thirdl="]);
	});

	it("collapses editor chrome before tiny allocations hide the cursor and restores the chosen shape after resize", () => {
		for (const shape of ["box", "field", "rail"]) {
			const editor = new Editor(defaultEditorTheme);
			editor.setBorderStyle(shape);
			editor.focused = true;
			editor.setText("tail");
			editor.setMaxHeight(1);
			expect(editor.render(0), shape).toEqual([""]);
			for (const width of [1, 2, 4]) {
				const rows = editor.render(width);
				expect(rows, shape).toHaveLength(1);
				expect(
					rows.every(row => visibleWidth(row) <= width),
					shape,
				).toBe(true);
				expect(rows.join(""), shape).toContain(CURSOR_MARKER);
			}
			editor.setMaxHeight(5);
			const resized = editor.render(30);
			expect(Bun.stripANSI(resized[0]), shape).toStartWith(shape === "box" ? "+" : "|");
			expect(editor.getBorderStyle(), shape).toBe(shape);
			expect(editor.getText(), shape).toBe("tail");
		}
	});

	it("paints a full selected list row and moves the band and hit map with keyboard navigation", () => {
		const list = new SelectList(
			[
				{ value: "one", label: "One" },
				{ value: "two", label: "Two" },
				{ value: "three", label: "Three" },
			],
			2,
			{ ...defaultEditorTheme.selectList, selected: selectedBackground },
			{ overflowSearch: false },
		);
		const terminal = new VirtualTerminal(20, 3);
		terminal.write(list.render(20).join("\r\n"));
		expect(terminal.getViewportRowBackgroundColumns(0)).toEqual(Array.from({ length: 19 }, (_, i) => i));
		expect(terminal.getViewportRowBackgroundColumns(1)).toEqual([]);
		list.handleInput("\x1b[B");
		terminal.write(`\x1b[H${list.render(20).join("\r\n")}`);
		expect(terminal.getViewportRowBackgroundColumns(0)).toEqual([]);
		expect(terminal.getViewportRowBackgroundColumns(1)).toEqual(Array.from({ length: 19 }, (_, i) => i));
		expect(list.hitTest(1)).toBe(1);
	});

	it("windows settings categories to the row budget and preserves section mouse targets and narrow flat selection", () => {
		const items = Array.from({ length: 8 }, (_, index) => [
			{ id: `heading${index}`, label: `Section ${index}`, currentValue: "", heading: true },
			{ id: `item${index}`, label: `Item ${index}`, currentValue: "off", values: ["off", "on"] },
		]).flat();
		const changes: string[] = [];
		const list = new SettingsList(
			items,
			2,
			settingsTheme,
			id => changes.push(id),
			() => {},
			{ hint: "", typeToSearch: false },
		);
		list.selectItem("item7");
		list.toggleSectionFocus();
		const split = list.render(100);
		expect(split).toHaveLength(6);
		expect(split.every(row => visibleWidth(row) <= 100)).toBe(true);
		expect(split[1]).toContain("Section 7");
		expect(list.hitTest(1, 0)).toBe("item7");
		expect(Bun.stripANSI(split.join("\n"))).not.toMatch(/[^\x00-\x7f]/);
		list.toggleSectionFocus();
		const flat = list.render(24);
		expect(flat.slice(0, 2).join("\n")).toContain("Item 7");
		expect(flat.every(row => visibleWidth(row) <= 24)).toBe(true);
		const terminal = new VirtualTerminal(24, 10);
		terminal.write(flat.join("\r\n"));
		expect(terminal.getViewportRowBackgroundColumns(1)).toEqual(Array.from({ length: 23 }, (_, i) => i));
		list.handleInput("\n");
		expect(changes).toEqual(["item7"]);
		expect(list.getSelectedItem()?.currentValue).toBe("on");
	});
});
