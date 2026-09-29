import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "bun:test";
import { Agent } from "@harvest/pi-agent-core";
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
 * and abort independently. Background work in one must never stop, reroute,
 * or leak into the other. This is the headless prerequisite for live tabs —
 * the TUI retargeting layer builds on top of it.
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

	function makeSession(mock: MockModel): AgentSession {
		const agent = new Agent({
			getApiKey: () => "test-key",
			initialState: { model: mock.model, systemPrompt: [], tools: [], messages: [] },
			streamFn: mock.stream,
		});
		const session = new AgentSession({
			agent,
			sessionManager: SessionManager.inMemory(),
			settings: Settings.isolated({ "compaction.enabled": false }),
			modelRegistry,
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
});
