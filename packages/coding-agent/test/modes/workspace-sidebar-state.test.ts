import { beforeEach, describe, expect, it } from "bun:test";
import { Composer } from "@harvest/pi-coding-agent/modes/composer";
import { emptySidebarSnapshot, WorkspaceSidebar } from "@harvest/pi-coding-agent/modes/components/workspace-sidebar";
import { TranscriptContainer } from "@harvest/pi-coding-agent/modes/components/transcript-container";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { computeWorkspaceLayout, sidebarOverlayVisible } from "@harvest/pi-coding-agent/modes/workspace-layout";
import { Text, visibleWidth, CURSOR_MARKER } from "@harvest/pi-tui";
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

	it("opens the temporary overlay from auto/hide on explicit toggle, never by itself", () => {
		// Explicit toggle/focus paints the narrow overlay in any mode; without
		// the open flag nothing paints, and wide viewports dock instead.
		expect(sidebarOverlayVisible(80, "auto", true)).toBe(true);
		expect(sidebarOverlayVisible(80, "hide", true)).toBe(true);
		expect(sidebarOverlayVisible(160, "show", true)).toBe(false);
		const auto = computeWorkspaceLayout(
			{ columns: 80, rows: 24 },
			{ hasConversation: true, sidebarPreference: "auto", sidebarOverlayOpen: true },
		);
		expect(auto.sidebarOverlayRequested).toBe(true);
		expect(auto.hitTest(79)).toBe("sidebar");
		const autoClosed = computeWorkspaceLayout(
			{ columns: 80, rows: 24 },
			{ hasConversation: true, sidebarPreference: "auto", sidebarOverlayOpen: false },
		);
		expect(autoClosed.sidebarOverlayRequested).toBe(false);
		expect(autoClosed.hitTest(79)).toBe("main");
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

	it("keeps the home draft editable at 24x4 with decorations and warnings", () => {
		for (const [columns, rows] of [
			[20, 4],
			[24, 4],
		] as const) {
			const terminal = new VirtualTerminal(columns, rows);
			const composer = new Composer({
				preferences: { fullscreen: true, quiet: false, sidebar: "hide" },
				terminal,
			});
			try {
				composer.editor.setText("DRAFT9");
				composer.setHeaderExtras([new Text("WARN-MARKER")], []);
				composer.start();
				const viewport = [...composer.renderFrame({ columns, rows }).viewport].map(row => Bun.stripANSI(row));
				expect(viewport.length).toBeLessThanOrEqual(rows);
				for (const row of viewport) expect(paintedWidth(row)).toBeLessThanOrEqual(columns);
				// Editable draft survives home decorations, warnings, and hints.
				// (Marker is short: at 20 columns a long draft wraps, which is
				// correct rendering — the contract is draft visibility.)
				expect(viewport.join("\n")).toContain("DRAFT9");
			} finally {
				composer.stop();
			}
		}
	});

	it("paints the pixel wordmark, key-style hints, and version stamp on a roomy home (text-home)", () => {
		const terminal = new VirtualTerminal(100, 30);
		const composer = new Composer({
			preferences: { fullscreen: true, quiet: false, sidebar: "hide" },
			terminal,
			welcome: { version: "9.9.9" },
		});
		try {
			composer.start();
			const viewport = [...composer.renderFrame({ columns: 100, rows: 30 }).viewport].map(row => Bun.stripANSI(row));
			const art = viewport.filter(row => row.includes("█"));
			// Five pixel rows forming one 41-column centered block.
			expect(art).toHaveLength(5);
			const firsts = art.map(row => row.indexOf("█"));
			expect(new Set(firsts).size).toBe(1);
			expect(firsts[0]).toBe(Math.floor((100 - 41) / 2));
			expect(viewport.join("\n")).toContain("send");
			expect(viewport.join("\n")).toContain("v9.9.9");
		} finally {
			composer.stop();
		}
	});

	it("falls back to the text wordmark on narrow viewports (art-crush)", () => {
		const terminal = new VirtualTerminal(40, 12);
		const composer = new Composer({
			preferences: { fullscreen: true, quiet: false, sidebar: "hide" },
			terminal,
		});
		try {
			composer.start();
			const viewport = [...composer.renderFrame({ columns: 40, rows: 12 }).viewport].map(row => Bun.stripANSI(row));
			expect(viewport.join("\n")).not.toContain("█");
			expect(viewport.join("\n")).toContain("harvest");
		} finally {
			composer.stop();
		}
	});

	it("shows the hint row below the session composer on roomy screens (hintless-session)", () => {
		const terminal = new VirtualTerminal(100, 30);
		const composer = new Composer({ preferences: { fullscreen: true, quiet: false }, terminal });
		const transcript = new TranscriptContainer();
		transcript.addChild(new Text("hello line"));
		composer.setRuntimeChildren([transcript, composer.editor]);
		try {
			composer.start();
			const viewport = [...composer.renderFrame({ columns: 100, rows: 30 }).viewport].map(row => Bun.stripANSI(row));
			expect(viewport.join("\n")).toContain("hello line");
			expect(viewport.join("\n")).toContain("send");
			expect(viewport.join("\n")).toContain("commands");
		} finally {
			composer.stop();
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
