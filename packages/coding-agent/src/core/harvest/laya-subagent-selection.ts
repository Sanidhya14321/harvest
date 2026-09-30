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

import * as fs from "node:fs/promises";
import * as path from "node:path";
import { getAgentDir, isEnoent, logger } from "@harvest/pi-utils";
import { type Settings, settings as globalSettings, type SettingPath } from "../../config/settings";
import type { AgentDefinition } from "../../task/types";
import { getLayaClient, type LayaClient, type LayaQuestionDefinition } from "./laya-client";
import { getDerivedTimeoutMsSync, getExplicitSetting } from "./laya-calibration";

export const DEFAULT_SUBAGENT_SELECTION_CONFIDENCE_THRESHOLD = 0.01;
export const DEFAULT_SUBAGENT_TIMEOUT_MS = 300;

export const BUILTIN_AGENT_CRITERIA: Record<string, string> = {
	scout:
		"Codebase navigation and discovery. Select when the user wants to understand or find code without modifying it. Example tasks: 'Where is the auth handler defined?', 'Find all references to AgentMessage', 'Trace the request lifecycle'.",
	reviewer:
		"Code review and pull request inspection. Select when the user wants qualitative feedback or bug inspection on changes. Example tasks: 'Review this diff for bugs', 'Check this PR for edge cases', 'Verify correctness of this commit'.",
	"security-reviewer":
		"Security and vulnerability assessment. Select when the task is specifically focused on security threats or sensitive data leaks. Example tasks: 'Check for SQL injection or path traversal', 'Scan for hardcoded API keys', 'Audit permissions'.",
	sonic:
		"Mechanical and repetitive tasks. Select for non-creative, simple bulk edits that don't need deep reasoning. Example tasks: 'Replace tab indents with spaces across all files', 'Convert CRLF to LF', 'Rename imports'.",
	task:
		"Implementation and active software engineering. Select when the agent must write code, create files, implement algorithms, or execute end-to-end tasks. Example tasks: 'Build a rate limiter', 'Fix this bug and write tests', 'Implement feature X'.",
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
	readonly shadow?: boolean;
	readonly agentDir?: string;
	/** Parent abort signal; an aborted signal settles immediately to a fallback. */
	readonly signal?: AbortSignal;
	/** Pre-allocated trace id so background shadow runs can join outcomes recorded by the dispatcher. */
	readonly traceId?: string;
}

/** True when settings enable active Laya auto-pick (dispatch must await the decision). */
export function isLayaSubagentAutoPickEnabled(settings?: Settings): boolean {
	return safeGetSetting<boolean>(settings, "laya.subagentSelection" as SettingPath) === true;
}

/** True when shadow telemetry should be recorded (defaults on unless auto-pick or settings disable it). */
export function isLayaSubagentShadowEnabled(settings?: Settings): boolean {
	const setting = safeGetSetting<boolean>(settings, "laya.subagentSelectionShadow" as SettingPath);
	if (setting !== undefined) return setting;
	return safeGetSetting<boolean>(settings, "laya.subagentSelection" as SettingPath) !== false;
}

/** Maximum concurrent background shadow classifications (telemetry only; dispatch never waits). */
export const MAX_SHADOW_SUBAGENT_IN_FLIGHT = 2;

let shadowSubagentInFlight = 0;

/**
 * Run shadow subagent classification without blocking dispatch: the caller
 * uses its default agent immediately while Laya scores the assignment in
 * the background for audit/disagreement data. Bounded (excess requests are
 * dropped, never queued) and parent-cancellable via `options.signal`.
 */
export function classifySubagentShadow(
	assignment: string,
	options: LayaSubagentSelectionOptions = {},
): void {
	if (shadowSubagentInFlight >= MAX_SHADOW_SUBAGENT_IN_FLIGHT) {
		logger.debug("Dropping shadow subagent classification: too many in flight", {
			inFlight: shadowSubagentInFlight,
		});
		return;
	}
	shadowSubagentInFlight++;
	void selectSubagentWithLaya(assignment, { ...options, shadow: true })
		.catch(error => {
			logger.debug("Background shadow subagent classification failed", {
				error: error instanceof Error ? error.message : String(error),
			});
		})
		.finally(() => {
			shadowSubagentInFlight--;
		});
}

