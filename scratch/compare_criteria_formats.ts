import { getLayaClient, type LayaQuestionDefinition } from "../packages/coding-agent/src/core/harvest/laya-client";

interface TestCase {
	readonly id: string;
	readonly category: string;
	readonly description: string;
	readonly task: string;
	readonly expectedAgent: string;
}

const TEST_CASES: TestCase[] = [
	{
		id: "TC-01",
		category: "Code Exploration",
		description: "Find where custom tool registrations are stored",
		task: "Locate and map all files where tool definitions and registrations are declared in the codebase without editing any files.",
		expectedAgent: "scout",
	},
	{
		id: "TC-02",
		category: "Code Exploration",
		description: "Trace model selection hierarchy",
		task: "Trace how model overrides and model roles are resolved across settings and session configurations.",
		expectedAgent: "scout",
	},
	{
		id: "TC-03",
		category: "Code Review",
		description: "Review uncommitted git diff",
		task: "Review the git diff of pending changes for logic errors, off-by-one boundary bugs, and regressions before committing.",
		expectedAgent: "reviewer",
	},
	{
		id: "TC-04",
		category: "Code Review",
		description: "Analyze pull request refactoring",
		task: "Perform a thorough code review on the newly added authentication middleware, checking adherence to project conventions and error handling.",
		expectedAgent: "reviewer",
	},
	{
		id: "TC-05",
		category: "Security Audit",
		description: "Audit HTTP endpoints for path traversal",
		task: "Audit all URL and file loading handlers in the codebase for potential path traversal (CWE-22) vulnerabilities and return evidence.",
		expectedAgent: "security-reviewer",
	},
	{
		id: "TC-06",
		category: "Security Audit",
		description: "Check for hardcoded secrets and token leakage",
		task: "Scan all source files and test fixtures for hardcoded API keys, exposed JWT secrets, and credentials.",
		expectedAgent: "security-reviewer",
	},
	{
		id: "TC-07",
		category: "Mechanical Work",
		description: "Convert CRLF line endings to LF",
		task: "Mechanical cleanup: normalize all CRLF line endings to LF and remove trailing whitespace across all .ts and .md files.",
		expectedAgent: "sonic",
	},
	{
		id: "TC-08",
		category: "Mechanical Work",
		description: "Batch rename imports",
		task: "Perform bulk mechanical find-and-replace of deprecated import @harvest/old-utils with @harvest/pi-utils across the repository.",
		expectedAgent: "sonic",
	},
	{
		id: "TC-09",
		category: "Implementation",
		description: "Build rate-limiting middleware",
		task: "Implement a sliding-window in-memory rate limiter with burst tolerance and redis adapter support, writing tests and types.",
		expectedAgent: "task",
	},
	{
		id: "TC-10",
		category: "Implementation",
		description: "Refactor session state machine",
		task: "Refactor the session lifecycle state machine to support pause/resume transitions and persistent checkpointing.",
		expectedAgent: "task",
	},
];

const FORMAT_A = {
	name: "Format A: Current Baseline (1-line phrases)",
	criteria: {
		scout: "Exploratory codebase research, rapid code analysis, broad pattern searches, symbol location, finding where things are defined without modifying files",
		reviewer: "Code review specialist for analyzing quality, logic correctness, regressions, edge cases, and reviewing PR diffs",
		"security-reviewer": "Read-only security specialist for evidence-backed repository vulnerability discovery, CWE analysis, and security audits",
		task: "General-purpose multi-step implementation, complex coding, refactoring, and feature additions requiring full tool capabilities",
		sonic: "Low-reasoning agent for strictly mechanical updates, bulk formatting, or simple data collection",
	},
};

const FORMAT_B = {
	name: "Format B: Detailed Capabilities & Scope Boundaries",
	criteria: {
		scout: "Read-only codebase exploration and architecture discovery. Use for: finding symbol definitions, tracing control flow, searching file patterns, and answering questions about structure without modifying any code.",
		reviewer: "Code review and quality analysis. Use for: inspecting git diffs, auditing pull requests, identifying logic errors, checking boundary conditions, and verifying test coverage before merging code.",
		"security-reviewer": "Specialized security vulnerability auditor. Use for: detecting CWE vulnerabilities, finding path traversals, locating leaked credentials/secrets, and performing security audits.",
		task: "Full-capability software implementation. Use for: writing new features, modifying multi-file systems, writing tests, fixing complex bugs, and refactoring business logic.",
		sonic: "Fast mechanical execution. Use for: bulk text formatting, whitespace normalization, CRLF/LF conversions, trivial search-and-replace, and routine lint fixes requiring minimal reasoning.",
	},
};

const FORMAT_C = {
	name: "Format C: Workflow-Oriented with Concrete Examples",
	criteria: {
		scout: "Codebase navigation and discovery. Select when the user wants to understand or find code without modifying it. Example tasks: 'Where is the auth handler defined?', 'Find all references to AgentMessage', 'Trace the request lifecycle'.",
		reviewer: "Code review and pull request inspection. Select when the user wants qualitative feedback or bug inspection on changes. Example tasks: 'Review this diff for bugs', 'Check this PR for edge cases', 'Verify correctness of this commit'.",
		"security-reviewer": "Security and vulnerability assessment. Select when the task is specifically focused on security threats or sensitive data leaks. Example tasks: 'Check for SQL injection or path traversal', 'Scan for hardcoded API keys', 'Audit permissions'.",
		task: "Implementation and active software engineering. Select when the agent must write code, create files, implement algorithms, or execute end-to-end tasks. Example tasks: 'Build a rate limiter', 'Fix this bug and write tests', 'Implement feature X'.",
		sonic: "Mechanical and repetitive tasks. Select for non-creative, simple bulk edits that don't need deep reasoning. Example tasks: 'Replace tab indents with spaces across all files', 'Convert CRLF to LF', 'Rename imports'.",
	},
};

