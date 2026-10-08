import { describe, expect, it } from "bun:test";
import { OmpErrors, type } from "../src";

/** Call count that guarantees the JIT has kicked in (threshold is 3). */
const JIT = 6;

/**
 * Optional-prop presence is an own-property check (CODE_REVIEW_3 P3): `in`
 * also matches the prototype chain, so `{}` "had" `toString`/`constructor`
 * and inherited junk properties were validated as if declared.
 */
describe("optional-prop own-property presence", () => {
	it("treats Object.prototype members as absent for optional props (interp + JIT)", () => {
		const schema = type({ "toString?": "string", "constructor?": "number" });
		for (let i = 0; i < JIT; i++) {
			// Untyped: TS carries Object.prototype's `constructor: Function`
			// on every object type, so `{}` can never satisfy a schema type
			// that narrows those props. The contract here is runtime
			// own-property presence, not assignability.
			expect(schema({}) as unknown).toEqual({});
			expect(schema.allows({})).toBe(true);
		}
	});

	it("treats prototype-inherited values as absent for optional props (interp + JIT)", () => {
		const schema = type({ name: "string", "age?": "number" });
		const input = Object.assign(Object.create({ age: "not-a-number" }), { name: "ada" });
		for (let i = 0; i < JIT; i++) {
			// Morph-free schemas return the input as-is once valid.
			expect(schema(input)).toBe(input);
			expect(schema.allows(input)).toBe(true);
		}
	});

	it("still validates and rejects own optional props (interp + JIT)", () => {
		const schema = type({ name: "string", "age?": "number" });
		for (let i = 0; i < JIT; i++) {
			expect(schema({ name: "ada", age: 36 })).toEqual({ name: "ada", age: 36 });
			expect(schema({ name: "ada", age: "old" })).toBeInstanceOf(OmpErrors);
		}
	});

	it("still reports missing required props without consulting the prototype (interp + JIT)", () => {
		const schema = type({ toString: "string" });
		for (let i = 0; i < JIT; i++) {
			const out = schema({});
			expect(out).toBeInstanceOf(OmpErrors);
			if (out instanceof OmpErrors) expect(out[0].path).toEqual(["toString"]);
		}
	});
});
