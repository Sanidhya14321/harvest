import { describe, expect, it, vi } from "bun:test";
import { SelectorController } from "@harvest/pi-coding-agent/modes/controllers/selector-controller";
import type { InteractiveModeContext } from "@harvest/pi-coding-agent/modes/types";

describe("SelectorController prompt-affecting settings", () => {
	it("refreshes the active prompt when xdev docs mode changes", async () => {
		const refreshBaseSystemPrompt = vi.fn(async () => {});
		const ctx = {
			sessionManager: { getSessionFile: () => undefined },
			session: { refreshBaseSystemPrompt },
			showError: vi.fn(),
		} as unknown as InteractiveModeContext;
		const controller = new SelectorController(ctx);

		controller.handleSettingChange("tools.xdevDocs", "catalog");
		await Promise.resolve();

		expect(refreshBaseSystemPrompt).toHaveBeenCalledTimes(1);
		expect(ctx.showError).not.toHaveBeenCalled();
	});
});
