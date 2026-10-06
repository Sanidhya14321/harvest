import { stripVTControlCharacters } from "node:util";
import { type Component, type Tab, TabBar, truncateToWidth, visibleWidth } from "@harvest/pi-tui";
import { normalizePathForComparison } from "@harvest/pi-utils";
import { resolveNavigationTarget, type NavigationTarget, type SessionTabs } from "../../session/session-tabs";
import type { LiveSessionSnapshot } from "../../session/live-session-registry";
import { sanitizeStatusText, getTabBarTheme } from "../shared";
import { theme } from "../theme/theme";
import { bottomBorder, row, topBorder } from "./overlay-box";

/** Grapheme segmenter for terminal-cell budgeting (wide/emoji/combining-safe). */
const closeCellSegmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });

/**
 * Terminal-cell columns for an ANSI-stripped row. Each entry is the grapheme
 * covering that cell (wide glyphs repeat across both cells); zero-width
 * graphemes occupy no columns. Widths come from `Bun.stringWidth` so combining
 * marks (0 cells), wide CJK (2 cells), and emoji (2 cells) budget exactly as
 * the terminal renders them. Titles are already sanitized via
 * `sanitizeStatusText` before width math, so ANSI/styles never enter here.
 */
function plainCells(plain: string): { cells: string[]; total: number } {
	const cells: string[] = [];
	for (const { segment } of closeCellSegmenter.segment(plain)) {
		const width = Math.max(0, Bun.stringWidth(segment));
		for (let i = 0; i < width; i++) cells.push(segment);
	}
	return { cells, total: cells.length };
}

/** ASCII close affordance appended to every session tab label (active and inactive). */
export const TAB_CLOSE_GLYPH = "x";

/** Optional warm-first resolution inputs for the strip's mouse path. */
export interface SessionTabStripOptions {
	/** Synchronous disk probe; when omitted, non-warm tabs fall through to the legacy select path. */
	fileExists?: (path: string) => boolean;
}

/** Visible tab strip backed by session files and optional live runtime state. */
export class SessionTabStrip implements Component {
	readonly #bar = new TabBar("Sessions", [], getTabBarTheme());
	readonly #workspaceBar = new TabBar("", [], getTabBarTheme());
	#onNew: (() => void) | undefined;
	#onClose: ((path: string) => void) | undefined;
	#onSelectTarget: ((target: NavigationTarget) => Promise<void>) | undefined;
	readonly #fileExists: ((path: string) => boolean) | undefined;
	#workspaceVisibleRows = 0;
	#hoveredWorkspaceTab: string | null = null;
	#knownWorkspaceTabIds = new Set<string>();
	#flashWorkspaceTabId: string | null = null;
	/** Normalized current path seen on the last render; any change clears the flash. */
	#lastSelectedKey: string | undefined;
	/** `row:col` → tab path for rendered close (`x`) cells, rebuilt every render. */
	#closeCells = new Map<string, string>();

