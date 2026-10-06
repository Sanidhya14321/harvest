/**
 * Regression coverage for per-turn usage placement across transcript rebuilds.
 * Read-only turns keep their metrics inside the compact read group; other
 * assistant turns retain a standalone row below their visible content/tools.
 */
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "bun:test";
import type { AgentMessage } from "@harvest/pi-agent-core";
import type { AssistantMessage } from "@harvest/pi-ai";
import { resetSettingsForTest, Settings, settings } from "@harvest/pi-coding-agent/config/settings";
import { ChatTranscriptBuilder } from "@harvest/pi-coding-agent/modes/components/chat-transcript-builder";
import { ReadToolGroupComponent } from "@harvest/pi-coding-agent/modes/components/read-tool-group";
import { ToolExecutionComponent } from "@harvest/pi-coding-agent/modes/components/tool-execution";
import { formatCompletionEndcap } from "@harvest/pi-coding-agent/modes/components/usage-row";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { InteractiveModeContext } from "@harvest/pi-coding-agent/modes/types";
import { UiHelpers } from "@harvest/pi-coding-agent/modes/utils/ui-helpers";
import type { SessionContext } from "@harvest/pi-coding-agent/session/session-context";
import { Container, TUI } from "@harvest/pi-tui";
import { formatNumber } from "@harvest/pi-utils";
import { VirtualTerminal } from "../../tui/test/virtual-terminal";

// 4242 → "4.2K": distinctive enough not to collide with a read group's render.
const USAGE_INPUT = 4242;
const USAGE_LABEL = formatNumber(USAGE_INPUT);
// Fixed local wall-clock time so the rendered stamp is deterministic across time zones.
// Single-digit month/day/hour/minute/second exercise the formatter's zero-padding.
const USAGE_TS = new Date(2026, 0, 2, 3, 4, 5).getTime();
const USAGE_TS_LABEL = "2026-01-02 03:04:05";
const SECOND_USAGE_TS = new Date(2026, 0, 2, 3, 4, 6).getTime();
const SECOND_USAGE_TS_LABEL = "2026-01-02 03:04:06";

function readTurn(
	toolCallId = "r1",
	filePath = "src/foo.ts",
	usageInput = USAGE_INPUT,
	timestamp = USAGE_TS,
): AgentMessage[] {
	const assistant = {
		role: "assistant",
		content: [{ type: "toolCall", id: toolCallId, name: "read", arguments: { path: filePath } }],
		api: "anthropic-messages",
		provider: "anthropic",
		model: "claude-sonnet-4-5",
		stopReason: "stop",
		usage: {
			input: usageInput,
			output: 7,
			cacheRead: 0,
			cacheWrite: 0,
			totalTokens: usageInput + 7,
			cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
		},
		timestamp,
	} as unknown as AgentMessage;
	const toolResult = {
		role: "toolResult",
		toolCallId,
		toolName: "read",
		content: [{ type: "text", text: "line1\nline2" }],
		timestamp,
	} as unknown as AgentMessage;
	return [assistant, toolResult];
}

function makeHarness(showTokenUsage: boolean): { ctx: InteractiveModeContext; helpers: UiHelpers } {
	const ctx = {
		chatContainer: new Container(),
		transcriptMessageComponents: new WeakMap(),
		pendingTools: new Map(),
		ui: { requestRender: vi.fn() },
		statusLine: { invalidate: vi.fn() },
		updateEditorBorderColor: vi.fn(),
		settings: { get: (key: string) => (key === "display.showTokenUsage" ? showTokenUsage : false) },
		addMessageToChat: (message: AgentMessage) => helpers.addMessageToChat(message),
		session: {
			retryAttempt: 0,
			getToolByName: () => undefined,
			sessionManager: { getCwd: () => process.cwd() },
		},
		get viewSession() {
			return (this as typeof ctx).session;
		},
		toolOutputExpanded: false,
		hideThinkingBlock: false,
		clearTransientSessionUi: () => {},
	} as unknown as InteractiveModeContext;
	const helpers = new UiHelpers(ctx);
	return { ctx, helpers };
}

