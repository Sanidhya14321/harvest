import assert from "node:assert";

export const describe = (name, fn) => {
	console.log(`\n--- ${name} ---`);
	fn();
};

export const it = (name, fn) => {
	try {
		const res = fn();
		if (res && typeof res.then === "function") {
			return res.then(
				() => console.log(`  ✓ ${name}`),
				(err) => {
					console.error(`  ✗ ${name}`);
					console.error(err);
					process.exitCode = 1;
				},
			);
		}
		console.log(`  ✓ ${name}`);
	} catch (err) {
		console.error(`  ✗ ${name}`);
		console.error(err);
		process.exitCode = 1;
	}
};

export const test = it;

export function expect(actual) {
	return {
		toBe(expected) {
			assert.strictEqual(actual, expected);
		},
		toEqual(expected) {
			assert.deepStrictEqual(actual, expected);
		},
		toMatchObject(expected) {
			for (const k of Object.keys(expected)) {
				assert.deepStrictEqual(actual[k], expected[k]);
			}
		},
		toBeUndefined() {
			assert.strictEqual(actual, undefined);
		},
		toBeNull() {
			assert.strictEqual(actual, null);
		},
		toBeDefined() {
			assert.notStrictEqual(actual, undefined);
		},
		toBeTruthy() {
			assert.ok(actual);
		},
		toBeFalsy() {
			assert.ok(!actual);
		},
		toContain(expected) {
			assert.ok(actual.includes(expected));
		},
		toMatch(pattern) {
			if (pattern instanceof RegExp) {
				assert.ok(pattern.test(String(actual)));
			} else {
				assert.ok(String(actual).includes(String(pattern)));
			}
		},
		toBeGreaterThan(expected) {
			assert.ok(actual > expected);
		},
		toBeGreaterThanOrEqual(expected) {
			assert.ok(actual >= expected);
		},
		toBeLessThan(expected) {
			assert.ok(actual < expected);
		},
		toBeLessThanOrEqual(expected) {
			assert.ok(actual <= expected);
		},
		toThrow(expected) {
			let threw = false;
			let error;
			try {
				if (typeof actual === "function") {
					actual();
				}
			} catch (e) {
				threw = true;
				error = e;
			}
			if (!threw) {
				assert.fail("Expected function to throw, but it did not throw.");
			}
			if (expected) {
				if (typeof expected === "string") {
					assert.ok(String(error?.message || error).includes(expected));
				} else if (expected instanceof RegExp) {
					assert.ok(expected.test(String(error?.message || error)));
				}
			}
		},
		get not() {
			return {
				toBe(expected) {
					assert.notStrictEqual(actual, expected);
				},
				toEqual(expected) {
					assert.notDeepStrictEqual(actual, expected);
				},
				toContain(expected) {
					assert.ok(!actual.includes(expected));
				},
				toThrow() {
					let threw = false;
					let error;
					try {
						if (typeof actual === "function") {
							actual();
						}
					} catch (e) {
						threw = true;
						error = e;
					}
					if (threw) {
						assert.fail(`Expected function not to throw, but it threw: ${error}`);
					}
				},
			};
		},
	};
}

export const beforeEach = (fn) => {};
export const afterEach = (fn) => {};
export const beforeAll = (fn) => {};
export const afterAll = (fn) => {};