export interface SubagentOutcome {
	readonly completedCleanly: boolean;
	readonly exitCode?: number;
	readonly error?: string;
	readonly durationMs?: number;
	readonly timestamp: number;
}

export interface SubagentSelectionDecision {
	readonly traceId?: string;
	readonly selectedAgent: string;
	readonly layaPick?: string;
	readonly confidence?: number;
	readonly threshold: number;
	readonly decisionType: "auto_pick" | "escalation" | "fallback" | "shadow";
	readonly fallback: boolean;
	readonly fallbackReason?: string;
	readonly latencyMs: number;
}

export interface SubagentSelectionAuditRecord {
	readonly id: string;
	readonly timestamp: number;
	readonly sessionId: string;
	readonly assignment: string;
	readonly availableAgents: readonly string[];
	readonly layaPick?: string;
	readonly confidence?: number;
	readonly selectedAgent: string;
	readonly groundTruth?: string;
	readonly decisionType: "auto_pick" | "escalation" | "fallback" | "shadow";
	readonly latencyMs: number;
	outcome?: SubagentOutcome;
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
	const traceId = options.traceId ?? generateTraceId();
	const activeSettings = options.settings ?? globalSettings;
	const defaultAgent = options.defaultAgent ?? "task";

	const configuredThreshold =
		safeGetSetting<number>(activeSettings, "laya.subagentSelectionConfidenceThreshold" as SettingPath) ??
		DEFAULT_SUBAGENT_SELECTION_CONFIDENCE_THRESHOLD;
	const threshold = options.confidenceThreshold ?? configuredThreshold;

	// An already-aborted parent settles immediately to a fallback without
	// touching the sidecar — the caller is gone and the decision is moot.
	if (options.signal?.aborted) {
		return createFallbackResult(defaultAgent, threshold, "operation_cancelled", startTime, traceId);
	}

	// Check if Laya or subagent selection is disabled
	if (process.env.LAYA_ENABLED === "false" || process.env.LAYA_SUBAGENT_SELECTION === "false") {
		return createFallbackResult(defaultAgent, threshold, "laya_subagent_selection_disabled_by_env", startTime, traceId);
	}

	if (safeGetSetting<boolean>(activeSettings, "laya.enabled") === false) {
		return createFallbackResult(defaultAgent, threshold, "laya_disabled_by_settings", startTime, traceId);
	}

	const activeSelectionEnabled = safeGetSetting<boolean>(activeSettings, "laya.subagentSelection" as SettingPath) === true;
	const shadowSetting = safeGetSetting<boolean>(activeSettings, "laya.subagentSelectionShadow" as SettingPath);
	const shadowSelectionEnabled =
		options.shadow ??
		(shadowSetting !== undefined
			? shadowSetting
			: safeGetSetting<boolean>(activeSettings, "laya.subagentSelection" as SettingPath) !== false);

	if (!activeSelectionEnabled && !shadowSelectionEnabled && !options.forcedAuto) {
		return createFallbackResult(defaultAgent, threshold, "laya_subagent_selection_disabled_by_settings", startTime, traceId);
	}

	const available = options.availableAgents ?? [];
	const nameMap = new Map<string, string>();
	for (const a of available) {
		nameMap.set(a.name.toLowerCase(), a.name);
	}
	if (!nameMap.has(defaultAgent.toLowerCase())) {
		nameMap.set(defaultAgent.toLowerCase(), defaultAgent);
	}

