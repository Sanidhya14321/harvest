/** Terminal framing is shared with the Rust SDK; this adapter owns event delivery and host timers. */
import { NativeStdinBuffer, type NativePasteEvent } from "@harvest/pi-natives";
import { EventEmitter } from "events";
import { isKittyProtocolActive } from "./keys";

export type StdinBufferOptions = {
	/**
	 * Maximum time to wait for sequence completion (default: 75ms).
	 * After this time, a genuinely incomplete escape is flushed.
	 */
	timeout?: number;
	/**
	 * Maximum extra time (default: 150ms) an unambiguous escape partial — an
	 * SGR mouse prefix, or any dangling escape while the kitty keyboard
	 * protocol is active — is held past `timeout` waiting for its tail.
	 */
	partialHoldTimeout?: number;
	/**
	 * Paste-mode inactivity watchdog (default: 1000ms). If no input arrives for
	 * this long while waiting for the bracketed-paste end marker, the paste is
	 * assumed truncated: accumulated bytes are delivered and input recovers.
	 */
	pasteTimeout?: number;
	/**
	 * Paste-mode byte cap (default: 64 MiB). Exceeding it aborts paste mode the
	 * same way, bounding memory when the end marker never arrives.
	 */
	pasteByteLimit?: number;
};

export type StdinBufferEventMap = {
	data: [string];
	/**
	 * A completed bracketed paste. `enter` carries an Enter keypress that shared
	 * the paste's stdin read, so the terminal can dispatch paste and submit to
	 * the same focused component before a paste-triggered overlay can take focus.
	 */
	paste: [content: string, enter?: string];
};

export class StdinBuffer extends EventEmitter<StdinBufferEventMap> {
	readonly #native: NativeStdinBuffer;
	readonly #chunkBytes: number;
	readonly #clockStart = performance.now();
	#lastTime = 0;
	#timer?: ReturnType<typeof setTimeout>;

	constructor(options: StdinBufferOptions = {}) {
		super();
		this.#native = new NativeStdinBuffer({ ...options, hostTime: 0 });
		this.#chunkBytes = this.#native.chunkBytes;
	}

	process(data: string | Buffer): void {
		// Preserve the historical single high-byte Meta-key input convention.
		const text = Buffer.isBuffer(data)
			? data.length === 1 && data[0]! > 127
				? `\x1b${String.fromCharCode(data[0]! - 128)}`
				: data.toString()
			: data;
		const bytes = Buffer.from(text);
		this.#syncClock();
		this.#cancelTimer();
		try {
			if (bytes.length === 0) {
				this.#dispatch(this.#native.process(bytes, isKittyProtocolActive(), true));
			}
			for (let offset = 0; offset < bytes.length; offset += this.#chunkBytes) {
				const end = Math.min(offset + this.#chunkBytes, bytes.length);
				this.#dispatch(
					this.#native.process(bytes.subarray(offset, end), isKittyProtocolActive(), end === bytes.length),
				);
			}
		} finally {
			this.#armTimer();
		}
	}

	flush(): string[] {
		this.#cancelTimer();
		const sequences = this.#native.flush(isKittyProtocolActive());
		this.#armTimer();
		return sequences;
	}

	clear(): void {
		this.#cancelTimer();
		this.#native.clear();
	}

	destroy(): void {
		this.clear();
	}

	getBuffer(): string {
		return this.#native.getBuffer();
	}

	#dispatch(events: (string | NativePasteEvent)[]): void {
		for (const event of events) {
			if (typeof event === "string") this.emit("data", event);
			else this.emit("paste", event.content, event.enter);
		}
	}

	#cancelTimer(): void {
		if (this.#timer !== undefined) clearTimeout(this.#timer);
		this.#timer = undefined;
	}

	#syncClock(): void {
		this.#lastTime = Math.max(this.#lastTime, performance.now() - this.#clockStart);
		this.#native.setTime(this.#lastTime);
	}

	#armTimer(): void {
		this.#cancelTimer();
		this.#syncClock();
		const delay = this.#native.delay();
		if (delay === null) return;
		this.#timer = setTimeout(() => {
			this.#timer = undefined;
			try {
				this.#syncClock();
				this.#dispatch(this.#native.tick(isKittyProtocolActive()));
			} finally {
				this.#armTimer();
			}
		}, Math.ceil(delay));
	}
}
