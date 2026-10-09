/**
 * Live status board for `omp cleanse`.
 *
 * Interactive terminals get a transient board repainted in place: a phase
 * spinner (model resolution, checker discovery), one row per running checker,
 * and one row per repair subagent showing its latest intent, current tool,
 * tool count, and elapsed time from {@link AgentProgress} snapshots. Finished
 * work is promoted to permanent scrollback lines as it settles.
 *
 * Non-TTY output keeps the original plain-line protocol
 * (`[start]`/`[done]`/`[fail]`), so scripted callers see unchanged output.
 */
import { formatDuration, formatNumber, sanitizeText } from "@harvest/pi-utils";
import chalk from "@harvest/pi-utils/chalk";
import { createLiveBoard, type LiveBoardOutput } from "../cli/live-board";
import type { AgentProgress } from "../task/types";
import type { CleanseCheckerDescriptor } from "./checkers";
import type { CleanseAgentOutcome, CleanseAssignment, CleanseCheckResult } from "./types";

const BAR_WIDTH = 16;
const ACTIVITY_WIDTH = 96;
const ERROR_WIDTH = 300;

export type CleanseBoardColor = "success" | "warning" | "error" | "accent" | "dim";
export type CleanseBoardLogRenderer = () => string;

/** Presentation supplied by the terminal host; CLI callers retain their existing output. */
export interface CleanseBoardPresentation {
	fg(color: CleanseBoardColor, text: string): string;
	bold(text: string): string;
	readonly success: string;
	readonly warning: string;
	readonly error: string;
	readonly separator: string;
	readonly filled: string;
	readonly empty: string;
}

const CLI_PRESENTATION: CleanseBoardPresentation = {
	fg: (color, text) => {
		switch (color) {
			case "success":
				return chalk.green(text);
			case "warning":
				return chalk.yellow(text);
			case "error":
				return chalk.red(text);
			case "accent":
				return chalk.cyan(text);
			case "dim":
				return chalk.dim(text);
		}
	},
	bold: text => chalk.bold(text),
	success: "✓",
	warning: "●",
	error: "✗",
	separator: " · ",
	filled: "█",
	empty: "░",
};

/** Rendering surface for one `omp cleanse` run. */
export interface CleanseStatusBoard {
	readonly interactive: boolean;
	/** Print a permanent line above the live area (plain write when non-TTY). */
	log(text: string): void;
	/** Show a transient spinner line; `undefined` clears it. Non-TTY prints the text once. */
	phase(text: string | undefined): void;
	checkerStarted(checker: CleanseCheckerDescriptor): void;
	checkerFinished(check: CleanseCheckResult, durationMs: number): void;
	/** End the repair phase and drop its live rows before verification. */
	repairFinished(): void;
	agentStarted(name: string, assignment: CleanseAssignment): void;
	agentProgress(name: string, progress: AgentProgress): void;
	agentFinished(outcome: CleanseAgentOutcome, assignment: CleanseAssignment): void;
	/** Clear the live area and restore the cursor. Idempotent. */
	close(): void;
}

interface RunningChecker {
	label: string;
	startedAt: number;
}

interface RunningAgent {
	assignment: CleanseAssignment;
	startedAt: number;
	progress?: AgentProgress;
}

/**
 * Live-view state for one cleanse run, shared by the CLI stdout board and the
 * interactive-mode overlay panel so both surfaces render identical rows.
 *
 * Mutators mirror {@link CleanseStatusBoard}; the finish mutators return the
 * permanent line the surface should log above the live area.
 */
export class CleanseBoardModel {
	constructor(private readonly presentation: CleanseBoardPresentation = CLI_PRESENTATION) {}
	#phaseText: string | undefined;
	readonly #checkers = new Map<string, RunningChecker>();
	readonly #agents = new Map<string, RunningAgent>();
	/** Lifetime token/cost totals per agent; survives row removal for the header sums. */
	readonly #totals = new Map<string, { tokens: number; cost: number }>();
	#repairTotal = 0;
	#repairDone = 0;
	#repairStartedAt = 0;

	phase(text: string | undefined): void {
		this.#phaseText = text;
	}

	checkerStarted(checker: CleanseCheckerDescriptor): void {
		this.#checkers.set(checker.id, { label: checker.label, startedAt: Date.now() });
	}

	/** Drop the checker's live row and build its permanent verdict line. */
	checkerFinished(check: CleanseCheckResult, durationMs: number): string {
		return this.finishChecker(check, durationMs)();
	}

