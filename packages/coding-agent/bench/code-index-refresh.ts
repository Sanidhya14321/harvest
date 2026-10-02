/** Controlled scheduler probe, not a real terminal or an overall throughput claim. */
import * as fs from "node:fs";
import * as path from "node:path";
import { TempDir } from "@harvest/pi-utils";
import { extractSymbolsFromContent } from "../src/core/harvest/code-index";
import { SearchCodeTool } from "../src/tools/search-code";

using root = TempDir.createSync("@harvest-index-bench-");
const fileCount = 500;
const filler = "// fixture padding\n".repeat(400);
for (let batch = 0; batch < fileCount; batch += 20) {
	await Promise.all(
		Array.from({ length: 20 }, (_, offset) =>
			Bun.write(
				root.join(`source-${batch + offset}.ts`),
				`export function RenderBenchmark${batch + offset}() {}\n${filler}`,
			),
		),
	);
}

async function measure(label: string, run: () => Promise<void>) {
	const start = performance.now();
	let scheduledCallbackMs = 0;
	const callback = Bun.sleep(0).then(() => {
		scheduledCallbackMs = performance.now() - start;
	});
	await run();
	const totalMs = performance.now() - start;
	await callback;
	console.log(
		JSON.stringify({
			label,
			fileCount,
			totalMs: +totalMs.toFixed(2),
			scheduledCallbackMs: +scheduledCallbackMs.toFixed(2),
		}),
	);
}

// Reproduce the previous cold-scan I/O pattern on the same fixture; this
// excludes terminal rendering and is deliberately not the full old tool.
await measure("synchronous cold-scan pattern", async () => {
	for (const entry of fs.readdirSync(root.path())) {
		if (!entry.endsWith(".ts")) continue;
		const file = path.join(root.path(), entry);
		fs.statSync(file);
		extractSymbolsFromContent(fs.readFileSync(file, "utf8"), entry);
	}
});
const tool = new SearchCodeTool({ cwd: root.path() });
await measure("async cold symbol search", async () => {
	await tool.execute("cold", { query: "RenderBenchmark", mode: "symbols" });
});
await measure("async unchanged symbol search", async () => {
	await tool.execute("warm", { query: "RenderBenchmark", mode: "symbols" });
});
