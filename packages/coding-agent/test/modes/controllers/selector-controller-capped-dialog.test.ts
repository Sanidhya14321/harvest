import { afterEach, beforeEach, describe, expect, test, vi } from "bun:test";
import { Effort } from "@harvest/pi-ai";
import { OverlayPanel } from "@harvest/pi-coding-agent/modes/components/overlay-box";
import { PluginSelectorComponent } from "@harvest/pi-coding-agent/modes/components/plugin-selector";
import { SessionSelectorComponent } from "@harvest/pi-coding-agent/modes/components/session-selector";
import { ThinkingSelectorComponent } from "@harvest/pi-coding-agent/modes/components/thinking-selector";
import { SelectorController } from "@harvest/pi-coding-agent/modes/controllers/selector-controller";
import { createTheme, getBuiltinThemes } from "@harvest/pi-coding-agent/modes/theme/loader";
import { setThemeInstance, theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { InteractiveModeContext } from "@harvest/pi-coding-agent/modes/types";
import type { SessionInfo } from "@harvest/pi-coding-agent/session/session-listing";
import { SessionManager } from "@harvest/pi-coding-agent/session/session-manager";
import { Container, Input, Text, TUI } from "@harvest/pi-tui";
import { CELL_U32 } from "kitty-vt-wasm";
import { StressRenderScheduler } from "../../../../tui/test/render-stress-scheduler";
import { VirtualTerminal } from "../../../../tui/test/virtual-terminal";

let previousTheme = theme;
const activeUis: TUI[] = [];
beforeEach(() => {
	previousTheme = theme;
	setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "truecolor", symbolPresetOverride: "ascii" }));
});
afterEach(() => {
	for (const ui of activeUis.splice(0)) ui.stop();
	vi.restoreAllMocks();
	setThemeInstance(previousTheme);
});

function createHost(columns = 160, rows = 24) {
	const term = new VirtualTerminal(columns, rows);
	const scheduler = new StressRenderScheduler();
	const ui = new TUI(term, undefined, { renderScheduler: scheduler });
	const editor = new Input();
	editor.setValue("Draft retained");
	const editorContainer = new Container();
	editorContainer.addChild(editor);
	ui.addChild(new Text("Transcript retained", 0, 0));
	ui.addChild(editorContainer);
	ui.setFocus(editor);
	const ctx = {
		ui,
		editor,
		editorContainer,
		sessionManager: {
			getSessionFile: () => undefined,
			getSessionId: () => "current",
			getCwd: () => "/tmp",
			getSessionDir: () => "/tmp",
		},
	} as unknown as InteractiveModeContext;
	const controller = new SelectorController(ctx);
	activeUis.push(ui);
	ui.start();
	return { term, scheduler, ui, editor, editorContainer, controller };
}

