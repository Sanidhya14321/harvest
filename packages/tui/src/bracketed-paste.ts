export { decodeReencodedPasteControls } from "@harvest/pi-natives";

const PASTE_START = "\x1b[200~";
const PASTE_END = "\x1b[201~";

export type PasteResult = { handled: false } | { handled: true; pasteContent?: string; remaining: string };

/**
 * Options for {@link BracketedPasteHandler}.
 */
export type BracketedPasteHandlerOptions = {
	/**
	 * Byte cap for buffered paste content (default: 64 MiB). When exceeded,
	 * paste mode is aborted and the accumulated content is delivered as
	 * `pasteContent` on the same `process()` call so a lost/corrupted end
	 * marker cannot consume unbounded memory. Mirrors `StdinBuffer#abortPaste`
	 * — defense in depth for callers that bypass `StdinBuffer` (issue #4073
	 * case B). The normal `ProcessTerminal` path re-wraps `StdinBuffer`'s
	 * bounded paste with both markers, so this cap only fires on alternate
	 * callers.
	 */
	byteLimit?: number;
};

const DEFAULT_BYTE_LIMIT = 64 * 1024 * 1024;

/**
 * Handles bracketed paste mode buffering for terminal input components.
 *
 * Bracketed paste mode wraps pasted content between start (\x1b[200~) and
 * end (\x1b[201~) markers, which may arrive split across multiple chunks.
 * This class buffers incoming data and assembles complete paste payloads.
 */
export class BracketedPasteHandler {
	#buffer = "";
	#active = false;
	readonly #byteLimit: number;

	constructor(options: BracketedPasteHandlerOptions = {}) {
		this.#byteLimit = options.byteLimit ?? DEFAULT_BYTE_LIMIT;
	}

	/**
	 * Process incoming terminal data for bracketed paste sequences.
	 *
	 * @returns `{ handled: false }` if the data contains no paste sequence and
	 *          should be processed normally. `{ handled: true }` if the data was
	 *          consumed by paste buffering — `pasteContent` is set when a complete
	 *          paste has been assembled (or the byte cap has aborted a runaway
	 *          buffer); omitted when still buffering.
	 */
	process(data: string): PasteResult {
		if (data.includes(PASTE_START)) {
			this.#active = true;
			this.#buffer = "";
			data = data.replace(PASTE_START, "");
		}

		if (!this.#active) return { handled: false };

		this.#buffer += data;

		const endIndex = this.#buffer.indexOf(PASTE_END);
		if (endIndex !== -1) {
			const pasteContent = this.#buffer.substring(0, endIndex);
			const remaining = this.#buffer.substring(endIndex + PASTE_END.length);

			this.#buffer = "";
			this.#active = false;

			return { handled: true, pasteContent, remaining };
		}

		// Byte cap: a lost/corrupted end marker (ssh/tmux truncation) must not
		// consume unbounded memory. Deliver the accumulated bytes so they are
		// neither lost nor held forever, and reset paste mode so subsequent
		// input recovers. See `StdinBuffer#abortPaste` for the sibling recovery
		// semantics inside `StdinBuffer`.
		if (this.#buffer.length > this.#byteLimit) {
			const pasteContent = this.#buffer;
			this.#buffer = "";
			this.#active = false;
			return { handled: true, pasteContent, remaining: "" };
		}

		return { handled: true, remaining: "" };
	}
}
