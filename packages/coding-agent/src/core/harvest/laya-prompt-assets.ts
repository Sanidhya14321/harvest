/**
 * Shared versioned Laya prompt assets.
 *
 * Leaf module: imports only static `.md` prompt files and `@harvest/pi-utils`.
 * Production call sites (`laya-subagent-selection`) and hardware calibration
 * (`laya-calibration`) both build their sidecar questions from here so
 * calibration benchmarks measure the exact wording production sends. The
 * strict parsers throw at import time on a malformed asset rather than
 * sending a degraded question to the sidecar.
 */

import { prompt } from "@harvest/pi-utils";
import subagentSelectionInstructionsDoc from "../../prompts/laya/subagent-selection.md" with { type: "text" };
import subagentCriteriaDoc from "../../prompts/laya/subagent-criteria.md" with { type: "text" };
import pruningRelevanceTemplateDoc from "../../prompts/laya/pruning-relevance.md" with { type: "text" };
import pruningCriteriaDoc from "../../prompts/laya/pruning-criteria.md" with { type: "text" };
import calibrationSanityCheckDoc from "../../prompts/laya/calibration-sanity-check.md" with { type: "text" };

/** Production `choice` instructions for subagent selection. */
export const SUBAGENT_SELECTION_INSTRUCTIONS: string = subagentSelectionInstructionsDoc.trim();

/** Built-in subagent roster whose criteria must be covered by the versioned asset. */
export const REQUIRED_SUBAGENT_CRITERIA_NAMES: readonly string[] = [
	"scout",
	"reviewer",
	"security-reviewer",
	"task",
	"sonic",
];

/**
 * Parse `subagent-criteria.md` (`name: description` per line). Throws on a
 * malformed asset: lines without a colon, empty names/descriptions,
 * duplicates, unknown agent names, or a missing built-in agent.
 */
export function parseSubagentCriteria(doc: string): Record<string, string> {
	const criteria: Record<string, string> = {};
	for (const line of doc.split("\n")) {
		const trimmed = line.trim();
		if (!trimmed) continue;
		const colon = trimmed.indexOf(":");
		if (colon === -1) {
			throw new Error(`subagent-criteria.md: expected "name: description", got ${JSON.stringify(trimmed)}`);
		}
		const name = trimmed.slice(0, colon).trim().toLowerCase();
		const description = trimmed.slice(colon + 1).trim();
		if (!name || !description) {
			throw new Error(`subagent-criteria.md: empty name or description in ${JSON.stringify(trimmed)}`);
		}
		if (!REQUIRED_SUBAGENT_CRITERIA_NAMES.includes(name)) {
			throw new Error(`subagent-criteria.md: unknown agent ${JSON.stringify(name)}`);
		}
		if (criteria[name] !== undefined) {
			throw new Error(`subagent-criteria.md: duplicate entry for ${JSON.stringify(name)}`);
		}
		criteria[name] = description;
	}
	const missing = REQUIRED_SUBAGENT_CRITERIA_NAMES.filter(name => criteria[name] === undefined);
	if (missing.length > 0) {
		throw new Error(`subagent-criteria.md: missing criteria for ${missing.join(", ")}`);
	}
	return criteria;
}

/** Built-in subagent criteria parsed from the versioned prompt asset. */
export const SUBAGENT_CRITERIA: Record<string, string> = parseSubagentCriteria(subagentCriteriaDoc);

/**
 * Parse `pruning-criteria.md` (one relevance level per non-empty line).
 * Mirrors the strict parser in `laya-pruning.ts` (kept local to this leaf so
 * neither importer creates a core-module cycle); both throw unless the asset
 * defines exactly 4 levels, which is what score normalization divides by.
 */
export function parsePruningRelevanceCriteria(doc: string): readonly string[] {
	const levels = doc
		.split("\n")
		.map(line => line.trim())
		.filter(line => line.length > 0);
	if (levels.length !== 4) {
		throw new Error(`pruning-criteria.md must define exactly 4 relevance levels, found ${levels.length}`);
	}
	return levels;
}

/** Relevance levels parsed from the versioned prompt asset for calibration payloads. */
export const CALIBRATION_PRUNING_CRITERIA: readonly string[] = parsePruningRelevanceCriteria(pruningCriteriaDoc);

/** Render the versioned relevance question for one chunk label. */
export function renderCalibrationRelevanceInstructions(label: string): string {
	return prompt.render(pruningRelevanceTemplateDoc, { label }).trim();
}

/** Sanity-baseline `noul` instructions for the calibration ultra-short benchmark shape. */
export const CALIBRATION_SANITY_CHECK_INSTRUCTIONS: string = calibrationSanityCheckDoc.trim();
