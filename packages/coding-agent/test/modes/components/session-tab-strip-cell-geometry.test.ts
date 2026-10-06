import { beforeAll, describe, expect, it } from "bun:test";
import { stripVTControlCharacters } from "node:util";
import { visibleWidth } from "@harvest/pi-tui";
import { SessionTabStrip } from "../../../src/modes/components/session-tab-strip";
import { initTheme } from "../../../src/modes/theme/theme";
import { SessionTabs } from "../../../src/session/session-tabs";

beforeAll(async () => {
	await initTheme(false);
});

const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });

/** Terminal-cell columns for a stripped row (test-side reference, same width engine as production). */
function cellsOf(plain: string): string[] {
	const cells: string[] = [];
	for (const { segment } of segmenter.segment(plain)) {
		const width = Math.max(0, Bun.stringWidth(segment));
		for (let i = 0; i < width; i++) cells.push(segment);
	}
	return cells;
}

function makeStrip(titles: [string, string, string, string]) {
	const tabs = new SessionTabs();
	tabs.open("/work/first.jsonl", titles[0]);
	tabs.open("/work/second.jsonl", titles[1]);
	let selected = "/work/first.jsonl";
	const closed: string[] = [];
	const strip = new SessionTabStrip(
		tabs,
		() => selected,
		() => titles[2],
		async path => {
			selected = path;
		},
	);
	strip.setOnClose(path => {
		closed.push(path);
	});
	return { tabs, strip, closed, getSelected: () => selected, liveTitle: titles[3] };
}

function renderPlain(strip: SessionTabStrip, width: number): { rows: readonly string[]; plains: string[] } {
	const rows = strip.renderWorkspace(width, true);
	const plains = rows.map(line => stripVTControlCharacters(line));
	return { rows, plains };
}

/** All close cells in cell coordinates: list of {row,col,path}. */
function collectCloseCells(strip: SessionTabStrip, plains: string[]): { row: number; col: number; path: string }[] {
	const out: { row: number; col: number; path: string }[] = [];
	plains.forEach((plain, row) => {
		const total = cellsOf(plain).length;
		for (let col = 0; col < total; col++) {
			const path = strip.closeTargetAt(row, col);
			if (path) out.push({ row, col, path });
		}
	});
	return out;
}

