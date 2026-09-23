/**
 * CLI Handler for `harvest laya [calibrate|status|review-shadow|recalibrate-subagent]`.
 *
 * Provides user-facing inspection, shadow-mode trace review, disagreement sampling,
 * and empirical confidence threshold re-derivation.
 */

import * as readline from "node:readline/promises";
import chalk from "@harvest/pi-utils/chalk";
import { getAgentDir } from "@harvest/pi-utils";
import { settings } from "../config/settings";
import {
	DEFAULT_MAX_ACCEPTABLE_TURN_LATENCY_MS,
	ensureCalibrated,
	loadCalibration,
	saveCalibration,
	type CalibrationRecord,
} from "../core/harvest/laya-calibration";
import { getLayaClient } from "../core/harvest/laya-client";
import { loadAuditRecords } from "../core/harvest/laya-subagent-selection";
import {
	deriveOptimalThreshold,
	filterDisagreements,
	getShadowTraceStats,
	loadReviewLabels,
	MINIMUM_LABELED_DISAGREEMENTS_FLOOR,
	saveReviewLabel,
	type HumanReviewLabel,
	type LayaLabeledTrace,
} from "../core/harvest/laya-shadow-review";

export type LayaAction = "calibrate" | "status" | "review-shadow" | "recalibrate-subagent";

