import { afterEach, beforeAll, describe, expect, it, vi } from "bun:test";
import type { Component } from "@harvest/pi-tui";
import type { SessionSelectorComponent } from "@harvest/pi-coding-agent/modes/components/session-selector";
import { SelectorController } from "@harvest/pi-coding-agent/modes/controllers/selector-controller";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { InteractiveModeContext } from "@harvest/pi-coding-agent/modes/types";
import type { SessionInfo } from "@harvest/pi-coding-agent/session/session-listing";
import { SessionManager } from "@harvest/pi-coding-agent/session/session-manager";

beforeAll(async () => {
	await initTheme();
});

afterEach(() => {
	vi.restoreAllMocks();
});

interface EditorSlot {
	children: unknown[];
	clear: () => void;
	addChild: (child: unknown) => void;
}

function createEditorSlot(...initial: unknown[]): EditorSlot {
	return {
		children: [...initial],
		clear() {
			this.children = [];
		},
		addChild(child: unknown) {
			this.children.push(child);
		},
	};
}

function createCtx(slot: EditorSlot, editor: unknown) {
	const setFocus = vi.fn();
	const ctx = {
		editor,
		editorContainer: slot,
		sessionManager: { getSessionFile: () => undefined },
		ui: {
			setFocus,
			requestRender: vi.fn(),
		},
	} as unknown as InteractiveModeContext;
	return { ctx, setFocus };
}

describe("SelectorController.focusActiveEditorArea", () => {
	// Regression for issue #3349: closing a fullscreen overlay (settings,
	// extensions dashboard, agents dashboard) while a hook selector / approval
	// prompt occupies the editor slot must restore focus to that prompt — not
	// to the editor that the prompt replaced. Pre-fix, the close handlers
	// hardcoded `setFocus(this.ctx.editor)`, leaving keystrokes routed to a
	// no-longer-mounted editor while the visible prompt sat unreachable.

	it("focuses the editor when the slot has only the editor in it", () => {
		const editor = { id: "editor" };
		const slot = createEditorSlot(editor);
		const { ctx, setFocus } = createCtx(slot, editor);

		new SelectorController(ctx).focusActiveEditorArea();

		expect(setFocus).toHaveBeenCalledTimes(1);
		expect(setFocus).toHaveBeenCalledWith(editor);
	});

	it("focuses the active hook-selector-style prompt when the slot holds it instead of the editor", () => {
		const editor = { id: "editor" };
		const approvalPrompt = { id: "approval-prompt" };
		// Mirrors `ExtensionUiController.showHookSelector`: the hook surface
		// clears the slot and replaces the editor with its prompt component.
		const slot = createEditorSlot(approvalPrompt);
		const { ctx, setFocus } = createCtx(slot, editor);

		new SelectorController(ctx).focusActiveEditorArea();

		expect(setFocus).toHaveBeenCalledTimes(1);
		expect(setFocus).toHaveBeenCalledWith(approvalPrompt);
		expect(setFocus).not.toHaveBeenCalledWith(editor);
	});

	it("falls back to the editor when the slot is empty (defensive)", () => {
		const editor = { id: "editor" };
		const slot = createEditorSlot();
		const { ctx, setFocus } = createCtx(slot, editor);

		new SelectorController(ctx).focusActiveEditorArea();

		expect(setFocus).toHaveBeenCalledTimes(1);
		expect(setFocus).toHaveBeenCalledWith(editor);
	});
});

describe("SelectorController editor-slot close paths", () => {
	// `showSelector` done and `closeOverlayToEditorArea` (the palette/sidebar
	// close wiring) must never evict a hook selector/input/editor mounted
	// while the selector or overlay was up: restore the editor only when it
	// still owns the slot, then focus the visible slot owner.

	it("showSelector done restores the editor in the common case", () => {
		const editor = { id: "editor" };
		const selectorView = { id: "selector" } as unknown as Component;
		const focusTarget = { id: "selector-focus" } as unknown as Component;
		const slot = createEditorSlot(editor);
		const { ctx, setFocus } = createCtx(slot, editor);

		let done!: () => void;
		new SelectorController(ctx).showSelector(release => {
			done = release;
			return { component: selectorView, focus: focusTarget };
		});
		expect(slot.children).toEqual([selectorView]);
		expect(setFocus).toHaveBeenLastCalledWith(focusTarget);

		done();
		expect(slot.children).toEqual([editor]);
		expect(setFocus).toHaveBeenLastCalledWith(editor);
	});

	it("showSelector done keeps a hook widget mounted meanwhile and focuses it", () => {
		const editor = { id: "editor" };
		const hookPrompt = { id: "hook-prompt" };
		const selectorView = { id: "selector" } as unknown as Component;
		const focusTarget = { id: "selector-focus" } as unknown as Component;
		const slot = createEditorSlot(editor);
		const { ctx, setFocus } = createCtx(slot, editor);

		let done!: () => void;
		new SelectorController(ctx).showSelector(release => {
			done = release;
			return { component: selectorView, focus: focusTarget };
		});
		// An approval prompt replaces the selector before done runs.
		slot.clear();
		slot.addChild(hookPrompt);
		done();
		expect(slot.children).toEqual([hookPrompt]);
		expect(setFocus).toHaveBeenLastCalledWith(hookPrompt);
		expect(setFocus).not.toHaveBeenLastCalledWith(editor);
	});

	it("closeOverlayToEditorArea hides and refocuses the slot owner", () => {
		const editor = { id: "editor" };
		const approvalPrompt = { id: "approval-prompt" };
		const slot = createEditorSlot(approvalPrompt);
		const { ctx, setFocus } = createCtx(slot, editor);
		const hide = vi.fn();

		new SelectorController(ctx).closeOverlayToEditorArea({
			hide,
			setHidden: vi.fn(),
			isHidden: () => false,
		});

		expect(hide).toHaveBeenCalledTimes(1);
		expect(setFocus).toHaveBeenCalledWith(approvalPrompt);
		expect(ctx.ui.requestRender).toHaveBeenCalled();
	});
});