const FORMAT_D = {
	name: "Format D: Contrastive Discrimination with Explicit DO NOT Rules",
	criteria: {
		scout: "Read-only exploration and symbol search. Do NOT use if code needs to be edited or reviewed. Best for: finding files, inspecting declarations, codebase mapping.",
		reviewer: "Diff and patch review for correctness and quality. Do NOT use for general implementation or initial code search. Best for: code review, checking PRs, spotting regressions.",
		"security-reviewer": "Security vulnerabilities and credential scanning. Do NOT use for regular code quality reviews. Best for: CWE audits, penetration analysis, secret scanning.",
		task: "Multi-step development and active implementation. Do NOT use for read-only exploration or simple mechanical sweeps when specialist agents exist. Best for: building features, fixing bugs, refactoring.",
		sonic: "Trivial repetitive text transformations. Do NOT use for tasks requiring logic reasoning or algorithmic design. Best for: bulk formatting, find-and-replace, CRLF normalization.",
	},
};

async function evaluateFormat(client: any, formatDef: { name: string; criteria: Record<string, string> }) {
	console.log(`\n================================================================================`);
	console.log(`EVALUATING: ${formatDef.name}`);
	console.log(`================================================================================`);

	let correctCount = 0;
	const confidences: number[] = [];
	const topProbs: number[] = [];
	const margins: number[] = [];

	for (const tc of TEST_CASES) {
		const qDef: LayaQuestionDefinition = {
			type: "choice",
			instructions: "Select the most suitable specialized subagent for this task from the available roster:",
			criteria: formatDef.criteria,
		};

		const state = `Task Assignment:\n${tc.task}`;
		const res = await client.decide(state, { subagent_choice: qDef }, { callSite: "format_eval", timeoutMs: 15000 });
		if (!res.success || !res.data) {
			throw new Error(`Decision failed: ${res.fallbackReason}`);
		}
		const ans = res.data["subagent_choice"];

		const pick = ans.choice;
		const conf = ans.confidence;
		const probs: Record<string, number> = ans.probabilities || {};
		const sortedProbs = Object.values(probs).sort((a, b) => b - a);
		const p1 = sortedProbs[0] || 0;
		const p2 = sortedProbs[1] || 0;
		const margin = p1 - p2;
		const isCorrect = pick === tc.expectedAgent;

		if (isCorrect) correctCount++;
		confidences.push(conf);
		topProbs.push(p1);
		margins.push(margin);

		const mark = isCorrect ? "✓ PASS" : "✗ FAIL";
		console.log(
			`[${tc.id}] ${mark} Pick: ${pick.padEnd(17)} (Exp: ${tc.expectedAgent.padEnd(17)}) | Conf: ${conf.toFixed(4)} | P1: ${(p1 * 100).toFixed(1)}% | P2: ${(p2 * 100).toFixed(1)}% | Margin: ${(margin * 100).toFixed(1)}%`
		);
	}

	const avgConf = confidences.reduce((a, b) => a + b, 0) / confidences.length;
	const maxConf = Math.max(...confidences);
	const minConf = Math.min(...confidences);
	const avgP1 = topProbs.reduce((a, b) => a + b, 0) / topProbs.length;
	const avgMargin = margins.reduce((a, b) => a + b, 0) / margins.length;

	console.log(`\n--- SUMMARY FOR: ${formatDef.name} ---`);
	console.log(`  Accuracy:         ${correctCount}/${TEST_CASES.length} (${((correctCount / TEST_CASES.length) * 100).toFixed(1)}%)`);
	console.log(`  Avg Confidence:   ${avgConf.toFixed(4)} (range: ${minConf.toFixed(4)} - ${maxConf.toFixed(4)})`);
	console.log(`  Avg Top-1 Prob:   ${(avgP1 * 100).toFixed(1)}%`);
	console.log(`  Avg Margin (P1-P2): ${(avgMargin * 100).toFixed(1)}%`);

	return {
		name: formatDef.name,
		accuracy: correctCount / TEST_CASES.length,
		avgConf,
		minConf,
		maxConf,
		avgP1,
		avgMargin,
	};
}

async function main() {
	const client = getLayaClient();
	const formats = [FORMAT_A, FORMAT_B, FORMAT_C, FORMAT_D];
	const results = [];

	for (const f of formats) {
		const res = await evaluateFormat(client, f);
		results.push(res);
	}

	console.log("\n\n================================================================================");
	console.log("FINAL COMPARATIVE TABLE ACROSS ALL FORMATS");
	console.log("================================================================================");
	console.log(
		"Format Name".padEnd(45) +
		"Accuracy".padEnd(12) +
		"Avg Conf".padEnd(12) +
		"Max Conf".padEnd(12) +
		"Avg P1".padEnd(12) +
		"Avg Margin"
	);
	console.log("-".repeat(105));

	for (const r of results) {
		console.log(
			r.name.slice(0, 43).padEnd(45) +
			`${(r.accuracy * 100).toFixed(0)}%`.padEnd(12) +
			r.avgConf.toFixed(4).padEnd(12) +
			r.maxConf.toFixed(4).padEnd(12) +
			`${(r.avgP1 * 100).toFixed(1)}%`.padEnd(12) +
			`${(r.avgMargin * 100).toFixed(1)}%`
		);
	}
}

main().catch(console.error);
