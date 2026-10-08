/**
 * Workstream C (U3/T14) — attachment chip-band layout: gap-aware admission,
 * omission without reservation, and omitted-attachment operability.
 *
 * Contracts (band + real editor only; the compositor concatenates child
 * renders, so an empty band reserves no rows — verified by returns here):
 * - Admission accounts for the inter-card gap: 28/29-cell rows show one
 *   card, 30 shows two, 44 two, 46 three, 62 four (14-cell cards + 2-cell
 *   gaps: card k starts at 16(k-1) and needs 16(k-1)+14 ≤ width).
 * - A row too narrow for even one card renders nothing (no empty rows).
 * - Omitted cards stay operable: the editor buffer (the keyboard path owns
 *   it, not the band) still holds every token, removal still drops the
 *   attachment from submission and preview, and payloads stay intact.
 * - Central sanitization holds on every rendered row: no tabs leak, rows
 *   never exceed the frame in real cell width.
 */
import { beforeAll, describe, expect, it } from "bun:test";
import { AttachmentChipsBand } from "@harvest/pi-coding-agent/modes/components/attachment-chips";
import { CustomEditor } from "@harvest/pi-coding-agent/modes/components/custom-editor";
import { getEditorTheme, initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { ImageBudget } from "@harvest/pi-tui";

function makeBand(editor: CustomEditor): AttachmentChipsBand {
	return new AttachmentChipsBand(editor, new ImageBudget(8), () => {});
}

/** Placed-card count from the top-caption row (`… #N …` per card). */
function placedCards(rows: readonly string[]): number {
	if (rows.length === 0) return 0;
	const top = Bun.stripANSI(rows[0] ?? "");
	const matches = top.match(/#\d+/g);
	return matches?.length ?? 0;
}

function cellWidths(rows: readonly string[]): number[] {
	return rows.map(row => Bun.stringWidth(Bun.stripANSI(row)));
}

beforeAll(async () => {
	await initTheme(false);
});

describe("U3 chip-band gap-aware admission (T14)", () => {
	it("admits 1 card at 28/29 cells and 2 at 30 (gap counted, no 30-cell overflow)", async () => {
		// Failure mode: admission `x + CARD_COLS > width` ignores the gap,
		// so a 28/29-cell row admits two cards whose 30-cell span overflows.
		const editor = new CustomEditor(getEditorTheme());
		editor.insertTextAttachment("alpha");
		editor.insertTextAttachment("beta");
		editor.insertTextAttachment("gamma");
		const band = makeBand(editor);
		expect(placedCards(band.render(28))).toBe(1);
		expect(placedCards(band.render(29))).toBe(1);
		expect(placedCards(band.render(30))).toBe(2);
		for (const width of [28, 29, 30]) {
			for (const cells of cellWidths(band.render(width))) {
				expect(cells).toBeLessThanOrEqual(width);
			}
		}
	});

	it("holds successive gap edges: 14→1, 44→2, 46→3, 62→4", async () => {
		// Failure mode: the fix holds only the first edge and drifts on
		// later cards (gap applied once instead of per card).
		const editor = new CustomEditor(getEditorTheme());
		for (const word of ["one", "two", "three", "four", "five"]) editor.insertTextAttachment(word);
		const band = makeBand(editor);
		expect(placedCards(band.render(14))).toBe(1);
		expect(placedCards(band.render(44))).toBe(2);
		expect(placedCards(band.render(45))).toBe(2);
		expect(placedCards(band.render(46))).toBe(3);
		expect(placedCards(band.render(61))).toBe(3);
		expect(placedCards(band.render(62))).toBe(4);
		expect(placedCards(band.render(200))).toBe(5);
		for (const width of [14, 44, 46, 62, 200]) {
			for (const cells of cellWidths(band.render(width))) {
				expect(cells).toBeLessThanOrEqual(width);
			}
		}
	});

	it("renders nothing when no card fits and nothing when none staged", async () => {
		// Failure mode: a too-narrow row still reserves six empty rows,
		// pushing the prompt box away for zero visible cards.
		const empty = new CustomEditor(getEditorTheme());
		expect(makeBand(empty).render(120)).toEqual([]);
		const editor = new CustomEditor(getEditorTheme());
		editor.insertTextAttachment("alpha");
		editor.insertTextAttachment("beta");
		const band = makeBand(editor);
		expect(band.render(13)).toEqual([]);
		expect(band.render(14).length).toBe(6);
	});

	it("omitted cards stay keyboard-operable through the editor buffer", async () => {
		// Failure mode: band omission drops the token from the draft, so a
		// keyboard operator can no longer reach, move, or delete the
		// attachment the band chose not to preview.
		const editor = new CustomEditor(getEditorTheme());
		editor.insertTextAttachment("first-paste-body");
		editor.insertTextAttachment("second-paste-body");
		editor.insertTextAttachment("third-paste-body");
		const band = makeBand(editor);
		expect(placedCards(band.render(28))).toBe(1);
		// All three tokens stay in the keyboard-owned buffer with full payloads.
		expect(editor.composerChips()).toHaveLength(3);
		expect(editor.getExpandedText()).toContain("first-paste-body");
		expect(editor.getExpandedText()).toContain("second-paste-body");
		expect(editor.getExpandedText()).toContain("third-paste-body");
		// Deleting an omitted token still drops exactly that attachment from
		// submission and preview.
		const omitted = editor.pendingTexts[2];
		if (!omitted) throw new Error("third attachment missing");
		editor.setText(editor.getText().replace(omitted.label, ""));
		expect(editor.composerChips()).toHaveLength(2);
		expect(editor.getExpandedText()).not.toContain("third-paste-body");
		expect(editor.getExpandedText()).toContain("first-paste-body");
	});

	it("mixed image + paste cards admit identically and sanitize every row", async () => {
		// Failure mode: image cards admit under a different rule than paste
		// cards, or tab/CJK payload text leaks unsanitized into border rows.
		const editor = new CustomEditor(getEditorTheme());
		// setDraft first: it replaces the draft (clearing staged pastes),
		// then the paste stages on top — paste + image = 2 cards.
		editor.setDraft("[Image #1]", [{ type: "image", data: "aGVsbG8=", mimeType: "image/png" }]);
		editor.insertTextAttachment("tab\there\n日本語テスト\naveryveryveryverylonglinebeyondtwelve");
		expect(editor.composerChips()).toHaveLength(2);
		const band = makeBand(editor);
		// Paste + image = 2 cards: 28 shows one, 30 shows both.
		expect(placedCards(band.render(28))).toBe(1);
		expect(placedCards(band.render(30))).toBe(2);
		for (const width of [28, 30, 80]) {
			const rows = band.render(width);
			expect(rows.length).toBe(6);
			for (const row of rows) {
				const plain = Bun.stripANSI(row);
				expect(plain).not.toContain("\t");
				expect(Bun.stringWidth(plain)).toBeLessThanOrEqual(width);
			}
		}
	});
});
