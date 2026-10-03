import { afterEach, describe, expect, it, vi } from "bun:test";
import type { AssistantMessage, ToolResultMessage, UserMessage } from "@harvest/pi-ai";
import type { AgentMessage } from "@harvest/pi-agent-core";
import {
	createScoringExcerpt,
	estimateTextTokens,
	extractMessageText,
	extractTaskGoal,
	invalidatePruningLocksOnHistoryRewrite,
	partitionMessagesIntoTurns,
	PRUNING_AUDIT_LOG,
	PRUNING_PROMPT_REVISION,
	pruneContextWithLaya,
	pruningLockKey,
	resetLockedPruningDecisions,
} from "../src/core/harvest/laya-pruning";
import { LayaClient } from "../src/core/harvest/laya-client";
import { setMemoryCachedCalibration, type CalibrationRecord } from "../src/core/harvest/laya-calibration";
import { Settings } from "../src/config/settings";

function makeUserMessage(text: string, pinned?: boolean): UserMessage {
	return {
		role: "user",
		content: text,
		timestamp: Date.now(),
		...(pinned ? { pinned: true } : {}),
	} as unknown as UserMessage;
}

function makeAssistantMessage(text: string): AssistantMessage {
	return {
		role: "assistant",
		provider: "mock",
		model: "mock/mock",
		api: "mock" as unknown as AssistantMessage["api"],
		content: [{ type: "text", text }],
		stopReason: "stop",
		timestamp: Date.now(),
	} as unknown as AssistantMessage;
}

function makeToolResultMessage(toolCallId: string, toolName: string, text: string): ToolResultMessage {
	return {
		role: "toolResult",
		toolCallId,
		toolName,
		content: [{ type: "text", text }],
		timestamp: Date.now(),
	} as unknown as ToolResultMessage;
}

afterEach(() => {
	vi.restoreAllMocks();
});

