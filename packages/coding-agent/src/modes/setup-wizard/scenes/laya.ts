import {
	routeSelectListMouse,
	type SelectItem,
	SelectList,
	type SgrMouseEvent,
	truncateToWidth,
} from "@harvest/pi-tui";
import { configureLayaLocally, type LayaSetupStep } from "../../../core/harvest/laya-service";
import { getSelectListTheme, theme } from "../../theme/theme";
import type { SetupScene, SetupSceneController, SetupSceneHost } from "./types";

const LAYA_CHOICES: readonly SelectItem[] = [
	{
		value: "configure",
		label: "Yes, configure Laya locally (Recommended)",
		description: "Set up local Python sidecar with convaiinnovations/laya-typed-decisions (no API cost)",
	},
	{
		value: "skip",
		label: "No, skip Laya for now",
		description: "Use default full-LLM reasoning and standard approval prompts without a local model",
	},
];

class LayaSceneController implements SetupSceneController {
	title = "Configure Laya";
	subtitle = "Use local typed decisions for tool gating, model routing, and completion checks.";

	#selectList: SelectList;
	#configuring = false;
	#done = false;
	#error: string | undefined;
	#steps: LayaSetupStep[] = [
		{ id: "python", label: "Detect Python 3.9+ runtime", status: "pending" },
		{ id: "dependencies", label: "Verify Python dependencies (laya, torch, fastapi, uvicorn)", status: "pending" },
		{ id: "model", label: "Verify single-model checkpoint (convaiinnovations/laya-typed-decisions)", status: "pending" },
		{ id: "sidecar", label: "Connect to local sidecar daemon (127.0.0.1:8177)", status: "pending" },
		{ id: "connect", label: "Connect to Harvest settings", status: "pending" },
	];
	#listRowStart = 0;

	constructor(private readonly host: SetupSceneHost) {
		this.#selectList = new SelectList(LAYA_CHOICES, LAYA_CHOICES.length, getSelectListTheme());
		this.#selectList.setSelectedIndex(0);

		this.#selectList.onSelect = item => {
			if (item.value === "configure") {
				void this.#runConfiguration();
			} else {
				void this.#skip();
			}
		};

		this.#selectList.onCancel = () => {
			if (this.#configuring && !this.#done && !this.#error) return;
			void this.#skip();
		};
	}

	invalidate(): void {
		this.#selectList.invalidate();
	}

	handleInput(data: string): void {
		if (this.#configuring) {
			if (this.#done) {
				// Any confirmation key finishes
				if (data === "\r" || data === "\n" || data === " ") {
					this.host.finish("done");
				}
				return;
			}
			if (this.#error) {
				if (data === "r" || data === "R") {
					this.#error = undefined;
					void this.#runConfiguration();
					return;
				}
				if (data === "\x1b" || data === "\r" || data === "\n") {
					void this.#skip();
					return;
				}
			}
			return;
		}

		if (data === "1") {
			this.#selectList.setSelectedIndex(0);
			void this.#runConfiguration();
			return;
		}
		if (data === "2") {
			this.#selectList.setSelectedIndex(1);
			void this.#skip();
			return;
		}

		this.#selectList.handleInput(data);
	}

	routeMouse(event: SgrMouseEvent, line: number, _col: number): void {
		if (this.#configuring) return;
		const listLine = line - this.#listRowStart;
		routeSelectListMouse(this.#selectList, event, listLine);
	}

	render(width: number, _maxLines?: number): readonly string[] {
		const lines: string[] = [];

		if (!this.#configuring) {
			lines.push(
				theme.fg(
					"muted",
					"Laya runs ModernBERT-large locally to gate risky tool calls (rm/chmod), route prompts, and check step completion.",
				),
				"",
			);
			this.#listRowStart = lines.length;
			lines.push(...this.#selectList.render(width));
			lines.push(
				"",
				theme.fg("dim", "Enter confirms highlighted choice · Esc skips"),
			);
			return lines.map(line => truncateToWidth(line, width));
		}

		// Configuring / Progress view
		lines.push(theme.bold(theme.fg("accent", "Setting up local Laya decision layer...")), "");

		for (const step of this.#steps) {
			let icon: string;
			let stepStyle: (text: string) => string;

			switch (step.status) {
				case "done":
					icon = theme.fg("success", theme.status.success);
					stepStyle = t => theme.fg("text", t);
					break;
				case "running":
					icon = theme.fg("accent", "⠋");
					stepStyle = t => theme.fg("accent", t);
					break;
				case "error":
					icon = theme.fg("error", theme.status.error);
					stepStyle = t => theme.fg("error", t);
					break;
				default:
					icon = theme.fg("dim", "○");
					stepStyle = t => theme.fg("dim", t);
					break;
			}

			const label = stepStyle(step.label);
			lines.push(`  ${icon}  ${label}`);
			if (step.error) {
				lines.push(`      ${theme.fg("error", step.error)}`);
			}
		}

		lines.push("");

		if (this.#done) {
			lines.push(
				theme.fg("success", `${theme.status.success} Laya is configured, verified, and connected to Harvest!`),
				"",
				theme.bold("Press Enter to continue"),
			);
		} else if (this.#error) {
			lines.push(
				theme.fg("error", `Configuration halted: ${this.#error}`),
				"",
				theme.fg("dim", "Press 'r' to retry, or Enter/Esc to skip Laya"),
			);
		} else {
			lines.push(theme.fg("dim", "Please wait while local inference dependencies and model weights are verified..."));
		}

		return lines.map(line => truncateToWidth(line, width));
	}

	async #runConfiguration(): Promise<void> {
		this.#configuring = true;
		this.#done = false;
		this.#error = undefined;

		// Reset steps to pending
		for (const step of this.#steps) {
			step.status = "pending";
			step.error = undefined;
		}
		this.host.requestRender();

		const result = await configureLayaLocally({
			settings: this.host.ctx.settings,
			onStepUpdate: (stepId, status, message) => {
				const step = this.#steps.find(s => s.id === stepId);
				if (step) {
					step.status = status;
					if (message && status === "done") {
						step.label = message;
					}
					if (status === "error" && message) {
						step.error = message;
					}
				}
				this.host.requestRender();
			},
		});

		if (result.success) {
			this.#done = true;
			this.host.requestRender();
			// Auto advance after 1.5 seconds if user hasn't pressed Enter
			setTimeout(() => {
				this.host.finish("done");
			}, 1500);
		} else {
			this.#error = result.error || "Failed to configure Laya";
			this.host.requestRender();
		}
	}

	async #skip(): Promise<void> {
		try {
			this.host.ctx.settings.set("laya.enabled", false);
			await this.host.ctx.settings.flush();
		} finally {
			this.host.finish("skipped");
		}
	}
}

export const layaSetupScene: SetupScene = {
	id: "laya-decision-layer",
	title: "Configure Laya",
	minVersion: 3,
	mount: host => new LayaSceneController(host),
};
