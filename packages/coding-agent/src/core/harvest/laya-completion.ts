/**
 * Laya Step & Task Completion Evaluation Layer.
 *
 * Gating point 3: Evaluates step execution / tool output or assistant stop
 * using batched `noul` questions ("did this succeed / is this done?").
 *
 * Contract:
 * - Fail OPEN: If sidecar is unavailable, times out (~300ms), or state is
 *   non-English, fall back to the existing full-LLM evaluation check.
 * - Asks only the needed questions per call (the production unexpected-stop
 *   path sends just `unexpected_stop`) and propagates the turn signal so a
 *   cancelled turn never waits out the decision timeout.
 */

import { logger } from "@harvest/pi-utils";
import { type Settings, settings } from "../../config/settings";
import { getLayaClient, type LayaClient } from "./laya-client";
import stepSuccessInstructions from "../../prompts/laya/completion-step-success.md" with { type: "text" };
import unexpectedStopInstructions from "../../prompts/laya/completion-unexpected-stop.md" with { type: "text" };

export interface StepCompletionState {
	readonly command?: string;
	readonly output?: string;
	readonly assistantText?: string;
	readonly taskContext?: string;
}

export interface CompletionDecision {
	readonly isSuccess: boolean;
	readonly isPrematureStop: boolean;
	readonly confidence?: number;
	readonly fallback: boolean;
	readonly fallbackReason?: string;
	readonly latencyMs: number;
}

export async function checkCompletionWithLaya(
	state: StepCompletionState,
	options: {
		client?: LayaClient;
		sessionId?: string;
		settings?: Settings;
		/** Turn abort signal: an aborted turn settles immediately without touching the sidecar. */
		signal?: AbortSignal;
		/**
		 * Which questions to ask. Defaults to both for the general
		 * step-evaluation contract; the production unexpected-stop path asks
		 * only `unexpected_stop` so it never pays for the unused success
		 * question.
		 */
		checks?: readonly ("success" | "unexpected_stop")[];
	} = {},
): Promise<CompletionDecision> {
	if (process.env.LAYA_ENABLED === "false") {
		return {
			isSuccess: true,
			isPrematureStop: false,
			fallback: true,
			fallbackReason: "laya_disabled",
			latencyMs: 0,
		};
	}
	if (options.signal?.aborted) {
		return {
			isSuccess: true,
			isPrematureStop: false,
			fallback: true,
			fallbackReason: "operation_cancelled",
			latencyMs: 0,
		};
	}
	const currentSettings = options.settings ?? settings;
	try {
		if (currentSettings.get("laya.enabled") === false) {
			return {
				isSuccess: true,
				isPrematureStop: false,
				fallback: true,
				fallbackReason: "laya_disabled_by_settings",
				latencyMs: 0,
			};
		}
	} catch {
		// Ignore if settings context not initialized
	}

	const client = options.client ?? getLayaClient();

	const statePayload = {
		command: state.command || "",
		output: (state.output || "").slice(-1500), // inspect recent diagnostic tail
		text: (state.assistantText || "").slice(0, 1000),
		...(state.taskContext ? { task_context: state.taskContext.slice(0, 1000) } : {}),
	};

	// Ask only the needed questions: the production unexpected-stop path
	// sends just `unexpected_stop`, while the general step-evaluation
	// contract keeps both.
	const checks = options.checks ?? (["success", "unexpected_stop"] as const);
	const questions: Record<string, { type: "noul"; instructions: string }> = {};
	if (checks.includes("success")) {
		questions.step_success = {
			type: "noul" as const,
			instructions: stepSuccessInstructions.trim(),
		};
	}
	if (checks.includes("unexpected_stop")) {
		questions.unexpected_stop = {
			type: "noul" as const,
			instructions: unexpectedStopInstructions.trim(),
		};
	}

	const decision = await client.decide(statePayload, questions, {
		callSite: "completion_check",
		sessionId: options.sessionId,
		signal: options.signal,
	});

	// If sidecar failed or timed out: FAIL OPEN to main LLM / heuristic
	if (decision.fallback || !decision.data) {
		logger.warn("Laya completion check fallback [FAIL OPEN]: delegating to main LLM / heuristic", {
			reason: decision.fallbackReason,
			latencyMs: decision.latencyMs,
		});

		return {
			isSuccess: true, // Fail OPEN
			isPrematureStop: false, // Fail OPEN
			fallback: true,
			fallbackReason: decision.fallbackReason || "unknown",
			latencyMs: decision.latencyMs,
		};
	}

	const successAnswer = decision.data.step_success;
	const stopAnswer = decision.data.unexpected_stop;

	// In noul questions, action/act_probability indicates affirmative detection
	const successScore = typeof successAnswer?.noul === "number" ? successAnswer.noul : 0.5;
	const prematureStopScore = typeof stopAnswer?.noul === "number" ? stopAnswer.noul : 0.0;
	const avgConfidence = ((successAnswer?.confidence ?? 0.5) + (stopAnswer?.confidence ?? 0.5)) / 2;

	const isSuccess = successScore >= 0.4;
	const isPrematureStop = prematureStopScore >= 0.5;

	logger.info("Laya completion check evaluated", {
		isSuccess,
		isPrematureStop,
		successScore,
		prematureStopScore,
		latencyMs: decision.latencyMs,
	});

	return {
		isSuccess,
		isPrematureStop,
		confidence: avgConfidence,
		fallback: false,
		latencyMs: decision.latencyMs,
	};
}
