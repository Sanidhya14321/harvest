import { expect, test } from "bun:test";
import fixtures from "../../../crates/harvest-terminal/tests/fixtures/fuzzy.json";
import { chunkForConPTY } from "../src/terminal";
import {
	canPrefixTerminalWidth,
	chunkTerminalOutput,
	coalesceTerminalSgr,
	SelectorFuzzyMatcher,
	SelectorFuzzyText,
} from "../wasm/terminal.js";

test("installed WASM preserves selector scores for phrases, Unicode, abbreviations and typos", () => {
	const matcher = new SelectorFuzzyMatcher();
	for (const fixture of fixtures) {
		expect(matcher.matches(fixture.query, fixture.text)).toEqual({ matches: fixture.matches, score: fixture.score });
		const prepared = new SelectorFuzzyText(fixture.text);
		expect(prepared.matches(fixture.query)).toEqual({ matches: fixture.matches, score: fixture.score });
		prepared.dispose();
	}
	const candidates = ["image provider", "Settings", "reimage image provider", "image provider"];
	expect(matcher.rank("image", ["\ud800image", "image\0provider", "Settings"]).map(result => result.index)).toEqual([
		0, 1,
	]);
	expect(matcher.rank("", [])).toEqual([]);
	const rank = () => matcher.rank("image provider", candidates).map(result => result.index);
	expect(rank()).toEqual([0, 3, 2]);
	matcher.clear();
	expect(rank()).toEqual([0, 3, 2]);
});

test("released WASM search handles reject use and cannot affect replacement handles", () => {
	const old = new SelectorFuzzyText("OpenCode Go");
	old.dispose();
	old.dispose();
	const replacement = new SelectorFuzzyText("Settings");
	expect(() => old.matches("og")).toThrow("released");
	expect(replacement.matches("og").matches).toBe(false);
	expect(replacement.matches("settings").matches).toBe(true);
	replacement.dispose();
});

test("portable ConPTY writes preserve byte caps, line boundaries, NUL and unmatched surrogates", () => {
	const cases = ["", "你好\n😀\nlast", "a\0\ud800😀\udfff\n字\0tail", "\x1b[31m" + "x".repeat(64)];
	for (const input of cases) {
		for (const cap of [1, 4, 8, 16, 0, -1, Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
			const chunks = chunkTerminalOutput(input, cap);
			expect(chunks.join("")).toBe(input);
			expect(chunks).toEqual(chunkForConPTY(input, cap));
		}
	}
	expect(chunkTerminalOutput("你好\n😀\nlast", 8)).toEqual(["你好\n", "😀\n", "last"]);
});

test("portable SGR merging retains incomplete colors and exact UTF-16 output", () => {
	expect(coalesceTerminalSgr("\ud800\0\x1b[1m\x1b[31mtext\udfff\0")).toBe("\ud800\0\x1b[1;31mtext\udfff\0");
	expect(coalesceTerminalSgr("\x1b[38;2;255;0m\x1b[31m\x1b[m\x1b[1mX\x1b[2K")).toBe(
		"\x1b[38;2;255;0m\x1b[31;0;1mX\x1b[2K",
	);
	expect(canPrefixTerminalWidth("\x1b[1m\u0301")).toBe(false);
	expect(canPrefixTerminalWidth("\x1b[1mA")).toBe(true);
});
