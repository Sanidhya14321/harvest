/**
 * Regression coverage for parked-transcript late results (review defect B).
 *
 * Failure mode 1: ChatTranscriptBuilder.rebuild() sealed + deleted unfinished
 * non-background tool calls, so a toolResult arriving later via append() (the
 * agent/advisor viewer path) hit the no-pending early return and was silently
 * discarded — the card stayed a forever-pending spinner snapshot with no
 * result. Rebuild must retain sealed calls (bounded) and route late results to
 * them in place, without adding or moving rows.
 *
 * Failure mode 2: formatUsageRow() had no width bound, so a long metrics row
 * left an overflow remnant past the owner's fixed width.
 */
import { afterEach, beforeAll, beforeEach, describe, expect, it } from "bun:test";
import type { AgentMessage } from "@harvest/pi-agent-core";
import { resetSettingsForTest, Settings, settings } from "@harvest/pi-coding-agent/config/settings";
import { ChatTranscriptBuilder } from "@harvest/pi-coding-agent/modes/components/chat-transcript-builder";
import { ToolExecutionComponent } from "@harvest/pi-coding-agent/modes/components/tool-execution";
import { formatUsageRow } from "@harvest/pi-coding-agent/modes/components/usage-row";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { SessionMessageEntry } from "@harvest/pi-coding-agent/session/session-entries";
import { TUI } from "@harvest/pi-tui";
import { VirtualTerminal } from "../../tui/test/virtual-terminal";

const LATE_TOOL_CALL_ID = "late-1";
const LATE_RESULT_MARKER = "late-result-marker-xyz";
const UNKNOWN_RESULT_MARKER = "unknown-result-marker-xyz";
const USAGE_TS = new Date(2026, 0, 2, 3, 4, 5).getTime();

function assistantToolCall(toolCallId: string): AgentMessage {
	return {
		role: "assistant",
		content: [{ type: "toolCall", id: toolCallId, name: "bash", arguments: { command: "echo late" } }],
		api: "anthropic-messages",
		provider: "anthropic",
		model: "claude-sonnet-4-5",
		stopReason: "toolUse",
		usage: {
			input: 4242,
			output: 7,
			cacheRead: 0,
			cacheWrite: 0,
			totalTokens: 4249,
			cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
		},
		timestamp: USAGE_TS,
	} as unknown as AgentMessage;
}

function toolResult(toolCallId: string, text: string): AgentMessage {
	return {
		role: "toolResult",
		toolCallId,
		toolName: "bash",
		content: [{ type: "text", text }],
		timestamp: USAGE_TS,
	} as unknown as AgentMessage;
}

function entry(id: string, parentId: string | null, message: AgentMessage): SessionMessageEntry {
	return { type: "message", id, parentId, timestamp: new Date(0).toISOString(), message };
}

function builder(): ChatTranscriptBuilder {
	return new ChatTranscriptBuilder({
		ui: new TUI(new VirtualTerminal(120, 30)),
		cwd: process.cwd(),
		requestRender: () => {},
	});
}

function rendered(builderInstance: ChatTranscriptBuilder): string {
	return Bun.stripANSI(builderInstance.container.render(120).join("\n"));
}

function toolComponents(builderInstance: ChatTranscriptBuilder): ToolExecutionComponent[] {
	return builderInstance.container.children.filter(
		(component): component is ToolExecutionComponent => component instanceof ToolExecutionComponent,
	);
}

describe("ChatTranscriptBuilder late tool results after a parked rebuild", () => {
	beforeAll(async () => {
		await initTheme();
	});
	beforeEach(async () => {
		await Settings.init({ inMemory: true, cwd: process.cwd() });
		settings.set("display.showTokenUsage", true);
	});
	afterEach(() => {
		resetSettingsForTest();
	});

	it("routes a late toolResult to the rebuild-sealed card instead of silently dropping it, without adding or moving rows", () => {
		const transcript = builder();
		transcript.rebuild([entry("m1", null, assistantToolCall(LATE_TOOL_CALL_ID))]);

		// The dangling call is sealed as history but must still own exactly one row.
		const sealed = toolComponents(transcript);
		expect(sealed).toHaveLength(1);
		const card = sealed[0]!;
		const cardIndex = transcript.container.children.indexOf(card);
		expect(cardIndex).toBeGreaterThanOrEqual(0);
		const childCount = transcript.container.children.length;
		const versionBefore = card.getTranscriptBlockVersion();
		expect(rendered(transcript)).not.toContain(LATE_RESULT_MARKER);

		transcript.append([entry("m2", "m1", toolResult(LATE_TOOL_CALL_ID, LATE_RESULT_MARKER))]);

		// The late result updates the sealed card visibly: same instance, same
		// row, no extra components, and the result text renders.
		expect(transcript.container.children).toHaveLength(childCount);
		expect(toolComponents(transcript)).toEqual([card]);
		expect(transcript.container.children.indexOf(card)).toBe(cardIndex);
		expect(card.getTranscriptBlockVersion()).toBeGreaterThan(versionBefore);
		expect(rendered(transcript)).toContain(LATE_RESULT_MARKER);
	});

	it("ignores a toolResult for an unknown id instead of adding a row", () => {
		const transcript = builder();
		transcript.rebuild([entry("m1", null, assistantToolCall(LATE_TOOL_CALL_ID))]);
		transcript.append([entry("m2", "m1", toolResult(LATE_TOOL_CALL_ID, LATE_RESULT_MARKER))]);

		const childCount = transcript.container.children.length;
		const versionAfterLate = toolComponents(transcript)[0]!.getTranscriptBlockVersion();

		transcript.append([entry("m3", "m2", toolResult("no-such-call", UNKNOWN_RESULT_MARKER))]);

		expect(transcript.container.children).toHaveLength(childCount);
		expect(toolComponents(transcript)).toHaveLength(1);
		expect(toolComponents(transcript)[0]!.getTranscriptBlockVersion()).toBe(versionAfterLate);
		expect(rendered(transcript)).not.toContain(UNKNOWN_RESULT_MARKER);
	});
});

describe("formatUsageRow width clipping", () => {
	beforeAll(async () => {
		await initTheme();
	});
	beforeEach(async () => {
		await Settings.init({ inMemory: true, cwd: process.cwd() });
	});
	afterEach(() => {
		resetSettingsForTest();
	});

	it("clips the usage row to the requested width instead of leaving an overflow remnant", () => {
		const usage = {
			input: 123456789,
			output: 98765432,
			cacheRead: 13579,
			cacheWrite: 24680,
			totalTokens: 222222222,
			cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
		};
		const width = 40;
		const full = formatUsageRow(usage, 12345, 1200, USAGE_TS, 15000);
		// Guard against a vacuous assertion: the unclipped row must actually overflow.
		expect(Bun.stringWidth(Bun.stripANSI(full))).toBeGreaterThan(width);

		const clipped = formatUsageRow(usage, 12345, 1200, USAGE_TS, 15000, width);
		expect(Bun.stringWidth(Bun.stripANSI(clipped))).toBeLessThanOrEqual(width);
		// Omitting the width keeps the previous full-line contract.
		expect(formatUsageRow(usage, 12345, 1200, USAGE_TS, 15000, undefined)).toBe(full);
	});
});
