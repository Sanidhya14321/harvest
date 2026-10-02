import { afterEach, expect, it, vi } from "bun:test";
import { type } from "@harvest/omptype";
import { Agent, type AgentTool, type AgentToolCall } from "@harvest/pi-agent-core";
import { TempDir } from "@harvest/pi-utils";
import { ModelRegistry } from "../src/config/model-registry";
import { Settings } from "../src/config/settings";
import { LayaClient } from "../src/core/harvest/laya-client";
import { AgentSession } from "../src/session/agent-session";
import { SessionManager } from "../src/session/session-manager";
import { classifyUnexpectedStop } from "../src/session/unexpected-stop-classifier";
import { createAssistantMessage, createInMemoryAuthStorage } from "./helpers/agent-session-setup";

afterEach(() => vi.restoreAllMocks());

it("keeps tool approval and completion decisions scoped to each session's Laya setting", async () => {
	using tempDir = TempDir.createSync("harvest-laya-owner-");
	const authStorage = createInMemoryAuthStorage();
	const registry = new ModelRegistry(authStorage);
	const decide = vi
		.spyOn(LayaClient.prototype, "decide")
		.mockResolvedValue({ success: false, fallback: true, fallbackReason: "sidecar_unavailable", latencyMs: 0 });
	const sessions: AgentSession[] = [];
	const tool: AgentTool = {
		name: "publish",
		label: "publish",
		description: "publish",
		approval: "write",
		parameters: type({}),
		execute: async () => ({ content: [{ type: "text", text: "published" }] }),
	};
	try {
		for (const enabled of [false, true]) {
			const agent = new Agent();
			const settings = Settings.isolated({ "laya.enabled": enabled });
			const session = new AgentSession({
				agent,
				sessionManager: SessionManager.inMemory(tempDir.path()),
				settings,
				modelRegistry: registry,
			});
			sessions.push(session);
			const toolCall: AgentToolCall = { type: "toolCall", id: `publish-${enabled}`, name: tool.name, arguments: {} };
			if (!agent.beforeToolCall) throw new Error("session preflight hook missing");
			await agent.beforeToolCall({
				tool,
				toolCall,
				args: {},
				assistantMessage: createAssistantMessage("publish"),
				context: { systemPrompt: [], messages: [] },
			});
			if (enabled) {
				expect(toolCall.providerMetadata).toMatchObject({ layaGatingRequired: true });
			} else {
				expect(toolCall.providerMetadata).toBeUndefined();
				expect(decide).not.toHaveBeenCalled();
			}
		}
		expect(decide).toHaveBeenCalledTimes(1);
		decide.mockClear();
		decide.mockResolvedValue({
			success: true,
			fallback: false,
			latencyMs: 0,
			data: { unexpected_stop: { type: "noul", noul: 0.9, confidence: 0.95 } },
		});
		await classifyUnexpectedStop("I will continue.", {
			settings: sessions[0].settings,
			registry,
			sessionId: "disabled-owner",
		});
		expect(decide).not.toHaveBeenCalled();
		expect(
			await classifyUnexpectedStop("I will continue.", {
				settings: sessions[1].settings,
				registry,
				sessionId: "enabled-owner",
			}),
		).toBe(true);
		expect(decide).toHaveBeenCalledTimes(1);
	} finally {
		for (const session of sessions) await session.dispose();
		authStorage.close();
	}
});