describe("UiHelpers.renderSessionContext token-usage row placement", () => {
	beforeAll(async () => {
		await initTheme();
	});

	it("nests the usage row inside the read group for a read-only turn", () => {
		const { ctx, helpers } = makeHarness(true);
		helpers.renderSessionContext({ messages: readTurn() } as SessionContext);

		const children = ctx.chatContainer.children;
		const group = children.find(
			(component): component is ReadToolGroupComponent => component instanceof ReadToolGroupComponent,
		);
		expect(group).toBeDefined();

		const rendered = group!.render(120).join("\n");
		expect(rendered).toContain(USAGE_LABEL);
		expect(rendered).toContain(USAGE_TS_LABEL);
		// The completion endcap lands once, after the group — never nested per
		// segment — carrying the completion's model alongside the usage row.
		const tail = children[children.length - 1]!;
		expect(tail).not.toBe(group!);
		expect(tail.render(120).join("\n")).toContain("claude-sonnet-4-5");
		expect(children.filter(component => component.render(120).join("\n").includes(USAGE_LABEL))).toHaveLength(1);
	});

	it("interleaves consecutive read-only turns with their paths in one group", () => {
		const { ctx, helpers } = makeHarness(true);
		const messages = [...readTurn(), ...readTurn("r2", "src/bar.ts", 2121, SECOND_USAGE_TS)];
		helpers.renderSessionContext({ messages } as SessionContext);

		const groups = ctx.chatContainer.children.filter(
			(component): component is ReadToolGroupComponent => component instanceof ReadToolGroupComponent,
		);
		expect(groups).toHaveLength(1);
		const lines = Bun.stripANSI(groups[0]!.render(120).join("\n")).split("\n");
		const fooIndex = lines.findIndex(line => line.includes("src/foo.ts"));
		const firstUsageIndex = lines.findIndex(line => line.includes(USAGE_TS_LABEL));
		const barIndex = lines.findIndex(line => line.includes("src/bar.ts"));
		const secondUsageIndex = lines.findIndex(line => line.includes(SECOND_USAGE_TS_LABEL));
		expect(fooIndex).toBeLessThan(firstUsageIndex);
		expect(firstUsageIndex).toBeLessThan(barIndex);
		expect(barIndex).toBeLessThan(secondUsageIndex);
	});

	it("renders no usage row when showTokenUsage is off", () => {
		const { ctx, helpers } = makeHarness(false);
		helpers.renderSessionContext({ messages: readTurn() } as SessionContext);

		const children = ctx.chatContainer.children;
		expect(children.some(c => c.render(120).join("\n").includes(USAGE_LABEL))).toBe(false);
		// Last block is the read group, not a usage row.
		expect(children[children.length - 1]).toBeInstanceOf(ReadToolGroupComponent);
	});
});

describe("ChatTranscriptBuilder token-usage row timestamp", () => {
	beforeEach(async () => {
		await Settings.init({ inMemory: true, cwd: process.cwd() });
		settings.set("display.showTokenUsage", true);
	});
	afterEach(() => {
		resetSettingsForTest();
	});

	it("renders the turn's local timestamp on the rebuilt usage row", () => {
		const builder = new ChatTranscriptBuilder({
			ui: { requestRender: () => {}, requestComponentRender: () => {} } as unknown as TUI,
			cwd: process.cwd(),
			requestRender: () => {},
		});
		const message = {
			role: "assistant",
			content: [{ type: "text", text: "done" }],
			api: "anthropic-messages",
			provider: "anthropic",
			model: "claude-sonnet-4-5",
			stopReason: "stop",
			usage: {
				input: USAGE_INPUT,
				output: 7,
				cacheRead: 0,
				cacheWrite: 0,
				totalTokens: USAGE_INPUT + 7,
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
			},
			timestamp: USAGE_TS,
		} as unknown as AgentMessage;
		builder.rebuild([{ type: "message", id: "m1", parentId: null, timestamp: new Date(0).toISOString(), message }]);
		const children = builder.container.children;
		// The usage row keeps the turn's local timestamp; the completion endcap
		// follows it once with the completion's model (elapsed is gated off here).
		const usageRow = children[children.length - 2]!;
		const usageRendered = usageRow.render(120).join("\n");
		expect(usageRendered).toContain(USAGE_TS_LABEL);
		expect(usageRendered).toContain(USAGE_LABEL);
		const endcap = children[children.length - 1]!;
		expect(endcap.render(120).join("\n")).toContain("claude-sonnet-4-5");
		expect(children.filter(component => component.render(120).join("\n").includes(USAGE_LABEL))).toHaveLength(1);
	});

	it("deep-links tool-only assistant entries to their first rendered row", () => {
		const builder = new ChatTranscriptBuilder({
			ui: new TUI(new VirtualTerminal(120, 20)),
			cwd: process.cwd(),
			requestRender: () => {},
		});
		const message: AssistantMessage = {
			role: "assistant",
			content: [{ type: "toolCall", id: "call-1", name: "bash", arguments: { command: "echo ok" } }],
			api: "anthropic-messages",
			provider: "anthropic",
			model: "claude-sonnet-4-5",
			stopReason: "toolUse",
			usage: {
				input: 1,
				output: 1,
				cacheRead: 0,
				cacheWrite: 0,
				totalTokens: 2,
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
			},
			timestamp: 1_000,
		};
		builder.rebuild([
			{ type: "message", id: "tool-entry", parentId: null, timestamp: new Date(0).toISOString(), message },
		]);

		const rendered = builder.container.render(120);
		expect(Bun.stripANSI(rendered.join("\n"))).toContain("echo ok");
		expect(builder.rowForEntry("tool-entry")).toBe(0);
	});

	it("keeps grouped read metrics nested on the reusable transcript-builder path", () => {
		const builder = new ChatTranscriptBuilder({
			ui: { requestRender: () => {}, requestComponentRender: () => {} } as unknown as TUI,
			cwd: process.cwd(),
			requestRender: () => {},
		});
		const messages = [...readTurn(), ...readTurn("r2", "src/bar.ts", 2121, SECOND_USAGE_TS)];
		builder.rebuild(
			messages.map((message, index) => ({
				type: "message",
				id: `m${index}`,
				parentId: index === 0 ? null : `m${index - 1}`,
				timestamp: new Date(0).toISOString(),
				message,
			})),
		);

		const groups = builder.container.children.filter(
			(component): component is ReadToolGroupComponent => component instanceof ReadToolGroupComponent,
		);
		expect(groups).toHaveLength(1);
		const rendered = groups[0]!.render(120).join("\n");
		expect(rendered).toContain(USAGE_TS_LABEL);
		expect(rendered).toContain(SECOND_USAGE_TS_LABEL);
		expect(
			builder.container.children.filter(component => component.render(120).join("\n").includes(USAGE_LABEL)),
		).toEqual([groups[0]!]);
	});
});

