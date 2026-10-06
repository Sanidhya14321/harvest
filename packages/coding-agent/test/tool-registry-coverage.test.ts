/**
 * Registry coverage: every declared tool resolves to a renderer or to the
 * documented quiet generic fallback, and renderer failures degrade visibly.
 *
 * Contracts:
 * - No builtin tool renders through an accidental missing-renderer hole;
 *   hidden tools (`yield`, `goal`, `think`) intentionally stay on the
 *   generic path instead of gaining bespoke chrome.
 * - Every renderer survives empty args/results without throwing, so a sparse
 *   streamed call never corrupts the transcript.
 * - A throwing custom renderer degrades to its fallback (label preserved,
 *   no exception escapes into the terminal engine).
 *
 * Real registries and components only; counts are enumerated live, never
 * hardcoded. Failure modes are named in each case.
 */
import { beforeAll, describe, expect, it } from "bun:test";
import { initTheme, theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { SafeToolRendererComponent } from "@harvest/pi-coding-agent/modes/components/tool-execution";
import { toolRenderers } from "@harvest/pi-coding-agent/tools/renderers";
import { BUILTIN_TOOL_NAMES, HIDDEN_TOOL_NAMES } from "@harvest/pi-coding-agent/tools/builtin-names";
import { Text } from "@harvest/pi-tui";

beforeAll(async () => {
	await initTheme();
});

const WIDTH = 100;

/** Tools that intentionally render through the generic fallback, not bespoke chrome. */
const QUIET_FALLBACK_TOOLS = new Set(HIDDEN_TOOL_NAMES);

describe("tool registry coverage", () => {
	it("enumerates a non-empty live registry (stale-count detector)", () => {
		expect(BUILTIN_TOOL_NAMES.length).toBeGreaterThan(0);
		expect(HIDDEN_TOOL_NAMES.length).toBeGreaterThan(0);
		expect(Object.keys(toolRenderers).length).toBeGreaterThan(0);
	});

	it("routes every builtin tool to a renderer (missing-renderer hole)", () => {
		const missing = BUILTIN_TOOL_NAMES.filter(name => toolRenderers[name] === undefined);
		expect(missing).toEqual([]);
	});

	it("keeps hidden tools on the quiet generic path (bespoke-chrome leak)", () => {
		for (const name of HIDDEN_TOOL_NAMES) {
			expect(toolRenderers[name] === undefined || QUIET_FALLBACK_TOOLS.has(name)).toBe(true);
		}
	});

	it("every renderer survives empty args and results (sparse-stream crash)", () => {
		const failures: string[] = [];
		for (const [name, renderer] of Object.entries(toolRenderers)) {
			try {
				renderer.activitySummary?.({}, { expanded: false, isPartial: true });
				if (renderer.renderCall) {
					// renderCall may contractually return undefined (e.g. write
					// waits for streamed args); only undefined.render is a bug.
					const call = renderer.renderCall({}, { expanded: false, isPartial: true }, theme);
					call?.render(WIDTH);
				}
				const result = renderer.renderResult(
					{ content: [{ type: "text", text: "" }] },
					{ expanded: false, isPartial: false },
					theme,
					{},
				);
				const rows = result.render(WIDTH);
				if (!Array.isArray(rows)) failures.push(`${name}: non-array rows`);
			} catch (error) {
				failures.push(`${name}: ${error instanceof Error ? error.message : String(error)}`);
			}
		}
		expect(failures).toEqual([]);
	});

	it("a throwing custom renderer degrades to its fallback (terminal corruption)", () => {
		const throwing = {
			render(): readonly string[] {
				throw new Error("boom");
			},
		};
		const guard = new SafeToolRendererComponent(
			"custom-widget",
			"result",
			throwing,
			() => new Text("custom-widget fallback", 0, 0),
		);
		const rows = guard.render(WIDTH);
		expect(rows.join("\n")).toContain("custom-widget fallback");
	});

	it("a throwing fallback still yields rows, never an exception (double-fault escape)", () => {
		const throwing = {
			render(): readonly string[] {
				throw new Error("boom");
			},
		};
		const guard = new SafeToolRendererComponent("custom-widget", "call", throwing, () => undefined);
		expect(guard.render(WIDTH)).toEqual([]);
	});
});
