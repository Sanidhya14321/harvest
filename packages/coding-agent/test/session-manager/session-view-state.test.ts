import { describe, expect, it } from "bun:test";
import { SessionViewStateStore, type DraftEditor } from "../../src/session/session-view-state";

function fakeEditor(text = "", images: number[] = []): DraftEditor & { current: () => string } {
	let current = text;
	return {
		current: () => current,
		getText: () => current,
		setText: value => {
			current = value;
		},
		pendingImages: images.map(index => ({ kind: "image", index }) as never),
		pendingImageLinks: images.map(() => undefined),
		imageLinks: undefined,
	};
}

describe("SessionViewStateStore", () => {
	it("restores each session's draft without leaking it into the other", () => {
		const store = new SessionViewStateStore();
		const editor = fakeEditor();
		editor.setText("draft for A");
		store.saveDraft("a", editor);
		editor.setText("draft for B");
		store.saveDraft("b", editor);
		store.restoreDraft("a", editor);
		expect(editor.current()).toBe("draft for A");
		store.restoreDraft("b", editor);
		expect(editor.current()).toBe("draft for B");
	});

	it("clears the editor when the target session has no saved draft", () => {
		const store = new SessionViewStateStore();
		const editor = fakeEditor("leftover from A");
		store.saveDraft("a", editor);
		store.restoreDraft("b", editor);
		expect(editor.current()).toBe("");
		store.restoreDraft("a", editor);
		expect(editor.current()).toBe("leftover from A");
	});

	it("drops empty drafts so a cleared composer stays cleared", () => {
		const store = new SessionViewStateStore();
		const editor = fakeEditor("typed then cleared");
		store.saveDraft("a", editor);
		editor.setText("");
		store.saveDraft("a", editor);
		expect(store.hasDraft("a")).toBe(false);
		editor.setText("other session");
		store.restoreDraft("a", editor);
		expect(editor.current()).toBe("");
	});

	it("keeps image attachments with their session's draft", () => {
		const store = new SessionViewStateStore();
		const editor = fakeEditor("see attached", [1, 2]);
		editor.pendingImageLinks = ["file://a", undefined];
		store.saveDraft("a", editor);
		store.restoreDraft("b", editor);
		expect(editor.pendingImages).toHaveLength(0);
		store.restoreDraft("a", editor);
		expect(editor.current()).toBe("see attached");
		expect(editor.pendingImages).toHaveLength(2);
		expect(editor.pendingImageLinks).toEqual(["file://a", undefined]);
	});

	it("restores each session's scroll offset without moving the other", () => {
		const store = new SessionViewStateStore();
		store.saveScrollOffset("a", 12);
		store.saveScrollOffset("b", 0);
		expect(store.scrollOffset("a")).toBe(12);
		expect(store.scrollOffset("b")).toBe(0);
		expect(store.scrollOffset("unknown")).toBe(0);
		store.saveScrollOffset("a", 3);
		expect(store.scrollOffset("a")).toBe(3);
	});

	it("evicts the oldest scroll offset past the bound", () => {
		const store = new SessionViewStateStore();
		for (let index = 0; index <= SessionViewStateStore.maxScrollEntries; index++) {
			store.saveScrollOffset(`session-${index}`, index);
		}
		expect(store.scrollOffset("session-0")).toBe(0);
		expect(store.scrollOffset(`session-${SessionViewStateStore.maxScrollEntries}`)).toBe(
			SessionViewStateStore.maxScrollEntries,
		);
	});
});
