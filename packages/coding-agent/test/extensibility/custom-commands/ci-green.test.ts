import { afterEach, describe, expect, it, vi } from "bun:test";
import { type } from "@harvest/omptype";
import type * as TypeBox from "@harvest/omptype/typebox";
import * as zod from "@harvest/omptype/zod";
import * as piCodingAgent from "@harvest/pi-coding-agent";
import { GreenCommand } from "@harvest/pi-coding-agent/extensibility/custom-commands/bundled/ci-green";
import type { CustomCommandAPI } from "@harvest/pi-coding-agent/extensibility/custom-commands/types";
import type { HookCommandContext } from "@harvest/pi-coding-agent/extensibility/hooks/types";
import type { VcsGitRepo } from "@harvest/pi-natives";
import * as vcs from "@harvest/pi-natives/vcs";

afterEach(() => {
	vi.restoreAllMocks();
});

function createApi(): CustomCommandAPI {
	return {
		cwd: "/tmp/test",
		exec: async () => ({
			stdout: "",
			stderr: "",
			code: 0,
			killed: false,
		}),
		typebox: {} as unknown as typeof TypeBox,
		arktype: Object.assign(Function.prototype.bind.call(type, undefined) as typeof type, type, { type }),
		zod,
		pi: piCodingAgent,
	};
}

describe("GreenCommand", () => {
	it("includes tag instructions when HEAD has a tag", async () => {
		vi.spyOn(vcs, "requireGit").mockReturnValue({
			tagsAt: async () => ["v0.1.0-alpha2"],
		} as unknown as VcsGitRepo);
		const command = new GreenCommand(createApi());

		const result = await command.execute([], {} as HookCommandContext);

		expect(result).toContain("v0.1.0-alpha2");
		expect(result).not.toContain("timeouts due to the harnesses");
	});

	it("omits tag instructions when HEAD is not tagged", async () => {
		vi.spyOn(vcs, "requireGit").mockReturnValue({
			tagsAt: async () => [],
		} as unknown as VcsGitRepo);
		const command = new GreenCommand(createApi());

		const result = await command.execute([], {} as HookCommandContext);

		expect(result).not.toContain("v0.1.0-alpha2");
	});
});
