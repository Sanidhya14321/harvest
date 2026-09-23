/**
 * Laya Subagent Selection Engine for Harvest (Phase 2).
 *
 * Evaluates a given task/step assignment against Harvest's available subagent
 * roster using Laya's ModernBERT-large local typed decisions (`choice` question).
 *
 * Core Contracts:
 * 1. Fixed Roster Criteria: Distinct, mutually exclusive criteria for each subagent
 *    derived directly from Harvest's built-in agent descriptions.
 * 2. Confidence-Gated Escalation (Step 2):
 *    - If confidence >= threshold (default 0.80): Auto-pick Laya's choice directly.
 *    - If confidence < threshold: Escalate to caller / main LLM, logging an escalation event.
 * 3. Fail-Open Fallback (Step 3):
 *    - If sidecar is unavailable, times out (~300ms budget), or errors: fail OPEN
 *      to Harvest's pre-Laya logic (caller pick or default `task`).
 * 4. Calibration & Telemetry Logging (Step 4):
 *    - Records every decision in SUBAGENT_SELECTION_AUDIT_LOG and central logger.
 */

import { logger } from "@harvest/pi-utils";
import { type Settings, settings as globalSettings, type SettingPath } from "../../config/settings";
import type { AgentDefinition } from "../../task/types";
import { getLayaClient, type LayaClient, type LayaQuestionDefinition } from "./laya-client";
import { getDerivedTimeoutMsSync } from "./laya-calibration";

export const DEFAULT_SUBAGENT_SELECTION_CONFIDENCE_THRESHOLD = 0.8;
export const DEFAULT_SUBAGENT_TIMEOUT_MS = 300;

export const BUILTIN_AGENT_CRITERIA: Record<string, string> = {
	scout:
		"Exploratory codebase research, rapid code analysis, broad pattern searches, symbol location, finding where things are defined without modifying files",
	reviewer:
		"Code review specialist for analyzing quality, logic correctness, regressions, edge cases, and reviewing PR diffs",
	"security-reviewer":
		"Read-only security specialist for evidence-backed repository vulnerability discovery, CWE analysis, and security audits",
	sonic:
		"Low-reasoning agent for strictly mechanical updates, bulk formatting, or simple data collection",
	task:
		"General-purpose multi-step implementation, complex coding, refactoring, and feature additions requiring full tool capabilities",
};

export interface LayaSubagentSelectionOptions {
	readonly client?: LayaClient;
	readonly settings?: Settings;
	readonly availableAgents?: readonly AgentDefinition[];
	readonly defaultAgent?: string;
	readonly confidenceThreshold?: number;
	readonly sessionId?: string;
	readonly context?: string;
	readonly timeoutMs?: number;
	readonly forcedAuto?: boolean;
}

export interface SubagentSelectionDecision {
	readonly selectedAgent: string;
	readonly layaPick?: string;
	readonly confidence?: number;
	readonly threshold: number;
	readonly decisionType: "auto_pick" | "escalation" | "fallback";
	readonly fallback: boolean;
	readonly fallbackReason?: string;
	readonly latencyMs: number;
}

export interface SubagentSelectionAuditRecord {
	readonly timestamp: number;
	readonly sessionId: string;
	readonly assignment: string;
	readonly availableAgents: readonly string[];
	readonly layaPick?: string;
	readonly confidence?: number;
	readonly selectedAgent: string;
	readonly decisionType: "auto_pick" | "escalation" | "fallback";
	readonly latencyMs: number;
}

/** In-memory log of recent subagent selection decisions for inspection and calibration. */
export const SUBAGENT_SELECTION_AUDIT_LOG: SubagentSelectionAuditRecord[] = [];

function safeGetSetting<T>(settings: Settings | undefined, key: SettingPath): T | undefined {
	if (!settings) return undefined;
	try {
		return settings.get(key) as T;
	} catch {
		return undefined;
	}
}

