import { expect, test } from "bun:test";
import { chunkTerminalOutput, coalesceTerminalSgr } from "@harvest/pi-natives";

test("native SGR transformation preserves embedded NUL and unpaired UTF-16 units", () => {
	const input = "\ud800\0\x1b[1m\x1b[31mtext\udfff\0";
	expect(coalesceTerminalSgr(input)).toBe("\ud800\0\x1b[1;31mtext\udfff\0");
});

test("native ConPTY chunks preserve exact UTF-16 across byte boundaries", () => {
	const input = "a\0\ud800😀\udfff\n字\0tail";
	const chunks = chunkTerminalOutput(input, 4);
	expect(chunks.join("")).toBe(input);
	for (const chunk of chunks) expect(Buffer.byteLength(chunk)).toBeLessThanOrEqual(4);
});
