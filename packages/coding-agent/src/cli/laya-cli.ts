/**
 * CLI Handler for `harvest laya [calibrate|status]`.
 *
 * Provides user-facing inspection and triggering of the hardware-aware
 * self-calibration benchmark, showing detected hardware, measured latencies,
 * derived settings, and user overrides with safety warnings.
 */

import chalk from "@harvest/pi-utils/chalk";
import { getAgentDir } from "@harvest/pi-utils";
import { settings } from "../config/settings";
import {
	DEFAULT_MAX_ACCEPTABLE_TURN_LATENCY_MS,
	ensureCalibrated,
	loadCalibration,
	runCalibrationBenchmark,
	saveCalibration,
	type CalibrationRecord,
} from "../core/harvest/laya-calibration";
import { getLayaClient } from "../core/harvest/laya-client";

export type LayaAction = "calibrate" | "status";

export interface LayaCommandArgs {
	action: LayaAction;
	flags: {
		force?: boolean;
		json?: boolean;
	};
}

function writeLine(text = ""): void {
	process.stdout.write(`${text}\n`);
}

function formatStatus(cal: CalibrationRecord | null, isOnline: boolean): void {
	writeLine(chalk.bold("Harvest Laya Decision System Status"));
	writeLine("=".repeat(60));

	if (!isOnline) {
		writeLine(`${chalk.red("● Sidecar Status:")} Offline / Unreachable`);
		writeLine(chalk.dim("  The local sidecar is not responding at its configured URL."));
		writeLine(chalk.dim("  Start it with: python decision-sidecar/server.py"));
	} else {
		writeLine(`${chalk.green("● Sidecar Status:")} Online & Ready`);
	}

	if (!cal) {
		writeLine(`\n${chalk.yellow("● Calibration:")} Not yet calibrated on this machine.`);
		writeLine(chalk.dim("  Run `harvest laya calibrate` to benchmark and derive optimal settings."));
		return;
	}

	const hw = cal.hardware;
	const derived = cal.derivedSettings;
	const b = cal.benchmarks;

	writeLine(`\n${chalk.cyan("Hardware Detection:")}`);
	writeLine(`  Tier:      ${chalk.bold(hw.tier.toUpperCase())}`);
	writeLine(`  Device:    ${hw.device_name}`);
	writeLine(`  Signature: ${chalk.dim(hw.signature)}`);
	writeLine(`  Details:   ${chalk.dim(hw.reason)}`);

	writeLine(`\n${chalk.cyan("Measured Latency Benchmarks:")}`);
	writeLine(`  Ultra-short (~40 tok):         ${b.ultraShort.medianMs.toFixed(1)}ms (min: ${b.ultraShort.minMs.toFixed(1)}ms)`);
	writeLine(`  Single Choice (~350 tok):      ${b.singleChoice.medianMs.toFixed(1)}ms (min: ${b.singleChoice.minMs.toFixed(1)}ms)`);
	writeLine(`  Single Score (~150 tok):       ${b.singleScore.medianMs.toFixed(1)}ms (min: ${b.singleScore.minMs.toFixed(1)}ms)`);
	writeLine(`  Batched Score (B=2, L=1024):   ${b.batchedScore.medianMs.toFixed(1)}ms (min: ${b.batchedScore.minMs.toFixed(1)}ms)`);

	writeLine(`\n${chalk.cyan("Derived Operational Settings:")}`);
	writeLine(
		`  Subagent Selection: ${
			derived.subagentSelectionRecommendEnabled ? chalk.green("RECOMMENDED") : chalk.yellow("NOT RECOMMENDED")
		} (${derived.subagentSelectionReason})`,
	);
	writeLine(`  Subagent Timeout:   ${chalk.bold(`${derived.subagentSelectionTimeoutMs}ms`)} (safety margin over measured latency)`);
	writeLine(
		`  Context Pruning:    ${
			derived.pruningRecommendEnabled ? chalk.green("RECOMMENDED") : chalk.yellow("NOT RECOMMENDED")
		} (${derived.pruningReason})`,
	);
	writeLine(`  Turn Latency Cap:   ${derived.maxAcceptableLatencyPerTurnMs}ms acceptable added time`);

	// Check explicit user overrides in settings
	writeLine(`\n${chalk.cyan("Active User Overrides & Configuration:")}`);
	let userPruning: boolean | undefined;
	let userSubagent: boolean | undefined;
	let userTimeout: number | undefined;

	try {
		userPruning = settings.get("laya.pruning");
		userSubagent = settings.get("laya.subagentSelection");
		userTimeout = settings.get("laya.subagentSelectionTimeoutMs");
	} catch {
		// Isolated environment
	}

	const pruningStatus =
		userPruning !== undefined
			? userPruning
				? chalk.bold.green("ENABLED (User Override)")
				: chalk.bold.red("DISABLED (User Override)")
			: derived.pruningRecommendEnabled
				? chalk.green("ENABLED (Derived Default)")
				: chalk.yellow("DISABLED (Derived Default)");

	const subagentStatus =
		userSubagent !== undefined
			? userSubagent
				? chalk.bold.green("ENABLED (User Override)")
				: chalk.bold.red("DISABLED (User Override)")
			: derived.subagentSelectionRecommendEnabled
				? chalk.green("ENABLED (Derived Default)")
				: chalk.yellow("DISABLED (Derived Default)");

	writeLine(`  laya.pruning:           ${pruningStatus}`);
	writeLine(`  laya.subagentSelection: ${subagentStatus}`);
	writeLine(`  laya.timeout:           ${userTimeout ?? derived.subagentSelectionTimeoutMs}ms`);

	// Warnings on overrides contradicting calibration
	if (userPruning === true && !derived.pruningRecommendEnabled) {
		writeLine(
			`\n${chalk.yellow("WARNING:")} Pruning is explicitly enabled in user settings, but measured batched latency ` +
				`(${derived.estimatedAddedLatencyPerTurnMs.toFixed(0)}ms) exceeds the turn budget (${derived.maxAcceptableLatencyPerTurnMs}ms). ` +
				`This may add noticeable delay to interactive turns.`,
		);
	}
	if (userSubagent === true && !derived.subagentSelectionRecommendEnabled) {
		writeLine(
			`\n${chalk.yellow("WARNING:")} Subagent selection is explicitly enabled in user settings, but single-choice latency ` +
				`(${b.singleChoice.medianMs.toFixed(0)}ms) is high on this hardware tier.`,
		);
	}
}

