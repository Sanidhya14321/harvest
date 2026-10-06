import { afterEach, describe, expect, it, vi } from "bun:test";
import type { ImageContent } from "@harvest/pi-ai";
import { InputController } from "@harvest/pi-coding-agent/modes/controllers/input-controller";
import type { InteractiveModeContext } from "@harvest/pi-coding-agent/modes/types";

function createHomeContext(
	opts: { detached: boolean; creationSucceeds: boolean },
	attachments?: { images: ImageContent[]; links: (string | undefined)[] },
) {
	let editorText = "hello home";
	const prompt = vi.fn(async () => {});
	const createSessionFromHomeDetached = vi.fn(async () => {
		if (!opts.creationSucceeds) return false;
		// Production creation rebuilds the view and clears the editor, like
		// #finishNewSessionFlow + restoreDraft with no saved draft.
		editorText = "";
		editor.pendingImages = [];
		editor.pendingImageLinks = [];
		editor.imageLinks = undefined;
		return true;
	});
	const editor = {
		getText: () => editorText,
		setText: (text: string) => {
			editorText = text;
		},
		setCollapsedText: (text: string) => {
			editorText = text;
		},
		composerChips: () => [],
		addToHistory: vi.fn(),
		clearDraft: vi.fn(),
		imageLinks: (attachments ? [...attachments.links] : undefined) as (string | undefined)[] | undefined,
		pendingImages: [...(attachments?.images ?? [])] as ImageContent[],
		pendingImageLinks: [...(attachments?.links ?? [])] as (string | undefined)[],
	};
	const ctx = {
		editor,
		ui: { requestRender: vi.fn() },
		session: {
			isStreaming: false,
			isCompacting: false,
			isBashRunning: false,
			isEvalRunning: false,
			extensionRunner: undefined,
			prompt,
			maybeStartTitleGeneration: vi.fn(),
		},
		focusedAgentId: undefined,
		compactionQueuedMessages: [],
		locallySubmittedUserSignatures: new Set<string>(),
		isHomeDetached: () => opts.detached,
		createSessionFromHomeDetached,
		showError: vi.fn(),
		showStatus: vi.fn(),
		updatePendingMessagesDisplay: vi.fn(),
		flushPendingBashComponents: vi.fn(),
		onInputCallback: undefined,
		withLocalSubmission: async <T>(_text: string, fn: () => Promise<T>) => fn(),
	} as unknown as InteractiveModeContext;
	return { ctx, editor, prompt, createSessionFromHomeDetached };
}

describe("InputController zero-tab Home gate", () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("creates a fresh runtime before dispatching Home input, never the hidden session", async () => {
		const { ctx, prompt, createSessionFromHomeDetached } = createHomeContext({
			detached: true,
			creationSucceeds: true,
		});
		const controller = new InputController(ctx);
		controller.setupEditorSubmitHandler();
		await ctx.editor.onSubmit?.("hello home");
		expect(createSessionFromHomeDetached).toHaveBeenCalledTimes(1);
		expect(prompt).toHaveBeenCalledTimes(1);
	});

	it("preserves the Home draft when creation fails and dispatches nothing", async () => {
		const { ctx, editor, prompt, createSessionFromHomeDetached } = createHomeContext({
			detached: true,
			creationSucceeds: false,
		});
		const controller = new InputController(ctx);
		controller.setupEditorSubmitHandler();
		await ctx.editor.onSubmit?.("hello home");
		expect(createSessionFromHomeDetached).toHaveBeenCalledTimes(1);
		expect(prompt).not.toHaveBeenCalled();
		expect(editor.getText()).toBe("hello home");
	});

	it("dispatches directly when the view is attached to a live session", async () => {
		const { ctx, prompt, createSessionFromHomeDetached } = createHomeContext({
			detached: false,
			creationSucceeds: false,
		});
		const controller = new InputController(ctx);
		controller.setupEditorSubmitHandler();
		await ctx.editor.onSubmit?.("hello home");
		expect(createSessionFromHomeDetached).not.toHaveBeenCalled();
		expect(prompt).toHaveBeenCalledTimes(1);
	});

	it("creates nothing for empty detached-Home submits", async () => {
		const { ctx, prompt, createSessionFromHomeDetached } = createHomeContext({
			detached: true,
			creationSucceeds: true,
		});
		const controller = new InputController(ctx);
		controller.setupEditorSubmitHandler();
		await ctx.editor.onSubmit?.("");
		expect(createSessionFromHomeDetached).not.toHaveBeenCalled();
		expect(prompt).not.toHaveBeenCalled();
	});

	it("preserves PNG attachments and image links across fresh-session creation", async () => {
		const image: ImageContent = { type: "image", mimeType: "image/png", data: "aW1hZ2U=" };
		const { ctx, editor, prompt } = createHomeContext(
			{ detached: true, creationSucceeds: true },
			{ images: [image], links: ["local://draft.png"] },
		);
		const controller = new InputController(ctx);
		controller.setupEditorSubmitHandler();
		await ctx.editor.onSubmit?.("look at this [Image #1]");
		expect(prompt).toHaveBeenCalledTimes(1);
		const [, options] = prompt.mock.calls[0] as unknown as [string, { images?: ImageContent[] }?];
		expect(options?.images).toEqual([image]);
		// Successful dispatch clears the composer, like any normal submit.
		expect(editor.pendingImages).toEqual([]);
	});

	it("dispatches image-only Home submissions to the fresh session", async () => {
		const image: ImageContent = { type: "image", mimeType: "image/png", data: "aW1hZ2U=" };
		const { ctx, prompt } = createHomeContext(
			{ detached: true, creationSucceeds: true },
			{ images: [image], links: [undefined] },
		);
		const controller = new InputController(ctx);
		controller.setupEditorSubmitHandler();
		await ctx.editor.onSubmit?.("[Image #1]");
		expect(prompt).toHaveBeenCalledTimes(1);
	});

	it("restores images and links when creation fails", async () => {
		const image: ImageContent = { type: "image", mimeType: "image/png", data: "aW1hZ2U=" };
		const { ctx, editor, prompt } = createHomeContext(
			{ detached: true, creationSucceeds: false },
			{ images: [image], links: ["local://draft.png"] },
		);
		const controller = new InputController(ctx);
		controller.setupEditorSubmitHandler();
		await ctx.editor.onSubmit?.("look at this [Image #1]");
		expect(prompt).not.toHaveBeenCalled();
		expect(editor.pendingImages).toEqual([image]);
		expect(editor.pendingImageLinks).toEqual(["local://draft.png"]);
		expect(editor.imageLinks).toEqual(["local://draft.png"]);
	});
});
