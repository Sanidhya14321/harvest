import { expect, test } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";

test("a detached bundle without an addon or adjacent WASM retains selector callbacks, identity and ranking", async () => {
	const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-wasm-install-"));
	try {
		const nativeSource = path.resolve(import.meta.dir, "../../natives/native");
		const nativeStage = path.join(root, "stage/native");
		await fs.mkdir(nativeStage, { recursive: true });
		for (const file of await fs.readdir(nativeSource)) {
			if (file.endsWith(".js")) await fs.copyFile(path.join(nativeSource, file), path.join(nativeStage, file));
		}
		await Bun.write(path.join(nativeStage, "embedded-addon.js"), "export const embeddedAddon = null;\n");
		await fs.copyFile(path.resolve(nativeSource, "../package.json"), path.join(root, "stage/package.json"));
		const entry = path.join(root, "entry.ts");
		const modulePath = path.resolve(import.meta.dir, "../src/fuzzy.js");
		await Bun.write(
			entry,
			`import { fuzzyRank, fuzzyMatch, FuzzyText } from ${JSON.stringify(modulePath)};
import { SelectorFuzzyMatcher, truncateToWidth, wrapTextWithAnsi, visibleWidth, setHangulCompatJamoWidthOverride } from "@harvest/pi-natives";
const items = [{ text: "image provider" }, { text: "Settings" }, { text: "image provider" }];
const seen = [];
const ranked = fuzzyRank(items, "image provider", item => { if (!fuzzyMatch("settings", "Settings").matches) throw new Error("reentrant callback failed"); seen.push(item); return item.text; });
let blankCalls = 0;
const blank = fuzzyRank(items, " ", item => { blankCalls++; return item.text; });
let punctuationCalls = 0;
const punctuation = fuzzyRank(items, "!!!", item => { punctuationCalls++; return item.text; });
const prepared = new FuzzyText("OpenCode Go");
setHangulCompatJamoWidthOverride(1);
process.stdout.write(JSON.stringify({
  missingAddon: !("rank" in new SelectorFuzzyMatcher()),
  ranked: ranked.map(result => items.indexOf(result.item)),
  callbacks: seen.map(item => items.indexOf(item)), blankCalls,
  blankIdentity: blank.every((result, index) => result.item === items[index]),
  punctuationCalls, punctuation: punctuation.map(result => items.indexOf(result.item)),
  prepared: prepared.match("og").matches,
  clipped: truncateToWidth("界🙂tail", 4, 0, false, 3),
  wrapped: wrapTextWithAnsi("abcdefgh", 3, 3),
  correctedWidth: visibleWidth("ㅁ", 3),
}));
`,
		);
		const outdir = path.join(root, "installed");
		const config = {
			entrypoints: [entry],
			outdir,
			target: "bun",
			plugins: [
				{
					name: "staged-native-package",
					setup(builder) {
						builder.onResolve({ filter: /^@harvest\/pi-natives$/ }, () => ({
							path: path.join(nativeStage, "index.js"),
						}));
					},
				},
			],
		} satisfies Bun.BuildConfig;
		const build = await Bun.build(config);
		expect(build.success).toBe(true);
		const executable = path.join(outdir, process.platform === "win32" ? "selector.exe" : "selector");
		const compiled = await Bun.build({ ...config, compile: { outfile: executable } });
		expect(compiled.success).toBe(true);
		await fs.rm(path.join(root, "stage"), { recursive: true });
		await fs.rm(entry);
		for (const command of [[process.execPath, path.join(outdir, "entry.js")], [executable]]) {
			const child = Bun.spawn(command, {
				cwd: outdir,
				env: { ...process.env, XDG_DATA_HOME: path.join(root, "cache") },
				stdout: "pipe",
				stderr: "pipe",
			});
			const [stdout, stderr, exitCode] = await Promise.all([
				new Response(child.stdout).text(),
				new Response(child.stderr).text(),
				child.exited,
			]);
			expect({ exitCode, stderr: exitCode === 0 ? "" : stderr }).toEqual({ exitCode: 0, stderr: "" });
			expect(JSON.parse(stdout)).toEqual({
				missingAddon: true,
				ranked: [0, 2],
				callbacks: [0, 1, 2],
				blankCalls: 0,
				blankIdentity: true,
				punctuationCalls: 3,
				punctuation: [0, 1, 2],
				prepared: true,
				clipped: "界…",
				wrapped: ["abc", "def", "gh"],
				correctedWidth: 1,
			});
		}
	} finally {
		await fs.rm(root, { recursive: true, force: true });
	}
}, 30_000);