const CANONICAL_AGENT_ORDER = ["scout", "reviewer", "security-reviewer", "task", "sonic"];

/**
 * Build a mutually distinct criteria map for the provided agent roster.
 */
export function buildSubagentCriteria(availableAgents: readonly AgentDefinition[]): Record<string, string> {
	const criteria: Record<string, string> = {};

	// Sort agents canonically so model prompt ordering is deterministic
	const sortedAgents = [...availableAgents].sort((a, b) => {
		const idxA = CANONICAL_AGENT_ORDER.indexOf(a.name.toLowerCase());
		const idxB = CANONICAL_AGENT_ORDER.indexOf(b.name.toLowerCase());
		if (idxA !== -1 && idxB !== -1) return idxA - idxB;
		if (idxA !== -1) return -1;
		if (idxB !== -1) return 1;
		return a.name.localeCompare(b.name);
	});

	for (const agent of sortedAgents) {
		const name = agent.name.toLowerCase();
		if (BUILTIN_AGENT_CRITERIA[name]) {
			criteria[name] = BUILTIN_AGENT_CRITERIA[name];
		} else if (agent.description && agent.description.trim().length > 0) {
			criteria[name] = agent.description.trim();
		} else {
			criteria[name] = `Specialized agent '${name}' for delegated tasks`;
		}
	}

	return criteria;
}

/**
 * Select the appropriate subagent for a given task assignment using Laya.
 */
