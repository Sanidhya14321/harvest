import { beforeAll, describe, expect, it } from "bun:test";
import { loadTheme } from "@harvest/pi-coding-agent/modes/theme/loader";
import { highlightCode, type Theme } from "@harvest/pi-coding-agent/modes/theme/theme";

const unifiedDiffChunks = [
	[
		"diff --git a/src/example.ts b/src/example.ts",
		"index 1234567..89abcde 100644",
		"--- a/src/example.ts",
		"+++ b/src/example.ts",
		"@@ -1,4 +1,5 @@",
		' import { run } from "./run";',
		"-const enabled = false;",
		"+const enabled = true;",
		"+run(enabled);",
	],
	[
		"@@ -8,4 +9,4 @@ export function start() {",
		" context();",
		'-return "old";',
		'+return "new";',
		"\\ No newline at end of file",
	],
].map(lines => lines.join("\n"));

const unifiedDiff = unifiedDiffChunks.join("\n");
const diffLanguages: Array<"diff" | "patch"> = ["diff", "patch"];

let darkTheme: Theme;
beforeAll(async () => {
	darkTheme = await loadTheme("dark", { mode: "truecolor" });
});

describe("diff highlighter chunk parity", () => {
	for (const lang of diffLanguages) {
		it(`highlights newline-complete ${lang} chunks with whole-block visual styles`, () => {
			const completeDiff = `${unifiedDiff}\n`;
			const wholeBlock = highlightCode(completeDiff, lang, darkTheme);
			const chunkedLines = unifiedDiffChunks.flatMap(chunk => {
				const highlighted = highlightCode(`${chunk}\n`, lang, darkTheme);
				return highlighted.slice(0, -1);
			});
			chunkedLines.push("");

			expect(Bun.stripANSI(wholeBlock.join("\n"))).toBe(completeDiff);
			expect(wholeBlock.join("\n")).not.toBe(completeDiff);
			expect(chunkedLines.map(line => line.replace(/^\x1b\[39m/u, ""))).toEqual(
				wholeBlock.map(line => line.replace(/^\x1b\[39m/u, "")),
			);
		});
	}
});
