import { describe, expect, it } from "bun:test";
import {
	clampDialogWidth,
	composerMaxHeight,
	computeWorkspaceLayout,
	pickDialogCap,
	resolveWorkspaceComposerShape,
	sidebarOverlayWidth,
	WORKSPACE_LAYOUT,
} from "@harvest/pi-coding-agent/modes/workspace-layout";

describe("workspace layout geometry", () => {
	it("docks the sidebar when wide and hides it when narrow", () => {
		const wide = computeWorkspaceLayout(
			{ columns: 160, rows: 45 },
			{ hasConversation: true, sidebarPreference: "auto" },
		);
		expect(wide.sidebarDocked).toBe(true);
		expect(wide.sidebar?.width).toBe(WORKSPACE_LAYOUT.sidebarWidth);
		expect(wide.main.width).toBe(160 - WORKSPACE_LAYOUT.sidebarWidth);

		const narrow = computeWorkspaceLayout(
			{ columns: 80, rows: 24 },
			{ hasConversation: true, sidebarPreference: "auto" },
		);
		expect(narrow.sidebarDocked).toBe(false);
		expect(narrow.sidebar).toBeUndefined();
	});

	it("crosses the dock/hidden breakpoint without corrupting widths", () => {
		const docked = computeWorkspaceLayout(
			{ columns: 121, rows: 32 },
			{ hasConversation: true, sidebarPreference: "auto" },
		);
		const hidden = computeWorkspaceLayout(
			{ columns: 120, rows: 32 },
			{ hasConversation: true, sidebarPreference: "auto" },
		);
		expect(docked.sidebarDocked).toBe(true);
		expect(hidden.sidebarDocked).toBe(false);
		expect(docked.main.width + (docked.sidebar?.width ?? 0)).toBe(121);
		expect(hidden.main.width).toBe(120);
		expect(hidden.transcriptWidth).toBeGreaterThan(0);
		expect(hidden.composerWidth).toBeGreaterThan(0);
	});

	it("requests an overlay instead of crushing the transcript for narrow show", () => {
		const narrow = computeWorkspaceLayout(
			{ columns: 80, rows: 24 },
			{ hasConversation: true, sidebarPreference: "show", sidebarOverlayOpen: true },
		);
		expect(narrow.sidebarDocked).toBe(false);
		expect(narrow.sidebarOverlayRequested).toBe(true);
		expect(narrow.main.width).toBe(80);
	});

	it("clamps dialogs to 60/88/116 caps and viewport", () => {
		expect(pickDialogCap(160)).toBe(116);
		expect(pickDialogCap(100)).toBe(88);
		expect(pickDialogCap(80)).toBe(60);
		expect(clampDialogWidth(80, 116)).toBe(78);
		expect(clampDialogWidth(20, 60)).toBe(18);
	});

	it("maps fullscreen band to rail consistently", () => {
		expect(resolveWorkspaceComposerShape(true, "band")).toBe("rail");
		expect(resolveWorkspaceComposerShape(false, "band")).toBe("band");
		expect(resolveWorkspaceComposerShape(true, "box")).toBe("box");
	});

	it("keeps the composer editable at 24x4 and 20x4", () => {
		expect(composerMaxHeight(4)).toBeGreaterThanOrEqual(1);
		expect(composerMaxHeight(24)).toBeLessThanOrEqual(8);
		const tiny = computeWorkspaceLayout(
			{ columns: 20, rows: 4 },
			{ hasConversation: false, sidebarPreference: "auto" },
		);
		expect(tiny.main.width).toBe(20);
		expect(tiny.homePromptWidth).toBeGreaterThan(0);
		expect(sidebarOverlayWidth(20)).toBeGreaterThan(0);
	});

	it("routes mouse through final rectangles", () => {
		const wide = computeWorkspaceLayout(
			{ columns: 160, rows: 45 },
			{ hasConversation: true, sidebarPreference: "auto" },
		);
		expect(wide.hitTest(10)).toBe("main");
		expect(wide.hitTest(159)).toBe("sidebar");
		expect(wide.hitTest(200)).toBe("outside");
	});

	it("caps the home prompt at 75 columns", () => {
		expect(WORKSPACE_LAYOUT.homePromptMaxWidth).toBe(75);
		const wide = computeWorkspaceLayout(
			{ columns: 160, rows: 45 },
			{ hasConversation: false, sidebarPreference: "auto" },
		);
		expect(wide.homePromptWidth).toBe(75);
	});
});
