import { afterEach, beforeAll, describe, expect, it } from "bun:test";
import { KeybindingsManager } from "@harvest/pi-coding-agent/config/keybindings";
import { CommandPaletteComponent } from "@harvest/pi-coding-agent/modes/components/command-palette";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { setKeybindings } from "@harvest/pi-tui";

const PAGE_UP = "\x1b[5~";
const ENTER = "\n";

function makeItems(count: number): { id: string; title: string }[] {
	return Array.from({ length: count }, (_, i) => ({
		id: `cmd-${i}`,
		title: `Command ${String(i).padStart(2, "0")}`,
	}));
}

beforeAll(() => {
	initTheme();
});

afterEach(() => {
	setKeybindings(KeybindingsManager.inMemory());
});

describe("command palette navigation", () => {
	it("keeps the selected row inside the rendered window past 10 items", () => {
		// Failure mode: selection traverses invisible items — render() sliced
		// the first 10 rows while selection moved over the full list, so the
		// highlighted row was never drawn once selected >= 10.
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems(makeItems(25));

		for (let i = 0; i < 12; i++) palette.moveSelection(1);

		const selected = palette.selectedItem();
		expect(selected?.title).toBe("Command 12");
		const rendered = palette.render(80).join("\n");
		expect(rendered).toContain("Command 12");
	});

	it("clamps PageUp at the top with few items so Enter still selects", () => {
		// Failure mode: PageUp underflow — (0 - 10 + len) % len stays negative
		// in JS when len < 10, leaving selected at -1 so Enter no-ops.
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems(makeItems(3));

		palette.handleInput(PAGE_UP);

		const selected = palette.selectedItem();
		expect(selected?.title).toBe("Command 00");

		const picked: string[] = [];
		palette.onSelect = item => picked.push(item.id);
		palette.handleInput(ENTER);
		expect(picked).toEqual(["cmd-0"]);
	});

	it("appends multi-char paste/IME input to the query and filters", () => {
		// Failure mode: pasted/IME-committed runs dropped — handleInput only
		// accepted single chars, so the query never filtered on paste.
		setKeybindings(KeybindingsManager.inMemory());
		const palette = new CommandPaletteComponent();
		palette.setItems([
			{ id: "alpha", title: "Alpha" },
			{ id: "beta", title: "Beta" },
		]);

		palette.handleInput("alp");

		expect(palette.query).toBe("alp");
		expect(palette.filtered().map(item => item.id)).toEqual(["alpha"]);
		expect(palette.selectedItem()?.id).toBe("alpha");
		const rendered = palette.render(80).join("\n");
		expect(rendered).toContain("Alpha");
		expect(rendered).not.toContain("Beta");

		// Unmatched escape sequences are chords, not text: query is untouched.
		palette.handleInput("\x1b[999~");
		expect(palette.query).toBe("alp");

		// Control chars inside a pasted run are stripped.
		const scrub = new CommandPaletteComponent();
		scrub.setItems(makeItems(2));
		scrub.handleInput("a\x01b\x7f");
		expect(scrub.query).toBe("ab\x7f");
	});
});
