import {
	ensureCalibrated,
	getDerivedTimeoutMsSync,
	getExplicitSetting,
	loadCalibration,
} from "../core/harvest/laya-calibration";
import { getLayaClient } from "../core/harvest/laya-client";
import {
	DEFAULT_LAYA_URL,
	configureLayaLocally,
	isLayaSidecarRunning,
	type LayaSetupResult,
} from "../core/harvest/laya-service";
import type { SettingPath, Settings } from "../config/settings";
import { commandConsumed, errorMessage, parseSubcommand, usage } from "./helpers/parse";
import type { SlashCommandRuntime, SlashCommandSpec, TuiSlashCommandRuntime } from "./types";

type SettingsLike = Pick<Settings, "get" | "set" | "flush"> & Partial<Pick<Settings, "isConfigured">>;
type OutputFn = (text: string) => Promise<void> | void;

function readSetting<T>(settings: SettingsLike, path: SettingPath): T | undefined {
	try {
		return settings.get(path) as T;
	} catch {
		return undefined;
	}
}

function resolveBaseUrl(settings: SettingsLike): string {
	return readSetting<string>(settings, "laya.url") || DEFAULT_LAYA_URL;
}

function formatLayaStatusText(args: {
	enabled: boolean;
	connected: boolean;
	url: string;
	pruning?: boolean;
	pruningEffective?: boolean;
	subagent?: boolean;
	pruningRecommended?: boolean;
	subagentRecommended?: boolean;
	subagentTimeoutMs?: number;
	subagentTimeoutEffectiveMs?: number;
}): string {
	const pruningSource =
		args.pruning === undefined
			? `auto (effective: ${args.pruningEffective === false ? "off" : "on"})`
			: args.pruning
				? "on (explicit)"
				: "off (explicit)";
	const timeoutSource =
		args.subagentTimeoutMs === undefined
			? `auto (effective: ${args.subagentTimeoutEffectiveMs ?? 300}ms)`
			: `${args.subagentTimeoutMs}ms (explicit)`;
	const lines = [
		`Laya: ${args.enabled ? "on" : "off"} (${args.connected ? `connected to ${args.url}` : "sidecar offline"})`,
		`Gating: ${args.enabled ? (args.connected ? "local irreversibility checks (<30ms fast path, fail-closed)" : "fail-closed approvals until sidecar starts") : "disabled"}`,
		`Pruning: ${pruningSource}${args.pruningRecommended !== undefined ? ` (calibrated: ${args.pruningRecommended ? "recommended" : "not recommended on this hardware"})` : ""}`,
		`Subagent routing: ${args.subagent === undefined ? "default" : args.subagent ? "on" : "off"}${args.subagentRecommended !== undefined ? ` (calibrated: ${args.subagentRecommended ? "recommended" : "not recommended"})` : ""}, timeout ${timeoutSource}`,
	];
	return lines.join("\n");
}

async function buildStatusText(settings: SettingsLike): Promise<string> {
	const enabled = readSetting<boolean>(settings, "laya.enabled") === true;
	const url = resolveBaseUrl(settings);
	const pruning = getExplicitSetting<boolean>(settings, "laya.pruning");
	const subagent = readSetting<boolean>(settings, "laya.subagentSelection");
	const timeout = getExplicitSetting<number>(settings, "laya.subagentSelectionTimeoutMs");
	let connected = false;
	try {
		connected = await isLayaSidecarRunning(url);
	} catch {
		connected = false;
	}
	let pruningRecommended: boolean | undefined;
	let subagentRecommended: boolean | undefined;
	try {
		const cal = await loadCalibration();
		pruningRecommended = cal?.derivedSettings.pruningRecommendEnabled;
		subagentRecommended = cal?.derivedSettings.subagentSelectionRecommendEnabled;
	} catch {
		// Calibration cache is optional; status still reports live settings.
	}
	const pruningEffective = pruning ?? pruningRecommended ?? true;
	const timeoutEffective = timeout ?? getDerivedTimeoutMsSync(300);
	return formatLayaStatusText({
		enabled,
		connected,
		url,
		pruning,
		pruningEffective,
		subagent,
		pruningRecommended,
		subagentRecommended,
		subagentTimeoutMs: timeout,
		subagentTimeoutEffectiveMs: timeoutEffective,
	});
}

function summarizeSetupResult(result: LayaSetupResult): string {
	if (result.success) {
		const where = result.effectiveUrl ?? DEFAULT_LAYA_URL;
		const tier = result.calibration ? ` (${result.calibration.hardware.tier}, calibrated)` : "";
		const smoke = result.smokeTest ? ` Smoke ${result.smokeTest.latencyMs}ms.` : "";
		const fast = result.idempotentFastPath ? " Already set up; verified existing sidecar." : "";
		return `Laya on and connected to ${where}${tier}.${smoke}${fast}`;
	}
	const detail = result.error ?? "unknown error";
	const bundle = result.diagnosticBundlePath ? ` Details: ${result.diagnosticBundlePath}.` : "";
	return `Laya setup failed: ${detail}.${bundle} Core agent still works; Laya disabled.`;
}

