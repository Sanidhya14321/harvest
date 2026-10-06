/**
 * Short-terminal viewport budgets for fullscreen dialogs (review §4.7).
 *
 * Settings/ModelHub/AgentsHub/Usage/History assumed a tall alt-screen
 * (Math.max(14/16, rows)) and spilled past the viewport on short terminals,
 * clipping required controls into scrollback. Each test below renders at
 * 80x10 and 60x8 and asserts the frame never exceeds the viewport while the
 * search input (if any), the selected row, and the footer affordance stay
 * visible — including after moving the selection past the viewport end.
 */
import { afterAll, afterEach, beforeAll, describe, expect, test, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { stripVTControlCharacters } from "node:util";
import type { Model } from "@harvest/pi-ai";
import { buildModel } from "@harvest/pi-catalog/build";
import type { ModelRegistry } from "@harvest/pi-coding-agent/config/model-registry";
import { resetSettingsForTest, Settings } from "@harvest/pi-coding-agent/config/settings";
import { AgentsHubComponent } from "@harvest/pi-coding-agent/modes/components/agents-hub";
import { HistorySearchComponent } from "@harvest/pi-coding-agent/modes/components/history-search";
import { ModelHubComponent, resetProviderAutoRefreshGuard } from "@harvest/pi-coding-agent/modes/components/model-hub";
import { SettingsSelectorComponent } from "@harvest/pi-coding-agent/modes/components/settings-selector";
import { UsageDashboardComponent } from "@harvest/pi-coding-agent/modes/components/usage-dashboard";
import { initTheme, theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { HistoryEntry, HistoryStorage } from "@harvest/pi-coding-agent/session/history-storage";
import * as discovery from "@harvest/pi-coding-agent/task/discovery";
import type { DailyActivityPoint } from "@harvest/omp-stats/shared-types";
import type { TUI } from "@harvest/pi-tui";
import { removeWithRetries } from "@harvest/pi-utils";

const DOWN = "\x1b[B";
const RIGHT = "\x1b[C";

const VIEWPORTS = [
	{ cols: 80, rows: 10 },
	{ cols: 60, rows: 8 },
] as const;

function plain(lines: readonly string[]): string {
	return stripVTControlCharacters(lines.join("\n"));
}

function raw(lines: readonly string[]): string {
	return lines.join("\n");
}

/** Frame row carrying the keyboard-selection cursor, or -1 when none is drawn. */
function cursorRow(lines: readonly string[]): number {
	const cursor = theme.nav.cursor;
	return lines.findIndex(line => line.includes(cursor));
}

/**
 * True when `label` is drawn on the selected row (the cursor glyph shares the
 * line), as opposed to a detail/status line that merely mentions it.
 */
function selectedRowVisible(lines: readonly string[], label: string): boolean {
	const cursor = theme.nav.cursor;
	return lines.some(line => stripVTControlCharacters(line).includes(label) && line.includes(cursor));
}

/** Temporarily report `rows` for process.stdout.rows (the dialogs' viewport source). */
function stubStdoutRows(rows: number): () => void {
	const desc = Object.getOwnPropertyDescriptor(process.stdout, "rows");
	Object.defineProperty(process.stdout, "rows", { configurable: true, get: () => rows, set: () => {} });
	return () => {
		if (desc) Object.defineProperty(process.stdout, "rows", desc);
		else Reflect.deleteProperty(process.stdout, "rows");
	};
}

function makeModel(provider: string, id: string): Model {
	return buildModel({
		id,
		name: id,
		api: "ollama-chat",
		provider,
		baseUrl: "https://example.com",
		reasoning: false,
		input: ["text"],
		cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
		contextWindow: 128_000,
		maxTokens: 1024,
	});
}

function makeRegistry(models: Model[]): ModelRegistry {
	return {
		refresh: async () => {},
		refreshProvider: async () => {},
		getError: () => undefined,
		getAvailable: () => models,
		getAll: () => models,
		getDiscoverableProviders: () => [],
		getProviderDiscoveryState: () => undefined,
		authStorage: { hasAuth: () => false },
	} as unknown as ModelRegistry;
}

function fakeHistory(entries: HistoryEntry[]): HistoryStorage {
	return {
		getRecent: (limit: number) => entries.slice(0, limit),
		search: (query: string, limit: number) => {
			const tokens = query
				.toLowerCase()
				.split(/[^\p{L}\p{N}]+/u)
				.filter(Boolean);
			return entries
				.filter(entry => tokens.every(token => entry.prompt.toLowerCase().includes(token)))
				.slice(0, limit);
		},
	} as unknown as HistoryStorage;
}

function activityPoints(days: number): DailyActivityPoint[] {
	const points: DailyActivityPoint[] = [];
	const today = new Date();
	for (let i = days - 1; i >= 0; i--) {
		const date = new Date(today.getFullYear(), today.getMonth(), today.getDate() - i);
		const month = String(date.getMonth() + 1).padStart(2, "0");
		const day = String(date.getDate()).padStart(2, "0");
		points.push({
			day: `${date.getFullYear()}-${month}-${day}`,
			cost: (i % 5) + 0.5,
			requests: (i % 7) + 1,
			totalTokens: (i + 1) * 1000,
		});
	}
	return points;
}

let tempCwd = "";
const openHubs: ModelHubComponent[] = [];

beforeAll(async () => {
	await initTheme();
	resetSettingsForTest();
	await Settings.init({ inMemory: true });
	tempCwd = await fs.mkdtemp(path.join(os.tmpdir(), "omp-dialog-viewport-"));
});

afterAll(async () => {
	await removeWithRetries(tempCwd);
});

afterEach(() => {
	resetProviderAutoRefreshGuard();
	for (const hub of openHubs.splice(0)) hub.dispose();
	vi.restoreAllMocks();
});

describe("dialog viewport budgets", () => {
	for (const viewport of VIEWPORTS) {
		const { cols, rows } = viewport;

		test(`settings selector fits ${cols}x${rows} and keeps the selection visible`, () => {
			const restore = stubStdoutRows(rows);
			try {
				const selector = new SettingsSelectorComponent(
					{
						availableThinkingLevels: [],
						thinkingLevel: undefined,
						availableThemes: ["dark"],
						providers: [],
						cwd: process.cwd(),
					},
					{ onChange: () => {}, onCancel: () => {} },
				);
				let lines = selector.render(cols);
				expect(lines.length).toBeLessThanOrEqual(rows);
				expect(plain(lines)).toContain("Settings");
				expect(cursorRow(lines)).toBeGreaterThanOrEqual(0);
				expect(plain(lines)).toContain("Enter");

				// Move the selection past the viewport end: the scroll window
				// follows, so the cursor (selected row) stays drawn.
				for (let i = 0; i < 30; i++) selector.handleInput(DOWN);
				lines = selector.render(cols);
				expect(lines.length).toBeLessThanOrEqual(rows);
				expect(cursorRow(lines)).toBeGreaterThanOrEqual(0);
				expect(plain(lines)).toContain("Enter");
			} finally {
				restore();
			}
		});

		test(`settings search keeps its banner, selection, and footer at ${cols}x${rows}`, () => {
			const restore = stubStdoutRows(rows);
			try {
				const selector = new SettingsSelectorComponent(
					{
						availableThinkingLevels: [],
						thinkingLevel: undefined,
						availableThemes: ["dark"],
						providers: [],
						cwd: process.cwd(),
					},
					{ onChange: () => {}, onCancel: () => {} },
				);
				for (const char of "e") selector.handleInput(char);
				let lines = selector.render(cols);
				expect(lines.length).toBeLessThanOrEqual(rows);
				expect(cursorRow(lines)).toBeGreaterThanOrEqual(0);
				const text = plain(lines);
				expect(text).toContain("matches");
				expect(text).toContain("Esc");
				// Step the selection past the first window and then far beyond
				// it: the drawn window recenters on the selection instead of
				// clipping it from the top.
				selector.handleInput(DOWN);
				selector.handleInput(DOWN);
				lines = selector.render(cols);
				expect(lines.length).toBeLessThanOrEqual(rows);
				expect(cursorRow(lines)).toBeGreaterThanOrEqual(0);
				for (let i = 0; i < 30; i++) selector.handleInput(DOWN);
				lines = selector.render(cols);
				expect(lines.length).toBeLessThanOrEqual(rows);
				expect(cursorRow(lines)).toBeGreaterThanOrEqual(0);
				expect(plain(lines)).toContain("Enter");
			} finally {
				restore();
			}
		});

		test(`model hub list keeps search, selection, and footer at ${cols}x${rows}`, () => {
			// Distinct plain-word ids sort alphabetically, so 11 downs from the
			// top land deterministically on the last row.
			const ids = [
				"alpha",
				"bravo",
				"charlie",
				"delta",
				"echo",
				"foxtrot",
				"golf",
				"hotel",
				"india",
				"juliet",
				"kilo",
				"lima",
			];
			const ui = { requestRender: () => {}, terminal: { rows } } as unknown as TUI;
			const hub = new ModelHubComponent(
				ui,
				Settings.isolated({}),
				makeRegistry(ids.map(id => makeModel("acme", id))),
				[],
				{
					onAssign: () => {},
					onUnassign: () => {},
					onCancel: () => {},
				},
			);
			openHubs.push(hub);
			hub.handleInput(RIGHT);
			for (let i = 0; i < 11; i++) hub.handleInput(DOWN);
			const lines = hub.render(cols);
			expect(lines.length).toBeLessThanOrEqual(rows);
			const text = plain(lines);
			expect(text).toContain("Models");
			// The selected row itself (cursor line), not just the detail line
			// that describes the selection.
			expect(selectedRowVisible(lines, "lima")).toBe(true);
			// Search input row (magnifier + query editor) survives the budget.
			expect(raw(lines)).toContain(theme.symbol("icon.search"));
			expect(text).toContain("Enter");
		});

		test(`model hub sidebar keeps the active scope visible at ${cols}x${rows}`, () => {
			const providers = Array.from({ length: 12 }, (_, i) => `p${String(i).padStart(2, "0")}`);
			const ui = { requestRender: () => {}, terminal: { rows } } as unknown as TUI;
			const hub = new ModelHubComponent(
				ui,
				Settings.isolated({}),
				makeRegistry(providers.map(provider => makeModel(provider, "m"))),
				[],
				{ onAssign: () => {}, onUnassign: () => {}, onCancel: () => {} },
			);
			openHubs.push(hub);
			for (let i = 0; i < 12; i++) hub.handleInput(DOWN);
			const lines = hub.render(cols);
			expect(lines.length).toBeLessThanOrEqual(rows);
			const text = plain(lines);
			expect(text).toContain("Models");
			expect(selectedRowVisible(lines, "p11")).toBe(true);
			expect(text).toContain("Enter");
		});

		test(`agents hub keeps search, selection, and footer at ${cols}x${rows}`, async () => {
			const agents = Array.from({ length: 12 }, (_, i) => ({
				name: `agent-${String(i).padStart(2, "0")}`,
				description: `Agent ${i}`,
				systemPrompt: "",
				source: "bundled" as const,
			}));
			vi.spyOn(discovery, "discoverAgents").mockResolvedValue({ projectAgentsDir: null, agents });
			const ui = { requestRender: () => {}, terminal: { rows } } as unknown as TUI;
			const hub = await AgentsHubComponent.create(ui, tempCwd, Settings.isolated(), {}, { onCancel: () => {} });
			for (let i = 0; i < 11; i++) hub.handleInput(DOWN);
			const lines = hub.render(cols);
			expect(lines.length).toBeLessThanOrEqual(rows);
			const text = plain(lines);
			expect(text).toContain("Agents");
			expect(selectedRowVisible(lines, "agent-11")).toBe(true);
			expect(text).toContain("search:");
			expect(text).toContain("Enter");
			hub.dispose();
		});

		test(`usage dashboard fits ${cols}x${rows} across overview scroll and detail`, () => {
			const restore = stubStdoutRows(rows);
			try {
				const options = {
					reports: [],
					renderDetail: () => "detail-a\ndetail-b",
					loadActivity: (push: (points: DailyActivityPoint[]) => void) => {
						push(activityPoints(30));
						return Promise.resolve();
					},
					requestRender: () => {},
					onClose: () => {},
				};
				const dashboard = new UsageDashboardComponent(options);
				let lines = dashboard.render(cols);
				expect(lines.length).toBeLessThanOrEqual(rows);
				expect(plain(lines)).toContain("Usage");
				expect(plain(lines)).toContain("Esc");
				// Scroll past the content end: the frame stays within budget.
				for (let i = 0; i < 20; i++) dashboard.handleInput(DOWN);
				lines = dashboard.render(cols);
				expect(lines.length).toBeLessThanOrEqual(rows);
				expect(plain(lines)).toContain("Esc");
				dashboard.dispose();

				const detail = new UsageDashboardComponent(options);
				detail.handleInput("\r");
				lines = detail.render(cols);
				expect(lines.length).toBeLessThanOrEqual(rows);
				expect(plain(lines)).toContain("detail-a");
				detail.dispose();
			} finally {
				restore();
			}
		});

		test(`history search keeps input, selection, and hint at ${cols}x${rows}`, () => {
			const restore = stubStdoutRows(rows);
			try {
				const now = Math.floor(Date.now() / 1000);
				const entries: HistoryEntry[] = Array.from({ length: 15 }, (_, i) => ({
					id: i,
					prompt: `history prompt ${i}`,
					created_at: now - i * 60,
				}));
				const component = new HistorySearchComponent(
					fakeHistory(entries),
					() => {},
					() => {},
				);
				// Clamp to the oldest entry: the centered window must still draw it.
				for (let i = 0; i < 14; i++) component.handleInput(DOWN);
				const lines = component.render(cols);
				expect(lines.length).toBeLessThanOrEqual(rows);
				const text = plain(lines);
				expect(text).toContain("History");
				expect(text).toContain("history prompt 14");
				expect(text.toLowerCase()).toContain("esc");
				expect(raw(lines)).toContain(theme.getBgAnsi("selectedBg"));
			} finally {
				restore();
			}
		});
	}

	test("settings keeps its appearance preview on tall viewports", () => {
		const restore = stubStdoutRows(40);
		try {
			const selector = new SettingsSelectorComponent(
				{
					availableThinkingLevels: [],
					thinkingLevel: undefined,
					availableThemes: ["dark"],
					providers: [],
					cwd: process.cwd(),
				},
				{ onChange: () => {}, onCancel: () => {} },
			);
			const text = plain(selector.render(80));
			expect(text).toContain("Preview:");
		} finally {
			restore();
		}
	});
});
