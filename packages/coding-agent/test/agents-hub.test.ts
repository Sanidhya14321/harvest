/**
 * Contracts of the fullscreen /agents hub: frame geometry, scope sidebar
 * filtering, type-to-filter search, the Space enable/disable toggle, and the
 * strip-driven configuration flows (property strips, pattern input, and the
 * model-browser pick) persisting to the per-agent settings records.
 */
import { afterAll, afterEach, beforeAll, describe, expect, test, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { Effort } from "@harvest/pi-ai";
import { buildModel } from "@harvest/pi-catalog/build";
import type { ModelRegistry } from "@harvest/pi-coding-agent/config/model-registry";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import { AgentsHubComponent } from "@harvest/pi-coding-agent/modes/components/agents-hub";
import { initTheme, setThemeInstance, theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { createTheme, getBuiltinThemes } from "@harvest/pi-coding-agent/modes/theme/loader";
import * as discovery from "@harvest/pi-coding-agent/task/discovery";
import { type TUI, visibleWidth } from "@harvest/pi-tui";
import { removeWithRetries } from "@harvest/pi-utils";

const ANSI_PATTERN = /\x1b\[[0-?]*[ -/]*[@-~]/g;
let tempCwd: string;

// Narrow TUI stub: the hub only reads terminal rows and requests renders.
const tuiStub = { requestRender: () => {}, terminal: { rows: 30 } } as unknown as TUI;

const sonnet = buildModel({
	id: "claude-sonnet-4-5",
	name: "Claude Sonnet 4.5",
	api: "anthropic-messages",
	provider: "anthropic",
	baseUrl: "https://api.anthropic.com",
	reasoning: true,
	thinking: { mode: "budget", efforts: [Effort.Low, Effort.Medium, Effort.High] },
	input: ["text"],
	cost: { input: 3, output: 15, cacheRead: 0.3, cacheWrite: 3.75 },
	contextWindow: 200000,
	maxTokens: 8192,
});

// Registry stub: the hub uses getAvailable() for browser items and resolution.
const registryStub = { getAvailable: () => [sonnet] } as unknown as ModelRegistry;

function mockAgents(): void {
	vi.spyOn(discovery, "discoverAgents").mockResolvedValue({
		projectAgentsDir: null,
		agents: [
			{ name: "dev", description: "Development agent", systemPrompt: "", source: "project" },
			{ name: "scout", description: "Read-only research", systemPrompt: "", source: "bundled" },
			{ name: "task", description: "Generic task agent", systemPrompt: "", source: "bundled" },
		],
	});
}

async function createHub(settings: Settings): Promise<{
	hub: AgentsHubComponent;
	strip: () => string;
	type: (text: string) => void;
	cancelled: () => boolean;
}> {
	let cancelled = false;
	const hub = await AgentsHubComponent.create(
		tuiStub,
		tempCwd,
		settings,
		{ modelRegistry: registryStub },
		{ onCancel: () => (cancelled = true) },
	);
	return {
		hub,
		strip: () => hub.render(120).join("\n").replace(ANSI_PATTERN, ""),
		type: (text: string) => {
			for (const char of text) hub.handleInput(char);
		},
		cancelled: () => cancelled,
	};
}

beforeAll(async () => {
	await initTheme(false);
	tempCwd = await fs.mkdtemp(path.join(os.tmpdir(), "omp-agents-hub-"));
});

afterAll(async () => {
	await removeWithRetries(tempCwd);
});

afterEach(() => {
	vi.restoreAllMocks();
});

describe("AgentsHub layout", () => {
	test("keeps the selected agent and pointer actions inside the allocated single pane after shrinking", async () => {
		mockAgents();
		const previousTheme = theme;
		setThemeInstance(createTheme(getBuiltinThemes().harvest, { mode: "none", symbolPresetOverride: "ascii" }));
		const settings = Settings.isolated();
		const { hub } = await createHub(settings);
		try {
			hub.setMaxHeight(4);
			const rows = hub.render(24);
			expect(rows).toHaveLength(4);
			expect(rows.map(Bun.stripANSI).join("\n")).toContain("dev");
			expect(rows.every(line => visibleWidth(line) === 24)).toBe(true);
			hub.handleInput("\x1b[<0;4;4M"); // footer is not an agent
			expect(settings.get("task.disabledAgents") ?? []).toEqual([]);
			const scoutRow = rows.findIndex(line => Bun.stripANSI(line).includes("scout"));
			expect(scoutRow).toBeGreaterThan(0);
			hub.handleInput(`\x1b[<0;4;${scoutRow + 1}M`);
			hub.handleInput(" ");
			expect(settings.get("task.disabledAgents")).toEqual(["scout"]);
			hub.setMaxHeight(1);
			expect(Bun.stripANSI(hub.render(24).join("\n"))).toContain("scout");
			hub.handleInput("\x1b[D");
			expect(Bun.stripANSI(hub.render(24).join("\n"))).toContain("All agents");
			hub.handleInput("\x1b[C");
			hub.setMaxHeight(30);
			expect(Bun.stripANSI(hub.render(120).join("\n"))).toContain("scout");
		} finally {
			hub.dispose();
			setThemeInstance(previousTheme);
		}
	});

	test("shows the insertion end in a one-row pattern prompt and saves the full pattern after expansion", async () => {
		mockAgents();
		const settings = Settings.isolated();
		const { hub, type } = await createHub(settings);
		try {
			hub.handleInput("\r");
			hub.handleInput("\r");
			hub.handleInput("\x1b[C");
			hub.handleInput("\r");
			const pattern = "provider/model-with-a-long-identifier-final-tail";
			type(pattern);
			hub.setMaxHeight(1);
			const tiny = hub.render(24);
			expect(tiny).toHaveLength(1);
			expect(Bun.stripANSI(tiny[0]!)).toContain("final-tail");
			hub.setMaxHeight(30);
			expect(Bun.stripANSI(hub.render(120).join("\n"))).toContain(pattern);
			hub.handleInput("\r");
			expect(settings.get("task.agentModelOverrides")).toEqual({ dev: pattern });
		} finally {
			hub.dispose();
		}
	});
	test("renders the full-height split frame with sidebar scopes and agent rows", async () => {
		mockAgents();
		const { hub, strip } = await createHub(Settings.isolated());
		const lines = hub.render(120);
		// top border + content rows + divider + footer + bottom border = terminal rows.
		expect(lines.length).toBe(30);
		const rendered = strip();
		expect(rendered).toContain("Agents");
		expect(rendered).toContain("All agents");
		expect(rendered).toContain("Project");
		expect(rendered).toContain("Bundled");
		expect(rendered).toContain("dev");
		expect(rendered).toContain("scout");
		expect(rendered).toContain("+ New agent");
	});

	test("sidebar scope filters the rows to one source", async () => {
		mockAgents();
		const { hub, strip } = await createHub(Settings.isolated());
		hub.handleInput("\x1b[D"); // left → scope focus
		hub.handleInput("\x1b[B"); // down → Project
		const rendered = strip();
		expect(rendered).toContain("Project agents · 1");
		expect(rendered).toContain("dev");
		expect(rendered).not.toContain("scout");
	});

	test("type-to-filter narrows the list and Esc clears the query first", async () => {
		mockAgents();
		const { hub, strip, type, cancelled } = await createHub(Settings.isolated());
		type("sco");
		let rendered = strip();
		expect(rendered).toContain("scout");
		expect(rendered).not.toContain("dev");
		hub.handleInput("\x1b"); // Esc clears the query, not the hub
		expect(cancelled()).toBe(false);
		rendered = strip();
		expect(rendered).toContain("dev");
		hub.handleInput("\x1b");
		expect(cancelled()).toBe(true);
	});
});

describe("AgentsHub configuration strips", () => {
	test("Space toggles the selected agent's enabled state", async () => {
		mockAgents();
		const settings = Settings.isolated();
		const { hub } = await createHub(settings);
		hub.handleInput(" ");
		expect(settings.get("task.disabledAgents")).toEqual(["dev"]);
		hub.handleInput(" ");
		expect(settings.get("task.disabledAgents")).toEqual([]);
	});

	test("Enter opens the property strip; advisor → on persists task.agentAdvisor", async () => {
		mockAgents();
		const settings = Settings.isolated();
		const { hub, strip } = await createHub(settings);
		hub.handleInput("\r"); // agent strip for `dev`
		expect(strip()).toContain("dev →");
		hub.handleInput("\x1b[C"); // model → prewalk
		hub.handleInput("\x1b[C"); // prewalk → advisor
		hub.handleInput("\r"); // advisor value strip
		expect(strip()).toContain("dev · advisor →");
		hub.handleInput("\x1b[C"); // agent default → on
		hub.handleInput("\r");
		expect(settings.get("task.agentAdvisor")).toEqual({ dev: "on" });
		expect(strip()).toContain("dev advisor: on (@advisor)");
	});

	test("pattern… commits a custom advisor pattern and empty submit clears it", async () => {
		mockAgents();
		const settings = Settings.isolated();
		settings.set("task.agentAdvisor", { dev: "on" });
		const { hub, type } = await createHub(settings);
		hub.handleInput("\r");
		hub.handleInput("\x1b[C");
		hub.handleInput("\x1b[C");
		hub.handleInput("\r"); // advisor strip
		// agent default → on → off → pick model… → pattern…
		for (let i = 0; i < 4; i++) hub.handleInput("\x1b[C");
		hub.handleInput("\r"); // pattern input, pre-filled "on"
		type("\x7f\x7f"); // clear the prefill
		type("moonshot/k3:high");
		hub.handleInput("\r");
		expect(settings.get("task.agentAdvisor")).toEqual({ dev: "moonshot/k3:high" });
	});

	test("pick model… dives into the model browser and persists the model override", async () => {
		mockAgents();
		const settings = Settings.isolated();
		const { hub, strip } = await createHub(settings);
		hub.handleInput("\r"); // agent strip (model chip preselected)
		hub.handleInput("\r"); // model value strip → [pick model…] first
		expect(strip()).toContain("dev · model →");
		hub.handleInput("\r"); // assign mode: model browser
		expect(strip()).toContain("Picking model override for dev");
		expect(strip()).toContain("claude-sonnet-4-5");
		hub.handleInput("\r"); // pick the only model
		expect(settings.get("task.agentModelOverrides")).toEqual({ dev: "anthropic/claude-sonnet-4-5" });
		// Back on the list with the override reflected.
		expect(strip()).toContain("anthropic/claude-sonnet-4-5");
	});

	test("clear override chip removes an existing model override", async () => {
		mockAgents();
		const settings = Settings.isolated();
		settings.set("task.agentModelOverrides", { dev: "anthropic/claude-sonnet-4-5" });
		const { hub, strip } = await createHub(settings);
		hub.handleInput("\r"); // agent strip
		hub.handleInput("\r"); // model value strip
		expect(strip()).toContain("clear override");
		hub.handleInput("\x1b[C"); // pick model… → pattern…
		hub.handleInput("\x1b[C"); // pattern… → clear override
		hub.handleInput("\r");
		expect(settings.get("task.agentModelOverrides")).toEqual({});
	});

	test("Esc steps back from a value strip to the agent strip before closing", async () => {
		mockAgents();
		const settings = Settings.isolated();
		const { hub, strip, cancelled } = await createHub(settings);
		hub.handleInput("\r"); // agent strip
		hub.handleInput("\r"); // model value strip
		hub.handleInput("\x1b"); // back to agent strip
		expect(strip()).toContain("dev →");
		hub.handleInput("\x1b"); // close strip
		expect(strip()).not.toContain("dev →");
		expect(cancelled()).toBe(false);
	});
});
