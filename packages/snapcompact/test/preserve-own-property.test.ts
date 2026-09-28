import { describe, expect, it } from "bun:test";
import * as snapcompact from "../src";
import { PRESERVE_KEY } from "../src/snapcompact";

function validArchive() {
	return {
		frames: [],
		totalChars: 10,
		truncatedChars: 0,
		text: "hello",
	};
}

describe("preserve-data own-property guards", () => {
	it("stripPreservedArchive ignores a prototype-inherited archive key", () => {
		const proto = { [PRESERVE_KEY]: validArchive() } as Record<string, unknown>;
		const data = Object.assign(Object.create(proto), { other: "keep" });
		// The archive key is not an own property: nothing is stripped and the
		// input is returned untouched (previously `in` entered the strip path).
		expect(snapcompact.stripPreservedArchive(data)).toBe(data);
	});

	it("stripPreservedArchive still strips an own archive key", () => {
		const data = { [PRESERVE_KEY]: validArchive(), other: "keep" };
		expect(snapcompact.stripPreservedArchive(data)).toEqual({ other: "keep" });
	});

	it("getPreservedArchive ignores a prototype-inherited archive", () => {
		const proto = { [PRESERVE_KEY]: validArchive() } as Record<string, unknown>;
		const data = Object.create(proto) as Record<string, unknown>;
		expect(snapcompact.getPreservedArchive(data)).toBeUndefined();
	});

	it("getPreservedArchive still reads an own archive key", () => {
		const archive = snapcompact.getPreservedArchive({ [PRESERVE_KEY]: validArchive() });
		expect(archive?.text).toBe("hello");
	});

	it("archive helpers tolerate Object.prototype keys as ordinary entries", () => {
		const data = {
			[PRESERVE_KEY]: validArchive(),
			toString: "kept",
			constructor: "kept",
		} as Record<string, unknown>;
		expect(snapcompact.stripPreservedArchive(data)).toEqual({ toString: "kept", constructor: "kept" });
	});
});
