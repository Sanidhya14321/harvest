import { type Component, Markdown, Spacer, Text, type TUI } from "@harvest/pi-tui";
import { replaceTabs } from "../../tools/render-utils";
import { getMarkdownTheme, getThemeEpoch, theme } from "../theme/theme";
import { dialogContentWidth, OverlayPanel, renderDialog } from "./overlay-box";

export type OmfgPanelState =
	| "generating"
	| "validating"
	| "confirming"
	| "saving"
	| "saved"
	| "rejected"
	| "aborted"
	| "error";

interface OmfgPanelComponentOptions {
	complaint: string;
	tui: TUI;
}

export class OmfgPanelComponent extends OverlayPanel {
	#tui: TUI;
	#state: OmfgPanelState = "generating";
	#status = "Generating TTSR rule";
	#preview = "";
	#savedPath: string | undefined;
	#errorMessage: string | undefined;
	#closed = false;
	#body!: Component;
	#bodyThemeEpoch = -1;

	constructor(options: OmfgPanelComponentOptions) {
		super(`/omfg ${replaceTabs(options.complaint)}`);
		this.#tui = options.tui;
		this.#rebuild();
	}

	appendDraft(delta: string): void {
		if (!delta || this.#closed) return;
		this.#preview += delta;
		this.#rebuild();
	}

	setRule(text: string): void {
		if (this.#closed) return;
		this.#preview = text;
		this.#rebuild();
	}

	setStatus(state: OmfgPanelState, status: string): void {
		if (this.#closed) return;
		this.#state = state;
		this.#status = status;
		this.#errorMessage = undefined;
		this.#rebuild();
	}

	markSaved(path: string): void {
		if (this.#closed) return;
		this.#state = "saved";
		this.#savedPath = path;
		this.#status = `Saved ${path}`;
		this.#errorMessage = undefined;
		this.#rebuild();
	}

	markRejected(): void {
		if (this.#closed) return;
		this.#state = "rejected";
		this.#status = "Rule was not saved.";
		this.#errorMessage = undefined;
		this.#rebuild();
	}

	markAborted(): void {
		if (this.#closed) return;
		this.#state = "aborted";
		this.#status = "Cancelled.";
		this.#errorMessage = undefined;
		this.#rebuild();
	}

	markError(message: string): void {
		if (this.#closed) return;
		this.#state = "error";
		this.#status = "Could not create rule.";
		this.#errorMessage = message;
		this.#rebuild();
	}

	close(): void {
		this.#closed = true;
	}

	override render(width: number): readonly string[] {
		if (this.#bodyThemeEpoch !== getThemeEpoch()) {
			this.#body = this.#contentComponent();
			this.#bodyThemeEpoch = getThemeEpoch();
		}
		const height = this.getMaxHeight();
		const footer = this.#footerLine();
		const content = this.#body.render(dialogContentWidth(width));
		const body =
			height === 1 ? [footer] : height < 5 ? content : [theme.fg("muted", replaceTabs(this.#status)), ...content];
		return renderDialog(this.title, body, width, height, height > 1 ? footer : "").lines;
	}

	#rebuild(): void {
		this.clear();
		this.addChild(new Spacer(1));
		this.addChild(new Text(theme.fg("muted", replaceTabs(this.#status)), 0, 0));
		this.addChild(new Spacer(1));
		this.#body = this.#contentComponent();
		this.#bodyThemeEpoch = getThemeEpoch();
		this.addChild(this.#body);
		this.addChild(new Spacer(1));
		this.addChild(new Text(this.#footerLine(), 0, 0));
		this.#tui.requestRender();
	}

	#footerLine(): string {
		switch (this.#state) {
			case "generating":
			case "validating":
			case "confirming":
			case "saving":
				return theme.fg("muted", "Esc cancel /omfg");
			case "saved":
				return theme.fg(
					"success",
					`${theme.status.success} Registered live${theme.sep.dot}${replaceTabs(this.#savedPath ?? "saved")}${theme.sep.dot}Esc dismiss`,
				);
			case "rejected":
				return theme.fg("warning", `${theme.status.warning} Not saved${theme.sep.dot}Esc dismiss`);
			case "aborted":
				return theme.fg("warning", `${theme.status.warning} Cancelled${theme.sep.dot}Esc dismiss`);
			case "error":
				return theme.fg("error", `${theme.status.error} Error${theme.sep.dot}Esc dismiss`);
		}
	}

	#contentComponent(): Component {
		if (this.#state === "error") {
			return new Text(theme.fg("error", replaceTabs(this.#errorMessage ?? "Unknown error")), 0, 0);
		}
		const text = replaceTabs(this.#preview).trim();
		if (!text) {
			return new Text(
				theme.fg("dim", `${theme.status.pending} Waiting for candidate rule${theme.symbol("sep.ellipsis")}`),
				0,
				0,
			);
		}
		return new Markdown(text, 0, 0, getMarkdownTheme());
	}
}