/** Guard against parallel long setups from repeated /laya invocations. */
let setupInFlight: Promise<LayaSetupResult> | null = null;

async function runSetupAndConnect(
	settings: SettingsLike,
	output: OutputFn,
	options: { forceReinstall?: boolean } = {},
): Promise<LayaSetupResult> {
	if (setupInFlight) {
		await output("Laya setup already in progress; status via /laya status.");
		return setupInFlight;
	}
	const baseUrl = resolveBaseUrl(settings);
	const task: Promise<LayaSetupResult> = (async () => {
		try {
			return await configureLayaLocally({
				settings: settings as Settings,
				baseUrl,
				forceReinstall: options.forceReinstall,
				onStepUpdate: (stepId, status, message) => {
					if (status === "error") {
						void output(`Laya setup [${stepId}] failed: ${message ?? "unknown error"}`);
					}
				},
			});
		} finally {
			setupInFlight = null;
		}
	})();
	setupInFlight = task;
	const result = await task;
	await output(summarizeSetupResult(result));
	return result;
}

async function runCalibrateAndReport(settings: SettingsLike, output: OutputFn): Promise<void> {
	const setup = await runSetupAndConnect(settings, output);
	if (!setup.success) return;
	const effectiveUrl = setup.effectiveUrl ?? resolveBaseUrl(settings);
	try {
		const maxTurnLatency = readSetting<number>(settings, "laya.maxAcceptableLatencyPerTurnMs");
		const record = await ensureCalibrated(getLayaClient(effectiveUrl), {
			force: true,
			...(maxTurnLatency !== undefined ? { maxAcceptableLatencyPerTurnMs: maxTurnLatency } : {}),
		});
		await output(
			`Laya calibrated for ${record.hardware.tier}: subagent timeout ${record.derivedSettings.subagentSelectionTimeoutMs}ms, pruning ${record.derivedSettings.pruningRecommendEnabled ? "recommended" : "not recommended"}.`,
		);
	} catch (err) {
		await output(`Laya recalibration failed: ${errorMessage(err)}`);
	}
}

function parseSetupFlags(rest: string): { forceReinstall: boolean } {
	const tokens = rest.toLowerCase().split(/\s+/).filter(Boolean);
	return { forceReinstall: tokens.includes("--reinstall") || tokens.includes("--force") };
}

async function persistEnabled(settings: SettingsLike, enabled: boolean): Promise<string | null> {
	try {
		settings.set("laya.enabled", enabled);
		await settings.flush();
		return null;
	} catch (err) {
		return `Failed to ${enabled ? "enable" : "disable"} Laya: ${errorMessage(err)}`;
	}
}

async function handleLayaArgs(args: string, runtime: SlashCommandRuntime): Promise<void> {
	const { verb, rest } = parseSubcommand(args);
	switch (verb) {
		case "":
		case "status": {
			await runtime.output(await buildStatusText(runtime.settings));
			return;
		}
		case "on": {
			const failure = await persistEnabled(runtime.settings, true);
			if (failure) {
				await runtime.output(failure);
				return;
			}
			if (runtime.runCommandInBackground) {
				await runtime.output("Enabling Laya and setting up the sidecar from your config in the background…");
				runtime.runCommandInBackground(() => {
					return (async () => {
						await runSetupAndConnect(runtime.settings, runtime.output);
					})();
				});
				return;
			}
			await runtime.output("Enabling Laya and setting up the sidecar from your config…");
			await runSetupAndConnect(runtime.settings, runtime.output);
			return;
		}
		case "off": {
			const failure = await persistEnabled(runtime.settings, false);
			await runtime.output(
				failure ?? "Laya disabled. Tool gating, routing, and pruning fall back to built-in behavior.",
			);
			return;
		}
		case "setup": {
			const failure = await persistEnabled(runtime.settings, true);
			if (failure) {
				await runtime.output(failure);
				return;
			}
			const flags = parseSetupFlags(rest);
			if (runtime.runCommandInBackground) {
				await runtime.output("Starting Laya setup from your config in the background; status via /laya status.");
				runtime.runCommandInBackground(() => {
					return (async () => {
						await runSetupAndConnect(runtime.settings, runtime.output, flags);
					})();
				});
				return;
			}
			await runtime.output("Starting Laya setup from your config…");
			await runSetupAndConnect(runtime.settings, runtime.output, flags);
			return;
		}
		case "calibrate": {
			if (runtime.runCommandInBackground) {
				await runtime.output("Recalibrating Laya in the background; status via /laya status.");
				runtime.runCommandInBackground(() => {
					return (async () => {
						await runCalibrateAndReport(runtime.settings, runtime.output);
					})();
				});
				return;
			}
			await runCalibrateAndReport(runtime.settings, runtime.output);
			return;
		}
		case "pruning":
		case "subagent": {
			const toggle = rest.trim().toLowerCase();
			const key: SettingPath = verb === "pruning" ? "laya.pruning" : "laya.subagentSelection";
			if (toggle !== "on" && toggle !== "off") {
				await usage(`Usage: /laya ${verb} [on|off]`, runtime);
				return;
			}
			try {
				runtime.settings.set(key, toggle === "on");
				await runtime.settings.flush();
				await runtime.output(`Laya ${verb} ${toggle === "on" ? "enabled" : "disabled"}.`);
			} catch (err) {
				await usage(`Failed to update ${key}: ${errorMessage(err)}`, runtime);
			}
			return;
		}
		default:
			await usage("Usage: /laya [on|off|status|setup|calibrate|pruning on|off|subagent on|off]", runtime);
	}
}

