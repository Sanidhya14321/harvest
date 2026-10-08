import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import type { AgentMessage } from "@harvest/pi-agent-core";
import type { AssistantMessage, Usage } from "@harvest/pi-ai";
import { TUI, visibleWidth } from "@harvest/pi-tui";
import { resetSettingsForTest, Settings } from "../src/config/settings";
import { createAdvisorMessageCard } from "../src/modes/components/advisor-message";
import { BashExecutionComponent } from "../src/modes/components/bash-execution";
import { AssistantMessageComponent } from "../src/modes/components/assistant-message";
import { ChatTranscriptBuilder } from "../src/modes/components/chat-transcript-builder";
import { CollabPromptMessageComponent } from "../src/modes/components/collab-prompt-message";
import { CompactionSummaryMessageComponent } from "../src/modes/components/compaction-summary-message";
import { EvalExecutionComponent } from "../src/modes/components/eval-execution";
import { appendOutlineEntries } from "../src/modes/components/transcript-outline";
import { UserMessageComponent } from "../src/modes/components/user-message";
import { EventController } from "../src/modes/controllers/event-controller";
import { UiHelpers } from "../src/modes/utils/ui-helpers";
import { initTheme, setThemeInstance, type Theme, theme } from "../src/modes/theme/theme";
import type { CollabPromptDetails } from "../src/collab/protocol";
import type { CompactionSummaryMessage, CustomMessage } from "../src/session/messages";
import type { SessionMessageEntry } from "../src/session/session-entries";
import { renderInlineActivity, renderOutputBlock, renderOutputPanelLines } from "../src/tui/output-block";
import { VirtualTerminal } from "../../tui/test/virtual-terminal";
import { createInteractiveModeContext } from "./helpers/interactive-mode-context";

let previousTheme: Theme;
beforeEach(async () => {
	previousTheme = theme;
	resetSettingsForTest();
	await Settings.init({
		inMemory: true,
		overrides: { "display.showTokenUsage": true, "display.showTurnTime": true, "display.smoothStreaming": false },
	});
	await initTheme(false, "ascii");
});
afterEach(() => {
	setThemeInstance(previousTheme);
	resetSettingsForTest();
});

const usage: Usage = {
	input: 1,
	output: 1,
	cacheRead: 0,
	cacheWrite: 0,
	totalTokens: 2,
	cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
};
function completion(marker: string, model: string, timestamp: number): AssistantMessage {
	return {
		role: "assistant",
		content: [{ type: "text", text: marker }],
		api: "anthropic-messages",
		provider: "anthropic",
		model,
		stopReason: "stop",
		usage,
		timestamp,
		completedAt: timestamp + 500,
	};
}
function entry(id: string, message: AgentMessage): SessionMessageEntry {
	return { type: "message", id, parentId: null, timestamp: new Date(0).toISOString(), message };
}
function createBuilder(): ChatTranscriptBuilder {
	return new ChatTranscriptBuilder({
		ui: new TUI(new VirtualTerminal(120, 30)),
		cwd: process.cwd(),
		requestRender: () => {},
	});
}
function markers(lines: readonly string[]): string[] {
	return lines
		.map(line => Bun.stripANSI(line))
		.flatMap(line => ["STEP_ONE", "STEP_TWO", "MODEL_ONE", "MODEL_TWO"].filter(marker => line.includes(marker)));
}
function expectBounded(lines: readonly string[], width: number): void {
	for (const line of lines) expect(visibleWidth(line)).toBeLessThanOrEqual(width);
}

