import { describe, expect, it } from "bun:test";
import { MODEL_ROUTING_CRITERIA } from "../src/core/harvest/laya-routing";
import { RELEVANCE_CRITERIA, renderRelevanceInstructions } from "../src/core/harvest/laya-pruning";
import {
	CALIBRATION_PRUNING_CRITERIA,
	CALIBRATION_SANITY_CHECK_INSTRUCTIONS,
	SUBAGENT_CRITERIA,
	SUBAGENT_SELECTION_INSTRUCTIONS,
	parseSubagentCriteria,
	renderCalibrationRelevanceInstructions,
} from "../src/core/harvest/laya-prompt-assets";
import { BENCHMARK_PAYLOADS } from "../src/core/harvest/laya-calibration";
import { BUILTIN_AGENT_CRITERIA } from "../src/core/harvest/laya-subagent-selection";

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

	it("covers every built-in subagent with distinct non-empty choice descriptions", () => {
		expect(Object.keys(SUBAGENT_CRITERIA).sort()).toEqual([
			"reviewer",
			"scout",
			"security-reviewer",
			"sonic",
			"task",
		]);
		const descriptions = Object.values(SUBAGENT_CRITERIA);
		for (const description of descriptions) {
			expect(description.trim().length).toBeGreaterThan(0);
		}
		expect(new Set(descriptions).size).toBe(descriptions.length);
		// Production roster resolution consumes the same versioned criteria.
		expect(BUILTIN_AGENT_CRITERIA).toEqual(SUBAGENT_CRITERIA);
	});

	it("keeps subagent selection instructions versioned and non-empty", () => {
		expect(SUBAGENT_SELECTION_INSTRUCTIONS.trim().length).toBeGreaterThan(0);
		expect(SUBAGENT_SELECTION_INSTRUCTIONS).not.toContain("{{");
	});

	it("rejects malformed subagent criteria assets instead of sending degraded questions", () => {
		expect(() => parseSubagentCriteria("scout without a colon")).toThrow();
		expect(() => parseSubagentCriteria("scout: \nreviewer: x")).toThrow();
		expect(() => parseSubagentCriteria("unknown-agent: does things")).toThrow();
		expect(() =>
			parseSubagentCriteria("scout: a\nreviewer: b\nsecurity-reviewer: c\nsonic: d\ntask: e\nscout: dup"),
		).toThrow();
		expect(() => parseSubagentCriteria("scout: only one agent")).toThrow();
	});

	it("benchmarks the exact production wording so calibration measures what ships", () => {
		const choice = BENCHMARK_PAYLOADS.singleChoice.questions.subagent_choice;
		expect(choice.instructions).toBe(SUBAGENT_SELECTION_INSTRUCTIONS);
		expect(choice.criteria).toEqual(SUBAGENT_CRITERIA);

		const single = BENCHMARK_PAYLOADS.singleScore.questions.score_relevance;
		expect(single.instructions).toBe(renderCalibrationRelevanceInstructions("chunk"));
		expect(single.criteria).toEqual([...CALIBRATION_PRUNING_CRITERIA]);

		const batch = BENCHMARK_PAYLOADS.batchedScore.questions;
		expect(batch.chunk_1.instructions).toBe(renderCalibrationRelevanceInstructions("Tool 'bash' result"));
		expect(batch.chunk_2.instructions).toBe(renderCalibrationRelevanceInstructions("Tool 'read_file' result"));
		expect(batch.chunk_1.criteria).toEqual([...RELEVANCE_CRITERIA]);
		expect(CALIBRATION_PRUNING_CRITERIA).toEqual([...RELEVANCE_CRITERIA]);

		expect(BENCHMARK_PAYLOADS.ultraShort.questions.status_check.instructions).toBe(
			CALIBRATION_SANITY_CHECK_INSTRUCTIONS,
		);
		expect(CALIBRATION_SANITY_CHECK_INSTRUCTIONS.trim().length).toBeGreaterThan(0);
	});
});
