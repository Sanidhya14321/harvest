import { beforeAll, describe, expect, it } from "bun:test";
import { CommandController } from "@harvest/pi-coding-agent/modes/controllers/command-controller";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { InteractiveModeContext } from "@harvest/pi-coding-agent/modes/types";

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
