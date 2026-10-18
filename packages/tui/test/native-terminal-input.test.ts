import { expect, test } from "bun:test";
import { TerminalInput } from "@harvest/pi-natives";
import * as inputBaseline from "../src/input-compat";
import * as printableBaseline from "../src/keys";

const terminal = new TerminalInput();

function scan(data: string, hint: number) {
	const result = terminal.scan(data, hint);
	const sequences: string[] = [];
	for (let index = 0; index < result.ranges.length; index += 2) {
		sequences.push(data.slice(result.ranges[index], result.ranges[index + 1]));
	}
	return {
		sequences,
		remainder: data.slice(result.remainderStart),
		resumeSearchFrom: result.resumeSearchFrom,
		...(result.discardFrom == null ? {} : { discardFrom: result.discardFrom }),
	};
}

test("native input preserves fragmented protocol events and original UTF-16 across the host boundary", () => {
	const wire =
		"a\0\ud800😀\udfff\x1b[<0;20;5M\x1b]52;c;payload\x1b\\\x1bPq\x1b\\\x1b_Gr=1;\x1b\\\x1b\x1b[A\x1b\x1b[<35;8;9m\x1b[M abc";
	for (let split = 0; split <= wire.length; split++) {
		const head = wire.slice(0, split);
		const baseline = inputBaseline.extractCompleteSequences(head, 0);
		const native = scan(head, 0);
		expect(native).toEqual(baseline);
		const tail = baseline.remainder + wire.slice(split);
		expect(scan(tail, baseline.resumeSearchFrom)).toEqual(
			inputBaseline.extractCompleteSequences(tail, baseline.resumeSearchFrom),
		);
	}
});

test("native torn CSI bounds and resumed string searches preserve the host discard contract", () => {
	const csi = "\x1b[" + "1".repeat(4100);
	expect(scan(csi, 0)).toEqual(inputBaseline.extractCompleteSequences(csi, 0));
	for (const prefix of ["\x1b]52;c;", "\x1bPq", "\x1b_Gr=1;"]) {
		const head = prefix + "x".repeat(65536) + "\x1b";
		const baseline = inputBaseline.extractCompleteSequences(head, 0);
		expect(scan(head, 0)).toEqual(baseline);
		const tail = head + "\\z";
		expect(scan(tail, baseline.resumeSearchFrom)).toEqual(
			inputBaseline.extractCompleteSequences(tail, baseline.resumeSearchFrom),
		);
	}
});

test("native printable decoding matches retained text, shortcut and malformed numeric contracts", () => {
	const modifiers = ["1", "2", "3", "5", "9", "65", "129", "193", "257", "0", "4294967297", "9".repeat(500)];
	const codes = [0, 31, 32, 97, 127, 128, 55296, 57344, 57400, 57413, 63743, 1114111, 1114112];
	for (const modifier of modifiers) {
		for (const code of codes) {
			for (const sequence of [
				`\x1b[${code};${modifier}u`,
				`\x1b[${code}:65;${modifier}:3u`,
				`\x1b[${code};${modifier};128512:769u`,
				`\x1b[27;${modifier};${code}~`,
			]) {
				expect(terminal.decodePrintableKey(sequence) ?? undefined).toBe(
					printableBaseline.decodePrintableKey(sequence),
				);
				const keypadText =
					(code === 57400 || code === 57413) && sequence.endsWith("u")
						? printableBaseline.decodePrintableKey(sequence)
						: undefined;
				expect(terminal.decodeKeypadText(sequence) ?? undefined).toBe(keypadText);
			}
		}
	}
	for (const text of ["a\0b", "\ud800", "😀\udfff", "a\x85", "字", "\x1b[0;1;0:127:97u", "\x1b[97::99;2u"]) {
		expect(terminal.extractPrintableText(text) ?? undefined).toBe(printableBaseline.extractPrintableText(text));
	}
});
