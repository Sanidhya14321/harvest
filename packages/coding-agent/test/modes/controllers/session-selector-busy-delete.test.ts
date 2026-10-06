import { beforeAll, describe, expect, it, vi } from "bun:test";
import {
	isSessionBusyForDelete,
	SessionSelectorComponent,
} from "@harvest/pi-coding-agent/modes/components/session-selector";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { SessionInfo } from "@harvest/pi-coding-agent/session/session-listing";

beforeAll(() => {
	initTheme();
});

function createSession(id: string, title: string): SessionInfo {
	return {
		path: `/tmp/${id}.jsonl`,
		id,
		cwd: "/tmp",
		title,
		created: new Date("2024-01-01T00:00:00Z"),
		modified: new Date("2024-01-02T00:00:00Z"),
		messageCount: 1,
		size: 0,
		firstMessage: `${title} first message`,
		allMessagesText: `${title} first message`,
	};
}

const idle = () => false;
function busyState(overrides: Partial<Record<"isStreaming" | "isBashRunning" | "isEvalRunning", boolean>> = {}) {
	return {
		isStreaming: overrides.isStreaming ?? false,
		isBashRunning: overrides.isBashRunning ?? false,
		isEvalRunning: overrides.isEvalRunning ?? false,
		hasPendingAsyncWork: () => false,
	};
}

describe("isSessionBusyForDelete", () => {
	it("refuses every live-run signal from the selector-controller predicate", () => {
		// Failure mode: the picker deletes a target the /delete path would
		// refuse, orphaning its in-flight work.
		expect(isSessionBusyForDelete(busyState())).toBe(false);
		expect(isSessionBusyForDelete(busyState({ isStreaming: true }))).toBe(true);
		expect(isSessionBusyForDelete(busyState({ isBashRunning: true }))).toBe(true);
		expect(isSessionBusyForDelete(busyState({ isEvalRunning: true }))).toBe(true);
		expect(isSessionBusyForDelete({ ...busyState(), hasPendingAsyncWork: () => true })).toBe(true);
	});
});

describe("SessionSelectorComponent busy-delete guard", () => {
	function createSelector(options: {
		onDelete: (session: SessionInfo) => Promise<boolean>;
		isSessionBusy?: (session: SessionInfo) => boolean;
		onStopAndDelete?: (session: SessionInfo) => Promise<boolean>;
	}): SessionSelectorComponent {
		return new SessionSelectorComponent(
			[createSession("session-a", "Alpha"), createSession("session-b", "Beta")],
			() => {},
			() => {},
			() => {},
			options,
		);
	}

	function renderText(selector: SessionSelectorComponent): string {
		return Bun.stripANSI(selector.render(120).join("\n"));
	}

	it("refuses a busy target without invoking the plain delete", async () => {
		// Failure mode: Delete on a streaming session removes its file while
		// the run keeps appending to it.
		const onDelete = vi.fn(async () => true);
		const selector = createSelector({
			onDelete,
			isSessionBusy: session => session.id === "session-a",
		});

		selector.handleInput("\x1b[3~");
		expect(renderText(selector)).toContain("Session is busy");
		expect(renderText(selector)).not.toContain("Delete session?");
		expect(onDelete).not.toHaveBeenCalled();
	});

	it("deletes through stop-and-delete only after explicit confirmation", async () => {
		const onDelete = vi.fn(async () => true);
		const onStopAndDelete = vi.fn(async () => true);
		const selector = createSelector({
			onDelete,
			isSessionBusy: session => session.id === "session-a",
			onStopAndDelete,
		});

		selector.handleInput("\x1b[3~");
		expect(renderText(selector)).toContain("Stop & Delete");

		selector.handleInput("\n");
		await Bun.sleep(0);

		expect(onStopAndDelete).toHaveBeenCalledTimes(1);
		expect(onDelete).not.toHaveBeenCalled();
		expect(renderText(selector)).not.toContain("Alpha");
		expect(renderText(selector)).toContain("Beta");
	});

	it("explains how to stop first when no stop-and-delete is wired", async () => {
		const onDelete = vi.fn(async () => true);
		const selector = createSelector({
			onDelete,
			isSessionBusy: () => true,
		});

		selector.handleInput("\x1b[3~");
		selector.handleInput("\n");
		await Bun.sleep(0);

		expect(onDelete).not.toHaveBeenCalled();
		const rendered = renderText(selector);
		expect(rendered).toContain("stop the run first");
		expect(rendered).toContain("Alpha");
	});

	it("keeps the plain delete flow for idle targets", async () => {
		const onDelete = vi.fn(async () => true);
		const selector = createSelector({ onDelete, isSessionBusy: idle });

		selector.handleInput("\x1b[3~");
		expect(renderText(selector)).toContain("Delete session?");
		selector.handleInput("\n");
		await Bun.sleep(0);

		expect(onDelete).toHaveBeenCalledTimes(1);
		expect(renderText(selector)).not.toContain("Alpha");
	});
});
