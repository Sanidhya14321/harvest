import { type Component, type Tab, TabBar, truncateToWidth } from "@harvest/pi-tui";
import { normalizePathForComparison } from "@harvest/pi-utils";
import type { SessionTabs } from "../../session/session-tabs";
import type { LiveSessionSnapshot } from "../../session/live-session-registry";
import { sanitizeStatusText, getTabBarTheme } from "../shared";

/** Visible tab strip backed by session files and optional live runtime state. */
export class SessionTabStrip implements Component {
	readonly #bar = new TabBar("Sessions", [], getTabBarTheme());
	readonly #workspaceBar = new TabBar("", [], getTabBarTheme());
	#onNew: (() => void) | undefined;

	constructor(
		private readonly tabs: SessionTabs,
		private readonly currentPath: () => string | undefined,
		private readonly currentTitle: () => string | undefined,
		private readonly onSelect: (path: string) => Promise<void>,
		private readonly liveSnapshot?: (path: string) => LiveSessionSnapshot | undefined,
	) {
		this.#bar.showHint = false;
		this.#bar.onTabChange = tab => void this.onSelect(tab.id);
		this.#workspaceBar.showHint = false;
		this.#workspaceBar.onTabChange = tab => {
			if (tab.id === "new-session") this.#onNew?.();
			else void this.onSelect(tab.id);
		};
	}

	setOnNew(callback: () => void): void {
		this.#onNew = callback;
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

	renderWorkspace(width: number, hasConversation: boolean): readonly string[] {
		const paths = this.tabs.paths;
		if (width < 12 || (paths.length < 2 && !hasConversation)) {
			this.#workspaceBar.setTabs([]);
			return [];
		}
		const current = this.currentPath();
		const activeIndex = current ? this.tabs.indexOf(current) : -1;
		const start = Math.max(0, Math.min(activeIndex - 2, Math.max(0, paths.length - 5)));
		const visible = paths.slice(start, start + 5);
		const displayed: Tab[] = visible.map((sessionPath, index) => ({
			id: sessionPath,
			label: this.#label(sessionPath, current, 22),
			short: `${start + index + 1}`,
		}));
		if (start > 0) displayed.unshift({ id: "hidden-before", label: "‹", muted: true });
		if (paths.length > start + visible.length) displayed.push({ id: "hidden-after", label: "›", muted: true });
		displayed.push({ id: "new-session", label: "+ New session", short: "+" });
		this.#workspaceBar.setTabs(
			displayed,
			current
				? visible.find(item => normalizePathForComparison(item) === normalizePathForComparison(current))
				: undefined,
		);
		return this.#workspaceBar.render(width);
	}

	clickWorkspace(row: number, col: number): boolean {
		const tab = this.#workspaceBar.tabAt(row, col);
		if (!tab || tab.muted) return false;
		if (tab.id === "new-session") this.#onNew?.();
		else if (normalizePathForComparison(tab.id) !== normalizePathForComparison(this.currentPath() ?? "")) {
			void this.onSelect(tab.id);
		}
		return true;
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
			label: `${start + index + 1} ${this.#label(sessionPath, current, 20)}`,
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
