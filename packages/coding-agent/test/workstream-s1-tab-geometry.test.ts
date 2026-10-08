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
import { AgentSession } from "../src/session/agent-session";
import { AuthStorage } from "../src/session/auth-storage";
import { SessionManager } from "../src/session/session-manager";

/**
 * T01/T02: tab-strip mouse geometry through real VT frames plus close order.
 * Boots three empty-transcript sessions (Home-centered layout with visible
 * tabs, the S1 repro shape) and sends SGR reports at VISIBLE screen cells:
 * the visible × closes exactly its tab (right-neighbor selection), while
 * blank/stale cells, covered cells, and open-modal clicks do nothing.
 */
describe("tab-strip screen geometry and close order", () => {
	let directory: TempDir;
	let auth: AuthStorage;
	let session: AgentSession;
	let mode: InteractiveMode;
	let terminal: VirtualTerminal;
	let files: { first: string; second: string; third: string };

	async function makeSession(name: string): Promise<SessionManager> {
		const manager = SessionManager.create(directory.path(), directory.path());
		await manager.setSessionName(name, "user");
		await manager.ensureOnDisk();
		const file = manager.getSessionFile();
		if (!file) throw new Error(`Session ${name} has no file`);
		await manager.close();
		return manager;
	}

	beforeEach(async () => {
		resetSettingsForTest();
		directory = TempDir.createSync("@harvest-tab-geometry-");
		await Settings.init({ inMemory: true, cwd: directory.path() });
		Settings.instance.set("tui.fullscreen", true);
		await initTheme(false);
		auth = await AuthStorage.create(path.join(directory.path(), "auth.db"));
		const registry = new ModelRegistry(auth);
		const model = registry.find("anthropic", "claude-sonnet-4-5");
		if (!model) throw new Error("Missing test model");
		const first = await makeSession("First");
		const second = await makeSession("Second");
		const third = await makeSession("Third");
		files = {
			first: first.getSessionFile()!,
			second: second.getSessionFile()!,
			third: third.getSessionFile()!,
		};
		session = new AgentSession({
			agent: new Agent({ initialState: { model, systemPrompt: ["Test"], tools: [], messages: [] } }),
			sessionManager: first,
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
		// Open all three tabs; the transcripts stay empty so the composer
		// uses the centered Home group with visible tabs (S1 repro shape).
		for (const file of [files.second, files.third]) {
			const opened = await mode.handleSessionTabsCommand(`open ${path.basename(file, ".jsonl")}`);
			if (!opened.includes("Opened session tab")) throw new Error(`open failed: ${opened}`);
		}
		const switched = await mode.handleSessionTabsCommand("switch 3");
		if (!switched.includes("Switched") && !switched.includes("Already viewing")) {
			throw new Error(`switch failed: ${switched}`);
		}
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

	function rows(): string[] {
		return terminal.getViewport().map(row => Bun.stripANSI(row));
	}

	function activeFile(): string | undefined {
		return mode.sessionManager.getSessionFile() ?? undefined;
	}

	async function listTabs(): Promise<string> {
		return mode.handleSessionTabsCommand("list");
	}

	/** Zero-based screen cell of the visible × for the tab titled `label`. */
	function findCloseCell(label: string): { row: number; col: number } {
		const view = rows();
		const row = view.findIndex(text => text.includes(label));
		if (row < 0) throw new Error(`Tab ${label} is not visible`);
		const text = view[row]!;
		const labelAt = text.indexOf(label);
		const after = text.indexOf("x", labelAt + label.length);
		if (after < 0 || text[after - 1] !== " " || (after + 1 < text.length && text[after + 1] !== " ")) {
			throw new Error(`No close cell after ${label} on row ${row}: ${JSON.stringify(text)}`);
		}
		return { row, col: after };
	}

	function clickAt(row: number, col: number): void {
		// SGR reports are 1-based; press then release.
		terminal.sendInput(`\x1b[<0;${col + 1};${row + 1}M`);
		terminal.sendInput(`\x1b[<0;${col + 1};${row + 1}m`);
	}

	it("closes the visible tab at its own cell and selects the left neighbor for the rightmost tab", async () => {
		expect(activeFile()).toBe(files.third);
		const frame = mode.composer.workspaceStripFrame();
		expect(frame).toBeDefined();
		const third = findCloseCell("Third");
		// The strip row sits below the centered Home group, never at
		// screen row zero (the S1 failure mode).
		expect(third.row).toBeGreaterThan(0);
		clickAt(third.row, third.col);
		await terminal.waitForRender(() => activeFile() === files.second);
		expect(await listTabs()).not.toContain("Third");
		// Rightmost close selected the left neighbor (S7/T02), runtime kept.
		expect(activeFile()).toBe(files.second);
		// Closing the new rightmost (Second, active middle/second) selects First.
		const second = findCloseCell("Second");
		clickAt(second.row, second.col);
		await terminal.waitForRender(() => activeFile() === files.first);
		expect(await listTabs()).not.toContain("Second");
		// Final close reaches detached Home with the runtime manageable.
		const first = findCloseCell("First");
		clickAt(first.row, first.col);
		await terminal.waitForRender(() => mode.isHomeDetached());
		expect(await listTabs()).toContain("No session tabs are open");
		expect(mode.isHomeDetached()).toBe(true);
	});

	it("ignores blank cells, stale screen-zero hits, and clicks under an open modal", async () => {
		const errors: string[] = [];
		const showError = vi.spyOn(mode, "showError").mockImplementation(message => {
			errors.push(message);
		});
		try {
			// Blank top-left cell: never a tab target, even though the old
			// screen-coordinate path closed tabs from row zero.
			clickAt(0, 0);
			await Bun.sleep(50);
			expect(activeFile()).toBe(files.third);
			expect(await listTabs()).toContain("Third");
			// A modal overlay owns its input: the visible × beneath it stays dead.
			await mode.openCommandPalette();
			expect(mode.ui.hasOverlay()).toBe(true);
			const third = findCloseCell("Third");
			clickAt(third.row, third.col);
			await Bun.sleep(50);
			expect(mode.ui.hasOverlay()).toBe(true);
			expect(activeFile()).toBe(files.third);
			expect(await listTabs()).toContain("Third");
			expect(errors).toEqual([]);
		} finally {
			showError.mockRestore();
		}
	});

	it("keeps hover on translated cells and clears it off-strip", async () => {
		const third = findCloseCell("Third");
		terminal.sendInput(`\x1b[<32;${third.col + 1};${third.row + 1}M`);
		await terminal.waitForRender();
		// Moving far off-strip clears the hover without selecting anything.
		terminal.sendInput(`\x1b[<32;1;1M`);
		await Bun.sleep(30);
		expect(activeFile()).toBe(files.third);
		expect(await listTabs()).toContain("Third");
	});
});
