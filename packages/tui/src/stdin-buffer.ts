/**
 * StdinBuffer buffers input and emits complete sequences.
 *
 * This is necessary because stdin data events can arrive in partial chunks,
 * especially for escape sequences like mouse events. Without buffering,
 * partial sequences can be misinterpreted as regular keypresses.
 *
 * For example, the mouse SGR sequence `\x1b[<35;20;5m` might arrive as:
 * - Event 1: `\x1b`
 * - Event 2: `[<35`
 * - Event 3: `;20;5m`
 *
 * The buffer accumulates these until a complete sequence is detected.
 * Call the `process()` method to feed input data.
 *
 * Based on code from OpenTUI (https://github.com/anomalyco/opentui)
 * MIT License - Copyright (c) 2025 opentui
 */
import { EventEmitter } from "events";
import * as compatibility from "./input-compat";
import { isKittyProtocolActive } from "./keys";
import { nativeInput } from "./native-input";

const ESC = "\x1b";
const BRACKETED_PASTE_START = "\x1b[200~";
const BRACKETED_PASTE_END = "\x1b[201~";
// Paste-mode recovery bounds: a lost/corrupted end marker (ssh/tmux
// truncation) must not hang input forever or grow memory unboundedly.
const PASTE_INACTIVITY_TIMEOUT_MS = 1000;
const PASTE_MAX_BYTES = 64 * 1024 * 1024;
// A buggy double-report (CSI-u event plus the bare printable for the same
// keypress) arrives in the same terminal write; a bare char that shows up
// later than this window is a real keystroke and must not be swallowed.
const KITTY_PRINTABLE_DEDUP_WINDOW_MS = 25;
// An SGR mouse report prefix is unambiguous: no keyboard sequence starts with
// `\x1b[<`, so a buffer still matching this is always the head of a split
// mouse report. Flushing it on timeout would deliver the tail as literal
// typed text to whatever component is focused (fullscreen overlays enable
// any-motion tracking, so report floods plus render stalls make the split
// routine — see the settings search leaking `[<35;8;16M`).
const SGR_MOUSE_PARTIAL = /^\x1b\[<[\d;]*$/;
// Upper bound on how long an unambiguous partial is held past the flush
// timeout before being delivered raw anyway (terminal died mid-sequence).
// This is also the worst-case added latency for a partial that never
// completes (e.g. a bare ESC delivered while the kitty-active flag is
// stale); keep it small.
const PARTIAL_HOLD_MAX_MS = 150;
// Torn-string discard bounds. When a capped or timeout-expired OSC/DCS/APC
// partial is dropped, the bytes that follow are mid-string payload (base64
// clipboard data, Sixel, …) up to the next ST/BEL. Replaying them as
// keystrokes types kilobytes of garbage into the focused component (kitty
// OSC 5522 image paste spam), so discard mode swallows the tail until a
// terminator — bounded by a byte budget for a terminator-less stream and an
// inactivity watchdog for a stream that simply stops.
const STRING_DISCARD_MAX_BYTES = 32 * 1024 * 1024;
const STRING_DISCARD_INACTIVITY_MS = 1000;
// Partial that can only be the head of an OSC/DCS/APC string sequence.
const STRING_SEQ_PARTIAL = /^\x1b[\]P_]/;

// Raw-paste classification holds CR/LF-bearing, ESC-free input briefly so
// adjacent stdin reads from one unmarked paste can be considered together.
// Fixed from the first break-bearing read (not an inactivity debounce): normal
// Enter latency and candidate memory remain bounded even under a continuous
// stream. Ten milliseconds spans adjacent PTY reads without becoming perceptible.
const RAW_PASTE_CLASSIFICATION_TIMEOUT_MS = 10;

/**
 * Whether `text` has two completed logical line breaks (three line segments).
 *
 * A single Enter may be batched with surrounding keystrokes in one stdin read,
 * so one break is ambiguous and must stay on the key path. CRLF counts as one
 * logical break. Content after the second break completes the third segment;
 * until then the classification window keeps buffering.
 */
function isRawMultilineBurst(text: string): boolean {
	let breaks = 0;
	for (let i = 0; i < text.length; i++) {
		const code = text.charCodeAt(i);
		if (code === 0x0d) {
			breaks++;
			if (text.charCodeAt(i + 1) === 0x0a) i++;
			continue;
		}
		if (code === 0x0a) {
			breaks++;
			continue;
		}
		if (breaks >= 2) return true;
	}
	return false;
}

