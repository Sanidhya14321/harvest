import type { ImageContent } from "@harvest/pi-ai";
import type { TranscriptReadingAnchor } from "../modes/components/transcript-container";

/** Unsent composer text plus its image attachments for one session. */
export interface ComposerDraftState {
	text: string;
	pendingImages: ImageContent[];
	pendingImageLinks: (string | undefined)[];
	imageLinks: readonly (string | undefined)[] | undefined;
}

/** Minimal editor surface the draft store reads and writes. */
export interface DraftEditor {
	getText(): string;
	setText(text: string): void;
	pendingImages: ImageContent[];
	pendingImageLinks: (string | undefined)[];
	imageLinks?: readonly (string | undefined)[];
}

/**
 * In-memory per-session composer drafts, keyed by stable session ID.
 *
 * Switching tabs must not leak one session's unsent text into another, and
 * returning must restore what was typed. Drafts stay in memory only: process
 * exit drops them so sensitive unsent text is never persisted.
 *
 * Home entry/exit uses the same primitives: entering Home saves the closed
 * session's draft/scroll/anchor under its UUID (`saveDraft` +
 * `saveScrollOffset` + `saveReadingAnchor`), then empties the composer with
 * {@link clearEditor} so no text, attachments, or title leak into Home.
 * Exiting Home to a fresh session restores via `restoreDraft` — a session
 * with no saved draft clears to an empty composer by construction.
 */
export class SessionViewStateStore {
	readonly #drafts = new Map<string, ComposerDraftState>();
	readonly #scrollOffsets = new Map<string, number>();
	readonly #readingAnchors = new Map<string, TranscriptReadingAnchor>();
	/** Bounds the scroll-offset index; drafts are already bounded by tab counts. */
	static readonly maxScrollEntries = 20;

	/** Snapshot the editor's draft under `sessionId`; empty drafts release the entry. */
	saveDraft(sessionId: string, editor: DraftEditor): void {
		const text = editor.getText();
		const pendingImages = [...editor.pendingImages];
		if (!text && pendingImages.length === 0) {
			this.#drafts.delete(sessionId);
			return;
		}
		this.#drafts.set(sessionId, {
			text,
			pendingImages,
			pendingImageLinks: [...editor.pendingImageLinks],
			imageLinks: editor.imageLinks ? [...editor.imageLinks] : undefined,
		});
	}

	/** Replace the editor contents with `sessionId`'s draft, clearing when none was saved. */
	restoreDraft(sessionId: string, editor: DraftEditor): void {
		const draft = this.#drafts.get(sessionId);
		if (!draft) {
			this.clearEditor(editor);
			return;
		}
		editor.pendingImages = [...draft.pendingImages];
		editor.pendingImageLinks = [...draft.pendingImageLinks];
		editor.imageLinks = draft.imageLinks ? [...draft.imageLinks] : undefined;
		editor.setText(draft.text);
	}

	/** Drop `sessionId`'s draft without touching the editor. */
	clearDraft(sessionId: string): void {
		this.#drafts.delete(sessionId);
	}

	/**
	 * Empty the editor to a fresh Home composer: no text, attachments, or
	 * image links. Mirrors the empty branch of `restoreDraft` so Home entry
	 * and fresh-session restores share one clearing contract.
	 */
	clearEditor(editor: DraftEditor): void {
		editor.setText("");
		editor.pendingImages = [];
		editor.pendingImageLinks = [];
		editor.imageLinks = undefined;
	}

	hasDraft(sessionId: string): boolean {
		return this.#drafts.has(sessionId);
	}

	/**
	 * Remember the transcript scroll-back offset for `sessionId`. Refreshes
	 * recency and evicts the oldest entry past the bound so long tab histories
	 * cannot grow the index without limit.
	 */
	saveScrollOffset(sessionId: string, offset: number): void {
		this.#scrollOffsets.delete(sessionId);
		this.#scrollOffsets.set(sessionId, Math.max(0, offset));
		while (this.#scrollOffsets.size > SessionViewStateStore.maxScrollEntries) {
			const oldest = this.#scrollOffsets.keys().next();
			if (oldest.done) break;
			this.#scrollOffsets.delete(oldest.value);
		}
	}

	/** Saved scroll-back offset in rows, or 0 when the session has none. */
	scrollOffset(sessionId: string): number {
		return this.#scrollOffsets.get(sessionId) ?? 0;
	}

	/** Remember the pinned reading anchor for `sessionId`; undefined releases to the tail. */
	saveReadingAnchor(sessionId: string, anchor: TranscriptReadingAnchor | undefined): void {
		if (anchor === undefined) {
			this.#readingAnchors.delete(sessionId);
			return;
		}
		this.#readingAnchors.delete(sessionId);
		this.#readingAnchors.set(sessionId, { block: anchor.block, row: anchor.row });
		while (this.#readingAnchors.size > SessionViewStateStore.maxScrollEntries) {
			const oldest = this.#readingAnchors.keys().next();
			if (oldest.done) break;
			this.#readingAnchors.delete(oldest.value);
		}
	}

	/** Saved reading anchor, or undefined when following the tail. */
	readingAnchor(sessionId: string): TranscriptReadingAnchor | undefined {
		return this.#readingAnchors.get(sessionId);
	}
}
