/**
 * Workstream C (C4) — composer attachment/clipboard/editor flows through the
 * real editor, chip band, and failure contracts.
 *
 * Contracts (editor/component/utils surface only; submit gating and
 * controller wiring are owner-owned and untouched):
 * - Staged text pastes preview as band cards; deleting the inline token
 *   removes the attachment from the submission (composerChips) and the
 *   preview (band renders nothing).
 * - An image-only draft carries the image attachment with empty text.
 * - Clipboard reads never throw: text degrades to "" and images to null.
 * - A failing external editor resolves null (never throws) and cleans up
 *   its temp file.
 * - Large pastes within the transport cap land intact at the editor level.
 */
import { afterEach, beforeAll, describe, expect, it, spyOn, vi } from "bun:test";
import * as os from "node:os";
import * as path from "node:path";
import type { ImageContent } from "@harvest/pi-ai";
import { AttachmentChipsBand } from "@harvest/pi-coding-agent/modes/components/attachment-chips";
import { CustomEditor } from "@harvest/pi-coding-agent/modes/components/custom-editor";
import { InputController } from "@harvest/pi-coding-agent/modes/controllers/input-controller";
import type { InteractiveModeContext } from "@harvest/pi-coding-agent/modes/types";
import { getEditorTheme, initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { openInEditor } from "@harvest/pi-coding-agent/utils/external-editor";
import { readImageFromClipboard, readTextFromClipboard } from "@harvest/pi-coding-agent/utils/clipboard";
import { ImageBudget } from "@harvest/pi-tui";

const image: ImageContent = { type: "image", data: "aGVsbG8=", mimeType: "image/png" };

function makeEditor(): { editor: CustomEditor; band: AttachmentChipsBand } {
	const editor = new CustomEditor(getEditorTheme());
	return { editor, band: new AttachmentChipsBand(editor, new ImageBudget(8), () => {}) };
}

beforeAll(async () => {
	await initTheme(false);
});

afterEach(() => {
	vi.restoreAllMocks();
});

describe("composer attachment removal and preview (C4)", () => {
	it("deleting the inline token drops the attachment from submission and preview", () => {
		// Failure mode: a removed chip's text stays staged and submits
		// stale content, or the band keeps previewing a deleted attachment.
		const { editor, band } = makeEditor();
		editor.insertTextAttachment("first");
		editor.insertTextAttachment("second");
		expect(editor.composerChips()).toHaveLength(2);
		expect(band.render(120)).not.toEqual([]);

		const second = editor.pendingTexts[1];
		expect(second).toBeDefined();
		editor.setText(editor.getText().replace(second!.label, ""));
		expect(editor.composerChips()).toHaveLength(1);
		expect(editor.composerChips()[0]).toMatchObject({ kind: "paste", n: 1 });

		const first = editor.pendingTexts[0];
		editor.setText(editor.getText().replace(first!.label, ""));
		expect(editor.composerChips()).toEqual([]);
		expect(band.render(120)).toEqual([]);
	});

	it("previews staged pastes as snippet cards with size captions", () => {
		// Failure mode: the band renders nothing for a staged paste (dead
		// preview), or wraps cards instead of omitting them at narrow widths.
		const { editor, band } = makeEditor();
		expect(band.render(120)).toEqual([]);
		editor.insertTextAttachment("line1\nline2\nline3");
		const rendered = Bun.stripANSI(band.render(120).join("\n"));
		expect(rendered).toContain("line1");
		expect(rendered).toMatch(/3 lines|chars/);
		for (const row of band.render(60)) {
			expect(Bun.stripANSI(row).length).toBeLessThanOrEqual(60);
		}
	});

	it("submits an image-only draft through the real follow-up path", async () => {
		// Failure mode: an image with no text is dropped from draft state,
		// so an image-only submission has nothing to send. The contract is
		// the controller's own empty-submit gate (`!text && !images` in
		// `handleFollowUp`): a token-backed image draft submits instead of
		// no-op, asserted by the downstream prompt payload. (Marker-less
		// staged bytes are dropped by the deleted-chip contract — covered in
		// workstream-c-input-paths.)
		const { editor } = makeEditor();
		const prompt = vi.fn(async (_text: string, _options?: unknown) => {});
		const ctx = {
			editor,
			ui: { requestRender: () => {} },
			skillCommands: new Map<string, string>(),
			session: {
				isStreaming: false,
				isCompacting: false,
				isBashRunning: false,
				isEvalRunning: false,
				extensionRunner: undefined,
				prompt,
			},
			loopModeEnabled: false,
			compactionQueuedMessages: [],
			locallySubmittedUserSignatures: new Set<string>(),
			updatePendingMessagesDisplay: () => {},
			showError: () => {},
			planModeEnabled: false,
			planModePaused: false,
			vibeModeEnabled: false,
			goalModeEnabled: false,
			goalModePaused: false,
			handleGoalModeCommand: async () => true,
			handlePlanModeCommand: async () => true,
			handleVibeModeCommand: async () => true,
			withLocalSubmission: async (_text: string, fn: () => unknown) => fn(),
			isHomeDetached: () => false,
			createSessionFromHomeDetached: async () => false,
			focusedAgentId: undefined,
		} as unknown as InteractiveModeContext;
		editor.setDraft("[Image #1]", [image]);
		await new InputController(ctx).handleFollowUp();
		expect(prompt).toHaveBeenCalledTimes(1);
		const call = prompt.mock.calls[0] as unknown as [string, { images?: unknown[] }?];
		if (!call) throw new Error("expected session.prompt to be called");
		expect(call[1]?.images).toEqual([image]);
		expect(editor.pendingImages).toEqual([]);
	});

	it("shows a token-backed image chip for preview", () => {
		const { editor } = makeEditor();
		editor.setDraft("[Image #1]", [image]);
		expect(editor.composerChips()).toMatchObject([{ kind: "image", n: 1 }]);
	});
});

describe("clipboard and external-editor failure contracts (C4)", () => {
	it("clipboard reads degrade to empty/null instead of throwing", async () => {
		// Failure mode: a missing clipboard backend (headless host, no
		// display, empty selection) throws and breaks the paste path instead
		// of falling back to text/empty. These are host read-only calls.
		const text = await readTextFromClipboard();
		expect(typeof text).toBe("string");
		const clipboardImage = await readImageFromClipboard();
		if (clipboardImage !== null) {
			expect(clipboardImage.mimeType.startsWith("image/")).toBe(true);
			expect(clipboardImage.data.byteLength).toBeGreaterThan(0);
		}
	});

	it("a failing external editor resolves null and removes its temp file", async () => {
		// Failure mode: a misconfigured editor (bad $VISUAL/$EDITOR, crash
		// on launch) throws out of the edit flow or leaks temp files.
		let tmpFile = "";
		const spawn = spyOn(Bun, "spawn").mockImplementation(((...args: unknown[]) => {
			const cmd = (args[0] as string[]).join(" ");
			const match = cmd.match(/harvest-editor-[^"\\s]+/);
			if (match) tmpFile = match[0];
			return { exited: Promise.resolve(1) } as never;
		}) as never);
		try {
			const result = await openInEditor("editor-that-fails", "original");
			expect(result).toBeNull();
			expect(spawn).toHaveBeenCalledTimes(1);
			expect(spawn.mock.calls[0]?.[1]).toMatchObject({
				stdin: "inherit",
				stdout: "inherit",
				stderr: "inherit",
			});
			if (tmpFile) {
				const matches = await Array.fromAsync(
					new Bun.Glob("harvest-editor-*").scan({ cwd: os.tmpdir(), absolute: true }),
				).catch(() => [] as string[]);
				expect(matches).not.toContain(path.join(os.tmpdir(), tmpFile));
			}
		} finally {
			spawn.mockRestore();
		}
	});
});

describe("bounded large paste at the editor level (C4)", () => {
	it("lands a 200KB bracketed paste intact without truncation or crash", () => {
		// Failure mode: large pastes truncate mid-content or crash the
		// editor; the transport cap (64MiB stdin bound) sits far above this.
		// The editor's default collapse keeps the buffer navigable while the
		// full payload stays expandable for submit — no byte is lost.
		const { editor } = makeEditor();
		const payload = `paste-body-${"x".repeat(200 * 1024)}`;
		editor.handleInput(`\x1b[200~${payload}\x1b[201~`);
		expect(editor.getText()).toContain("[Paste #1,");
		expect(editor.getExpandedText()).toContain(payload);
		expect(editor.getExpandedText().length).toBeGreaterThanOrEqual(payload.length);
	});
});
