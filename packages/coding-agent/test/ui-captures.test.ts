/**
 * Visual capture suite: boots the real InteractiveMode at the required
 * matrix sizes/themes and emits VT-cell JSON captures (rasterized by
 * bench/render-terminal-captures.py, inspected as PNGs — generating them
 * is not review).
 *
 * Isolated fixtures only (TempDir sessions, in-memory settings, bundled
 * model registry); no provider calls. Skips silently without
 * HARVEST_TERMINAL_CAPTURE_DIR.
 */
import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import * as path from "node:path";
import { Agent } from "@harvest/pi-agent-core";
import { TempDir } from "@harvest/pi-utils";
import { VirtualTerminal } from "../../tui/test/virtual-terminal";
import { ModelRegistry } from "../src/config/model-registry";
import { resetSettingsForTest, Settings } from "../src/config/settings";
import { Composer } from "../src/modes/composer";
import { InteractiveMode } from "../src/modes/interactive-mode";
import { getThemeByName, initTheme, setThemeInstance } from "../src/modes/theme/theme";
import { AuthStorage } from "../src/session/auth-storage";
import { AgentSession } from "../src/session/agent-session";
import { SessionManager } from "../src/session/session-manager";

async function boot(
	columns: number,
	rows: number,
	options: { light?: boolean; ascii?: boolean; withSecondTab?: boolean; empty?: boolean } = {},
): Promise<{
	mode: InteractiveMode;
	terminal: VirtualTerminal;
	teardown: () => Promise<void>;
}> {
	const directory = TempDir.createSync("@harvest-ui-captures-");
	await Settings.init({ inMemory: true, cwd: directory.path() });
	Settings.instance.set("tui.fullscreen", true);
	if (options.ascii) Settings.instance.set("symbolPreset", "ascii");
	await initTheme(false);
	const auth = await AuthStorage.create(path.join(directory.path(), "auth.db"));
	const registry = new ModelRegistry(auth);
	const model = registry.find("anthropic", "claude-sonnet-4-5");
	if (!model) throw new Error("Missing test model");
	const manager = SessionManager.create(directory.path(), directory.path());
	await manager.setSessionName("Capture session", "user");
	if (!options.empty) {
		manager.appendMessage({ role: "user", content: "Render the workspace for review", timestamp: Date.now() });
	}
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
	if (options.light) {
		const lightTheme = await getThemeByName("harvest-light");
		if (!lightTheme) throw new Error("Missing harvest-light theme");
		setThemeInstance(lightTheme);
	}
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

async function capture(terminal: VirtualTerminal, name: string): Promise<void> {
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

describe("ui captures", () => {
	let cleanups: Array<() => Promise<void>> = [];

	beforeEach(() => {
		cleanups = [];
	});

	afterEach(async () => {
		for (const cleanup of cleanups.splice(0)) await cleanup();
		resetSettingsForTest();
	});

	async function shot(
		columns: number,
		rows: number,
		name: string,
		act?: (ctx: { mode: InteractiveMode; terminal: VirtualTerminal }) => Promise<void> | void,
		bootOptions: { light?: boolean; ascii?: boolean; empty?: boolean } = {},
	): Promise<void> {
		const { mode, terminal, teardown } = await boot(columns, rows, bootOptions);
		cleanups.push(teardown);
		try {
			if (act) await act({ mode, terminal });
			await capture(terminal, name);
			expect(terminal.getViewport().length).toBeLessThanOrEqual(rows);
		} finally {
			await teardown();
			cleanups.splice(cleanups.indexOf(teardown), 1);
		}
	}

	it("captures the matrix", async () => {
		await shot(100, 30, "cap-100x30-session");
		await shot(100, 30, "cap-100x30-home", undefined, { empty: true });
		await shot(100, 30, "cap-100x30-palette", async ({ mode }) => {
			await mode.openCommandPalette();
		});
		await shot(100, 30, "cap-100x30-submitted", async ({ terminal }) => {
			terminal.sendInput("hello harvest");
			await terminal.waitForRender(() => terminal.getViewport().some(row => row.includes("hello harvest")));
			terminal.sendInput("\r");
			// Optimistic user panel paints before any model reply; capture it
			// without waiting on provider round-trips.
			await terminal.waitForRender(() => terminal.getViewport().some(row => row.includes("hello harvest")));
		});
		await shot(160, 45, "cap-160x45-docked");
		await shot(121, 32, "cap-121x32-dock-edge");
		await shot(120, 32, "cap-120x32-hide-edge");
		await shot(80, 24, "cap-80x24-narrow");
		await shot(80, 24, "cap-80x24-overlay", async ({ mode }) => {
			mode.toggleSidebar();
		});
		await shot(60, 16, "cap-60x16-short");
		await shot(24, 4, "cap-24x4-minimal");
	}, 120000);

	it("captures light and ascii variants", async () => {
		const light = await boot(100, 30, { light: true });
		try {
			await capture(light.terminal, "cap-100x30-light");
		} finally {
			await light.teardown();
		}
		const ascii = await boot(80, 24, { ascii: true });
		try {
			await capture(ascii.terminal, "cap-80x24-ascii");
		} finally {
			await ascii.teardown();
		}
	}, 120000);

	it("captures session-management states", async () => {
		// Multiple tabs with the × close affordance.
		const tabs = await boot(100, 30, {});
		try {
			const dir = tabs.mode.sessionManager.getSessionDir();
			const second = SessionManager.create(tabs.mode.sessionManager.getCwd(), dir);
			await second.setSessionName("Second capture tab", "user");
			await second.ensureOnDisk();
			const file = second.getSessionFile();
			if (!file) throw new Error("Second session has no file");
			await second.close();
			const opened = await tabs.mode.handleSessionTabsCommand(`open ${path.basename(file, ".jsonl")}`);
			expect(opened).toContain("Opened session tab");
			await capture(tabs.terminal, "cap-100x30-tabs");
			expect(tabs.terminal.getViewport().length).toBeLessThanOrEqual(30);
			// Closing every tab displays Home while the session keeps running.
			await tabs.mode.handleSessionTabsCommand("close 2");
			const last = await tabs.mode.handleSessionTabsCommand("close 1");
			expect(last).toContain("keeps running");
			expect(tabs.mode.isHomeDetached()).toBe(true);
			await capture(tabs.terminal, "cap-100x30-lasthome");
			expect(tabs.terminal.getViewport().length).toBeLessThanOrEqual(30);
		} finally {
			await tabs.teardown();
		}
		// Palette selection near its lower boundary at 80x24.
		const palette = await boot(80, 24, {});
		try {
			await palette.mode.openCommandPalette();
			for (let i = 0; i < 14; i++) palette.terminal.sendInput("\x1b[B");
			await capture(palette.terminal, "cap-80x24-palette-bottom");
			expect(palette.terminal.getViewport().length).toBeLessThanOrEqual(24);
		} finally {
			await palette.teardown();
		}
		// ASCII home.
		const asciiHome = await boot(80, 24, { ascii: true, empty: true });
		try {
			await capture(asciiHome.terminal, "cap-80x24-ascii-home");
		} finally {
			await asciiHome.teardown();
		}
		// Hub Sessions section with live + archived rows and actions.
		const hub = await boot(100, 30, {});
		try {
			const dir = hub.mode.sessionManager.getSessionDir();
			const archived = SessionManager.create(hub.mode.sessionManager.getCwd(), dir);
			await archived.setSessionName("Archived capture session", "user");
			await archived.ensureOnDisk();
			const archivedFile = archived.getSessionFile();
			if (!archivedFile) throw new Error("Archived session has no file");
			const archivedId = archived.getSessionId();
			await archived.close();
			const opened = await hub.mode.handleSessionTabsCommand(`open ${path.basename(archivedFile, ".jsonl")}`);
			expect(opened).toContain("Opened session tab");
			const facade = hub.mode.sessions;
			if (!facade) throw new Error("Session facade is unavailable in capture boot");
			await facade.archive(archivedId);
			expect(facade.inspect(archivedId)?.archived).toBe(true);
			hub.mode.showAgentHub({ initialSection: "sessions" });
			await hub.terminal.waitForRender(() => hub.terminal.getViewport().some(row => row.includes("Sessions")));
			await capture(hub.terminal, "cap-100x30-hub-sessions");
			expect(hub.terminal.getViewport().length).toBeLessThanOrEqual(30);
		} finally {
			await hub.teardown();
		}
	}, 180000);
});
