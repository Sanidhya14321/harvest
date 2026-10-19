import { describe, expect, it } from "bun:test";
import * as natives from "@harvest/pi-natives";
import { CELL_U32, type Cell, decodeCell } from "kitty-vt-wasm";
import { VirtualTerminal } from "./virtual-terminal";
import {
	analyzeBgFillLine,
	DECSACE_DEFAULT,
	DECSACE_RECT,
	detectRectangularSgrSupport,
	encodeDeccara,
	planDeccaraFills,
} from "@harvest/pi-tui";

// Truecolor background open token used throughout the integration tests.
const BG_OPEN = "\x1b[48;2;10;20;30m";
const BG_SGR = "48;2;10;20;30";

describe("detectRectangularSgrSupport", () => {
	it("enables only kitty, which implements the SGR-background extension", () => {
		expect(detectRectangularSgrSupport("kitty", {})).toBe(true);
		// Ghostty leaves CSI $r unimplemented (ghostty-org/ghostty#632) — excluded.
		expect(detectRectangularSgrSupport("ghostty", {})).toBe(false);
		expect(detectRectangularSgrSupport("wezterm", {})).toBe(false);
		expect(detectRectangularSgrSupport("iterm2", {})).toBe(false);
		expect(detectRectangularSgrSupport("alacritty", {})).toBe(false);
		expect(detectRectangularSgrSupport("base", {})).toBe(false);
		expect(detectRectangularSgrSupport("trueColor", {})).toBe(false);
	});

	it("honors the PI_NO_DECCARA kill switch (truthy values only)", () => {
		expect(detectRectangularSgrSupport("kitty", { PI_NO_DECCARA: "1" })).toBe(false);
		expect(detectRectangularSgrSupport("kitty", { PI_NO_DECCARA: "true" })).toBe(false);
		// A falsey assignment is not a kill: support stays on.
		expect(detectRectangularSgrSupport("kitty", { PI_NO_DECCARA: "0" })).toBe(true);
		expect(detectRectangularSgrSupport("kitty", { PI_NO_DECCARA: "false" })).toBe(true);
	});

	it("disables under tmux/screen/zellij/cmux multiplexers", () => {
		expect(detectRectangularSgrSupport("kitty", { TMUX: "/tmp/tmux-1000/default,123,0" })).toBe(false);
		expect(detectRectangularSgrSupport("kitty", { STY: "1234.pts-0" })).toBe(false);
		expect(detectRectangularSgrSupport("kitty", { ZELLIJ: "0" })).toBe(false);
		expect(detectRectangularSgrSupport("kitty", { CMUX_SURFACE_ID: "surface" })).toBe(false);
		expect(detectRectangularSgrSupport("kitty", { TERM: "tmux-256color" })).toBe(false);
		expect(detectRectangularSgrSupport("kitty", { TERM: "screen.xterm" })).toBe(false);
	});
});

function paintRows(terminal: VirtualTerminal, texts: string[], sequence = ""): void {
	for (const [index, text] of texts.entries()) {
		terminal.write(`\x1b[${index + 2};1H\x1b[0m\x1b[2K${text}\x1b[0m`);
	}
	terminal.write(sequence);
}

function visualCells(terminal: VirtualTerminal): Cell[][] {
	return terminal.getViewportCellRows().map(row => {
		const words = Uint32Array.from(row);
		return Array.from({ length: row.length / CELL_U32 }, (_, column) => {
			const cell = decodeCell(words, column * CELL_U32);
			// Written spaces and erased blank cells render alike. Ignore invisible
			// foreground/font attributes only when no decoration can draw on them.
			if (cell.ch === "" || cell.ch === " ") {
				cell.ch = " ";
				if (!cell.reverse && cell.underline === 0 && !cell.strikethrough) {
					cell.fg = null;
					cell.bold = false;
					cell.dim = false;
					cell.italic = false;
					cell.blink = false;
				}
			}
			return cell;
		});
	});
}

describe("native rectangular-fill host boundary", () => {
	it("preserves the rendered Kitty cells, including inverse and underlined padding", () => {
		const lines = [
			`${BG_OPEN}a\ud800${" ".repeat(38)}\x1b[0m`,
			`${BG_OPEN}🙂${" ".repeat(38)}\x1b[0m`,
			`${BG_OPEN}界${" ".repeat(38)}\x1b[0m`,
			"plain row",
			`\x1b[7;41mreverse${" ".repeat(33)}\x1b[0m`,
			`\x1b[4;44munder${" ".repeat(35)}\x1b[0m`,
		];
		const plan = planDeccaraFills(lines, 40, 1);
		expect(plan.sequence).toBe(`${DECSACE_RECT}${encodeDeccara(2, 3, 4, 40, BG_SGR)}${DECSACE_DEFAULT}`);
		const reference = new VirtualTerminal(40, 8);
		const optimized = new VirtualTerminal(40, 8);
		paintRows(reference, lines);
		paintRows(optimized, plan.texts, plan.sequence);
		expect(optimized.getViewport()).toEqual(reference.getViewport());
		expect(visualCells(optimized)).toEqual(visualCells(reference));
	});

	it("applies UTF-16 cutoffs to the same row snapshot used by the native planner", () => {
		const prefix = `${BG_OPEN}a\ud800`;
		const rows = [`${prefix}${" ".repeat(78)}\x1b[0m`];
		let reads = 0;
		Object.defineProperty(rows, 0, {
			get: () => (reads++ === 0 ? `${prefix}${" ".repeat(78)}\x1b[0m` : "different text"),
		});
		const plan = planDeccaraFills(rows, 80);
		expect(plan.texts).toEqual([`${prefix}\x1b[0m`]);
		expect(plan.sequence).toBe(`${DECSACE_RECT}${encodeDeccara(1, 3, 1, 80, BG_SGR)}${DECSACE_DEFAULT}`);
	});

	it("keeps complete original rows when the optional optimizer rejects invalid geometry", () => {
		const lines = [`${BG_OPEN}${" ".repeat(40)}\x1b[0m`];
		expect(() => natives.planDeccaraCuts(lines, Number.NaN)).toThrow(/whole cells/);
		expect(planDeccaraFills(lines, Number.NaN)).toEqual({ texts: lines, sequence: "" });
		expect(analyzeBgFillLine(lines[0], Number.NaN)).toBeNull();
	});

	it("rejects JavaScript coordinates that would be silently coerced by a u32 bridge", () => {
		for (const coordinate of [Number.NaN, -1, 0.5, 4_294_967_296]) {
			expect(() => encodeDeccara(coordinate, 1, 2, 40, "41")).toThrow(/whole cells/);
		}
	});
});
