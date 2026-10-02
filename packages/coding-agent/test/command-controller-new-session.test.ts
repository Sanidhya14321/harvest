import { beforeAll, describe, expect, it, vi } from "bun:test";
import { CommandController } from "@harvest/pi-coding-agent/modes/controllers/command-controller";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { InteractiveModeContext } from "@harvest/pi-coding-agent/modes/types";
import type { AgentSession } from "@harvest/pi-coding-agent/session/agent-session";
import { LiveSessionRegistry } from "@harvest/pi-coding-agent/session/live-session-registry";

beforeAll(async () => {
	await initTheme(false);
});

interface NewSessionHarness {
	ctx: InteractiveModeContext;
	controller: CommandController;
	counts: {
		newSession: () => number;
		unfocusSession: () => number;
		resetTranscriptAnchors: () => number;
		resetTranscript: () => number;
		presented: () => number;
		status: () => number;
		renders: () => number;
	};
	setFocused: (id: string | undefined) => void;
}

function makeHarness(fullscreen = false): NewSessionHarness {
	let newSession = 0;
	let unfocusSession = 0;
	let resetTranscriptAnchors = 0;
	let resetTranscript = 0;
	let presented = 0;
	let status = 0;
	let renders = 0;
	let focusedAgentId: string | undefined = "subagent-1";

	const ctx = {
		session: {
			isCompacting: false,
			newSession: async () => {
				newSession++;
				return true;
			},
		},
		sessionManager: {
			getSessionName: () => undefined,
			getCwd: () => "/tmp",
		},
		get focusedAgentId() {
			return focusedAgentId;
		},
		unfocusSession: async () => {
			unfocusSession++;
			focusedAgentId = undefined;
		},
		eventController: {
			resetTranscriptAnchors: () => {
				resetTranscriptAnchors++;
			},
		},
		resetObserverRegistry: () => {},
		statusLine: {
			invalidate: () => {},
			resetActiveTime: () => {},
		},
		updateEditorBorderColor: () => {},
		clearTransientSessionUi: () => {},
		resetTranscript: () => {
			resetTranscript++;
		},
		present: () => {
			presented++;
		},
		showStatus: () => {
			status++;
		},
		settings: { get: (key: string) => key === "tui.fullscreen" && fullscreen },
		reloadTodos: async () => {},
		ui: {
			requestRender: () => {
				renders++;
			},
		},
	} as unknown as InteractiveModeContext;

	return {
		ctx,
		controller: new CommandController(ctx),
		counts: {
			newSession: () => newSession,
			unfocusSession: () => unfocusSession,
			resetTranscriptAnchors: () => resetTranscriptAnchors,
			resetTranscript: () => resetTranscript,
			presented: () => presented,
			status: () => status,
			renders: () => renders,
		},
		setFocused: id => {
			focusedAgentId = id;
		},
	};
}

describe("CommandController new-session teardown", () => {
	it("returns a focused subagent view to main and purges transcript anchors on /new", async () => {
		const harness = makeHarness();

		await harness.controller.handleClearCommand();

		expect(harness.counts.newSession()).toBe(1);
		expect(harness.counts.unfocusSession()).toBe(1);
		expect(harness.ctx.focusedAgentId).toBeUndefined();
		expect(harness.counts.resetTranscriptAnchors()).toBe(1);
		expect(harness.counts.resetTranscript()).toBe(1);
		expect(harness.counts.presented()).toBe(1);
	});

	it("skips the unfocus round-trip when already on the main session", async () => {
		const harness = makeHarness();
		harness.setFocused(undefined);

		await harness.controller.handleClearCommand();

		expect(harness.counts.newSession()).toBe(1);
		expect(harness.counts.unfocusSession()).toBe(0);
		expect(harness.counts.resetTranscriptAnchors()).toBe(1);
		expect(harness.counts.resetTranscript()).toBe(1);
	});

	it("keeps the fullscreen landing view clear after creating a session", async () => {
		const harness = makeHarness(true);
		await harness.controller.handleClearCommand();
		expect(harness.counts.newSession()).toBe(1);
		expect(harness.counts.presented()).toBe(0);
		expect(harness.counts.status()).toBe(1);
		expect(harness.counts.renders()).toBe(1);
	});
});

