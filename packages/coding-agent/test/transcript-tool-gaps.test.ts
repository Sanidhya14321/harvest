/**
 * Focused geometry + generic-fallback quiet coverage for transcript/tool gaps.
 *
 * - Ask markdown renders at the framed content width (no +1 overflow column).
 * - Eval plain-text fallback truncates at the content width.
 * - Task assignment markdown aligns to width-4 (the framed content inset).
 * - Generic tool fallback settles quietly on success (rail/inline, no rounded
 *   frame) while errors/running keep their chrome.
 */
import { describe, expect, it } from "bun:test";
import type { Component } from "@harvest/pi-tui";
import { visibleWidth } from "@harvest/pi-tui";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import { ToolExecutionComponent, type ToolExecutionUi } from "@harvest/pi-coding-agent/modes/components/tool-execution";
import { getThemeByName, setThemeInstance, theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { taskToolRenderer } from "@harvest/pi-coding-agent/task/renderer";
import { askToolRenderer } from "@harvest/pi-coding-agent/tools/ask";
import { evalToolRenderer } from "@harvest/pi-coding-agent/tools/eval-render";
import { outputBlockContentWidth } from "@harvest/pi-coding-agent/tui/output-block";

const WIDTH = 80;

function strippedLines(lines: readonly string[]): string[] {
	return lines.map(line => Bun.stripANSI(line));
}

function maxVisibleWidth(lines: readonly string[]): number {
	let max = 0;
	for (const line of lines) max = Math.max(max, visibleWidth(Bun.stripANSI(line)));
	return max;
}

const ui: ToolExecutionUi = {
	requestRender() {},
	requestComponentRender(_component: Component) {},
	resetDisplay() {},
};

describe("transcript geometry widths", () => {
	it("frames content at width-4", () => {
		expect(outputBlockContentWidth(80)).toBe(76);
		expect(outputBlockContentWidth(80, 0)).toBe(78);
		expect(outputBlockContentWidth(80, 1, 1)).toBe(76);
	});

	it("ask markdown stays within the frame width", async () => {
		await Settings.init({ inMemory: true });
		const loaded = await getThemeByName("dark");
		if (!loaded) throw new Error("theme unavailable");
		setThemeInstance(loaded);
		const component = askToolRenderer.renderCall(
			{
				question: "Which authentication method should this API use for all of its public endpoints and webhooks?",
				options: [
					{ label: "JWT bearer tokens for stateless API clients everywhere" },
					{ label: "OAuth2 delegated authorization with external identity providers" },
				],
			} as never,
			{ expanded: false, isPartial: false },
			theme,
		);
		const lines = strippedLines(component.render(WIDTH));
		expect(lines.length).toBeGreaterThan(0);
		expect(maxVisibleWidth(lines)).toBeLessThanOrEqual(WIDTH);
	});

	it("eval plain-text fallback truncates at the content width", async () => {
		await Settings.init({ inMemory: true });
		const loaded = await getThemeByName("dark");
		if (!loaded) throw new Error("theme unavailable");
		setThemeInstance(loaded);
		const longLine = `output:${"x".repeat(200)}`;
		const component = evalToolRenderer.renderResult(
			{ content: [{ type: "text", text: longLine }] },
			{ expanded: false, isPartial: false },
			theme,
		);
		const lines = strippedLines(component.render(WIDTH));
		expect(lines.length).toBeGreaterThan(0);
		// Inside the tool block's content inset: never wider than the framed width.
		expect(maxVisibleWidth(lines)).toBeLessThanOrEqual(outputBlockContentWidth(WIDTH));
	});

	it("task assignment markdown aligns to the framed content width", async () => {
		await Settings.init({ inMemory: true });
		const loaded = await getThemeByName("dark");
		if (!loaded) throw new Error("theme unavailable");
		setThemeInstance(loaded);
		const component = taskToolRenderer.renderCall(
			{ task: "Survey the repository layout, list every top-level package, and summarize each." } as never,
			{ expanded: false, isPartial: true },
			theme,
		);
		const lines = strippedLines(component.render(WIDTH));
		expect(lines.length).toBeGreaterThan(0);
		expect(maxVisibleWidth(lines)).toBeLessThanOrEqual(WIDTH);
	});
});

describe("generic fallback quiet success", () => {
	it("renders settled output as a frameless rail without a rounded frame", async () => {
		await Settings.init({ inMemory: true });
		const loaded = await getThemeByName("dark");
		if (!loaded) throw new Error("theme unavailable");
		setThemeInstance(loaded);
		const component = new ToolExecutionComponent(
			"mcp_widget",
			{ query: "context" },
			{ showImages: false },
			undefined,
			ui,
			process.cwd(),
		);
		component.updateResult({ content: [{ type: "text", text: "line1\nline2" }] }, false);
		const rendered = strippedLines(component.render(WIDTH)).join("\n");
		expect(rendered).toContain("line1");
		expect(rendered).toContain("line2");
		expect(rendered).not.toContain("╭─");
	});

	it("collapses output-less success to one inline row", async () => {
		await Settings.init({ inMemory: true });
		const loaded = await getThemeByName("dark");
		if (!loaded) throw new Error("theme unavailable");
		setThemeInstance(loaded);
		const component = new ToolExecutionComponent(
			"mcp_widget",
			{},
			{ showImages: false },
			undefined,
			ui,
			process.cwd(),
		);
		component.updateResult({ content: [] }, false);
		const lines = strippedLines(component.render(WIDTH));
		expect(lines).toHaveLength(1);
		expect(lines[0]).toContain("mcp_widget");
		expect(lines.join("\n")).not.toContain("╭─");
	});

	it("keeps error and running chrome instead of quieting them", async () => {
		await Settings.init({ inMemory: true });
		const loaded = await getThemeByName("dark");
		if (!loaded) throw new Error("theme unavailable");
		setThemeInstance(loaded);
		const failed = new ToolExecutionComponent("mcp_widget", {}, { showImages: false }, undefined, ui, process.cwd());
		failed.updateResult({ content: [{ type: "text", text: "boom failed" }], isError: true }, false);
		expect(strippedLines(failed.render(WIDTH)).join("\n")).toContain("boom failed");

		const running = new ToolExecutionComponent(
			"mcp_widget",
			{ query: "context" },
			{ showImages: false },
			undefined,
			ui,
			process.cwd(),
		);
		expect(strippedLines(running.render(WIDTH)).join("\n")).toContain("mcp_widget");
	});
});
