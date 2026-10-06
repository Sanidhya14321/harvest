import { describe, expect, it } from "bun:test";
import {
	describeAutoLearnState,
	formatAutoLearnStatus,
	type AutoLearnToolsPresent,
} from "@harvest/pi-coding-agent/autolearn/controller";
import { Settings } from "@harvest/pi-coding-agent/config/settings";

const PRESENT: AutoLearnToolsPresent = { manageSkill: true, learn: true };
const ABSENT: AutoLearnToolsPresent = { manageSkill: false, learn: false };

function settings(enabled: boolean, autoContinue: boolean): Settings {
	return Settings.isolated({ "autolearn.enabled": enabled, "autolearn.autoContinue": autoContinue });
}

describe("auto-learn configured vs effective state", () => {
	it("is disabled when the flag is off, regardless of tools or controller", () => {
		const state = describeAutoLearnState(settings(false, true), PRESENT, true);
		expect(state.configuredEnabled).toBe(false);
		expect(state.effective).toBe("disabled");
	});

	it("is disabled when the tools never made the active set", () => {
		const state = describeAutoLearnState(settings(true, true), ABSENT, true);
		expect(state.effective).toBe("disabled");
		expect(state.reason).toMatch(/not in the active set/);
	});

	it("is guidance-only without the controller subscription", () => {
		const state = describeAutoLearnState(settings(true, true), PRESENT, false);
		expect(state.effective).toBe("guidance-only");
	});

	it("is guidance-only when autoContinue is off", () => {
		const state = describeAutoLearnState(settings(true, false), PRESENT, true);
		expect(state.configuredAutoContinue).toBe(false);
		expect(state.effective).toBe("guidance-only");
	});

	it("captures only when every opt-in is present", () => {
		const state = describeAutoLearnState(settings(true, true), PRESENT, true);
		expect(state.effective).toBe("capturing");
	});

	it("never silently activates: effective requires explicit flags", () => {
		// Controller installed + tools present but flag off → disabled.
		expect(describeAutoLearnState(settings(false, false), PRESENT, true).effective).toBe("disabled");
		// Flag on but autoContinue off → guidance only, never capturing.
		expect(describeAutoLearnState(settings(true, false), PRESENT, true).effective).not.toBe("capturing");
	});

	it("formats configured vs effective for live settings displays", () => {
		const text = formatAutoLearnStatus(describeAutoLearnState(settings(true, false), PRESENT, true));
		expect(text).toContain("configured: on");
		expect(text).toContain("effective: guidance-only");
		expect(text).toContain("manage_skill");
		const off = formatAutoLearnStatus(describeAutoLearnState(settings(false, false), ABSENT, false));
		expect(off).toContain("configured: off");
		expect(off).toContain("effective: disabled");
	});
});