describe("completion endcap aggregation", () => {
	function builderWithDisplay(): ChatTranscriptBuilder {
		return new ChatTranscriptBuilder({
			ui: { requestRender: () => {}, requestComponentRender: () => {} } as unknown as TUI,
			cwd: process.cwd(),
			requestRender: () => {},
		});
	}

	function billedUsage(input = USAGE_INPUT) {
		return {
			input,
			output: 7,
			cacheRead: 0,
			cacheWrite: 0,
			totalTokens: input + 7,
			cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
		};
	}

	function assistantWithTool(toolCallId: string, extra: Record<string, unknown> = {}): AgentMessage {
		return {
			role: "assistant",
			content: [{ type: "toolCall", id: toolCallId, name: "bash", arguments: { command: "echo ok" } }],
			api: "anthropic-messages",
			provider: "anthropic",
			model: "claude-sonnet-4-5",
			stopReason: "toolUse",
			usage: billedUsage(),
			timestamp: USAGE_TS,
			...extra,
		} as unknown as AgentMessage;
	}

	beforeEach(async () => {
		await Settings.init({ inMemory: true, cwd: process.cwd() });
		settings.set("display.showTokenUsage", true);
		settings.set("display.showTurnTime", true);
		await initTheme();
	});
	afterEach(() => {
		resetSettingsForTest();
	});

	it("emits the endcap once after tools with model and prompt-to-yield elapsed", () => {
		const builder = builderWithDisplay();
		const userTs = USAGE_TS - 60_000;
		const completedAt = userTs + 2_500;
		const user = {
			role: "user",
			content: "run it",
			timestamp: userTs,
		} as unknown as AgentMessage;
		const assistant = assistantWithTool("b1", { completedAt });
		const toolResult = {
			role: "toolResult",
			toolCallId: "b1",
			toolName: "bash",
			content: [{ type: "text", text: "ok" }],
			timestamp: USAGE_TS,
		} as unknown as AgentMessage;
		builder.rebuild(
			[user, assistant, toolResult].map((message, index) => ({
				type: "message",
				id: `e${index}`,
				parentId: index === 0 ? null : `e${index - 1}`,
				timestamp: new Date(0).toISOString(),
				message,
			})),
		);

		const children = builder.container.children;
		const endcaps = children.filter(component =>
			Bun.stripANSI(component.render(120).join("\n")).includes("claude-sonnet-4-5"),
		);
		// Exactly one endcap for the displayed completion, carrying the
		// prompt-to-yield delta derived from completedAt (2.5s).
		expect(endcaps).toHaveLength(1);
		expect(Bun.stripANSI(endcaps[0]!.render(120).join("\n"))).toContain("2.5s");
		// It lands after the tool block, not per assistant segment.
		const toolIndex = children.findIndex(component => component instanceof ToolExecutionComponent);
		expect(toolIndex).toBeGreaterThanOrEqual(0);
		expect(children.indexOf(endcaps[0]!)).toBeGreaterThan(toolIndex);
	});

	it("omits unavailable values and emits nothing when all are missing", () => {
		expect(formatCompletionEndcap({})).toBeUndefined();
		expect(formatCompletionEndcap({ mode: undefined, model: undefined, elapsedMs: undefined })).toBeUndefined();

		const builder = builderWithDisplay();
		const unbilled = {
			role: "assistant",
			content: [{ type: "text", text: "done" }],
			api: "anthropic-messages",
			provider: "anthropic",
			stopReason: "stop",
			usage: {
				input: 0,
				output: 0,
				cacheRead: 0,
				cacheWrite: 0,
				totalTokens: 0,
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
			},
			timestamp: USAGE_TS,
		} as unknown as AgentMessage;
		builder.rebuild([
			{ type: "message", id: "u1", parentId: null, timestamp: new Date(0).toISOString(), message: unbilled },
		]);
		// No usage row and no endcap: a single assistant block, nothing appended.
		expect(builder.container.children).toHaveLength(1);
	});

	it("does not append a second completion when a late result arrives", () => {
		const builder = builderWithDisplay();
		const assistant = assistantWithTool("late-1", { completedAt: USAGE_TS + 1_000 });
		const toolResult = {
			role: "toolResult",
			toolCallId: "late-1",
			toolName: "bash",
			content: [{ type: "text", text: "ok" }],
			timestamp: USAGE_TS,
		} as unknown as AgentMessage;
		const entries = [assistant, toolResult].map((message, index) => ({
			type: "message" as const,
			id: `l${index}`,
			parentId: index === 0 ? null : `l${index - 1}`,
			timestamp: new Date(0).toISOString(),
			message,
		}));
		builder.rebuild(entries);
		const countEndcaps = () =>
			builder.container.children.filter(component =>
				Bun.stripANSI(component.render(120).join("\n")).includes("claude-sonnet-4-5"),
			).length;
		expect(countEndcaps()).toBe(1);

		// A late duplicate result for the already-settled call routes nowhere
		// and must not produce a second completion row.
		builder.append([
			{
				type: "message" as const,
				id: "l2",
				parentId: "l1",
				timestamp: new Date(0).toISOString(),
				message: toolResult,
			},
		]);
		expect(countEndcaps()).toBe(1);
	});
});

