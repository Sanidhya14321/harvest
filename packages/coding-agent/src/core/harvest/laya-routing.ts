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
import modelRoutingInstructions from "../../prompts/laya/model-routing.md" with { type: "text" };
import modelRoutingCriteriaDoc from "../../prompts/laya/model-routing-criteria.md" with { type: "text" };
import specialistRoutingInstructions from "../../prompts/laya/specialist-routing.md" with { type: "text" };

export type ModelTierRole = "smol" | "slow" | "default";

export interface ModelRoutingDecision {
	readonly selectedRole: ModelTierRole;
	readonly confidence?: number;
	readonly fallback: boolean;
	readonly fallbackReason?: string;
	readonly latencyMs: number;
}

/**
 * Tier choice descriptions parsed from the versioned prompt asset, so
 * calibration data stays tied to an exact prompt revision. Throws at import
 * time on a malformed asset rather than sending a degraded question.
 */
function parseTierCriteria(doc: string): Record<ModelTierRole, string> {
	const criteria = {} as Record<ModelTierRole, string>;
	for (const line of doc.split("\n")) {
		const trimmed = line.trim();
		if (!trimmed) continue;
		const colon = trimmed.indexOf(":");
		if (colon === -1) {
			throw new Error(`model-routing-criteria.md: expected "tier: description", got ${JSON.stringify(trimmed)}`);
		}
		const tier = trimmed.slice(0, colon).trim();
		if (tier !== "smol" && tier !== "slow" && tier !== "default") {
			throw new Error(`model-routing-criteria.md: unknown tier ${JSON.stringify(tier)}`);
		}
		criteria[tier] = trimmed.slice(colon + 1).trim();
	}
	if (!criteria.smol || !criteria.slow || !criteria.default) {
		throw new Error("model-routing-criteria.md: must define smol, slow, and default tiers");
	}
	return criteria;
}

export const MODEL_ROUTING_CRITERIA: Record<ModelTierRole, string> = parseTierCriteria(modelRoutingCriteriaDoc);

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
			instructions: modelRoutingInstructions.trim(),
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
			instructions: specialistRoutingInstructions.trim(),
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
