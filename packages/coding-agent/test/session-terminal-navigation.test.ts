import { afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import * as path from "node:path";
import { Agent } from "@harvest/pi-agent-core";
import { TempDir } from "@harvest/pi-utils";
import { VirtualTerminal } from "../../tui/test/virtual-terminal";
import { ModelRegistry } from "../src/config/model-registry";
import { resetSettingsForTest, Settings } from "../src/config/settings";
import { Composer } from "../src/modes/composer";
import { InteractiveMode } from "../src/modes/interactive-mode";
import { initTheme } from "../src/modes/theme/theme";
import { SelectorController } from "../src/modes/controllers/selector-controller";
import { AgentSession } from "../src/session/agent-session";
import { AuthStorage } from "../src/session/auth-storage";
import { SessionManager } from "../src/session/session-manager";

describe("session navigation through terminal input", () => {
	let directory: TempDir;
	let auth: AuthStorage;
	let session: AgentSession;
	let mode: InteractiveMode;
	let terminal: VirtualTerminal;
	let first: string;
	let second: string;

	beforeEach(async () => {
		resetSettingsForTest();
		directory = TempDir.createSync("@harvest-terminal-navigation-");
		await Settings.init({ inMemory: true, cwd: directory.path() });
		Settings.instance.set("tui.fullscreen", true);
		await initTheme(false);
		auth = await AuthStorage.create(path.join(directory.path(), "auth.db"));
		const registry = new ModelRegistry(auth);
		const model = registry.find("anthropic", "claude-sonnet-4-5");
		if (!model) throw new Error("Missing test model");
		const manager = SessionManager.create(directory.path(), directory.path());
		await manager.setSessionName("First task", "user");
		manager.appendMessage({ role: "user", content: "First transcript", timestamp: Date.now() });
		await manager.ensureOnDisk();
		first = manager.getSessionFile()!;
		const other = SessionManager.create(directory.path(), directory.path());
		await other.setSessionName("Second task", "user");
		other.appendMessage({ role: "user", content: "Second transcript", timestamp: Date.now() });
		await other.ensureOnDisk();
		second = other.getSessionFile()!;
		await other.close();
		session = new AgentSession({
			agent: new Agent({ initialState: { model, systemPrompt: ["Test"], tools: [], messages: [] } }),
			sessionManager: manager,
			settings: Settings.isolated(),
			modelRegistry: registry,
		});
		terminal = new VirtualTerminal(100, 30);
		mode = new InteractiveMode(
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
		void mode.getUserInput();
		const errors: string[] = [];
		const showError = vi.spyOn(mode, "showError").mockImplementation(message => {
			errors.push(message);
		});
		const opened = await mode.handleSessionTabsCommand(`open ${path.basename(second, ".jsonl")}`);
		showError.mockRestore();
		if (!opened.includes("Opened session tab")) throw new Error(`${opened}: ${errors.join("; ")}`);
		await mode.handleResumeSession(first);
		await terminal.waitForRender();
	});

	afterEach(async () => {
		vi.restoreAllMocks();
		mode?.stop();
		await mode?.liveSessions?.dispose();
		await session?.dispose();
		auth?.close();
		await directory?.remove();
		resetSettingsForTest();
	});

	async function capture(name: string): Promise<void> {
		const output = Bun.env.HARVEST_TERMINAL_CAPTURE_DIR;
		if (!output) return;
		await terminal.waitForRender();
		await Bun.write(
			path.join(output, `${name}.json`),
			JSON.stringify(
				{
					label: name,
					columns: terminal.columns,
					rows: terminal.rows,
					viewport: terminal.getViewport(),
					cells: terminal.getViewportCellRows(),
				},
				null,
				2,
			),
		);
	}

	function click(label: string): void {
		const rows = terminal.getViewport().map(row => Bun.stripANSI(row));
		const row = rows.findIndex(text => text.includes(label));
		expect(row).toBeGreaterThanOrEqual(0);
		const col = rows[row].indexOf(label);
		terminal.sendInput(`\x1b[<0;${col + 1};${row + 1}M`);
		terminal.sendInput(`\x1b[<0;${col + 1};${row + 1}m`);
	}

	it("switches by SGR mouse, preserves each draft, and leaves the active tab untouched", async () => {
		terminal.sendInput("FIRST_DRAFT");
		await terminal.waitForRender(() => terminal.getViewport().some(row => row.includes("FIRST_DRAFT")));
		await capture("01-before-switch");
		const flush = vi.spyOn(mode.settings, "flush");
		click("First task");
		await Bun.sleep(20);
		expect(flush).not.toHaveBeenCalled();
		click("Second task");
		await terminal.waitForRender(() => mode.sessionManager.getSessionFile() === second);
		expect(mode.editor.getText()).toBe("");
		terminal.sendInput("SECOND_DRAFT");
		await terminal.waitForRender(() => terminal.getViewport().some(row => row.includes("SECOND_DRAFT")));
		await capture("02-after-mouse-switch");
		click("First task");
		await terminal.waitForRender(
			() =>
				terminal.getViewport().some(row => row.includes("FIRST_DRAFT")) &&
				terminal.getViewport().some(row => row.includes("First transcript")),
		);
		expect(mode.sessionManager.getSessionFile()).toBe(first);
		await capture("03-restored-draft");
	});

	it("keeps input usable after a settings-save failure and supports command retry", async () => {
		terminal.sendInput("KEEP_DRAFT");
		await terminal.waitForRender();
		const flush = vi.spyOn(mode.settings, "flush").mockRejectedValueOnce(new Error("disk unavailable"));
		click("Second task");
		await terminal.waitForRender(() => terminal.getViewport().some(row => row.includes("disk unavailable")));
		expect(mode.sessionManager.getSessionFile()).toBe(first);
		expect(mode.editor.getText()).toBe("KEEP_DRAFT");
		await capture("04-save-failure");
		flush.mockRestore();
		expect(await mode.handleSessionTabsCommand("switch 2")).toContain("Switched");
		expect(mode.sessionManager.getSessionFile()).toBe(second);
	});

	it("serializes in-flight navigation and honors the last rapid request", async () => {
		const gate = Promise.withResolvers<void>();
		const entered = Promise.withResolvers<void>();
		const flush = vi.spyOn(mode.settings, "flush").mockImplementationOnce(async () => {
			entered.resolve();
			await gate.promise;
		});
		const pending = mode.handleResumeSession(second);
		await entered.promise;
		const returnToFirst = mode.handleResumeSession(first);
		try {
			gate.resolve();
			await Promise.all([pending, returnToFirst]);
			expect(mode.sessionManager.getSessionFile()).toBe(first);
			terminal.sendInput("STILL_USABLE");
			await terminal.waitForRender(() => mode.editor.getText() === "STILL_USABLE");
			await capture("05-rapid-navigation");
		} finally {
			gate.resolve();
			flush.mockRestore();
		}
	});

	it.each(["missing", "corrupt", "locked"] as const)(
		"preserves the current run and input when a cold target is %s",
		async failure => {
			const target = SessionManager.create(directory.path(), directory.path());
			target.appendMessage({ role: "user", content: "Cold target", timestamp: Date.now() });
			await target.ensureOnDisk();
			const targetPath = target.getSessionFile()!;
			try {
				if (failure !== "locked") await target.close();
				if (failure === "missing") await Bun.file(targetPath).delete();
				if (failure === "corrupt") await Bun.write(targetPath, "broken journal\n");
				terminal.sendInput("RECOVERABLE_DRAFT");
				await terminal.waitForRender();
				const abort = vi.spyOn(mode.session, "abort");
				await mode.handleResumeSession(targetPath);
				expect(mode.sessionManager.getSessionFile()).toBe(first);
				expect(mode.editor.getText()).toBe("RECOVERABLE_DRAFT");
				expect(abort).not.toHaveBeenCalled();
				await terminal.waitForRender(() => terminal.getViewport().some(row => row.includes("Error:")));
				await capture(`06-cold-${failure}`);
				terminal.sendInput("_EDITABLE");
				await terminal.waitForRender(() => mode.editor.getText() === "RECOVERABLE_DRAFT_EDITABLE");
			} finally {
				await target.close();
			}
		},
	);

	it("keeps keyboard navigation usable when a short viewport clips every tab", async () => {
		terminal.resize(24, 4);
		await terminal.waitForRender();
		const flush = vi.spyOn(mode.settings, "flush");
		terminal.sendInput("\x1b[<0;1;1M");
		await Bun.sleep(20);
		expect(flush).not.toHaveBeenCalled();
		expect(mode.sessionManager.getSessionFile()).toBe(first);
		await capture("07-short-viewport");
		expect(await mode.handleSessionTabsCommand("switch 2")).toContain("Switched");
		expect(mode.sessionManager.getSessionFile()).toBe(second);
	});

	it("creates a new tab by mouse without stopping the source and restores its draft on return", async () => {
		terminal.sendInput("SOURCE_BEFORE_NEW");
		await terminal.waitForRender();
		const previousId = mode.sessionManager.getSessionId();
		const abort = vi.spyOn(mode.session, "abort");
		click("New session");
		await terminal.waitForRender(
			() => mode.sessionManager.getSessionId() !== previousId && mode.editor.getText() === "",
		);
		expect(abort).not.toHaveBeenCalled();
		await capture("08-new-tab-by-mouse");
		click("First task");
		await terminal.waitForRender(
			() =>
				terminal.getViewport().some(row => row.includes("SOURCE_BEFORE_NEW")) &&
				terminal.getViewport().some(row => row.includes("First transcript")),
		);
		expect(mode.sessionManager.getSessionFile()).toBe(first);
	});

	it("coalesces a repeated New session click while the first runtime is still opening", async () => {
		const ids = new Set(mode.liveSessions!.sessions.map(runtime => runtime.sessionManager.getSessionId()));
		const settings = mode.settings;
		const clone = settings.cloneForCwd.bind(settings);
		const gate = Promise.withResolvers<void>();
		const entered = Promise.withResolvers<void>();
		const delayedClone = vi.spyOn(settings, "cloneForCwd").mockImplementationOnce(async cwd => {
			entered.resolve();
			await gate.promise;
			return clone(cwd);
		});
		const firstClick = mode.handleClearCommand();
		await entered.promise;
		const duplicateClick = mode.handleClearCommand();
		try {
			gate.resolve();
			await Promise.all([firstClick, duplicateClick]);
			const newTabs = mode.liveSessions!.sessions.filter(runtime => !ids.has(runtime.sessionManager.getSessionId()));
			expect(newTabs).toHaveLength(1);
			expect(mode.sessionManager.getSessionId()).toBe(newTabs[0].sessionManager.getSessionId());
		} finally {
			gate.resolve();
			delayedClone.mockRestore();
		}
	});

	it("retains text typed during a failed asynchronous load instead of replaying an older draft", async () => {
		terminal.sendInput("INITIAL");
		await terminal.waitForRender();
		const gate = Promise.withResolvers<void>();
		const entered = Promise.withResolvers<void>();
		const resume = vi.spyOn(SelectorController.prototype, "handleResumeSession").mockImplementationOnce(async () => {
			entered.resolve();
			await gate.promise;
			throw new Error("target read failed");
		});
		const pending = mode.handleResumeSession(second);
		await entered.promise;
		try {
			terminal.sendInput("_TYPED_DURING_LOAD");
			gate.resolve();
			await pending;
			expect(mode.sessionManager.getSessionFile()).toBe(first);
			expect(mode.editor.getText()).toBe("INITIAL_TYPED_DURING_LOAD");
			await terminal.waitForRender(() => terminal.getViewport().some(row => row.includes("target read failed")));
			await capture("09-typing-during-failed-load");
		} finally {
			gate.resolve();
			resume.mockRestore();
		}
	});
});
