import { describe, expect, it } from "bun:test";
import type { Component } from "@harvest/pi-tui";
import { TranscriptContainer } from "@harvest/pi-coding-agent/modes/components/transcript-container";

/** Width-independent block; `finalized=false` keeps the live tail active. */
class Rows implements Component {
	lines: string[];
	private finalized: boolean;
	constructor(lines: string[], finalized = true) {
		this.lines = [...lines];
		this.finalized = finalized;
	}
	isTranscriptBlockFinalized(): boolean {
		return this.finalized;
	}
	render(_width: number): readonly string[] {
		return [...this.lines];
	}
}

/** Retire the settled prefix through the real pipeline, as viewport pressure would. */
function commitHistory(container: TranscriptContainer): void {
	for (let i = 0; i < 10_000; i++) {
		const batch = container.peekFinalizedBatch(100, 0);
		if (!batch) return;
		container.acknowledgeFinalizedBatch(batch.id);
	}
	throw new Error("history commit did not converge");
}

function historyWithTail(historyBlocks: number): { container: TranscriptContainer; tail: Rows } {
	const container = new TranscriptContainer();
	for (let i = 0; i < historyBlocks; i++) {
		container.addChild(new Rows([`sealed block ${i} line 0`, `sealed block ${i} line 1`]));
	}
	const tail = new Rows(["live tail 0", "live tail 1"], false);
	container.addChild(tail);
	commitHistory(container);
	return { container, tail };
}

describe("transcript reading anchor", () => {
	it("keeps the pinned block stable while the tail streams and rewraps", () => {
		const container = new TranscriptContainer();
		container.addChild(new Rows(["a0", "a1", "a2", "a3", "a4"]));
		container.addChild(new Rows(["b0", "b1", "b2"]));
		const tail = new Rows(["t0", "t1"], false);
		container.addChild(tail);

		container.pinReadingAnchor(0, 1);
		const before = container.renderAnchoredViewport(80, 4);
		expect(before).toEqual(["a1", "a2", "a3", "a4"]);

		for (let i = 0; i < 10; i++) tail.lines.push(`t${i + 2}`);
		expect(container.renderAnchoredViewport(80, 4)).toEqual(before);
		// Rewrap/sidebar resize recomputes the same anchored window.
		expect(container.renderAnchoredViewport(40, 4)).toEqual(before);
		expect(container.isFollowingTail()).toBe(false);
	});

	it("reports tail arrival so the owner can reset follow mode", () => {
		const container = new TranscriptContainer();
		container.addChild(new Rows(["a0", "a1"]));
		container.addChild(new Rows(["t0", "t1"], false));

		container.pinReadingAnchor(0, 0);
		expect(container.isAnchorAtTail(80, 4)).toBe(false);

		container.pinReadingAnchor(1, 0);
		expect(container.isAnchorAtTail(80, 10)).toBe(true);
		container.followLiveTail();
		expect(container.isFollowingTail()).toBe(true);
		expect(container.renderAnchoredViewport(80, 10)).toEqual([]);
	});

	it("round-trips the anchor through the per-session snapshot", () => {
		const container = new TranscriptContainer();
		container.addChild(new Rows(["a0", "a1", "a2", "a3"]));
		container.addChild(new Rows(["t0"], false));

		container.pinReadingAnchor(0, 2);
		const snapshot = container.readingAnchorSnapshot();
		expect(snapshot).toEqual({ block: 0, row: 2 });

		container.followLiveTail();
		expect(container.isFollowingTail()).toBe(true);
		container.restoreReadingAnchor(snapshot);
		expect(container.isFollowingTail()).toBe(false);
		expect(container.renderAnchoredViewport(80, 3)).toEqual(["a2", "a3", ""]);

		container.restoreReadingAnchor(undefined);
		expect(container.isFollowingTail()).toBe(true);
	});

	it("keeps windowed tail work bounded after sealed history commits", () => {
		const small = historyWithTail(20);
		const large = historyWithTail(200);
		const frame = { tick: 1, now: 0 };

		const smallRows = small.container.renderViewport(100, 40, frame);
		const largeRows = large.container.renderViewport(100, 40, frame);
		expect(smallRows.length).toBeLessThanOrEqual(40);
		expect(largeRows.length).toBeLessThanOrEqual(40);
		const bytes = (rows: readonly string[]): number => rows.reduce((sum, row) => sum + row.length, 0);
		// 10x sealed history must not change the visible-window work.
		expect(bytes(largeRows)).toBe(bytes(smallRows));

		// A streaming tail tick still composes the new rows inside the window.
		large.tail.lines.push("live tail 2", "live tail 3");
		const ticked = large.container.renderViewport(100, 40, { tick: 2, now: 16 });
		expect(ticked.length).toBeLessThanOrEqual(40);
		expect(ticked.join("\n")).toContain("live tail 3");
	});
});