describe("Laya Context Pruning (Phase 1)", () => {
	it.each([undefined, Number.NaN, 4])(
		"keeps full context when the sidecar supplies an invalid relevance score (%s)",
		async score => {
			const client = new LayaClient();
			vi.spyOn(client, "decide").mockImplementation(async (_state, questions) => ({
				success: true,
				fallback: false,
				latencyMs: 0,
				data: Object.fromEntries(Object.keys(questions).map(key => [key, { type: "score", score, confidence: 1 }])),
			}));
			const messages: AgentMessage[] = [
				makeUserMessage("Inspect database transactions"),
				makeToolResultMessage("call-old", "read", "transaction evidence ".repeat(100)),
				makeUserMessage("Now fix the transaction"),
			];
			const testSettings = {
				get: (key: string) => (key === "laya.enabled" || key === "laya.pruning" ? true : undefined),
			} as unknown as Settings;
			const result = await pruneContextWithLaya(messages, {
				client,
				settings: testSettings,
				keepRecentTurns: 1,
				minKeptTurns: 1,
				prunableTokenBudget: 0,
			});
			expect(result.messages).toEqual(messages);
			expect(result.fallback).toBe(true);
			expect(result.fallbackReason).toBe("invalid_relevance_scores");
		},
	);
	describe("Token estimation and scoring excerpts (Step 2)", () => {
		it("estimates tokens based on character length", () => {
			expect(estimateTextTokens("")).toBe(0);
			expect(estimateTextTokens("1234")).toBe(1);
			expect(estimateTextTokens("a".repeat(400))).toBe(100);
		});

		it("leaves text under 800 tokens un-truncated", () => {
			const shortText = "const a = 1;\nconsole.log(a);";
			expect(createScoringExcerpt(shortText, 800)).toBe(shortText);
		});

		it("truncates chunks > 800 tokens to representative first+last window for scoring", () => {
			// 1000 tokens ~ 4000 characters
			const longText = "HEADER_START_" + "x".repeat(3980) + "_FOOTER_END";
			const excerpt = createScoringExcerpt(longText, 800);

			expect(excerpt).toContain("HEADER_START_");
			expect(excerpt).toContain("_FOOTER_END");
			expect(excerpt).toContain("omitted for relevance scoring");
			expect(excerpt.length).toBeLessThan(longText.length);
		});
	});

	describe("Message partitioning and goal extraction (Step 0 & 1)", () => {
		it("extracts the first user message as the current task goal", () => {
			const messages: AgentMessage[] = [
				makeUserMessage("Fix the bug in auth-broker where tokens expire prematurely"),
				makeAssistantMessage("I will inspect auth-broker.ts"),
				makeUserMessage("Also make sure tests pass"),
			];
			const goal = extractTaskGoal(messages);
			expect(goal).toContain("Fix the bug in auth-broker");
		});

		it("scores a changed task against the latest request, keeping the original as context", async () => {
			const mockDecide = vi.fn(async (_state: Record<string, string>) => ({
				success: true,
				fallback: false,
				latencyMs: 5,
				data: { tool_0_1_bash: { type: "score", score: 2, confidence: 0.9 } },
			}));
			const mockClient = {
				decide: mockDecide,
			} as unknown as LayaClient;

			await pruneContextWithLaya(
				[
					makeUserMessage("Migrate the billing database schema"),
					makeToolResultMessage("c0", "bash", "MIGRATION_LOG_".repeat(100)),
					makeUserMessage("Now fix the login page styling instead"),
					makeUserMessage("Styling follow-up"),
				],
				{
					client: mockClient,
					keepRecentTurns: 1,
					minKeptTurns: 1,
					prunableTokenBudget: 10,
					sessionId: "goal-change-test",
				},
			);

			expect(mockDecide).toHaveBeenCalledTimes(1);
			const state = mockDecide.mock.calls[0][0] as Record<string, string>;
			const scoredText = Object.values(state)[0] ?? "";
			expect(scoredText).toContain("Current request: Styling follow-up");
			expect(scoredText).toContain("Original task: Migrate the billing database schema");
			resetLockedPruningDecisions("goal-change-test");
		});

		it("partitions messages into interaction turns based on user boundaries", () => {
			const messages: AgentMessage[] = [
				makeUserMessage("Turn 0 user"),
				makeAssistantMessage("Turn 0 assistant"),
				makeToolResultMessage("c1", "bash", "Turn 0 tool"),
				makeUserMessage("Turn 1 user"),
				makeAssistantMessage("Turn 1 assistant"),
				makeUserMessage("Turn 2 user"),
				makeAssistantMessage("Turn 2 assistant"),
			];
			const turns = partitionMessagesIntoTurns(messages);
			expect(turns.length).toBe(3);
			expect(turns[0].messageIndices).toEqual([0, 1, 2]);
			expect(turns[1].messageIndices).toEqual([3, 4]);
			expect(turns[2].messageIndices).toEqual([5, 6]);
		});
	});

	describe("Always-keep set and candidate filtering (Step 1)", () => {
		it("bypasses scoring and never prunes if total turns <= keepRecentTurns", async () => {
			const mockDecide = vi.fn();
			const mockClient = {
				decide: mockDecide,
			} as unknown as LayaClient;

			const messages: AgentMessage[] = [
				makeUserMessage("Turn 0 user"),
				makeAssistantMessage("Turn 0 assistant"),
				makeUserMessage("Turn 1 user"),
				makeAssistantMessage("Turn 1 assistant"),
			];

			const result = await pruneContextWithLaya(messages, {
				client: mockClient,
				keepRecentTurns: 2,
			});

			expect(mockDecide).not.toHaveBeenCalled();
			expect(result.pruned).toBe(false);
			expect(result.messages.length).toBe(messages.length);
		});

		it("skips scoring entirely when all candidate tokens fit the budget", async () => {
			const mockDecide = vi.fn();
			const mockClient = {
				decide: mockDecide,
			} as unknown as LayaClient;

			// 3 turns with two ~100-token tool results in prunable turns.
			// Total (~200 tokens) fits the 1500-token safety floor, so no
			// inference round trip can drop anything.
			const messages: AgentMessage[] = [
				makeUserMessage("Turn 0 user"),
				makeToolResultMessage("c0", "read", "evidence ".repeat(45)),
				makeUserMessage("Turn 1 user"),
				makeToolResultMessage("c1", "read", "evidence ".repeat(45)),
				makeUserMessage("Turn 2 user"),
			];

			const result = await pruneContextWithLaya(messages, {
				client: mockClient,
				keepRecentTurns: 1,
				sessionId: "budget-bypass-test",
			});

			expect(mockDecide).not.toHaveBeenCalled();
			expect(result.pruned).toBe(false);
			expect(result.droppedCount).toBe(0);
			expect(result.tokensSaved).toBe(0);
			expect(result.messages).toEqual(messages);
		});

		it("chunks scoring across server batch limits and merges the scores", async () => {
			const batchSizes: number[] = [];
			const mockDecide = vi.fn(async (_state: unknown, questions: Record<string, unknown>) => {
				batchSizes.push(Object.keys(questions).length);
				return {
					success: true,
					fallback: false,
					latencyMs: 5,
					data: Object.fromEntries(
						Object.keys(questions).map((key, index) => [
							key,
							{ type: "score", score: index % 4, confidence: 0.9 },
						]),
					),
				};
			});
			const mockClient = {
				decide: mockDecide,
			} as unknown as LayaClient;

			// 70 prunable tool results: one 64-question batch plus a 6-question tail.
			const messages: AgentMessage[] = [];
			for (let turn = 0; turn < 71; turn++) {
				messages.push(makeUserMessage(`Turn ${turn} user`));
				if (turn < 70) messages.push(makeToolResultMessage(`c${turn}`, "read", "evidence ".repeat(45)));
			}

			const result = await pruneContextWithLaya(messages, {
				client: mockClient,
				keepRecentTurns: 1,
				sessionId: "chunk-test",
			});

			expect(mockDecide).toHaveBeenCalledTimes(2);
			expect(batchSizes).toEqual([64, 6]);
			expect(result.droppedCount).toBeGreaterThan(0);
			expect(result.tokensSaved).toBeGreaterThan(0);
		});

		it("always keeps user messages, pinned messages, and recent N turns", async () => {
			// 4 turns:
			// Turn 0: User (goal), Assistant, Tool Result (large, prunable)
			// Turn 1: User (pinned), Tool Result (large, prunable)
			// Turn 2: User (recent turn 2 - kept)
			// Turn 3: User (recent turn 3 - kept), Tool Result (kept)
			const largeOldToolOutput = "LOG_DATA_".repeat(100); // ~900 chars (~225 tokens)
			const recentToolOutput = "RECENT_DATA_".repeat(100);

			const messages: AgentMessage[] = [
				makeUserMessage("Task: Refactor database client"),
				makeAssistantMessage("Running initial check"),
				makeToolResultMessage("c0", "read_file", largeOldToolOutput),
				makeUserMessage("Check this pinned config", true),
				makeToolResultMessage("c1", "read_file", largeOldToolOutput),
				makeUserMessage("Continuing refactor turn 2"),
				makeAssistantMessage("Working on turn 2"),
				makeUserMessage("Latest turn 3"),
				makeToolResultMessage("c3", "bash", recentToolOutput),
			];

			const mockDecide = vi.fn().mockResolvedValue({
				success: true,
				fallback: false,
				latencyMs: 15,
				data: {
					tool_0_2_read_file: { score: 0.1, confidence: 0.9 },
					tool_1_4_read_file: { score: 2.8, confidence: 0.95 },
				},
			});

			const mockClient = {
				decide: mockDecide,
			} as unknown as LayaClient;

			const result = await pruneContextWithLaya(messages, {
				client: mockClient,
				keepRecentTurns: 2, // Turns 2 and 3 are always kept
				minKeptTurns: 1,
				prunableTokenBudget: 250, // Budget fits exactly one 225-token chunk
			});

			// Verify Step 3: Exactly ONE batch decide call was made
			expect(mockDecide).toHaveBeenCalledTimes(1);
			const [_stateArg, questionsArg] = mockDecide.mock.calls[0];

			// Only candidate chunks from older turns (Turn 0 and Turn 1) were scored
			expect(Object.keys(questionsArg)).toContain("tool_0_2_read_file");
			expect(Object.keys(questionsArg)).toContain("tool_1_4_read_file");
			// Recent tool result from Turn 3 was NEVER sent to Laya
			expect(Object.keys(questionsArg)).not.toContain("tool_3_8_bash");

			// Verify Step 4: Low relevance chunk was dropped and replaced with placeholder
			const prunedTool0 = result.messages[2] as ToolResultMessage;
			const text0 = extractMessageText(prunedTool0);
			expect(text0).toContain("[Earlier tool output for 'read_file' omitted for relevance");
			expect(prunedTool0.prunedAt).toBeDefined();

			// High relevance chunk in Turn 1 was kept in FULL original content
			const keptTool1 = result.messages[4] as ToolResultMessage;
			const text1 = extractMessageText(keptTool1);
			expect(text1).toBe(largeOldToolOutput);

			// Recent Turn 3 tool was kept in full
			const keptTool3 = result.messages[8] as ToolResultMessage;
			expect(extractMessageText(keptTool3)).toBe(recentToolOutput);

			// User messages are never pruned
			expect(extractMessageText(result.messages[0])).toContain("Task: Refactor database client");
			expect(extractMessageText(result.messages[3])).toContain("Check this pinned config");
		});
	});

	describe("Budget-based selection and transparent placeholders (Step 4)", () => {
		it("replaces dropped chunks with informative placeholders and keeps full content for kept chunks", async () => {
			const messages: AgentMessage[] = [
				makeUserMessage("Turn 0: Start build process"),
				makeToolResultMessage("c0", "bash", "BUILD_LOG_FAIL_".repeat(80)),
				makeUserMessage("Turn 1: Fix build config"),
				makeToolResultMessage("c1", "write_file", "UPDATED_CONFIG_".repeat(80)),
				makeUserMessage("Turn 2: Re-run build"),
				makeAssistantMessage("Re-running"),
				makeUserMessage("Turn 3: Check status"),
				makeAssistantMessage("Done"),
			];

			const mockClient = {
				decide: async () => ({
					success: true,
					fallback: false,
					latencyMs: 20,
					data: {
						tool_0_1_bash: { score: 0.2, confidence: 0.8 },
						tool_1_3_write_file: { score: 2.7, confidence: 0.9 },
					},
				}),
			} as unknown as LayaClient;

			const result = await pruneContextWithLaya(messages, {
				client: mockClient,
				keepRecentTurns: 2,
				minKeptTurns: 1,
				prunableTokenBudget: 350, // Fits one 320-token chunk, drops the other
			});

			expect(result.pruned).toBe(true);
			expect(result.droppedCount).toBe(1);
			expect(result.tokensSaved).toBeGreaterThan(0);

			// Dropped chunk has visible placeholder indicating tool name and score
			const dropped = result.messages[1] as ToolResultMessage;
			expect(extractMessageText(dropped)).toMatch(
				/\[Earlier tool output for 'bash' omitted for relevance \(Laya score: 0\.07\)]/,
			);

			// Kept chunk is completely intact
			const kept = result.messages[3] as ToolResultMessage;
			expect(extractMessageText(kept)).toBe("UPDATED_CONFIG_".repeat(80));
		});
	});

	describe("Safety floor (Step 5)", () => {
		it("never prunes turns within minKeptTurns even if all candidates score 0.0", async () => {
			const messages: AgentMessage[] = [
				makeUserMessage("Turn 0: ancient task"),
				makeToolResultMessage("c0", "bash", "ANCIENT_LOG_".repeat(50)),
				makeUserMessage("Turn 1: middle turn"),
				makeToolResultMessage("c1", "bash", "MIDDLE_LOG_".repeat(50)),
				makeUserMessage("Turn 2: safety floor turn"),
				makeToolResultMessage("c2", "bash", "SAFETY_FLOOR_LOG_".repeat(50)),
				makeUserMessage("Turn 3: recent turn"),
				makeAssistantMessage("Assistant turn 3"),
				makeUserMessage("Turn 4: most recent turn"),
				makeAssistantMessage("Assistant turn 4"),
			];

			// All candidates score zero
			const mockClient = {
				decide: async () => ({
					success: true,
					fallback: false,
					latencyMs: 10,
					data: {
						tool_0_1_bash: { score: 0.0, confidence: 0.9 },
						tool_1_3_bash: { score: 0.0, confidence: 0.9 },
						tool_2_5_bash: { score: 0.0, confidence: 0.9 },
					},
				}),
			} as unknown as LayaClient;

			const result = await pruneContextWithLaya(messages, {
				client: mockClient,
				keepRecentTurns: 2, // Turns 3 & 4
				minKeptTurns: 3, // Turns 2, 3 & 4 are protected by safety floor
				prunableTokenBudget: 0, // Budget 0 would drop everything if not for safety floor
			});

			// Turn 2 is protected by minKeptTurns=3 (total turns = 5; threshold = 5 - 3 = 2)
			const turn2Tool = result.messages[5] as ToolResultMessage;
			expect(extractMessageText(turn2Tool)).toBe("SAFETY_FLOOR_LOG_".repeat(50));
			expect(turn2Tool.prunedAt).toBeUndefined();

			// Older turns (0 and 1) were dropped
			const turn0Tool = result.messages[1] as ToolResultMessage;
			expect(extractMessageText(turn0Tool)).toContain("omitted for relevance");
		});
	});

	describe("Fail-open fallback (Step 6)", () => {
		it("fails OPEN and returns full unpruned context if sidecar errors or times out", async () => {
			// The tool output must exceed the pruning budget floor so scoring
			// is actually attempted (smaller sessions bypass inference).
			const criticalLog = "CRITICAL_SETUP_LOG_".repeat(400);
			const messages: AgentMessage[] = [
				makeUserMessage("Turn 0: Setup project"),
				makeToolResultMessage("c0", "bash", criticalLog),
				makeUserMessage("Turn 1: Run tests"),
				makeAssistantMessage("Tests running"),
				makeUserMessage("Turn 2: Check output"),
				makeAssistantMessage("Done"),
			];

			// Mock sidecar failure (timeout or network error)
			const mockClient = {
				decide: async () => ({
					success: false,
					fallback: true,
					fallbackReason: "sidecar_connection_refused",
					latencyMs: 300,
					data: null,
				}),
			} as unknown as LayaClient;

			const result = await pruneContextWithLaya(messages, {
				client: mockClient,
				keepRecentTurns: 1,
			});

			expect(result.fallback).toBe(true);
			expect(result.pruned).toBe(false);
			expect(result.droppedCount).toBe(0);
			expect(result.tokensSaved).toBe(0);
			// Full original context is completely preserved
			expect(result.messages.length).toBe(messages.length);
			expect(extractMessageText(result.messages[1])).toBe(criticalLog);
		});
	});

	describe("Telemetry and calibration logging (Step 7)", () => {
		it("records audit log entries with relevance scores and actions", async () => {
			const initialLogCount = PRUNING_AUDIT_LOG.length;

			const messages: AgentMessage[] = [
				makeUserMessage("Turn 0: Inspect auth module"),
				makeToolResultMessage("c0", "grep", "FIND_AUTH_TOKENS_".repeat(40)),
				makeUserMessage("Turn 1: Edit token validator"),
				makeAssistantMessage("Updated"),
				makeUserMessage("Turn 2: Final verification"),
				makeAssistantMessage("Done"),
			];

			const mockClient = {
				decide: async () => ({
					success: true,
					fallback: false,
					latencyMs: 12,
					data: {
						tool_0_1_grep: { score: 1.8, confidence: 0.88 },
					},
				}),
			} as unknown as LayaClient;

			await pruneContextWithLaya(messages, {
				client: mockClient,
				keepRecentTurns: 1,
				minKeptTurns: 1,
				sessionId: "test-session-123",
				prunableTokenBudget: 10, // Force drop
			});

			expect(PRUNING_AUDIT_LOG.length).toBeGreaterThan(initialLogCount);
			const latestRecord = PRUNING_AUDIT_LOG[PRUNING_AUDIT_LOG.length - 1];
			expect(latestRecord.sessionId).toBe("test-session-123");
			expect(latestRecord.toolName).toBe("grep");
			expect(latestRecord.score).toBe(1.8);
			expect(latestRecord.action).toBe("dropped");
		});
	});

	describe("Live Sidecar Integration (Step 8)", () => {
		it("communicates with live Laya daemon if running on port 8177", async () => {
			const { getLayaClient } = await import("../src/core/harvest/laya-client");
			const liveClient = getLayaClient();
			const isOnline = await liveClient.isHealthy();
			if (!isOnline) {
				console.log("Live sidecar not running on port 8177 - skipping live probe");
				return;
			}

			const messages: AgentMessage[] = [
				makeUserMessage("Task: Fix race condition in connection pool"),
				makeAssistantMessage("I will start by reviewing the connection pool logs"),
				makeToolResultMessage("c0", "read_file", "IRRELEVANT_CSS_STYLES { color: red; } ".repeat(50)),
				makeUserMessage("Turn 1: Check pool lock mechanism"),
				makeToolResultMessage("c1", "grep", "ACQUIRE_LOCK mutex.lock() connection_pool.acquire()".repeat(30)),
				makeUserMessage("Turn 2: Most recent turn"),
				makeAssistantMessage("Proceeding with patch"),
			];

			const result = await pruneContextWithLaya(messages, {
				client: liveClient,
				keepRecentTurns: 1,
				minKeptTurns: 1,
				prunableTokenBudget: 150,
				timeoutMs: 60000,
			});

			expect(result.fallback).toBe(false);
			expect(result.candidatesCount).toBe(2);
			expect(result.latencyMs).toBeGreaterThan(0);
			expect(result.messages.length).toBe(messages.length);
		}, 70000);
	});

	describe("Prompt-cache prefix stability & decision locking (Phase 4 Step 4)", () => {
		it("produces byte-identical prefixes for already-committed chunks across subsequent turns", async () => {
			const mockDecide = vi.fn(async (_state, questions) => {
				const answers: Record<string, any> = {};
				for (const qid of Object.keys(questions)) {
					// Low relevance for compiler trace, high for pool config
					answers[qid] = qid.includes("bash") ? { score: 0.1, confidence: 0.9 } : { score: 2.8, confidence: 0.95 };
				}
				return { success: true, fallback: false, latencyMs: 10, data: answers };
			});

			const mockClient = {
				decide: mockDecide,
			} as unknown as LayaClient;

			const testSessionId = `test-prefix-stability-${Date.now()}`;

			// Turn 1 messages
			const turn1 = [
				makeUserMessage("Goal: Fix pool connection bug"),
				makeAssistantMessage("Checking compiler error"),
				makeToolResultMessage("c0", "bash", "ERROR_TRACE_".repeat(80)), // large prunable chunk
			];

			// Turn 2 messages
			const turn2 = [
				makeUserMessage("Turn 2: View pool interface"),
				makeAssistantMessage("Reading pool.ts"),
				makeToolResultMessage("c1", "read_file", "POOL_INTERFACE_CONFIG_".repeat(60)),
			];

			// Turn 3 messages
			const turn3 = [makeUserMessage("Turn 3: Fix syntax in pool.ts"), makeAssistantMessage("Applying fix")];

			// Turn 4 messages
			const turn4 = [
				makeUserMessage("Turn 4: Run unit tests"),
				makeAssistantMessage("Running test suite"),
				makeToolResultMessage("c2", "bash", "PASS test/pool.test.ts\n"),
			];

			// Execute Turn 3 (Turn 1 chunk ages out of keepRecentTurns=1 and is evaluated/locked)
			const sessionHistoryT3 = [...turn1, ...turn2, ...turn3];
			const testSettings = {
				get: (k: string) => (k === "laya.enabled" || k === "laya.pruning" ? true : undefined),
			} as any;

			const resultT3 = await pruneContextWithLaya(sessionHistoryT3, {
				client: mockClient,
				settings: testSettings,
				sessionId: testSessionId,
				keepRecentTurns: 1,
				minKeptTurns: 1,
				prunableTokenBudget: 200,
			});

			expect(resultT3.pruned).toBe(true);
			expect(mockDecide).toHaveBeenCalledTimes(1);

			// Serialized wire representation of Turn 3's pruned messages
			const wireT3 = resultT3.messages.map(m => ({
				role: m.role,
				content: extractMessageText(m),
			}));

			// Execute Turn 4 (adds new turn, Turn 2 chunk ages out, but Turn 1 chunk was already committed)
			const sessionHistoryT4 = [...turn1, ...turn2, ...turn3, ...turn4];
			const resultT4 = await pruneContextWithLaya(sessionHistoryT4, {
				client: mockClient,
				settings: testSettings,
				sessionId: testSessionId,
				keepRecentTurns: 1,
				minKeptTurns: 1,
				prunableTokenBudget: 200,
			});

			const wireT4 = resultT4.messages.map(m => ({
				role: m.role,
				content: extractMessageText(m),
			}));

			// Turn 1's tool result was dropped at index 2
			expect(wireT3[2].content).toContain("omitted for relevance");
			// Assert that Turn 4 retains the EXACT byte-identical placeholder for message index 2
			expect(wireT4[2].content).toBe(wireT3[2].content);

			// Assert that the entire prefix corresponding to Turn 3's message count is byte-identical
			for (let i = 0; i < resultT3.messages.length; i++) {
				expect(wireT4[i].role).toBe(wireT3[i].role);
				expect(wireT4[i].content).toBe(wireT3[i].content);
			}

			// Clean up test session locks
			resetLockedPruningDecisions(testSessionId);
		});

		it("scores afresh when a reused position holds different content (rewind/branch)", async () => {
			const sessionId = `test-rewind-rescore-${Date.now()}`;
			try {
				const seenStates: string[] = [];
				const mockDecide = vi.fn(async (state: unknown) => {
					seenStates.push(JSON.stringify(state));
					return {
						success: true,
						fallback: false,
						latencyMs: 5,
						data: { tool_0_1_bash: { type: "score", score: 0, confidence: 0.9 } },
					};
				});
				const mockClient = {
					decide: mockDecide,
				} as unknown as LayaClient;
				const testSettings = {
					get: (k: string) => (k === "laya.enabled" || k === "laya.pruning" ? true : undefined),
				} as any;

				// First pass locks a drop for the chunk at index 1.
				const first = [
					makeUserMessage("Goal: inspect logs"),
					makeToolResultMessage("c0", "bash", "ALPHA_LOG_".repeat(100)),
					makeUserMessage("Turn 1"),
					makeUserMessage("Turn 2"),
				];
				const result1 = await pruneContextWithLaya(first, {
					client: mockClient,
					settings: testSettings,
					sessionId,
					keepRecentTurns: 1,
					minKeptTurns: 1,
					prunableTokenBudget: 10,
				});
				expect(mockDecide).toHaveBeenCalledTimes(1);
				expect(result1.droppedCount).toBe(1);

				// Rewind/branch reuses the same position for different content.
				const second = [
					makeUserMessage("Goal: inspect logs"),
					makeToolResultMessage("c0", "bash", "BETA_LOG_".repeat(100)),
					makeUserMessage("Turn 1"),
					makeUserMessage("Turn 2"),
				];
				const result2 = await pruneContextWithLaya(second, {
					client: mockClient,
					settings: testSettings,
					sessionId,
					keepRecentTurns: 1,
					minKeptTurns: 1,
					prunableTokenBudget: 10,
				});

				// The stale positional lock must not replay: new content is scored afresh.
				expect(mockDecide).toHaveBeenCalledTimes(2);
				expect(seenStates[1]).toContain("BETA_LOG_");
				expect(seenStates[1]).not.toContain("ALPHA_LOG_");
				expect(result2.droppedCount).toBe(1);
			} finally {
				resetLockedPruningDecisions(sessionId);
			}
		});

		it("bounds the global lock registry, evicting oldest entries first", async () => {			const store = new Map();
			for (let i = 0; i < 1005; i++) store.set(`stale::k${i}`, { chunkId: `k${i}`, action: "kept" });
			const mockDecide = vi.fn(async (_state: unknown, questions: Record<string, unknown>) => ({
				success: true,
				fallback: false,
				latencyMs: 5,
				data: Object.fromEntries(
					Object.keys(questions).map(key => [key, { type: "score", score: 3, confidence: 0.9 }]),
				),
			}));
			const mockClient = {
				decide: mockDecide,
			} as unknown as LayaClient;

			await pruneContextWithLaya(
				[
					makeUserMessage("Goal: bound check"),
					makeToolResultMessage("c0", "bash", "BOUND_LOG_".repeat(100)),
					makeUserMessage("Turn 1"),
					makeUserMessage("Turn 2"),
				],
				{
					client: mockClient,
					sessionId: "bound-test",
					lockedDecisions: store as any,
					keepRecentTurns: 1,
					minKeptTurns: 1,
					prunableTokenBudget: 10,
				},
			);

			expect(store.size).toBeLessThanOrEqual(1000);
			expect([...store.keys()].some(key => key.startsWith("bound-test::tool_0_1_bash::"))).toBe(true);
		});

		it("honors calibrated auto-disable when pruning is unset, explicit enable still scores", async () => {
			const slowRecord = {
				timestamp: Date.now(),
				hardware: {
					tier: "cpu",
					device: "cpu",
					device_name: "slow",
					reason: "slow",
					signature: "slow-sig",
					details: {},
				},
				benchmarks: {
					ultraShort: { medianMs: 5, minMs: 4, samplesMs: [5] },
					singleChoice: { medianMs: 5000, minMs: 4500, samplesMs: [5000] },
					singleScore: { medianMs: 2000, minMs: 1800, samplesMs: [2000] },
					batchedScore: { medianMs: 9000, minMs: 8000, samplesMs: [9000] },
				},
				derivedSettings: {
					rawSingleChoiceLatencyMs: 5000,
					subagentSelectionTimeoutMs: 10000,
					subagentSelectionRecommendEnabled: false,
					subagentSelectionReason: "slow",
					pruningRecommendEnabled: false,
					pruningReason: "slow hardware",
					maxAcceptableLatencyPerTurnMs: 150,
					estimatedAddedLatencyPerTurnMs: 5000,
					worstCaseBatchLatencyMs: 9000,
				},
			} as unknown as CalibrationRecord;
			setMemoryCachedCalibration(slowRecord);
			try {
				const messages: AgentMessage[] = [
					makeUserMessage("Goal: inspect logs"),
					makeToolResultMessage("c0", "bash", "AUTO_LOG_".repeat(100)),
					makeUserMessage("Turn 1"),
					makeUserMessage("Turn 2"),
				];
				const mockDecide = vi.fn(async (_state: unknown, questions: Record<string, unknown>) => ({
					success: true,
					fallback: false,
					latencyMs: 5,
					data: Object.fromEntries(
						Object.keys(questions).map(key => [key, { type: "score", score: 3, confidence: 0.9 }]),
					),
				}));
				const mockClient = {
					decide: mockDecide,
				} as unknown as LayaClient;

				// Unset in settings: the calibrated recommendation disables pruning.
				const autoResult = await pruneContextWithLaya(messages, {
					client: mockClient,
					settings: Settings.isolated({ "laya.enabled": true }),
					sessionId: "auto-pruning-test",
					keepRecentTurns: 1,
					minKeptTurns: 1,
					prunableTokenBudget: 10,
				});
				expect(mockDecide).not.toHaveBeenCalled();
				expect(autoResult.pruned).toBe(false);
				expect(autoResult.fallbackReason).toBe("laya_pruning_disabled_by_hardware_calibration");

				// Explicitly enabled: calibration advises otherwise, but the user override scores.
				const explicitResult = await pruneContextWithLaya(messages, {
					client: mockClient,
					settings: Settings.isolated({ "laya.enabled": true, "laya.pruning": true }),
					sessionId: "auto-pruning-test",
					keepRecentTurns: 1,
					minKeptTurns: 1,
					prunableTokenBudget: 10,
				});
				expect(mockDecide).toHaveBeenCalledTimes(1);
				expect(explicitResult.candidatesCount).toBe(1);
			} finally {
				setMemoryCachedCalibration(null);
				resetLockedPruningDecisions("auto-pruning-test");
			}
		});
	});

	describe("History-rewrite invalidation P0-3 and prompt-revision re-key", () => {
		it("embeds content hash and prompt revision in the lock key", () => {
			const a = pruningLockKey("s", "tool_0_1_bash", "ALPHA");
			const b = pruningLockKey("s", "tool_0_1_bash", "BETA");
			const c = pruningLockKey("s", "tool_0_1_bash", "ALPHA", "rev-other");
			expect(a).toContain(PRUNING_PROMPT_REVISION);
			expect(a).not.toBe(b);
			expect(a).not.toBe(c);
		});

		it("rescores identical content after a rewind/compact/restore invalidation instead of replaying stale drops", async () => {
			const sessionId = `test-history-rewrite-${Date.now()}`;
			try {
				const mockDecide = vi.fn(async (_state: unknown, questions: Record<string, unknown>) => ({
					success: true,
					fallback: false,
					latencyMs: 5,
					data: Object.fromEntries(
						Object.keys(questions).map(key => [key, { type: "score", score: 0, confidence: 0.9 }]),
					),
				}));
				const mockClient = { decide: mockDecide } as unknown as LayaClient;
				const testSettings = {
					get: (k: string) => (k === "laya.enabled" || k === "laya.pruning" ? true : undefined),
				} as unknown as Settings;
				const messages: AgentMessage[] = [
					makeUserMessage("Goal: inspect logs"),
					makeToolResultMessage("c0", "bash", "STALE_LOG_".repeat(100)),
					makeUserMessage("Turn 1"),
					makeUserMessage("Turn 2"),
				];
				const opts = {
					client: mockClient,
					settings: testSettings,
					sessionId,
					keepRecentTurns: 1,
					minKeptTurns: 1,
					prunableTokenBudget: 10,
				};
				const first = await pruneContextWithLaya(messages, opts);
				expect(first.droppedCount).toBe(1);
				expect(mockDecide).toHaveBeenCalledTimes(1);

				// Same transcript, no rewrite: locked decision replays without rescoring.
				const second = await pruneContextWithLaya(messages, opts);
				expect(mockDecide).toHaveBeenCalledTimes(1);
				expect(second.droppedCount).toBe(1);

				// Rewind/compact/restore clears the session's locks: identical
				// content in a rewritten transcript is scored afresh.
				invalidatePruningLocksOnHistoryRewrite(sessionId);
				const third = await pruneContextWithLaya(messages, opts);
				expect(mockDecide).toHaveBeenCalledTimes(2);
				expect(third.droppedCount).toBe(1);
			} finally {
				resetLockedPruningDecisions(sessionId);
			}
		});

		it("rescores identical content when the prompt revision changes", async () => {
			const sessionId = `test-prompt-rev-${Date.now()}`;
			try {
				const mockDecide = vi.fn(async (_state: unknown, questions: Record<string, unknown>) => ({
					success: true,
					fallback: false,
					latencyMs: 5,
					data: Object.fromEntries(
						Object.keys(questions).map(key => [key, { type: "score", score: 3, confidence: 0.9 }]),
					),
				}));
				const mockClient = { decide: mockDecide } as unknown as LayaClient;
				const testSettings = {
					get: (k: string) => (k === "laya.enabled" || k === "laya.pruning" ? true : undefined),
				} as unknown as Settings;
				const messages: AgentMessage[] = [
					makeUserMessage("Goal: inspect logs"),
					makeToolResultMessage("c0", "bash", "PROMPT_REV_LOG_".repeat(100)),
					makeUserMessage("Turn 1"),
					makeUserMessage("Turn 2"),
				];
				const base = {
					client: mockClient,
					settings: testSettings,
					sessionId,
					keepRecentTurns: 1,
					minKeptTurns: 1,
					prunableTokenBudget: 10,
				};
				await pruneContextWithLaya(messages, { ...base, promptRevision: "rev-a" });
				expect(mockDecide).toHaveBeenCalledTimes(1);
				// Same revision replays from the lock.
				await pruneContextWithLaya(messages, { ...base, promptRevision: "rev-a" });
				expect(mockDecide).toHaveBeenCalledTimes(1);
				// New prompt revision misses and scores afresh.
				await pruneContextWithLaya(messages, { ...base, promptRevision: "rev-b" });
				expect(mockDecide).toHaveBeenCalledTimes(2);
			} finally {
				resetLockedPruningDecisions(sessionId);
			}
		});

		it("fails open without scoring when the shared prune+rerank budget is exhausted", async () => {
			const mockDecide = vi.fn();
			const mockClient = { decide: mockDecide } as unknown as LayaClient;
			const testSettings = {
				get: (k: string) => (k === "laya.enabled" || k === "laya.pruning" ? true : undefined),
			} as unknown as Settings;
			const messages: AgentMessage[] = [
				makeUserMessage("Goal: inspect logs"),
				makeToolResultMessage("c0", "bash", "BUDGET_LOG_".repeat(100)),
				makeUserMessage("Turn 1"),
				makeUserMessage("Turn 2"),
			];
			const exhausted = { totalMs: 600, deadline: 0, remainingMs: () => 0 };
			const result = await pruneContextWithLaya(messages, {
				client: mockClient,
				settings: testSettings,
				sessionId: `test-budget-${Date.now()}`,
				keepRecentTurns: 1,
				minKeptTurns: 1,
				prunableTokenBudget: 10,
				budget: exhausted,
			});
			expect(mockDecide).not.toHaveBeenCalled();
			expect(result.fallback).toBe(true);
			expect(result.fallbackReason).toBe("shared_budget_exhausted");
			expect(result.messages).toEqual(messages);
		});
	});
});
