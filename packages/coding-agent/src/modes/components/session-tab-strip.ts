import * as path from "node:path";
import { type Component, TabBar } from "@harvest/pi-tui";
import { normalizePathForComparison } from "@harvest/pi-utils";
import type { SessionTabs } from "../../session/session-tabs";
import { sanitizeStatusText, getTabBarTheme } from "../shared";

/** Visible tab strip backed by session files; switching still uses AgentSession's transaction. */
export class SessionTabStrip implements Component {
	readonly #bar = new TabBar("Sessions", [], getTabBarTheme());

	constructor(
		private readonly tabs: SessionTabs,
		private readonly currentPath: () => string | undefined,
		private readonly onSelect: (path: string) => Promise<void>,
	) {
		this.#bar.showHint = false;
		this.#bar.onTabChange = tab => void this.onSelect(tab.id);
	}

	render(width: number): readonly string[] {
		const paths = this.tabs.paths;
		if (paths.length === 0 || width < 12) return [];
		const current = this.currentPath();
		this.#bar.setTabs(
			paths.map((sessionPath, index) => ({
				id: sessionPath,
				label: `${index + 1} ${sanitizeStatusText(this.tabs.label(sessionPath) ?? path.basename(sessionPath, ".jsonl"))}`,
				short: `${index + 1}`,
			})),
			current
				? paths.find(item => normalizePathForComparison(item) === normalizePathForComparison(current))
				: undefined,
		);
		return this.#bar.render(width);
	}
}
