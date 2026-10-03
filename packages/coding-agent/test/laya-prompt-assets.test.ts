import { describe, expect, it } from "bun:test";
import { MODEL_ROUTING_CRITERIA } from "../src/core/harvest/laya-routing";
import { RELEVANCE_CRITERIA, renderRelevanceInstructions } from "../src/core/harvest/laya-pruning";

/**
 * P2-5: Laya decision wording lives in versioned `.md` prompt assets, not
 * inline strings. These tests pin the structural contracts consumers depend
 * on (tier coverage, level counts that bound score ranges, template
 * substitution) without pinning prose.
 */
describe("Laya versioned prompt assets", () => {
	it("covers every model tier with non-empty choice descriptions", () => {
		expect(Object.keys(MODEL_ROUTING_CRITERIA).sort()).toEqual(["default", "slow", "smol"]);
		for (const description of Object.values(MODEL_ROUTING_CRITERIA)) {
			expect(description.trim().length).toBeGreaterThan(0);
		}
	});

	it("keeps four relevance levels so score normalization stays bounded", () => {
		expect(RELEVANCE_CRITERIA).toHaveLength(4);
		for (const level of RELEVANCE_CRITERIA) {
			expect(level.trim().length).toBeGreaterThan(0);
		}
	});

	it("substitutes the chunk label into the relevance question without residue", () => {
		const rendered = renderRelevanceInstructions("Tool 'bash' result");
		expect(rendered).toContain("Tool 'bash' result");
		expect(rendered).not.toContain("{{");
		expect(rendered).not.toContain("}}");
	});
});
