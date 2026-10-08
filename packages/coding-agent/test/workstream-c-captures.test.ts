/**
 * Workstream C captures: palette (80x24 + 24x4 ultra-compact), the /agents
 * hub managed-revision manager (draft row, evaluating, inspect), the skill
 * revision manager, and the attachment chip band, rasterized via
 * bench/render-terminal-captures.py.
 *
 * Captures only when HARVEST_TERMINAL_CAPTURE_DIR is set; otherwise the
 * scenes still execute as smoke (no-ops for evidence, never failures).
 */
import { afterEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { Agent } from "@harvest/pi-agent-core";
import { TempDir } from "@harvest/pi-utils";
import { getAgentDir, setAgentDir } from "@harvest/pi-utils/dirs";
import { VirtualTerminal } from "../../tui/test/virtual-terminal";
import { KeybindingsManager } from "../src/config/keybindings";
import { ModelRegistry } from "../src/config/model-registry";
import { resetSettingsForTest, Settings } from "../src/config/settings";
import { Composer } from "../src/modes/composer";
import { AgentsHubComponent } from "../src/modes/components/agents-hub";
import { AttachmentChipsBand } from "../src/modes/components/attachment-chips";
import { CustomEditor } from "../src/modes/components/custom-editor";
import { SkillRevisionsComponent } from "../src/modes/components/skill-revisions";
import { getEditorTheme, initTheme } from "../src/modes/theme/theme";
import { InteractiveMode } from "../src/modes/interactive-mode";
import { AuthStorage } from "../src/session/auth-storage";
import { AgentSession } from "../src/session/agent-session";
import { SessionManager } from "../src/session/session-manager";
import * as discovery from "../src/task/discovery";
import { createPresetDraft, setPresetEvalRunner } from "../src/task/agents";
import {
	createSkillDraft,
	evaluateSkillRevision,
	setSkillEvalRunner,
} from "../src/autolearn/managed-skills";
import type { TUI } from "@harvest/pi-tui";
import { ImageBudget } from "@harvest/pi-tui";
import { setKeybindings } from "@harvest/pi-tui";

const ANSI_PATTERN = /\x1b\[[0-?]*[ -/]*[@-~]/g;

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

describe("workstream C captures", () => {
	const cleanups: Array<() => Promise<void>> = [];
	afterEach(async () => {
		for (const cleanup of cleanups.splice(0)) await cleanup();
		vi.restoreAllMocks();
		resetSettingsForTest();
		setAgentDir((globalThis as { __cCaptureAgentDir?: string }).__cCaptureAgentDir ?? getAgentDir());
	});

	it("palette at 80x24 and ultra-compact 24x4", async () => {
		const directory = TempDir.createSync("@harvest-c-capture-palette-");
		await Settings.init({ inMemory: true, cwd: directory.path() });
		Settings.instance.set("tui.fullscreen", true);
		setKeybindings(KeybindingsManager.inMemory());
		await initTheme(false);
		const auth = await AuthStorage.create(path.join(directory.path(), "auth.db"));
		const registry = new ModelRegistry(auth);
		const model = registry.find("anthropic", "claude-sonnet-4-5");
		if (!model) throw new Error("Missing test model");
		const manager = SessionManager.create(directory.path(), directory.path());
		manager.appendMessage({ role: "user", content: "Capture review", timestamp: Date.now() });
		await manager.ensureOnDisk();
		const session = new AgentSession({
			agent: new Agent({ initialState: { model, systemPrompt: ["Test"], tools: [], messages: [] } }),
			sessionManager: manager,
			settings: Settings.isolated(),
			modelRegistry: registry,
		});
		const terminal = new VirtualTerminal(80, 24);
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
		const teardown = async (): Promise<void> => {
			mode?.stop();
			await mode?.liveSessions?.dispose();
			await session?.dispose();
			auth?.close();
			await directory?.remove();
			resetSettingsForTest();
		};
		cleanups.push(teardown);
		try {
			await mode.init({ suppressWelcomeIntro: true });
			await mode.renderInitialMessages();
			void mode.getUserInput();
			await terminal.waitForRender();
			await mode.openCommandPalette();
			await terminal.waitForRender();
			await capture(terminal, "c-palette-80x24");
			terminal.resize(24, 4);
			process.stdout.emit("resize");
			await terminal.waitForRender();
			const text = terminal
				.getViewport()
				.map(row => Bun.stripANSI(row))
				.join("\n");
			expect(text.length).toBeGreaterThan(0);
			await capture(terminal, "c-palette-24x4");
		} finally {
			await teardown();
			cleanups.splice(cleanups.indexOf(teardown), 1);
		}
	}, 60000);

	it("hub managed-revision manager", async () => {
		setKeybindings(KeybindingsManager.inMemory());
		await initTheme(false);
		const tempAgentDir = await fs.mkdtemp(path.join(os.tmpdir(), "omp-c-capture-agentdir-"));
		const original = getAgentDir();
		(globalThis as { __cCaptureAgentDir?: string }).__cCaptureAgentDir = original;
		setAgentDir(path.join(tempAgentDir, "agent"));
		try {
			const tempCwd = await fs.mkdtemp(path.join(os.tmpdir(), "omp-c-capture-cwd-"));
			try {
				const draft = await createPresetDraft({
					name: "capture-managed-demo",
					description: "Use this agent when demonstrating the hub revision manager.",
					systemPrompt: "You demonstrate revision management.",
				});
				setPresetEvalRunner(async () => ({ passed: true, summary: "demo pass" }));
				const { evaluatePresetRevision } = await import("../src/task/agents");
				await evaluatePresetRevision("capture-managed-demo", draft.id, {
					task: "demo task",
					expectedOutcome: "demo outcome",
				});
				vi.spyOn(discovery, "discoverAgents").mockResolvedValue({
					projectAgentsDir: null,
					agents: [
						{
							name: "capture-managed-demo",
							description: "Use this agent when demonstrating the hub revision manager.",
							systemPrompt: "",
							source: "user",
							filePath: path.join(tempAgentDir, "agent", "managed-presets", "capture-managed-demo.md"),
						},
					],
				});
				const tuiStub = { requestRender: () => {}, terminal: { rows: 30 } } as unknown as TUI;
				const hub = await AgentsHubComponent.create(
					tuiStub,
					tempCwd,
					Settings.isolated(),
					{},
					{ onCancel: () => {} },
				);
				hub.handleInput("\r"); // agent strip
				hub.handleInput("\x1b[C");
				hub.handleInput("\x1b[C");
				hub.handleInput("\x1b[C"); // → revisions…
				hub.handleInput("\r");
				const start = Date.now();
				while (!hub.render(100).join("\n").replace(ANSI_PATTERN, "").includes("passing")) {
					if (Date.now() - start > 8000) throw new Error("revision manager did not load");
					await Bun.sleep(25);
				}
				const terminal = new VirtualTerminal(100, 30);
				terminal.write(hub.render(100).join("\r\n"));
				await terminal.waitForRender();
				const text = terminal
					.getViewport()
					.map(row => Bun.stripANSI(row))
					.join("\n");
				expect(text).toContain("capture-managed-demo");
				await capture(terminal, "c-hub-revisions-100x30");
			} finally {
				const { removeWithRetries } = await import("@harvest/pi-utils");
				await removeWithRetries(tempCwd);
			}
		} finally {
			setAgentDir(original);
			setPresetEvalRunner(undefined);
			const { removeWithRetries } = await import("@harvest/pi-utils");
			await removeWithRetries(tempAgentDir);
		}
	}, 60000);

	it("hub draft row, evaluating, and inspect states", async () => {
		setKeybindings(KeybindingsManager.inMemory());
		await initTheme(false);
		const tempAgentDir = await fs.mkdtemp(path.join(os.tmpdir(), "omp-c-capture-states-"));
		const original = getAgentDir();
		(globalThis as { __cCaptureAgentDir?: string }).__cCaptureAgentDir = original;
		setAgentDir(path.join(tempAgentDir, "agent"));
		try {
			const tempCwd = await fs.mkdtemp(path.join(os.tmpdir(), "omp-c-capture-states-cwd-"));
			try {
				const draft = await createPresetDraft({
					name: "capture-draft-demo",
					description: "Use this agent when demonstrating draft states.",
					systemPrompt: "You demonstrate draft states.",
				});
				vi.spyOn(discovery, "discoverAgents").mockResolvedValue({
					projectAgentsDir: null,
					agents: [
						{
							name: "dev",
							description: "Development agent",
							systemPrompt: "",
							source: "project",
						},
					],
				});
				// Blocking runner: the evaluating state stays up for capture.
				let releaseEval!: () => void;
				const gate = new Promise<void>(resolve => {
					releaseEval = resolve;
				});
				setPresetEvalRunner(async () => {
					await gate;
					return { passed: true, summary: "capture pass" };
				});
				const tuiStub = { requestRender: () => {}, terminal: { rows: 30 } } as unknown as TUI;
				const hub = await AgentsHubComponent.create(
					tuiStub,
					tempCwd,
					Settings.isolated(),
					{},
					{ onCancel: () => {} },
				);
				const plain = () => hub.render(100).join("\n").replace(ANSI_PATTERN, "");
				// Draft-only row with its badge (U1).
				const startRow = Date.now();
				while (!plain().includes("capture-draft-demo")) {
					if (Date.now() - startRow > 8000) throw new Error("draft row did not appear");
					await Bun.sleep(25);
				}
				expect(plain()).toContain("draft");
				const listTerminal = new VirtualTerminal(100, 30);
				listTerminal.write(hub.render(100).join("\r\n"));
				await listTerminal.waitForRender();
				await capture(listTerminal, "c-hub-draft-100x30");

				// Drive into the manager and start the evaluation.
				for (const char of "capture-draft-demo") hub.handleInput(char);
				hub.handleInput("\r");
				hub.handleInput("\x1b[C");
				hub.handleInput("\x1b[C");
				hub.handleInput("\x1b[C");
				hub.handleInput("\r");
				const startRev = Date.now();
				while (!plain().includes(draft.id)) {
					if (Date.now() - startRev > 8000) throw new Error("revision manager did not load");
					await Bun.sleep(25);
				}
				hub.handleInput("\r");
				hub.handleInput("\r");
				for (const char of "demo task") hub.handleInput(char);
				hub.handleInput("\r");
				for (const char of "demo outcome") hub.handleInput(char);
				hub.handleInput("\r");
				const startEval = Date.now();
				while (!plain().includes("evaluating…")) {
					if (Date.now() - startEval > 8000) throw new Error("evaluating state did not render");
					await Bun.sleep(25);
				}
				const evalTerminal = new VirtualTerminal(100, 30);
				evalTerminal.write(hub.render(100).join("\r\n"));
				await evalTerminal.waitForRender();
				expect(
					evalTerminal
						.getViewport()
						.map(row => Bun.stripANSI(row))
						.join("\n"),
				).toContain("evaluating");
				await capture(evalTerminal, "c-hub-evaluating-100x30");
				// Let the hub's own run settle (it records at the service
				// level) before teardown drops the runner slot.
				releaseEval();
				await Bun.sleep(500);
			} finally {
				setPresetEvalRunner(undefined);
				const { removeWithRetries } = await import("@harvest/pi-utils");
				await removeWithRetries(tempCwd);
			}
		} finally {
			setAgentDir(original);
			const { removeWithRetries } = await import("@harvest/pi-utils");
			await removeWithRetries(tempAgentDir);
		}
	}, 60000);

	it("skill revision manager with passing, failing, and unevaluated rows", async () => {
		setKeybindings(KeybindingsManager.inMemory());
		await initTheme(false);
		const tempAgentDir = await fs.mkdtemp(path.join(os.tmpdir(), "omp-c-capture-skill-"));
		const original = getAgentDir();
		(globalThis as { __cCaptureAgentDir?: string }).__cCaptureAgentDir = original;
		setAgentDir(path.join(tempAgentDir, "agent"));
		try {
			const passing = await createSkillDraft({
				name: "capture-skill-demo",
				description: "When to use the capture skill.",
				body: "Passing body.",
			});
			setSkillEvalRunner(async () => ({ passed: true, summary: "capture pass" }));
			await evaluateSkillRevision("capture-skill-demo", passing.id, {
				task: "demo",
				expectedOutcome: "demo",
			});
			const failing = await createSkillDraft({
				name: "capture-skill-demo",
				description: "When to use the capture skill.",
				body: "Failing body.",
			});
			setSkillEvalRunner(async () => ({ passed: false, summary: "capture fail" }));
			await evaluateSkillRevision("capture-skill-demo", failing.id, {
				task: "demo",
				expectedOutcome: "demo",
			});
			await createSkillDraft({
				name: "capture-skill-demo",
				description: "When to use the capture skill.",
				body: "Unevaluated body.",
			});
			setSkillEvalRunner(undefined);
			const tuiStub = { requestRender: () => {}, terminal: { rows: 30 } } as unknown as TUI;
			const manager = await SkillRevisionsComponent.create(tuiStub, "capture-skill-demo", {}, { onClose: () => {} });
			const plain = () => manager.render(100).join("\n").replace(ANSI_PATTERN, "");
			const start = Date.now();
			while (!plain().includes("unevaluated")) {
				if (Date.now() - start > 8000) throw new Error("skill manager did not load");
				await Bun.sleep(25);
			}
			expect(plain()).toContain("passing");
			expect(plain()).toContain("failing");
			const terminal = new VirtualTerminal(100, 30);
			terminal.write(manager.render(100).join("\r\n"));
			await terminal.waitForRender();
			await capture(terminal, "c-skill-manager-100x30");
		} finally {
			setAgentDir(original);
			setSkillEvalRunner(undefined);
			const { removeWithRetries } = await import("@harvest/pi-utils");
			await removeWithRetries(tempAgentDir);
		}
	}, 60000);

	it("attachment chip band at the 30-cell two-card edge", async () => {
		setKeybindings(KeybindingsManager.inMemory());
		await initTheme(false);
		const editor = new CustomEditor(getEditorTheme());
		editor.insertTextAttachment("first paste body");
		editor.insertTextAttachment("second paste body");
		const band = new AttachmentChipsBand(editor, new ImageBudget(8), () => {});
		const rows30 = band.render(30);
		expect(rows30.length).toBe(6);
		const terminal = new VirtualTerminal(60, 12);
		terminal.write(rows30.join("\r\n"));
		await terminal.waitForRender();
		const text = terminal
			.getViewport()
			.map(row => Bun.stripANSI(row))
			.join("\n");
		expect(text).toContain("#1");
		expect(text).toContain("#2");
		await capture(terminal, "c-attachments-60x12");
	});
});
