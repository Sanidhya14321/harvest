import { expect, it } from "bun:test";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import { TempDir } from "@harvest/pi-utils";
import { CodeIndex } from "../src/core/harvest/code-index";

it("does not load or overwrite a cache through a symlinked .harvest parent", async () => {
	using workspace = TempDir.createSync("harvest-index-root-");
	using outside = TempDir.createSync("harvest-index-outside-");
	const cache = outside.join("code-index.json");
	const sentinel = JSON.stringify({
		version: 1,
		files: {
			"outside.ts": {
				mtimeMs: 0,
				size: 0,
				symbols: [
					{
						name: "PrivateOutsideSymbol",
						subwords: ["private", "outside", "symbol"],
						filePath: "outside.ts",
						kind: "function",
						lineNumber: 1,
						column: 0,
						signature: "private",
					},
				],
			},
		},
	});
	await Bun.write(cache, sentinel);
	await fs.symlink(outside.path(), workspace.join(".harvest"), process.platform === "win32" ? "junction" : "dir");
	await Bun.write(workspace.join("service.ts"), "export function WorkspaceSymbol() {}\n");
	const index = new CodeIndex(workspace.path());
	expect(await index.searchSymbols("PrivateOutsideSymbol")).toEqual([]);
	await index.updateIndex(["service.ts"]);
	expect((await index.searchSymbols("WorkspaceSymbol")).map(symbol => symbol.name)).toEqual(["WorkspaceSymbol"]);
	expect(await Bun.file(cache).text()).toBe(sentinel);
});

it("rejects outside source paths and refreshes a deleted indexed file when updated", async () => {
	using workspace = TempDir.createSync("harvest-index-source-");
	using outside = TempDir.createSync("harvest-index-source-outside-");
	await Bun.write(outside.join("private.ts"), "export function PrivateOutsideSymbol() {}\n");
	await fs.symlink(outside.path(), workspace.join("linked"), process.platform === "win32" ? "junction" : "dir");
	const index = new CodeIndex(workspace.path());
	expect(await index.indexFile(path.relative(workspace.path(), outside.join("private.ts")))).toEqual([]);
	expect(await index.indexFile("linked/private.ts")).toEqual([]);
	await Bun.write(workspace.join("service.ts"), "export function WorkspaceSymbol() {}\n");
	await index.updateIndex(["service.ts"]);
	expect((await index.searchSymbols("WorkspaceSymbol")).map(symbol => symbol.name)).toEqual(["WorkspaceSymbol"]);
	await fs.unlink(workspace.join("service.ts"));
	await index.updateIndex(["service.ts"]);
	expect(await index.searchSymbols("WorkspaceSymbol")).toEqual([]);
});
