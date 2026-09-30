import { beforeEach, describe, expect, it } from "bun:test";
import { TranscriptContainer } from "@harvest/pi-coding-agent/modes/components/transcript-container";
import { Composer } from "@harvest/pi-coding-agent/modes/composer";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { Text } from "@harvest/pi-tui";
import { VirtualTerminal } from "../../../tui/test/virtual-terminal";

/**
 * Scroll-direction contract: the transcript's live tail sits at offset 0.
 * Wheel-up / page-up look back in history (older rows, offset grows);
 * wheel-down / page-down return toward the live tail (offset shrinks).
 * A prior wiring passed raw wheel/key signs straight through, so scrolling
 * down moved the viewport up.
 */

const COLUMNS = 80;
const ROWS = 24;

function startWorkspace(lines: number): { composer: Composer; frame: () => string[] } {
	const terminal = new VirtualTerminal(COLUMNS, ROWS);
	const composer = new Composer({ preferences: { fullscreen: true, quiet: true }, terminal });
	const transcript = new TranscriptContainer();
	for (let index = 1; index <= lines; index++) {
		transcript.addChild(new Text(`line ${String(index).padStart(2, "0")}`));
	}
	composer.setRuntimeChildren([transcript, composer.editor]);
	composer.start();
	const frame = () => composer.renderFrame({ columns: COLUMNS, rows: ROWS }).viewport.map(row => Bun.stripANSI(row));
	return { composer, frame };
}

beforeEach(async () => {
	await initTheme();
});

describe("workspace scroll direction", () => {
	it("sits on the live tail at rest and looks back on positive deltas", () => {
		const { composer, frame } = startWorkspace(30);
		try {
			expect(frame().join("\n")).toContain("line 30");
			expect(frame().join("\n")).not.toContain("line 01");
			composer.scrollWorkspace(100);
			const back = frame().join("\n");
			expect(back).toContain("line 01");
			expect(back).not.toContain("line 30");
			composer.scrollWorkspace(-100);
			expect(frame().join("\n")).toContain("line 30");
		} finally {
			composer.stop();
		}
	});

	it("clamps below the live tail instead of overshooting", () => {
		const { composer, frame } = startWorkspace(30);
		try {
			composer.scrollWorkspace(10);
			composer.scrollWorkspace(-100);
			expect(composer.workspaceScrollOffset).toBe(0);
			expect(frame().join("\n")).toContain("line 30");
		} finally {
			composer.stop();
		}
	});

	it("maps wheel-up to history and wheel-down to live", () => {
		const { composer } = startWorkspace(30);
		try {
			composer.scrollWorkspaceWheel(-1);
			expect(composer.workspaceScrollOffset).toBeGreaterThan(0);
			const back = composer.workspaceScrollOffset;
			composer.scrollWorkspaceWheel(1);
			expect(composer.workspaceScrollOffset).toBe(back - 3);
			composer.scrollWorkspaceWheel(1, 100);
			expect(composer.workspaceScrollOffset).toBe(0);
		} finally {
			composer.stop();
		}
	});

	it("maps page-up to history and page-down to live", () => {
		const { composer } = startWorkspace(30);
		try {
			composer.scrollWorkspacePage("up", ROWS);
			expect(composer.workspaceScrollOffset).toBe(ROWS - 4);
			composer.scrollWorkspacePage("down", ROWS);
			expect(composer.workspaceScrollOffset).toBe(0);
			composer.scrollWorkspacePage("down", ROWS);
			expect(composer.workspaceScrollOffset).toBe(0);
		} finally {
			composer.stop();
		}
	});
});
