import { describe, expect, it } from "bun:test";
import { Ellipsis, type SliceResult } from "../native/index.js";
import { createFallbackBindings } from "../native/loader-state.js";

interface FallbackTextBindings {
	truncateToWidth(text: string, width: number, ellipsis: Ellipsis, pad: boolean, tabWidth: number): string;
	sliceWithWidth(text: string, start: number, length: number, strict: boolean, tabWidth: number): SliceResult;
}

const textBindings = createFallbackBindings() as unknown as FallbackTextBindings;

describe("JavaScript terminal text fallback", () => {
	it("keeps an ST-terminated hyperlink intact when truncating its visible label", () => {
		const open = "\x1b]8;id=join;https://example.com/session\x1b\\";
		const close = "\x1b]8;;\x1b\\";
		const result = textBindings.truncateToWidth(`${open}Join this session${close}`, 7, Ellipsis.Unicode, false, 3);

		expect(result).toBe(`${open}Join t${close}…`);
		expect(Bun.stringWidth(result, { countAnsiEscapeCodes: false })).toBe(7);
	});

	it("preserves a BEL-terminated hyperlink while slicing visible columns", () => {
		const open = "\x1b]8;;https://example.com/session\x07";
		const close = "\x1b]8;;\x07";
		const result = textBindings.sliceWithWidth(`${open}abcdef${close}`, 2, 2, true, 3);

		expect(Bun.stripANSI(result.text)).toBe("cd");
		expect(result.text).toContain(open);
		expect(result.width).toBe(2);
	});

	it("keeps a Kitty graphics command atomic instead of displaying its payload", () => {
		const graphic = "\x1b_Ga=T,f=100;YWJjZA==\x1b\\";
		const result = textBindings.truncateToWidth(`${graphic}abcdef`, 4, Ellipsis.Omit, false, 3);

		expect(result).toBe(`${graphic}abcd`);
		expect(Bun.stringWidth(result, { countAnsiEscapeCodes: false })).toBe(4);
	});

	it("keeps a BEL-terminated cursor marker intact under clipping", () => {
		const marker = "\x1b_pi:c\x07";
		const result = textBindings.truncateToWidth(`ab${marker}cdef`, 4, Ellipsis.Omit, false, 3);

		expect(result).toBe(`ab${marker}cd`);
	});

	it("degrades a clipped scaled-text span to readable plain text", () => {
		const scaled = "\x1b]66;s=2;Hi\x1b\\";
		const result = textBindings.truncateToWidth(scaled, 3, Ellipsis.Unicode, false, 3);

		expect(result).toBe("Hi…");
	});

	it("slices scaled emoji by visible cells without splitting its surrogate pair", () => {
		const scaled = "\x1b]66;s=2;🙂x\x1b\\";
		const result = textBindings.sliceWithWidth(scaled, 4, 2, true, 3);

		expect(result).toEqual({ text: "x", width: 1 });
	});

	it("honors a custom tab width when deciding whether text needs clipping", () => {
		const result = textBindings.truncateToWidth("a\tb", 4, Ellipsis.Omit, true, 4);

		expect(result).toBe("a   ");
	});

	it("bounds ASCII ellipses and padding in a one-cell allocation", () => {
		expect(textBindings.truncateToWidth("abcdef", 1, Ellipsis.Ascii, true, 3)).toBe(".");
		expect(textBindings.truncateToWidth("abcdef", 0, Ellipsis.Ascii, true, 3)).toBe("");
	});

	it("truncates large Unicode output without breaking an emoji grapheme", () => {
		const family = "👨‍👩‍👧‍👦";
		const result = textBindings.truncateToWidth(`${family}界`.repeat(100_000), 8, Ellipsis.Unicode, true, 3);

		expect(Bun.stripANSI(result)).toBe(`${family}界${family}… `);
		expect(Bun.stringWidth(result, { countAnsiEscapeCodes: false })).toBe(8);
	});
});
