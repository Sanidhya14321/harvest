/**
 * CLI Handler for `harvest laya [calibrate|status|review-shadow|recalibrate-subagent]`.
 *
 * Provides user-facing inspection, shadow-mode trace review, disagreement sampling,
 * and empirical confidence threshold re-derivation.
 */

import * as readline from "node:readline/promises";
import chalk from "@harvest/pi-utils/chalk";
import { getAgentDir } from "@harvest/pi-utils";
import { Settings, settings } from "../config/settings";
import {
	DEFAULT_MAX_ACCEPTABLE_TURN_LATENCY_MS,
	ensureCalibrated,
	getExplicitSetting,
	loadCalibration,
	saveCalibration,
	type CalibrationRecord,
	type DerivedLayaSettings,
} from "../core/harvest/laya-calibration";
import { getLayaClient } from "../core/harvest/laya-client";
import { configureLayaLocally } from "../core/harvest/laya-service";
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

export type LayaAction = "setup" | "calibrate" | "status" | "review-shadow" | "recalibrate-subagent";

export interface LayaCommandArgs {
	action: LayaAction;
	flags: {
		force?: boolean;
		reinstall?: boolean;
		port?: number;
		url?: string;
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

/** Where an effective status value came from. Never report an unset key as a user override. */
export type LayaSettingProvenance = "user-configured" | "calibrated" | "schema-default";

export interface LayaResolvedSetting<T> {
	readonly value: T;
	readonly provenance: LayaSettingProvenance;
}

export interface LayaExplicitOverrides {
	readonly pruning?: boolean;
	readonly subagentSelection?: boolean;
	readonly subagentSelectionTimeoutMs?: number;
}

export interface LayaStatusSchemaDefaults {
	readonly pruning: boolean;
	readonly subagentSelection: boolean;
	readonly subagentSelectionTimeoutMs: number;
}

export interface LayaResolvedStatusSettings {
	readonly pruning: LayaResolvedSetting<boolean>;
	readonly subagentSelection: LayaResolvedSetting<boolean>;
	readonly subagentSelectionTimeoutMs: LayaResolvedSetting<number>;
}

/**
 * Resolve configured vs calibrated vs effective Laya settings for status display.
 *
 * Precedence is explicit user configuration, then hardware-calibrated
 * recommendation, then the schema default. A key the user never configured
 * resolves to `calibrated`/`schema-default` provenance — never
 * `user-configured` — so `settings.get()` schema defaults can no longer be
 * misreported as overrides.
 */
export function resolveLayaStatusSettings(
	explicit: LayaExplicitOverrides,
	derived: DerivedLayaSettings | null | undefined,
	schemaDefaults: LayaStatusSchemaDefaults,
): LayaResolvedStatusSettings {
	const pruningCalibrated = derived?.pruningRecommendEnabled;
	const subagentCalibrated = derived?.subagentSelectionRecommendEnabled;
	const timeoutCalibrated = derived?.subagentSelectionTimeoutMs;
	return {
		pruning:
			explicit.pruning !== undefined
				? { value: explicit.pruning, provenance: "user-configured" }
				: pruningCalibrated !== undefined
					? { value: pruningCalibrated, provenance: "calibrated" }
					: { value: schemaDefaults.pruning, provenance: "schema-default" },
		subagentSelection:
			explicit.subagentSelection !== undefined
				? { value: explicit.subagentSelection, provenance: "user-configured" }
				: subagentCalibrated !== undefined
					? { value: subagentCalibrated, provenance: "calibrated" }
					: { value: schemaDefaults.subagentSelection, provenance: "schema-default" },
		subagentSelectionTimeoutMs:
			explicit.subagentSelectionTimeoutMs !== undefined
				? { value: explicit.subagentSelectionTimeoutMs, provenance: "user-configured" }
				: timeoutCalibrated !== undefined
					? { value: timeoutCalibrated, provenance: "calibrated" }
					: { value: schemaDefaults.subagentSelectionTimeoutMs, provenance: "schema-default" },
	};
}

function formatStatus(cal: CalibrationRecord | null, isOnline: boolean): void {
	writeLine(chalk.bold("Harvest Laya Decision System Status"));
	writeLine("=".repeat(60));

	if (!isOnline) {
		writeLine(`${chalk.red("● Sidecar Status:")} Offline / Unreachable`);
		writeLine(chalk.dim("  The local sidecar is not responding at its configured URL."));
		writeLine(chalk.cyan("  Run `harvest laya setup` to automatically install, calibrate, and start Laya."));
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

	// Explicit user overrides (never fall back to schema defaults here: an
	// unset key must be reported as calibrated/schema-default, not as an override)
	writeLine(`\n${chalk.cyan("Active Configuration (effective ← user override wins over calibration):")}`);
	const explicit: LayaExplicitOverrides = {
		pruning: getExplicitSetting<boolean>(settings, "laya.pruning"),
		subagentSelection: getExplicitSetting<boolean>(settings, "laya.subagentSelection"),
		subagentSelectionTimeoutMs: getExplicitSetting<number>(settings, "laya.subagentSelectionTimeoutMs"),
	};

	let schemaDefaults: LayaStatusSchemaDefaults = {
		pruning: true,
		subagentSelection: false,
		subagentSelectionTimeoutMs: 300,
	};
	try {
		const isolated = Settings.isolated({});
		schemaDefaults = {
			pruning: isolated.get("laya.pruning"),
			subagentSelection: isolated.get("laya.subagentSelection"),
			subagentSelectionTimeoutMs: isolated.get("laya.subagentSelectionTimeoutMs"),
		};
	} catch {
		// Isolated environment: keep compiled fallbacks above
	}

	const resolved = resolveLayaStatusSettings(explicit, derived, schemaDefaults);

	const onOff = (value: boolean | undefined): string => (value === undefined ? "—" : value ? "on" : "off");
	const msOrDash = (value: number | undefined): string => (value === undefined ? "—" : `${value}ms`);
	const provenanceTag = (provenance: LayaSettingProvenance): string =>
		provenance === "user-configured"
			? chalk.bold("user override")
			: provenance === "calibrated"
				? chalk.green("calibrated")
				: chalk.dim("schema default");

	const pruningState = resolved.pruning.value ? chalk.bold.green("ENABLED") : chalk.bold.red("DISABLED");
	const subagentState = resolved.subagentSelection.value ? chalk.bold.green("ENABLED") : chalk.bold.red("DISABLED");

	writeLine(
		`  laya.pruning:            ${pruningState} (${provenanceTag(resolved.pruning.provenance)}) ` +
			chalk.dim(
				`[configured: ${onOff(explicit.pruning)} | calibrated: ${derived.pruningRecommendEnabled ? "recommended" : "not recommended"} | schema default: ${onOff(schemaDefaults.pruning)}]`,
			),
	);
	writeLine(
		`  laya.subagentSelection:  ${subagentState} (${provenanceTag(resolved.subagentSelection.provenance)}) ` +
			chalk.dim(
				`[configured: ${onOff(explicit.subagentSelection)} | calibrated: ${derived.subagentSelectionRecommendEnabled ? "recommended" : "not recommended"} | schema default: ${onOff(schemaDefaults.subagentSelection)}]`,
			),
	);
	writeLine(
		`  laya.timeout:            ${chalk.bold(`${resolved.subagentSelectionTimeoutMs.value}ms`)} (${provenanceTag(resolved.subagentSelectionTimeoutMs.provenance)}) ` +
			chalk.dim(
				`[configured: ${msOrDash(explicit.subagentSelectionTimeoutMs)} | calibrated: ${derived.subagentSelectionTimeoutMs}ms | schema default: ${schemaDefaults.subagentSelectionTimeoutMs}ms]`,
			),
	);

	// Warnings on explicit overrides contradicting calibration
	if (resolved.pruning.provenance === "user-configured" && resolved.pruning.value && !derived.pruningRecommendEnabled) {
		writeLine(
			`\n${chalk.yellow("WARNING:")} Pruning is explicitly enabled in user settings, but estimated turn latency ` +
				`(~${derived.estimatedAddedLatencyPerTurnMs.toFixed(0)}ms) exceeds the turn budget (${derived.maxAcceptableLatencyPerTurnMs}ms). ` +
				`This may add noticeable delay to interactive turns.`,
		);
	}
	if (
		resolved.subagentSelection.provenance === "user-configured" &&
		resolved.subagentSelection.value &&
		!derived.subagentSelectionRecommendEnabled
	) {
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

async function runLayaSetup(command: LayaCommandArgs, agentDir: string): Promise<void> {
	if (!command.flags.json) {
		writeLine(chalk.bold("\n🚀 Harvest Laya Autonomous Setup"));
		writeLine("=".repeat(60));
		writeLine(chalk.dim("Configuring local ModernBERT-large typed decisions for zero-API-cost tool gating, routing, and completion checks.\n"));
	}

	const baseUrl = command.flags.url || (command.flags.port ? `http://127.0.0.1:${command.flags.port}` : undefined);

	const stepTitles: Record<string, string> = {
		python: "Detecting Python runtime & managed virtualenv",
		dependencies: "Verifying dependencies (laya, torch, fastapi, uvicorn)",
		model: "Verifying model checkpoint (convaiinnovations/laya-typed-decisions)",
		sidecar: "Starting local decision sidecar daemon",
		calibrate: "Performing hardware self-calibration",
		smoketest: "Executing end-to-end smoke test (/health + /v1/decide)",
		connect: "Connecting Harvest settings",
	};

	let activeSettings = settings;
	try {
		settings.get("laya.enabled");
	} catch {
		try {
			await Settings.init();
			activeSettings = settings;
		} catch {
			activeSettings = Settings.isolated();
		}
	}

	const isTTY = Boolean(process.stdout.isTTY && !process.env.CI);
	let lastNonTtyUpdate = 0;
	let currentStepId = "";

	const result = await configureLayaLocally({
		settings: activeSettings,
		baseUrl,
		forceReinstall: command.flags.reinstall,
		onStepUpdate: (stepId, status, message) => {
			if (command.flags.json) return;
			const title = stepTitles[stepId] || stepId;
			if (status === "running") {
				if (isTTY) {
					process.stdout.write(`\r\x1b[K  ${chalk.cyan("⠋")} ${chalk.bold(title)}... ${message ? chalk.dim(`(${message})`) : ""}`);
				} else {
					const now = Date.now();
					if (stepId !== currentStepId || now - lastNonTtyUpdate > 1500) {
						currentStepId = stepId;
						lastNonTtyUpdate = now;
						writeLine(`  ${chalk.cyan("⠋")} ${chalk.bold(title)}... ${message ? chalk.dim(`(${message})`) : ""}`);
					}
				}
			} else if (status === "done") {
				if (isTTY) {
					process.stdout.write(`\r\x1b[K  ${chalk.green("✔")} ${chalk.bold(title)}: ${chalk.green(message || "Done")}\n`);
				} else {
					writeLine(`  ${chalk.green("✔")} ${chalk.bold(title)}: ${chalk.green(message || "Done")}`);
				}
			} else if (status === "error") {
				if (isTTY) {
					process.stdout.write(`\r\x1b[K  ${chalk.red("✖")} ${chalk.bold(title)}: ${chalk.red(message || "Failed")}\n`);
				} else {
					writeLine(`  ${chalk.red("✖")} ${chalk.bold(title)}: ${chalk.red(message || "Failed")}`);
				}
			}
		},
	});

	if (command.flags.json) {
		writeLine(JSON.stringify(result, null, 2));
		if (!result.success) process.exitCode = 1;
		return;
	}

	if (!result.success) {
		writeLine("\n" + "=".repeat(60));
		writeLine(chalk.bold("  Harvest + Laya Setup Summary"));
		writeLine("=".repeat(60));
		writeLine(`  ${chalk.green("✔")} Core Harvest:         Ready & functional (fail-open architecture)`);
		writeLine(`  ${chalk.red("✖")} Laya Decision Layer:  Setup failed: ${result.error || "unknown error"}`);
		writeLine(`  ${chalk.yellow("!")} Feature State:        Laya disabled (core agent remains fully functional)`);
		if (result.diagnosticBundlePath) {
			writeLine(`  ${chalk.yellow("!")} Diagnostic Bundle:    ${result.diagnosticBundlePath}`);
		}
		if (result.llmDiagnosis?.available && result.llmDiagnosis.proposal) {
			writeLine("-".repeat(60));
			writeLine(chalk.cyan.bold("  LLM-Assisted Failure Diagnosis (Second-Tier):"));
			writeLine(`  • Diagnosis:     ${result.llmDiagnosis.proposal.diagnosis}`);
			writeLine(`  • Proposed Fix:  ${result.llmDiagnosis.proposal.proposedFix}`);
			writeLine(`  • Risk Level:    ${result.llmDiagnosis.proposal.effectiveRiskLevel.toUpperCase()} (${result.llmDiagnosis.proposal.riskReason})`);
			if (result.llmDiagnosis.proposal.autoApplied) {
				writeLine(`  • Action Status: ${chalk.green.bold("Auto-applied (Low-risk reversible)")}`);
			} else if (result.llmDiagnosis.proposal.suggestedAction) {
				writeLine(`  • Action Status: ${chalk.yellow.bold("Requires User Confirmation")}: ${result.llmDiagnosis.proposal.suggestedAction}`);
			}
		}
		writeLine(`  ${chalk.dim("Setup Log:")}           ~/.harvest/agent/logs/laya-setup.log`);
		writeLine("-".repeat(60));
		writeLine(`  ${chalk.yellow.bold("Status:")} Core Harvest ready; Laya decision sidecar disabled.`);
		writeLine(`  Run 'harvest setup laya' to retry, or check diagnostic bundle for details.`);
		writeLine("=".repeat(60) + "\n");
		process.exitCode = 1;
		return;
	}

	writeLine("\n" + "=".repeat(60));
	writeLine(chalk.bold("  Harvest + Laya Setup Summary"));
	writeLine("=".repeat(60));
	writeLine(`  ${chalk.green("✔")} Python Runtime:        Verified & isolated virtualenv`);
	writeLine(`  ${chalk.green("✔")} Laya Dependencies:     Verified (torch, fastapi, uvicorn, laya)`);
	writeLine(`  ${chalk.green("✔")} Model Checkpoint:      convaiinnovations/laya-typed-decisions`);
	writeLine(`  ${chalk.green("✔")} Sidecar Daemon:       Active at ${result.effectiveUrl || "http://127.0.0.1:8177"}`);
	if (result.calibration) {
		writeLine(`  ${chalk.green("✔")} Hardware Calibration:  Tier: ${result.calibration.hardware.tier.toUpperCase()} (${result.calibration.hardware.device_name})`);
	}
	if (result.smokeTest) {
		writeLine(`  ${chalk.green("✔")} Smoke Test:           /health OK, /v1/decide OK (${result.smokeTest.latencyMs}ms)`);
	}
	writeLine(`  ${chalk.green("✔")} Harvest Connection:    Connected (laya.enabled=true, laya.autostart=true)`);
	writeLine("-".repeat(60));
	writeLine(`  ${chalk.green.bold("Status:")} COMPLETE & VERIFIED (All subsystems operational)`);
	writeLine(`  ${chalk.dim("Setup Log:")} ~/.harvest/agent/logs/laya-setup.log`);
	writeLine("=".repeat(60) + "\n");

	formatStatus(result.calibration ?? (await loadCalibration(agentDir)), true);
}

export async function runLayaCommand(command: LayaCommandArgs): Promise<void> {
	const client = getLayaClient();
	const agentDir = command.flags.agentDir ?? getAgentDir();

	if (command.action === "setup") {
		await runLayaSetup(command, agentDir);
		return;
	}

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
			writeLine(chalk.yellow("Laya sidecar is offline. Launching autonomous setup and startup..."));
			const setupRes = await configureLayaLocally({
				settings,
				baseUrl: client.baseUrl,
				onStepUpdate: (id, status, msg) => {
					if (status === "running") writeLine(chalk.dim(`  [setup] ${id}: ${msg ?? ""}`));
					if (status === "error") writeLine(chalk.red(`  [setup] ${id} error: ${msg ?? ""}`));
				},
			});
			if (!setupRes.success) {
				writeLine(chalk.red("Cannot start sidecar: " + setupRes.error));
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
