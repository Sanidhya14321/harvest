import { afterEach, describe, expect, it, vi } from "bun:test";
import type { AgentMessage, AssistantMessage, ToolResultMessage, UserMessage } from "@harvest/pi-ai";
import {
	createScoringExcerpt,
	estimateTextTokens,
	extractMessageText,
	extractTaskGoal,
	partitionMessagesIntoTurns,
	PRUNING_AUDIT_LOG,
	pruneContextWithLaya,
	resetLockedPruningDecisions,
} from "../src/core/harvest/laya-pruning";
import type { LayaClient } from "../src/core/harvest/laya-client";

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
			const mockClient: LayaClient = {
				isAvailable: async () => true,
				decide: mockDecide,
				getHealth: async () => ({ status: "ok", device: "cpu" }),
			};

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
				fallback: false,
				latencyMs: 15,
				data: {
					tool_0_2_read_file: { score: 0.1, confidence: 0.9 },
					tool_1_4_read_file: { score: 2.8, confidence: 0.95 },
				},
			} as LayaDecideResult);

			const mockClient: LayaClient = {
				isAvailable: async () => true,
				decide: mockDecide,
				getHealth: async () => ({ status: "ok", device: "cpu" }),
			};

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

			const mockClient: LayaClient = {
				isAvailable: async () => true,
				decide: async () => ({
					fallback: false,
					latencyMs: 20,
					data: {
						tool_0_1_bash: { score: 0.2, confidence: 0.8 },
						tool_1_3_write_file: { score: 2.7, confidence: 0.9 },
					},
				}),
				getHealth: async () => ({ status: "ok", device: "cpu" }),
			};

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
			const mockClient: LayaClient = {
				isAvailable: async () => true,
				decide: async () => ({
					fallback: false,
					latencyMs: 10,
					data: {
						tool_0_1_bash: { score: 0.0, confidence: 0.9 },
						tool_1_3_bash: { score: 0.0, confidence: 0.9 },
						tool_2_5_bash: { score: 0.0, confidence: 0.9 },
					},
				}),
				getHealth: async () => ({ status: "ok", device: "cpu" }),
			};

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
			const messages: AgentMessage[] = [
				makeUserMessage("Turn 0: Setup project"),
				makeToolResultMessage("c0", "bash", "CRITICAL_SETUP_LOG_".repeat(50)),
				makeUserMessage("Turn 1: Run tests"),
				makeAssistantMessage("Tests running"),
				makeUserMessage("Turn 2: Check output"),
				makeAssistantMessage("Done"),
			];

			// Mock sidecar failure (timeout or network error)
			const mockClient: LayaClient = {
				isAvailable: async () => false,
				decide: async () => ({
					fallback: true,
					fallbackReason: "sidecar_connection_refused",
					latencyMs: 300,
					data: null,
				}),
				getHealth: async () => ({ status: "error", error: "Connection refused" }),
			};

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
			expect(extractMessageText(result.messages[1])).toBe("CRITICAL_SETUP_LOG_".repeat(50));
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

			const mockClient: LayaClient = {
				isAvailable: async () => true,
				decide: async () => ({
					fallback: false,
					latencyMs: 12,
					data: {
						tool_0_1_grep: { score: 1.8, confidence: 0.88 },
					},
				}),
				getHealth: async () => ({ status: "ok", device: "cpu" }),
			};

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
				return { fallback: false, latencyMs: 10, data: answers };
			});

			const mockClient: LayaClient = {
				isAvailable: async () => true,
				decide: mockDecide as any,
				getHealth: async () => ({ status: "ok", device: "cpu" }),
			};

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
	});
});
