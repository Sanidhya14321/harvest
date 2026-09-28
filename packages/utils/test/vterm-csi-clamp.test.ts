import { describe, expect, test } from "bun:test";
import { Terminal } from "../src/vterm";

/**
 * CSI repeat-count DoS guard (CODE_REVIEW_3 P2): the CSI sequence itself is
 * length-capped, but the repeat count was not — `ESC[99999999S` looped
 * `scrollUp` a hundred million times and `ESC[10000000@` built a ten-million
 * entry blank array. Counts now clamp to `rows*4` (scroll) / `cols*4`
 * (cells), matching the existing insert/delete-line clamp pattern.
 */
describe("vterm CSI repeat-count clamp", () => {
	test("ESC[99999999S completes quickly with scrollback-bounded growth", () => {
		const terminal = new Terminal({ cols: 80, rows: 24, scrollback: 100 });
		const start = performance.now();
		terminal.write("\x1b[99999999S");
		const elapsed = performance.now() - start;
		expect(elapsed).toBeLessThan(3000);
		// Effect saturates: the viewport scrolled, but never past scrollback.
		expect(terminal.buffer.active.baseY).toBeGreaterThan(0);
		expect(terminal.buffer.active.baseY).toBeLessThanOrEqual(100);
		expect(terminal.buffer.active.length).toBeLessThanOrEqual(24 + 100);
	});

	test("ESC[99999999T completes quickly", () => {
		const terminal = new Terminal({ cols: 80, rows: 24, scrollback: 100 });
		const start = performance.now();
		terminal.write("\x1b[99999999T");
		expect(performance.now() - start).toBeLessThan(3000);
		expect(terminal.buffer.active.length).toBeLessThanOrEqual(24 + 100);
	});

	test("ESC[10000000@ completes quickly with cols-bounded insertion", () => {
		const terminal = new Terminal({ cols: 80, rows: 24 });
		terminal.write("hello");
		const start = performance.now();
		terminal.write("\x1b[10000000@");
		expect(performance.now() - start).toBeLessThan(3000);
		const line = terminal.buffer.active.getLine(0);
		expect(line?.cells.length).toBe(80);
		expect(line?.translateToString(true)).toBe("hello");
	});

	test("ESC[10000000P completes quickly and clears only the visible line", () => {
		const terminal = new Terminal({ cols: 80, rows: 24 });
		terminal.write("hello\r\x1b[10000000P");
		const line = terminal.buffer.active.getLine(0);
		expect(line?.cells.length).toBe(80);
		expect(line?.translateToString(true)).toBe("");
	});

	test("ESC[10000000X completes quickly and erases only the visible line", () => {
		const terminal = new Terminal({ cols: 80, rows: 24 });
		terminal.write("hello\r\x1b[10000000X");
		const line = terminal.buffer.active.getLine(0);
		expect(line?.cells.length).toBe(80);
		expect(line?.translateToString(true)).toBe("");
	});
});