describe("completion association across transcript views", () => {
	it("keeps both completion labels at the turn tail in live, rebuilt and per-entry outline views", async () => {
		const one = completion("STEP_ONE", "MODEL_ONE", 1_000);
		const two = completion("STEP_TWO", "MODEL_TWO", 2_000);
		const entries = [entry("one", one), entry("two", two)];
		const ctx = createInteractiveModeContext();
		const controller = new EventController(ctx);
		await controller.handleEvent({ type: "agent_start" });
		for (const message of [one, two]) {
			await controller.handleEvent({ type: "message_start", message });
			await controller.handleEvent({ type: "message_end", message });
		}
		await controller.handleEvent({ type: "agent_end", messages: [one, two] });
		const expected = ["STEP_ONE", "STEP_TWO", "MODEL_ONE", "MODEL_TWO"];
		expect(markers(ctx.chatContainer.render(120))).toEqual(expected);
		const rebuilt = createBuilder();
		rebuilt.rebuild(entries);
		expect(markers(rebuilt.container.render(120))).toEqual(expected);
		const restored = createInteractiveModeContext();
		const helpers = new UiHelpers(restored);
		restored.addMessageToChat = (message, options) => helpers.addMessageToChat(message, options);
		helpers.renderSessionContext({ messages: [one, two], models: {}, injectedTtsrRules: [], mode: "none" });
		expect(markers(restored.chatContainer.render(120))).toEqual(expected);
		const outline = createBuilder();
		const targets = appendOutlineEntries(outline, entries);
		expect(markers(outline.container.render(120))).toEqual(expected);
		expect(targets.at(-1)?.end).toBe(outline.container.children.length);
		rebuilt.dispose();
		outline.dispose();
		ctx.chatContainer.dispose();
		restored.chatContainer.dispose();
	});

	it("preserves the trailing completion queue when a live transcript is rebuilt before turn end", async () => {
		const one = completion("STEP_ONE", "MODEL_ONE", 1_000);
		const two = completion("STEP_TWO", "MODEL_TWO", 2_000);
		let streaming = true;
		const ctx = createInteractiveModeContext({
			session: {
				get isStreaming() {
					return streaming;
				},
			},
		});
		const controller = new EventController(ctx);
		ctx.eventController = controller;
		const helpers = new UiHelpers(ctx);
		ctx.addMessageToChat = (message, options) => helpers.addMessageToChat(message, options);
		await controller.handleEvent({ type: "agent_start" });
		await controller.handleEvent({ type: "message_start", message: one });
		await controller.handleEvent({ type: "message_end", message: one });
		ctx.chatContainer.clear();
		controller.resetTranscriptAnchors();
		helpers.renderSessionContext({ messages: [one], models: {}, injectedTtsrRules: [], mode: "none" });
		expect(markers(ctx.chatContainer.render(120))).toEqual(["STEP_ONE"]);
		await controller.handleEvent({ type: "message_start", message: two });
		await controller.handleEvent({ type: "message_end", message: two });
		streaming = false;
		await controller.handleEvent({ type: "agent_end", messages: [one, two] });
		expect(markers(ctx.chatContainer.render(120))).toEqual(["STEP_ONE", "STEP_TWO", "MODEL_ONE", "MODEL_TWO"]);
		ctx.chatContainer.dispose();
	});

	it("does not publish a completion between poll batches and flushes exactly once on an idle transition", () => {
		const builder = createBuilder();
		builder.rebuild([entry("one", completion("STEP_ONE", "MODEL_ONE", 1_000))], { turnComplete: false });
		builder.append([entry("two", completion("STEP_TWO", "MODEL_TWO", 2_000))], { turnComplete: false });
		expect(markers(builder.container.render(120))).toEqual(["STEP_ONE", "STEP_TWO"]);
		expect(builder.completeTurn()).toBe(true);
		expect(builder.completeTurn()).toBe(false);
		expect(markers(builder.container.render(120))).toEqual(["STEP_ONE", "STEP_TWO", "MODEL_ONE", "MODEL_TWO"]);
		builder.dispose();
	});
});

