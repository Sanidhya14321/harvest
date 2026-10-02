import { expect, test } from "bun:test";
import { globPaths } from "../src/glob";
import { TempDir } from "../src/temp";

test("glob exclusions preserve explicit dependency searches and user filters", async () => {
	using root = TempDir.createSync("@harvest-glob-contract-");
	await Bun.write(root.join("src/live.ts"), "source");
	await Bun.write(root.join("dist/generated.ts"), "generated");
	await Bun.write(root.join("node_modules/vendor.ts"), "vendor");
	await Bun.write(root.join(".git/private.ts"), "private");
	expect(await globPaths("**/*.ts", { cwd: root.path(), dot: true, exclude: ["**/dist/**"] })).toEqual([
		"src/live.ts",
	]);
	expect(await globPaths("node_modules/**/*.ts", { cwd: root.path() })).toEqual(["node_modules/vendor.ts"]);
});

test("an already cancelled empty glob rejects instead of appearing successfully empty", async () => {
	using root = TempDir.createSync("@harvest-glob-cancel-");
	await expect(
		globPaths("**/*.ts", { cwd: root.path(), signal: AbortSignal.abort(new Error("Cancelled")) }),
	).rejects.toThrow("Cancelled");
});