	/** Capture a verdict once while permitting the host's presentation to change. */
	finishChecker(check: CleanseCheckResult, durationMs: number): CleanseBoardLogRenderer {
		this.#checkers.delete(check.id);
		const count = check.diagnostics.length;
		const label = check.label;
		const color = count === 0 ? "success" : "warning";
		const verdictText = count === 0 ? "clean" : `${count} issue${count === 1 ? "" : "s"}`;
		const duration = formatDuration(durationMs);
		return () => {
			const verdict = this.presentation.fg(color, verdictText);
			const glyph = this.presentation.fg(color, count === 0 ? this.presentation.success : this.presentation.warning);
			return `${glyph} ${label} ${verdict}${this.presentation.fg("dim", `${this.presentation.separator}${duration}`)}`;
		};
	}

	repairFinished(): void {
		this.#repairTotal = 0;
		this.#repairDone = 0;
		this.#agents.clear();
	}

	agentStarted(name: string, assignment: CleanseAssignment): void {
		if (this.#repairStartedAt === 0) this.#repairStartedAt = Date.now();
		this.#repairTotal += 1;
		this.#agents.set(name, { assignment, startedAt: Date.now() });
	}

	agentProgress(name: string, progress: AgentProgress): void {
		this.#totals.set(name, { tokens: progress.tokens, cost: progress.cost });
		const agent = this.#agents.get(name);
		if (agent) agent.progress = progress;
	}

	/** Drop the agent's live row, advance the repair bar, and build its permanent outcome line. */
	agentFinished(outcome: CleanseAgentOutcome, assignment: CleanseAssignment): string {
		return this.finishAgent(outcome, assignment)();
	}

	/** Capture settled metadata without retaining diagnostic or progress payloads. */
	finishAgent(outcome: CleanseAgentOutcome, assignment: CleanseAssignment): CleanseBoardLogRenderer {
		const agent = this.#agents.get(outcome.name);
		this.#agents.delete(outcome.name);
		this.#repairDone = Math.min(this.#repairDone + 1, this.#repairTotal);
		return outcomeLineRenderer(outcome, assignment, agent, this.#totals.get(outcome.name), this.presentation);
	}

	/** Render the transient live rows for the current spinner frame. */
	renderLive(spinner: string): string[] {
		const lines: string[] = [];
		if (this.#phaseText) lines.push(`${this.presentation.fg("warning", spinner)} ${this.#phaseText}`);
		for (const checker of this.#checkers.values()) {
			const elapsed = formatDuration(Date.now() - checker.startedAt);
			lines.push(
				`${this.presentation.fg("warning", spinner)} ${checker.label}${this.presentation.fg("dim", `${this.presentation.separator}${elapsed}`)}`,
			);
		}
		if (this.#repairTotal > 0) {
			lines.push(
				renderWaveHeader(
					spinner,
					this.#repairTotal,
					this.#repairDone,
					this.#agents.size,
					this.#totals,
					this.#repairStartedAt,
					this.presentation,
				),
			);
			const rows = [...this.#agents.entries()].sort(
				(left, right) => left[1].assignment.index - right[1].assignment.index,
			);
			for (const [name, agent] of rows) lines.push(renderAgentRow(spinner, name, agent, this.presentation));
		}
		return lines;
	}
}

/** Create the cleanse status board bound to `output` (default `process.stdout`). */
export function createCleanseStatusBoard(
	output: LiveBoardOutput = process.stdout,
	errors: LiveBoardOutput = process.stderr,
): CleanseStatusBoard {
	const model = new CleanseBoardModel();
	const board = createLiveBoard(spinner => model.renderLive(spinner), output);

	return {
		interactive: board.interactive,
		log: board.log,
		phase(text) {
			if (!board.interactive) {
				if (text) output.write(`${text}\n`);
				return;
			}
			model.phase(text);
			board.repaint();
		},
		checkerStarted(checker) {
			if (!board.interactive) return;
			model.checkerStarted(checker);
			board.repaint();
		},
		checkerFinished(check, durationMs) {
			board.log(model.checkerFinished(check, durationMs));
		},
		repairFinished() {
			model.repairFinished();
			board.repaint();
		},
		agentStarted(name, assignment) {
			if (!board.interactive) {
				const files = assignment.groups.map(group => group.file ?? "<project>").join(", ");
				output.write(`[start] ${name}: ${files} (weight ${assignment.weight})\n`);
				return;
			}
			model.agentStarted(name, assignment);
			board.repaint();
		},
		agentProgress(name, progress) {
			model.agentProgress(name, progress);
		},
		agentFinished(outcome, assignment) {
			if (!board.interactive) {
				model.agentFinished(outcome, assignment);
				if (outcome.success) {
					output.write(`[done] ${outcome.name}${outcome.resolvedModel ? ` (${outcome.resolvedModel})` : ""}\n`);
				} else {
					errors.write(`[fail] ${outcome.name}: ${oneLine(outcome.error ?? "subagent failed", ERROR_WIDTH)}\n`);
				}
				return;
			}
			board.log(model.agentFinished(outcome, assignment));
		},
		close: board.close,
	};
}

function renderWaveHeader(
	spinner: string,
	total: number,
	done: number,
	running: number,
	totals: ReadonlyMap<string, { tokens: number; cost: number }>,
	startedAt: number,
	presentation: CleanseBoardPresentation,
): string {
	const filled = Math.round(Math.min(done / total, 1) * BAR_WIDTH);
	const bar =
		presentation.fg("accent", presentation.filled.repeat(filled)) +
		presentation.fg("dim", presentation.empty.repeat(BAR_WIDTH - filled));
	let tokens = 0;
	let cost = 0;
	for (const entry of totals.values()) {
		tokens += entry.tokens;
		cost += entry.cost;
	}
	const parts = [`${done}/${total}`];
	if (running > 0) parts.push(`${running} running`);
	if (tokens > 0) parts.push(`${formatNumber(tokens)} tok`);
	if (cost > 0) parts.push(formatCost(cost));
	parts.push(formatDuration(Date.now() - startedAt));
	return `${presentation.fg("accent", spinner)} Repairing [${bar}] ${parts.join(presentation.fg("dim", presentation.separator))}`;
}

function renderAgentRow(
	spinner: string,
	agentName: string,
	agent: RunningAgent,
	presentation: CleanseBoardPresentation,
): string {
	const label = agentName.replace(/^Cleanse/, "");
	const meta: string[] = [];
	const toolCount = agent.progress?.toolCount ?? 0;
	if (toolCount > 0) meta.push(`${toolCount} tool${toolCount === 1 ? "" : "s"}`);
	meta.push(formatDuration(Date.now() - agent.startedAt));
	return (
		`${presentation.fg("warning", spinner)} ${presentation.bold(label)} ${compactFiles(agent.assignment)}` +
		`${presentation.fg("dim", presentation.separator)}${agentActivity(agent.progress, presentation)}${presentation.fg("dim", `${presentation.separator}${meta.join(presentation.separator)}`)}`
	);
}

function outcomeLineRenderer(
	outcome: CleanseAgentOutcome,
	assignment: CleanseAssignment,
	agent: RunningAgent | undefined,
	total: { tokens: number; cost: number } | undefined,
	presentation: CleanseBoardPresentation,
): CleanseBoardLogRenderer {
	const files = compactFiles(assignment);
	const name = outcome.name;
	if (!outcome.success) {
		const message = oneLine(outcome.error ?? "subagent failed", ERROR_WIDTH);
		return () =>
			`${presentation.fg("error", presentation.error)} ${name} ${files} ${presentation.fg("error", message)}`;
	}
	const meta: string[] = [];
	const toolCount = agent?.progress?.toolCount ?? 0;
	if (toolCount > 0) meta.push(`${toolCount} tool${toolCount === 1 ? "" : "s"}`);
	if (total && total.tokens > 0) meta.push(`${formatNumber(total.tokens)} tok`);
	if (agent) meta.push(formatDuration(Date.now() - agent.startedAt));
	return () => {
		const suffix =
			meta.length > 0 ? presentation.fg("dim", `${presentation.separator}${meta.join(presentation.separator)}`) : "";
		return `${presentation.fg("success", presentation.success)} ${name} ${files}${suffix}`;
	};
}

/** Latest human-readable activity for a repair agent row. */
function agentActivity(progress: AgentProgress | undefined, presentation: CleanseBoardPresentation): string {
	if (!progress) return presentation.fg("dim", "starting");
	if (progress.retryState) {
		return presentation.fg(
			"warning",
			`rate-limited${presentation.separator}retry ${progress.retryState.attempt}/${progress.retryState.maxAttempts}`,
		);
	}
	const intent = oneLine(progress.lastIntent ?? "", ACTIVITY_WIDTH);
	if (progress.currentTool) {
		const args = oneLine(progress.currentToolArgs ?? "", ACTIVITY_WIDTH);
		const tool = presentation.fg("dim", args ? `${progress.currentTool} ${args}` : progress.currentTool);
		return intent ? `${intent} ${tool}` : tool;
	}
	return intent || presentation.fg("dim", "thinking");
}

function compactFiles(assignment: CleanseAssignment): string {
	const files = assignment.groups.map(group => group.file ?? "<project>");
	const first = files[0] ?? "<project>";
	return files.length > 1 ? `${first} +${files.length - 1}` : first;
}

function oneLine(text: string, width: number): string {
	return sanitizeText(text).replace(/\s+/g, " ").trim().slice(0, width);
}

function formatCost(cost: number): string {
	return `$${cost >= 0.095 ? cost.toFixed(2) : cost.toFixed(3)}`;
}
