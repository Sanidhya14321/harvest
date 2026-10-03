import { beforeEach, describe, expect, it } from "bun:test";
import { Composer } from "@harvest/pi-coding-agent/modes/composer";
import {
	emptySidebarSnapshot,
	WorkspaceSidebar,
} from "@harvest/pi-coding-agent/modes/components/workspace-sidebar";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import {
	computeWorkspaceLayout,
	sidebarOverlayVisible,
} from "@harvest/pi-coding-agent/modes/workspace-layout";
import { visibleWidth, CURSOR_MARKER } from "@harvest/pi-tui";
import { VirtualTerminal } from "../../../tui/test/virtual-terminal";

beforeEach(async () => {
	await initTheme();
});

// CURSOR_MARKER is an internal editor sentinel the TUI strips after the
// frame; painted widths exclude it.
function paintedWidth(row: string): number {
	return visibleWidth(row.split(CURSOR_MARKER).join(""));
}

function richSidebar(): WorkspaceSidebar {
	const sidebar = new WorkspaceSidebar();
	sidebar.setSnapshot({
		...emptySidebarSnapshot(),
		title: "session",
		contextKnown: true,
		contextPercent: 12,
		contextTokens: 1200,
		contextWindow: 10000,
		costKnown: true,
		costLabel: "$0.01",
		mcpServers: [
			{ name: "fs", connected: true },
			{ name: "git", connected: false },
		],
		todos: Array.from({ length: 20 }, (_, i) => ({ label: `task ${i}`, done: false })),
		changes: Array.from({ length: 8 }, (_, i) => ({
			path: `src/file-${i}.ts`,
			staged: false,
			untracked: false,
			added: 1,
			removed: 0,
		})),
		changesState: "ready",
		version: "0.0.0",
	});
	return sidebar;
}

describe("sidebar overlay hit testing", () => {
	it("routes overlay columns to the sidebar only while the overlay is open", () => {
		const open = computeWorkspaceLayout(
			{ columns: 80, rows: 24 },
			{ hasConversation: true, sidebarPreference: "show", sidebarOverlayOpen: true },
		);
		expect(open.sidebarDocked).toBe(false);
		expect(open.sidebarOverlayRequested).toBe(true);
		expect(sidebarOverlayVisible(80, "show", true)).toBe(true);
		expect(open.hitTest(79)).toBe("sidebar");
		expect(open.hitTest(10)).toBe("main");
		expect(open.hitTest(200)).toBe("outside");

		const closed = computeWorkspaceLayout(
			{ columns: 80, rows: 24 },
			{ hasConversation: true, sidebarPreference: "show", sidebarOverlayOpen: false },
		);
		expect(sidebarOverlayVisible(80, "show", false)).toBe(false);
		expect(closed.hitTest(79)).toBe("main");
	});

	it("never routes overlay columns to the sidebar for auto/hide preferences", () => {
		expect(sidebarOverlayVisible(80, "auto", true)).toBe(false);
		expect(sidebarOverlayVisible(80, "hide", true)).toBe(false);
		expect(sidebarOverlayVisible(160, "show", true)).toBe(false);
		const auto = computeWorkspaceLayout(
			{ columns: 80, rows: 24 },
			{ hasConversation: true, sidebarPreference: "auto", sidebarOverlayOpen: true },
		);
		expect(auto.hitTest(79)).toBe("main");
	});
});

describe("sidebar per-session state", () => {
	it("clamps scroll offset to the viewport height in render", () => {
		const total = richSidebar().render(42).length;
		expect(total).toBeGreaterThan(10);
		const sidebar = richSidebar();
		sidebar.setScrollOffset(1000);
		const rows = sidebar.render(42, 10);
		expect(rows.length).toBe(10);
		expect(sidebar.scrollOffset).toBe(total - 10);
		expect(sidebar.render(42, 0)).toEqual([]);
	});

	it("moves focus and toggles the focused section by keyboard", () => {
		const sidebar = richSidebar();
		const titles = sidebar.sectionTitles();
		expect(titles.length).toBeGreaterThan(2);
		expect(sidebar.handleInput("down")).toBe(true);
		expect(sidebar.focusIndex).toBe(1);
		expect(sidebar.handleInput("up")).toBe(true);
		expect(sidebar.focusIndex).toBe(0);
		// Focus index 0 is the session title row, which never collapses.
		sidebar.handleInput("down");
		const focusedTitle = titles[sidebar.focusIndex]!;
		expect(sidebar.handleInput("enter")).toBe(true);
		expect(sidebar.isCollapsed(focusedTitle)).toBe(true);
		expect(sidebar.handleInput("pagedown")).toBe(true);
		expect(sidebar.handleInput("pageup")).toBe(true);
		expect(sidebar.handleInput("bogus-key")).toBe(false);
	});

	it("fires onClose on Escape", () => {
		const sidebar = richSidebar();
		let closed = 0;
		sidebar.setOnClose(() => {
			closed += 1;
		});
		expect(sidebar.handleInput("escape")).toBe(true);
		expect(closed).toBe(1);
	});

	it("isolates collapse state per session key", () => {
		const sidebar = richSidebar();
		sidebar.toggleSection("Context");
		expect(sidebar.isCollapsed("Context")).toBe(true);
		sidebar.setSessionKey("session-b");
		expect(sidebar.isCollapsed("Context")).toBe(false);
		sidebar.setSessionKey("default");
		expect(sidebar.isCollapsed("Context")).toBe(true);
		sidebar.resetForSession();
		expect(sidebar.isCollapsed("Context")).toBe(false);
	});
});

describe("composer tiny viewports and overlay splice", () => {
	function startComposer(columns: number, rows: number): { composer: Composer; frame: () => string[] } {
		const terminal = new VirtualTerminal(columns, rows);
		const composer = new Composer({
			preferences: { fullscreen: true, quiet: true, sidebar: "show" },
			terminal,
		});
		composer.setWorkspaceSidebar(richSidebar());
		composer.setSidebarOverlayOpen(true);
		composer.start();
		const frame = () => [...composer.renderFrame({ columns, rows }).viewport];
		return { composer, frame };
	}

	it("renders 20x4 and 24x4 without negative widths or overflow", () => {
		for (const [columns, rows] of [
			[20, 4],
			[24, 4],
		] as const) {
			const { composer, frame } = startComposer(columns, rows);
			try {
				const viewport = frame();
				expect(viewport.length).toBeLessThanOrEqual(rows);
				for (const row of viewport) expect(paintedWidth(row)).toBeLessThanOrEqual(columns);
			} finally {
				composer.stop();
			}
		}
	});

	it("splices the overlay without byte-slicing styled rows", () => {
		const { composer, frame } = startComposer(80, 24);
		try {
			const viewport = frame();
			expect(viewport.length).toBeLessThanOrEqual(24);
			for (const row of viewport) expect(paintedWidth(row)).toBeLessThanOrEqual(80);
			expect(Bun.stripANSI(viewport.join("\n"))).toContain("Context");
		} finally {
			composer.stop();
		}
	});
});