	constructor(
		private readonly tabs: SessionTabs,
		private readonly currentPath: () => string | undefined,
		private readonly currentTitle: () => string | undefined,
		private readonly onSelect: (path: string) => Promise<void>,
		private readonly liveSnapshot?: (path: string) => LiveSessionSnapshot | undefined,
		options?: SessionTabStripOptions,
	) {
		this.#fileExists = options?.fileExists;
		this.#bar.showHint = false;
		this.#bar.onTabChange = tab => {
			this.#flashWorkspaceTabId = null;
			void this.onSelect(tab.id);
		};
		this.#workspaceBar.showHint = false;
		this.#workspaceBar.onTabChange = tab => {
			this.#flashWorkspaceTabId = null;
			if (tab.id === "new-session") this.#onNew?.();
			else void this.onSelect(tab.id);
		};
	}

	setOnNew(callback: () => void): void {
		this.#onNew = callback;
	}

	/**
	 * Close affordance handler: invoked with the tab path when its `x` cell is
	 * clicked. Close hides the view and preserves the runtime — it never
	 * activates the tab first and never aborts, archives, or deletes it.
	 */
	setOnClose(callback: (path: string) => void): void {
		this.#onClose = callback;
	}

	/**
	 * Warm-first select handler for mouse clicks. Receives the
	 * {@link resolveNavigationTarget} outcome (stable session ID when warm);
	 * when unset, clicks fall back to the legacy {@link onSelect} path.
	 */
	setOnSelectTarget(callback: (target: NavigationTarget) => Promise<void>): void {
		this.#onSelectTarget = callback;
	}

	/** Tab path whose close (`x`) cell sits at `row:col`, if any. */
	closeTargetAt(row: number, col: number): string | undefined {
		return this.#closeCells.get(`${row}:${col}`);
	}

	#label(path: string, current: string | undefined, limit: number): string {
		const snapshot = this.liveSnapshot?.(path);
		const active = current && normalizePathForComparison(path) === normalizePathForComparison(current);
		const title = sanitizeStatusText(
			snapshot?.title ?? (active ? this.currentTitle() : undefined) ?? this.tabs.label(path) ?? "New session",
		);
		const indicator =
			snapshot?.status === "running"
				? "● "
				: snapshot?.status === "waiting"
					? "? "
					: snapshot?.status === "error"
						? "! "
						: snapshot?.unread
							? "✓ "
							: "";
		return `${indicator}${truncateToWidth(title, Math.max(1, limit - Bun.stringWidth(indicator)))}`;
	}

	/**
	 * Tab label with the ASCII close column. The trailing `x` is part of the
	 * label so active and inactive tabs alike expose an independent close hit
	 * target (resolved by {@link closeTargetAt}, never by the tab hit zone).
	 */
	#closableLabel(path: string, current: string | undefined, limit: number): string {
		const closeSuffix = ` ${TAB_CLOSE_GLYPH}`;
		return `${this.#label(path, current, Math.max(1, limit - Bun.stringWidth(closeSuffix)))}${closeSuffix}`;
	}

	/** Effective hover highlight: a real mouse hover always wins over the open/close flash. */
	#syncHoverHighlight(): void {
		this.#workspaceBar.setHoverTab(this.#hoveredWorkspaceTab ?? this.#flashWorkspaceTabId);
	}

	/**
	 * Rebuild the close-cell map from the just-rendered rows in terminal-cell
	 * geometry. Zones are probed through `tabAt` (exact, including
	 * wrap/truncation/collapse) per visible column — never per UTF-16 unit —
	 * using `Bun.stringWidth` grapheme budgeting, so wide CJK (2 cells), emoji
	 * (2 cells), and combining marks (0 cells, part of their base grapheme)
	 * land on the same columns the terminal shows. ANSI styles are stripped
	 * before width math (titles already passed through `sanitizeStatusText`).
	 *
	 * The `x` cell is accepted only when all three hold: the tab's hit-zone
	 * width equals its expected ` closable ` chunk width, and the zone ends in
	 * `␣x␣` (suffix space, close glyph, wrapper space). A truncated, collapsed,
	 * or otherwise mismeasured title fails the width or suffix check and
	 * exposes NO close target — safe (hidden ×) instead of mis-hitting a
	 * neighbor. Clipped narrow widths therefore hide × by construction.
	 */
	#rebuildCloseCells(rows: readonly string[], closeableIds: ReadonlySet<string>, current: string | undefined): void {
		this.#closeCells.clear();
		const expectedWidths = new Map<string, number>();
		for (const id of closeableIds) {
			expectedWidths.set(id, visibleWidth(` ${this.#closableLabel(id, current, 22)} `));
		}
		rows.forEach((line, lineIndex) => {
			const plain = stripVTControlCharacters(line);
			const { cells, total } = plainCells(plain);
			let col = 0;
			while (col < total) {
				const tab = this.#workspaceBar.tabAt(lineIndex, col);
				if (!tab || !closeableIds.has(tab.id)) {
					col++;
					continue;
				}
				let end = col + 1;
				while (end < total && this.#workspaceBar.tabAt(lineIndex, end)?.id === tab.id) end++;
				const expected = expectedWidths.get(tab.id);
				if (expected !== undefined && end - col === expected && end - col >= 4) {
					const beforeX = cells[end - 3];
					const xCell = cells[end - 2];
					const afterX = cells[end - 1];
					if (beforeX === " " && xCell === TAB_CLOSE_GLYPH && afterX === " ") {
						this.#closeCells.set(`${lineIndex}:${end - 2}`, tab.id);
					}
				}
				col = end;
			}
		});
	}

	renderWorkspace(width: number, hasConversation: boolean, maxRows = Number.POSITIVE_INFINITY): readonly string[] {
		this.#workspaceVisibleRows = 0;
		const paths = this.tabs.paths;
		// Any selection change — mouse, keyboard, or programmatic — clears the
		// open/close flash. The strip reads the live current path every render,
		// so keyboard navigation (which never fires onTabChange) is covered too.
		const current = this.currentPath();
		const currentKey = current ? normalizePathForComparison(current) : "";
		if (this.#lastSelectedKey !== undefined && currentKey !== this.#lastSelectedKey) {
			this.#flashWorkspaceTabId = null;
		}
		this.#lastSelectedKey = currentKey;
		if (width < 12 || (paths.length < 2 && !hasConversation)) {
			this.#workspaceBar.setTabs([]);
			this.#closeCells.clear();
			return [];
		}
		const activeIndex = current ? this.tabs.indexOf(current) : -1;
		const start = Math.max(0, Math.min(activeIndex - 2, Math.max(0, paths.length - 5)));
		const visible = paths.slice(start, start + 5);
		const closeableIds = new Set(visible);
		const displayed: Tab[] = visible.map((sessionPath, index) => ({
			id: sessionPath,
			label: this.#closableLabel(sessionPath, current, 22),
			short: `${start + index + 1}`,
		}));
		if (start > 0) displayed.unshift({ id: paths[start - 1], label: "‹", short: "‹" });
		if (paths.length > start + visible.length)
			displayed.push({ id: paths[start + visible.length], label: "›", short: "›" });
		displayed.push({ id: "new-session", label: "+ New session", short: "+" });
		// Newly opened tabs flash with the hover highlight until the next
		// selection change, so opening/closing reads as a visible transition
		// instead of a silent strip swap.
		if (this.#knownWorkspaceTabIds.size > 0) {
			const opened = paths.filter(sessionPath => !this.#knownWorkspaceTabIds.has(sessionPath));
			if (opened.length > 0) this.#flashWorkspaceTabId = opened[opened.length - 1] ?? null;
		}
		this.#knownWorkspaceTabIds = new Set(paths);
		if (this.#flashWorkspaceTabId !== null && !this.#knownWorkspaceTabIds.has(this.#flashWorkspaceTabId)) {
			this.#flashWorkspaceTabId = null;
		}
		// Mouse hover is independent of the flash: the flash only fills the
		// highlight slot while the pointer is elsewhere, and never rewrites the
		// stored hover target.
		this.#syncHoverHighlight();
		this.#workspaceBar.setTabs(
			displayed,
			current
				? visible.find(item => normalizePathForComparison(item) === normalizePathForComparison(current))
				: undefined,
		);
		const rows = this.#workspaceBar.render(width).slice(0, Math.max(0, maxRows));
		this.#rebuildCloseCells(rows, closeableIds, current);
		this.#workspaceVisibleRows = rows.length;
		return rows;
	}

	hoverWorkspace(row: number, col: number): boolean {
		const tab = row >= 0 && row < this.#workspaceVisibleRows ? this.#workspaceBar.tabAt(row, col) : undefined;
		const id = tab && !tab.muted ? tab.id : null;
		if (id === this.#hoveredWorkspaceTab) return false;
		this.#hoveredWorkspaceTab = id;
		this.#syncHoverHighlight();
		return true;
	}

	clickWorkspace(row: number, col: number): boolean {
		if (row < 0 || row >= this.#workspaceVisibleRows) return false;
		// The close target is independent of the tab target: hitting `x` hides
		// the tab (runtime preserved) without activating it first.
		const closeTarget = this.closeTargetAt(row, col);
		if (closeTarget) {
			this.#flashWorkspaceTabId = null;
			this.#onClose?.(closeTarget);
			return true;
		}
		const tab = this.#workspaceBar.tabAt(row, col);
		if (!tab || tab.muted) return false;
		if (tab.id === "new-session") this.#onNew?.();
		else if (normalizePathForComparison(tab.id) !== normalizePathForComparison(this.currentPath() ?? "")) {
			this.#selectTabWarmFirst(tab.id);
		}
		return true;
	}

	/**
	 * Mouse selection resolved warm-runtime-first through the shared
	 * {@link resolveNavigationTarget} helper (stable session ID / registry
	 * snapshot before disk checks) — the same helper the keyboard path uses.
	 */
	#selectTabWarmFirst(path: string): void {
		const selectTarget = this.#onSelectTarget;
		if (!selectTarget) {
			void this.onSelect(path);
			return;
		}
		const snapshot = this.liveSnapshot?.(path);
		const warm = snapshot ? [{ id: snapshot.id, path: snapshot.path }] : [];
		const target = resolveNavigationTarget(
			{ id: this.tabs.idForPath(path) ?? snapshot?.id, path },
			warm,
			this.#fileExists ?? (() => true),
		);
		if (!target) {
			// Neither warm nor on disk: fall through to the legacy path so the
			// owner removes the stale tab exactly as before.
			void this.onSelect(path);
			return;
		}
		void selectTarget(target);
	}

	/**
	 * Home empty state for the last-tab close: no tabs are open and no session
	 * is active. The owner must gate Home input so it never executes against a
	 * hidden session.
	 */
	renderHome(width: number): readonly string[] {
		const w = Math.max(20, Math.min(60, Math.max(1, width)));
		return [
			topBorder(w, "Home"),
			row(theme.fg("muted", "No open sessions — background runtimes keep running."), w),
			row(theme.fg("dim", "Reopen: Ctrl+Shift+T · New: + · Resume: /tab open <id>"), w),
			bottomBorder(w),
		];
	}

	render(width: number): readonly string[] {
		const paths = this.tabs.paths;
		if (paths.length < 2 || width < 12) return [];
		const current = this.currentPath();
		const activeIndex = current
			? paths.findIndex(item => normalizePathForComparison(item) === normalizePathForComparison(current))
			: -1;
		const start = Math.max(0, Math.min(activeIndex - 3, Math.max(0, paths.length - 7)));
		const visible = paths.slice(start, start + 7);
		const displayed: Tab[] = visible.map((sessionPath, index) => ({
			id: sessionPath,
			label: `${start + index + 1} ${this.#closableLabel(sessionPath, current, 20)}`,
			short: `${start + index + 1}`,
		}));
		if (start > 0) displayed.unshift({ id: "hidden-before", label: `‹ ${start} more`, short: "‹", muted: true });
		const remaining = paths.length - start - visible.length;
		if (remaining > 0) displayed.push({ id: "hidden-after", label: `${remaining} more ›`, short: "›", muted: true });
		this.#bar.setTabs(
			displayed,
			current
				? visible.find(item => normalizePathForComparison(item) === normalizePathForComparison(current))
				: undefined,
		);
		return this.#bar.render(width);
	}
}

