/**
 * Laya Tool-Call Gating Layer.
 *
 * Gating point 1: Evaluates high-risk tool calls with a `noul` question
 * ("does this call write, delete, publish, or change access irreversibly?").
 *
 * Contract:
 * - High-risk tool list: bash, write, edit, ast-edit, patch.
 * - Fail CLOSED: If sidecar is unavailable, times out (~300ms), or returns
 *   high irreversibility score, require approval.
 * - Feeds into Harvest's existing approval logic as one signal, never replacing it.
 */

import { logger } from "@harvest/pi-utils";
import { type Settings, settings } from "../../config/settings";
import { getLayaClient, type LayaClient } from "./laya-client";

export const HIGH_RISK_TOOLS: ReadonlySet<string> = new Set([
	"bash",
	"exec",
	"command",
	"write",
	"write_to_file",
	"edit",
	"replace_file_content",
	"ast-edit",
	"ast_edit",
	"patch",
]);

export interface ToolGatingDecision {
	readonly isHighRiskTool: boolean;
	readonly requireApproval: boolean;
	readonly irreversibleScore?: number;
	readonly confidence?: number;
	readonly fallback: boolean;
	readonly reason: string;
	readonly latencyMs: number;
}

/** Threshold above which a noul score is classified as irreversible / dangerous. */
export const IRREVERSIBLE_NOUL_THRESHOLD = 0.35;

/** Minimum confidence required to auto-clear a high-risk tool call without approval. */
export const MIN_GATING_CONFIDENCE = 0.75;

export async function checkToolCallGating(
	toolName: string,
	args: Record<string, unknown>,
	options: { client?: LayaClient; sessionId?: string; settings?: Settings } = {},
): Promise<ToolGatingDecision> {
	if (process.env.LAYA_ENABLED === "false") {
		return {
			isHighRiskTool: false,
			requireApproval: false,
			fallback: false,
			reason: "laya_disabled",
			latencyMs: 0,
		};
	}
	const currentSettings = options.settings ?? settings;
	try {
		if (currentSettings.get("laya.enabled") === false) {
			return {
				isHighRiskTool: false,
				requireApproval: false,
				fallback: false,
				reason: "laya_disabled_by_settings",
				latencyMs: 0,
			};
		}
	} catch {
		// Ignore if settings context not initialized
	}

	const normalizedTool = toolName.toLowerCase().trim();
	const isHighRisk = HIGH_RISK_TOOLS.has(normalizedTool);

	// Low-risk tools bypass gating check completely
	if (!isHighRisk) {
		return {
			isHighRiskTool: false,
			requireApproval: false,
			fallback: false,
			reason: "tool_not_in_high_risk_list",
			latencyMs: 0,
		};
	}

	const client = options.client ?? getLayaClient();

	// Format state representation for Laya
	const state = {
		tool: normalizedTool,
		args: sanitizeGatingArgs(args),
	};

	const questions = {
		irreversibility: {
			type: "noul" as const,
			instructions: "does this call write, delete, publish, or change access irreversibly?",
		},
	};

	const decision = await client.decide(state, questions, {
		callSite: "tool_gating",
		sessionId: options.sessionId,
	});

	// If sidecar call failed, timed out, or state was non-English: FAIL CLOSED
	if (decision.fallback || !decision.data?.irreversibility) {
		logger.warn(`Laya gating fallback [FAIL CLOSED]: tool '${toolName}' requires approval`, {
			reason: decision.fallbackReason,
			latencyMs: decision.latencyMs,
		});

		return {
			isHighRiskTool: true,
			requireApproval: true, // FAIL CLOSED
			fallback: true,
			reason: `fallback_${decision.fallbackReason || "unknown"}`,
			latencyMs: decision.latencyMs,
		};
	}

	const answer = decision.data.irreversibility;
	// noul probability represents likelihood of irreversible action
	const noulScore = typeof answer.noul === "number" ? answer.noul : 0.5;
	const confidence = answer.confidence ?? 0.5;

	const isIrreversible = noulScore >= IRREVERSIBLE_NOUL_THRESHOLD;
	const isLowConfidence = confidence < MIN_GATING_CONFIDENCE;
	const requireApproval = isIrreversible || isLowConfidence;

	logger.info(`Laya gating decision for '${toolName}'`, {
		noulScore,
		confidence,
		isIrreversible,
		isLowConfidence,
		requireApproval,
		latencyMs: decision.latencyMs,
	});

	let reason = "laya_classified_reversible_safe";
	if (isIrreversible) {
		reason = "laya_classified_irreversible";
	} else if (isLowConfidence) {
		reason = "laya_low_confidence_escalation";
	}

	return {
		isHighRiskTool: true,
		requireApproval,
		irreversibleScore: noulScore,
		confidence,
		fallback: false,
		reason,
		latencyMs: decision.latencyMs,
	};
}

/** Sanitize tool arguments to avoid huge file strings or unbounded payloads in state. */
function sanitizeGatingArgs(args: Record<string, unknown>): Record<string, unknown> {
	const sanitized: Record<string, unknown> = {};

	for (const [k, v] of Object.entries(args)) {
		if (typeof v === "string") {
			// Truncate long content/files
			sanitized[k] = v.length > 500 ? `${v.slice(0, 500)}...[truncated]` : v;
		} else if (Array.isArray(v)) {
			sanitized[k] = v.slice(0, 10);
		} else if (typeof v === "object" && v !== null) {
			sanitized[k] = "[object]";
		} else {
			sanitized[k] = v;
		}
	}

	return sanitized;
}
