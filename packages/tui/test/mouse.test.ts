import { afterEach, describe, expect, it, spyOn, vi } from "bun:test";
import * as natives from "@harvest/pi-natives";
import {
	parseSgrMouse,
	routeSelectListMouse,
	routeSgrMouseInput,
	type SelectListMouseTarget,
	type SgrMouseEvent,
} from "@harvest/pi-tui/mouse";

afterEach(() => vi.restoreAllMocks());

describe("parseSgrMouse", () => {
	it("keeps mouse routing available when an optional native addon declines decoding", () => {
		spyOn(natives, "parseSgrMouse").mockReturnValue(null);
		const calls: number[] = [];
		expect(routeSgrMouseInput("\x1b[<65;4;8M", event => calls.push(event.wheel ?? 0) > 0)).toBe(true);
		expect(calls).toEqual([1]);
	});

	it("preserves large numeric button flags and zero-based coordinate boundaries", () => {
		const event = parseSgrMouse("\x1b[<4294967361;0000;9007199254740993M");
		expect(event?.wheel).toBe(1);
		expect(event?.col).toBe(-1);
		expect(event?.row).toBe(9007199254740991);
		expect(parseSgrMouse("\x1b[<0;1;1M\n")).toBeNull();
		expect(parseSgrMouse("\x1b[<0;１;1M")).toBeNull();
	});
	it("returns null for non-mouse input", () => {
		expect(parseSgrMouse("a")).toBeNull();
		expect(parseSgrMouse("\x1b[A")).toBeNull();
		expect(parseSgrMouse("\x1b[<bogus")).toBeNull();
	});

	it("decodes left clicks with 0-based coordinates", () => {
		const event = parseSgrMouse("\x1b[<0;5;9M");
		expect(event).toEqual({
			button: 0,
			col: 4,
			row: 8,
			release: false,
			wheel: null,
			motion: false,
			leftClick: true,
		});
	});

	it("decodes releases as non-clicks", () => {
		const event = parseSgrMouse("\x1b[<0;5;9m");
		expect(event?.release).toBe(true);
		expect(event?.leftClick).toBe(false);
	});

	it("decodes wheel direction from the low button bit", () => {
		expect(parseSgrMouse("\x1b[<64;1;1M")?.wheel).toBe(-1);
		expect(parseSgrMouse("\x1b[<65;1;1M")?.wheel).toBe(1);
		expect(parseSgrMouse("\x1b[<65;1;1M")?.leftClick).toBe(false);
	});

	it("does not read a horizontal wheel report as a vertical direction", () => {
		// A two-finger trackpad scroll drifts sideways; buttons 66/67 must not
		// move a list up and back down.
		for (const report of ["\x1b[<66;1;1M", "\x1b[<67;1;1M", "\x1b[<70;1;1M"]) {
			const event = parseSgrMouse(report);
			expect(event?.wheel).toBeNull();
			expect(event?.leftClick).toBe(false);
			expect(event?.motion).toBe(false);
		}
		// Modifier bits on a vertical wheel still decode as a direction.
		expect(parseSgrMouse("\x1b[<69;1;1M")?.wheel).toBe(1);
	});

	it("decodes motion reports without treating them as clicks", () => {
		const event = parseSgrMouse("\x1b[<35;10;3M");
		expect(event?.motion).toBe(true);
		expect(event?.leftClick).toBe(false);
		expect(event?.wheel).toBeNull();
	});
});

describe("routeSgrMouseInput", () => {
	it("lets a handler decline a mouse event so the host can continue routing", () => {
		expect(routeSgrMouseInput("\x1b[<0;2;3M", () => false)).toBe(false);
	});
	it("returns false and does not call the handler for non-mouse input", () => {
		let called = false;
		const handled = routeSgrMouseInput("a", () => {
			called = true;
			return true;
		});
		expect(handled).toBe(false);
		expect(called).toBe(false);
	});

	it("decodes and forwards an SGR mouse report", () => {
		let received: SgrMouseEvent | null = null;
		const handled = routeSgrMouseInput("\x1b[<0;2;3M", event => {
			received = event;
			return true;
		});
		expect(handled).toBe(true);
		if (received === null) throw new Error("expected routeSgrMouseInput to forward an event");
		const event: SgrMouseEvent = received;
		expect(event.row).toBe(2);
		expect(event.col).toBe(1);
		expect(event.leftClick).toBe(true);
	});
});

describe("routeSelectListMouse", () => {
	function makeTarget(hit: number | undefined) {
		const calls: string[] = [];
		const target: SelectListMouseTarget = {
			handleWheel: delta => calls.push(`wheel:${delta}`),
			hitTest: () => hit,
			setHoverIndex: index => calls.push(`hover:${index}`),
			clickItem: index => calls.push(`click:${index}`),
		};
		return { target, calls };
	}

	const baseEvent: SgrMouseEvent = {
		button: 0,
		col: 0,
		row: 0,
		release: false,
		wheel: null,
		motion: false,
		leftClick: false,
	};

	it("forwards wheel notches", () => {
		const { target, calls } = makeTarget(undefined);
		const handled = routeSelectListMouse(target, { ...baseEvent, wheel: 1 }, 0);
		expect(handled).toBe(true);
		expect(calls).toEqual(["wheel:1"]);
	});

	it("hovers the hit-tested row on motion", () => {
		const { target, calls } = makeTarget(4);
		const handled = routeSelectListMouse(target, { ...baseEvent, motion: true }, 0);
		expect(handled).toBe(true);
		expect(calls).toEqual(["hover:4"]);
	});

	it("clears hover when motion misses a row", () => {
		const { target, calls } = makeTarget(undefined);
		const handled = routeSelectListMouse(target, { ...baseEvent, motion: true }, 0);
		expect(handled).toBe(true);
		expect(calls).toEqual(["hover:null"]);
	});

	it("clicks the hit-tested row", () => {
		const { target, calls } = makeTarget(2);
		const handled = routeSelectListMouse(target, { ...baseEvent, leftClick: true }, 0);
		expect(handled).toBe(true);
		expect(calls).toEqual(["click:2"]);
	});

	it("ignores release events", () => {
		const { target, calls } = makeTarget(2);
		const handled = routeSelectListMouse(target, { ...baseEvent, release: true }, 0);
		expect(handled).toBe(false);
		expect(calls).toEqual([]);
	});
});
