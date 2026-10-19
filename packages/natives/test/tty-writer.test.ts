import { expect, test } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { TtyWriter } from "../native/index.js";

test.skipIf(process.platform === "win32")(
	"terminal writer owns its descriptor and rejects whole oversized frames without poisoning later writes",
	async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-terminal-writer-"));
		const target = path.join(root, "output");
		let writer: TtyWriter | undefined;
		try {
			const file = await fs.open(target, "w+");
			try {
				writer = new TtyWriter(file.fd, 16);
			} finally {
				await file.close();
			}
			// UTF-8 byte overflow; then the scratch and spill UTF-16 limit paths.
			for (const oversized of ["🙂".repeat(5), "x".repeat(17), "x".repeat(100_000)]) {
				expect(() => writer!.write(oversized)).toThrow();
			}
			writer.write("字🙂\ud800");
			expect(writer.flushSync(2000)).toBe(true);
			writer.stop(2000);
			// Preserve the legacy adapter's permissive lone-surrogate bytes.
			expect(await Bun.file(target).bytes()).toEqual(
				new Uint8Array([...new TextEncoder().encode("字🙂"), 0xed, 0xa0, 0x80]),
			);
			expect(() => writer!.write("after stop")).toThrow();
		} finally {
			writer?.stop(0);
			await fs.rm(root, { recursive: true, force: true });
		}
	},
);

test.skipIf(process.platform === "win32")("invalid terminal descriptors fail before output can be queued", () => {
	expect(() => new TtyWriter(-1)).toThrow();
});
