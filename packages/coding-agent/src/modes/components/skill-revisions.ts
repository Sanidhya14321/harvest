/**
 * Fullscreen-capable managed-skill revision manager: history, inspection,
 * evaluation, cancellation, promotion, rollback, conflict and restriction
 * display for one managed skill through the existing revision service.
 *
 * Draft/edit authoring (new revision content) stays model-callable through
 * the manage-skill tool — this surface never invents content. Unevaluated
 * promotion honors the explicit disclosure contract. Invocation wiring
 * (palette entry) is a patch request on the owning controller; the component
 * itself is fully operable and tested standalone.
 */

import { type Component, Input, matchesKey, replaceTabs, type TUI, truncateToWidth } from "@harvest/pi-tui";
import { EVAL_DEFAULT_MODEL_PATTERN } from "../../autolearn/eval-executor";
import {
	evaluateSkillRevision,
	listSkillRevisions,
	promoteSkillRevision,
	rollbackSkillRevision,
	sanitizeSkillName,
} from "../../autolearn/managed-skills";
import type { ToolSession } from "../../tools/index";
import { matchesSelectCancel, matchesSelectDown, matchesSelectUp } from "../utils/keybinding-matchers";
import { theme } from "../theme/theme";
import {
	formatEvaluationLines,
	formatPromotedNotice,
	revisionEvalStatus,
	type RevisionEvalStatus,
} from "./revision-views";

/**
 * Production evaluation context for skill runs started here. Same contract
 * as the hub: the host threads the owning production ToolSession plus an
 * AbortSignal (see the patch request on the palette/selector owner). Without
 * a parent, evaluations render unavailable instead of running unscoped.
 */
export interface SkillRevisionsEvalContext {
	/** Owning production session: model/registry/credentials/restrictions source. */
	parent?: ToolSession;
	/** Caller abort: aborts the run, records nothing, never passes/promotes. */
	signal?: AbortSignal;
}

export interface SkillRevisionsCallbacks {
	onClose: () => void;
}

/** One skill revision as this manager displays it. */
export interface SkillRevisionView {
	id: string;
	state: string;
	active: boolean;
	evaluations: number;
	evalStatus: RevisionEvalStatus;
}

/**
 * Managed-skill revision manager for one skill. The host must call
 * {@link SkillRevisionsComponent.dispose} when the overlay closes.
 */
export class SkillRevisionsComponent implements Component {
	#tui: TUI;
	#name: string;
	#evalContext: SkillRevisionsEvalContext;
	#callbacks: SkillRevisionsCallbacks;

	#items: SkillRevisionView[] = [];
	#active: string | null = null;
	#index = 0;
	#scroll = 0;
	#error: string | null = null;
	#notice: string | null = null;
	#actionError: string | null = null;

	#evalInput: { revId: string; step: "task" | "outcome"; task: string; input: Input } | null = null;
	#running: { revId: string; generation: number; cancel: () => void } | null = null;
	#generation = 0;
	#inspecting: { revId: string; error: string | null; body: string[] } | null = null;

	private constructor(
		tui: TUI,
		name: string,
		evalContext: SkillRevisionsEvalContext,
		callbacks: SkillRevisionsCallbacks,
	) {
		this.#tui = tui;
		this.#name = name;
		this.#evalContext = evalContext;
		this.#callbacks = callbacks;
	}

	static async create(
		tui: TUI,
		skillName: string,
		evalContext: SkillRevisionsEvalContext = {},
		callbacks: SkillRevisionsCallbacks = { onClose: () => {} },
	): Promise<SkillRevisionsComponent> {
		const component = new SkillRevisionsComponent(tui, sanitizeSkillName(skillName), evalContext, callbacks);
		await component.#reload();
		return component;
	}

	dispose(): void {
		// Fence late evaluation callbacks on teardown.
		this.#generation++;
		this.#running = null;
	}
	invalidate(): void {}