function parseUnmodifiedKittyPrintableCodepoint(sequence: string): number | undefined {
	const match = sequence.match(/^\x1b\[(\d+)(?::\d*)?(?::\d+)?u$/);
	if (!match) return undefined;

	const codepoint = parseInt(match[1]!, 10);
	return codepoint >= 32 ? codepoint : undefined;
}

// Small packets are faster in the existing host. Use Rust for large protocol
// replies, where bounded scanning amortizes the N-API transfer. The native Rust
// terminal can call the shared engine directly without this bridge.
// A resumed host scan stays local: copying the full accumulated string through
// N-API would defeat the offset hint, which scans only the newly arrived tail.
const MIN_NATIVE_PROTOCOL_UNITS = 4096;
function extractCompleteSequences(buffer: string, resumeSearchFrom: number) {
	if (
		!nativeInput ||
		resumeSearchFrom > 0 ||
		buffer.length < MIN_NATIVE_PROTOCOL_UNITS ||
		!STRING_SEQ_PARTIAL.test(buffer)
	) {
		return compatibility.extractCompleteSequences(buffer, resumeSearchFrom);
	}
	const result = nativeInput.scan(buffer, resumeSearchFrom);
	const sequences: string[] = [];
	for (let index = 0; index < result.ranges.length; index += 2) {
		sequences.push(buffer.slice(result.ranges[index], result.ranges[index + 1]));
	}
	return {
		sequences,
		remainder: buffer.slice(result.remainderStart),
		resumeSearchFrom: result.resumeSearchFrom,
		discardFrom: result.discardFrom ?? undefined,
	};
}

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

const KITTY_ENTER = /^\x1b\[13(?:;1)?u/u;

/**
 * The Enter keypress that immediately follows a bracketed-paste end marker in
 * the same stdin read: legacy CR/LF, or the kitty CSI-u encoding with no
 * modifier. Nothing else qualifies — any other trailing byte takes the normal
 * data route.
 */
function leadingEnter(input: string): string | undefined {
	if (input.startsWith("\r") || input.startsWith("\n")) return input[0];
	return KITTY_ENTER.exec(input)?.[0];
}

export type StdinBufferEventMap = {
	data: [string];
	/**
	 * A completed bracketed paste. `enter` carries an Enter keypress that shared
	 * the paste's stdin read, so the terminal can dispatch paste and submit to
	 * the same focused component before a paste-triggered overlay can take focus.
	 */
	paste: [content: string, enter?: string];
};

/**
 * Buffers stdin input and emits complete sequences via the 'data' event.
 * Handles partial escape sequences that arrive across multiple chunks.
 */
export class StdinBuffer extends EventEmitter<StdinBufferEventMap> {
	#buffer: string = "";
	#timeout?: NodeJS.Timeout;
	#flushDeferral?: NodeJS.Timeout;
	#partialHoldStartMs = 0;
	readonly #timeoutMs: number;
	readonly #partialHoldMaxMs: number;
	readonly #pasteTimeoutMs: number;
	readonly #pasteByteLimit: number;
	#pasteMode: boolean = false;
	#pasteChunks: string[] = [];
	#pasteOverlap: string = "";
	#pasteBytes = 0;
	#pasteWatchdog?: NodeJS.Timeout;
	#pendingKittyPrintableCodepoint: number | undefined;
	#pendingKittyPrintableAtMs = 0;
	#escapeSearchOffset = 0;
	#rawPasteCandidate = "";
	#rawPasteTimer?: NodeJS.Timeout;
	#stringDiscardActive = false;
	#stringDiscardBytes = 0;
	#stringDiscardEscHeld = false;
	#stringDiscardWatchdog?: NodeJS.Timeout;

	constructor(options: StdinBufferOptions = {}) {
		super();
		this.#timeoutMs = options.timeout ?? 75;
		this.#partialHoldMaxMs = options.partialHoldTimeout ?? PARTIAL_HOLD_MAX_MS;
		this.#pasteTimeoutMs = options.pasteTimeout ?? PASTE_INACTIVITY_TIMEOUT_MS;
		this.#pasteByteLimit = options.pasteByteLimit ?? PASTE_MAX_BYTES;
	}

	process(data: string | Buffer): void {
		// Handle high-byte conversion (for compatibility with parseKeypress)
		// If buffer has single byte > 127, convert to ESC + (byte - 128)
		let str: string;
		if (Buffer.isBuffer(data)) {
			if (data.length === 1 && data[0]! > 127) {
				const byte = data[0]! - 128;
				str = `\x1b${String.fromCharCode(byte)}`;
			} else {
				str = data.toString();
			}
		} else {
			str = data;
		}
		if (this.#stringDiscardActive) {
			// Mid torn-string: swallow payload up to the terminator; only what
			// follows the terminator re-enters normal parsing.
			str = this.#consumeStringDiscard(str);
			if (str.length === 0) return;
		}

		if (this.#flushDeferral && this.#isFreshEscapeAfterDeferredFlush(str)) {
			// The buffered partial already hit its flush timeout. A new escape is
			// a fresh sequence, not a tail; flush the stale partial first so the
			// new sequence can be parsed from a clean buffer.
			this.#flushExpired();
		} else {
			// Cancel any pending flush — new data may complete the buffered partial.
			this.#clearFlushTimer();
		}

		if (str.length === 0 && this.#buffer.length === 0 && this.#rawPasteCandidate.length === 0) {
			this.#emitDataSequence("");
			return;
		}

		if (this.#pasteMode) {
			this.#consumePasteChunk(str);
			return;
		}

		if (this.#rawPasteCandidate.length > 0) {
			if (str.indexOf(ESC) !== -1) {
				// Escape-bearing input cannot belong to an unmarked raw paste.
				// Replay the ambiguous prefix as keys before parsing the escape.
				this.#flushRawPasteCandidate();
			} else {
				this.#rawPasteCandidate += str;
				if (isRawMultilineBurst(this.#rawPasteCandidate)) {
					this.#emitRawPasteCandidate();
				}
				return;
			}
		}

		if (
			this.#buffer.length === 0 &&
			str.indexOf(ESC) === -1 &&
			(str.indexOf("\r") !== -1 || str.indexOf("\n") !== -1)
		) {
			// Hold the first break-bearing read briefly. A split raw paste can
			// then accumulate enough logical lines to classify; an ordinary
			// Enter is replayed unchanged when the fixed window expires.
			this.#rawPasteCandidate = str;
			if (isRawMultilineBurst(str)) {
				this.#emitRawPasteCandidate();
			} else {
				this.#armRawPasteTimer();
			}
			return;
		}

		this.#buffer += str;

		const startIndex = this.#buffer.indexOf(BRACKETED_PASTE_START);
		if (startIndex !== -1) {
			if (startIndex > 0) {
				const beforePaste = this.#buffer.slice(0, startIndex);
				const result = extractCompleteSequences(beforePaste, 0);
				for (const sequence of result.sequences) {
					this.#emitDataSequence(sequence);
				}
			}

			this.#escapeSearchOffset = 0;
			this.#pendingKittyPrintableCodepoint = undefined;
			this.#buffer = this.#buffer.slice(startIndex + BRACKETED_PASTE_START.length);
			const firstChunk = this.#buffer;
			this.#buffer = "";
			this.#pasteMode = true;
			this.#pasteChunks = [];
			this.#pasteOverlap = "";
			this.#pasteBytes = 0;
			this.#consumePasteChunk(firstChunk);
			return;
		}

		const result = extractCompleteSequences(this.#buffer, this.#escapeSearchOffset);
		if (result.discardFrom !== undefined) {
			const junk = this.#buffer.slice(result.discardFrom);
			this.#buffer = "";
			this.#escapeSearchOffset = 0;
			for (const sequence of result.sequences) {
				this.#emitDataSequence(sequence);
			}
			this.#enterStringDiscard();
			const after = this.#consumeStringDiscard(junk);
			if (after.length > 0) this.process(after);
			return;
		}
		this.#buffer = result.remainder;
		this.#escapeSearchOffset = result.resumeSearchFrom;

		for (const sequence of result.sequences) {
			this.#emitDataSequence(sequence);
		}

		if (this.#buffer.length > 0) {
			this.#armFlushTimer();
		} else {
			this.#partialHoldStartMs = 0;
		}
	}

	/**
	 * Consume one chunk of paste-mode input. Chunks are accumulated in an array
	 * and only joined once the end marker arrives, so a large paste delivered in
	 * many small terminal reads stays O(total) instead of the O(total^2) cost of
	 * re-concatenating and rescanning the whole buffer on every chunk. A short
	 * overlap tail (end-marker length - 1) is carried across chunk boundaries so
	 * a marker split between two reads is still detected without rescanning.
	 */
	#consumePasteChunk(chunk: string): void {
		const probe = this.#pasteOverlap + chunk;
		if (probe.indexOf(BRACKETED_PASTE_END) === -1) {
			this.#pasteChunks.push(chunk);
			this.#pasteBytes += chunk.length;
			const keep = BRACKETED_PASTE_END.length - 1;
			this.#pasteOverlap = probe.length > keep ? probe.slice(probe.length - keep) : probe;
			if (this.#pasteBytes > this.#pasteByteLimit) {
				this.#abortPaste();
				return;
			}
			this.#armPasteWatchdog();
			return;
		}

		// End marker arrived: join once and split at its first occurrence,
		// matching the prior indexOf-from-start semantics exactly.
		const flat = this.#pasteChunks.length > 0 ? `${this.#pasteChunks.join("")}${chunk}` : chunk;
		const endIndex = flat.indexOf(BRACKETED_PASTE_END);
		const pastedContent = flat.slice(0, endIndex);
		const remaining = flat.slice(endIndex + BRACKETED_PASTE_END.length);

		this.#clearPasteWatchdog();
		this.#pasteMode = false;
		this.#pasteChunks = [];
		this.#pasteOverlap = "";
		this.#pasteBytes = 0;
		this.#pendingKittyPrintableCodepoint = undefined;

		// A paste-and-Enter burst (automation, terminals that batch reads) must
		// not split into two dispatches: the paste may open an overlay that
		// would then swallow the Enter meant to submit it.
		const enter = leadingEnter(remaining);
		this.emit("paste", pastedContent, enter);

		const rest = enter === undefined ? remaining : remaining.slice(enter.length);
		if (rest.length > 0) {
			this.process(rest);
		}
	}

	/** Re-arm the paste-mode inactivity watchdog after each chunk. */
	#armPasteWatchdog(): void {
		if (this.#pasteWatchdog) clearTimeout(this.#pasteWatchdog);
		this.#pasteWatchdog = setTimeout(() => {
			this.#pasteWatchdog = undefined;
			this.#abortPaste();
		}, this.#pasteTimeoutMs);
	}

	#clearPasteWatchdog(): void {
		if (this.#pasteWatchdog) {
			clearTimeout(this.#pasteWatchdog);
			this.#pasteWatchdog = undefined;
		}
	}

	/**
	 * Recover from a paste whose end marker never arrived (dropped or corrupted
	 * in transit, or past the byte cap): exit paste mode and deliver the
	 * accumulated bytes as a paste, so they are neither lost, replayed as
	 * keystrokes, nor accumulated forever while input appears dead.
	 */
	#abortPaste(): void {
		this.#clearPasteWatchdog();
		const content = this.#pasteChunks.join("");
		this.#pasteMode = false;
		this.#pasteChunks = [];
		this.#pasteOverlap = "";
		this.#pasteBytes = 0;
		this.emit("paste", content);
	}

	/** Start one fixed window from the first break-bearing raw read. */
	#armRawPasteTimer(): void {
		if (this.#rawPasteTimer) return;
		this.#rawPasteTimer = setTimeout(() => {
			this.#rawPasteTimer = undefined;
			this.#flushRawPasteCandidate();
		}, RAW_PASTE_CLASSIFICATION_TIMEOUT_MS);
	}

	#clearRawPasteTimer(): void {
		if (this.#rawPasteTimer) {
			clearTimeout(this.#rawPasteTimer);
			this.#rawPasteTimer = undefined;
		}
	}

	#takeRawPasteCandidate(): string {
		this.#clearRawPasteTimer();
		const content = this.#rawPasteCandidate;
		this.#rawPasteCandidate = "";
		return content;
	}

	/** Emit a classified raw multiline burst through the paste channel. */
	#emitRawPasteCandidate(): void {
		const content = this.#takeRawPasteCandidate();
		this.#pendingKittyPrintableCodepoint = undefined;
		this.emit("paste", content);
	}

	/** Replay an ambiguous raw candidate as the original per-key data events. */
	#flushRawPasteCandidate(): void {
		const content = this.#takeRawPasteCandidate();
		if (content.length === 0) return;
		const result = extractCompleteSequences(content, 0);
		for (const sequence of result.sequences) {
			this.#emitDataSequence(sequence);
		}
	}

	#emitDataSequence(sequence: string): void {
		const rawCodepoint = sequence.length === 1 ? sequence.codePointAt(0) : undefined;
		if (
			rawCodepoint !== undefined &&
			rawCodepoint === this.#pendingKittyPrintableCodepoint &&
			Date.now() - this.#pendingKittyPrintableAtMs <= KITTY_PRINTABLE_DEDUP_WINDOW_MS
		) {
			this.#pendingKittyPrintableCodepoint = undefined;
			return;
		}

		this.#pendingKittyPrintableCodepoint = parseUnmodifiedKittyPrintableCodepoint(sequence);
		if (this.#pendingKittyPrintableCodepoint !== undefined) {
			this.#pendingKittyPrintableAtMs = Date.now();
		}
		this.emit("data", sequence);
	}

	/**
	 * setTimeout(0): when the event loop stalls past the timeout (heavy render)
	 * while the tail of a split escape is already queued on stdin, expired
	 * timers run before the poll phase that delivers the tail — flushing
	 * straight from the timer would tear the sequence apart and leak the tail
	 * as typed text. The zero-delay deferral runs on the next timers pass,
	 * after poll has had a chance to deliver the pending chunk to process()
	 * and cancel the deferral.
	 */
	#armFlushTimer(): void {
		this.#timeout = setTimeout(() => {
			this.#timeout = undefined;
			this.#flushDeferral = setTimeout(() => {
				this.#flushDeferral = undefined;
				this.#flushExpired(true);
			});
		}, this.#timeoutMs);
	}

	#clearFlushTimer(): void {
		if (this.#timeout) {
			clearTimeout(this.#timeout);
			this.#timeout = undefined;
		}
		if (this.#flushDeferral) {
			clearTimeout(this.#flushDeferral);
			this.#flushDeferral = undefined;
		}
	}

	/**
	 * A deferred flush means the current buffer already waited for the
	 * incomplete-sequence timeout. If the next chunk starts a fresh escape, do
	 * not merge it into the stale partial. Keep ESC-backslash as a continuation
	 * for OSC/DCS/APC string terminators (`ST`).
	 */
	#isFreshEscapeAfterDeferredFlush(str: string): boolean {
		if (!str.startsWith(ESC) || this.#buffer.length === 0) return false;
		if (
			str.startsWith(`${ESC}\\`) &&
			(this.#buffer.startsWith(`${ESC}]`) ||
				this.#buffer.startsWith(`${ESC}P`) ||
				this.#buffer.startsWith(`${ESC}_`))
		) {
			return false;
		}
		return true;
	}

	/**
	 * Whether the dangling partial cannot be a finished keypress and is worth
	 * holding for its tail instead of flushing:
	 * - SGR mouse prefixes (`\x1b[<…`) — no keyboard sequence uses them.
	 * - Any partial while the kitty keyboard protocol is active — the ESC key
	 *   arrives as `\x1b[27u` and alt-chords as CSI-u, so a bare `\x1b` (or
	 *   any unterminated escape) is always a split sequence, never a key.
	 */
	#shouldHoldPartial(): boolean {
		return SGR_MOUSE_PARTIAL.test(this.#buffer) || isKittyProtocolActive();
	}

	/** Timeout-driven flush: hold unambiguous partials (bounded), else deliver. */
	#flushExpired(fromTimer = false): void {
		if (this.#buffer.length === 0) {
			this.#partialHoldStartMs = 0;
			return;
		}
		if (this.#shouldHoldPartial()) {
			if (this.#partialHoldStartMs === 0) this.#partialHoldStartMs = Date.now();
			if (Date.now() - this.#partialHoldStartMs < this.#partialHoldMaxMs) {
				this.#armFlushTimer();
				return;
			}
		}
		this.#partialHoldStartMs = 0;
		for (const sequence of this.#drainBuffered(fromTimer)) {
			this.#emitDataSequence(sequence);
		}
	}

	flush(): string[] {
		return this.#drainBuffered(false);
	}

	#drainBuffered(discardTornString: boolean): string[] {
		this.#clearFlushTimer();

		const rawCandidate = this.#takeRawPasteCandidate();
		const sequences = rawCandidate.length > 0 ? extractCompleteSequences(rawCandidate, 0).sequences : [];

		if (this.#buffer.length === 0) {
			this.#pendingKittyPrintableCodepoint = undefined;
			return sequences;
		}

		const buffered = this.#buffer;
		this.#buffer = "";
		this.#escapeSearchOffset = 0;
		this.#pendingKittyPrintableCodepoint = undefined;
		// Bare double-ESC remainder (no disambiguating "[" / "O" arrived in time):
		// two real Esc keypresses bursted by terminal batching, not a meta-CSI/SS3
		// prefix. `parseKey` returns undefined for the combined chunk, so a single
		// emission swallows the double-escape gesture (#3857). Mirror the inline
		// split in `extractCompleteSequences` and deliver two ESC events.
		if (buffered === `${ESC}${ESC}`) {
			sequences.push(ESC, ESC);
		} else if (isKittyProtocolActive() && STRING_SEQ_PARTIAL.test(buffered)) {
			// Torn OSC/DCS/APC head. Under the kitty keyboard protocol no key
			// ever arrives as a bare `\x1b]`/`\x1bP`/`\x1b_`, so this is always
			// terminal protocol data (kitty OSC 5522 clipboard read, graphics
			// reply) whose tail is still in flight. Never deliver the head as a
			// key; on a timeout flush also swallow the tail up to its
			// terminator so kilobytes of base64 are not typed into the focused
			// component (image paste spam).
			if (discardTornString) this.#enterStringDiscard();
		} else {
			sequences.push(buffered);
		}
		return sequences;
	}

	/** Enter torn-string discard: swallow mid-string bytes until ST/BEL. */
	#enterStringDiscard(): void {
		this.#stringDiscardActive = true;
		this.#stringDiscardBytes = 0;
		this.#stringDiscardEscHeld = false;
		this.#armStringDiscardWatchdog();
	}

	#exitStringDiscard(): void {
		this.#stringDiscardActive = false;
		this.#stringDiscardBytes = 0;
		this.#stringDiscardEscHeld = false;
		if (this.#stringDiscardWatchdog) {
			clearTimeout(this.#stringDiscardWatchdog);
			this.#stringDiscardWatchdog = undefined;
		}
	}

	/**
	 * Swallow mid-string bytes; returns the input remaining after the string
	 * terminator (empty while still inside the torn string). A trailing ESC is
	 * held across calls so an `ESC \` split between reads is still detected.
	 * The byte budget bounds a terminator-less stream; the inactivity watchdog
	 * recovers a stream that stops without ever terminating.
	 */
	#consumeStringDiscard(str: string): string {
		if (this.#stringDiscardEscHeld) {
			this.#stringDiscardEscHeld = false;
			if (str.charCodeAt(0) === 0x5c /* \ */) {
				this.#exitStringDiscard();
				return str.slice(1);
			}
		}
		for (let i = 0; i < str.length; i++) {
			const code = str.charCodeAt(i);
			if (code === 0x07 /* BEL */) {
				this.#exitStringDiscard();
				return str.slice(i + 1);
			}
			if (code === 0x1b /* ESC */) {
				if (i + 1 === str.length) {
					this.#stringDiscardEscHeld = true;
					break;
				}
				if (str.charCodeAt(i + 1) === 0x5c /* \ */) {
					this.#exitStringDiscard();
					return str.slice(i + 2);
				}
			}
		}
		this.#stringDiscardBytes += str.length;
		if (this.#stringDiscardBytes > STRING_DISCARD_MAX_BYTES) {
			this.#exitStringDiscard();
			return "";
		}
		this.#armStringDiscardWatchdog();
		return "";
	}

	#armStringDiscardWatchdog(): void {
		if (this.#stringDiscardWatchdog) clearTimeout(this.#stringDiscardWatchdog);
		this.#stringDiscardWatchdog = setTimeout(() => {
			this.#stringDiscardWatchdog = undefined;
			this.#exitStringDiscard();
		}, STRING_DISCARD_INACTIVITY_MS);
	}

	clear(): void {
		this.#clearFlushTimer();
		this.#clearPasteWatchdog();
		this.#clearRawPasteTimer();
		this.#exitStringDiscard();
		this.#buffer = "";
		this.#rawPasteCandidate = "";
		this.#pasteMode = false;
		this.#pasteChunks = [];
		this.#pasteOverlap = "";
		this.#pasteBytes = 0;
		this.#pendingKittyPrintableCodepoint = undefined;
		this.#partialHoldStartMs = 0;
		this.#escapeSearchOffset = 0;
	}

	getBuffer(): string {
		return `${this.#rawPasteCandidate}${this.#buffer}`;
	}

	destroy(): void {
		this.clear();
	}
}