describe("bounded transcript surfaces", () => {
	it("collapses chrome before a tiny width can overflow, with ASCII-owned activity symbols", () => {
		for (const width of [0, 1, 2, 3, 4, 20]) {
			expectBounded(renderInlineActivity(theme, "Read", "source.ts", "success", width), width);
			expectBounded(
				renderOutputBlock(
					{ header: "Read source.ts", state: "success", width, sections: [{ lines: ["line ending"] }] },
					theme,
				),
				width,
			);
		}
		const activity = Bun.stripANSI(renderInlineActivity(theme, "Read", "source.ts", "success", 40).join("\n"));
		expect(activity).toContain("source.ts");
		expect(activity).not.toMatch(/[›·•]/);
	});

	it("preserves SIXEL protocol rows while applying the shared panel to surrounding text", () => {
		const payload = "\x1bPqabc\x1b\\";
		const source = Object.freeze(["output before image", payload, "output after image"]);
		const lines = renderOutputPanelLines(source, 40, theme, { background: "panelBg" });
		expect(lines[1]).toBe(payload);
		expect(Bun.stripANSI(lines[0]!)).toContain("| output before image");
		expect(Bun.stripANSI(lines[2]!)).toContain("| output after image");
	});

	it("keeps compaction amounts within narrow allocation and the complete summary reachable", () => {
		const message: CompactionSummaryMessage = {
			role: "compactionSummary",
			method: "remote",
			tokensBefore: 500_000,
			tokensAfter: 25_000,
			summary: "SUMMARY_START retain safeguards SUMMARY_END",
			timestamp: 1,
		};
		const component = new CompactionSummaryMessageComponent(message);
		expectBounded(component.render(20), 20);
		component.setExpanded(true);
		const expanded = component.render(20);
		expectBounded(expanded, 20);
		expect(Bun.stripANSI(expanded.join("\n"))).toContain("SUMMARY_START");
		expect(Bun.stripANSI(expanded.join("\n"))).toContain("SUMMARY_END");
	});

	it("does not erase the first note word when an advisor attribution is longer than the viewport", () => {
		const card = createAdvisorMessageCard(
			{
				notes: [
					{
						note: "CRITICAL safeguard remains required. LASTWORD",
						severity: "blocker",
						advisor: "AdvisorName".repeat(8),
					},
				],
			},
			() => false,
			theme,
		);
		const rows = card.render(40);
		expectBounded(rows, 40);
		const text = Bun.stripANSI(rows.join("\n"));
		expect(text).toContain("blocker");
		expect(text).toContain("CRITICAL");
		expect(text).toContain("LASTWORD");
	});

	it("uses the same user rail and attachment chips for guests without claiming a local shell prompt zone", () => {
		const content = "MESSAGE_START [Image #1, 20x20] MESSAGE_END";
		const local = new UserMessageComponent(content);
		const message: CustomMessage<CollabPromptDetails> = {
			role: "custom",
			customType: "collab-prompt",
			content,
			display: true,
			timestamp: 1,
			details: { from: "Guest" },
		};
		const guest = new CollabPromptMessageComponent(message);
		const localRows = local.render(40);
		const guestRows = guest.render(40);
		expectBounded(localRows, 40);
		expectBounded(guestRows, 40);
		expect(Bun.stripANSI(guestRows.join("\n"))).toContain("Guest");
		expect(Bun.stripANSI(guestRows.join("\n"))).not.toContain("20x20");
		for (const rows of [localRows, guestRows]) {
			expect(rows.some(row => Bun.stripANSI(row).startsWith("| "))).toBe(true);
			expect(Bun.stripANSI(rows.join("\n"))).toContain("MESSAGE_START");
			expect(Bun.stripANSI(rows.join("\n"))).toContain("MESSAGE_END");
		}
		expect(localRows.join("\n")).toContain("\x1b]133;A\x07");
		expect(localRows.join("\n")).toContain("\x1b]133;D;0\x07");
		expect(guestRows.join("\n")).not.toContain("\x1b]133;");
		expect(local.render(40)).toBe(localRows);
	});

	it("keeps the hidden reasoning pulse readable without Unicode-owned chrome in ASCII mode", () => {
		const message = completion("unused", "model", 1_000);
		message.content = [{ type: "thinking", thinking: "private reasoning" }];
		const assistant = new AssistantMessageComponent(undefined, true);
		assistant.updateContent(message);
		const rows = assistant.render(40);
		const text = Bun.stripANSI(rows.join("\n"));
		expect(text).toContain("Thinking");
		expect(text).not.toContain("private reasoning");
		expect(text).not.toMatch(/[✻✼❉❊✺✹✸✶·]/);
		expectBounded(rows, 40);
		assistant.dispose();
	});

	it("retains output, failure and unknown outcome meaning after manual execution adopts compact panels", () => {
		const ui = new TUI(new VirtualTerminal(40, 20));
		const bash = new BashExecutionComponent("echo output", ui);
		const evalCell = new EvalExecutionComponent("print('output')", ui);
		bash.setComplete(3, false, { output: "BASH_OUTPUT_END" });
		evalCell.setComplete(undefined, false, { output: "EVAL_OUTPUT_END" });
		const bashRows = bash.render(40);
		const evalRows = evalCell.render(40);
		expectBounded(bashRows, 40);
		expectBounded(evalRows, 40);
		expect(Bun.stripANSI(bashRows.join("\n"))).toContain("BASH_OUTPUT_END");
		expect(Bun.stripANSI(bashRows.join("\n"))).toContain("exit 3");
		expect(Bun.stripANSI(evalRows.join("\n"))).toContain("EVAL_OUTPUT_END");
		expect(Bun.stripANSI(evalRows.join("\n"))).toContain("exit status unavailable");
		expect(bash.isTranscriptBlockFinalized()).toBe(true);
		expect(evalCell.isTranscriptBlockFinalized()).toBe(true);
		expect(bash.getOutput()).toBe("BASH_OUTPUT_END");
		expect(evalCell.getOutput()).toBe("EVAL_OUTPUT_END");
	});
});
