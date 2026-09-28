import { type Component, type Tab, TabBar, truncateToWidth } from "@harvest/pi-tui";
import { normalizePathForComparison } from "@harvest/pi-utils";
import type { SessionTabs } from "../../session/session-tabs";
import { sanitizeStatusText, getTabBarTheme } from "../shared";

/** Visible tab strip backed by session files; switching still uses AgentSession's transaction. */
export class SessionTabStrip implements Component {
	readonly #bar = new TabBar("Sessions", [], getTabBarTheme());

	constructor(
		private readonly tabs: SessionTabs,
		private readonly currentPath: () => string | undefined,
		private readonly currentTitle: () => string | undefined,
		private readonly onSelect: (path: string) => Promise<void>,
	) {
		this.#bar.showHint = false;
		this.#bar.onTabChange = tab => void this.onSelect(tab.id);
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
			label: `${start + index + 1} ${truncateToWidth(
				sanitizeStatusText(
					current && normalizePathForComparison(sessionPath) === normalizePathForComparison(current)
						? (this.currentTitle() ?? this.tabs.label(sessionPath) ?? "New session")
						: (this.tabs.label(sessionPath) ?? "New session"),
				),
				20,
			)}`,
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
