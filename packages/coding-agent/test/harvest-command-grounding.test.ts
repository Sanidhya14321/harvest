import { expect, it } from "bun:test";
import { Agent, type AgentToolResult } from "@harvest/pi-agent-core";
import { TempDir } from "@harvest/pi-utils";
import { ModelRegistry } from "../src/config/model-registry";
import { Settings } from "../src/config/settings";
import { interceptSessionToolCall } from "../src/core/agent-session";
import { AgentSession } from "../src/session/agent-session";
import { SessionManager } from "../src/session/session-manager";
import { createAssistantMessage, createInMemoryAuthStorage } from "./helpers/agent-session-setup";

it("validates mutations only from completed command outcomes and their final executed arguments", async () => {
	using tempDir = TempDir.createSync("harvest-command-grounding-");
	const authStorage = createInMemoryAuthStorage();
	const agent = new Agent();
	const session = new AgentSession({
		agent,
		sessionManager: SessionManager.inMemory(tempDir.path()),
		settings: Settings.isolated(),
		modelRegistry: new ModelRegistry(authStorage),
	});
	try {
		const hooks = session.harvestHooks;
		hooks.groundingEngine.recordFileMutation("app.ts");
		expect(interceptSessionToolCall(hooks, { name: "bash", args: { command: "bun run check" } }).allowed).toBe(true);
		expect(hooks.groundingEngine.isGroundingValidated()).toBe(false);
		const complete = async (result: AgentToolResult<unknown>, isError: boolean) => {
			if (!agent.afterToolCall) throw new Error("session completion hook missing");
			await agent.afterToolCall({
				assistantMessage: createAssistantMessage("checking"),
				toolCall: { type: "toolCall", id: "check-call", name: "bash", arguments: { command: "echo proposed" } },
				args: { command: "bun run check" },
				result,
				isError,
				context: { systemPrompt: [], messages: [] },
			});
		};
		await complete({ content: [{ type: "text", text: "permission denied" }], details: {} }, true);
		expect(hooks.groundingEngine.isGroundingValidated()).toBe(false);
		await complete(
			{ content: [{ type: "text", text: "FAIL app.test.ts\nAssertionError: mismatch" }], details: {} },
			false,
		);
		expect(hooks.groundingEngine.isGroundingValidated()).toBe(false);
		await complete(
			{
				content: [{ type: "text", text: "still executing" }],
				details: { async: { state: "running", jobId: "job-1" } },
			},
			false,
		);
		expect(hooks.groundingEngine.isGroundingValidated()).toBe(false);
		await complete({ content: [{ type: "text", text: "PASS app.test.ts" }], details: { exitCode: 0 } }, false);
		expect(hooks.groundingEngine.isGroundingValidated()).toBe(true);
		expect(hooks.groundingEngine.hasRecoveredFromFailure()).toBe(true);
	} finally {
		await session.dispose();
		authStorage.close();
	}
});