/** State the tab strip needs to describe the currently selected session. */
export interface SelectedSessionDescription {
	sessionId: string;
	sessionFile: string | undefined;
	sessionName: string | undefined;
	/** Tab path being described; only the selected session's path resolves. */
	path: string;
	isStreaming: boolean;
	approvalOpen: boolean;
}

/**
 * Synthesize the selected session's live snapshot for the tab strip: running
 * while a turn streams, waiting while an approval dialog holds the editor,
 * idle otherwise. Other paths resolve to undefined until the live-session
 * registry feeds background runtimes. The selected tab never carries unread.
 */
export function describeSelectedSession(description: SelectedSessionDescription): LiveSessionSnapshot | undefined {
	const file = description.sessionFile;
	if (!file || normalizePathForComparison(file) !== normalizePathForComparison(description.path)) return undefined;
	return {
		id: description.sessionId,
		path: file,
		title: description.sessionName,
		status: description.isStreaming ? "running" : description.approvalOpen ? "waiting" : "idle",
		unread: false,
		selected: true,
	};
}

/**
 * Prefer a registry snapshot for background tabs, but never let it describe
 * the visible tab: selection always comes from the live current path, and an
 * approval holding the editor forces the waiting state even when the
 * registry has not been told (its wait markers are set by callers that do
 * not exist yet on every path).
 */
export function mergeRegistrySnapshot(
	live: LiveSessionSnapshot | undefined,
	selected: LiveSessionSnapshot | undefined,
	approvalOpen: boolean,
): LiveSessionSnapshot | undefined {
	if (!live) return selected;
	if (!selected) return live;
	const merged: LiveSessionSnapshot = {
		...live,
		selected: selected.selected,
		title: selected.title ?? live.title,
	};
	if (selected.selected && approvalOpen) merged.status = "waiting";
	return merged;
}
