/**
 * Workstream C (U4c/T15+T16) — attachments, clipboard, and external-editor
 * flows through the ACTUAL input controller with downstream payload
 * assertions (never reimplemented gate predicates).
 *
 * Contracts (controller + real editor; host backends only via the
 * constructor clipboard seam and scoped process seams restored in
 * afterEach — no file-wide leaks):
 * - T15: image-only drafts submit (prompt carries the images, never a
 *   no-op); mixed payloads forward text+images; deleting a token drops that
 *   attachment from the submission with dense remapping; dispatch failure
 *   restores text, images, and links for retry; detached-Home failure keeps
 *   the draft; a 200KB bracketed paste lands intact at the model.
 * - T16: empty/throwing clipboard backends surface explicit statuses and
 *   change nothing; raw-text paste inserts on success; external-editor
 *   missing/nonzero/success/throw/cancel paths preserve focus (stop/start
 *   pairing), draft payloads, and temp hygiene through openExternalEditor.
 */
import { afterEach, beforeAll, describe, expect, it, spyOn, vi } from "bun:test";
import * as os from "node:os";
import * as path from "node:path";
import type { ImageContent } from "@harvest/pi-ai";
import { InputController } from "@harvest/pi-coding-agent/modes/controllers/input-controller";
import { CustomEditor } from "@harvest/pi-coding-agent/modes/components/custom-editor";
import { getEditorTheme, initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { InteractiveModeContext } from "@harvest/pi-coding-agent/modes/types";
import { TempDir } from "@harvest/pi-utils";

const IMAGE_A: ImageContent = { type: "image", data: "aGVsbG8=", mimeType: "image/png" };
const IMAGE_B: ImageContent = { type: "image", data: "d29ybGQ=", mimeType: "image/png" };
// 1x1 transparent PNG: exercises the real paste-decode pipeline deterministically.
const PIXEL_PNG_BASE64 =
	"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

interface CtxSpies {
	prompt: ReturnType<typeof vi.fn>;
	showError: ReturnType<typeof vi.fn>;
	showStatus: ReturnType<typeof vi.fn>;
	showWarning: ReturnType<typeof vi.fn>;
	requestRender: ReturnType<typeof vi.fn>;
	stop: ReturnType<typeof vi.fn>;
	start: ReturnType<typeof vi.fn>;
	updatePendingMessagesDisplay: ReturnType<typeof vi.fn>;
}

function createCtx(
	opts: {
		isStreaming?: boolean;
		homeDetached?: boolean;
		homeReady?: boolean;
		focused?: unknown;
	} = {},
): { ctx: InteractiveModeContext; editor: CustomEditor; spies: CtxSpies } {
	const editor = new CustomEditor(getEditorTheme());
	const prompt = vi.fn(async (_text: string, _options?: unknown) => {});
	const showError = vi.fn();
	const showStatus = vi.fn();
	const showWarning = vi.fn();
	const requestRender = vi.fn();
	const stop = vi.fn();
	const start = vi.fn();
	const updatePendingMessagesDisplay = vi.fn();
	const ctx = {
		editor,
		ui: { requestRender, getFocused: () => opts.focused ?? null, stop, start },
		skillCommands: new Map<string, string>(),
		session: {
			isStreaming: opts.isStreaming ?? false,
			isCompacting: false,
			isBashRunning: false,
			isEvalRunning: false,
			extensionRunner: undefined,
			prompt,
		},
		loopModeEnabled: false,
		compactionQueuedMessages: [],
		locallySubmittedUserSignatures: new Set<string>(),
		updatePendingMessagesDisplay,
		showError,
		showStatus,
		showWarning,
		planModeEnabled: false,
		planModePaused: false,
		vibeModeEnabled: false,
		goalModeEnabled: false,
		goalModePaused: false,
		handleGoalModeCommand: vi.fn(async () => true),
		handlePlanModeCommand: vi.fn(async () => true),
		handleVibeModeCommand: vi.fn(async () => true),
		withLocalSubmission: async (_text: string, fn: () => unknown) => fn(),
		isHomeDetached: () => opts.homeDetached ?? false,
		createSessionFromHomeDetached: async () => opts.homeReady ?? false,
		sessionManager: {
			getCwd: () => os.tmpdir(),
			putBlob: async () => "local://test-paste.png",
		},
		focusedAgentId: undefined,
	} as unknown as InteractiveModeContext;
	return {
		ctx,
		editor,
		spies: { prompt, showError, showStatus, showWarning, requestRender, stop, start, updatePendingMessagesDisplay },
	};
}

beforeAll(async () => {
	await initTheme(false);
});

afterEach(() => {
	vi.restoreAllMocks();
});

describe("T15 submission payloads through the real follow-up path", () => {
	it("submits an image-only draft instead of no-op (real empty-submit gate)", async () => {
		// Failure mode: an image with no text is dropped as an "empty
		// submit", so image-only messages never reach the model. The gate
		// under test is the controller's own deleted-chip contract
		// (`#compactDraftImages` + `!text && !images`) — asserted by the
		// downstream prompt payload, not a copy of the predicate. The token
		// form is the real image-only draft: pasting an image always stages
		// its buffer token alongside the bytes.
		const { ctx, spies } = createCtx();
		const controller = new InputController(ctx);
		ctx.editor.setDraft("[Image #1]", [IMAGE_A]);
		await controller.handleFollowUp();

		expect(spies.prompt).toHaveBeenCalledTimes(1);
		const call = spies.prompt.mock.calls[0];
		if (!call) throw new Error("expected session.prompt to be called");
		expect(call[1]).toMatchObject({ images: [IMAGE_A] });
		// Pending state is consumed so the next message does not resend it.
		expect(ctx.editor.pendingImages).toEqual([]);
	});

	it("drops marker-less staged bytes instead of submitting stale content", async () => {
		// Failure mode: bytes whose buffer token was deleted stay staged
		// and submit invisibly. Both submit paths enforce the deleted-chip
		// contract: unreferenced attachments never reach the model.
		const { ctx, spies } = createCtx();
		const controller = new InputController(ctx);
		ctx.editor.setDraft("", [IMAGE_A]);
		await controller.handleFollowUp();

		expect(spies.prompt).not.toHaveBeenCalled();
		expect(ctx.editor.pendingImages).toEqual([]);
	});

	it("forwards mixed text+image payloads with the visible text", async () => {
		const { ctx, spies } = createCtx();
		const controller = new InputController(ctx);
		ctx.editor.setDraft("[Image #1] describe this", [IMAGE_A]);
		await controller.handleFollowUp();

		expect(spies.prompt).toHaveBeenCalledTimes(1);
		const call = spies.prompt.mock.calls[0];
		if (!call) throw new Error("expected session.prompt to be called");
		expect(String(call[0])).toContain("describe this");
		expect(call[1]).toMatchObject({ images: [IMAGE_A] });
	});

	it("drops a token-deleted image from the submission with dense remapping", async () => {
		// Failure mode: a removed chip's bytes stay staged and submit stale
		// content, or surviving markers keep sparse numbering and misalign
		// the positional `[Image #N] ↔ images[N-1]` mapping.
		const { ctx, editor, spies } = createCtx();
		const controller = new InputController(ctx);
		editor.setDraft("[Image #1] [Image #2]", [IMAGE_A, IMAGE_B]);
		expect(editor.composerChips()).toHaveLength(2);
		editor.setText("[Image #1]");
		expect(editor.composerChips()).toHaveLength(1);

		await controller.handleFollowUp();

		expect(spies.prompt).toHaveBeenCalledTimes(1);
		const call = spies.prompt.mock.calls[0];
		if (!call) throw new Error("expected session.prompt to be called");
		// Exactly the surviving attachment, densely remapped to slot 1.
		expect(call[1]).toMatchObject({ images: [IMAGE_A] });
	});

	it("restores text, images, and links when dispatch rejects", async () => {
		// Failure mode: a rejected dispatch loses the draft (or its links),
		// so an image-only or text+image message cannot be retried.
		const { ctx, editor, spies } = createCtx();
		spies.prompt.mockImplementationOnce(async () => {
			throw new Error("model not configured");
		});
		const controller = new InputController(ctx);
		editor.setDraft("[Image #1] look at this", [IMAGE_A]);
		editor.pendingImageLinks = ["local://draft.png"];
		editor.imageLinks = ["local://draft.png"];
		await controller.handleFollowUp();

		expect(spies.showError).toHaveBeenCalledWith("model not configured");
		// The failed draft collapses back to chip tokens (band cards
		// return) with images and links intact for retry.
		expect(editor.getText()).toContain("look at this");
		expect(editor.composerChips()).toHaveLength(1);
		expect(ctx.editor.pendingImages).toEqual([IMAGE_A]);
		expect(ctx.editor.pendingImageLinks).toEqual(["local://draft.png"]);
		expect(ctx.editor.imageLinks).toEqual(["local://draft.png"]);
	});

	it("keeps the full draft when detached-Home recovery fails", async () => {
		// Failure mode: a failed Home→session transition consumes the draft
		// and the operator's text+images vanish with no session to show.
		const { ctx, editor, spies } = createCtx({ homeDetached: true, homeReady: false });
		const controller = new InputController(ctx);
		editor.setDraft("unsent work [Image #1]", [IMAGE_A]);
		await controller.handleFollowUp();

		expect(spies.prompt).not.toHaveBeenCalled();
		expect(editor.getText()).toContain("unsent work");
		expect(ctx.editor.pendingImages).toEqual([IMAGE_A]);
	});

	it("delivers a 200KB bracketed paste intact to the model", async () => {
		// Failure mode: large pastes truncate mid-content on the submit
		// path; the transport cap sits far above this, so every byte must
		// arrive.
		const { ctx, spies } = createCtx();
		const controller = new InputController(ctx);
		const payload = `paste-body-${"x".repeat(200 * 1024)}`;
		ctx.editor.handleInput(`\x1b[200~${payload}\x1b[201~`);
		expect(ctx.editor.getExpandedText()).toContain(payload);

		await controller.handleFollowUp();

		expect(spies.prompt).toHaveBeenCalledTimes(1);
		const call = spies.prompt.mock.calls[0];
		if (!call) throw new Error("expected session.prompt to be called");
		expect(String(call[0])).toContain(payload);
		expect(String(call[0]).length).toBeGreaterThanOrEqual(payload.length);
	});
});

describe("T16 clipboard failures through the real controller seams", () => {
	it("reports an empty clipboard and changes nothing", async () => {
		const { ctx, editor, spies } = createCtx();
		const controller = new InputController(ctx, {
			readImage: async () => null,
			readText: async () => "",
		});
		const before = editor.getText();
		const result = await controller.handleImagePaste();

		expect(result).toBe(false);
		expect(spies.showStatus).toHaveBeenCalledWith("Clipboard is empty");
		expect(editor.getText()).toBe(before);
		expect(editor.pendingImages).toEqual([]);
	});

	it("reports a clipboard backend failure instead of throwing", async () => {
		// Failure mode: a headless host's clipboard backend throws out of
		// the paste chord and breaks the session instead of degrading.
		const { ctx, editor, spies } = createCtx();
		const controller = new InputController(ctx, {
			readImage: async () => {
				throw new Error("no display");
			},
			readText: async () => {
				throw new Error("no display");
			},
		});
		const result = await controller.handleImagePaste();

		expect(result).toBe(false);
		expect(spies.showStatus).toHaveBeenCalledWith("Failed to read clipboard");
		expect(editor.pendingImages).toEqual([]);
	});

	it("attaches a real clipboard bitmap through the decode pipeline", async () => {
		// Failure mode: the bitmap path throws on missing session seams or
		// uninitialized settings instead of attaching the pasted screenshot.
		// Settings init is scoped to this test with a full reset after.
		const { resetSettingsForTest, Settings } = await import("@harvest/pi-coding-agent/config/settings");
		const directory = TempDir.createSync("@harvest-c-clipboard-bitmap-");
		try {
			await Settings.init({ inMemory: true, cwd: directory.path() });
			const { ctx, editor, spies } = createCtx();
			const controller = new InputController(ctx, {
				readImage: async () => ({ data: Buffer.from(PIXEL_PNG_BASE64, "base64"), mimeType: "image/png" }),
				readText: vi.fn(async () => {
					throw new Error("text must not be consulted when an image is present");
				}),
			});
			const result = await controller.handleImagePaste();

			expect(result).toBe(true);
			expect(editor.pendingImages).toHaveLength(1);
			expect(editor.composerChips()).toHaveLength(1);
			expect(spies.showStatus).not.toHaveBeenCalled();
		} finally {
			resetSettingsForTest();
			await directory.remove();
		}
	});

	it("raw-text paste with an empty clipboard warns and inserts nothing", async () => {
		const { ctx, editor, spies } = createCtx();
		const controller = new InputController(ctx, {
			readImage: async () => null,
			readText: async () => "",
		});
		await controller.handleClipboardTextRawPaste();

		expect(spies.showStatus).toHaveBeenCalledWith("No text in clipboard to paste raw");
		expect(editor.getText()).toBe("");
	});

	it("raw-text paste failure warns instead of throwing", async () => {
		const { ctx, spies } = createCtx();
		const controller = new InputController(ctx, {
			readImage: async () => null,
			readText: async () => {
				throw new Error("pbpaste missing");
			},
		});
		await controller.handleClipboardTextRawPaste();

		expect(spies.showStatus).toHaveBeenCalledWith("Failed to paste raw text from clipboard");
	});

	it("raw-text paste inserts clipboard text on success", async () => {
		const { ctx, editor } = createCtx();
		const controller = new InputController(ctx, {
			readImage: async () => null,
			readText: async () => "pasted-raw-text",
		});
		await controller.handleClipboardTextRawPaste();

		expect(editor.getText()).toContain("pasted-raw-text");
	});
});

describe("T16 external editor through openExternalEditor", () => {
	const originalVisual = Bun.env.VISUAL;
	const originalEditor = Bun.env.EDITOR;
	afterEach(() => {
		if (originalVisual === undefined) delete Bun.env.VISUAL;
		else Bun.env.VISUAL = originalVisual;
		if (originalEditor === undefined) delete Bun.env.EDITOR;
		else Bun.env.EDITOR = originalEditor;
	});

	function resolveTmpFile(argv: unknown): string {
		// resolveEditorSpawnCommand quotes the temp path into the command
		// line (cmd.exe verbatim on Windows), so extract the file token
		// from the joined argv instead of assuming an argv slot.
		if (!Array.isArray(argv)) throw new Error("expected argv array in spawn call");
		const flat = argv.filter((entry): entry is string => typeof entry === "string").join(" ");
		const token = flat.match(/harvest-editor-[^"\\\s]+/);
		if (!token) throw new Error("editor temp file not found in spawn argv");
		return path.isAbsolute(token[0]) ? token[0] : path.join(os.tmpdir(), token[0]);
	}

	function tmpFileFromSpawn(spawn: ReturnType<typeof spyOn>): string {
		return resolveTmpFile(spawn.mock.calls[0]?.[0]);
	}

	it("missing editor warns with focus untouched and no spawn", async () => {
		// Failure mode: a misconfigured host spawns nothing but also drops
		// UI focus or clears the draft looking for an editor. Platform is
		// scoped to POSIX for this test only (same save/restore shape as
		// external-editor.test.ts) so the missing-editor path is real.
		const originalPlatform = process.platform;
		try {
			Object.defineProperty(process, "platform", { value: "linux" });
			delete Bun.env.VISUAL;
			delete Bun.env.EDITOR;
			const { ctx, editor, spies } = createCtx();
			const spawn = spyOn(Bun, "spawn");
			try {
				editor.setText("keep me");
				await new InputController(ctx).openExternalEditor();
				expect(spies.showWarning).toHaveBeenCalledWith(
					"No editor configured. Set $VISUAL or $EDITOR environment variable.",
				);
				expect(spawn).not.toHaveBeenCalled();
				expect(spies.stop).not.toHaveBeenCalled();
				expect(spies.start).not.toHaveBeenCalled();
				expect(editor.getText()).toBe("keep me");
			} finally {
				spawn.mockRestore();
			}
		} finally {
			Object.defineProperty(process, "platform", { value: originalPlatform });
		}
	});

	it("nonzero exit keeps the draft, restores focus, and cleans the temp file", async () => {
		Bun.env.VISUAL = "failing-editor";
		delete Bun.env.EDITOR;
		const { ctx, editor, spies } = createCtx();
		const spawn = spyOn(Bun, "spawn").mockReturnValue({ exited: Promise.resolve(1) } as never);
		try {
			editor.setText("original draft");
			await new InputController(ctx).openExternalEditor();

			expect(spies.showWarning).not.toHaveBeenCalled();
			expect(editor.getText()).toBe("original draft");
			expect(spies.stop).toHaveBeenCalledTimes(1);
			expect(spies.start).toHaveBeenCalledTimes(1);
			const tmpFile = tmpFileFromSpawn(spawn);
			expect(await Bun.file(tmpFile).exists()).toBe(false);
		} finally {
			spawn.mockRestore();
		}
	});

	it("successful edit lands the edited payload in the buffer", async () => {
		Bun.env.VISUAL = "working-editor";
		delete Bun.env.EDITOR;
		const { ctx, editor, spies } = createCtx();
		const spawn = spyOn(Bun, "spawn").mockImplementation(((...args: unknown[]) => {
			return { exited: Bun.write(resolveTmpFile(args[0]), "edited in external editor").then(() => 0) };
		}) as never);
		try {
			editor.setText("original draft");
			await new InputController(ctx).openExternalEditor();

			expect(editor.getText()).toBe("edited in external editor");
			expect(spies.showWarning).not.toHaveBeenCalled();
			expect(spies.stop).toHaveBeenCalledTimes(1);
			expect(spies.start).toHaveBeenCalledTimes(1);
			expect(await Bun.file(tmpFileFromSpawn(spawn)).exists()).toBe(false);
		} finally {
			spawn.mockRestore();
		}
	});

	it("spawn failure warns, keeps the draft, restores focus, and cleans up", async () => {
		// Failure mode: a missing editor binary throws out of the edit flow
		// (or leaks its temp file) instead of warning with the draft intact.
		Bun.env.VISUAL = "editor-that-does-not-exist";
		delete Bun.env.EDITOR;
		const { ctx, editor, spies } = createCtx();
		let tmpFile = "";
		const spawn = spyOn(Bun, "spawn").mockImplementation(((...args: unknown[]) => {
			try {
				tmpFile = resolveTmpFile(args[0]);
			} catch {
				tmpFile = "";
			}
			throw new Error("spawn ENOENT");
		}) as never);
		try {
			editor.setText("original draft");
			await new InputController(ctx).openExternalEditor();

			expect(spies.showWarning).toHaveBeenCalledWith(expect.stringContaining("Failed to open external editor"));
			expect(editor.getText()).toBe("original draft");
			expect(spies.stop).toHaveBeenCalledTimes(1);
			expect(spies.start).toHaveBeenCalledTimes(1);
			if (tmpFile) expect(await Bun.file(tmpFile).exists()).toBe(false);
		} finally {
			spawn.mockRestore();
		}
	});

	it("cancel (exit 0, unmodified) preserves the draft byte-for-byte", async () => {
		Bun.env.VISUAL = "cancelling-editor";
		delete Bun.env.EDITOR;
		const { ctx, editor, spies } = createCtx();
		const spawn = spyOn(Bun, "spawn").mockReturnValue({ exited: Promise.resolve(0) } as never);
		try {
			editor.setText("original draft");
			await new InputController(ctx).openExternalEditor();

			expect(editor.getText()).toBe("original draft");
			expect(spies.showWarning).not.toHaveBeenCalled();
			expect(spies.start).toHaveBeenCalledTimes(1);
		} finally {
			spawn.mockRestore();
		}
	});
});