describe("session tab strip cell geometry", () => {
	it("keeps exact × cells for combining-mark titles with no neighbor mis-hits", () => {
		// Failure mode: UTF-16 indexing puts × one column off when a title
		// contains a combining mark, so clicking × hits the neighbor tab.
		const combining = "e\u0301lan task";
		const { strip } = makeStrip([combining, "Second task", combining, combining]);
		const { plains } = renderPlain(strip, 100);
		expect(plains.length).toBeGreaterThan(0);
		const cells = collectCloseCells(strip, plains);
		// Two closeable tabs → exactly two × cells.
		expect(cells).toHaveLength(2);
		const byPath = new Map(cells.map(c => [c.path, c]));
		expect(byPath.has("/work/first.jsonl")).toBe(true);
		expect(byPath.has("/work/second.jsonl")).toBe(true);
		// Each × cell really renders 'x' flanked by spaces in cell geometry.
		for (const cell of cells) {
			const rowCells = cellsOf(plains[cell.row]!);
			expect(rowCells[cell.col]).toBe("x");
			expect(rowCells[cell.col - 1]).toBe(" ");
			expect(rowCells[cell.col + 1]).toBe(" ");
		}
		// Title cells are never close targets (no mis-hit into the neighbor).
		plains.forEach((plain, row) => {
			const rowCells = cellsOf(plain);
			const titleCell = rowCells.findIndex((g, idx) => {
				// First cell of the combining grapheme "e\u0301" in the first tab.
				if (g !== "e\u0301") return false;
				return strip.closeTargetAt(row, idx) === undefined;
			});
			expect(titleCell).toBeGreaterThan(-1);
		});
		// Clicking each × closes exactly that tab.
		const first = byPath.get("/work/first.jsonl")!;
		const second = byPath.get("/work/second.jsonl")!;
		expect(strip.clickWorkspace(first.row, first.col)).toBe(true);
		expect(strip.clickWorkspace(second.row, second.col)).toBe(true);
	});

	it("keeps exact × cells for emoji and wide titles", () => {
		const emoji = "Fix 🎉 party";
		const wide = "日本語タスク";
		const { strip } = makeStrip([emoji, wide, emoji, emoji]);
		const { plains } = renderPlain(strip, 120);
		const cells = collectCloseCells(strip, plains);
		expect(cells).toHaveLength(2);
		const paths = cells.map(c => c.path).sort();
		expect(paths).toEqual(["/work/first.jsonl", "/work/second.jsonl"]);
		for (const cell of cells) {
			const rowCells = cellsOf(plains[cell.row]!);
			expect(rowCells[cell.col]).toBe("x");
			// Wide/emoji titles budget 2 cells per glyph: the × itself stays 1 cell.
			expect(Bun.stringWidth(rowCells[cell.col]!)).toBe(1);
		}
		// No title cell mis-hits: every non-× cell in either tab zone is not a close target,
		// except the verified × cells above.
		const closeSet = new Set(cells.map(c => `${c.row}:${c.col}`));
		plains.forEach((plain, row) => {
			const total = cellsOf(plain).length;
			for (let col = 0; col < total; col++) {
				const target = strip.closeTargetAt(row, col);
				if (target) expect(closeSet.has(`${row}:${col}`)).toBe(true);
			}
		});
	});

	it("ANSI styles in titles do not shift × cells", () => {
		const ansi = "\x1b[31mRed task\x1b[0m";
		const { strip } = makeStrip([ansi, "Second task", "Red task", ansi]);
		const { plains } = renderPlain(strip, 100);
		// Sanitized title renders without escape bytes.
		expect(plains.join("\n")).toContain("Red task");
		expect(plains.join("\n")).not.toContain("\x1b[31m");
		const cells = collectCloseCells(strip, plains);
		expect(cells).toHaveLength(2);
		for (const cell of cells) {
			expect(cellsOf(plains[cell.row]!)[cell.col]).toBe("x");
		}
		// Visible width matches cell count (ANSI adds zero columns).
		for (const plain of plains) {
			expect(cellsOf(plain).length).toBe(visibleWidth(plain));
		}
	});

	it("clipped narrow widths hide × instead of mis-hitting a neighbor", () => {
		const tabs = new SessionTabs();
		for (let i = 1; i <= 8; i++) tabs.open(`/work/${i}.jsonl`, `Task number ${i} with a long title`);
		let selected = "/work/1.jsonl";
		const closed: string[] = [];
		const strip = new SessionTabStrip(
			tabs,
			() => selected,
			() => "Task",
			async path => {
				selected = path;
			},
		);
		strip.setOnClose(path => {
			closed.push(path);
		});
		const { plains } = renderPlain(strip, 28);
		const cells = collectCloseCells(strip, plains);
		// Collapsed/truncated tabs expose no close target; survivors (if any)
		// still point at their own tab.
		for (const cell of cells) {
			const rowCells = cellsOf(plains[cell.row]!);
			expect(rowCells[cell.col]).toBe("x");
			expect(tabs.paths).toContain(cell.path);
		}
		// Clicking a non-close title cell never closes a different tab.
		const before = closed.length;
		plains.forEach((plain, row) => {
			const rowCells = cellsOf(plain);
			for (let col = 0; col < rowCells.length; col++) {
				if (strip.closeTargetAt(row, col)) continue;
				strip.clickWorkspace(row, col);
			}
		});
		// Only close-cell clicks push to closed; title clicks select instead.
		expect(closed.length).toBe(before);
	});
});
