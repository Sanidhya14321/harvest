/**
 * Laya-Based Context and Token Pruning Engine for Harvest (Phase 1).
 *
 * Evaluates candidate prunable context chunks (older tool results, earlier turns)
 * against the current task/goal using batched Laya relevance scoring.
 * Drops low-relevance content within a budget to shrink prompts sent to the main LLM.
 *
 * Core Guarantees:
 * 1. Always-Keep Set: System prompt, current task/goal description, most recent N turns
 *    (default N=2), and pinned messages bypass scoring entirely and are always included.
 * 2. Representative Scoring Window: Chunks > ~800 tokens (~3200 chars) are truncated
 *    to first ~400 + last ~400 tokens solely for scoring. Full original content is preserved if kept.
 * 3. Batched Scoring: Sends exactly ONE /v1/decide request containing all candidate score questions.
 * 4. Budget-Based Selection: Ranks candidates by score descending, keeps until budget is met,
 *    and replaces drops with informative placeholders (e.g. "[Earlier tool output omitted for relevance (Laya score: 0.12)]").
 * 5. Safety Floor: Never prunes below minimum turns (default 3) or safety token floor.
 * 6. Fallback: Fails OPEN on timeout (~300ms), sidecar unreachable, or error, passing full context.
 * 7. Calibration Logging: Logs relevance scores, kept/dropped status, and latency.
 */

import type { AgentMessage, ToolResultMessage, TextContent } from "@harvest/pi-ai";
import { logger } from "@harvest/pi-utils";
import { type Settings, settings as globalSettings, type SettingPath } from "../../config/settings";
import { getLayaClient, type LayaClient, type LayaQuestionDefinition } from "./laya-client";
import { getDerivedPruningEnabledSync } from "./laya-calibration";

function safeGetSetting<T>(settings: Settings | undefined, key: SettingPath): T | undefined {
	if (!settings) return undefined;
	try {
		return settings.get(key) as T;
	} catch {
		return undefined;
	}
}

export const DEFAULT_PRUNING_KEEP_RECENT_TURNS = 2;
export const DEFAULT_PRUNING_MIN_KEPT_TURNS = 3;
export const DEFAULT_PRUNING_MIN_CHUNK_TOKENS = 80;
export const DEFAULT_PRUNING_SAFETY_TOKEN_FLOOR = 1500;
export const MAX_SCORING_CHUNK_TOKENS = 800; // ~3200 characters

export const RELEVANCE_CRITERIA: readonly string[] = [
	"irrelevant to current task",
	"low relevance",
	"moderately relevant",
	"highly relevant to current task",
] as const;

export interface PruningCandidateChunk {
	readonly id: string;
	readonly messageIndex: number;
	readonly turnIndex: number;
	readonly role: string;
	readonly toolName?: string;
	readonly originalText: string;
	readonly estimatedTokens: number;
	readonly scoringExcerpt: string;
	score?: number;
	normalizedScore?: number;
	confidence?: number;
	dropped?: boolean;
}

export interface LayaPruningOptions {
	readonly client?: LayaClient;
	readonly settings?: Settings;
	readonly sessionId?: string;
	readonly taskGoal?: string;
	readonly keepRecentTurns?: number;
	readonly minKeptTurns?: number;
	readonly prunableTokenBudget?: number;
	readonly minChunkTokens?: number;
	readonly safetyTokenFloor?: number;
	readonly timeoutMs?: number;
	readonly lockedDecisions?: Map<string, LockedPruningDecision>;
	readonly lockDecisions?: boolean;
}

export interface LockedPruningDecision {
	readonly chunkId: string;
	readonly action: "kept" | "dropped";
	readonly placeholder?: string;
	readonly score: number;
	readonly normalizedScore: number;
	readonly confidence: number;
	readonly timestamp: number;
	readonly estimatedTokens: number;
}

/**
 * In-memory registry of locked pruning decisions.
 * Keyed by `${sessionId}::${chunkId}`.
 * Once a candidate chunk ages out of the recent window and is evaluated,
 * its keep/drop decision is locked here permanently for the session.
 * Subsequent turns never re-score or re-drop it, guaranteeing prefix byte-stability.
 */