	if (available.length <= 1) {
		// Only 0 or 1 agent available; no selection decision to make
		const onlyAgent = available[0]?.name ?? defaultAgent;
		return {
			traceId,
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
		getExplicitSetting<number>(activeSettings, "laya.subagentSelectionTimeoutMs") ??
		derivedTimeout;

	const decideResult = await client.decide(state, questions, {
		callSite: "subagent_selection",
		sessionId: options.sessionId,
		timeoutMs,
		signal: options.signal,
	});

	const latencyMs = performance.now() - startTime;

	// Step 3: Fail OPEN if sidecar is down, times out, or errors
	if (!decideResult || decideResult.fallback || !decideResult.data?.subagent_choice) {
		const fallbackReason = decideResult?.fallbackReason || "sidecar_unavailable";
		logger.warn("Laya subagent selection fallback [FAIL OPEN]: using default agent", {
			defaultAgent,
			reason: fallbackReason,
			latencyMs,
		});

		recordAuditLog({
			id: traceId,
			timestamp: Date.now(),
			sessionId: options.sessionId || "default",
			assignment,
			availableAgents: agentNames,
			selectedAgent: defaultAgent,
			decisionType: "fallback",
			latencyMs,
		}, options.agentDir);

		return {
			traceId,
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
	const layaPickNormalized = rawChoice.toLowerCase().trim();
	const confidence = answer.confidence ?? 0.0;

	// Validate that Laya's pick is in the available roster and restore original exact case
	const validPickNormalized = agentNames.includes(layaPickNormalized) ? layaPickNormalized : defaultAgent.toLowerCase();
	const validPick = nameMap.get(validPickNormalized) ?? defaultAgent;

	// Step 2a: Shadow mode - record telemetry and audit logs, but ALWAYS dispatch default/caller agent
	if (!activeSelectionEnabled && !options.forcedAuto) {
		logger.info(
			`Laya subagent selection [SHADOW MODE]: suggested '${validPick}' (conf: ${confidence.toFixed(4)}), executing with default '${defaultAgent}'`,
			{
				layaPick: validPick,
				confidence,
				threshold,
				actualAgent: defaultAgent,
				latencyMs,
			},
		);

		recordAuditLog({
			id: traceId,
			timestamp: Date.now(),
			sessionId: options.sessionId || "default",
			assignment,
			availableAgents: agentNames,
			layaPick: validPick,
			confidence,
			selectedAgent: defaultAgent,
			groundTruth: defaultAgent,
			decisionType: "shadow",
			latencyMs,
		}, options.agentDir);

		return {
			traceId,
			selectedAgent: defaultAgent,
			layaPick: validPick,
			confidence,
			threshold,
			decisionType: "shadow",
			fallback: false,
			latencyMs,
		};
	}

	// Step 2b: Active selection mode - confidence-gated auto-pick vs escalation
	const shouldAutoPick = options.forcedAuto || confidence >= threshold;

	if (shouldAutoPick) {
		logger.info(`Laya subagent selection auto-picked '${validPick}'`, {
			confidence,
			threshold,
			latencyMs,
		});

		recordAuditLog({
			id: traceId,
			timestamp: Date.now(),
			sessionId: options.sessionId || "default",
			assignment,
			availableAgents: agentNames,
			layaPick: validPick,
			confidence,
			selectedAgent: validPick,
			decisionType: "auto_pick",
			latencyMs,
		}, options.agentDir);

		return {
			traceId,
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
	logger.info(`Laya subagent selection escalated (confidence ${confidence.toFixed(4)} < ${threshold.toFixed(4)})`, {
		layaPick: validPick,
		confidence,
		threshold,
		escalatedTo: defaultAgent,
		latencyMs,
	});

	recordAuditLog({
		id: traceId,
		timestamp: Date.now(),
		sessionId: options.sessionId || "default",
		assignment,
		availableAgents: agentNames,
		layaPick: validPick,
		confidence,
		selectedAgent: defaultAgent,
		decisionType: "escalation",
		latencyMs,
	}, options.agentDir);

	return {
		traceId,
		selectedAgent: defaultAgent,
		layaPick: validPick,
		confidence,
		threshold,
		decisionType: "escalation",
		fallback: false,
		latencyMs,
	};
}

export function generateTraceId(): string {
	return `laya_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
}

export function getAuditLogPath(agentDir?: string): string {
	const dir = agentDir ?? getAgentDir();
	return path.join(dir, "laya-subagent-selection-audit.jsonl");
}

function createFallbackResult(
	defaultAgent: string,
	threshold: number,
	reason: string,
	startTime: number,
	traceId?: string,
): SubagentSelectionDecision {
	return {
		traceId: traceId ?? generateTraceId(),
		selectedAgent: defaultAgent,
		threshold,
		decisionType: "fallback",
		fallback: true,
		fallbackReason: reason,
		latencyMs: performance.now() - startTime,
	};
}

export function recordAuditLog(record: SubagentSelectionAuditRecord, agentDir?: string): void {
	SUBAGENT_SELECTION_AUDIT_LOG.push(record);
	if (SUBAGENT_SELECTION_AUDIT_LOG.length > 500) {
		SUBAGENT_SELECTION_AUDIT_LOG.splice(0, SUBAGENT_SELECTION_AUDIT_LOG.length - 500);
	}
	// Append record to disk asynchronously
	const filePath = getAuditLogPath(agentDir);
	const line = JSON.stringify({ type: "decision", ...record }) + "\n";
	fs.appendFile(filePath, line, "utf-8").catch(err => {
		logger.warn("Failed to append subagent selection audit record to disk", { filePath, error: String(err) });
	});
}

export async function recordSubagentOutcome(
	traceId: string,
	outcome: SubagentOutcome,
	agentDir?: string,
): Promise<void> {
	if (!traceId) return;

	const inMemory = SUBAGENT_SELECTION_AUDIT_LOG.find(r => r.id === traceId);
	if (inMemory) {
		inMemory.outcome = outcome;
	}

	try {
		const filePath = getAuditLogPath(agentDir);
		const line = JSON.stringify({ type: "outcome", traceId, ...outcome }) + "\n";
		await fs.appendFile(filePath, line, "utf-8");
	} catch (err) {
		logger.warn("Failed to append subagent outcome to disk audit log", { traceId, error: String(err) });
	}
}

export async function loadAuditRecords(agentDir?: string): Promise<SubagentSelectionAuditRecord[]> {
	const filePath = getAuditLogPath(agentDir);
	let content: string;
	try {
		content = await fs.readFile(filePath, "utf-8");
	} catch (err) {
		if (isEnoent(err)) return [];
		logger.warn("Failed to read subagent selection audit log", { filePath, error: String(err) });
		return [];
	}

	const recordMap = new Map<string, SubagentSelectionAuditRecord>();
	const lines = content.split("\n");

	for (const rawLine of lines) {
		const line = rawLine.trim();
		if (!line) continue;
		try {
			const entry = JSON.parse(line);
			if (entry.type === "outcome" && entry.traceId) {
				const existing = recordMap.get(entry.traceId);
				if (existing) {
					existing.outcome = {
						completedCleanly: Boolean(entry.completedCleanly),
						exitCode: entry.exitCode,
						error: entry.error,
						durationMs: entry.durationMs,
						timestamp: entry.timestamp ?? Date.now(),
					};
				}
			} else if (entry.id) {
				const record: SubagentSelectionAuditRecord = {
					id: entry.id,
					timestamp: entry.timestamp ?? Date.now(),
					sessionId: entry.sessionId ?? "default",
					assignment: entry.assignment ?? "",
					availableAgents: entry.availableAgents ?? [],
					layaPick: entry.layaPick,
					confidence: entry.confidence,
					selectedAgent: entry.selectedAgent,
					groundTruth: entry.groundTruth,
					decisionType: entry.decisionType,
					latencyMs: entry.latencyMs ?? 0,
					outcome: entry.outcome,
				};
				recordMap.set(entry.id, record);
			}
		} catch {
			// Skip corrupted or unparseable lines
		}
	}

	return Array.from(recordMap.values());
}
