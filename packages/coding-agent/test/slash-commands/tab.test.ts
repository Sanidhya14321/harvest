import { expect, it, vi } from "bun:test";
import type { InteractiveModeContext } from "@harvest/pi-coding-agent/modes/types";
import { executeBuiltinSlashCommand } from "@harvest/pi-coding-agent/slash-commands/builtin-registry";

it("routes /tab next to the session tab manager and displays its result", async () => {
	const handleSessionTabsCommand = vi.fn(async () => "Switched to session tab 2.");
	const showSessionInfo = vi.fn();
	const setText = vi.fn();
	const runtime = {
		ctx: {
			collabGuest: false,
			handleSessionTabsCommand,
			showSessionInfo,
			editor: { setText },
		} as unknown as InteractiveModeContext,
	};
	expect(await executeBuiltinSlashCommand("/tab next", runtime)).toBe(true);
	expect(handleSessionTabsCommand).toHaveBeenCalledWith("next");
	expect(showSessionInfo).toHaveBeenCalledWith("Switched to session tab 2.");
});