describe("parked viewer background tasks", () => {
	beforeEach(async () => {
		await Settings.init({ inMemory: true, cwd: process.cwd() });
		settings.set("display.showTokenUsage", true);
		await initTheme();
	});
	afterEach(() => {
		resetSettingsForTest();
	});

	it("parks a detached task snapshot as finalized history without blocking the usage flush", () => {
		const builder = new ChatTranscriptBuilder({
			ui: { requestRender: () => {}, requestComponentRender: () => {} } as unknown as TUI,
			cwd: process.cwd(),
			requestRender: () => {},
		});
		const assistant = {
			role: "assistant",
			content: [{ type: "toolCall", id: "task-1", name: "task", arguments: { task: "survey the repo" } }],
			api: "anthropic-messages",
			provider: "anthropic",
			model: "claude-sonnet-4-5",
			stopReason: "toolUse",
			usage: {
				input: USAGE_INPUT,
				output: 7,
				cacheRead: 0,
				cacheWrite: 0,
				totalTokens: USAGE_INPUT + 7,
				cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
			},
			timestamp: USAGE_TS,
		} as unknown as AgentMessage;
		const runningSnapshot = {
			role: "toolResult",
			toolCallId: "task-1",
			toolName: "task",
			content: [{ type: "text", text: "Background job started" }],
			details: { async: { state: "running", jobId: "job-1" } },
			timestamp: USAGE_TS,
		} as unknown as AgentMessage;
		builder.rebuild(
			[assistant, runningSnapshot].map((message, index) => ({
				type: "message",
				id: `p${index}`,
				parentId: index === 0 ? null : `p${index - 1}`,
				timestamp: new Date(0).toISOString(),
				message,
			})),
		);

		// The parked card finalizes (never pins history retirement) …
		const parked = builder.container.children.find(
			(component): component is ToolExecutionComponent => component instanceof ToolExecutionComponent,
		);
		expect(parked).toBeDefined();
		expect(parked!.isTranscriptBlockFinalized()).toBe(true);
		// … and the trailing usage flush is not held back by the retained card.
		expect(builder.container.children.some(component => component.render(120).join("\n").includes(USAGE_LABEL))).toBe(
			true,
		);
	});
});
