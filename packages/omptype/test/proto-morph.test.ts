import { describe, expect, it } from "bun:test";
import { type } from "../src";

describe("__proto__ property in morphing object schemas", () => {
	it("preserves __proto__ as an own property without mutating the prototype in morphing schemas", () => {
		// Schema with "+" (delete undeclared keys) and a morphed field
		const Schema = type({
			["__proto__"]: "string",
			name: type("string").pipe(s => s.toUpperCase()),
			"+": "delete",
		});

		const input = JSON.parse('{"__proto__":"innocent","name":"alice","extra":"junk"}');
		const result = Schema.assert(input) as Record<string, unknown>;

		expect(Object.hasOwn(result, "__proto__")).toBe(true);
		expect(result.__proto__).toBe("innocent");
		expect(Object.getPrototypeOf(result)).toBe(Object.prototype);
		expect(result.name).toBe("ALICE");
		expect(result.extra).toBeUndefined();
	});
});
