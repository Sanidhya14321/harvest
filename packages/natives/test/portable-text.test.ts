import { describe, expect, it } from "bun:test";
import fixtures from "../../../crates/harvest-terminal/tests/fixtures/text-bindings.json";
import * as portable from "../native/terminal.js";

interface TextFixture {
	method: string;
	args: unknown[];
	expected: unknown;
}
const methods: Record<string, (...args: never[]) => unknown> = {
	visibleWidth: portable.visibleWidth,
	wrapTextWithAnsi: portable.wrapTextWithAnsi,
	truncateToWidth: portable.truncateToWidth,
	sliceWithWidth: portable.sliceWithWidth,
	extractSegments: portable.extractSegments,
};

describe("Rust/WASM terminal text contracts", () => {
	for (const [index, fixture] of (fixtures as TextFixture[]).entries()) {
		it(`${fixture.method} preserves native transformation fixture ${index + 1}`, () => {
			const method = methods[fixture.method];
			if (!method) throw new Error(`Unknown text fixture operation: ${fixture.method}`);
			expect(method(...(fixture.args as never[]))).toEqual(fixture.expected);
		});
	}
	it("closes a clipped hyperlink before the ellipsis so following UI remains unlinked", () => {
		const open = "\x1b]8;id=join;https://example.com/session\x1b\\";
		expect(portable.truncateToWidth(`${open}Join this session\x1b]8;;\x1b\\`, 7, 0, false, 3)).toBe(
			`${open}Join t\x1b]8;;\x1b\\…`,
		);
	});
	it("rejects oversized dimensions without trapping or breaking later operations", () => {
		expect(() => portable.truncateToWidth("x", 2 ** 30, 0, true, 3)).toThrow("bound");
		expect(portable.truncateToWidth("界🙂tail", 4, 0, false, 3)).toBe("界…");
	});
});
