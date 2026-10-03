import { describe, expect, test } from "bun:test";
import {
	MAX_RPC_INPUT_LINE_BYTES,
	readRpcInputFrames,
	readRpcInputFramesWithBackpressure,
} from "@harvest/pi-coding-agent/modes/rpc/rpc-input";

/**
 * Regression test for issue #5194: a non-JSON stdin line crashed the whole RPC
 * process with an uncaught parse error escaping the frame loop. A malformed
 * line must instead be reported and the reader must keep yielding later frames.
 */
describe("RPC mode malformed stdin", () => {
	test("reports a bad line and keeps reading subsequent commands", async () => {
		const input = new Blob([
			"this is not json\n",
			`${JSON.stringify({ type: "get_state", id: "probe" })}\n`,
			`${JSON.stringify({ type: "get_messages_page", id: "page-probe", limit: 1 })}\n`,
		]).stream();
		const frames: unknown[] = [];
		const parseErrors: string[] = [];

		await readRpcInputFrames(
			input,
			frame => frames.push(frame),
			message => parseErrors.push(message),
		);

		expect(parseErrors).toHaveLength(1);
		expect(parseErrors[0]).toContain("Failed to parse command");
		expect(frames).toEqual([
			{ type: "get_state", id: "probe" },
			{ type: "get_messages_page", id: "page-probe", limit: 1 },
		]);
	});

	test("an over-limit line is skipped and the next valid frame still processes", async () => {
		const giant = `{"type":"prompt","message":"${"x".repeat(MAX_RPC_INPUT_LINE_BYTES)}"}\n`;
		const input = new Blob([giant, `${JSON.stringify({ type: "get_state", id: "after-giant" })}\n`]).stream();
		const frames: unknown[] = [];
		const parseErrors: string[] = [];

		await readRpcInputFrames(
			input,
			frame => frames.push(frame),
			message => parseErrors.push(message),
		);

		expect(parseErrors).toHaveLength(1);
		expect(parseErrors[0]).toContain(`exceeds ${MAX_RPC_INPUT_LINE_BYTES} bytes`);
		expect(frames).toEqual([{ type: "get_state", id: "after-giant" }]);
	});

	test("an unterminated over-limit stream stays bounded and reports once", async () => {
		const input = new Blob([
			`${JSON.stringify({ type: "get_state", id: "first" })}\n`,
			"x".repeat(MAX_RPC_INPUT_LINE_BYTES + 16),
		]).stream();
		const frames: unknown[] = [];
		const parseErrors: string[] = [];

		await readRpcInputFrames(
			input,
			frame => frames.push(frame),
			message => parseErrors.push(message),
		);

		expect(frames).toEqual([{ type: "get_state", id: "first" }]);
		expect(parseErrors).toHaveLength(1);
		expect(parseErrors[0]).toContain(`exceeds ${MAX_RPC_INPUT_LINE_BYTES} bytes`);
	});
});

describe("RPC stdin backpressure", () => {
	test("the reader pauses on a full queue and resumes on drain with accounting", async () => {
		const encoder = new TextEncoder();
		const chunks = [
			`{"type":"get_state","id":"a"}\n`,
			`{"type":"get_state","id":"b"}\n`,
			`{"type":"get_state","id":"c"}\n`,
			`{"type":"get_state","id":"d"}\n`,
		];
		let pulls = 0;
		let index = 0;
		const input = new ReadableStream<Uint8Array>({
			pull(controller) {
				pulls++;
				if (index < chunks.length) controller.enqueue(encoder.encode(chunks[index++]));
				else controller.close();
			},
		});
		const frames: unknown[] = [];
		const parseErrors: string[] = [];
		const stats = { pauses: 0, resumes: 0 };
		// Simulate a full serial queue after two frames: the reader must stop
		// pulling stdin until the queue drains.
		let paused = false;
		let releaseDrain: (() => void) | undefined;
		const drained = new Promise<void>(resolve => {
			releaseDrain = resolve;
		});
		const reading = readRpcInputFramesWithBackpressure(
			input,
			frame => {
				frames.push(frame);
				if (frames.length === 2) paused = true;
			},
			message => parseErrors.push(message),
			{ shouldPause: () => paused, waitForDrain: () => drained, stats },
		);

		await Bun.sleep(25);
		expect(frames).toHaveLength(2);
		expect(parseErrors).toEqual([]);
		expect(stats.pauses).toBeGreaterThanOrEqual(1);
		expect(stats.resumes).toBe(0);
		// Bounded while paused: no further stdin pulls after the pause.
		const pullsWhilePaused = pulls;
		await Bun.sleep(25);
		expect(pulls).toBe(pullsWhilePaused);
		expect(frames).toHaveLength(2);

		paused = false;
		releaseDrain?.();
		await reading;
		expect(frames).toEqual([
			{ type: "get_state", id: "a" },
			{ type: "get_state", id: "b" },
			{ type: "get_state", id: "c" },
			{ type: "get_state", id: "d" },
		]);
		expect(parseErrors).toEqual([]);
		expect(stats.resumes).toBeGreaterThanOrEqual(1);
		expect(stats.pauses).toBe(stats.resumes);
	});

	test("an idle queue never pauses the reader", async () => {
		const input = new Blob([`{"type":"get_state","id":"a"}\n`, `{"type":"get_state","id":"b"}\n`]).stream();
		const frames: unknown[] = [];
		const stats = { pauses: 0, resumes: 0 };

		await readRpcInputFramesWithBackpressure(
			input,
			frame => frames.push(frame),
			() => {},
			{ shouldPause: () => false, waitForDrain: () => Promise.resolve(), stats },
		);

		expect(frames).toHaveLength(2);
		expect(stats).toEqual({ pauses: 0, resumes: 0 });
	});
});
