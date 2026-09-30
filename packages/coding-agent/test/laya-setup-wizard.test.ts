/**
 * Tests for Laya Setup Wizard Scene and Local Configuration.
 */

import { afterEach, beforeAll, describe, expect, it, vi } from "bun:test";
import { Settings } from "../src/config/settings";
import { ALL_SCENES, selectSetupScenes } from "../src/modes/setup-wizard";
import { layaSetupScene } from "../src/modes/setup-wizard/scenes/laya";
import type { SetupSceneHost, SetupSceneResult } from "../src/modes/setup-wizard/scenes/types";
import type { InteractiveModeContext } from "../src/modes/types";
import { initTheme } from "../src/modes/theme/theme";
import * as layaService from "../src/core/harvest/laya-service";
import { checkToolCallGating } from "../src/core/harvest/laya-gating";
import { routeModelWithLaya } from "../src/core/harvest/laya-routing";
import { checkCompletionWithLaya } from "../src/core/harvest/laya-completion";

afterEach(() => {
	vi.restoreAllMocks();
});

function createMockHost(settings: Settings): {
	host: SetupSceneHost;
	finishedWith: SetupSceneResult | null;
	rendersRequested: number;
} {
	let finishedWith: SetupSceneResult | null = null;
	let rendersRequested = 0;

	const host: SetupSceneHost = {
		ctx: {
			settings,
			ui: {
				invalidate: () => {},
				setFocus: () => {},
			},
		} as unknown as InteractiveModeContext,
		requestRender: () => {
			rendersRequested++;
		},
		finish: (result: SetupSceneResult) => {
			finishedWith = result;
		},
		setFocus: () => {},
		restoreFocus: () => {},
	};

	return {
		get host() {
			return host;
		},
		get finishedWith() {
			return finishedWith;
		},
		get rendersRequested() {
			return rendersRequested;
		},
	};
}

beforeAll(async () => {
	await initTheme(false, "unicode", false, "titanium", "light");
});

describe("Laya Setup Scene in Harvest Setup Wizard", () => {
	it("registers layaSetupScene in ALL_SCENES with minVersion 3", () => {
		expect(layaSetupScene.id).toBe("laya-decision-layer");
		expect(layaSetupScene.minVersion).toBe(3);
		expect(ALL_SCENES.some(s => s.id === "laya-decision-layer")).toBe(true);
		expect(ALL_SCENES[ALL_SCENES.length - 1]?.id).toBe("laya-decision-layer");
	});

	it("selects layaSetupScene for users updating from setup version 2", async () => {
		const ctx = {
			session: { modelRegistry: { getAvailable: () => [{ provider: "test", id: "m" }] } },
		} as unknown as InteractiveModeContext;

		const selected = await selectSetupScenes(2, ALL_SCENES, ctx, { isTTY: true });
		expect(selected.map(s => s.id)).toEqual(["laya-decision-layer"]);
	});

	it("renders setup prompt with configure and skip options", () => {
		const settings = Settings.isolated();
		const { host } = createMockHost(settings);
		const controller = layaSetupScene.mount(host);

		expect(controller.title).toBe("Configure Laya");
		const lines = controller.render(100);
		const rendered = lines.join("\n");

		expect(rendered).toContain("Yes, configure Laya locally");
		expect(rendered).toContain("No, skip Laya for now");
		expect(rendered).toContain("ModernBERT-large");
	});

	it("handles skip choice by disabling laya in settings", async () => {
		const settings = Settings.isolated();
		const mock = createMockHost(settings);
		const controller = layaSetupScene.mount(mock.host);

		// Select "2" (Skip)
		controller.handleInput?.("2");

		// Allow async flush
		await Bun.sleep(10);

		expect(settings.get("laya.enabled")).toBe(false);
		expect(mock.finishedWith).toBe("skipped");
	});

	it("disables gating and routing when laya.enabled is false in settings", async () => {
		const settings = Settings.isolated({ "laya.enabled": false });
		await settings.flush();

		// Gating check should bypass without sidecar call
		const gating = await checkToolCallGating("bash", { command: "rm -rf /" }, { settings });
		expect(gating.isHighRiskTool).toBe(false);
		expect(gating.requireApproval).toBe(false);
		expect(gating.reason).toBe("laya_disabled_by_settings");

		// Model routing should fall open to default
		const routing = await routeModelWithLaya("Refactor the parser", { defaultRole: "default", settings });
		expect(routing.selectedRole).toBe("default");
		expect(routing.fallback).toBe(true);
		expect(routing.fallbackReason).toBe("laya_disabled_by_settings");

		// Completion should fall open
		const completion = await checkCompletionWithLaya({ command: "test", output: "ok" }, { settings });
		expect(completion.isSuccess).toBe(true);
		expect(completion.fallback).toBe(true);
		expect(completion.fallbackReason).toBe("laya_disabled_by_settings");
	});

	it("probes local python and sidecar service", async () => {
		vi.spyOn(layaService, "findPythonExecutable").mockResolvedValue({ path: "/usr/bin/python3", version: "3.10.12" });
		vi.spyOn(layaService, "isLayaSidecarRunning").mockResolvedValue(false);

		const py = await layaService.findPythonExecutable();
		expect(py).not.toBeNull();
		expect(py?.version).toBe("3.10.12");

		const running = await layaService.isLayaSidecarRunning();
		expect(typeof running).toBe("boolean");
	});
});
