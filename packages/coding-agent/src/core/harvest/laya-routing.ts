/**
 * Laya Model Routing Layer.
 *
 * Gating point 2: Evaluates task difficulty / requirements with a `choice`
 * question to pick the appropriate model tier (`smol`, `slow`, or `default`).
 *
 * Contract:
 * - Fail OPEN: If sidecar is unavailable, times out (~300ms), or returns
 *   low confidence, fall back to Harvest's existing default model.
 * - Feeds into Harvest's existing model/provider selection mechanism.
 */

import { logger } from "@harvest/pi-utils";
import { type Settings, settings } from "../../config/settings";
import { getLayaClient, type LayaClient } from "./laya-client";

export type ModelTierRole = "smol" | "slow" | "default";

export interface ModelRoutingDecision {
	readonly selectedRole: ModelTierRole;
	readonly confidence?: number;
	readonly fallback: boolean;
	readonly fallbackReason?: string;
	readonly latencyMs: number;
}

export const MODEL_ROUTING_CRITERIA: Record<ModelTierRole, string> = {
	smol: "Simple, repetitive, local syntax fixes, small queries, single-file edits, non-architectural changes",
	slow: "Complex multi-file architecture, deep algorithmic reasoning, difficult debugging, high-context planning",
	default: "Balanced standard software engineering tasks, moderate refactoring, component implementations",
};

export async function routeModelWithLaya(
	prompt: string,
	options: {
		client?: LayaClient;
		defaultRole?: ModelTierRole;
		sessionId?: string;
		settings?: Settings;
	} = {},
): Promise<ModelRoutingDecision> {
	const defaultRole = options.defaultRole ?? "default";

	if (process.env.LAYA_ENABLED === "false") {
		return {
			selectedRole: defaultRole,
			fallback: true,
			fallbackReason: "laya_disabled",
			latencyMs: 0,
		};
	}
	const currentSettings = options.settings ?? settings;
	try {
		if (currentSettings.get("laya.enabled") === false) {
			return {
				selectedRole: defaultRole,
				fallback: true,
				fallbackReason: "laya_disabled_by_settings",
				latencyMs: 0,
			};
		}
	} catch {
		// Ignore if settings context not initialized
	}

	const client = options.client ?? getLayaClient();

	const state = {
		prompt: prompt.slice(0, 1000), // bounded prompt snippet
	};

	const questions = {
		model_tier: {
			type: "choice" as const,
			instructions: "Which model capability tier is required to solve this coding task accurately?",
			criteria: MODEL_ROUTING_CRITERIA,
		},
	};

	const decision = await client.decide(state, questions, {
		callSite: "model_routing",
		sessionId: options.sessionId,
	});

	// If sidecar failed or timed out: FAIL OPEN to default
	if (decision.fallback || !decision.data?.model_tier) {
		logger.warn(`Laya model routing fallback [FAIL OPEN]: using role '${defaultRole}'`, {
			reason: decision.fallbackReason,
			latencyMs: decision.latencyMs,
		});

		return {
			selectedRole: defaultRole,
			fallback: true,
			fallbackReason: decision.fallbackReason || "unknown",
			latencyMs: decision.latencyMs,
		};
	}

	const answer = decision.data.model_tier;
	const selectedKey = (answer.answer as ModelTierRole) || defaultRole;
	const confidence = answer.confidence ?? 0.5;

	// Validate choice
	const validRole: ModelTierRole =
		selectedKey === "smol" || selectedKey === "slow" || selectedKey === "default"
			? selectedKey
			: defaultRole;

	logger.info(`Laya model routing selected role '${validRole}'`, {
		confidence,
		latencyMs: decision.latencyMs,
	});

	return {
		selectedRole: validRole,
		confidence,
		fallback: false,
		latencyMs: decision.latencyMs,
	};
}

export async function routeSpecialistRoleWithLaya(
	prompt: string,
	candidateRoles: string[],
	options: { client?: LayaClient; defaultRole?: string; sessionId?: string } = {},
): Promise<{ selectedRole: string; confidence?: number; fallback: boolean; latencyMs: number }> {
	const defaultRole = options.defaultRole ?? candidateRoles[0] ?? "backend";

	if (process.env.LAYA_ENABLED === "false") {
		return { selectedRole: defaultRole, fallback: true, latencyMs: 0 };
	}
	try {
		if (settings.get("laya.enabled") === false) {
			return { selectedRole: defaultRole, fallback: true, latencyMs: 0 };
		}
	} catch {
		// Ignore if settings context not initialized
	}

	const client = options.client ?? getLayaClient();

	const criteria: Record<string, string> = {};
	for (const role of candidateRoles) {
		criteria[role] = `Focus primarily on ${role} engineering aspects of the user prompt`;
	}

	const state = { prompt: prompt.slice(0, 1000) };
	const questions = {
		specialist_role: {
			type: "choice" as const,
			instructions: "Which engineering specialist role is best suited to lead this task?",
			criteria,
		},
	};

	const decision = await client.decide(state, questions, {
		callSite: "model_routing",
		sessionId: options.sessionId,
	});

	if (decision.fallback || !decision.data?.specialist_role) {
		logger.warn(`Laya specialist routing fallback [FAIL OPEN]: using role '${defaultRole}'`, {
			reason: decision.fallbackReason,
			latencyMs: decision.latencyMs,
		});
		return {
			selectedRole: defaultRole,
			fallback: true,
			latencyMs: decision.latencyMs,
		};
	}

	const answer = decision.data.specialist_role;
	const chosen = answer.answer && candidateRoles.includes(answer.answer) ? answer.answer : defaultRole;

	return {
		selectedRole: chosen,
		confidence: answer.confidence,
		fallback: false,
		latencyMs: decision.latencyMs,
	};
}