export const LOCKED_PRUNING_DECISIONS = new Map<string, LockedPruningDecision>();

export function resetLockedPruningDecisions(sessionId?: string): void {
	if (!sessionId) {
		LOCKED_PRUNING_DECISIONS.clear();
		return;
	}
	const prefix = `${sessionId}::`;
	for (const key of LOCKED_PRUNING_DECISIONS.keys()) {
		if (key.startsWith(prefix)) {
			LOCKED_PRUNING_DECISIONS.delete(key);
		}
	}
}

export interface PruningAuditRecord {
	readonly timestamp: number;
	readonly sessionId: string;
	readonly chunkId: string;
	readonly role: string;
	readonly toolName?: string;
	readonly estimatedTokens: number;
	readonly score: number;
	readonly normalizedScore: number;
	readonly confidence: number;
	readonly action: "kept" | "dropped";
}

export interface LayaPruningResult {
	readonly messages: AgentMessage[];
	readonly pruned: boolean;
	readonly fallback: boolean;
	readonly fallbackReason?: string;
	readonly totalOriginalTokens: number;
	readonly totalPrunedTokens: number;
	readonly tokensSaved: number;
	readonly candidatesCount: number;
	readonly droppedCount: number;
	readonly latencyMs: number;
	readonly auditRecords: readonly PruningAuditRecord[];
}

/** In-memory log of recent pruning audit records for inspection and calibration. */
export const PRUNING_AUDIT_LOG: PruningAuditRecord[] = [];

/**
 * Fast estimation of token count from string content (~4 chars per token).
 */
export function estimateTextTokens(text: string): number {
	if (!text) return 0;
	return Math.ceil(text.length / 4);
}

/**
 * Extract clean string representation of an AgentMessage's content.
 */
export function extractMessageText(message: AgentMessage): string {
	if (typeof message.content === "string") {
		return message.content;
	}
	if (Array.isArray(message.content)) {
		return message.content
			.map(part => {
				if ("text" in part && typeof part.text === "string") return part.text;
				if ("thinking" in part && typeof part.thinking === "string") return part.thinking;
				return "";
			})
			.join("\n");
	}
	return "";
}

/**
 * Truncate a chunk to a representative scoring window (first ~400 + last ~400 tokens)
 * solely for Laya scoring, strictly respecting ModernBERT's context limits.
 */
export function createScoringExcerpt(text: string, maxTokens: number = MAX_SCORING_CHUNK_TOKENS): string {
	const estimated = estimateTextTokens(text);
	if (estimated <= maxTokens) {
		return text;
	}

	const halfChars = Math.floor((maxTokens / 2) * 4); // ~1600 chars (~400 tokens)
	const first = text.slice(0, halfChars);
	const last = text.slice(-halfChars);
	const omittedTokens = Math.max(0, estimated - maxTokens);

	return `${first}\n...[${omittedTokens} tokens omitted for relevance scoring]...\n${last}`;
}

/**
 * Group flat messages array into logical interaction turns.
 * In Harvest, each user-initiated message marks the start of a turn.
 */
export function partitionMessagesIntoTurns(
	messages: readonly AgentMessage[],
): Array<{ turnIndex: number; messageIndices: number[] }> {
	const turns: Array<{ turnIndex: number; messageIndices: number[] }> = [];
	let currentTurn: number[] = [];

	for (let i = 0; i < messages.length; i++) {
		const msg = messages[i];
		if (msg.role === "user" && currentTurn.length > 0) {
			turns.push({ turnIndex: turns.length, messageIndices: currentTurn });
			currentTurn = [];
		}
		currentTurn.push(i);
	}

	if (currentTurn.length > 0) {
		turns.push({ turnIndex: turns.length, messageIndices: currentTurn });
	}

	return turns;
}

/**
 * Extract the task/goal description from the session messages.
 */
export function extractTaskGoal(messages: readonly AgentMessage[]): string {
	// Prefer the first user message as the session task definition
	for (const msg of messages) {
		if (msg.role === "user") {
			const text = extractMessageText(msg).trim();
			if (text.length > 0) {
				return text.length > 500 ? `${text.slice(0, 500)}...` : text;
			}
		}
	}
	return "Complete the requested coding task.";
}