	#selected(): SkillRevisionView | undefined {
		return this.#items[this.#index];
	}

	async #reload(): Promise<void> {
		this.#error = null;
		try {
			const { active, revisions } = await listSkillRevisions(this.#name);
			this.#items = revisions.map(rev => ({
				id: rev.id,
				state: rev.state,
				active: rev.id === active,
				evaluations: rev.evaluations.length,
				evalStatus: revisionEvalStatus(rev.evaluations),
			}));
			this.#active = active;
			this.#index = Math.max(0, Math.min(this.#index, Math.max(0, this.#items.length - 1)));
		} catch (error) {
			this.#items = [];
			this.#error = error instanceof Error ? error.message : String(error);
		}
		this.#tui.requestRender();
	}

	// ── Evaluation ─────────────────────────────────────────────────────

	#beginEval(revId: string): void {
		const input = new Input();
		this.#evalInput = { revId, step: "task", task: "", input };
		this.#actionError = null;
	}

	#advanceEval(): void {
		const evalState = this.#evalInput;
		if (!evalState) return;
		if (evalState.step === "task") {
			const task = evalState.input.getValue().trim();
			if (!task) {
				this.#actionError = "Evaluation needs an explicit task.";
				this.#tui.requestRender();
				return;
			}
			this.#evalInput = { ...evalState, step: "outcome", task, input: new Input() };
			this.#actionError = null;
			this.#tui.requestRender();
			return;
		}
		const expectedOutcome = evalState.input.getValue().trim();
		if (!expectedOutcome) {
			this.#actionError = "Evaluation needs an explicit expected outcome.";
			this.#tui.requestRender();
			return;
		}
		const { revId, task } = evalState;
		this.#evalInput = null;
		this.#startEval(revId, task, expectedOutcome);
	}

	#startEval(revId: string, task: string, expectedOutcome: string): void {
		this.#generation++;
		const generation = this.#generation;
		const runController = new AbortController();
		const ownerSignal = this.#evalContext.signal;
		const forwardAbort =
			ownerSignal && !ownerSignal.aborted ? () => runController.abort(ownerSignal.reason) : undefined;
		if (ownerSignal?.aborted) {
			runController.abort(ownerSignal.reason);
		} else if (forwardAbort && ownerSignal) {
			ownerSignal.addEventListener("abort", forwardAbort, { once: true });
		}
		const parent = this.#evalContext.parent;
		const cleanup = (): void => {
			if (forwardAbort && ownerSignal) ownerSignal.removeEventListener("abort", forwardAbort);
		};
		this.#actionError = null;
		this.#notice = `Evaluating skill ${this.#name} revision ${revId}…`;
		this.#running = { revId, generation, cancel: () => runController.abort() };
		this.#tui.requestRender();
		void evaluateSkillRevision(this.#name, revId, {
			task,
			expectedOutcome,
			parent,
			signal: runController.signal,
			sessionId: parent?.getSessionId?.() ?? undefined,
		})
			.then(async result => {
				cleanup();
				await this.#reload();
				if (this.#generation !== generation) return;
				this.#running = null;
				this.#actionError = null;
				this.#notice =
					`Evaluation of skill ${this.#name} revision ${revId} ` +
					`${result.passed ? "passed" : "FAILED"}: ${result.summary}`;
				this.#tui.requestRender();
			})
			.catch(async error => {
				cleanup();
				await this.#reload();
				const message = error instanceof Error ? error.message : String(error);
				if (/abort/i.test(message)) {
					if (this.#running?.generation === generation) this.#running = null;
					this.#actionError = null;
					this.#notice = `Evaluation of skill ${this.#name} revision ${revId} cancelled — nothing recorded.`;
					this.#tui.requestRender();
					return;
				}
				if (this.#generation !== generation) return;
				this.#running = null;
				if (/parent session context/i.test(message)) {
					this.#actionError =
						`Evaluation unavailable: no parent session is wired for this view. ` +
						`Production evaluations run the revision through restricted task execution, which needs ` +
						`the owning session's model, credentials, and restrictions — the revision stays unevaluated.`;
				} else {
					this.#actionError = message;
				}
				this.#tui.requestRender();
			});
	}

	#cancelEval(): void {
		const running = this.#running;
		if (!running) {
			this.#notice = "No evaluation is running.";
			this.#tui.requestRender();
			return;
		}
		running.cancel();
	}

	// ── Promote / rollback ─────────────────────────────────────────────

	async #promoteSelected(): Promise<void> {
		const rev = this.#selected();
		if (!rev) return;
		try {
			// Explicit disclosure contract: unevaluated content activates only
			// with the flag, and the notice says so.
			const result = await promoteSkillRevision(this.#name, rev.id, { discloseUnevaluated: true });
			await this.#reload();
			this.#actionError = null;
			this.#notice = formatPromotedNotice("skill", this.#name, result.revId, result.disclosedUnevaluated);
		} catch (error) {
			this.#actionError = error instanceof Error ? error.message : String(error);
		}
		this.#tui.requestRender();
	}

	async #rollbackSelected(): Promise<void> {
		const rev = this.#selected();
		if (!rev) return;
		try {
			await rollbackSkillRevision(this.#name, rev.id);
			await this.#reload();
			this.#actionError = null;
			this.#notice = `Rolled back managed skill ${this.#name} to revision ${rev.id}`;
		} catch (error) {
			this.#actionError = error instanceof Error ? error.message : String(error);
		}
		this.#tui.requestRender();
	}

	// ── Inspection ─────────────────────────────────────────────────────

	async #inspectSelected(): Promise<void> {
		const rev = this.#selected();
		if (!rev) return;
		const revId = rev.id;
		this.#inspecting = { revId, error: null, body: [] };
		this.#tui.requestRender();
		try {
			const { active, revisions } = await listSkillRevisions(this.#name);
			const current = this.#inspecting;
			if (!current || current.revId !== revId) return;
			const found = revisions.find(entry => entry.id === revId);
			if (!found) {
				current.error = `Revision ${revId} for skill "${this.#name}" not found.`;
			} else {
				const lines: string[] = [];
				lines.push(`revision ${found.id} (${found.state})${found.id === active ? " [active]" : ""}`);
				lines.push(`description: ${found.description}`);
				const provenance = [`actor=${found.provenance.actor}`];
				if (found.provenance.sessionId) provenance.push(`session=${found.provenance.sessionId}`);
				if (found.provenance.runId) provenance.push(`run=${found.provenance.runId}`);
				lines.push(`provenance: ${provenance.join(" ")}`);
				// Skills declare no tools/model of their own: the evaluator
				// always runs the fixed restricted set below.
				lines.push(
					`effective restrictions: tools=[read, glob, grep] model=${EVAL_DEFAULT_MODEL_PATTERN} (skill defaults)`,
				);
				lines.push(`evaluations (${found.evaluations.length}):`);
				for (const evaluation of found.evaluations) {
					lines.push(...formatEvaluationLines(evaluation));
				}
				lines.push(`content (${Buffer.byteLength(found.content, "utf8")} bytes):`);
				for (const contentLine of found.content.split("\n").slice(0, 8)) {
					lines.push(`  ${contentLine}`);
				}
				current.body = lines;
			}
		} catch (error) {
			const current = this.#inspecting;
			if (current && current.revId === revId) {
				current.error = error instanceof Error ? error.message : String(error);
			}
		}
		this.#tui.requestRender();
	}

	// ── Input ──────────────────────────────────────────────────────────

	handleInput(data: string): void {
		if (this.#evalInput) {
			if (matchesSelectCancel(data)) {
				this.#evalInput = null;
				this.#actionError = null;
			} else if (matchesKey(data, "enter") || matchesKey(data, "return") || data === "\n") {
				this.#advanceEval();
			} else {
				this.#evalInput.input.handleInput(data);
			}
			this.#tui.requestRender();
			return;
		}
		if (this.#inspecting) {
			if (matchesSelectCancel(data)) this.#inspecting = null;
			this.#tui.requestRender();
			return;
		}
		if (matchesSelectCancel(data)) {
			this.#callbacks.onClose();
			return;
		}
		if (matchesSelectUp(data)) {
			this.#index = Math.max(0, this.#index - 1);
			this.#tui.requestRender();
			return;
		}
		if (matchesSelectDown(data)) {
			this.#index = Math.min(Math.max(0, this.#items.length - 1), this.#index + 1);
			this.#tui.requestRender();
			return;
		}
		const key = data.toLowerCase();
		if (key === "r") {
			void this.#reload();
			return;
		}
		const rev = this.#selected();
		if (!rev) return;
		if (key === "e") {
			this.#beginEval(rev.id);
			this.#tui.requestRender();
		} else if (key === "p") {
			void this.#promoteSelected();
		} else if (key === "b") {
			void this.#rollbackSelected();
		} else if (key === "i") {
			void this.#inspectSelected();
		} else if (key === "x") {
			this.#cancelEval();
		}
	}

	// ── Render ─────────────────────────────────────────────────────────

	#headerRow(width: number): string {
		if (this.#error) return truncateToWidth(theme.fg("error", ` ${replaceTabs(this.#error)}`), width);
		if (this.#actionError) return truncateToWidth(theme.fg("error", ` ${replaceTabs(this.#actionError)}`), width);
		if (this.#notice) return truncateToWidth(theme.fg("success", ` ${replaceTabs(this.#notice)}`), width);
		const active = this.#active ? `active ${this.#active}` : "no active revision";
		return truncateToWidth(theme.fg("accent", ` Skill revisions for ${theme.bold(this.#name)} — ${active}`), width);
	}

	render(width: number): string[] {
		const height = Math.max(6, this.#tui.terminal?.rows || process.stdout.rows || 40);
		const rows = Math.max(1, height - 3);
		const lines: string[] = [this.#headerRow(width)];
		const inspecting = this.#inspecting;
		if (inspecting) {
			if (inspecting.error) {
				lines.push(truncateToWidth(theme.fg("error", ` ${replaceTabs(inspecting.error)}`), width));
			} else if (inspecting.body.length === 0) {
				lines.push(truncateToWidth(theme.fg("dim", " Loading revision…"), width));
			} else {
				for (const bodyLine of inspecting.body.slice(0, Math.max(1, rows - 1))) {
					lines.push(truncateToWidth(` ${replaceTabs(bodyLine)}`, width));
				}
			}
		} else {
			if (this.#items.length === 0 && !this.#error) {
				lines.push(
					truncateToWidth(theme.fg("dim", " No revisions yet — mint a draft via the manage-skill tool"), width),
				);
			}
			const visibleRows = Math.max(1, rows - 1);
			if (this.#index < this.#scroll) this.#scroll = this.#index;
			else if (this.#index >= this.#scroll + visibleRows) this.#scroll = this.#index - visibleRows + 1;
			this.#scroll = Math.max(0, Math.min(this.#scroll, Math.max(0, this.#items.length - visibleRows)));
			for (let i = this.#scroll; i < Math.min(this.#items.length, this.#scroll + visibleRows); i++) {
				const rev = this.#items[i];
				if (!rev) continue;
				const selected = i === this.#index;
				const cursor = selected ? theme.fg("accent", theme.nav.cursor) : " ";
				const running = this.#running?.revId === rev.id;
				const evalStyled = running
					? theme.fg("accent", "evaluating…")
					: rev.evalStatus === "passing"
						? theme.fg("success", `passing (${rev.evaluations})`)
						: rev.evalStatus === "failing"
							? theme.fg("error", `failing (${rev.evaluations})`)
							: theme.fg("dim", "unevaluated");
				const activeMark = rev.active ? theme.fg("success", " [active]") : "";
				const idStyled = rev.active ? theme.bold(theme.fg("accent", rev.id)) : rev.id;
				lines.push(
					truncateToWidth(
						` ${cursor} ${idStyled}  ${theme.fg("muted", rev.state)}  ${evalStyled}${activeMark}`,
						width,
					),
				);
			}
		}
		while (lines.length < rows) lines.push("");
		const footer = this.#evalInput
			? this.#evalInput.step === "task"
				? "Enter task · Esc back (evaluation task for this revision)"
				: "Enter run evaluation · Esc back (expected observable outcome)"
			: this.#inspecting
				? "Esc back to revisions"
				: "↑/↓ select · e evaluate · p promote · b rollback · i inspect · x cancel run · r reload · Esc close";
		lines.push(truncateToWidth(theme.fg("dim", footer), width));
		return lines.slice(0, rows + 1);
	}
}
