/** Isolated, provider-free captures through the production InteractiveMode routes.
 * Run with FORCE_COLOR=3 (or 0) and optional COLORFGBG=0;15 for the light slot:
 * bun bench/tui-visual-capture.ts OUTPUT_DIR [unicode|ascii]
 */
import * as path from "node:path";
import { Agent } from "@harvest/pi-agent-core";
import { setAgentDir, TempDir } from "@harvest/pi-utils";
import { VirtualTerminal } from "../../tui/test/virtual-terminal";
import { ModelRegistry } from "../src/config/model-registry";
import { resetSettingsForTest, Settings } from "../src/config/settings";
import { initConfiguredTheme } from "../src/config/theme";
import { Composer } from "../src/modes/composer";
import { InteractiveMode } from "../src/modes/interactive-mode";
import { getCurrentThemeName } from "../src/modes/theme/theme";
import type { SymbolPreset } from "../src/modes/theme/symbols";
import { AgentSession } from "../src/session/agent-session";
import { AuthStorage } from "../src/session/auth-storage";
import { SessionManager } from "../src/session/session-manager";

const output = process.argv[2];
if (!output) throw new Error("Pass an output directory for the isolated captures.");
const preset: SymbolPreset = process.argv[3] === "ascii" ? "ascii" : "unicode";
const profile = process.argv[4] ?? "representative";
const scenes = ["home", "session", "palette", "settings", "models", "sessions", "tree", "agents"] as const;
type Scene = (typeof scenes)[number];
const sizes: ReadonlyArray<readonly [number, number]> =
	profile === "resize"
		? [
				[160, 45],
				[121, 32],
				[120, 32],
				[100, 30],
				[80, 24],
				[60, 16],
				[80, 10],
				[60, 8],
				[24, 4],
				[100, 30],
			]
		: [
				[100, 30],
				[24, 4],
				[100, 30],
			];
const captures: Array<{ scene: Scene; theme?: string; file: string; columns: number; rows: number }> = [];
const retainedFixtures: string[] = [];

function terminalSize(columns: number, rows: number): void {
	Object.defineProperty(process.stdout, "columns", { value: columns, configurable: true });
	Object.defineProperty(process.stdout, "rows", { value: rows, configurable: true });
}

for (const scene of scenes.filter(scene => profile !== "resize" || scene === "palette" || scene === "settings")) {
	const directory = TempDir.createSync("@harvest-visual-capture-");
	const cwd = directory.path();
	setAgentDir(path.join(cwd, "agent"));
	await Settings.init({ inMemory: true, cwd, overrides: { symbolPreset: preset, "tui.fullscreen": true } });
	await initConfiguredTheme(Settings.instance);
	const auth = await AuthStorage.create(path.join(cwd, "auth.db"));
	const registry = new ModelRegistry(auth);
	const model = registry.find("anthropic", "claude-sonnet-4-5");
	if (!model) throw new Error("Bundled fixture model unavailable.");
	const messages =
		scene === "home"
			? []
			: [
					{
						role: "user" as const,
						content: "Review the terminal interface. Preserve all session controls and this draft.",
						timestamp: 1_000,
					},
				];
	const manager = SessionManager.create(cwd, path.join(cwd, "sessions"));
	await manager.setSessionName("Visual fixture", "user");
	for (const message of messages) manager.appendMessage(message);
	await manager.ensureOnDisk();
	const session = new AgentSession({
		agent: new Agent({ initialState: { model, tools: [], messages } }),
		sessionManager: manager,
		settings: Settings.isolated({ symbolPreset: preset, "tui.fullscreen": true }),
		modelRegistry: registry,
	});
	const terminal = new VirtualTerminal(100, 30);
	terminalSize(100, 30);
	const mode = new InteractiveMode(
		session,
		"visual-fixture",
		undefined,
		() => {},
		undefined,
		undefined,
		undefined,
		new Composer({ terminal }),
	);
	try {
		await mode.init({ suppressWelcomeIntro: true });
		await mode.renderInitialMessages();
		void mode.getUserInput();
		mode.editor.setText("Draft survives resize");
		await terminal.waitForRender();
		switch (scene) {
			case "palette":
				await mode.openCommandPalette();
				break;
			case "settings":
				mode.showSettingsSelector();
				break;
			case "models":
				mode.showModelSelector();
				break;
			case "sessions":
				mode.showSessionSelector();
				break;
			case "tree":
				mode.showTreeSelector();
				break;
			case "agents":
				mode.showAgentsDashboard();
				break;
		}
		await terminal.waitForRender();
		for (const [index, [columns, rows]] of sizes.entries()) {
			terminalSize(columns, rows);
			terminal.resize(columns, rows);
			mode.ui.requestRender();
			// The production renderer uses a 120ms transient resize buffer.
			await Bun.sleep(160);
			await terminal.waitForRender();
			const file = `${scene}-${index}-${columns}x${rows}.json`;
			await Bun.write(
				path.join(output, file),
				JSON.stringify({
					label: `InteractiveMode ${scene}, ${preset}, ${getCurrentThemeName()}; isolated/no provider call`,
					columns,
					rows,
					viewport: terminal.getViewport(),
					cells: terminal.getViewportCellRows(),
				}),
			);
			captures.push({ scene, theme: getCurrentThemeName(), file, columns, rows });
		}
	} finally {
		mode.stop();
		await mode.liveSessions.dispose();
		await session.dispose();
		auth.close();
		await manager.close();
		resetSettingsForTest();
		try {
			await directory.remove();
		} catch {
			retainedFixtures.push(cwd);
		}
	}
}
await Bun.write(
	path.join(output, "manifest.json"),
	JSON.stringify({ profile, preset, captures, retainedFixtures }, null, 2),
);
process.stdout.write(`Captured ${captures.length} production-route frames in ${path.resolve(output)}\n`);