describe("SelectorController session replacement overlay", () => {
	it("keeps the picker open until a new session has actually been created", async () => {
		vi.spyOn(SessionManager, "list").mockResolvedValue([]);
		let sessionId = "current";
		const create = Promise.withResolvers<void>();
		const hide = vi.fn();
		let selector: SessionSelectorComponent | undefined;
		const ctx = {
			sessionManager: {
				getSessionFile: () => undefined,
				getSessionId: () => sessionId,
				getCwd: () => "/tmp",
				getSessionDir: () => "/tmp",
			},
			handleClearCommand: vi.fn(() => create.promise),
			editor: {},
			editorContainer: createEditorSlot(),
			ui: {
				showOverlay: vi.fn(component => {
					selector = component as SessionSelectorComponent;
					return { hide, setHidden: vi.fn(), isHidden: () => false };
				}),
				setFocus: vi.fn(),
				requestRender: vi.fn(),
				terminal: { rows: 24 },
			},
		} as unknown as InteractiveModeContext;
		const controller = new SelectorController(ctx);
		await controller.showSessionSelector();
		const row = selector!.render(80).findIndex(line => line.includes("+ New session"));
		expect(row).toBeGreaterThanOrEqual(0);
		selector!.handleInput(`\x1b[<0;4;${row + 1}M`);
		expect(ctx.handleClearCommand).toHaveBeenCalledTimes(1);
		expect(hide).not.toHaveBeenCalled();
		sessionId = "next";
		create.resolve();
		await create.promise;
		await Bun.sleep(0);
		expect(hide).toHaveBeenCalledTimes(1);
	});

	it("keeps the fullscreen selector visible until the resumed transcript is ready", async () => {
		const session: SessionInfo = {
			path: "/tmp/resume.jsonl",
			id: "resume",
			cwd: "/tmp",
			title: "Resume target",
			created: new Date("2026-01-01T00:00:00Z"),
			modified: new Date("2026-01-02T00:00:00Z"),
			messageCount: 2,
			size: 1,
			firstMessage: "first",
			allMessagesText: "first second",
		};
		vi.spyOn(SessionManager, "list").mockResolvedValue([session]);

		const overlayHidden = Promise.withResolvers<void>();
		const hide = vi.fn(() => overlayHidden.resolve());
		let selector: SessionSelectorComponent | undefined;
		const editor = { id: "editor" };
		const editorContainer = createEditorSlot(editor);
		const ctx = {
			editor,
			editorContainer,
			sessionManager: {
				getSessionFile: () => undefined,
				getCwd: () => "/tmp",
				getSessionDir: () => "/tmp",
			},
			ui: {
				showOverlay: vi.fn(component => {
					selector = component as SessionSelectorComponent;
					return { hide, setHidden: vi.fn(), isHidden: () => false };
				}),
				setFocus: vi.fn(),
				requestRender: vi.fn(),
				terminal: { rows: 24 },
			},
		} as unknown as InteractiveModeContext;
		const controller = new SelectorController(ctx);
		const resumeStarted = Promise.withResolvers<void>();
		const resumed = Promise.withResolvers<boolean>();
		const handleResume = vi.spyOn(controller, "handleResumeSession").mockImplementation(() => {
			resumeStarted.resolve();
			return resumed.promise;
		});
		await controller.showSessionSelector();
		expect(selector).toBeDefined();
		selector!.handleInput("\n");
		await resumeStarted.promise;

		expect(handleResume).toHaveBeenCalledWith(session.path);
		expect(hide).not.toHaveBeenCalled();

		// The selector remains mounted until resume finishes, but it must not accept
		// a second selection or cancel the overlay during that interval.
		selector!.handleInput("\n");
		selector!.handleInput("\x1b");
		expect(handleResume).toHaveBeenCalledTimes(1);
		expect(hide).not.toHaveBeenCalled();

		resumed.resolve(true);
		await overlayHidden.promise;
		expect(hide).toHaveBeenCalledTimes(1);
	});
});
