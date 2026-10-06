import { describe, expect, it } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { SETTINGS_SCHEMA } from "../src/config/settings-schema";
import { MarkdownBrain } from "../src/core/harvest/brain";
import { executeBuiltinSlashCommand, lookupBuiltinSlashCommand } from "../src/slash-commands/builtin-registry";
import { discoverAgents } from "../src/task/discovery";
import { createInteractiveModeContext } from "./helpers/interactive-mode-context";

describe("Laya removal contracts", () => {
	it("keeps laya.* keys parsable with defaults but renders no settings UI", () => {
		for (const key of [
			"laya.enabled",
			"laya.url",
			"laya.autostart",
			"laya.pruning",
			"laya.subagentSelection",
		] as const) {
			const entry = SETTINGS_SCHEMA[key] as { default: unknown; ui?: unknown };
			expect(entry).toBeDefined();
			expect(entry.ui).toBeUndefined();
		}
		expect((SETTINGS_SCHEMA["laya.enabled"] as { default: unknown }).default).toBe(false);
		expect((SETTINGS_SCHEMA["brain.rerank"] as { default: unknown }).default).toBe(false);
	});

	it("brain retrieval ignores the removed rerank flag and keeps lexical order", async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-no-laya-"));
		try {
			await Bun.write(path.join(root, "a.md"), "## Database\nDatabase database database transactions.");
			await Bun.write(path.join(root, "b.md"), "## Database\nDatabase backups restore recovery.");
			const brain = new MarkdownBrain([{ scope: "project", directory: root }]);
			const lexical = await brain.retrieve("database");
			expect(lexical.length).toBeGreaterThan(0);
			expect((await brain.retrieve("database", { rerank: true })).map(page => page.id)).toEqual(
				lexical.map(page => page.id),
			);
		} finally {
			await fs.rm(root, { recursive: true, force: true });
		}
	});

	it("answers /laya with a removal notice instead of unknown-command silence", async () => {
		expect(lookupBuiltinSlashCommand("laya")).toBeUndefined();
		const ctx = createInteractiveModeContext();
		const shown: string[] = [];
		(ctx.showStatus as unknown as (message: string) => void) = (message: string) => {
			shown.push(message);
		};
		const result = await executeBuiltinSlashCommand("/laya status", { ctx });
		expect(result).toBe(true);
		expect(shown.some(message => /removed/i.test(message))).toBe(true);
	});
});

describe("native agent discovery (.harvest source)", () => {
	it("discovers project .harvest agents saved by the hub flow", async () => {
		const root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-discover-"));
		try {
			const dir = path.join(root, ".harvest", "agents");
			await fs.mkdir(dir, { recursive: true });
			await Bun.write(
				path.join(dir, "api-docs-writer.md"),
				"---\nname: api-docs-writer\ndescription: Use this agent when writing API docs.\n---\n\nWrite docs.\n",
			);
			const { agents } = await discoverAgents(root, os.tmpdir());
			expect(agents.some(agent => agent.name === "api-docs-writer")).toBe(true);
		} finally {
			await fs.rm(root, { recursive: true, force: true });
		}
	});
});