export async function selectSubagentWithLaya(
	assignment: string,
	options: LayaSubagentSelectionOptions = {},
): Promise<SubagentSelectionDecision> {
	const startTime = performance.now();
	const activeSettings = options.settings ?? globalSettings;
	const defaultAgent = options.defaultAgent ?? "task";

	const configuredThreshold =
		safeGetSetting<number>(activeSettings, "laya.subagentSelectionConfidenceThreshold" as SettingPath) ??
		DEFAULT_SUBAGENT_SELECTION_CONFIDENCE_THRESHOLD;
	const threshold = options.confidenceThreshold ?? configuredThreshold;

	// Check if Laya or subagent selection is disabled
	if (process.env.LAYA_ENABLED === "false" || process.env.LAYA_SUBAGENT_SELECTION === "false") {
		return createFallbackResult(defaultAgent, threshold, "laya_subagent_selection_disabled_by_env", startTime);
	}

	if (safeGetSetting<boolean>(activeSettings, "laya.enabled") === false) {
		return createFallbackResult(defaultAgent, threshold, "laya_disabled_by_settings", startTime);
	}

	if (safeGetSetting<boolean>(activeSettings, "laya.subagentSelection" as SettingPath) === false) {
		return createFallbackResult(defaultAgent, threshold, "laya_subagent_selection_disabled_by_settings", startTime);
	}

	const available = options.availableAgents ?? [];
	if (available.length <= 1) {
		// Only 0 or 1 agent available; no selection decision to make
		const onlyAgent = available[0]?.name ?? defaultAgent;
		return {
			selectedAgent: onlyAgent,
			threshold,
			decisionType: "auto_pick",
			fallback: false,
			latencyMs: 0,
		};
	}

	const criteria = buildSubagentCriteria(available);
	const agentNames = Object.keys(criteria);

	const client = options.client ?? getLayaClient();

	const state = {
		task_assignment: assignment.slice(0, 1000),
		...(options.context ? { shared_context: options.context.slice(0, 1000) } : {}),
	};

	const questions: Record<string, LayaQuestionDefinition> = {
		subagent_choice: {
			type: "choice",
			instructions: "Which specialized subagent is best suited to execute this assigned task?",
			criteria,
		},
	};

	const derivedTimeout = getDerivedTimeoutMsSync(DEFAULT_SUBAGENT_TIMEOUT_MS);
	const timeoutMs =
		options.timeoutMs ??
		safeGetSetting<number>(activeSettings, "laya.subagentSelectionTimeoutMs" as SettingPath) ??
		derivedTimeout;

	const decideResult = await client.decide(state, questions, {
		callSite: "subagent_selection",
		sessionId: options.sessionId,
		timeoutMs,
	});

	const latencyMs = performance.now() - startTime;

	// Step 3: Fail OPEN if sidecar is down, times out, or errors
	if (decideResult.fallback || !decideResult.data?.subagent_choice) {
		const fallbackReason = decideResult.fallbackReason || "sidecar_unavailable";
		logger.warn("Laya subagent selection fallback [FAIL OPEN]: using default agent", {
			defaultAgent,
			reason: fallbackReason,
			latencyMs,
		});

		recordAuditLog({
			timestamp: Date.now(),
			sessionId: options.sessionId || "default",
			assignment,
			availableAgents: agentNames,
			selectedAgent: defaultAgent,
			decisionType: "fallback",
			latencyMs,
		});

		return {
			selectedAgent: defaultAgent,
			threshold,
			decisionType: "fallback",
			fallback: true,
			fallbackReason,
			latencyMs,
		};
	}

	const answer = decideResult.data.subagent_choice;
	const rawChoice = answer.choice || answer.answer || (answer as { selected?: string }).selected || "";
	const layaPick = rawChoice.toLowerCase().trim();
	const confidence = answer.confidence ?? 0.5;

	// Validate that Laya's pick is in the available roster
	const validPick = agentNames.includes(layaPick) ? layaPick : defaultAgent;

	// Step 2: Confidence-gated escalation
	// If forcedAuto is true (benchmarking mode) or confidence meets threshold: auto-pick
	const shouldAutoPick = options.forcedAuto || confidence >= threshold;

	if (shouldAutoPick) {
		logger.info(`Laya subagent selection auto-picked '${validPick}'`, {
			confidence,
			threshold,
			latencyMs,
		});

		recordAuditLog({
			timestamp: Date.now(),
			sessionId: options.sessionId || "default",
			assignment,
			availableAgents: agentNames,
			layaPick: validPick,
			confidence,
			selectedAgent: validPick,
			decisionType: "auto_pick",
			latencyMs,
		});

		return {
			selectedAgent: validPick,
			layaPick: validPick,
			confidence,
			threshold,
			decisionType: "auto_pick",
			fallback: false,
			latencyMs,
		};
	}

	// Below threshold: escalate to caller/default
	logger.info(`Laya subagent selection escalated (confidence ${confidence.toFixed(2)} < ${threshold.toFixed(2)})`, {
		layaPick: validPick,
		confidence,
		threshold,
		escalatedTo: defaultAgent,
		latencyMs,
	});

	recordAuditLog({
		timestamp: Date.now(),
		sessionId: options.sessionId || "default",
		assignment,
		availableAgents: agentNames,
		layaPick: validPick,
		confidence,
		selectedAgent: defaultAgent,
		decisionType: "escalation",
		latencyMs,
	});

	return {
		selectedAgent: defaultAgent,
		layaPick: validPick,
		confidence,
		threshold,
		decisionType: "escalation",
		fallback: false,
		latencyMs,
	};
}

function createFallbackResult(
	defaultAgent: string,
	threshold: number,
	reason: string,
	startTime: number,
): SubagentSelectionDecision {
	return {
		selectedAgent: defaultAgent,
		threshold,
		decisionType: "fallback",
		fallback: true,
		fallbackReason: reason,
		latencyMs: performance.now() - startTime,
	};
}

function recordAuditLog(record: SubagentSelectionAuditRecord): void {
	SUBAGENT_SELECTION_AUDIT_LOG.push(record);
	if (SUBAGENT_SELECTION_AUDIT_LOG.length > 500) {
		SUBAGENT_SELECTION_AUDIT_LOG.splice(0, SUBAGENT_SELECTION_AUDIT_LOG.length - 500);
	}
}