describe("production capped selector mounts", () => {
	test("thinking child controls accept the first keyboard event while background focus stays fenced", async () => {
		const { term, scheduler, ui, editor, controller } = createHost();
		const selected = vi.fn();
		controller.showSelector(done => {
			const selector = new ThinkingSelectorComponent(
				Effort.Low,
				[Effort.Low, Effort.Medium, Effort.High],
				level => {
					selected(level);
					done();
				},
				done,
			);
			return { component: selector, focus: selector.getSelectList() };
		});
		// A modal child must own input before its first paint, even if another
		// command tries to focus the editor while the dialog is active.
		ui.setFocus(editor);
		term.sendInput("\x1b[B");
		term.sendInput("\r");
		await scheduler.drain(term);
		expect(selected).toHaveBeenCalledWith(Effort.Medium);
		expect(ui.hasOverlay()).toBe(false);
		expect(editor.getValue()).toBe("Draft retained");
		term.sendInput("!");
		expect(editor.getValue()).toBe("Draft retained!");
		expect(term.getViewport().join("\n")).toContain("Transcript retained");
	});

	test("a centered plugin dialog clips outside and footer clicks and selects the complete scoped plugin", async () => {
		const { term, scheduler, ui, editor, controller } = createHost();
		const selected = vi.fn();
		let selector!: PluginSelectorComponent;
		controller.showSelector(done => {
			selector = new PluginSelectorComponent(
				1,
				[
					{ plugin: { name: "alpha", description: "First plugin" }, marketplace: "local", scope: "user" },
					{ plugin: { name: "beta", description: "Second plugin" }, marketplace: "local", scope: "project" },
				],
				new Set(),
				{
					onSelect: (name, marketplace, scope) => {
						selected(name, marketplace, scope);
						done();
					},
					onCancel: done,
				},
			);
			return { component: selector, focus: selector.getSelectList() };
		}, 88);
		await scheduler.drain(term);
		const bounds = ui.getOverlayBounds(selector)!;
		expect(bounds.width).toBe(88);
		expect(bounds.col).toBe(36);
		const alphaRow = term.getViewport().findIndex(line => line.includes("alpha"));
		const selectedCells = term.getViewportCellRows()[alphaRow]!;
		// Every selected cell, including both outer insets, shares one band.
		const selectedBackground = selectedCells[(bounds.col + 4) * CELL_U32 + 4];
		for (let col = bounds.col; col < bounds.col + bounds.width; col++)
			expect(selectedCells[col * CELL_U32 + 4]).toBe(selectedBackground);
		const betaRow = term.getViewport().findIndex(line => line.includes("beta"));
		expect(betaRow).toBeGreaterThanOrEqual(bounds.row);
		term.sendInput(`\x1b[<0;1;${betaRow + 1}M`);
		const footerRow = term.getViewport().findIndex(line => line.includes("Enter select"));
		expect(footerRow).toBeGreaterThanOrEqual(bounds.row);
		term.sendInput(`\x1b[<0;${bounds.col + 5};${footerRow + 1}M`);
		expect(selected).not.toHaveBeenCalled();
		term.sendInput(`\x1b[<0;${bounds.col + 5};${betaRow + 1}M`);
		await scheduler.drain(term);
		expect(selected).toHaveBeenCalledWith("beta", "local", "project");
		expect(ui.hasOverlay()).toBe(false);
		expect(editor.getValue()).toBe("Draft retained");
	});

	test("nested modal input and cancellation preserve an approval prompt mounted during the dialog", async () => {
		const { term, scheduler, editor, editorContainer, controller } = createHost();
		const dialogInput = new Input();
		const nested = new Container();
		nested.addChild(dialogInput);
		controller.showSelector(done => {
			const panel = new OverlayPanel("Nested input");
			panel.addChild(nested);
			dialogInput.onEscape = done;
			return { component: panel, focus: dialogInput };
		});
		term.sendInput("Complete value");
		expect(dialogInput.getValue()).toBe("Complete value");
		const approval = new Input();
		approval.setValue("Approval");
		editorContainer.clear();
		editorContainer.addChild(approval);
		term.sendInput("\x1b");
		await scheduler.drain(term);
		term.sendInput("!");
		expect(editorContainer.children).toEqual([approval]);
		expect(approval.getValue()).toBe("Approval!");
		expect(editor.getValue()).toBe("Draft retained");
	});

	test("the actual session route retains selection across tiny allocation and rebases a click into pending resume", async () => {
		const sessions: SessionInfo[] = Array.from({ length: 12 }, (_, index) => ({
			id: `id-${index}`,
			path: `/tmp/session-${index}.jsonl`,
			cwd: "/tmp",
			title: `Choice ${index}`,
			created: new Date("2026-01-01"),
			modified: new Date("2026-01-02"),
			size: 1,
			messageCount: 2,
			firstMessage: `Prompt ${index}`,
			allMessagesText: `Prompt ${index}`,
		}));
		vi.spyOn(SessionManager, "list").mockResolvedValue(sessions);
		const { term, scheduler, ui, editor, controller } = createHost();
		const overlay = vi.spyOn(ui, "showOverlay");
		const resumed = Promise.withResolvers<boolean>();
		const resume = vi.spyOn(controller, "handleResumeSession").mockImplementation(() => resumed.promise);
		await controller.showSessionSelector();
		const selector = overlay.mock.calls.at(-1)![0];
		if (!(selector instanceof SessionSelectorComponent)) throw new Error("Session picker did not mount");
		await scheduler.drain(term);
		expect(ui.getOverlayBounds(selector)?.width).toBe(116);
		for (let index = 0; index < 7; index++) term.sendInput("\x1b[B");
		term.resize(24, 4);
		await Bun.sleep(120);
		await scheduler.drain(term);
		const bounds = ui.getOverlayBounds(selector)!;
		expect(bounds.width).toBe(24);
		expect(bounds.height).toBe(4);
		const choiceRow = term.getViewport().findIndex(line => line.includes("Choice 7"));
		expect(choiceRow).toBeGreaterThanOrEqual(0);
		term.sendInput(`\x1b[<0;5;${choiceRow + 1}M`);
		expect(resume).toHaveBeenCalledWith(sessions[7]!.path);
		term.sendInput("\r");
		term.sendInput("\x1b");
		expect(resume).toHaveBeenCalledTimes(1);
		expect(ui.hasOverlay()).toBe(true);
		resumed.resolve(true);
		await resumed.promise;
		await Bun.sleep(0);
		await scheduler.drain(term);
		expect(ui.hasOverlay()).toBe(false);
		expect(editor.getValue()).toBe("Draft retained");
	});
});
