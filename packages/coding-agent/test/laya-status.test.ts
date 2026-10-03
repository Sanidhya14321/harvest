import { describe, expect, it } from "bun:test";
import { resolveLayaStatusSettings, type LayaStatusSchemaDefaults } from "../src/cli/laya-cli";
import type { DerivedLayaSettings } from "../src/core/harvest/laya-calibration";

/**
 * P1-3: `/laya status` must distinguish a key the user actually configured
 * from a hardware-calibrated recommendation and from the schema default.
 * Before the fix, `settings.get()` schema defaults were reported as
 * "(User Override)". These tests pin the precedence contract behind the
 * display: explicit wins, then calibrated, then schema default — and an
 * unset key is never labeled `user-configured`.
 */
const SCHEMA_DEFAULTS: LayaStatusSchemaDefaults = {
	pruning: true,
	subagentSelection: false,
	subagentSelectionTimeoutMs: 300,
};

const CALIBRATED: DerivedLayaSettings = {
	rawSingleChoiceLatencyMs: 45,
	subagentSelectionTimeoutMs: 300,
	subagentSelectionRecommendEnabled: true,
	subagentSelectionReason: "Fast",
	pruningRecommendEnabled: false,
	pruningReason: "Over budget",
	maxAcceptableLatencyPerTurnMs: 150,
	estimatedAddedLatencyPerTurnMs: 2100,
	worstCaseBatchLatencyMs: 22500,
};

describe("resolveLayaStatusSettings (P1-3 configured vs calibrated vs effective)", () => {
	it("reports schema defaults — not user overrides — when nothing is configured or calibrated", () => {
		const resolved = resolveLayaStatusSettings({}, undefined, SCHEMA_DEFAULTS);
		expect(resolved.pruning).toEqual({ value: true, provenance: "schema-default" });
		expect(resolved.subagentSelection).toEqual({ value: false, provenance: "schema-default" });
		expect(resolved.subagentSelectionTimeoutMs).toEqual({ value: 300, provenance: "schema-default" });
	});

	it("reports calibrated recommendations when the user configured nothing", () => {
		const resolved = resolveLayaStatusSettings({}, CALIBRATED, SCHEMA_DEFAULTS);
		// Calibration recommended AGAINST pruning here: effective must follow
		// calibration, and must not claim a user override exists.
		expect(resolved.pruning).toEqual({ value: false, provenance: "calibrated" });
		expect(resolved.subagentSelection).toEqual({ value: true, provenance: "calibrated" });
		expect(resolved.subagentSelectionTimeoutMs).toEqual({ value: 300, provenance: "calibrated" });
	});

	it("lets an explicit user override win over a contradicting calibration", () => {
		const resolved = resolveLayaStatusSettings(
			{ pruning: true, subagentSelection: false, subagentSelectionTimeoutMs: 900 },
			CALIBRATED,
			SCHEMA_DEFAULTS,
		);
		expect(resolved.pruning).toEqual({ value: true, provenance: "user-configured" });
		expect(resolved.subagentSelection).toEqual({ value: false, provenance: "user-configured" });
		expect(resolved.subagentSelectionTimeoutMs).toEqual({ value: 900, provenance: "user-configured" });
	});

	it("resolves each key independently across the three sources", () => {
		const resolved = resolveLayaStatusSettings({ subagentSelectionTimeoutMs: 1200 }, CALIBRATED, SCHEMA_DEFAULTS);
		expect(resolved.pruning.provenance).toBe("calibrated");
		expect(resolved.subagentSelection.provenance).toBe("calibrated");
		expect(resolved.subagentSelectionTimeoutMs).toEqual({ value: 1200, provenance: "user-configured" });
	});
});
