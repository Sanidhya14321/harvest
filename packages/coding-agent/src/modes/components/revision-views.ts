/**
 * Shared managed-revision view helpers for the preset hub and the skill
 * revision manager. Pure presentation over the revision-service records —
 * both surfaces derive identical operator-visible state from the same
 * evaluation facts, so a revision reads the same everywhere.
 */

import type { EvaluationRecord } from "../../autolearn/revisions";

/** Operator-visible evaluation state derived from a revision's records. */
export type RevisionEvalStatus = "unevaluated" | "passing" | "failing";

/**
 * Derive the evaluation state: no records means unevaluated (never treated
 * as passed); otherwise every record must pass.
 */
export function revisionEvalStatus(evaluations: readonly Pick<EvaluationRecord, "passed">[]): RevisionEvalStatus {
	if (evaluations.length === 0) return "unevaluated";
	return evaluations.every(record => record.passed) ? "passing" : "failing";
}

/** One evaluation record as operator-readable detail lines (already sanitized content). */
export function formatEvaluationLines(
	evaluation: Pick<EvaluationRecord, "passed" | "createdAt" | "task" | "summary" | "model" | "modelBasis" | "runId">,
	maxSummaryChars = 160,
): string[] {
	const mark = evaluation.passed ? "PASS" : "FAIL";
	const bits = [`model=${evaluation.model ?? "?"}`];
	if (evaluation.modelBasis) bits.push(`basis=${evaluation.modelBasis}`);
	if (evaluation.runId) bits.push(`run=${evaluation.runId}`);
	return [
		`  ${mark} ${evaluation.createdAt} ${bits.join(" ")}`,
		`    task: ${evaluation.task}`,
		`    outcome: ${evaluation.summary.slice(0, maxSummaryChars)}`,
	];
}

/**
 * Promotion notice honoring the explicit unevaluated-disclosure contract:
 * unevaluated content activates only with disclosure, and the notice says so.
 */
export function formatPromotedNotice(
	kind: "preset" | "skill",
	name: string,
	revId: string,
	disclosedUnevaluated: boolean,
): string {
	const label = kind === "preset" ? "managed preset" : "managed skill";
	return (
		`Promoted ${label} ${name} to revision ${revId}` +
		(disclosedUnevaluated ? " (promoted unevaluated — disclosed)" : "")
	);
}
