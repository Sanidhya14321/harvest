/**
 * Regression coverage for the semantic tool renderers (`checkpoint`, `rewind`,
 * `security_scan`, `memory_edit`, `learn`, `manage_skill`, `search_code`).
 *
 * Contract per tool:
 * - collapsed result shows a human summary (label + primary-argument detail),
 *   never the raw JSON arg tree the generic fallback would emit;
 * - error results render distinctly from success (no success disposition leaks
 *   into failures, and unknown effects never read as success);
 * - expanded results reveal the permitted detail (timestamps, reports,
 *   findings, hit lists).
 *
 * Real renderers and components only; failure modes are named in each case.
 */
import { beforeAll, describe, expect, it } from "bun:test";
import { initTheme, theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { toolRenderers } from "@harvest/pi-coding-agent/tools/renderers";

beforeAll(async () => {
	await initTheme();
});

const WIDTH = 100;

function collapsed(name: string, result: unknown, args?: unknown): string {
	const renderer = toolRenderers[name];
	if (!renderer) throw new Error(`missing renderer for ${name}`);
	const component = renderer.renderResult(
		result as { content: Array<{ type: string; text?: string }>; details?: unknown; isError?: boolean },
		{ expanded: false, isPartial: false },
		theme,
		args,
	);
	return Bun.stripANSI(component.render(WIDTH).join("\n"));
}

function expanded(name: string, result: unknown, args?: unknown): string {
	const renderer = toolRenderers[name];
	if (!renderer) throw new Error(`missing renderer for ${name}`);
	const component = renderer.renderResult(
		result as { content: Array<{ type: string; text?: string }>; details?: unknown; isError?: boolean },
		{ expanded: true, isPartial: false },
		theme,
		args,
	);
	return Bun.stripANSI(component.render(WIDTH).join("\n"));
}

function summary(name: string, args: unknown): { label: string; detail?: string } {
	const renderer = toolRenderers[name];
	if (!renderer) throw new Error(`missing renderer for ${name}`);
	if (!renderer.activitySummary) throw new Error(`${name} is missing its activitySummary`);
	return renderer.activitySummary(args, { expanded: false, isPartial: false });
}

describe("checkpoint renderer", () => {
	const args = { goal: "Explore auth flow before refactoring login" };
	const result = {
		content: [{ type: "text", text: "Checkpoint: Explore auth flow before refactoring login" }],
		details: { goal: "Explore auth flow before refactoring login", startedAt: "2026-10-04T00:00:00.000Z" },
	};

	it("collapsed shows the goal as prose, not the raw JSON arg tree (fallback leak)", () => {
		const out = collapsed("checkpoint", result, args);
		expect(out).toContain("Checkpoint");
		expect(out).toContain("Explore auth flow");
		expect(out).toContain("created");
		expect(out).not.toContain('"goal"');
		expect(out).not.toContain('{"goal"');
	});

	it("activitySummary names the checkpoint goal for the compact transcript row", () => {
		const { label, detail } = summary("checkpoint", args);
		expect(label).toBe("Checkpoint");
		expect(detail ?? "").toContain("Explore auth flow");
	});

	it("checkpoint failure never wears the created disposition (false success)", () => {
		const out = collapsed(
			"checkpoint",
			{ content: [{ type: "text", text: "Checkpoint already active." }], isError: true },
			args,
		);
		expect(out).toContain("already active");
		expect(out).not.toContain("created");
	});

	it("expanded reveals the checkpoint timestamp (lost scope)", () => {
		const out = expanded("checkpoint", result, args);
		expect(out).toContain("2026-10-04");
	});
});

describe("rewind renderer", () => {
	const args = { report: "Auth uses JWT sessions; login refactors around the session middleware." };
	const result = {
		content: [{ type: "text", text: "Rewind requested.\nReport captured for context replacement." }],
		details: { report: "Auth uses JWT sessions; login refactors around the session middleware.", rewound: true },
	};

	it("collapsed shows the restored disposition with a report preview, not raw JSON (fallback leak)", () => {
		const out = collapsed("rewind", result, args);
		expect(out).toContain("Rewind");
		expect(out).toContain("restored");
		expect(out).toContain("JWT");
		expect(out).not.toContain('"report"');
	});

	it("activitySummary marks the rewind as restored for the compact transcript row", () => {
		const { label, detail } = summary("rewind", args);
		expect(label).toBe("Rewind");
		expect(detail ?? "").toContain("restored");
	});

	it("rewind without a checkpoint fails distinctly instead of reading as restored (false success)", () => {
		const out = collapsed(
			"rewind",
			{
				content: [{ type: "text", text: "No active checkpoint. Create a checkpoint before calling rewind." }],
				isError: true,
			},
			args,
		);
		expect(out).toContain("No active checkpoint");
		expect(out).not.toContain("restored");
	});

	it("expanded reveals the full retained report (truncated findings)", () => {
		const out = expanded("rewind", result, args);
		expect(out).toContain("session middleware");
	});
});

describe("security_scan renderer", () => {
	const args = { action: "status", operation_id: "op-123" };
	const result = {
		content: [{ type: "text", text: "Security scan scan-42: completed; 2 finding(s)." }],
		details: {
			action: "status",
			operation: {
				operationId: "op-123",
				planId: "plan-1",
				scanId: "scan-42",
				phase: "completed",
				createdAt: "2026-10-04T00:00:00.000Z",
				updatedAt: "2026-10-04T00:01:00.000Z",
				findingCount: 2,
			},
		},
	};

	it("collapsed shows action plus finding disposition, not the raw JSON arg tree (fallback leak)", () => {
		const out = collapsed("security_scan", result, args);
		expect(out).toContain("Security Scan");
		expect(out).toContain("2 findings");
		expect(out).not.toContain('"action"');
		expect(out).not.toContain('"operation_id"');
	});

	it("activitySummary names the scan action and target for the compact transcript row", () => {
		const { label, detail } = summary("security_scan", args);
		expect(label).toBe("Security Scan");
		expect(detail ?? "").toContain("status");
		expect(detail ?? "").toContain("op-123");
	});

	it("scan lookup failure renders as an error, never with a findings disposition (false success)", () => {
		const out = collapsed(
			"security_scan",
			{ content: [{ type: "text", text: "Unknown security operation: op-999" }], isError: true },
			{ action: "status", operation_id: "op-999" },
		);
		expect(out).toContain("Unknown security operation");
		expect(out).not.toContain("findings");
	});

	it("expanded reveals the scan snapshot behind the disposition (hidden phase)", () => {
		const out = expanded("security_scan", result, args);
		expect(out).toContain("scan-42");
		expect(out).toContain("completed");
	});
});

describe("memory_edit renderer", () => {
	const args = { op: "update", id: "mem-1" };
	const result = {
		content: [{ type: "text", text: "Memory mem-1 updated in bank session (working)." }],
		details: { status: "updated", bank: "session", store: "working" },
	};

	it("collapsed shows operation plus target, not the raw JSON arg tree (fallback leak)", () => {
		const out = collapsed("memory_edit", result, args);
		expect(out).toContain("Memory Edit");
		expect(out).toContain("mem-1");
		expect(out).toContain("updated");
		expect(out).not.toContain('"op"');
	});

	it("activitySummary names the edit operation and memory id for the compact transcript row", () => {
		const { label, detail } = summary("memory_edit", args);
		expect(label).toBe("Memory Edit");
		expect(detail ?? "").toContain("update");
		expect(detail ?? "").toContain("mem-1");
	});

	it("missing memory reads as not found, never as updated (false success)", () => {
		const out = collapsed(
			"memory_edit",
			{ content: [{ type: "text", text: "Memory mem-9 was not found." }], details: { status: "not_found" } },
			{ op: "forget", id: "mem-9" },
		);
		expect(out).toContain("not found");
		expect(out).not.toContain("updated");
	});

	it("expanded reveals the bank location behind the disposition (lost scope)", () => {
		const out = expanded("memory_edit", result, args);
		expect(out).toContain("session");
	});
});

describe("learn renderer", () => {
	const args = { memory: "Prefer Bun over Node for all new scripts in this repo." };
	const result = { content: [{ type: "text", text: "Lesson stored." }], details: { skill: null } };

	it("collapsed shows the lesson preview, not the raw JSON arg tree (fallback leak)", () => {
		const out = collapsed("learn", result, args);
		expect(out).toContain("Learn");
		expect(out).toContain("Bun");
		expect(out).toContain("stored");
		expect(out).not.toContain('"memory"');
	});

	it("activitySummary previews the lesson for the compact transcript row", () => {
		const { label, detail } = summary("learn", args);
		expect(label).toBe("Learn");
		expect(detail ?? "").toContain("Bun");
	});

	it("empty lesson failure never wears the stored disposition (false success)", () => {
		const out = collapsed(
			"learn",
			{ content: [{ type: "text", text: "Lesson was empty after sanitization; nothing stored." }], isError: true },
			{ memory: "   " },
		);
		expect(out).toContain("nothing stored");
	});

	it("expanded reveals the full lesson text (truncated memory)", () => {
		const out = expanded("learn", result, args);
		expect(out).toContain("new scripts");
	});
});

describe("manage_skill renderer", () => {
	const args = { action: "create", name: "review-checklist" };
	const result = {
		content: [
			{ type: "text", text: 'Created managed skill "review-checklist" (managed-skills/review-checklist/SKILL.md).' },
		],
		details: { action: "create", name: "review-checklist" },
	};

	it("collapsed shows operation plus skill name, not the raw JSON arg tree (fallback leak)", () => {
		const out = collapsed("manage_skill", result, args);
		expect(out).toContain("Manage Skill");
		expect(out).toContain("review-checklist");
		expect(out).toContain("created");
		expect(out).not.toContain('"action"');
	});

	it("activitySummary names the skill operation for the compact transcript row", () => {
		const { label, detail } = summary("manage_skill", args);
		expect(label).toBe("Manage Skill");
		expect(detail ?? "").toContain("create");
		expect(detail ?? "").toContain("review-checklist");
	});

	it("shadowed skill refusal fails distinctly instead of reading as created (false success)", () => {
		const out = collapsed(
			"manage_skill",
			{
				content: [
					{
						type: "text",
						text: 'Cannot create managed skill "review-checklist": an authored skill of that name already exists.',
					},
				],
				isError: true,
				details: { action: "create", name: "review-checklist", shadowed: true },
			},
			args,
		);
		expect(out.replace(/\s+/g, " ")).toContain("already exists");
		expect(out).not.toContain("Created managed skill");
	});

	it("expanded reveals the skill write receipt (lost outcome)", () => {
		const out = expanded("manage_skill", result, args);
		expect(out).toContain("managed-skills/review-checklist/SKILL.md");
	});
});

describe("search_code renderer", () => {
	const args = { query: "useState", mode: "all", limit: 20 };
	const result = {
		content: [
			{
				type: "text",
				text: [
					"### Code Symbols (2 matches):",
					"- **useState** (function) at `packages/tui/src/hooks/useDebounced.ts:9`",
					"  `function useState(initial: T): [T, Setter<T>]`",
					"",
					"### Documentation Sections (1 matches):",
					"- **State hooks** (`docs/hooks.md`, score: 4.20)",
					"  Usage",
					"  > Prefer useState for local component state...",
				].join("\n"),
			},
		],
		details: { query: "useState", totalMatches: 3 },
	};

	it("collapsed shows the query plus bounded hits, not the raw JSON arg tree (fallback leak)", () => {
		const out = collapsed("search_code", result, args);
		expect(out).toContain("Search Code");
		expect(out).toContain("useState");
		expect(out).toContain("3 matches");
		expect(out).toContain("useDebounced.ts");
		expect(out).not.toContain('"query"');
	});

	it("activitySummary names the search query for the compact transcript row", () => {
		const { label, detail } = summary("search_code", args);
		expect(label).toBe("Search Code");
		expect(detail ?? "").toContain("useState");
	});

	it("invalid search fails distinctly instead of reading as matches (false success)", () => {
		const out = collapsed(
			"search_code",
			{
				content: [
					{ type: "text", text: "Search requires a non-empty query and an integer limit between 1 and 100." },
				],
				isError: true,
			},
			{ query: "", limit: 0 },
		);
		expect(out).toContain("non-empty query");
		expect(out).not.toContain("matches");
	});

	it("empty results read as no matches instead of a zero-match success (false all-clear)", () => {
		const out = collapsed(
			"search_code",
			{
				content: [{ type: "text", text: "No matches found for query 'zzz'." }],
				details: { query: "zzz", totalMatches: 0 },
			},
			{ query: "zzz" },
		);
		expect(out).toContain("no matches");
	});

	it("expanded reveals documentation hits beyond the collapsed window (hidden hits)", () => {
		const out = expanded("search_code", result, args);
		expect(out).toContain("hooks.md");
	});
});
