import { afterEach, describe, expect, it } from "bun:test";
import { TempDir } from "@harvest/pi-utils";
// Import the command registry first: it establishes the module evaluation
// order that avoids a builtin-control/builtin-lifecycle init cycle.
import "@harvest/pi-coding-agent/slash-commands/builtin-registry";
import { relocateHeadlessSession } from "../src/slash-commands/builtin-lifecycle";
import { SessionManager } from "../src/session/session-manager";
import type { SlashCommandRuntime } from "../src/slash-commands/types";

/**
 * T17/U4: failed headless relocation keeps the prior usable workspace.
 * Exercises the exported `relocateHeadlessSession` seam with a failing
 * move: the command reports the failure and the session manager stays on
 * the original cwd (no partial retarget).
 */
describe("relocateHeadlessSession failure keeps workspace", () => {
	const dirs: TempDir[] = [];
	afterEach(async () => {
		for (const dir of dirs.splice(0)) await dir.remove().catch(() => undefined);
	});

	it("a throwing moveSession reports usage and preserves the original cwd", async () => {
		const directory = TempDir.createSync("@harvest-relocate-");
		dirs.push(directory);
		const elsewhere = TempDir.createSync("@harvest-relocate-target-");
		dirs.push(elsewhere);
		const manager = SessionManager.create(directory.path(), directory.path());
		const outputs: string[] = [];
		const runtime = {
			settings: { flush: async () => {} },
			sessionManager: manager,
			session: {
				moveSession: async () => {
					throw new Error("move unavailable in fallback mode");
				},
			},
			output: async (text: string) => {
				outputs.push(text);
			},
		} as unknown as SlashCommandRuntime;
		const before = manager.getCwd();
		const result = await relocateHeadlessSession(runtime, elsewhere.path());
		expect(result).toEqual({ consumed: true });
		expect(outputs.join("\n")).toMatch(/Move failed/);
		expect(manager.getCwd()).toBe(before);
	});
});
