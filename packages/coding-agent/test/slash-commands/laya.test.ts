import { afterEach, describe, expect, it, vi } from "bun:test";
import { executeAcpBuiltinSlashCommand } from "@harvest/pi-coding-agent/slash-commands/acp-builtins";
import * as layaService from "@harvest/pi-coding-agent/core/harvest/laya-service";
import * as layaCalibration from "@harvest/pi-coding-agent/core/harvest/laya-calibration";
import * as layaClient from "@harvest/pi-coding-agent/core/harvest/laya-client";

function layaRuntime(initial: Record<string, unknown> = {}) {
	const store: Record<string, unknown> = {
		"laya.enabled": false,
		"laya.url": "http://127.0.0.1:8177",
		"laya.pruning": true,
		"laya.subagentSelection": false,
		"laya.subagentSelectionTimeoutMs": 300,
		...initial,
	};
	const get = vi.fn((path: string) => store[path]);
	const set = vi.fn((path: string, value: unknown) => {
		store[path] = value;
	});
	const flush = vi.fn(async () => {});
	const output = vi.fn();
	const runtime = {
		session: {},
		sessionManager: { getCwd: () => process.cwd() },
		settings: { get, set, flush },
		cwd: process.cwd(),
		output,
		refreshCommands: vi.fn(),
		reloadPlugins: vi.fn(async () => {}),
	};
	return { output, runtime, set, store };
}

function mockSetupSuccess(effectiveUrl = "http://127.0.0.1:8177") {
	return vi.spyOn(layaService, "configureLayaLocally").mockResolvedValue({
		success: true,
		coreHarvestReady: true,
		effectiveUrl,
		calibration: {
			hardware: { tier: "cpu", device_name: "cpu", signature: "sig" },
			derivedSettings: {},
		},
		smokeTest: { success: true, healthOk: true, decideOk: true, latencyMs: 12 },
	} as never);
}

describe("/laya slash command", () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("reports status without mutating settings or provisioning", async () => {
		vi.spyOn(layaService, "isLayaSidecarRunning").mockResolvedValue(false);
		vi.spyOn(layaCalibration, "loadCalibration").mockResolvedValue(null);
		const configure = vi.spyOn(layaService, "configureLayaLocally");
		const h = layaRuntime({ "laya.enabled": true });
		const result = await executeAcpBuiltinSlashCommand("/laya status", h.runtime as never);
		expect(result).toEqual({ consumed: true });
		expect(h.set).not.toHaveBeenCalled();
		expect(configure).not.toHaveBeenCalled();
		expect(String(h.output.mock.calls[0]?.[0])).toContain("Laya: on (sidecar offline)");
	});

	it("provisions from the user's configured url and connects on /laya on", async () => {
		vi.spyOn(layaCalibration, "loadCalibration").mockResolvedValue(null);
		const configure = mockSetupSuccess("http://127.0.0.1:9999");
		const h = layaRuntime({ "laya.enabled": false, "laya.url": "http://127.0.0.1:9999" });
		await executeAcpBuiltinSlashCommand("/laya on", h.runtime as never);
		expect(h.set).toHaveBeenCalledWith("laya.enabled", true);
		expect(configure).toHaveBeenCalledTimes(1);
		expect(configure.mock.calls[0]?.[0]).toMatchObject({
			baseUrl: "http://127.0.0.1:9999",
		});
		expect(String(h.output.mock.calls.at(-1)?.[0])).toContain("http://127.0.0.1:9999");
	});

	it("surfaces setup failure instead of claiming connected", async () => {
		vi.spyOn(layaService, "configureLayaLocally").mockResolvedValue({
			success: false,
			coreHarvestReady: true,
			error: "python missing",
			diagnosticBundlePath: "/tmp/bundle.json",
		} as never);
		const h = layaRuntime({ "laya.enabled": false });
		await executeAcpBuiltinSlashCommand("/laya on", h.runtime as never);
		expect(h.set).toHaveBeenCalledWith("laya.enabled", true);
		expect(String(h.output.mock.calls.at(-1)?.[0])).toContain("Laya setup failed: python missing");
	});

	it("forwards --reinstall on explicit setup", async () => {
		const configure = mockSetupSuccess();
		const h = layaRuntime();
		await executeAcpBuiltinSlashCommand("/laya setup --reinstall", h.runtime as never);
		expect(configure).toHaveBeenCalledTimes(1);
		expect(configure.mock.calls[0]?.[0]).toMatchObject({ forceReinstall: true });
	});

	it("recalibrates hardware timeouts from the connected sidecar", async () => {
		mockSetupSuccess("http://127.0.0.1:8177");
		const client = { decide: vi.fn() };
		vi.spyOn(layaClient, "getLayaClient").mockReturnValue(client as never);
		const ensure = vi.spyOn(layaCalibration, "ensureCalibrated").mockResolvedValue({
			hardware: { tier: "cpu" },
			derivedSettings: { subagentSelectionTimeoutMs: 6231, pruningRecommendEnabled: false },
		} as never);
		const h = layaRuntime();
		await executeAcpBuiltinSlashCommand("/laya calibrate", h.runtime as never);
		expect(ensure).toHaveBeenCalledTimes(1);
		expect(ensure.mock.calls[0]?.[1]).toMatchObject({ force: true });
		expect(String(h.output.mock.calls.at(-1)?.[0])).toContain("6231ms");
	});

	it("turns off by persisting laya.enabled=false without provisioning", async () => {
		const configure = vi.spyOn(layaService, "configureLayaLocally");
		const h = layaRuntime({ "laya.enabled": true });
		await executeAcpBuiltinSlashCommand("/laya off", h.runtime as never);
		expect(h.set).toHaveBeenCalledWith("laya.enabled", false);
		expect(configure).not.toHaveBeenCalled();
		expect(String(h.output.mock.calls[0]?.[0])).toContain("Laya disabled");
	});

	it("toggles pruning and subagent flags explicitly", async () => {
		const h = layaRuntime();
		await executeAcpBuiltinSlashCommand("/laya pruning off", h.runtime as never);
		expect(h.set).toHaveBeenCalledWith("laya.pruning", false);
		await executeAcpBuiltinSlashCommand("/laya subagent on", h.runtime as never);
		expect(h.set).toHaveBeenCalledWith("laya.subagentSelection", true);
	});

	it("rejects unknown subcommands with usage", async () => {
		const h = layaRuntime();
		await executeAcpBuiltinSlashCommand("/laya bogus", h.runtime as never);
		expect(h.set).not.toHaveBeenCalled();
		expect(String(h.output.mock.calls[0]?.[0])).toContain("Usage: /laya");
	});
});