export interface LayaCommandArgs {
	action: LayaAction;
	flags: {
		force?: boolean;
		json?: boolean;
		yes?: boolean;
		label?: string;
		limit?: number;
		agentDir?: string;
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

	const chain = (hw.details?.detection_chain as Array<{ probe: string; status: string; message: string }>) ?? [];
	if (chain.length > 0) {
		writeLine(`  Detection Chain:`);
		for (const p of chain) {
			const statusColor = p.status === "pass" || p.status === "active" ? chalk.green : p.status === "misconfigured" ? chalk.red : chalk.dim;
			writeLine(`    ${chalk.dim("•")} ${p.probe.padEnd(24)} ${statusColor(`[${p.status}]`)} ${chalk.dim(p.message)}`);
		}
	}

	writeLine(`\n${chalk.cyan("Measured Latency Benchmarks:")}`);
	writeLine(`  Ultra-short (~40 tok):         ${b.ultraShort.medianMs.toFixed(1)}ms (min: ${b.ultraShort.minMs.toFixed(1)}ms)`);
	writeLine(`  Single Choice (~350 tok):      ${b.singleChoice.medianMs.toFixed(1)}ms (min: ${b.singleChoice.minMs.toFixed(1)}ms)`);
	writeLine(`  Single Score (~150 tok):       ${b.singleScore.medianMs.toFixed(1)}ms (min: ${b.singleScore.minMs.toFixed(1)}ms)`);
	writeLine(`  Batched Score (B=2, L=1024):   ${b.batchedScore.medianMs.toFixed(1)}ms (min: ${b.batchedScore.minMs.toFixed(1)}ms)`);

	writeLine(`\n${chalk.cyan("Derived Operational Settings (Latency-Only):")}`);
	writeLine(
		`  Subagent Selection: ${
			derived.subagentSelectionRecommendEnabled ? chalk.green("RECOMMENDED") : chalk.yellow("NOT RECOMMENDED")
		} (${derived.subagentSelectionReason})`,
	);
	writeLine(
		`  Subagent Timeout:   ${chalk.bold(`${derived.subagentSelectionTimeoutMs}ms`)} ` +
			`(2x safety margin over raw ${derived.rawSingleChoiceLatencyMs?.toFixed(1) ?? b.singleChoice.medianMs.toFixed(1)}ms measured choice latency)`,
	);
	writeLine(
		`  Context Pruning:    ${
			derived.pruningRecommendEnabled ? chalk.green("RECOMMENDED") : chalk.yellow("NOT RECOMMENDED")
		} (${derived.pruningReason})`,
	);
	writeLine(
		`  Turn Latency Budget:${derived.maxAcceptableLatencyPerTurnMs}ms max acceptable added time ` +
			`(realistic turn cost: ~${derived.estimatedAddedLatencyPerTurnMs.toFixed(0)}ms, worst-case batch: ${derived.worstCaseBatchLatencyMs?.toFixed(0) ?? b.batchedScore.medianMs.toFixed(0)}ms)`,
	);
	if (derived.subagentSelectionConfidenceThreshold !== undefined) {
		writeLine(`  Confidence Thresh:  ${chalk.bold(derived.subagentSelectionConfidenceThreshold.toFixed(3))} (owned by shadow-mode review)`);
	}

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
			`\n${chalk.yellow("WARNING:")} Pruning is explicitly enabled in user settings, but estimated turn latency ` +
				`(~${derived.estimatedAddedLatencyPerTurnMs.toFixed(0)}ms) exceeds the turn budget (${derived.maxAcceptableLatencyPerTurnMs}ms). ` +
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

async function runReviewShadow(command: LayaCommandArgs, agentDir: string): Promise<void> {
	const records = await loadAuditRecords(agentDir);
	const labels = await loadReviewLabels(agentDir);

	// Direct labeling flag: --label <traceId>:<label>
	if (command.flags.label) {
		const parts = command.flags.label.split(":");
		const traceId = parts[0]?.trim();
		const rawLabel = parts[1]?.trim().toLowerCase();

		if (!traceId || !rawLabel || !["laya_correct", "caller_correct", "ambiguous"].includes(rawLabel)) {
			writeLine(chalk.red("Error: Invalid label format. Expected: --label <traceId>:<laya_correct|caller_correct|ambiguous>"));
			process.exitCode = 1;
			return;
		}

		const matchingRecord = records.find(r => r.id === traceId);
		const label: LayaLabeledTrace = {
			traceId,
			timestamp: matchingRecord?.timestamp ?? Date.now(),
			assignment: matchingRecord?.assignment ?? "",
			layaPick: matchingRecord?.layaPick ?? "unknown",
			confidence: matchingRecord?.confidence ?? 0.0,
			callerPick: matchingRecord?.selectedAgent ?? "task",
			callerOutcomeClean: matchingRecord?.outcome?.completedCleanly,
			callerOutcomeError: matchingRecord?.outcome?.error,
			label: rawLabel as HumanReviewLabel,
			reviewedAt: Date.now(),
		};

		await saveReviewLabel(label, agentDir);
		writeLine(chalk.green(`Successfully saved label '${rawLabel}' for trace ${traceId}.`));
		return;
	}

	const stats = getShadowTraceStats(records, labels);
	const pending = filterDisagreements(records, labels, {
		unlabeledOnly: true,
		limit: command.flags.limit ?? 20,
	});

	if (command.flags.json) {
		writeLine(
			JSON.stringify(
				{
					stats,
					disagreements: pending,
					labels: Object.fromEntries(labels.entries()),
				},
				null,
				2,
			),
		);
		return;
	}

	writeLine(chalk.bold("Harvest Laya Shadow-Mode Trace Review"));
	writeLine("=".repeat(60));
	writeLine(`\n${chalk.cyan("Shadow-Mode Telemetry Stats:")}`);
	writeLine(`  Total Shadow Traces:   ${chalk.bold(stats.totalShadowTraces)} (${stats.recentShadowTraces} in last 30 days)`);
	writeLine(`  Total Disagreements:   ${chalk.bold(stats.totalDisagreements)} (${stats.recentDisagreements} in last 30 days)`);
	writeLine(`  Human-Labeled:         ${chalk.green(stats.labeledCount)}`);
	writeLine(`  Pending Review:        ${stats.pendingDisagreements > 0 ? chalk.yellow(stats.pendingDisagreements) : chalk.green("0")}`);

	if (pending.length === 0) {
		if (stats.totalDisagreements === 0) {
			writeLine(chalk.dim("\n  No disagreement traces found yet. Keep Harvest running in shadow mode to accumulate real data.\n"));
		} else {
			writeLine(chalk.green("\n  All disagreement traces have been reviewed!\n  Run `harvest laya recalibrate-subagent` once 30+ labels exist.\n"));
		}
		return;
	}

	writeLine(`\n${chalk.cyan(`Unlabeled Disagreements (Showing ${pending.length} of ${stats.pendingDisagreements}):`)}`);

	const isInteractive = Boolean(process.stdin.isTTY && !process.env.CI);
	let rl: readline.Interface | undefined;

	if (isInteractive) {
		rl = readline.createInterface({ input: process.stdin, output: process.stdout });
	}

	try {
		for (let i = 0; i < pending.length; i++) {
			const item = pending[i]!;
			const ageMin = Math.round((Date.now() - item.timestamp) / 60000);
			const ageStr = ageMin < 60 ? `${ageMin}m ago` : `${Math.round(ageMin / 60)}h ago`;

			writeLine("\n" + "─".repeat(60));
			writeLine(`[${i + 1}/${pending.length}] Trace ID: ${chalk.bold(item.id)} ${chalk.dim(`(${ageStr})`)}`);
			writeLine(`  Task:        ${chalk.white(item.assignment.replace(/[\r\n]+/g, " ").slice(0, 140))}`);
			writeLine(`  Laya Pick:   ${chalk.bold.yellow(item.layaPick ?? "none")} (conf: ${item.confidence !== undefined ? item.confidence.toFixed(4) : "N/A"})`);
			writeLine(`  Caller Pick: ${chalk.bold.cyan(item.selectedAgent)}`);

			let outcomeText = chalk.dim("No outcome signal recorded");
			if (item.outcome) {
				if (item.outcome.completedCleanly) {
					outcomeText = chalk.green("Caller pick completed cleanly (exit 0)");
				} else {
					outcomeText = chalk.red(`Caller pick failed/aborted (${item.outcome.error || `exit ${item.outcome.exitCode}`})`);
				}
			}
			writeLine(`  Outcome:     ${outcomeText}`);

			if (isInteractive && rl) {
				const ans = await rl.question(
					`\n  Label choice: [1] Laya right  [2] Caller right  [3] Ambiguous  [s] Skip  [q] Quit: `,
				);
				const trimmed = ans.trim().toLowerCase();
				if (trimmed === "q" || trimmed === "quit") {
					writeLine(chalk.dim("  Review session stopped."));
					break;
				}
				let chosenLabel: HumanReviewLabel | undefined;
				if (trimmed === "1" || trimmed === "laya") chosenLabel = "laya_correct";
				else if (trimmed === "2" || trimmed === "caller") chosenLabel = "caller_correct";
				else if (trimmed === "3" || trimmed === "ambiguous") chosenLabel = "ambiguous";

				if (chosenLabel) {
					const labelRecord: LayaLabeledTrace = {
						traceId: item.id,
						timestamp: item.timestamp,
						assignment: item.assignment,
						layaPick: item.layaPick ?? "unknown",
						confidence: item.confidence ?? 0.0,
						callerPick: item.selectedAgent,
						callerOutcomeClean: item.outcome?.completedCleanly,
						callerOutcomeError: item.outcome?.error,
						label: chosenLabel,
						reviewedAt: Date.now(),
					};
					await saveReviewLabel(labelRecord, agentDir);
					labels.set(item.id, labelRecord);
					writeLine(chalk.green(`  ✓ Saved as '${chosenLabel}'.`));
				} else {
					writeLine(chalk.dim("  Skipped."));
				}
			}
		}

		if (!isInteractive) {
			writeLine(chalk.dim("\nTo label a trace from the command line:"));
			writeLine(`  ${chalk.cyan("harvest laya review-shadow --label <traceId>:<laya_correct|caller_correct|ambiguous>")}\n`);
		}
	} finally {
		rl?.close();
	}
}

async function runRecalibrateSubagent(command: LayaCommandArgs, agentDir: string): Promise<void> {
	const labels = await loadReviewLabels(agentDir);
	const decisive = Array.from(labels.values()).filter(l => l.label === "laya_correct" || l.label === "caller_correct");

	if (decisive.length < MINIMUM_LABELED_DISAGREEMENTS_FLOOR) {
		writeLine(chalk.bold.red(`\nRefusing to recalibrate: insufficient human-labeled disagreement data.`));
		writeLine(`Found ${chalk.bold(decisive.length)} labeled disagreement traces (minimum floor is ${MINIMUM_LABELED_DISAGREEMENTS_FLOOR}).`);
		writeLine(chalk.dim("Recalibrating on too few samples leads to brittle, overfitted thresholds."));
		writeLine(`Run ${chalk.cyan("harvest laya review-shadow")} to review and label real disagreement traces first.\n`);
		process.exitCode = 1;
		return;
	}

	const result = deriveOptimalThreshold(Array.from(labels.values()));

	if (command.flags.json) {
		writeLine(JSON.stringify(result, null, 2));
		return;
	}

	writeLine(chalk.bold("Harvest Laya Subagent Confidence Recalibration"));
	writeLine("=".repeat(60));
	writeLine(`Evaluated ${chalk.bold(decisive.length)} decisive human-labeled disagreement traces.\n`);

	writeLine(chalk.cyan("Candidate Threshold Accuracy & Yield Sweep:"));
	writeLine(
		`  ${"Threshold".padEnd(11)} | ${"Auto-Picks".padEnd(11)} | ${"Correct".padEnd(8)} | ${"Wrong".padEnd(6)} | ${"Precision".padEnd(10)} | ${"Yield".padEnd(8)} | Status`,
	);
	writeLine("  " + "-".repeat(75));

	for (const p of result.points) {
		const isRec = p.threshold === result.recommendedThreshold;
		const line =
			`  ${p.threshold.toFixed(3).padEnd(11)} | ` +
			`${p.autoPicks.toString().padEnd(11)} | ` +
			`${p.correctAutoPicks.toString().padEnd(8)} | ` +
			`${p.wrongAutoPicks.toString().padEnd(6)} | ` +
			`${(p.precision * 100).toFixed(1).padStart(5)}%     | ` +
			`${(p.yield * 100).toFixed(1).padStart(5)}%  | ` +
			(isRec ? chalk.bold.green("[RECOMMENDED]") : "");

		writeLine(isRec ? chalk.green(line) : line);
	}

	writeLine(`\n${chalk.cyan("Recalibration Recommendation:")}`);
	writeLine(`  Recommended Threshold:  ${chalk.bold.green(result.recommendedThreshold.toFixed(3))}`);
	writeLine(
		`  Precision:              ${(result.selectedPoint.precision * 100).toFixed(1)}% ` +
			`(${result.selectedPoint.wrongAutoPicks} wrong auto-picks out of ${result.selectedPoint.autoPicks})`,
	);
	writeLine(
		`  Auto-Pick Yield:        ${(result.selectedPoint.yield * 100).toFixed(1)}% ` +
			`(${result.selectedPoint.correctAutoPicks} tasks correctly auto-picked)`,
	);
	writeLine(
		`  Escalation Yield:       ${((1 - result.selectedPoint.yield) * 100).toFixed(1)}% ` +
			`(${result.selectedPoint.escalations} tasks safely escalated to caller)`,
	);

	if (command.flags.yes) {
		const cal = await loadCalibration(agentDir);
		if (cal) {
			cal.derivedSettings.subagentSelectionConfidenceThreshold = result.recommendedThreshold;
			await saveCalibration(cal, agentDir);
			writeLine(chalk.bold.green(`\n✓ Calibration file updated successfully with threshold ${result.recommendedThreshold}!\n`));
		} else {
			writeLine(
				chalk.yellow(
					"\nNotice: No existing calibration record found to update. Run `harvest laya calibrate` first.\n",
				),
			);
		}
	} else {
		writeLine(chalk.yellow("\nRecalibration dry-run complete. Calibration file was NOT modified."));
		writeLine(chalk.dim("To apply this threshold to ~/.harvest/laya-calibration.json, run again with:"));
		writeLine(`  ${chalk.cyan("harvest laya recalibrate-subagent --yes")}\n`);
	}
}

export async function runLayaCommand(command: LayaCommandArgs): Promise<void> {
	const client = getLayaClient();
	const agentDir = command.flags.agentDir ?? getAgentDir();

	if (command.action === "review-shadow") {
		await runReviewShadow(command, agentDir);
		return;
	}

	if (command.action === "recalibrate-subagent") {
		await runRecalibrateSubagent(command, agentDir);
		return;
	}

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