export async function runLayaCommand(command: LayaCommandArgs): Promise<void> {
	const client = getLayaClient();
	const agentDir = getAgentDir();

	if (command.action === "calibrate") {
		writeLine(chalk.bold("Starting Harvest Laya Self-Calibration..."));
		writeLine("Connecting to local decision sidecar...");

		const isOnline = await client.isHealthy();
		if (!isOnline) {
			const hw = await client.getHardwareInfo();
			if (!hw) {
				writeLine(chalk.red("Error: Cannot connect to Laya sidecar at " + client.baseUrl));
				writeLine("Please ensure the sidecar is running (e.g. `python decision-sidecar/server.py`).");
				process.exitCode = 1;
				return;
			}
		}

		try {
			writeLine("Running representative benchmark shapes (warmup + 3 runs each)...");
			let maxTurnLatency = DEFAULT_MAX_ACCEPTABLE_TURN_LATENCY_MS;
			try {
				const configured = settings.get("laya.maxAcceptableLatencyPerTurnMs");
				if (typeof configured === "number") maxTurnLatency = configured;
			} catch {}

			const record = await ensureCalibrated(client, {
				agentDir,
				force: command.flags.force ?? true,
				maxAcceptableLatencyPerTurnMs: maxTurnLatency,
			});

			if (command.flags.json) {
				writeLine(JSON.stringify(record, null, 2));
				return;
			}

			writeLine(chalk.green("\nCalibration complete and saved successfully!\n"));
			formatStatus(record, true);
		} catch (err) {
			writeLine(chalk.red(`\nCalibration failed: ${(err as Error).message}`));
			process.exitCode = 1;
		}
		return;
	}

	// Default action: "status"
	const isOnline = await client.isHealthy();
	const cached = await loadCalibration(agentDir);

	if (command.flags.json) {
		writeLine(
			JSON.stringify(
				{
					online: isOnline,
					calibration: cached,
				},
				null,
				2,
			),
		);
		return;
	}

	formatStatus(cached, isOnline);
}
