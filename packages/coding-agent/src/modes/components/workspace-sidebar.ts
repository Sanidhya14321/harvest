import type { Component } from "@harvest/pi-tui";
import { applyBackgroundToLine, padding, replaceTabs, truncateToWidth } from "@harvest/pi-tui";
import { shortenPath } from "../../tools/render-utils";
import { theme } from "../theme/theme";
import { WORKSPACE_LAYOUT } from "../workspace-layout";

/** Typed, read-only snapshot consumed by the sidebar. No ANSI parsing, no I/O in render(). */
export interface WorkspaceSidebarSnapshot {
	readonly title: string;
	readonly contextKnown: boolean;
	readonly contextPercent: number | null;
	readonly contextTokens: number | null;
	readonly contextWindow: number | null;
	readonly costKnown: boolean;
	readonly costLabel: string | null;
	readonly mcpServers: readonly { name: string; connected: boolean; detail?: string }[];
	readonly lspServers: readonly { name: string; status: string }[];
	readonly todos: readonly { label: string; done: boolean }[];
	readonly agents: readonly { label: string; state: string }[];
	readonly changes: readonly {
		path: string;
		staged: boolean;
		untracked: boolean;
		added: number | null;
		removed: number | null;
	}[];
	readonly changesTruncated: boolean;
	readonly changesState: "ready" | "loading" | "non-repository" | "error";
	readonly changesError?: string;
	readonly extensions: readonly { label: string; detail?: string }[];
	readonly version: string;
	readonly sessionId?: string;
	readonly generation?: number;
}

export function emptySidebarSnapshot(): WorkspaceSidebarSnapshot {
	return {
		title: "",
		contextKnown: false,
		contextPercent: null,
		contextTokens: null,
		contextWindow: null,
		costKnown: false,
		costLabel: null,
		mcpServers: [],
		lspServers: [],
		todos: [],
		agents: [],
		changes: [],
		changesTruncated: false,
		changesState: "loading",
		extensions: [],
		version: "",
	};
}

export interface SidebarSectionState {
	collapsed: Record<string, boolean>;
	scrollOffset: number;
	focused: boolean;
	focusIndex: number;
}

function sectionKey(title: string): string {
	return title.toLowerCase().replace(/[^a-z0-9]+/g, "-");
}

/**
 * Right-hand session sidebar: title, context/cost, MCP/LSP, Todo, Agents,
 * Workspace Changes, extension statuses, version footer. Empty optional
 * sections are omitted. One scroll viewport; sections collapse.
 */
export class WorkspaceSidebar implements Component {
	#snapshot: WorkspaceSidebarSnapshot = emptySidebarSnapshot();
	#sessions = new Map<string, SidebarSectionState>();
	#sessionKey = "default";
	#onChange: (() => void) | undefined;
	#onClose: (() => void) | undefined;
	#lastViewportHeight = 0;

