import { afterEach, describe, expect, it } from "bun:test";
import * as path from "node:path";
import { Agent } from "@harvest/pi-agent-core";
import { TempDir } from "@harvest/pi-utils";
import { VirtualTerminal } from "../../tui/test/virtual-terminal";
import { ModelRegistry } from "../src/config/model-registry";
import { resetSettingsForTest, Settings } from "../src/config/settings";
import { Composer } from "../src/modes/composer";
import { resolvePaletteSelection, type CommandPaletteItem } from "../src/modes/components/command-palette";
import { InteractiveMode } from "../src/modes/interactive-mode";
import { initTheme } from "../src/modes/theme/theme";
import { AuthStorage } from "../src/session/auth-storage";
import { AgentSession } from "../src/session/agent-session";
import { SessionManager } from "../src/session/session-manager";

async function boot(
	columns: number,
	rows: number,
): Promise<{
	mode: InteractiveMode;
	terminal: VirtualTerminal;
	teardown: () => Promise<void>;
}> {
	const directory = TempDir.createSync("@harvest-palette-dispatch-");
	await Settings.init({ inMemory: true, cwd: directory.path() });
	Settings.instance.set("tui.fullscreen", true);
	await initTheme(false);
	const auth = await AuthStorage.create(path.join(directory.path(), "auth.db"));
	const registry = new ModelRegistry(auth);
	const model = registry.find("anthropic", "claude-sonnet-4-5");
	if (!model) throw new Error("Missing test model");
	const manager = SessionManager.create(directory.path(), directory.path());
	manager.appendMessage({ role: "user", content: "Palette dispatch review", timestamp: Date.now() });
	await manager.ensureOnDisk();
	const session = new AgentSession({
		agent: new Agent({ initialState: { model, systemPrompt: ["Test"], tools: [], messages: [] } }),
		sessionManager: manager,
		settings: Settings.isolated(),
		modelRegistry: registry,
	});
	const terminal = new VirtualTerminal(columns, rows);
	const mode = new InteractiveMode(
		session,
		"test",
		undefined,
		() => {},
		undefined,
		undefined,
		undefined,
		new Composer({ terminal }),
	);
	await mode.init({ suppressWelcomeIntro: true });
	await mode.renderInitialMessages();
	void mode.getUserInput();
	await terminal.waitForRender();
	return {
		mode,
		terminal,
		teardown: async () => {
			mode?.stop();
			await mode?.liveSessions?.dispose();
			await session?.dispose();
			auth?.close();
			await directory?.remove();
			resetSettingsForTest();
		},
	};
}

describe("palette selection routing (U1)", () => {
	it("drafts argument-bearing items instead of executing", () => {
		const item: CommandPaletteItem = { id: "tab close", title: "Close tab", intent: "draft" };
		expect(resolvePaletteSelection(item)).toEqual({ kind: "draft", text: "/tab close " });
	});

	it("dispatches runnable builtins through the builtin path", () => {
		const item: CommandPaletteItem = { id: "commands", title: "Commands", intent: "run" };
		expect(resolvePaletteSelection(item)).toEqual({ kind: "dispatch", text: "/commands" });
	});

	it("dispatches runnable non-builtins through normal submission, never bare execution", () => {
		const item: CommandPaletteItem = { id: "foo", title: "Foo custom", group: "extension", intent: "run" };
		// The owner routes `dispatch` plans for non-builtins through
		// editor.onSubmit (production submission: skills, extension-local,
		// custom/file/template expansion) exactly once — never a shell string
		// and never a silent no-op.
		expect(resolvePaletteSelection(item)).toEqual({ kind: "dispatch", text: "/foo" });
	});
});

describe("palette resize re-budget (U2)", () => {
	const cleanups: Array<() => Promise<void>> = [];
	afterEach(async () => {
		for (const cleanup of cleanups.splice(0)) await cleanup();
		resetSettingsForTest();
	});

	it("re-budgets the open palette on terminal resize so the selection stays visible", async () => {
		const { mode, terminal, teardown } = await boot(100, 30);
		cleanups.push(teardown);
		try {
			await mode.openCommandPalette();
			// Pin the selection to the last command, then shrink hard.
			for (let i = 0; i < 200; i++) terminal.sendInput("\x1b[B");
			await terminal.waitForRender();
			const before = terminal
				.getViewport()
				.map(row => Bun.stripANSI(row))
				.join("\n");
			const match = before.match(/›\s*(\/\S+)/);
			expect(match?.[1]).toBeDefined();
			const selected = match![1]!;
			terminal.resize(80, 10);
			process.stdout.emit("resize");
			await terminal.waitForRender();
			const viewport = terminal.getViewport();
			expect(viewport.length).toBeLessThanOrEqual(10);
			const text = viewport.map(row => Bun.stripANSI(row)).join("\n");
			expect(text).toContain(selected);
		} finally {
			await teardown();
			cleanups.splice(cleanups.indexOf(teardown), 1);
		}
	}, 60000);
});
