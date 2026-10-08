import { CancellableLoader, Container, type TUI } from "@harvest/pi-tui";
import type { Theme } from "../../modes/theme/theme";
import { dialogContentWidth, renderDialog } from "./overlay-box";

/** Loader wrapped with borders for hook UI */
export class BorderedLoader extends Container {
	#loader: CancellableLoader;
	#maxHeight = 4;

	constructor(tui: TUI, theme: Theme, message: string) {
		super();
		this.#loader = new CancellableLoader(
			tui,
			s => theme.fg("accent", s),
			s => theme.fg("muted", s),
			message,
			theme.getSpinnerFrames(),
		);
		this.addChild(this.#loader);
	}

	setMaxHeight(height: number): void {
		this.#maxHeight = Math.max(1, Math.floor(height));
	}

	override render(width: number): readonly string[] {
		const body = this.#loader.render(dialogContentWidth(width)).filter(line => line.trim().length > 0);
		return renderDialog("Working", this.#maxHeight === 1 ? ["Esc cancel"] : body, width, this.#maxHeight, "Esc cancel").lines;
	}

	get signal(): AbortSignal {
		return this.#loader.signal;
	}

	set onAbort(fn: (() => void) | undefined) {
		this.#loader.onAbort = fn;
	}

	handleInput(data: string): void {
		this.#loader.handleInput(data);
	}

	override dispose(): void {
		this.#loader.dispose();
	}
}