/**
 * Prune context messages using Laya relevance scoring before a main LLM call.
 */
export async function pruneContextWithLaya(
	messages: readonly AgentMessage[],
	options: LayaPruningOptions = {},
): Promise<LayaPruningResult> {
	const startTime = performance.now();
	const activeSettings = options.settings ?? globalSettings;

	// Check if Laya or pruning is disabled
	if (process.env.LAYA_ENABLED === "false" || process.env.LAYA_PRUNING === "false") {
		return createPassthroughResult(messages, "laya_pruning_disabled_by_env");
	}

	if (safeGetSetting<boolean>(activeSettings, "laya.enabled") === false) {
		return createPassthroughResult(messages, "laya_disabled_by_settings");
	}
	const explicitPruningSetting = safeGetSetting<boolean>(activeSettings, "laya.pruning");
	const effectivePruningEnabled = explicitPruningSetting ?? getDerivedPruningEnabledSync(true);
	if (!effectivePruningEnabled) {
		return createPassthroughResult(
			messages,
			explicitPruningSetting === false
				? "laya_pruning_disabled_by_settings"
				: "laya_pruning_disabled_by_hardware_calibration",
		);
	}
	if (explicitPruningSetting === true && !getDerivedPruningEnabledSync(true)) {
		logger.warn(
			"Laya pruning is explicitly enabled in settings, but local hardware calibration advised against it due to high latency.",
		);
	}

	if (messages.length === 0) {
		return createPassthroughResult(messages, "empty_messages");
	}

	const keepRecentTurns =
		options.keepRecentTurns ??
		safeGetSetting<number>(activeSettings, "laya.pruningKeepRecentTurns") ??
		DEFAULT_PRUNING_KEEP_RECENT_TURNS;
	const minKeptTurns =
		options.minKeptTurns ??
		safeGetSetting<number>(activeSettings, "laya.pruningMinKeptTurns") ??
		DEFAULT_PRUNING_MIN_KEPT_TURNS;
	const minChunkTokens =
		options.minChunkTokens ??
		safeGetSetting<number>(activeSettings, "laya.pruningMinChunkTokens") ??
		DEFAULT_PRUNING_MIN_CHUNK_TOKENS;
	const safetyTokenFloor = options.safetyTokenFloor ?? DEFAULT_PRUNING_SAFETY_TOKEN_FLOOR;

	// Partition messages into logical turns
	const turns = partitionMessagesIntoTurns(messages);
	const totalTurns = turns.length;

	// If total turns is within the always-keep window, no pruning needed
	if (totalTurns <= keepRecentTurns) {
		return createPassthroughResult(messages, "within_always_keep_window");
	}

	const taskGoal = options.taskGoal || extractTaskGoal(messages);
	const alwaysKeepTurnThreshold = totalTurns - keepRecentTurns; // turns at or after this are always kept

	const sessionId = options.sessionId || "default";
	const lockStore = options.lockedDecisions ?? (options.sessionId ? LOCKED_PRUNING_DECISIONS : undefined);
	const useLocking = options.lockDecisions !== false && lockStore !== undefined;
	const preLockedByMsgIdx = new Map<number, LockedPruningDecision>();

	// Step 1: Identify prunable candidate pool
	const candidates: PruningCandidateChunk[] = [];
	let totalCandidateTokens = 0;

	for (const turn of turns) {
		if (turn.turnIndex >= alwaysKeepTurnThreshold) {
			// Step 1: Recent N turns bypass scoring entirely
			continue;
		}

		for (const msgIdx of turn.messageIndices) {
			const msg = messages[msgIdx];

			// Never prune user messages or pinned messages
			if (msg.role === "user") continue;
			if ((msg as { pinned?: boolean }).pinned === true) continue;

			// Check tool results or verbose assistant text
			if (msg.role === "toolResult") {
				const toolMsg = msg as ToolResultMessage;
				// Already pruned results skip scoring
				if (toolMsg.prunedAt !== undefined) continue;

				const text = extractMessageText(toolMsg);
				const tokens = estimateTextTokens(text);

				if (tokens >= minChunkTokens) {
					const chunkId = `tool_${turn.turnIndex}_${msgIdx}_${toolMsg.toolName || "tool"}`;

					const lockKey = `${sessionId}::${chunkId}`;
					if (useLocking && lockStore.has(lockKey)) {
						preLockedByMsgIdx.set(msgIdx, lockStore.get(lockKey)!);
						continue;
					}

					candidates.push({
						id: chunkId,
						messageIndex: msgIdx,
						turnIndex: turn.turnIndex,
						role: "toolResult",
						toolName: toolMsg.toolName,
						originalText: text,
						estimatedTokens: tokens,
						scoringExcerpt: createScoringExcerpt(text),
					});
					totalCandidateTokens += tokens;
				}
			} else if (msg.role === "assistant") {
				const text = extractMessageText(msg);
				const tokens = estimateTextTokens(text);

				// Only consider very large assistant outputs in older turns
				if (tokens >= minChunkTokens * 2) {
					const chunkId = `assistant_${turn.turnIndex}_${msgIdx}`;
					const lockKey = `${sessionId}::${chunkId}`;
					if (useLocking && lockStore.has(lockKey)) {
						preLockedByMsgIdx.set(msgIdx, lockStore.get(lockKey)!);
						continue;
					}

					candidates.push({
						id: chunkId,
						messageIndex: msgIdx,
						turnIndex: turn.turnIndex,
						role: "assistant",
						originalText: text,
						estimatedTokens: tokens,
						scoringExcerpt: createScoringExcerpt(text),
					});
					totalCandidateTokens += tokens;
				}
			}
		}
	}

	// If no new candidate chunks exceed minimum size, apply pre-locked decisions directly or pass through
	if (candidates.length === 0) {
		if (preLockedByMsgIdx.size === 0) {
			return createPassthroughResult(messages, "no_prunable_candidates");
		}

		let tokensSaved = 0;
		let droppedCount = 0;
		const clonedMessages: AgentMessage[] = [];
		const auditRecords: PruningAuditRecord[] = [];

		for (let i = 0; i < messages.length; i++) {
			const original = messages[i];
			const locked = preLockedByMsgIdx.get(i);
			if (locked && locked.action === "dropped") {
				droppedCount++;
				tokensSaved += locked.estimatedTokens;
				auditRecords.push({
					timestamp: locked.timestamp,
					sessionId,
					chunkId: locked.chunkId,
					role: original.role,
					toolName: (original as ToolResultMessage).toolName,
					estimatedTokens: locked.estimatedTokens,
					score: locked.score,
					normalizedScore: locked.normalizedScore,
					confidence: locked.confidence,
					action: "dropped",
				});
				if (original.role === "toolResult") {
					clonedMessages.push({
						...(original as ToolResultMessage),
						content: [{ type: "text", text: locked.placeholder ?? "" } as TextContent],
						prunedAt: locked.timestamp,
					});
				} else if (original.role === "assistant") {
					clonedMessages.push({
						...original,
						content: [{ type: "text", text: locked.placeholder ?? "" } as TextContent],
					});
				} else {
					clonedMessages.push(original);
				}
			} else {
				clonedMessages.push(original);
			}
		}

		return {
			messages: clonedMessages,
			pruned: droppedCount > 0,
			fallback: false,
			totalOriginalTokens: tokensSaved,
			totalPrunedTokens: 0,
			tokensSaved,
			candidatesCount: preLockedByMsgIdx.size,
			droppedCount,
			latencyMs: performance.now() - startTime,
			auditRecords,
		};
	}

	// Step 3: Batch scoring call against Laya sidecar
	const client = options.client ?? getLayaClient();
	const questions: Record<string, LayaQuestionDefinition> = {};
	const statePerChunk: Record<string, string> = {};

	for (const c of candidates) {
		const label = c.toolName ? `Tool '${c.toolName}' result` : `${c.role} response`;
		questions[c.id] = {
			type: "score",
			instructions: `Rate how relevant this ${label} is to the current task/goal`,
			criteria: RELEVANCE_CRITERIA,
		};
		statePerChunk[c.id] = `Current Task/Goal:\n${taskGoal}\n\nCandidate Chunk (${label}):\n${c.scoringExcerpt}`;
	}

	const decideResult = await client.decide(statePerChunk, questions, {
		callSite: "context_pruning",
		sessionId: options.sessionId,
		timeoutMs: options.timeoutMs,
	});

	// Step 6: Fallback - fail OPEN if sidecar is down, times out, or errors
	if (decideResult.fallback || !decideResult.data) {
		const latencyMs = performance.now() - startTime;
		logger.warn("Laya context pruning fallback [FAIL OPEN]: keeping full unpruned context", {
			reason: decideResult.fallbackReason,
			latencyMs,
			candidatesCount: candidates.length,
		});

		return {
			messages: [...messages],
			pruned: false,
			fallback: true,
			fallbackReason: decideResult.fallbackReason || "sidecar_scoring_error",
			totalOriginalTokens: totalCandidateTokens,
			totalPrunedTokens: 0,
			tokensSaved: 0,
			candidatesCount: candidates.length,
			droppedCount: 0,
			latencyMs,
			auditRecords: [],
		};
	}

	// Attach returned scores to candidate records
	for (const c of candidates) {
		const ans = decideResult.data[c.id];
		if (ans && typeof ans.score === "number") {
			c.score = ans.score;
			// 4 levels: 0 to 3 -> normalize to [0, 1]
			c.normalizedScore = Math.min(1, Math.max(0, ans.score / 3.0));
			c.confidence = ans.confidence ?? 0.5;
		} else {
			// Default neutral score if missing
			c.score = 1.5;
			c.normalizedScore = 0.5;
			c.confidence = 0.5;
		}
	}

	// Step 4: Budget-based selection in Harvest
	// If prunableTokenBudget is not explicitly configured, retain 40% of candidate tokens
	const prunableBudget =
		options.prunableTokenBudget ?? Math.max(safetyTokenFloor, Math.floor(totalCandidateTokens * 0.4));

	// Rank candidates descending by score (highest relevance first)
	const rankedCandidates = [...candidates].sort((a, b) => (b.score ?? 0) - (a.score ?? 0));

	let accumulatedBudget = 0;
	const safetyFloorTurnThreshold = totalTurns - minKeptTurns;

	for (const c of rankedCandidates) {
		// Step 5: Safety Floor - never prune chunks in minKeptTurns
		if (c.turnIndex >= safetyFloorTurnThreshold) {
			c.dropped = false;
			accumulatedBudget += c.estimatedTokens;
			continue;
		}

		if (accumulatedBudget + c.estimatedTokens <= prunableBudget) {
			// Fits within budget: keep
			c.dropped = false;
			accumulatedBudget += c.estimatedTokens;
		} else {
			// Exceeds budget: drop
			c.dropped = true;
		}
	}

	const now = Date.now();

	// Step 4b: Lock decisions permanently for newly evaluated candidates
	if (useLocking) {
		for (const c of candidates) {
			const lockKey = `${sessionId}::${c.id}`;
			const placeholder = c.dropped
				? c.role === "toolResult"
					? `[Earlier tool output for '${c.toolName || "tool"}' omitted for relevance (Laya score: ${(c.normalizedScore ?? 0).toFixed(2)})]`
					: `[Earlier assistant output omitted for relevance (Laya score: ${(c.normalizedScore ?? 0).toFixed(2)})]`
				: undefined;

			lockStore.set(lockKey, {
				chunkId: c.id,
				action: c.dropped ? "dropped" : "kept",
				placeholder,
				score: c.score ?? 0,
				normalizedScore: c.normalizedScore ?? 0,
				confidence: c.confidence ?? 0,
				timestamp: now,
				estimatedTokens: c.estimatedTokens,
			});
		}
	}

	// Replace dropped chunks with transparent placeholders
	let tokensSaved = 0;
	let droppedCount = 0;
	const candidateByMsgIdx = new Map<number, PruningCandidateChunk>();
	for (const c of candidates) {
		candidateByMsgIdx.set(c.messageIndex, c);
	}

	const clonedMessages: AgentMessage[] = [];
	const auditRecords: PruningAuditRecord[] = [];

	for (let i = 0; i < messages.length; i++) {
		const original = messages[i];
		const candidate = candidateByMsgIdx.get(i);
		const preLocked = preLockedByMsgIdx.get(i);

		if (preLocked && preLocked.action === "dropped") {
			droppedCount++;
			tokensSaved += preLocked.estimatedTokens;
			auditRecords.push({
				timestamp: preLocked.timestamp,
				sessionId,
				chunkId: preLocked.chunkId,
				role: original.role,
				toolName: (original as ToolResultMessage).toolName,
				estimatedTokens: preLocked.estimatedTokens,
				score: preLocked.score,
				normalizedScore: preLocked.normalizedScore,
				confidence: preLocked.confidence,
				action: "dropped",
			});
			if (original.role === "toolResult") {
				clonedMessages.push({
					...(original as ToolResultMessage),
					content: [{ type: "text", text: preLocked.placeholder ?? "" } as TextContent],
					prunedAt: preLocked.timestamp,
				});
			} else if (original.role === "assistant") {
				clonedMessages.push({
					...original,
					content: [{ type: "text", text: preLocked.placeholder ?? "" } as TextContent],
				});
			} else {
				clonedMessages.push(original);
			}
		} else if (candidate && candidate.dropped) {
			droppedCount++;
			tokensSaved += candidate.estimatedTokens;

			auditRecords.push({
				timestamp: now,
				sessionId,
				chunkId: candidate.id,
				role: candidate.role,
				toolName: candidate.toolName,
				estimatedTokens: candidate.estimatedTokens,
				score: candidate.score ?? 0,
				normalizedScore: candidate.normalizedScore ?? 0,
				confidence: candidate.confidence ?? 0,
				action: "dropped",
			});

			if (original.role === "toolResult") {
				const toolMsg = original as ToolResultMessage;
				const placeholder = `[Earlier tool output for '${toolMsg.toolName || "tool"}' omitted for relevance (Laya score: ${(candidate.normalizedScore ?? 0).toFixed(2)})]`;
				clonedMessages.push({
					...toolMsg,
					content: [{ type: "text", text: placeholder } as TextContent],
					prunedAt: now,
				});
			} else if (original.role === "assistant") {
				const placeholder = `[Earlier assistant output omitted for relevance (Laya score: ${(candidate.normalizedScore ?? 0).toFixed(2)})]`;
				clonedMessages.push({
					...original,
					content: [{ type: "text", text: placeholder } as TextContent],
				});
			} else {
				clonedMessages.push(original);
			}
		} else {
			if (candidate) {
				auditRecords.push({
					timestamp: now,
					sessionId,
					chunkId: candidate.id,
					role: candidate.role,
					toolName: candidate.toolName,
					estimatedTokens: candidate.estimatedTokens,
					score: candidate.score ?? 0,
					normalizedScore: candidate.normalizedScore ?? 0,
					confidence: candidate.confidence ?? 0,
					action: "kept",
				});
			}
			clonedMessages.push(original);
		}
	}

	// Record in memory audit log for telemetry/calibration
	PRUNING_AUDIT_LOG.push(...auditRecords);
	if (PRUNING_AUDIT_LOG.length > 500) {
		PRUNING_AUDIT_LOG.splice(0, PRUNING_AUDIT_LOG.length - 500);
	}

	const latencyMs = performance.now() - startTime;
	logger.info("Laya context pruning complete", {
		candidatesCount: candidates.length,
		droppedCount,
		tokensSaved,
		latencyMs,
	});

	return {
		messages: clonedMessages,
		pruned: droppedCount > 0,
		fallback: false,
		totalOriginalTokens: totalCandidateTokens,
		totalPrunedTokens: totalCandidateTokens - tokensSaved,
		tokensSaved,
		candidatesCount: candidates.length,
		droppedCount,
		latencyMs,
		auditRecords,
	};
}

function createPassthroughResult(messages: readonly AgentMessage[], reason: string): LayaPruningResult {
	return {
		messages: [...messages],
		pruned: false,
		fallback: false,
		fallbackReason: reason,
		totalOriginalTokens: 0,
		totalPrunedTokens: 0,
		tokensSaved: 0,
		candidatesCount: 0,
		droppedCount: 0,
		latencyMs: 0,
		auditRecords: [],
	};
}
