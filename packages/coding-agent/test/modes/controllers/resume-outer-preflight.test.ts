import { afterEach, beforeAll, describe, expect, it, vi } from "bun:test";
import * as path from "node:path";
import * as core from "@harvest/pi-agent-core";
import { ModelRegistry } from "@harvest/pi-coding-agent/config/model-registry";
import { resetSettingsForTest, Settings } from "@harvest/pi-coding-agent/config/settings";
import { InteractiveMode } from "@harvest/pi-coding-agent/modes/interactive-mode";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { AgentSession } from "@harvest/pi-coding-agent/session/agent-session";
import { AuthStorage } from "@harvest/pi-coding-agent/session/auth-storage";
import { SessionManager } from "@harvest/pi-coding-agent/session/session-manager";
import { createTools, type Tool } from "@harvest/pi-coding-agent/tools";
import { getProjectDir, setProjectDir, TempDir } from "@harvest/pi-utils";

beforeAll(async () => {
	await initTheme();
});

afterEach(() => {
	vi.restoreAllMocks();
});

async function createMode(opts: { flushFails?: boolean } = {}): Promise<{
	mode: InteractiveMode;
	session: AgentSession;
	targetPath: string;
	cleanup: () => Promise<void>;
}> {
	resetSettingsForTest();
	const previousProjectDir = getProjectDir();
	const tempDir = TempDir.createSync("@pi-resume-outer-");
	await Settings.init({ inMemory: true, cwd: tempDir.path() });
	const settings = Settings.isolated({ "compaction.enabled": false });

	const authStorage = await AuthStorage.create(path.join(tempDir.path(), "testauth.db"));
	const modelRegistry = new ModelRegistry(authStorage);
	const model = modelRegistry.find("anthropic", "claude-sonnet-4-5");
	if (!model) throw new Error("Expected claude-sonnet-4-5 to exist in registry");

	const initialTools = await createTools(
		{ cwd: tempDir.path(), hasUI: false, getSessionFile: () => null, getSessionSpawns: () => "*", settings },
		["read"],
	);
	const toolRegistry = new Map<string, Tool>(initialTools.map(tool => [tool.name, tool] as const));
	const session = new AgentSession({
		agent: new core.Agent({
			initialState: { model, systemPrompt: ["Test"], tools: initialTools, messages: [] },
		}),
		sessionManager: SessionManager.create(tempDir.path(), tempDir.path()),
		settings,
		modelRegistry,
		toolRegistry,
		rebuildSystemPrompt: async () => ({ systemPrompt: ["Test"] }),
	});
	const mode = new InteractiveMode(session, "test");
	const targetPath = path.join(tempDir.path(), "target-session.jsonl");
	const targetCwd = path.join(tempDir.path(), "target-project");
	await Bun.write(path.join(targetCwd, "README.md"), "Target project\n");
	await Bun.write(
		targetPath,
		`${JSON.stringify({ type: "session", version: 3, id: "target", cwd: targetCwd, timestamp: new Date().toISOString() })}\n`,
	);
	vi.spyOn(mode, "addMessageToChat").mockReturnValue([]);
	vi.spyOn(mode, "ensureLoadingAnimation").mockImplementation(() => {});
	mode.ui.requestRender = vi.fn();

	// Make settings.flush fail or succeed as configured.
	vi.spyOn(mode.settings, "flush").mockImplementation(async () => {
		if (opts.flushFails) throw new Error("disk full");
	});

	return {
		mode,
		session,
		targetPath,
		cleanup: async () => {
			mode.ui.stop();
			await session.dispose();
			authStorage.close();
			resetSettingsForTest();
			setProjectDir(previousProjectDir);
			await tempDir.remove();
		},
	};
}

describe("InteractiveMode.handleResumeSession outer preflight flush", () => {
	it("aborts before disposing controllers or resetting observers when flush fails", async () => {
		const { mode, targetPath, cleanup } = await createMode({ flushFails: true });
		try {
			const resetSpy = vi.spyOn(mode, "resetObserverRegistry");
			const switchSpy = vi.spyOn(mode.session, "switchSession").mockResolvedValue(true);
			const showErrorSpy = vi.spyOn(mode, "showError");

			await mode.handleResumeSession(targetPath);

			expect(mode.settings.flush).toHaveBeenCalled();
			expect(showErrorSpy).toHaveBeenCalledWith(expect.stringContaining("disk full"));
			expect(resetSpy).not.toHaveBeenCalled();
			expect(switchSpy).not.toHaveBeenCalled();
		} finally {
			await cleanup();
		}
	});

	it("delegates resume after a successful flush and keeps the view when its session identity is unchanged", async () => {
		const { mode, session, targetPath, cleanup } = await createMode({ flushFails: false });
		try {
			const resetSpy = vi.spyOn(mode, "resetObserverRegistry");
			const switchSpy = vi.spyOn(session, "switchSession").mockResolvedValue(true);

			await mode.handleResumeSession(targetPath);

			expect(mode.settings.flush).toHaveBeenCalled();
			expect(resetSpy).not.toHaveBeenCalled();
			expect(switchSpy).toHaveBeenCalledWith(
				targetPath,
				expect.objectContaining({ onCwdChange: expect.any(Function) }),
			);
		} finally {
			await cleanup();
		}
	});
});
