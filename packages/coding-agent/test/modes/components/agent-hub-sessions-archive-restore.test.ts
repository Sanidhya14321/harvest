import { beforeAll, describe, expect, it, vi } from "bun:test";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import { IrcBus } from "@harvest/pi-coding-agent/irc/bus";
import {
	AgentHubOverlayComponent,
	filterSessionHandles,
	mergeSessionHandles,
} from "@harvest/pi-coding-agent/modes/components/agent-hub";
import { SessionObserverRegistry } from "@harvest/pi-coding-agent/modes/session-observer-registry";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { AgentRegistry } from "@harvest/pi-coding-agent/registry/agent-registry";
import type { ManagedSessionHandle } from "@harvest/pi-coding-agent/session/session-management-facade";

beforeAll(() => {
	initTheme();
});

function handles(): ManagedSessionHandle[] {
	return [
		{
			id: "live-running",
			path: "/work/running.jsonl",
			title: "Running work",
			status: "running",
			selected: true,
			visible: true,
			archived: false,
		},
		{
			id: "live-idle",
			path: "/work/idle.jsonl",
			title: "Idle work",
			status: "idle",
			selected: false,
			visible: true,
			archived: false,
		},
		{
			id: "cold-archived",
			path: "/work/old.jsonl",
			title: "Old work",
			status: "archived",
			selected: false,
			visible: false,
			archived: true,
		},
		{
			id: "child-done",
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
		onSessionArchive?: (id: string) => void | Promise<void>;
		onSessionRestore?: (id: string) => void | Promise<void>;
		onSessionSend?: (id: string, text: string) => void | Promise<void>;
		sessionList?: () => ManagedSessionHandle[];
		sessionListAsync?: () => Promise<ManagedSessionHandle[]>;
	} = {},
): AgentHubOverlayComponent {
	const agents = new AgentRegistry();
	return new AgentHubOverlayComponent({
		settings: Settings.isolated(),
		observers: new SessionObserverRegistry(),
		hubKeys: [],
		onDone: () => {},
		requestRender: () => {},
		registry: agents,
		irc: new IrcBus(agents),
		sessionList: () => handles().slice(0, 2),
		...overrides,
	});
}

function sessionsText(hub: AgentHubOverlayComponent, width = 110): string {
	hub.handleInput("3");
	return Bun.stripANSI(hub.render(width).join("\n"));
}

describe("hub Sessions archive/restore", () => {
	it("archived filter reflects real archived flags, never a hardcoded value", () => {
		const all = handles();
		expect(filterSessionHandles(all, "archived", "").map(h => h.id)).toEqual(["cold-archived"]);
		expect(filterSessionHandles(all, "open", "").map(h => h.id)).toEqual(["live-running", "live-idle"]);
		expect(filterSessionHandles(all, "closed", "").map(h => h.id)).toEqual(["child-done"]);
	});

	it("merges cold async rows without duplicating live IDs", () => {
		const live = handles().slice(0, 2);
		const asyncRows: ManagedSessionHandle[] = [
			{ ...live[0]!, status: "idle" },
			{
				id: "cold-archived",
				path: "/work/old.jsonl",
				title: "Old work",
				status: "archived",
				selected: false,
				visible: false,
				archived: true,
			},
		];
		const merged = mergeSessionHandles(live, asyncRows);
		expect(merged.map(h => h.id)).toEqual(["live-running", "live-idle", "cold-archived"]);
		// Live wins on duplicates (freshest runtime state).
		expect(merged[0]!.status).toBe("running");
		expect(merged.find(h => h.id === "cold-archived")!.archived).toBe(true);
	});

	it("archive calls through to the owner and surfaces busy refusals honestly", async () => {
		const onSessionArchive = vi.fn(async (_id: string) => {});
		const hub = makeHub({
			sessionList: () => handles(),
			onSessionArchive,
		});
		try {
			sessionsText(hub);
			// Select the idle row (index 1) then archive with `a`.
			hub.handleInput("j");
			hub.handleInput("a");
			await Bun.sleep(0);
			expect(onSessionArchive).toHaveBeenCalledTimes(1);
			expect(onSessionArchive).toHaveBeenCalledWith("live-idle");

			// Already-archived rows report honestly instead of re-archiving.
			onSessionArchive.mockClear();
			hub.handleInput("j");
			hub.handleInput("a");
			await Bun.sleep(0);
			expect(onSessionArchive).not.toHaveBeenCalled();
			expect(Bun.stripANSI(hub.render(110).join("\n"))).toContain("already archived");
		} finally {
			hub.dispose();
		}
	});

	it("restore calls through only for archived rows", async () => {
		const onSessionRestore = vi.fn(async (_id: string) => {});
		const hub = makeHub({
			sessionList: () => handles(),
			onSessionRestore,
		});
		try {
			sessionsText(hub);
			// First row is running (not archived) → honest no-op.
			hub.handleInput("u");
			await Bun.sleep(0);
			expect(onSessionRestore).not.toHaveBeenCalled();
			expect(Bun.stripANSI(hub.render(110).join("\n"))).toContain("not archived");

			// Move to the archived row and restore.
			hub.handleInput("j");
			hub.handleInput("j");
			hub.handleInput("u");
			await Bun.sleep(0);
			expect(onSessionRestore).toHaveBeenCalledTimes(1);
			expect(onSessionRestore).toHaveBeenCalledWith("cold-archived");
		} finally {
			hub.dispose();
		}
	});

	it("missing archive/restore callbacks report unavailable instead of failing silently", () => {
		const hub = makeHub({ sessionList: () => handles() });
		try {
			sessionsText(hub);
			hub.handleInput("a");
			expect(Bun.stripANSI(hub.render(110).join("\n"))).toContain("Archive is unavailable");
			hub.handleInput("u");
			expect(Bun.stripANSI(hub.render(110).join("\n"))).toContain("Restore is unavailable");
		} finally {
			hub.dispose();
		}
	});

	it("stop stays targeted: idle rows refuse, children show actual state", () => {
		const onSessionStop = vi.fn();
		const hub = makeHub({
			sessionList: () => handles(),
			onSessionStop,
		});
		try {
			const text = sessionsText(hub);
			// Children of stopped parents show actual state verbatim (no cascade).
			expect(text).toContain("Stopped child");
			expect(text).toContain("done");
			// Idle row stop is honestly disabled (no force-kill).
			hub.handleInput("j");
			hub.handleInput("x");
			expect(onSessionStop).not.toHaveBeenCalled();
			expect(Bun.stripANSI(hub.render(110).join("\n"))).toContain("only running or waiting");
		} finally {
			hub.dispose();
		}
	});

	it("async cold source merges without dropping live rows", async () => {
		const hub = makeHub({
			sessionList: () => handles().slice(0, 2),
			sessionListAsync: async () => handles().slice(2),
		});
		try {
			let text = sessionsText(hub);
			expect(text).toContain("Running work");
			// Cold rows arrive after the async source resolves.
			await Bun.sleep(10);
			text = Bun.stripANSI(hub.render(110).join("\n"));
			expect(text).toContain("Old work");
			expect(text).toContain("archived");
		} finally {
			hub.dispose();
		}
	});
});
