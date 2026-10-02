import { afterEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs";
import { TempDir } from "@harvest/pi-utils";
import { SearchCodeTool } from "../src/tools/search-code";

afterEach(() => vi.restoreAllMocks());
describe("source symbol refresh", () => {
	it("refreshes an existing cache hit after edits, deletes and newly added files", async () => {
		using root = TempDir.createSync("@harvest-source-refresh-");
		const file = root.join("service.ts");
		await Bun.write(file, "export function AuthToken() {}\n");
		const tool = new SearchCodeTool({ cwd: root.path() });
		expect((await tool.execute("1", { query: "AuthToken", mode: "symbols" })).details?.totalMatches).toBe(1);
		await Bun.write(file, "\n\nexport function AuthTokenUpdated() {}\n");
		const changed = await tool.execute("2", { query: "AuthToken", mode: "symbols" });
		expect(changed.content).toEqual([{ type: "text", text: expect.stringContaining("service.ts:3") }]);
		await fs.promises.unlink(file);
		expect((await tool.execute("3", { query: "AuthToken", mode: "symbols" })).details?.totalMatches).toBe(0);
		await Bun.write(root.join("new.ts"), "export function AuthTokenNew() {}\n");
		expect((await tool.execute("4", { query: "AuthToken", mode: "symbols" })).details?.totalMatches).toBe(1);
	});

	it("keeps cached content reads incremental, shares concurrent refreshes and reports progress", async () => {
		using root = TempDir.createSync("@harvest-source-cache-");
		await Bun.write(root.join("service.ts"), "export function WorkspaceSymbol() {}\n");
		await Bun.write(root.join("node_modules/vendor.ts"), "export function VendorPrivate() {}\n");
		const tool = new SearchCodeTool({ cwd: root.path() });
		const stat = fs.promises.stat;
		const entered = Promise.withResolvers<void>();
		const release = Promise.withResolvers<void>();
		let sourceStats = 0;
		const controlledStat = (async (...args: Parameters<typeof fs.promises.stat>) => {
			if (args[0].toString().endsWith("service.ts")) {
				sourceStats++;
				entered.resolve();
				await release.promise;
			}
			return stat(...args);
		}) as typeof fs.promises.stat;
		const spy = vi.spyOn(fs.promises, "stat").mockImplementation(controlledStat);
		const progress = vi.fn();
		const first = tool.execute("1", { query: "WorkspaceSymbol", mode: "symbols" }, undefined, progress);
		await entered.promise;
		const abort = new AbortController();
		const second = tool.execute("2", { query: "WorkspaceSymbol", mode: "symbols" }, abort.signal);
		abort.abort();
		await expect(second).rejects.toThrow("Aborted");
		expect(progress).toHaveBeenCalled();
		release.resolve();
		try {
			expect((await first).details?.totalMatches).toBe(1);
			expect(sourceStats).toBe(1);
		} finally {
			release.resolve();
			spy.mockRestore();
		}
		const file = Bun.file;
		const controlledFile = ((...args: Parameters<typeof Bun.file>) => {
			if (args[0].toString().endsWith("service.ts")) throw new Error("Source content read unavailable");
			return file(...args);
		}) as typeof Bun.file;
		const readGuard = vi.spyOn(Bun, "file").mockImplementation(controlledFile);
		try {
			expect(
				(await tool.execute("cached", { query: "WorkspaceSymbol", mode: "symbols" })).details?.totalMatches,
			).toBe(1);
		} finally {
			readGuard.mockRestore();
		}
		expect((await tool.execute("3", { query: "VendorPrivate", mode: "symbols" })).details?.totalMatches).toBe(0);
	});

	it("rejects invalid limits and cancelled calls before scanning", async () => {
		using root = TempDir.createSync("@harvest-source-input-");
		const tool = new SearchCodeTool({ cwd: root.path() });
		await expect(tool.execute("1", { query: " ", mode: "symbols" })).rejects.toThrow("non-empty");
		await expect(tool.execute("2", { query: "query", limit: -1 })).rejects.toThrow("integer limit");
		await expect(tool.execute("3", { query: "query", mode: "symbols" }, AbortSignal.abort())).rejects.toThrow();
		expect(await Bun.file(root.join(".harvest/code-index.json")).exists()).toBe(false);
	});
});
