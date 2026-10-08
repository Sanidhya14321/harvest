import { beforeAll, describe, expect, it, vi } from "bun:test";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import { IrcBus } from "@harvest/pi-coding-agent/irc/bus";
import { AgentHubOverlayComponent, filterSessionHandles } from "@harvest/pi-coding-agent/modes/components/agent-hub";
import { SessionObserverRegistry } from "@harvest/pi-coding-agent/modes/session-observer-registry";
import { initTheme, setThemeInstance, theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { createTheme, getBuiltinThemes } from "@harvest/pi-coding-agent/modes/theme/loader";
import { AgentRegistry } from "@harvest/pi-coding-agent/registry/agent-registry";
import type { ManagedSessionHandle } from "@harvest/pi-coding-agent/session/session-management-facade";

beforeAll(() => {
	initTheme();
});

function handles(): ManagedSessionHandle[] {
	return [
		{
			id: "session-running",
			path: "/work/running.jsonl",
			title: "Running work",
			status: "running",
			selected: true,
			visible: true,
			archived: false,
		},
		{
			id: "session-hidden",
			path: "/work/hidden.jsonl",
			title: "Hidden unsaved draft",
			status: "idle",
			selected: false,
			visible: false,
			archived: false,
		},
		{
			id: "session-unsaved",
			path: undefined,
			title: undefined,
			status: "waiting",
			selected: false,
			visible: false,
			archived: false,
		},
		{
			id: "session-archived",
			path: "/work/old.jsonl",
			title: "Old work",
			status: "archived",
			selected: false,
			visible: false,
			archived: true,
		},
		{
			id: "session-child",
			path: "/work/child.jsonl",
			title: "Stopped child",
			status: "completed",
			selected: false,
			visible: false,
			archived: false,
		},
	];
}

function makeHub(
	overrides: {
		onSessionReopen?: (id: string) => void | Promise<void>;
		onSessionStop?: (id: string) => void | Promise<void>;
		onSessionSend?: (id: string, text: string) => void | Promise<void>;
		sessionList?: () => ManagedSessionHandle[];
	} = {},
) {
	const agents = new AgentRegistry();
	const hub = new AgentHubOverlayComponent({
		settings: Settings.isolated(),
		observers: new SessionObserverRegistry(),
		hubKeys: [],
		onDone: () => {},
		requestRender: () => {},
		registry: agents,
		irc: new IrcBus(agents),
		sessionList: () => handles(),
		...overrides,
	});
	return hub;
}

function sessionsText(hub: AgentHubOverlayComponent, width = 100): string {
	hub.handleInput("3");
	return Bun.stripANSI(hub.render(width).join("\n"));
}

describe("agent hub Sessions section", () => {
	it("keeps tiny-screen sends tied to the selected stable session ID and preserves the full message", async () => {
		const previousTheme = theme;
		setThemeInstance(createTheme(getBuiltinThemes().harvest, { mode: "none", symbolPresetOverride: "ascii" }));
		const sent: Array<{ id: string; text: string }> = [];
		const hub = makeHub({
			onSessionSend: (id, text) => {
				sent.push({ id, text });
			},
		});
		try {
			hub.handleInput("3");
			hub.handleInput("j"); // hidden session still owns its message
			hub.setMaxHeight(1);
			hub.handleInput("m");
			const full = "A long message whose preserved payload ends in final-tail";
			for (const character of full) hub.handleInput(character);
			const tiny = hub.render(24);
			expect(tiny).toHaveLength(1);
			expect(tiny[0]).toContain("final-tail_");
			hub.setMaxHeight(24);
			expect(Bun.stripANSI(hub.render(120).join("\n"))).toContain(full);
			hub.handleInput("\r");
			await Bun.sleep(0);
			expect(sent).toEqual([{ id: "session-hidden", text: full }]);
		} finally {
			hub.dispose();
			setThemeInstance(previousTheme);
		}
	});
	it("filters facade snapshots without dropping hidden or unsaved sessions", () => {
		// Failure mode: hidden/unsaved sessions vanish from every filter and
		// become unreachable once their tab closes.
		const all = handles();
		expect(filterSessionHandles(all, "all", "").map(h => h.id)).toEqual([
			"session-running",
			"session-hidden",
			"session-unsaved",
			"session-archived",
			"session-child",
		]);
		expect(filterSessionHandles(all, "open", "").map(h => h.id)).toEqual(["session-running"]);
		expect(filterSessionHandles(all, "running", "").map(h => h.id)).toEqual(["session-running"]);
		expect(filterSessionHandles(all, "attention", "").map(h => h.id)).toEqual(["session-unsaved"]);
		expect(filterSessionHandles(all, "closed", "").map(h => h.id)).toEqual([
			"session-hidden",
			"session-unsaved",
			"session-child",
		]);
		expect(filterSessionHandles(all, "archived", "").map(h => h.id)).toEqual(["session-archived"]);
		expect(filterSessionHandles(all, "all", "hidden unsaved").map(h => h.id)).toEqual(["session-hidden"]);
	});

	it("renders the Sessions section with state per session, distinct from Activity rows", () => {
		const hub = makeHub();
		try {
			const text = sessionsText(hub);
			expect(text).toContain("3 Sessions");
			expect(text).toContain("Running work");
			expect(text).toContain("Hidden unsaved draft");
			// Pathless (never-saved) sessions stay reachable with a marker.
			expect(text).toContain("(unsaved");
			// Children of stopped parents show their actual state, verbatim.
			expect(text).toContain("Stopped child");
			expect(text).toContain("done");
		} finally {
			hub.dispose();
		}
	});

	it("exposes Reopen on Enter and targeted Stop on x", async () => {
		// Failure mode: closed running sessions are visible but offer no
		// Reopen/Stop actions, stranding their runtimes.
		const onSessionReopen = vi.fn();
		const onSessionStop = vi.fn();
		const hub = makeHub({ onSessionReopen, onSessionStop });
		try {
			sessionsText(hub);
			hub.handleInput("\n");
			expect(onSessionReopen).toHaveBeenCalledTimes(1);
			expect(onSessionReopen).toHaveBeenCalledWith("session-running");

			hub.handleInput("x");
			expect(onSessionStop).toHaveBeenCalledTimes(1);
			expect(onSessionStop).toHaveBeenCalledWith("session-running");
			await Bun.sleep(0);
		} finally {
			hub.dispose();
		}
	});

	it("cycles filters and searches without leaving the section", () => {
		const hub = makeHub();
		try {
			let text = sessionsText(hub);
			expect(text).toContain("filter:all");

			hub.handleInput("f");
			text = Bun.stripANSI(hub.render(100).join("\n"));
			expect(text).toContain("filter:open");
			expect(text).toContain("Running work");
			expect(text).not.toContain("Hidden unsaved draft");
		} finally {
			hub.dispose();
		}
	});

	it("sends text to the selected session where supported", async () => {
		const onSessionSend = vi.fn();
		const hub = makeHub({ onSessionSend });
		try {
			sessionsText(hub);
			hub.handleInput("m");
			for (const ch of "hi") hub.handleInput(ch);
			hub.handleInput("\n");
			await Bun.sleep(0);
			expect(onSessionSend).toHaveBeenCalledTimes(1);
			expect(onSessionSend).toHaveBeenCalledWith("session-running", "hi");
		} finally {
			hub.dispose();
		}
	});
});
