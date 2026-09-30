import { describe, expect, it } from "bun:test";
import { mmrRerank } from "@harvest/pi-mnemopi/core/mmr";

/**
 * MMR selection contract: multi-result recall must return a bounded,
 * score-led ranking whether the native batch kernel or the TypeScript
 * fallback produced it (U14 — the loader serves a null proxy when the
 * addon is unavailable, which previously threw instead of falling back).
 */
describe("mmrRerank selection contract", () => {
	const results = [
		{ content: "alpha beta gamma", score: 0.9 },
		{ content: "alpha delta epsilon", score: 0.7 },
		{ content: "zeta eta theta", score: 0.5 },
	];

	it("returns the score leader first within the requested limit", () => {
		const ranked = mmrRerank(results, 0.7, 2, () => 0);
		expect(ranked.length).toBe(2);
		expect(ranked[0]?.content).toBe("alpha beta gamma");
	});

	it("returns every input when the limit covers all results", () => {
		const ranked = mmrRerank(results, 0.7, 10, () => 0);
		expect(ranked.length).toBe(3);
		expect(ranked[0]?.content).toBe("alpha beta gamma");
	});

	it("returns bounded output on the default similarity path", () => {
		const ranked = mmrRerank(results, 0.7, 2);
		expect(ranked.length).toBeLessThanOrEqual(2);
		expect(ranked.length).toBeGreaterThan(0);
		expect(ranked[0]?.content).toBe("alpha beta gamma");
		for (const item of ranked) {
			expect(results.some(candidate => candidate.content === item.content)).toBe(true);
		}
	});

	it("returns empty for a zero limit and passes single results through", () => {
		expect(mmrRerank(results, 0.7, 0)).toEqual([]);
		expect(mmrRerank([{ content: "solo", score: 0.4 }], 0.7, 5)).toEqual([{ content: "solo", score: 0.4 }]);
	});
});
