/**
 * Simple text input component for hooks.
 */
import { type Focusable, Input, matchesKey, Spacer, Text, type TUI } from "@harvest/pi-tui";
import { theme } from "../../modes/theme/theme";
import { matchesAppInterrupt } from "../../modes/utils/keybinding-matchers";
import { CountdownTimer } from "./countdown-timer";
import { editorKey } from "./keybinding-hints";
import { OverlayPanel, renderDialog } from "./overlay-box";

export interface HookInputOptions {
	tui?: TUI;
	timeout?: number;
	onTimeout?: () => void;
}

export class HookInputComponent extends OverlayPanel implements Focusable {
	focused = false;
	#input: Input;
	#onSubmitCallback: (value: string) => void;
	#onCancelCallback: () => void;
	#baseTitle: string;
	#countdown: CountdownTimer | undefined;
	#placeholder: string | undefined;

	constructor(
		title: string,
		placeholder: string | undefined,
		onSubmit: (value: string) => void,
		onCancel: () => void,
		opts?: HookInputOptions,
	) {
		super(title);

		this.#onSubmitCallback = onSubmit;
		this.#onCancelCallback = onCancel;
		this.#baseTitle = title;
		this.#placeholder = placeholder;

		this.addChild(new Spacer(1));

		if (opts?.timeout && opts.timeout > 0 && opts.tui) {
			this.#countdown = new CountdownTimer(
				opts.timeout,
				opts.tui,
				s => (this.title = `${this.#baseTitle} (${s}s)`),
				() => {
					opts.onTimeout?.();
					this.#onCancelCallback();
				},
			);
		}

		this.#input = new Input();
		this.addChild(this.#input);
		this.addChild(new Spacer(1));
		this.addChild(new Text(theme.fg("dim", "enter submit  esc cancel"), 0, 0));
		this.addChild(new Spacer(1));
	}

	override render(width: number): readonly string[] {
		this.#input.focused = this.focused;
		const body = [...this.#input.render(Math.max(1, width - 4))];
		const hint = `${editorKey("app.interrupt") || "Esc"} cancel · Enter submit`;
		const height = this.getMaxHeight();
		if (this.#placeholder && height >= 5) body.unshift(theme.fg("muted", this.#placeholder));
		return renderDialog(this.title, body, width, height, hint, body.length - 1).lines;
	}

	handleInput(keyData: string): void {
		// Reset countdown on any interaction
		this.#countdown?.reset();
		if (matchesKey(keyData, "enter") || matchesKey(keyData, "return") || keyData === "\n") {
			this.#onSubmitCallback(this.#input.getValue());
		} else if (matchesAppInterrupt(keyData)) {
			this.#onCancelCallback();
		} else {
			this.#input.handleInput(keyData);
		}
	}

	/** Route non-bracketed paste transports (e.g. kitty's OSC 5522 enhanced clipboard)
	 *  into the inner input, mirroring bracketed-paste semantics. Pasting counts as
	 *  interaction, so the timeout countdown resets like any keystroke. */
	pasteText(text: string): void {
		this.#countdown?.reset();
		this.#input.pasteText(text);
	}

	override dispose(): void {
		this.#countdown?.dispose();
	}
}
