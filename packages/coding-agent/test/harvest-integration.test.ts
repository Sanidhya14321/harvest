import { describe, expect, it } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { SearchCodeTool } from "../src/tools/search-code";
import { BUILTIN_TOOLS } from "../src/tools/index";
import { BUILTIN_TOOL_NAMES } from "../src/tools/builtin-names";
import { buildSystemPrompt } from "../src/system-prompt";
import { SecuritySandbox } from "../src/core/harvest/security";

describe("Harvest End-to-End Integration Contracts", () => {
	it("registers search_code in builtin names and factories", () => {
		expect(BUILTIN_TOOL_NAMES.includes("search_code" as any)).toBe(true);
		expect(typeof BUILTIN_TOOLS.search_code).toBe("function");
	});

	it("executes search_code tool across symbols and docs", async () => {
		const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "harvest-search-test-"));
		try {
			const srcFile = path.join(tempDir, "service.ts");
			fs.writeFileSync(
				srcFile,
				"export function processTransactionPayload(amount: number) {\n\treturn amount * 2;\n}\n",
			);
			const mockSession = { cwd: tempDir } as any;
			const tool = new SearchCodeTool(mockSession);
			const result = await tool.execute("call-1", { query: "processTransaction", mode: "symbols" });
			expect(result.details?.totalMatches).toBeGreaterThan(0);
			const text = result.content[0]?.type === "text" ? result.content[0].text : "";
			expect(text).toContain("processTransactionPayload");
		} finally {
			fs.rmSync(tempDir, { recursive: true, force: true });
		}
	});

	it("injects Grounded Coding Task Contract into system prompt", async () => {
		const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "harvest-prompt-test-"));
		try {
			const promptResult = await buildSystemPrompt({
				cwd: tempDir,
				tools: new Map(),
			});
			const fullPrompt = promptResult.systemPrompt.join("\n\n");
			expect(fullPrompt).toContain("<coding-contract>");
			expect(fullPrompt).toContain("Decompose the request into small checkpoints");
			expect(fullPrompt).toContain("Finish only when every requested behavior is implemented");
		} finally {
			fs.rmSync(tempDir, { recursive: true, force: true });
		}
	});

	it("SecuritySandbox blocks destructive bash commands and path jail escapes", () => {
		const sandbox = new SecuritySandbox(process.cwd());
		expect(sandbox.checkCommand("rm -rf /").allowed).toBe(false);
		expect(sandbox.checkCommand("rm -rf *").allowed).toBe(false);
		expect(sandbox.checkCommand("mkfs.ext4 /dev/sda1").allowed).toBe(false);
		expect(sandbox.checkCommand(":(){ :|:& };:").allowed).toBe(false);
		expect(sandbox.assertPathJailed("../../../../../windows/system32").jailed).toBe(false);
	});
});
