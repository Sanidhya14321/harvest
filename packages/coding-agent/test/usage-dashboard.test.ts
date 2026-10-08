import { afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import type { DailyActivityPoint } from "@harvest/omp-stats/shared-types";
import type { UsageReport } from "@harvest/pi-ai";
import {
	buildHeatmapLayout,
	buildProviderCards,
	UsageDashboardComponent,
	type UsageDashboardOptions,
} from "@harvest/pi-coding-agent/modes/components/usage-dashboard";
import { loadThemeSync } from "@harvest/pi-coding-agent/modes/theme/loader";
import { setThemeInstance, theme, type Theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { visibleWidth } from "@harvest/pi-tui";

function day(day: string, cost: number, requests = 1): DailyActivityPoint {
	return { day, cost, requests, totalTokens: 0 };
}

function report(provider: string, email: string, limits: UsageReport["limits"]): UsageReport {
	return { provider, fetchedAt: Date.now(), limits, metadata: { email } };
}

function limit(
	provider: string,
	accountId: string,
	windowId: string,
	label: string,
	usedFraction: number,
	status: "ok" | "warning" | "exhausted",
	resetsAt?: number,
): UsageReport["limits"][number] {
	return {
		id: `${provider}:${accountId}:${windowId}`,
		label,
		scope: { provider, accountId, windowId },
		window: { id: windowId, label: windowId, resetsAt },
		amount: { usedFraction, unit: "percent" },
		status,
	};
}

describe("buildHeatmapLayout", () => {
	// 2026-08-31 is a Monday; keeps week alignment deterministic.
	const monday = new Date(2026, 7, 31, 12);

	it("aligns days Monday-first and marks future days null", () => {
		const layout = buildHeatmapLayout([day("2026-08-31", 5)], 2, monday);
		// Monday row, last column = today's week.
		expect(layout.cells[0][1]).toBe(4);
		// Tuesday..Sunday of the current week are in the future.
		for (let row = 1; row < 7; row++) expect(layout.cells[row][1]).toBeNull();
		// Previous week is fully in range but has no activity.
		for (let row = 0; row < 7; row++) expect(layout.cells[row][0]).toBe(0);
	});

	it("scales intensity by magnitude against the busiest day, not by rank", () => {
		const points = [
			day("2026-08-24", 100), // max → level 4
			day("2026-08-25", 30), // sqrt(0.3)≈0.55 → level 3
			day("2026-08-26", 6), // sqrt(0.06)≈0.24 → level 1
			day("2026-08-27", 0), // untouched → level 0
		];
		const layout = buildHeatmapLayout(points, 2, monday);
		expect(layout.cells[0][0]).toBe(4);
		expect(layout.cells[1][0]).toBe(3);
		expect(layout.cells[2][0]).toBe(1);
		expect(layout.cells[3][0]).toBe(0);
	});

	it("falls back to request counts when nothing in range is priced", () => {
		const layout = buildHeatmapLayout([day("2026-08-24", 0, 50), day("2026-08-25", 0, 3)], 2, monday);
		expect(layout.cells[0][0]).toBe(4);
		expect(layout.cells[1][0]).toBe(1);
		expect(layout.totalRequests).toBe(53);
	});

	it("labels a column when its week starts a new month", () => {
		// 6 weeks back from 2026-08-31 spans the July→August boundary.
		const layout = buildHeatmapLayout([], 6, monday);
		expect(layout.monthLabels[0]).toBe("Jul");
		expect(layout.monthLabels.filter(Boolean)).toEqual(["Jul", "Aug"]);
	});
});

describe("buildProviderCards", () => {
	const now = Date.now();

	it("averages a window across accounts instead of showing the worst account", () => {
		// One exhausted + one barely-used account: the classic report shows the
		// aggregate (~50% free), so the card must not read 0% free.
		const reports = [
			report("anthropic", "a@x.test", [limit("anthropic", "a", "7d", "Claude 7 Day", 1.0, "exhausted", now + 1000)]),
			report("anthropic", "b@x.test", [limit("anthropic", "b", "7d", "Claude 7 Day", 0.0, "ok", now + 99_000)]),
		];
		const cards = buildProviderCards(reports, now);
		expect(cards).toHaveLength(1);
		expect(cards[0].windows).toHaveLength(1);
		expect(cards[0].windows[0].fraction).toBeCloseTo(0.5);
		// Mixed healthy/exhausted accounts read as warning, not exhausted.
		expect(cards[0].windows[0].status).toBe("warning");
		// Reset countdown comes from the most-used account (when capacity returns).
		expect(cards[0].windows[0].resetMs).toBe(1000);
	});

	it("sorts pressured providers first and collapses untouched ones into idle", () => {
		const reports = [
			report("cursor", "c@x.test", [limit("cursor", "c", "monthly", "Cursor Models", 0.0, "ok")]),
			report("openai-codex", "o@x.test", [limit("openai-codex", "o", "7d", "7 days", 0.4, "ok")]),
			report("ollama-cloud", "l@x.test", []),
		];
		const cards = buildProviderCards(reports, now);
		expect(cards[0].provider).toBe("openai-codex");
		expect(cards[0].idle).toBe(false);
		const idle = cards.filter(card => card.idle).map(card => card.provider);
		expect(idle.sort()).toEqual(["cursor", "ollama-cloud"]);
		const unlimited = cards.find(card => card.provider === "ollama-cloud");
		expect(unlimited?.unlimited).toBe(true);
	});
});

describe("UsageDashboardComponent", () => {
	let previousTheme: Theme | undefined;
	const dashboards: UsageDashboardComponent[] = [];

	beforeEach(() => {
		previousTheme = theme;
		setThemeInstance(loadThemeSync("harvest", { mode: "none", symbolPresetOverride: "ascii" }));
	});

	afterEach(() => {
		for (const dashboard of dashboards.splice(0)) dashboard.dispose();
		if (previousTheme) setThemeInstance(previousTheme);
		vi.restoreAllMocks();
	});

	function dashboard(overrides: Partial<UsageDashboardOptions> = {}): UsageDashboardComponent {
		const component = new UsageDashboardComponent({
			reports: [],
			renderDetail: () => "first-line\nsecond-line\nthird-line\nlast-line",
			loadActivity: async push => push([]),
			requestRender: () => {},
			onClose: () => {},
			...overrides,
		});
		dashboards.push(component);
		return component;
	}

	it("keeps data visible within one to four allocated rows and tiny widths after resize", () => {
		const component = dashboard();
		component.handleInput("\r");
		for (const height of [1, 2, 3, 4]) {
			component.setMaxHeight(height);
			for (const width of [1, 2, 3, 20, 24]) {
				component.handleInput("\x1b[H");
				const lines = component.render(width);
				expect(lines).toHaveLength(height);
				expect(lines.map(visibleWidth)).toEqual(Array(height).fill(width));
				const contentWidth = width < 4 ? (width === 2 ? 2 : 1) : width - 4;
				expect(lines.map(line => Bun.stripANSI(line).trim())).toContain(
					contentWidth < 3 ? "first-line".slice(0, contentWidth) : "first-line",
				);
			}
		}
	});

	it("keeps narrow quota values readable and distinguishes activity intensity without color or Unicode glyphs", () => {
		const today = new Date();
		const points = [100, 8, 1].map((requests, offset) => {
			const date = new Date(today.getFullYear(), today.getMonth(), today.getDate() - offset);
			return day(
				`${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`,
				0,
				requests,
			);
		});
		const reports = [
			report("openai-codex", "fake@example.invalid", [limit("openai-codex", "fake", "7d", "Weekly", 0.4, "ok")]),
		];
		for (const options of [
			{ mode: "truecolor", symbolPresetOverride: "ascii" },
			{ mode: "none", symbolPresetOverride: "unicode" },
		] as const) {
			setThemeInstance(loadThemeSync("harvest", options));
			const component = dashboard({ reports, loadActivity: async push => push(points) });
			component.setMaxHeight(40);
			const wide = component.render(80).map(line => Bun.stripANSI(line));
			const cells = wide.filter(line => /^\s*[MTWFS] [ .1-4]+$/.test(line)).join("\n");
			expect(cells).toContain("1");
			expect(cells).toContain("2");
			expect(cells).toContain("4");
			expect(cells).toContain(".");
			if (options.symbolPresetOverride === "ascii") {
				expect(wide.join("\n")).not.toMatch(/[^\x00-\x7f]/);
				expect(wide.join("\n")).toMatch(/={2,}-{2,}/);
			}
			const narrow = component.render(24).map(line => Bun.stripANSI(line));
			expect(narrow.join("\n")).toContain("60% free Weekly");
			expect(narrow.map(visibleWidth)).toEqual(Array(narrow.length).fill(24));
		}
	});

	it("scrolls details with the mouse and restores overview before closing and aborting the loader", () => {
		const onClose = vi.fn();
		let loadSignal: AbortSignal | undefined;
		const component = dashboard({
			onClose,
			loadActivity: async (_push, signal) => {
				loadSignal = signal;
			},
		});
		component.setMaxHeight(4);
		component.handleInput("\r");
		component.render(24);
		component.handleInput("\x1b[<65;1;1M");
		expect(Bun.stripANSI(component.render(24).join("\n"))).toContain("third-line");
		component.handleInput("\x1b");
		expect(onClose).not.toHaveBeenCalled();
		expect(Bun.stripANSI(component.render(24).join("\n"))).toContain("No usage data");
		component.handleInput("\x1b");
		expect(onClose).toHaveBeenCalledTimes(1);
		expect(loadSignal?.aborted).toBe(true);
	});

	it("refreshes detail formatting for the assigned width and invalidation while preserving user Unicode", () => {
		const renderDetail = vi.fn((width: number) => `mañana ${width}`);
		const component = dashboard({ renderDetail });
		component.setMaxHeight(4);
		component.handleInput("\r");
		expect(Bun.stripANSI(component.render(24).join("\n"))).toContain("mañana 20");
		component.render(24);
		expect(renderDetail).toHaveBeenCalledTimes(1);
		expect(Bun.stripANSI(component.render(32).join("\n"))).toContain("mañana 28");
		component.invalidate();
		component.render(32);
		expect(renderDetail).toHaveBeenCalledTimes(3);
	});

	it("surfaces a local history load failure and suppresses late redraws after disposal", async () => {
		const failed = dashboard({
			loadActivity: async () => {
				throw new Error("fake local database failure");
			},
		});
		failed.setMaxHeight(10);
		await Promise.resolve();
		expect(Bun.stripANSI(failed.render(80).join("\n"))).toContain("stats database could not be read");

		const requestRender = vi.fn();
		const pending = Promise.withResolvers<void>();
		let pushActivity: ((points: DailyActivityPoint[]) => void) | undefined;
		let loadSignal: AbortSignal | undefined;
		const closed = dashboard({
			requestRender,
			loadActivity: (push, signal) => {
				pushActivity = push;
				loadSignal = signal;
				return pending.promise;
			},
		});
		closed.dispose();
		pushActivity?.([day("2026-10-08", 1)]);
		pending.resolve();
		await Promise.resolve();
		expect(loadSignal?.aborted).toBe(true);
		expect(requestRender).not.toHaveBeenCalled();
	});
});
