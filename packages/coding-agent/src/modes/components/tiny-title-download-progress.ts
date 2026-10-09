import { type Component, replaceTabs } from "@harvest/pi-tui";
import { formatBytes } from "@harvest/pi-utils";
import { getTinyTitleModelSpec, type TinyTitleLocalModelKey } from "../../tiny/models";
import type { TinyTitleProgressEvent } from "../../tiny/title-protocol";
import { theme } from "../theme/theme";
import { dialogContentWidth, renderDialog } from "./overlay-box";

const DEFAULT_BAR_WIDTH = 24;

function progressBar(progress: number | undefined, width: number): string {
	const barWidth = Math.max(0, Math.min(DEFAULT_BAR_WIDTH, width));
	if (progress === undefined) return theme.fg("muted", theme.progress.empty.repeat(barWidth));
	const ratio = Math.max(0, Math.min(1, progress / 100));
	const filled = Math.round(ratio * barWidth);
	return `${theme.fg("accent", theme.progress.filled.repeat(filled))}${theme.fg("muted", theme.progress.empty.repeat(barWidth - filled))}`;
}

function currentFile(event: TinyTitleProgressEvent | undefined): string | undefined {
	if (!event) return undefined;
	if (event.file) return event.file.split("/").at(-1) ?? event.file;
	if (event.files) {
		let largestFile: string | undefined;
		let largestLoaded = -1;
		for (const file in event.files) {
			const state = event.files[file];
			if (state.loaded <= largestLoaded || state.loaded >= state.total) continue;
			largestFile = file;
			largestLoaded = state.loaded;
		}
		return largestFile?.split("/").at(-1) ?? largestFile;
	}
	return undefined;
}

function statusLabel(event: TinyTitleProgressEvent | undefined): string {
	if (!event) return "Preparing";
	if (event.status === "error") return "Failed";
	if (event.status === "ready") return "Ready";
	if (event.status === "done") return "Downloaded";
	if (event.status === "download") return "Downloading";
	if (event.status === "progress" || event.status === "progress_total") return "Downloading";
	return "Preparing";
}

function byteLabel(event: TinyTitleProgressEvent | undefined): string | undefined {
	if (!event?.loaded || !event.total) return undefined;
	return `${formatBytes(event.loaded)} / ${formatBytes(event.total)}`;
}

export class TinyTitleDownloadProgressComponent implements Component {
	#modelKey: TinyTitleLocalModelKey;
	#event: TinyTitleProgressEvent | undefined;
	#maxHeight = 4;

	constructor(modelKey: TinyTitleLocalModelKey) {
		this.#modelKey = modelKey;
	}

	update(event: TinyTitleProgressEvent): void {
		this.#event = event;
	}

	isComplete(): boolean {
		return this.#event?.status === "ready" || this.#event?.status === "error";
	}

	invalidate(): void {
		// No cached state.
	}

	setMaxHeight(height: number): void {
		this.#maxHeight = Math.max(1, Math.floor(height));
	}

	render(width: number): readonly string[] {
		width = Math.max(1, width);
		const spec = getTinyTitleModelSpec(this.#modelKey);
		const status = statusLabel(this.#event);
		const file = currentFile(this.#event);
		const pct =
			this.#event?.progress === undefined ? "" : `${Math.floor(Math.max(0, Math.min(100, this.#event.progress)))}%`;
		const bytes = byteLabel(this.#event);
		const title = `Tiny model${theme.sep.dot}${spec.label}`;
		const statusLine = [theme.fg(this.#event?.status === "error" ? "error" : "accent", status), pct, bytes]
			.filter((part): part is string => Boolean(part))
			.join(theme.sep.dot);
		return renderDialog(
			title,
			[
				statusLine,
				progressBar(this.#event?.progress, dialogContentWidth(width)),
				...(file ? [replaceTabs(file)] : []),
			],
			width,
			Math.min(4, this.#maxHeight),
		).lines;
	}
}
