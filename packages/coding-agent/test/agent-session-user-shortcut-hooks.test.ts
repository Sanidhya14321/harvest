import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import { Agent } from "@harvest/pi-agent-core";
import { getBundledModel } from "@harvest/pi-catalog/models";
import { ModelRegistry } from "@harvest/pi-coding-agent/config/model-registry";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import * as pythonExecutor from "@harvest/pi-coding-agent/eval/py/executor";
import * as bashExecutor from "@harvest/pi-coding-agent/exec/bash-executor";
import type { ExtensionRunner } from "@harvest/pi-coding-agent/extensibility/extensions";
import { AgentSession } from "@harvest/pi-coding-agent/session/agent-session";
import { SessionManager } from "@harvest/pi-coding-agent/session/session-manager";
import { TempDir } from "@harvest/pi-utils";
import { createInMemoryAuthStorage } from "./helpers/agent-session-setup";

const sharedAuthStorage = createInMemoryAuthStorage();
const sharedModelRegistry = new ModelRegistry(sharedAuthStorage);

afterAll(() => {
	sharedAuthStorage.close();
});

describe("AgentSession user shortcut hooks", () => {
	let tempDir: TempDir;
	let session: AgentSession;
	let modelRegistry: ModelRegistry;

	beforeEach(() => {
		tempDir = TempDir.createSync("@pi-user-shortcut-hooks-");
		modelRegistry = sharedModelRegistry;
	});

	afterEach(async () => {
		vi.restoreAllMocks();
		if (session) {
			await session.dispose();
		}
		await pythonExecutor.disposeAllKernelSessions();
		tempDir.removeSync();
	});

	function createSession(extensionRunner?: ExtensionRunner): AgentSession {
		const model = getBundledModel("anthropic", "claude-sonnet-4-5");
		if (!model) throw new Error("Expected claude-sonnet-4-5 model to exist");

		const agent = new Agent({
			initialState: {
				model,
				systemPrompt: ["Test"],
				tools: [],
				messages: [],
			},
		});

		session = new AgentSession({
			agent,
			sessionManager: SessionManager.inMemory(tempDir.path()),
			settings: Settings.isolated({ "compaction.enabled": false }),
			modelRegistry,
			extensionRunner,
		});
		return session;
	}

	it("invokes user_bash hook and honors replacement result", async () => {
		const replacement = {
			output: "hooked bash output",
			exitCode: 0,
			cancelled: false,
			truncated: false,
			totalLines: 1,
			totalBytes: 18,
			outputLines: 1,
			outputBytes: 18,
		};
		const emitUserBash = vi.fn().mockResolvedValue({ result: replacement });
		const extensionRunner = {
			hasHandlers: vi.fn((eventType: string) => eventType === "user_bash"),
			emitUserBash,
		} as unknown as ExtensionRunner;
		const executeBashSpy = vi.spyOn(bashExecutor, "executeBash");

		createSession(extensionRunner);
		const result = await session.executeBash("echo hello", undefined, { excludeFromContext: true });

		expect(emitUserBash).toHaveBeenCalledWith({
			type: "user_bash",
			command: "echo hello",
			excludeFromContext: true,
			cwd: expect.any(String),
		});
		expect(executeBashSpy).not.toHaveBeenCalled();
		expect(result).toEqual(replacement);
		const bashMessage = session.messages.at(-1);
		expect(bashMessage?.role).toBe("bashExecution");
		expect(bashMessage).toMatchObject({
			output: "hooked bash output",
			excludeFromContext: true,
		});
	});

	it("rejects user_python before invoking an extension or executor", async () => {
		const emitUserPython = vi.fn().mockResolvedValue({ result: { output: "must not run" } });
		const extensionRunner = { hasHandlers: vi.fn(() => true), emitUserPython } as unknown as ExtensionRunner;
		const execute = vi.spyOn(pythonExecutor, "executePython");
		createSession(extensionRunner);
		await expect(session.executePython("opaque legacy input")).rejects.toThrow("Python evaluation was retired");
		expect(emitUserPython).not.toHaveBeenCalled();
		expect(execute).not.toHaveBeenCalled();
		expect(session.messages).toEqual([]);
	});

	it("falls back to normal execution when hook does not return a replacement", async () => {
		const extensionRunner = {
			hasHandlers: vi.fn((eventType: string) => eventType === "user_bash" || eventType === "user_python"),
			emitUserBash: vi.fn().mockResolvedValue({}),
			emitUserPython: vi.fn().mockResolvedValue(undefined),
		} as unknown as ExtensionRunner;
		vi.spyOn(bashExecutor, "executeBash").mockResolvedValue({
			output: "bash fallback",
			exitCode: 0,
			cancelled: false,
			truncated: false,
			totalLines: 1,
			totalBytes: 13,
			outputLines: 1,
			outputBytes: 13,
		});

		createSession(extensionRunner);
		const bashResult = await session.executeBash("pwd", undefined, { excludeFromContext: true });
		await expect(session.executePython("opaque legacy input")).rejects.toThrow("Python evaluation was retired");

		expect(bashResult.output).toBe("bash fallback");
		expect(bashExecutor.executeBash).toHaveBeenCalledTimes(1);
		expect(
			session.messages.some(message => message.role === "bashExecution" && message.excludeFromContext === true),
		).toBe(true);
		expect(
			session.messages.some(message => message.role === "pythonExecution" && message.excludeFromContext === false),
		).toBe(false);
	});

	it("does not start a kernel through direct execution or the session shortcut", async () => {
		createSession();
		await expect(pythonExecutor.executePython("opaque legacy input", { cwd: tempDir.path() })).rejects.toThrow(
			"Python evaluation was retired",
		);
		await expect(session.executePython("opaque legacy input")).rejects.toThrow("Python evaluation was retired");
		expect(session.messages).toEqual([]);
	});
});
