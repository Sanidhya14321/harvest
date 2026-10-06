import { beforeEach, describe, expect, it } from "bun:test";
import { TranscriptContainer } from "@harvest/pi-coding-agent/modes/components/transcript-container";
import { Composer } from "@harvest/pi-coding-agent/modes/composer";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { Text } from "@harvest/pi-tui";
import { VirtualTerminal } from "../../../tui/test/virtual-terminal";

/**
 * Reading-anchor contract: scrolling back pins the viewed text by block
 * identity, so rows appended below (live streaming) never move it.
 * Returning the offset to 0 releases the anchor and follows the tail.
 * A bottom-relative offset alone would drift the viewed text with every
 * appended row.
 */

const COLUMNS = 80;
const ROWS = 24;

function fill(transcript: TranscriptContainer, lines: number): void {
	for (let index = 1; index <= lines; index++) {
		transcript.addChild(new Text(`anchor-line ${String(index).padStart(2, "0")}`));
	}
}

function startWorkspace(lines: number): {
	composer: Composer;
	transcript: TranscriptContainer;
	frame: () => string[];
} {
	const terminal = new VirtualTerminal(COLUMNS, ROWS);
	const composer = new Composer({ preferences: { fullscreen: true, quiet: true }, terminal });
	const transcript = new TranscriptContainer();
	fill(transcript, lines);
	composer.setRuntimeChildren([transcript, composer.editor]);
	composer.start();
	const frame = () => composer.renderFrame({ columns: COLUMNS, rows: ROWS }).viewport.map(row => Bun.stripANSI(row));
	return { composer, transcript, frame };
}

beforeEach(async () => {
	await initTheme();
});

describe("workspace reading anchor", () => {
	it("locates the same window as the legacy tail slice", () => {
		const transcript = new TranscriptContainer();
		fill(transcript, 30);
		const available = 10;
		const offset = 5;
		const found = transcript.anchorForOffset(COLUMNS, available, offset);
		expect(found).toBeDefined();
		transcript.pinReadingAnchor(found!.block, found!.row);
		const anchored = transcript.renderAnchoredViewport(COLUMNS, available);
		const legacy = transcript.renderTail(COLUMNS, available + offset).slice(0, available);
		expect([...anchored]).toEqual([...legacy]);
		expect(anchored.length).toBeGreaterThan(0);
	});

	it("keeps the pinned window stable while streamed rows append below", () => {
		const transcript = new TranscriptContainer();
		fill(transcript, 30);
		const available = 10;
		const offset = 5;
		const found = transcript.anchorForOffset(COLUMNS, available, offset);
		transcript.pinReadingAnchor(found!.block, found!.row);
		const before = [...transcript.renderAnchoredViewport(COLUMNS, available)];
		fill(transcript, 5);
		const after = [...transcript.renderAnchoredViewport(COLUMNS, available)];
		expect(after).toEqual(before);
		expect(transcript.isFollowingTail()).toBe(false);
	});

	it("holds the scrolled-back composer frame stable across appended lines", () => {
		const { composer, transcript, frame } = startWorkspace(30);
		try {
			composer.scrollWorkspace(5);
			const back = frame();
			transcript.addChild(new Text("anchor-line 31"));
			transcript.addChild(new Text("anchor-line 32"));
			const streamed = frame();
			expect(streamed).toEqual(back);
			expect(streamed.join("\n")).not.toContain("anchor-line 32");
		} finally {
			composer.stop();
		}
	});

	it("returns to the newest output when the offset reaches the tail", () => {
		const { composer, transcript, frame } = startWorkspace(30);
		try {
			composer.scrollWorkspace(5);
			transcript.addChild(new Text("anchor-line 31"));
			composer.scrollWorkspace(-100);
			expect(composer.workspaceScrollOffset).toBe(0);
			expect(frame().join("\n")).toContain("anchor-line 31");
			expect(transcript.isFollowingTail()).toBe(true);
		} finally {
			composer.stop();
		}
	});

	it("recomputes the pinned window at a new width instead of drifting", () => {
		const transcript = new TranscriptContainer();
		fill(transcript, 30);
		const found = transcript.anchorForOffset(COLUMNS, 10, 5);
		transcript.pinReadingAnchor(found!.block, found!.row);
		const narrow = transcript.renderAnchoredViewport(40, 10);
		// Same pinned block still opens the window after rewrap.
		expect(narrow[0]).toContain("anchor-line");
		expect(transcript.isFollowingTail()).toBe(false);
	});
});