	#getState(): SidebarSectionState {
		let state = this.#sessions.get(this.#sessionKey);
		if (!state) {
			state = { collapsed: {}, scrollOffset: 0, focused: false, focusIndex: 0 };
			this.#sessions.set(this.#sessionKey, state);
		}
		return state;
	}

	/** Switch active per-session collapse/scroll/focus state (no global leakage). */
	setSessionKey(key: string): void {
		if (!key) return;
		if (this.#sessionKey === key) return;
		this.#sessionKey = key;
		this.#getState();
		this.#onChange?.();
	}

	/** Reset collapse/scroll/focus for the current (or given) session key. */
	resetForSession(key?: string): void {
		const target = key ?? this.#sessionKey;
		this.#sessions.set(target, { collapsed: {}, scrollOffset: 0, focused: false, focusIndex: 0 });
		this.#onChange?.();
	}

	setSnapshot(snapshot: WorkspaceSidebarSnapshot): void {
		this.#snapshot = snapshot;
	}

	setOnChange(cb: (() => void) | undefined): void {
		this.#onChange = cb;
	}

	setOnClose(cb: (() => void) | undefined): void {
		this.#onClose = cb;
	}

	get focused(): boolean {
		return this.#getState().focused;
	}

	setFocused(focused: boolean): void {
		this.#getState().focused = focused;
		this.#onChange?.();
	}

	isCollapsed(title: string): boolean {
		return this.#getState().collapsed[sectionKey(title)] === true;
	}

	toggleSection(title: string): void {
		const state = this.#getState();
		const key = sectionKey(title);
		state.collapsed[key] = !state.collapsed[key];
		this.#onChange?.();
	}

	setScrollOffset(offset: number): void {
		this.#getState().scrollOffset = Math.max(0, Math.floor(offset));
		this.#onChange?.();
	}

	get scrollOffset(): number {
		return this.#getState().scrollOffset;
	}

	get focusIndex(): number {
		return this.#getState().focusIndex;
	}

	moveFocus(delta: number, sectionCount: number): void {
		if (sectionCount <= 0) return;
		const state = this.#getState();
		const next = state.focusIndex + delta;
		state.focusIndex = Math.max(0, Math.min(sectionCount - 1, next));
		this.#ensureFocusVisible();
		this.#onChange?.();
	}

	/**
	 * Keyboard nav for the sidebar viewport. Returns true when handled.
	 * Accepted keys (case-insensitive): up/down move focus, pageup/pagedown
	 * move by page, enter/space/right toggle the focused section, left
	 * collapses it, escape fires onClose.
	 */
	handleInput(key: string): boolean {
		const normalized = key.toLowerCase();
		const sections = this.#buildSections();
		const state = this.#getState();
		const page = Math.max(1, this.#lastViewportHeight > 0 ? this.#lastViewportHeight - 1 : 5);
		switch (normalized) {
			case "up":
			case "k":
				if (sections.length <= 0) return false;
				state.focusIndex = Math.max(0, state.focusIndex - 1);
				this.#ensureFocusVisible();
				this.#onChange?.();
				return true;
			case "down":
			case "j":
				if (sections.length <= 0) return false;
				state.focusIndex = Math.min(sections.length - 1, state.focusIndex + 1);
				this.#ensureFocusVisible();
				this.#onChange?.();
				return true;
			case "pageup":
				if (sections.length <= 0) return false;
				state.focusIndex = Math.max(0, state.focusIndex - page);
				state.scrollOffset = Math.max(0, state.scrollOffset - page);
				this.#ensureFocusVisible();
				this.#onChange?.();
				return true;
			case "pagedown":
				if (sections.length <= 0) return false;
				state.focusIndex = Math.min(sections.length - 1, state.focusIndex + page);
				this.#ensureFocusVisible();
				this.#onChange?.();
				return true;
			case "enter":
			case "space":
			case "right":
			case "l": {
				const focused = sections[Math.min(state.focusIndex, Math.max(0, sections.length - 1))];
				if (!focused || focused.title === this.#snapshot.title) return false;
				this.toggleSection(focused.title);
				return true;
			}
			case "left":
			case "h": {
				const focused = sections[Math.min(state.focusIndex, Math.max(0, sections.length - 1))];
				if (!focused || focused.title === this.#snapshot.title) return false;
				const k = sectionKey(focused.title);
				if (state.collapsed[k] !== true) {
					state.collapsed[k] = true;
					this.#onChange?.();
				}
				return true;
			}
			case "escape":
			case "esc":
				this.#onClose?.();
				this.#onChange?.();
				return true;
			default:
				return false;
		}
	}

	#ensureFocusVisible(): void {
		if (this.#lastViewportHeight <= 0) return;
		const state = this.#getState();
		const maxOffset = Math.max(0, this.#rowCount() - this.#lastViewportHeight);
		state.scrollOffset = Math.max(0, Math.min(state.scrollOffset, maxOffset));
		const sections = this.#buildSections();
		let focusRow = 0;
		for (let index = 0; index < Math.min(state.focusIndex, sections.length); index++) {
			const section = sections[index]!;
			focusRow++;
			if (!this.isCollapsed(section.title)) {
				focusRow += section.rows.length + (index < sections.length - 1 ? 1 : 0);
			}
		}
		if (focusRow < state.scrollOffset) state.scrollOffset = focusRow;
		else if (focusRow >= state.scrollOffset + this.#lastViewportHeight) {
			state.scrollOffset = focusRow - this.#lastViewportHeight + 1;
		}
		state.scrollOffset = Math.min(state.scrollOffset, maxOffset);
	}

	sectionTitles(): string[] {
		return this.#buildSections().map(s => s.title);
	}

	#buildSections(): { title: string; rows: string[] }[] {
		const snap = this.#snapshot;
		const sections: { title: string; rows: string[] }[] = [];
		if (snap.title) sections.push({ title: snap.title, rows: [] });
		// Context / cost — unknown stays unknown, never 0%.
		const ctxRows: string[] = [];
		if (snap.contextKnown && snap.contextPercent !== null) {
			ctxRows.push(`Context ${snap.contextPercent.toFixed(0)}%`);
		} else {
			ctxRows.push(theme.fg("muted", "Context unknown"));
		}
		if (snap.costKnown && snap.costLabel) ctxRows.push(snap.costLabel);
		else if (!snap.costKnown) ctxRows.push(theme.fg("muted", "Cost unknown"));
		sections.push({ title: "Context", rows: ctxRows });
		if (snap.mcpServers.length > 0) {
			sections.push({
				title: "MCP",
				rows: snap.mcpServers.map(s => `${s.connected ? theme.status.enabled : theme.status.disabled} ${s.name}`),
			});
		}
		if (snap.lspServers.length > 0) {
			sections.push({ title: "LSP", rows: snap.lspServers.map(s => `${s.name}${theme.sep.dot}${s.status}`) });
		}
		if (snap.todos.length > 0) {
			sections.push({
				title: "Todo",
				rows: snap.todos
					.slice(0, 12)
					.map(t => `${t.done ? theme.status.success : theme.status.pending} ${t.label}`),
			});
		}
		if (snap.agents.length > 0) {
			sections.push({ title: "Agents", rows: snap.agents.map(a => `${a.label}${theme.sep.dot}${a.state}`) });
		}
		if (snap.changesState === "ready" && snap.changes.length > 0) {
			const rows = snap.changes.slice(0, 20).map(c => {
				const mark = c.untracked ? "?" : c.staged ? theme.status.enabled : theme.status.pending;
				return `${mark} ${truncateToWidth(shortenPath(c.path), 34)}`;
			});
			if (snap.changesTruncated) rows.push(theme.fg("muted", `+ more changes`));
			sections.push({ title: "Workspace Changes", rows });
		} else if (snap.changesState === "ready") {
			sections.push({ title: "Workspace Changes", rows: [theme.fg("muted", "clean")] });
		} else if (snap.changesState === "loading") {
			sections.push({
				title: "Workspace Changes",
				rows: [theme.fg("muted", `loading${theme.symbol("sep.ellipsis")}`)],
			});
		} else if (snap.changesState === "non-repository") {
			sections.push({ title: "Workspace Changes", rows: [theme.fg("muted", "not a repository")] });
		} else if (snap.changesState === "error") {
			sections.push({
				title: "Workspace Changes",
				rows: [theme.fg("error", truncateToWidth(snap.changesError ?? "unavailable", 36))],
			});
		}
		for (const ext of snap.extensions) {
			sections.push({ title: ext.label, rows: ext.detail ? [truncateToWidth(ext.detail, 36)] : [] });
		}
		return sections;
	}

	render(width: number, viewportHeight?: number): readonly string[] {
		const safeWidth = Math.max(1, Math.floor(width));
		const inset = Math.min(WORKSPACE_LAYOUT.sidebarInnerPaddingX, Math.floor((safeWidth - 1) / 2));
		const inner = Math.max(1, safeWidth - inset * 2);
		const pad = padding(inset);
		const rows: string[] = [];
		const sections = this.#buildSections();
		const state = this.#getState();
		const paintRow = (text: string, selected = false): string => {
			const background = selected ? "selectedBg" : "panelBg";
			return applyBackgroundToLine(truncateToWidth(pad + replaceTabs(text), safeWidth), safeWidth, line =>
				theme.bgFill(background, line),
			);
		};
		sections.forEach((section, index) => {
			const collapsed = this.isCollapsed(section.title);
			const marker =
				index === 0 ? "" : collapsed ? ` ${theme.symbol("nav.expand")}` : ` ${theme.symbol("nav.collapse")}`;
			const titleRow =
				index === 0
					? theme.bold(theme.fgOnBg("text", "panelBg", truncateToWidth(replaceTabs(section.title), inner)))
					: theme.fg("muted", truncateToWidth(replaceTabs(section.title + marker), inner));
			rows.push(paintRow(titleRow, state.focused && index === state.focusIndex));
			if (collapsed) return;
			for (const r of section.rows) rows.push(paintRow(truncateToWidth(replaceTabs(r), inner)));
			if (index < sections.length - 1) rows.push(paintRow(""));
		});
		if (this.#snapshot.version) {
			rows.push(paintRow(theme.fg("dim", truncateToWidth(`harvest ${this.#snapshot.version}`, inner))));
		}
		if (viewportHeight !== undefined) {
			const height = Math.max(0, Math.floor(viewportHeight));
			this.#lastViewportHeight = height;
			if (height === 0) return [];
			// Clamp the stored offset silently (no onChange: render stays pure).
			const maxOffset = Math.max(0, rows.length - height);
			state.scrollOffset = Math.max(0, Math.min(state.scrollOffset, maxOffset));
			state.focusIndex = Math.max(0, Math.min(state.focusIndex, Math.max(0, sections.length - 1)));
			const visible = rows.slice(state.scrollOffset, state.scrollOffset + height);
			while (visible.length < height) visible.push(paintRow(""));
			return visible;
		}
		const offset = Math.min(state.scrollOffset, Math.max(0, rows.length - 1));
		return rows.slice(offset);
	}

	#rowCount(): number {
		const sections = this.#buildSections();
		let count = 0;
		sections.forEach((section, index) => {
			count += 1;
			if (this.isCollapsed(section.title)) return;
			count += section.rows.length;
			if (index < sections.length - 1) count += 1;
		});
		if (this.#snapshot.version) count += 1;
		return count;
	}
}
