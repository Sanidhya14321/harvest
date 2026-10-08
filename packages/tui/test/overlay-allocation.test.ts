import { describe, expect, it } from "bun:test";
import { type Component, Text, TUI } from "@harvest/pi-tui";
import { VirtualTerminal } from "./virtual-terminal";

/** A consumer keeps its selection/close controls only when given its allocation. */
class ResponsiveMenu implements Component {
	#height = 40;
	#selected = 0;
	setMaxHeight(height: number): void {
		this.#height = height;
	}
	handleInput(): void {
		this.#selected++;
	}
	render(): string[] {
		const selection = `Selected action ${this.#selected}`;
		if (this.#height <= 2) return [selection, "Escape closes"];
		return ["Filter", ...Array.from({ length: this.#height - 3 }, () => "description"), selection, "Escape closes"];
	}
}

describe("overlay allocation", () => {
	it("reapplies the dialog width cap after viewport and margin changes", async () => {
		const terminal = new VirtualTerminal(120, 8);
		const ui = new TUI(terminal);
		const widths: number[] = [];
		const menu: Component = { render: width => { widths.push(width); return ["Selection"]; } };
		ui.addChild(new Text("Underlying draft"));
		ui.showOverlay(menu, { width: "100%", maxWidth: 60, maxHeight: "100%", margin: 1 });
		ui.start();
		try {
			await terminal.waitForRender();
			expect(widths.at(-1)).toBe(60);
			terminal.resize(24, 4);
			await terminal.waitForRender(() => widths.at(-1) === 22);
			expect(widths.at(-1)).toBe(22);
			terminal.resize(1, 2);
			await terminal.waitForRender(() => widths.at(-1) === 1);
			expect(widths.at(-1)).toBe(1);
			terminal.resize(120, 8);
			await terminal.waitForRender(() => widths.at(-1) === 60);
			expect(terminal.getViewport().join("\n")).toContain("Selection");
		} finally {
			ui.stop();
		}
	});
	it("retains selected and close controls through percentage/margin allocation and resize", async () => {
		const terminal = new VirtualTerminal(60, 12);
		const ui = new TUI(terminal);
		const menu = new ResponsiveMenu();
		ui.addChild(new Text("Underlying draft"));
		ui.showOverlay(menu, { width: 40, maxHeight: "50%", margin: 1 });
		ui.setFocus(menu);
		ui.start();
		try {
			await terminal.waitForRender();
			expect(terminal.getViewport().join("\n")).toContain("Selected action 0");
			expect(terminal.getViewport().join("\n")).toContain("Escape closes");
			terminal.resize(24, 4);
			await terminal.waitForRender(() => terminal.getViewport().join("\n").includes("Selected action 0"));
			terminal.sendInput("down");
			ui.requestRender();
			await terminal.waitForRender();
			const tiny = terminal.getViewport().join("\n");
			expect(tiny).toContain("Selected action 1");
			expect(tiny).toContain("Escape closes");
			terminal.resize(60, 12);
			await terminal.waitForRender(() => terminal.getViewport().join("\n").includes("Filter"));
			expect(terminal.getViewport().join("\n")).toContain("Filter");
			expect(terminal.getViewport().join("\n")).toContain("Selected action 1");
		} finally {
			ui.stop();
		}
	});
});
