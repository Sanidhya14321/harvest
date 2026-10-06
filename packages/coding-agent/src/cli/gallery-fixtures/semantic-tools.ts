/** Gallery fixtures for the semantic fallback tools (checkpoint, rewind, security, memory, skill, code search). */
import type { GalleryFixture } from "./types";

export const semanticToolsFixtures: Record<string, GalleryFixture> = {
	checkpoint: {
		label: "Checkpoint",
		streamingArgs: { goal: "Explore auth" },
		args: { goal: "Explore auth flow before refactoring login" },
		result: {
			content: [
				{
					type: "text",
					text: "Checkpoint: Explore auth flow before refactoring login\nFinish exploration and formulate findings.",
				},
			],
			details: { goal: "Explore auth flow before refactoring login", startedAt: "2026-10-04T00:00:00.000Z" },
		},
		errorResult: {
			content: [{ type: "text", text: "Checkpoint already active." }],
			isError: true,
		},
	},

	rewind: {
		label: "Rewind",
		streamingArgs: { report: "Auth uses JWT" },
		args: { report: "Auth uses JWT sessions; login refactors around the session middleware." },
		result: {
			content: [{ type: "text", text: "Rewind requested.\nReport captured for context replacement." }],
			details: { report: "Auth uses JWT sessions; login refactors around the session middleware.", rewound: true },
		},
		errorResult: {
			content: [{ type: "text", text: "No active checkpoint. Create a checkpoint before calling rewind." }],
			isError: true,
		},
	},

	security_scan: {
		label: "Security Scan",
		streamingArgs: { action: "status" },
		args: { action: "status", operation_id: "op-123" },
		result: {
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
		},
		errorResult: {
			content: [{ type: "text", text: "Unknown security operation: op-999" }],
			isError: true,
		},
	},

	memory_edit: {
		label: "Memory Edit",
		streamingArgs: { op: "update", id: "mem-1" },
		args: { op: "update", id: "mem-1", content: "User prefers Bun over Node." },
		result: {
			content: [{ type: "text", text: "Memory mem-1 updated in bank session (working)." }],
			details: { status: "updated", bank: "session", store: "working" },
		},
		errorResult: {
			content: [{ type: "text", text: "Memory mem-9 was not found." }],
			isError: true,
		},
	},

	learn: {
		label: "Learn",
		streamingArgs: { memory: "Prefer Bun over" },
		args: { memory: "Prefer Bun over Node for all new scripts in this repo." },
		result: {
			content: [{ type: "text", text: "Lesson stored." }],
			details: { skill: null },
		},
		errorResult: {
			content: [{ type: "text", text: "Lesson was empty after sanitization; nothing stored." }],
			isError: true,
		},
	},

	manage_skill: {
		label: "Manage Skill",
		streamingArgs: { action: "create", name: "review-checklist" },
		args: {
			action: "create",
			name: "review-checklist",
			description: "Run before sending a PR for review",
			body: "# Review checklist\n\n- Tests pass\n",
		},
		result: {
			content: [
				{
					type: "text",
					text: 'Created managed skill "review-checklist" (managed-skills/review-checklist/SKILL.md).',
				},
			],
			details: { action: "create", name: "review-checklist" },
		},
		errorResult: {
			content: [
				{
					type: "text",
					text: 'Cannot create managed skill "review-checklist": an authored skill of that name already exists, and managed skills cannot override authored ones. Choose a different name.',
				},
			],
			isError: true,
			details: { action: "create", name: "review-checklist", shadowed: true },
		},
	},

	search_code: {
		label: "Search Code",
		streamingArgs: { query: "useState" },
		args: { query: "useState", mode: "all", limit: 20 },
		result: {
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
		},
		errorResult: {
			content: [{ type: "text", text: "Search requires a non-empty query and an integer limit between 1 and 100." }],
			isError: true,
		},
	},
};
