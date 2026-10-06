import { describe, expect, it } from "bun:test";
import { SessionViewStateStore, type DraftEditor } from "../../src/session/session-view-state";

function editor(text = "", images = 0): DraftEditor & { current: () => string } {
	let current = text;
	return {
		current: () => current,
		getText: () => current,
		setText: value => {
			current = value;
		},
		pendingImages: Array.from({ length: images }, (_, i) => ({ kind: "image", index: i }) as never),
		pendingImageLinks: Array.from({ length: images }, () => undefined),
		imageLinks: undefined,
	};
}

describe("SessionViewStateStore Home entry/exit", () => {
	it("saves the closed session draft under its UUID and empties Home without leakage", () => {
		// Failure mode: entering Home reuses the closed session's text or
		// attachments in the fresh composer.
		const store = new SessionViewStateStore();
		const ed = editor("unsent work", 2);
		ed.pendingImageLinks = ["file://a", undefined];
		ed.imageLinks = ["file://a", undefined];

		// enterHomeDetached: snapshot under closed UUID, then empty composer.
		store.saveDraft("closed-uuid", ed);
		store.saveScrollOffset("closed-uuid", 14);
		store.saveReadingAnchor("closed-uuid", { block: 3, row: 1 });
		store.clearEditor(ed);

		expect(ed.current()).toBe("");
		expect(ed.pendingImages).toHaveLength(0);
		expect(ed.pendingImageLinks).toEqual([]);
		expect(ed.imageLinks).toBeUndefined();
		expect(store.scrollOffset("closed-uuid")).toBe(14);
		expect(store.readingAnchor("closed-uuid")).toEqual({ block: 3, row: 1 });

		// Home composer (no saved draft) restores empty — no leakage.
		store.restoreDraft("home-fresh", ed);
		expect(ed.current()).toBe("");
		expect(ed.pendingImages).toHaveLength(0);

		// Returning to the closed session restores its draft + attachments.
		store.restoreDraft("closed-uuid", ed);
		expect(ed.current()).toBe("unsent work");
		expect(ed.pendingImages).toHaveLength(2);
		expect(ed.pendingImageLinks).toEqual(["file://a", undefined]);
	});

	it("empty Home drafts release the entry so a cleared composer stays cleared", () => {
		const store = new SessionViewStateStore();
		const ed = editor("typed then cleared");
		store.saveDraft("a", ed);
		ed.setText("");
		ed.pendingImages = [];
		store.saveDraft("a", ed);
		expect(store.hasDraft("a")).toBe(false);
		store.restoreDraft("a", ed);
		expect(ed.current()).toBe("");
	});

	it("clearEditor drops attachments and links so Home never reuses them", () => {
		const store = new SessionViewStateStore();
		const ed = editor("see attached", 1);
		ed.pendingImageLinks = ["file://x"];
		ed.imageLinks = ["file://x"];
		store.clearEditor(ed);
		expect(ed.current()).toBe("");
		expect(ed.pendingImages).toHaveLength(0);
		expect(ed.pendingImageLinks).toEqual([]);
		expect(ed.imageLinks).toBeUndefined();
	});

	it("scroll and anchor stay keyed by closed UUID across Home", () => {
		const store = new SessionViewStateStore();
		store.saveScrollOffset("closed-uuid", 9);
		store.saveReadingAnchor("closed-uuid", { block: 1, row: 2 });
		expect(store.scrollOffset("home-fresh")).toBe(0);
		expect(store.readingAnchor("home-fresh")).toBeUndefined();
		expect(store.scrollOffset("closed-uuid")).toBe(9);
		expect(store.readingAnchor("closed-uuid")).toEqual({ block: 1, row: 2 });
	});
});