describe("CommandController live-tab new session", () => {
	function makeLiveSession(id: string, isStreaming = true) {
		const abort = vi.fn(async () => {});
		const dispose = vi.fn(async () => {});
		const newSession = vi.fn(async () => true);
		const session = {
			isStreaming,
			isCompacting: false,
			newSession,
			sessionManager: {
				getSessionId: () => id,
				getSessionFile: () => `/work/${id}.jsonl`,
				getSessionName: () => `Task ${id}`,
				getCwd: () => "/work",
				getSessionDir: () => "/work/.sessions",
				onSessionNameChanged: () => () => {},
			},
			settings: {},
			modelRegistry: {},
			model: { provider: "mock", id: "mock-model" },
			effectiveExtensionRoots: {},
			getAvailableModels: () => [],
			subscribe: () => () => {},
			abort,
			dispose,
		} as unknown as AgentSession;
		return { session, abort, dispose, newSession };
	}

	function makeLiveHarness(
		openLiveSession?: (options: Record<string, unknown>) => Promise<AgentSession>,
		isStreaming = true,
	) {
		const current = makeLiveSession("current", isStreaming);
		const fresh = makeLiveSession("fresh");
		const seen: Record<string, unknown>[] = [];
		let presented = 0;
		let status = 0;
		const errors: string[] = [];
		const selected: AgentSession[] = [];
		const registry = new LiveSessionRegistry(current.session, async () => {
			throw new Error("unexpected cold open");
		});
		const ctx = {
			session: current.session,
			sessionManager: current.session.sessionManager,
			settings: { get: (key: string) => key === "tui.fullscreen" },
			eventController: { resetTranscriptAnchors: () => {} },
			resetObserverRegistry: () => {},
			statusLine: { invalidate: () => {}, resetActiveTime: () => {} },
			updateEditorBorderColor: () => {},
			clearTransientSessionUi: () => {},
			resetTranscript: () => {},
			present: () => {
				presented++;
			},
			showStatus: () => {
				status++;
			},
			showError: (message: string) => {
				errors.push(message);
			},
			reloadTodos: async () => {},
			ui: { requestRender: () => {} },
			focusedAgentId: undefined,
			liveSessions: registry,
			selectMainSession: async (session: AgentSession) => {
				selected.push(session);
			},
			openLiveSession: openLiveSession
				? async (options: Record<string, unknown>) => {
						seen.push(options);
						return openLiveSession(options);
					}
				: undefined,
		} as unknown as InteractiveModeContext;
		return {
			ctx,
			controller: new CommandController(ctx),
			registry,
			current,
			fresh,
			seen,
			errors,
			selected,
			counts: { presented: () => presented, status: () => status },
		};
	}

	it("opens an independent tab without aborting the previous run", async () => {
		const harness = makeLiveHarness(async () => harness.fresh.session);
		await harness.controller.handleClearCommand();

		expect(harness.current.newSession).not.toHaveBeenCalled();
		expect(harness.current.abort).not.toHaveBeenCalled();
		expect(harness.registry.sessions).toContain(harness.fresh.session);
		expect(harness.selected).toEqual([harness.fresh.session]);
		expect(harness.seen).toHaveLength(1);
		expect(harness.counts.status()).toBe(1);
	});

	it("falls back to the in-place session when the factory fails", async () => {
		const harness = makeLiveHarness(async () => {
			throw new Error("no model resolved");
		}, false);
		await harness.controller.handleClearCommand();

		expect(harness.current.newSession).toHaveBeenCalledTimes(1);
		expect(harness.errors.join("\n")).toContain("falling back");
		expect(harness.counts.status()).toBe(1);
	});

	it("preserves an active run when independent tab creation fails", async () => {
		const harness = makeLiveHarness(async () => {
			throw new Error("sidecar startup failed");
		});
		await harness.controller.handleClearCommand();
		expect(harness.current.newSession).not.toHaveBeenCalled();
		expect(harness.current.abort).not.toHaveBeenCalled();
		expect(harness.selected).toEqual([]);
		expect(harness.errors.join(" ")).toContain("still active");
	});

	it("closes an independent runtime when shutdown wins the race before adoption", async () => {
		const harness = makeLiveHarness(async () => {
			await harness.registry.dispose();
			return harness.fresh.session;
		});
		await harness.controller.handleClearCommand();
		expect(harness.fresh.dispose).toHaveBeenCalledTimes(1);
		expect(harness.selected).toEqual([]);
		expect(harness.errors.join(" ")).toContain("closed");
	});
});
