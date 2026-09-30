import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "bun:test";
import { Agent, type AgentTool } from "@harvest/pi-agent-core";
import { type } from "@harvest/omptype";
import { createMockModel, type MockModel } from "@harvest/pi-ai/providers/mock";
import { ModelRegistry } from "@harvest/pi-coding-agent/config/model-registry";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import { AgentSession, type AgentSessionEvent } from "@harvest/pi-coding-agent/session/agent-session";
import { SessionManager } from "@harvest/pi-coding-agent/session/session-manager";
import { createInMemoryAuthStorage } from "../helpers/agent-session-setup";
import type { AuthStorage } from "@harvest/pi-coding-agent/session/auth-storage";

/**
 * Dual-session concurrency contract: two live AgentSession runtimes sharing
 * one process (and one model registry, as production does) prompt, stream,
 * execute tools, and abort independently. Background work in one must never
 * stop, reroute, or leak into the other. This is the headless prerequisite
 * for live tabs — the TUI retargeting layer builds on top of it.
 */
describe("concurrent AgentSession prompts", () => {
	let authStorage: AuthStorage;
	let modelRegistry: ModelRegistry;
	const sessions: AgentSession[] = [];

	beforeAll(() => {
		authStorage = createInMemoryAuthStorage();
		authStorage.setRuntimeApiKey("mock", "test-key");
		modelRegistry = new ModelRegistry(authStorage);
	});

	afterAll(() => {
		authStorage.close();
	});

	afterEach(async () => {
		vi.restoreAllMocks();
		for (const session of sessions.splice(0)) {
			await session.dispose();
		}
	});

	function makeSession(mock: MockModel, tools: AgentTool[] = []): AgentSession {
		const agent = new Agent({
			getApiKey: () => "test-key",
			initialState: { model: mock.model, systemPrompt: [], tools, messages: [] },
			streamFn: mock.stream,
		});
		const session = new AgentSession({
			agent,
			sessionManager: SessionManager.inMemory(),
			settings: Settings.isolated({ "compaction.enabled": false, "tools.approvalMode": "yolo" }),
			modelRegistry,
			toolRegistry: new Map(tools.map(tool => [tool.name, tool])),
		});
		sessions.push(session);
		return session;
	}

	function transcriptText(session: AgentSession): string {
		return JSON.stringify(session.agent.state.messages);
	}

	async function waitForStreaming(session: AgentSession): Promise<void> {
		for (let waited = 0; waited < 5000; waited += 5) {
			if (session.isStreaming) return;
			await Bun.sleep(5);
		}
		throw new Error("Session did not start streaming");
	}

	it("prompts both sessions concurrently without cross-talk", async () => {
		vi.spyOn(modelRegistry, "getApiKey").mockResolvedValue("test-key");
		const mockA = createMockModel({ responses: [{ content: ["answer A"] }] });
		const mockB = createMockModel({ responses: [{ content: ["answer B"] }] });
		const sessionA = makeSession(mockA);
		const sessionB = makeSession(mockB);
		const eventsA: AgentSessionEvent[] = [];
		const eventsB: AgentSessionEvent[] = [];
		sessionA.subscribe(event => eventsA.push(event));
		sessionB.subscribe(event => eventsB.push(event));

		await Promise.all([sessionA.prompt("question A"), sessionB.prompt("question B")]);

		expect(transcriptText(sessionA)).toContain("question A");
		expect(transcriptText(sessionA)).toContain("answer A");
		expect(transcriptText(sessionA)).not.toContain("question B");
		expect(transcriptText(sessionA)).not.toContain("answer B");
		expect(transcriptText(sessionB)).toContain("question B");
		expect(transcriptText(sessionB)).toContain("answer B");
		expect(transcriptText(sessionB)).not.toContain("question A");
		expect(transcriptText(sessionB)).not.toContain("answer A");
		expect(eventsA.map(event => event.type)).toContain("agent_end");
		expect(eventsB.map(event => event.type)).toContain("agent_end");
		expect(mockA.calls).toHaveLength(1);
		expect(mockB.calls).toHaveLength(1);
	});

	it("aborting one session leaves the other running to completion", async () => {
		vi.spyOn(modelRegistry, "getApiKey").mockResolvedValue("test-key");
		const mockA = createMockModel({ responses: [{ content: ["slow answer"], delayMs: 30_000 }] });
		const mockB = createMockModel({ responses: [{ content: ["quick answer"] }] });
		const sessionA = makeSession(mockA);
		const sessionB = makeSession(mockB);

		const runningA = sessionA.prompt("slow question");
		await waitForStreaming(sessionA);
		await sessionB.prompt("quick question");
		expect(transcriptText(sessionB)).toContain("quick answer");

		await sessionA.abort();
		await runningA;

		expect(mockA.calls).toHaveLength(1);
		expect(transcriptText(sessionB)).toContain("quick answer");
		const messagesA = sessionA.agent.state.messages;
		const lastAssistantA = messagesA.filter(message => message.role === "assistant").pop();
		expect(lastAssistantA).toMatchObject({ stopReason: "aborted" });
		expect(transcriptText(sessionA)).not.toContain("slow answer");
	});

	it("executes tools in both sessions concurrently without cross-talk", async () => {
		vi.spyOn(modelRegistry, "getApiKey").mockResolvedValue("test-key");
		const releaseSlowTool = Promise.withResolvers<void>();
		let slowStarted = false;
		const slowTool: AgentTool = {
			name: "slow-compute",
			label: "Slow compute",
			description: "Slow compute tool",
			parameters: type({ value: "string" }),
			strict: true,
			async execute() {
				slowStarted = true;
				await releaseSlowTool.promise;
				return { content: [{ type: "text", text: "slow result" }] };
			},
		};
		const quickTool: AgentTool = {
			name: "quick-compute",
			label: "Quick compute",
			description: "Quick compute tool",
			parameters: type({ value: "string" }),
			strict: true,
			async execute() {
				return { content: [{ type: "text", text: "quick result" }] };
			},
		};
		const mockA = createMockModel({
			responses: [
				{ content: [{ type: "toolCall", id: "call-slow", name: "slow-compute", arguments: { value: "a" } }] },
				{ content: ["slow turn done"] },
			],
		});
		const mockB = createMockModel({
			responses: [
				{ content: [{ type: "toolCall", id: "call-quick", name: "quick-compute", arguments: { value: "b" } }] },
				{ content: ["quick turn done"] },
			],
		});
		const sessionA = makeSession(mockA, [slowTool]);
		const sessionB = makeSession(mockB, [quickTool]);
		try {
			const runningA = sessionA.prompt("run the slow tool");
			for (let waited = 0; waited < 5000 && !slowStarted; waited += 5) {
				await Bun.sleep(5);
			}
			expect(slowStarted).toBe(true);
			await sessionB.prompt("run the quick tool");
			expect(transcriptText(sessionB)).toContain("quick result");
			expect(transcriptText(sessionB)).toContain("quick turn done");
			expect(transcriptText(sessionB)).not.toContain("slow result");

			releaseSlowTool.resolve();
			await runningA;
			expect(transcriptText(sessionA)).toContain("slow result");
			expect(transcriptText(sessionA)).toContain("slow turn done");
			expect(transcriptText(sessionA)).not.toContain("quick result");
			expect(mockA.calls).toHaveLength(2);
			expect(mockB.calls).toHaveLength(2);
		} finally {
			releaseSlowTool.resolve();
		}
	});
});