function refreshTuiStatusLine(ctx: TuiSlashCommandRuntime["ctx"]): void {
	try {
		ctx.statusLine.invalidate();
		ctx.ui.requestRender();
	} catch {
		// Status-line refresh is best-effort; the command result already reported.
	}
}

export const BUILTIN_LAYA_SLASH_COMMANDS: ReadonlyArray<SlashCommandSpec> = [
	{
		name: "laya",
		icon: "gauge",
		description: "Toggle the local Laya decision layer (fast gating, routing, pruning)",
		acpDescription: "Toggle the local Laya decision layer",
		acpInputHint: "[on|off|status|setup|calibrate]",
		subcommands: [
			{ name: "on", description: "Enable Laya: set up sidecar from your config if needed and connect" },
			{ name: "off", description: "Disable Laya (fall back to built-in behavior)" },
			{ name: "status", description: "Show Laya connection and calibration status" },
			{ name: "setup", description: "Install and start the local sidecar", usage: "[--reinstall]" },
			{ name: "calibrate", description: "Re-benchmark hardware timeouts" },
			{ name: "pruning", description: "Toggle context pruning", usage: "[on|off]" },
			{ name: "subagent", description: "Toggle subagent routing", usage: "[on|off]" },
		],
		allowArgs: true,
		getTuiAutocompleteDescription: runtime => {
			try {
				const enabled = runtime.ctx.settings.get("laya.enabled") === true;
				return enabled ? "Laya: on" : "Laya: off";
			} catch {
				return "Laya: status unknown";
			}
		},
		handle: async (command, runtime) => {
			await handleLayaArgs(command.args, runtime);
			return commandConsumed();
		},
		handleTui: async (command, runtime) => {
			const { verb } = parseSubcommand(command.args);
			const longRunning = verb === "on" || verb === "setup" || verb === "calibrate";
			if (!longRunning) {
				const adapted: SlashCommandRuntime = {
					session: runtime.ctx.session,
					sessionManager: runtime.ctx.sessionManager,
					settings: runtime.ctx.settings,
					cwd: runtime.ctx.sessionManager.getCwd(),
					output: (text: string) => {
						runtime.ctx.showStatus(text);
					},
					refreshCommands: () => runtime.ctx.refreshSlashCommandState(),
					reloadPlugins: async () => {},
				};
				await handleLayaArgs(command.args, adapted);
				refreshTuiStatusLine(runtime.ctx);
				runtime.ctx.editor.setText("");
				return;
			}
			const ctx = runtime.ctx;
			const settings = ctx.settings;
			ctx.showStatus(
				verb === "calibrate"
					? "Recalibrating Laya in the background…"
					: "Enabling Laya and setting up the sidecar from your config in the background…",
			);
			ctx.editor.setText("");
			void (async () => {
				const output: OutputFn = text => {
					ctx.showStatus(text);
				};
				try {
					if (verb === "calibrate") {
						await runCalibrateAndReport(settings, output);
					} else if (verb === "setup") {
						const { rest } = parseSubcommand(command.args);
						const failure = await persistEnabled(settings, true);
						if (failure) {
							output(failure);
							return;
						}
						await runSetupAndConnect(settings, output, parseSetupFlags(rest));
					} else {
						const failure = await persistEnabled(settings, true);
						if (failure) {
							output(failure);
							return;
						}
						await runSetupAndConnect(settings, output);
					}
				} catch (err) {
					output(`Laya setup failed: ${errorMessage(err)}`);
				} finally {
					refreshTuiStatusLine(ctx);
				}
			})();
		},
	},
];
