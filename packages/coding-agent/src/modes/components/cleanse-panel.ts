/**
 * Anchored overlay panel for `/cleanse`, mounted above the editor like the
 * `/omfg` panel. Implements {@link CleanseStatusBoard}, so the shared cleanse
 * core renders the exact live view `omp cleanse` shows on stdout: transient
 * checker/repair/agent rows from {@link CleanseBoardModel} animate in place while
 * permanent log lines accumulate above them.
 */
import { ScrollView, type TUI, wrapTextWithAnsi } from "@harvest/pi-tui";
import { type CleanseBoardLogRenderer, CleanseBoardModel, type CleanseStatusBoard } from "../../cleanse/board";
import type { CleanseCheckerDescriptor } from "../../cleanse/checkers";
import type { CleanseAgentOutcome, CleanseAssignment, CleanseCheckResult, CleanseRunStatus } from "../../cleanse/types";
import type { AgentProgress } from "../../task/types";
import { replaceTabs } from "../../tools/render-utils";
import { getSymbolTheme, theme } from "../theme/theme";
import { dialogContentWidth, OverlayPanel, renderDialog } from "./overlay-box";

const SPINNER_INTERVAL_MS = 80;
const MAX_LOG_LINES = 14;

interface CleansePanelComponentOptions {
	/** Free-form request shown in the header; omitted for checker-discovery runs. */
	request?: string;
	tui: TUI;
}

/** Terminal state of the run, mirrored into the footer once the core settles. */
type CleansePanelOutcome = CleanseRunStatus | "error";

export class CleansePanelComponent extends OverlayPanel implements CleanseStatusBoard {
	readonly interactive = true;

	readonly #tui: TUI;
	readonly #model = new CleanseBoardModel({
		fg: (color, text) => theme.fg(color, text),
		bold: text => theme.bold(text),
		get success() {
			return theme.status.success;
		},
		get warning() {
			return theme.status.warning;
		},
		get error() {
			return theme.status.error;
		},
		get separator() {
			return theme.sep.dot;
		},
		get filled() {
			return theme.symbol("progress.filled");
		},
		get empty() {
			return theme.symbol("progress.empty");
		},
	});
	#scrollView = new ScrollView([], { height: 1, scrollbar: "never" });
	readonly #logLines: Array<string | CleanseBoardLogRenderer> = [];
	#outcome: CleansePanelOutcome | undefined;
	#errorMessage: string | undefined;
	#frame = 0;
	#timer: NodeJS.Timeout | undefined;
	#liveClosed = false;

	constructor(options: CleansePanelComponentOptions) {
		super(options.request ? `/cleanse ${replaceTabs(options.request)}` : "/cleanse");
		this.#tui = options.tui;
		this.#timer = setInterval(() => {
			this.#frame = (this.#frame + 1) % theme.getSpinnerFrames().length;
			this.#rebuild();
		}, SPINNER_INTERVAL_MS);
		this.#timer.unref?.();
		this.#rebuild();
	}

	log(text: string): void {
		this.#appendLog(text);
	}

	#appendLog(text: string | CleanseBoardLogRenderer): void {
		this.#logLines.push(text);
		if (this.#logLines.length > MAX_LOG_LINES) this.#logLines.splice(0, this.#logLines.length - MAX_LOG_LINES);
		this.#rebuild();
	}

	/** Permanent line styled as a failure (the core's stderr-equivalent). */
	logError(text: string): void {
		this.#appendLog(() => theme.fg("error", text));
	}

	phase(text: string | undefined): void {
		this.#model.phase(text);
		this.#rebuild();
	}

	checkerStarted(checker: CleanseCheckerDescriptor): void {
		this.#model.checkerStarted(checker);
		this.#rebuild();
	}

	checkerFinished(check: CleanseCheckResult, durationMs: number): void {
		this.#appendLog(this.#model.finishChecker(check, durationMs));
	}

	repairFinished(): void {
		this.#model.repairFinished();
		this.#rebuild();
	}

	agentStarted(name: string, assignment: CleanseAssignment): void {
		this.#model.agentStarted(name, assignment);
		this.#rebuild();
	}

	agentProgress(name: string, progress: AgentProgress): void {
		this.#model.agentProgress(name, progress);
	}

	agentFinished(outcome: CleanseAgentOutcome, assignment: CleanseAssignment): void {
		this.#appendLog(this.#model.finishAgent(outcome, assignment));
	}

	/** Stop the live area; the panel stays mounted until the user dismisses it. */
	close(): void {
		this.#liveClosed = true;
		this.#stopTimer();
		this.#rebuild();
	}

	/** Record the settled run result and switch the footer to its dismiss hint. */
	finish(status: CleanseRunStatus): void {
		this.#outcome = status;
		this.close();
	}

	/** Record an unexpected failure and switch the footer to its dismiss hint. */
	markError(message: string): void {
		this.#outcome = "error";
		this.#errorMessage = message;
		this.close();
	}

	/** Release the repaint timer during teardown. */
	override dispose(): void {
		this.#stopTimer();
		super.dispose();
	}

	#stopTimer(): void {
		if (!this.#timer) return;
		clearInterval(this.#timer);
		this.#timer = undefined;
	}

	#rebuild(): void {
		this.#tui.requestComponentRender(this);
	}

	#footerLine(): string {
		const dismiss = "Esc dismiss";
		switch (this.#outcome) {
			case undefined:
				return theme.fg("muted", "Esc cancel /cleanse");
			case "clean":
				return theme.fg("success", `${dismiss}${theme.sep.dot}${theme.status.success} Clean`);
			case "unresolved":
				return theme.fg("warning", `${dismiss}${theme.sep.dot}${theme.status.warning} Diagnostics remain`);
			case "unsupported":
				return theme.fg("warning", `${dismiss}${theme.sep.dot}${theme.status.warning} No runnable checker`);
			case "cancelled":
				return theme.fg("warning", `${dismiss}${theme.sep.dot}${theme.status.warning} Cancelled`);
			case "error":
				return theme.fg("error", `${dismiss}${theme.sep.dot}${theme.status.error} Error`);
		}
	}

	override render(width: number): readonly string[] {
		const height = this.getMaxHeight();
		const footer = this.#footerLine();
		if (height === 1) return renderDialog(this.title, [footer], width, height).lines;
		const innerWidth = dialogContentWidth(width);
		const frames = theme.getSpinnerFrames();
		const live = this.#liveClosed ? [] : this.#model.renderLive(frames[this.#frame] ?? frames[0]);
		const text = this.#errorMessage
			? theme.fg("error", replaceTabs(this.#errorMessage))
			: [...this.#logLines, ...live].map(line => replaceTabs(typeof line === "string" ? line : line())).join("\n");
		const lines = wrapTextWithAnsi(text, innerWidth, { hard: true });
		const chrome = Number(height >= 3) + Number(height >= 2) + Number(height >= 6);
		this.#scrollView.setHeight(Math.max(1, Math.min(height - chrome, lines.length)));
		this.#scrollView.setLines(lines);
		this.#scrollView.setSymbols(getSymbolTheme());
		if (this.#errorMessage) this.#scrollView.scrollToTop();
		else this.#scrollView.scrollToBottom();
		return renderDialog(this.title, this.#scrollView.render(innerWidth), width, height, footer).lines;
	}
}
