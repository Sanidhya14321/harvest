/**
 * Claims Bun's singleton stdin reader immediately and exposes a separately readable stream.
 * RPC startup uses this before extension discovery so in-process modules cannot steal protocol input.
 */
export function claimRpcInput(): ReadableStream<Uint8Array> {
	const reader = Bun.stdin.stream().getReader();
	let released = false;
	const release = () => {
		if (released) return;
		released = true;
		try {
			reader.releaseLock();
		} catch {}
	};
	return new ReadableStream({
		async pull(controller) {
			try {
				const result = await reader.read();
				if (result.done) {
					release();
					controller.close();
				} else {
					controller.enqueue(result.value);
				}
			} catch (error) {
				release();
				controller.error(error);
			}
		},
		async cancel() {
			try {
				await reader.cancel();
			} finally {
				release();
			}
		},
	});
}

/**
 * Maximum bytes of one newline-delimited RPC input line (content bytes,
 * excluding the newline). Matches the advertised 1 MiB physical framing
 * size so an oversized paste fails fast instead of growing memory.
 */
export const MAX_RPC_INPUT_LINE_BYTES = 1024 * 1024;

/**
 * Yields complete newline-delimited lines with a byte cap. A line that
 * exceeds `maxBytes` is reported once via `onOversizedLine` and drained
 * through its newline so the next valid frame still processes; a stream
 * that never terminates a line stays bounded the same way. Lines are
 * assembled from raw bytes and split only on LF, so multi-byte sequences
 * are never decoded mid-character.
 */
export async function* readBoundedRpcLines(
	input: ReadableStream<Uint8Array>,
	onOversizedLine: (byteLength: number) => void,
	maxBytes = MAX_RPC_INPUT_LINE_BYTES,
): AsyncGenerator<Uint8Array> {
	const reader = input.getReader();
	let parts: Uint8Array[] = [];
	let buffered = 0;
	let discarding = false;
	let discarded = 0;
	const takeLine = (): Uint8Array => {
		if (parts.length === 1) {
			const only = parts[0]!;
			parts = [];
			buffered = 0;
			return only;
		}
		const line = new Uint8Array(buffered);
		let offset = 0;
		for (const part of parts) {
			line.set(part, offset);
			offset += part.length;
		}
		parts = [];
		buffered = 0;
		return line;
	};
	try {
		for (;;) {
			const { done, value } = await reader.read();
			if (done) break;
			if (!value || value.length === 0) continue;
			let start = 0;
			for (let i = 0; i < value.length; i++) {
				if (value[i] !== 0x0a) continue;
				const segment = value.subarray(start, i);
				start = i + 1;
				if (discarding) {
					discarded += segment.length;
					discarding = false;
					onOversizedLine(discarded);
					discarded = 0;
					continue;
				}
				if (buffered + segment.length > maxBytes) {
					onOversizedLine(buffered + segment.length);
					parts = [];
					buffered = 0;
					continue;
				}
				if (segment.length > 0) {
					parts.push(segment);
					buffered += segment.length;
				}
				yield takeLine();
			}
			const tail = value.subarray(start);
			if (tail.length === 0) continue;
			if (discarding) {
				discarded += tail.length;
			} else if (buffered + tail.length > maxBytes) {
				discarding = true;
				discarded = buffered + tail.length;
				parts = [];
				buffered = 0;
			} else {
				parts.push(tail);
				buffered += tail.length;
			}
		}
	} finally {
		reader.releaseLock();
	}
	if (discarding) {
		onOversizedLine(discarded);
		return;
	}
	if (buffered > 0) yield takeLine();
}

/**
 * Parses newline-delimited RPC input without letting one malformed line stop
 * subsequent protocol frames.
 *
 * Lines are bounded by {@link MAX_RPC_INPUT_LINE_BYTES} before JSON parsing:
 * an over-limit line is reported once and skipped through its newline so one
 * giant paste cannot grow memory or stall cancellation behind parsing.
 */
export async function readRpcInputFrames(
	input: ReadableStream<Uint8Array>,
	onFrame: (frame: unknown) => void,
	onParseError: (message: string) => void,
): Promise<void> {
	const decoder = new TextDecoder();
	const lines = readBoundedRpcLines(input, byteLength =>
		onParseError(
			`Input line exceeds ${MAX_RPC_INPUT_LINE_BYTES} bytes (${byteLength} bytes); skipping to next newline`,
		),
	);
	for await (const line of lines) {
		const text = decoder.decode(line).trim();
		if (!text) continue;
		let parsed: unknown;
		try {
			parsed = JSON.parse(text);
		} catch (error: unknown) {
			const message = error instanceof Error ? error.message : String(error);
			onParseError(`Failed to parse command: ${message}`);
			continue;
		}
		onFrame(parsed);
	}
}
