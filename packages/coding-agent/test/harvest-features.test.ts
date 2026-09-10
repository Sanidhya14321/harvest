import { describe, expect, it } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { Agent } from "@harvest/pi-agent-core";
import { ModelRegistry } from "../src/config/model-registry";
import { Settings } from "../src/config/settings";
import { AgentSession } from "../src/session/agent-session";
import { GROUNDING_NUDGE_MESSAGE_TYPE } from "../src/session/messages";
import { SessionManager } from "../src/session/session-manager";
import { createInMemoryAuthStorage } from "./helpers/agent-session-setup";

// 3.1 Roles & Routing
import {
	calculateEntropy,
	classifyIntent,
	getRoleDefinition,
	routeRole,
	ROUTING_CONFIDENCE_THRESHOLD,
} from "../src/core/harvest/roles";

// 3.2 Coding Contract
import {
	formatCodingContract,
	injectCodingContract,
	shouldInjectCodingContract,
} from "../src/core/harvest/coding-contract";

// 3.3 Pre-Read Enforcement
import { PreReadEnforcement } from "../src/core/harvest/loop-policy";

// 3.4 Freshness & File Session
import { atomicWriteFile, atomicWriteFileSync, FileSession } from "../src/core/tools/file-session";
import { FreshnessTracker } from "../src/core/tools/freshness";

// 3.5 Matcher & Edit
import { EditMutator } from "../src/core/tools/edit";
import { findEditMatch } from "../src/core/tools/matcher";

// 3.6 Grounding & Verification
import { ExecutionGroundingEngine } from "../src/core/harvest/grounding";
import { detectVerificationRunner, formatRepairPrompt } from "../src/core/harvest/verification";

// 3.7 Retrieval & Code Index
import { extractSymbolsFromContent } from "../src/core/harvest/code-index";
import { splitMarkdownSections } from "../src/core/harvest/retrieval";

// 3.8 & 3.9 Memory, Writeback & Graph
import { HarvestKnowledgeGraph } from "../src/core/harvest/graph";
import { ContradictionEngine } from "../src/core/harvest/memory";
import { HarvestWriteback } from "../src/core/harvest/writeback";

// 3.10 Skill Automation
import { HarvestSkillAutomation, normalizeToolSignature } from "../src/core/harvest/skill-automation";
import { SkillContextManager } from "../src/core/harvest/skill-context";

// 3.11 Subagent & Git Claim
import { HarvestSubagentRunner, hasGitCommitClaim } from "../src/core/harvest/subagent";

// 3.12 Model Tier Budgeting
import { detectContextTier, getTierBudget } from "../src/core/harvest/model-tier";
import { assembleHarvestSystemPrompt } from "../src/core/system-prompt";

// 3.13 Compaction
import { DeterministicCompactor, extractTranscriptEvidence } from "../src/core/compaction/compaction";

// 3.14 Security Sandbox
import { SecuritySandbox } from "../src/core/harvest/security";

// 3.15 JSON Repair
import { parseStreamingJson, repairJson } from "../../ai/src/utils/json-parse";

function createTempDir(prefix: string): string {
	const tmpBase = os.tmpdir();
	const dir = path.join(tmpBase, `harvest-test-${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`);
	fs.mkdirSync(dir, { recursive: true });
	return dir;
}

describe("Feature 3.1: Entropy-Gated Intent Routing & Specialist Roles", () => {
	it("defines all 6 specialist roles with capabilities, commands, and directives", () => {
		const roles = ["backend", "frontend", "qa", "coordinator", "bootstrap", "compaction"] as const;
		for (const role of roles) {
			const def = getRoleDefinition(role);
			expect(def.id).toBe(role);
			expect(def.capabilities.length).toBeGreaterThan(0);
			expect(def.directives.length).toBeGreaterThan(0);
			expect(def.toolPolicy.length).toBeGreaterThan(0);
			expect(def.promptAddendum.length).toBeGreaterThan(0);
		}
	});

	it("classifies intent into informational, mutation, and complex", () => {
		expect(classifyIntent("explain what this service does")).toBe("informational");
		expect(classifyIntent("add authentication route to express server")).toBe("mutation");
		expect(classifyIntent("migrate database and complete redesign from scratch")).toBe("complex");
	});

	it("routes clear single-domain prompts with high confidence", () => {
		const res = routeRole("build a button and modal styling with responsive tailwind CSS in react component");
		expect(res.role).toBe("frontend");
		expect(res.confidence).toBeGreaterThanOrEqual(ROUTING_CONFIDENCE_THRESHOLD);
		expect(res.isAmbiguous).toBe(false);
		expect(res.clarificationQuestion).toBeUndefined();
	});

	it("triggers the ambiguity gate and generates clarification question when multi-domain keywords clash", () => {
		const res = routeRole("fix database query endpoint and build react component with tailwind style");
		expect(res.isAmbiguous).toBe(true);
		expect(res.confidence).toBeLessThan(ROUTING_CONFIDENCE_THRESHOLD);
		expect(res.clarificationQuestion).toBeDefined();
		expect(res.clarificationQuestion).toContain("Which specialist role should lead this task?");
	});

	it("computes Shannon entropy correctly for equal vs skewed distributions", () => {
		// Single role matched: entropy 0, confidence 1
		const single = calculateEntropy({ backend: 5, frontend: 0, qa: 0, coordinator: 0, bootstrap: 0, compaction: 0 });
		expect(single.entropy).toBe(0);
		expect(single.confidence).toBe(1.0);

		// Equal distribution across 2 roles: normalized entropy 1.0, confidence 0.0
		const equal = calculateEntropy({ backend: 3, frontend: 3, qa: 0, coordinator: 0, bootstrap: 0, compaction: 0 });
		expect(equal.confidence).toBe(0.0);
	});

	it("correctly routes dominant multi-keyword task without false ambiguity on minor secondary keywords", () => {
		const res = routeRole("Implement a backend REST API with Postgres database service and test it");
		expect(res.role).toBe("backend");
		expect(res.isAmbiguous).toBe(false);
		expect(res.confidence).toBeGreaterThanOrEqual(ROUTING_CONFIDENCE_THRESHOLD);
		expect(res.clarificationQuestion).toBeUndefined();
	});
});

describe("Feature 3.2: The Grounded Coding Task Contract", () => {
	it("formats standard XML coding contract with all 5 checkpoints", () => {
		const contract = formatCodingContract();
		expect(contract).toContain("<coding-contract>");
		expect(contract).toContain("0. Decompose the request into small checkpoints");
		expect(contract).toContain("1. Inspect the target files");
		expect(contract).toContain("2. Implement the smallest complete change");
		expect(contract).toContain("3. Run the narrowest relevant test");
		expect(contract).toContain("4. Finish only when every requested behavior is implemented");
		expect(contract).toContain("Prefer evidence from tools over assumptions.");
		expect(contract).toContain("</coding-contract>");
	});

	it("injects contract for mutation and complex intents, but skips informational", () => {
		expect(shouldInjectCodingContract("mutation")).toBe(true);
		expect(shouldInjectCodingContract("complex")).toBe(true);
		expect(shouldInjectCodingContract("informational")).toBe(false);

		const base = "Base system prompt.";
		const injected = injectCodingContract(base, "mutation");
		expect(injected).toContain("<coding-contract>");

		const skipped = injectCodingContract(base, "informational");
		expect(skipped).toBe(base);
	});
});

describe("Feature 3.3: Pre-Read Enforcement & Blind Overwrite Shield", () => {
	it("blocks edit and write on existing files that have not been read in session", () => {
		const testDir = createTempDir("preread");
		const existingFile = path.join(testDir, "config.ts");
		fs.writeFileSync(existingFile, "export const PORT = 3000;\n");

		const policy = new PreReadEnforcement(testDir);

		// Edit without read -> blocked synchronously
		const editCheck = policy.checkMutationAllowed("edit", "config.ts");
		expect(editCheck.allowed).toBe(false);
		expect(editCheck.reason).toContain(
			"Blocked edit on 'config.ts': this file exists but has not been read in this session",
		);

		// Write on existing file without read -> blocked synchronously
		const writeCheck = policy.checkMutationAllowed("write", "config.ts");
		expect(writeCheck.allowed).toBe(false);
		expect(writeCheck.reason).toContain(
			"Blocked write on 'config.ts': this file exists but has not been read in this session",
		);

		// Write on new non-existent file -> allowed
		const newWriteCheck = policy.checkMutationAllowed("write", "new-file.ts");
		expect(newWriteCheck.allowed).toBe(true);

		// After reading, mutations are permitted
		policy.recordRead("config.ts");
		expect(policy.checkMutationAllowed("edit", "config.ts").allowed).toBe(true);
		expect(policy.checkMutationAllowed("write", "config.ts").allowed).toBe(true);

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("intercepts tool call payloads correctly", () => {
		const testDir = createTempDir("preread-tool");
		const existingFile = path.join(testDir, "app.ts");
		fs.writeFileSync(existingFile, "console.log('app');");

		const policy = new PreReadEnforcement(testDir);

		const intercepted = policy.interceptToolCall({ name: "edit", args: { path: "app.ts" } });
		expect(intercepted.allowed).toBe(false);

		policy.interceptToolCall({ name: "read", args: { path: "app.ts" } });
		const allowedNow = policy.interceptToolCall({ name: "edit", args: { path: "app.ts" } });
		expect(allowedNow.allowed).toBe(true);

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("intercepts mutation tools with diverse argument keys (TargetFile, AbsolutePath)", () => {
		const testDir = createTempDir("pre-read-keys");
		const targetFile = path.join(testDir, "existing.ts");
		fs.writeFileSync(targetFile, "export const x = 1;");

		const policy = new PreReadEnforcement(testDir);
		const blocked = policy.interceptToolCall({
			name: "replace_file_content",
			args: { TargetFile: targetFile },
		});
		expect(blocked.allowed).toBe(false);
		expect(blocked.error).toContain("Blocked edit");

		// Record view and retry
		policy.interceptToolCall({
			name: "view_file",
			args: { AbsolutePath: targetFile },
		});

		const allowed = policy.interceptToolCall({
			name: "replace_file_content",
			args: { TargetFile: targetFile },
		});
		expect(allowed.allowed).toBe(true);

		fs.rmSync(testDir, { recursive: true, force: true });
	});
});

describe("Feature 3.4: File Freshness Tracking & Atomic Checkpoint Rollback Stack", () => {
	it("records mtimeMs, size, and sha256, and detects stale external modifications", () => {
		const testDir = createTempDir("freshness");
		const targetFile = path.join(testDir, "service.ts");
		fs.writeFileSync(targetFile, "const version = 1;");

		const tracker = new FreshnessTracker(testDir);
		const rec = tracker.recordRead(targetFile);
		expect(rec).toBeDefined();
		expect(rec!.size).toBe(18);

		// Assert passes when content is unchanged
		tracker.assertCurrent(targetFile);

		// Modify file externally with different content
		fs.writeFileSync(targetFile, "const version = 999;");

		let threw = false;
		try {
			tracker.assertCurrent(targetFile);
		} catch (err: unknown) {
			threw = true;
			expect((err as Error).message).toContain("was modified since it was read");
		}
		expect(threw).toBe(true);

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("FileSession restores turn file snapshots atomically on undoTurn", async () => {
		const testDir = createTempDir("filesession");
		const fileA = path.join(testDir, "fileA.txt");
		const fileB = path.join(testDir, "fileB.txt");
		fs.writeFileSync(fileA, "original A");

		const session = new FileSession(testDir);

		// Simulate turn 1 modifying fileA and creating fileB
		session.recordMutation("turn-1", fileA, "original A", "modified A");
		session.recordMutation("turn-1", fileB, null, "new B content");
		fs.writeFileSync(fileA, "modified A");
		fs.writeFileSync(fileB, "new B content");

		const restoreResults = await session.undoTurn("turn-1");
		expect(restoreResults.length).toBe(2);
		expect(fs.readFileSync(fileA, "utf8")).toBe("original A");
		expect(fs.existsSync(fileB)).toBe(false); // created file deleted on rollback

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("atomicWriteFile writes cleanly and replaces target", async () => {
		const testDir = createTempDir("atomicwrite");
		const target = path.join(testDir, "atomic.txt");

		await atomicWriteFile(target, "atomic content 1");
		expect(fs.readFileSync(target, "utf8")).toBe("atomic content 1");

		await atomicWriteFile(target, "atomic content 2");
		expect(fs.readFileSync(target, "utf8")).toBe("atomic content 2");

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("atomicWriteFile safely handles concurrent writes and cleans up temporary files", async () => {
		const testDir = createTempDir("atomic-concurrent");
		const target = path.join(testDir, "shared.txt");

		// Run 10 concurrent writes to the same file
		const writes = Array.from({ length: 10 }, (_, i) => atomicWriteFile(target, `content version ${i}`));
		await Promise.all(writes);

		// Target file exists and has one of the valid written contents
		const finalContent = fs.readFileSync(target, "utf8");
		expect(finalContent).toMatch(/^content version \d+$/);

		// Verify no leftover .tmp files remain in the target directory
		const remainingFiles = fs.readdirSync(testDir);
		const tmpFiles = remainingFiles.filter(f => f.endsWith(".tmp"));
		expect(tmpFiles.length).toBe(0);

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("atomicWriteFileSync writes cleanly and safely", () => {
		const testDir = createTempDir("atomic-sync");
		const target = path.join(testDir, "sync.txt");

		atomicWriteFileSync(target, "sync content");
		expect(fs.readFileSync(target, "utf8")).toBe("sync content");

		fs.rmSync(testDir, { recursive: true, force: true });
	});
});

describe("Feature 3.5: Multi-Disjoint 3-Tier Edit Matcher & Atomic Mutator", () => {
	const sample = [
		"function calculateTotal(items) {",
		"    let total = 0;",
		"    for (const item of items) {",
		"        total += item.price;",
		"    }",
		"    return total;",
		"}",
		"function formatPrice(val) {",
		"    return '$' + val;",
		"}",
	].join("\n");

	it("Tier 1: exact LF match", () => {
		const res = findEditMatch(sample, "    let total = 0;\n    for (const item of items) {");
		expect(res.matched).toBe(true);
		if (res.matched) {
			expect(res.tier).toBe(1);
			expect(res.confidence).toBe(1.0);
		}
	});

	it("Tier 2: whitespace-flexible match", () => {
		const res = findEditMatch(sample, "  let   total   =   0;\n  for (const item of items) {");
		expect(res.matched).toBe(true);
		if (res.matched) {
			expect(res.tier).toBe(2);
			expect(res.confidence).toBe(0.95);
		}
	});

	it("Tier 3: fuzzy line-window Levenshtein match with accept & margin gates", () => {
		// Minor typo inside line
		const search = "    let totalll = 0;\n    for (const item of items) {";
		const res = findEditMatch(sample, search);
		expect(res.matched).toBe(true);
		if (res.matched) {
			expect(res.tier).toBe(3);
			expect(res.confidence).toBeGreaterThanOrEqual(0.88);
		}
	});

	it("returns actionable near-miss diagnostics when match fails", () => {
		const res = findEditMatch(sample, "nonExistentLineThatHasNoMatchHereAtAll()");
		expect(res.matched).toBe(false);
		if (!res.matched) {
			expect(res.diagnostics.closestLineNumber).toBeGreaterThan(0);
			expect(res.diagnostics.reason.length).toBeGreaterThan(0);
		}
	});

	it("EditMutator executes multi-disjoint edits in reverse offset order", async () => {
		const testDir = createTempDir("editmutator");
		const target = path.join(testDir, "code.ts");
		fs.writeFileSync(target, "const A = 'initialA';\nconst B = 'keepB';\nconst C = 'initialC';");

		const mutator = new EditMutator({ workspaceRoot: testDir });
		const result = await mutator.execute({
			path: "code.ts",
			edits: [
				{ oldText: "const A = 'initialA';", newText: "const A = 'updatedA';" },
				{ oldText: "const C = 'initialC';", newText: "const C = 'updatedC';" },
			],
		});

		expect(result.success).toBe(true);
		expect(result.appliedCount).toBe(2);
		expect(result.diff).toContain("+const A = 'updatedA';");
		expect(result.diff).toContain("+const C = 'updatedC';");

		const content = fs.readFileSync(target, "utf8");
		expect(content).toBe("const A = 'updatedA';\nconst B = 'keepB';\nconst C = 'updatedC';");

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("EditMutator rejects overlapping edits", async () => {
		const testDir = createTempDir("editoverlap");
		const target = path.join(testDir, "overlap.txt");
		fs.writeFileSync(target, "AAAA BBBB CCCC");

		const mutator = new EditMutator({ workspaceRoot: testDir });
		const result = await mutator.execute({
			path: "overlap.txt",
			edits: [
				{ oldText: "AAAA BBBB", newText: "1111" },
				{ oldText: "BBBB CCCC", newText: "2222" },
			],
		});

		expect(result.success).toBe(false);
		expect(result.error).toContain("Overlapping edits detected");

		fs.rmSync(testDir, { recursive: true, force: true });
	});
});

describe("Feature 3.6: Local Execution Grounding & Verify-Before-Done Nudging", () => {
	it("detects verification runners from workspace files", () => {
		const testDir = createTempDir("runner-detect");
		fs.writeFileSync(path.join(testDir, "package.json"), JSON.stringify({ scripts: { test: "vitest run" } }));

		const runner = detectVerificationRunner(testDir);
		expect(runner).toBeDefined();
		expect(runner!.runner).toBe("npm");
		expect(runner!.command).toBe("npm test");

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("ExecutionGroundingEngine nudges unverified mutations and caps at 2 nudges", () => {
		const engine = new ExecutionGroundingEngine(".");
		engine.recordFileMutation("src/index.ts");

		// Nudge 1
		const nudge1 = engine.checkVerifyBeforeDoneNudge({ stopReason: "stop", role: "backend" });
		expect(nudge1.shouldNudge).toBe(true);
		expect(nudge1.steerPrompt).toContain(
			"You modified ['src/index.ts'] but have not verified the changes successfully",
		);

		// Nudge 2
		const nudge2 = engine.checkVerifyBeforeDoneNudge({ stopReason: "stop", role: "backend" });
		expect(nudge2.shouldNudge).toBe(true);

		// Nudge 3 -> capped!
		const nudge3 = engine.checkVerifyBeforeDoneNudge({ stopReason: "stop", role: "backend" });
		expect(nudge3.shouldNudge).toBe(false);
	});

	it("exempts frontend role and standalone deliverables from verification nudging", () => {
		const engine = new ExecutionGroundingEngine(".");
		engine.recordFileMutation("src/button.tsx");

		const nudgeFrontend = engine.checkVerifyBeforeDoneNudge({ stopReason: "stop", role: "frontend" });
		expect(nudgeFrontend.shouldNudge).toBe(false);

		const nudgeDeliverable = engine.checkVerifyBeforeDoneNudge({
			stopReason: "stop",
			role: "backend",
			isStandaloneDeliverable: true,
		});
		expect(nudgeDeliverable.shouldNudge).toBe(false);
	});

	it("clears unverified state once a test passes after the mutation", () => {
		const engine = new ExecutionGroundingEngine(".");
		engine.recordFileMutation("src/handler.ts");
		expect(engine.isGroundingValidated()).toBe(false);

		engine.recordCommandExecution("npm test", 0, "PASS 10 tests", "");
		expect(engine.isGroundingValidated()).toBe(true);
		expect(engine.checkVerifyBeforeDoneNudge({ stopReason: "stop", role: "backend" }).shouldNudge).toBe(false);
	});

	it("does not clear unverified state when exitCode is 0 but diagnostic failures are detected", () => {
		const engine = new ExecutionGroundingEngine(".");
		engine.recordFileMutation("src/handler.ts");
		expect(engine.isGroundingValidated()).toBe(false);

		// Command exited with 0, but stdout contains test failure assertions
		engine.recordCommandExecution(
			"npm test",
			0,
			"FAIL test/handler.test.ts\nAssertionError: expected 200 to equal 500",
			"",
		);
		expect(engine.isGroundingValidated()).toBe(false);
		expect(engine.getUnverifiedMutations()).toContain("src/handler.ts");

		const nudge = engine.checkVerifyBeforeDoneNudge({ stopReason: "stop", role: "backend" });
		expect(nudge.shouldNudge).toBe(true);
	});

	it("parses test failures and formats structured repair prompt", () => {
		const stdout = "FAIL test/auth.test.ts\nAssertionError: expected 401 but got 200\n    at auth.ts:42";
		const prompt = formatRepairPrompt({
			command: "npm test",
			exitCode: 1,
			stdout,
			stderr: "",
		});

		expect(prompt).toContain("[EXECUTION GROUNDING FAILURE]");
		expect(prompt).toContain("Command: `npm test` (Exit code: 1)");
		expect(prompt).toContain("AssertionError: expected 401 but got 200");
	});

	it("AgentSession onTurnEnd injects steering grounding nudge without crashing on this.yieldQueue.push", async () => {
		const testDir = createTempDir("grounding-session");
		const authStorage = createInMemoryAuthStorage();
		authStorage.setRuntimeApiKey("anthropic", "test-key");
		const modelRegistry = new ModelRegistry(authStorage);
		const agent = new Agent();

		let turnEndHook: ((messages: any, signal?: any, context?: any) => Promise<void>) | undefined;
		const origSetOnTurnEnd = agent.setOnTurnEnd.bind(agent);
		agent.setOnTurnEnd = (fn: any) => {
			turnEndHook = fn;
			origSetOnTurnEnd(fn);
		};

		const session = new AgentSession({
			agent,
			sessionManager: SessionManager.inMemory(testDir),
			settings: Settings.isolated(),
			modelRegistry,
		});

		expect(turnEndHook).toBeDefined();

		// Record a file mutation in groundingEngine
		session.harvestHooks.groundingEngine.recordFileMutation("src/index.ts");

		// Call turnEndHook with willContinue = false (simulating turn end without verification)
		await turnEndHook!([], undefined, {
			willContinue: false,
			message: { role: "assistant", content: [{ type: "text", text: "done" }], stopReason: "stop" } as any,
			toolResults: [],
		});

		// Check that a steering message was queued with the grounding nudge
		const steers = agent.peekSteeringQueue();
		expect(steers.length).toBe(1);
		expect(steers[0].role).toBe("custom");
		expect((steers[0] as any).customType).toBe(GROUNDING_NUDGE_MESSAGE_TYPE);
		expect((steers[0] as any).content).toContain("src/index.ts");
		expect((steers[0] as any).content).toContain("Before finishing, run the project's verification command");

		await session.dispose();
		authStorage.close();
		fs.rmSync(testDir, { recursive: true, force: true });
	});
});

describe("Feature 3.7: Vectorless Dual-Stage Section Retrieval & BM25 Code Index", () => {
	it("splits markdown into sections and preserves internal '#' inside code fences", () => {
		const md = [
			"# Document Title",
			"",
			"## Section One",
			"Here is description one.",
			"```python",
			"# this is a comment with hash",
			"def run():",
			"    pass",
			"```",
			"",
			"### Section Two",
			"Here is description two.",
		].join("\n");

		const sections = splitMarkdownSections("doc.md", md, "backend");
		expect(sections.length).toBe(3);
		expect(sections[1].heading).toBe("Section One");
		expect(sections[1].content).toContain("# this is a comment with hash");
		expect(sections[2].heading).toBe("Section Two");
	});

	it("extracts column-0 symbols and splits identifiers into subwords", () => {
		const code = [
			"export function parseUserAuthToken(token: string): boolean {",
			"    return true;",
			"}",
			"class UserProfileManager {",
			"}",
			"export interface AuthSessionConfig {",
			"}",
		].join("\n");

		const symbols = extractSymbolsFromContent(code, "auth.ts");
		expect(symbols.length).toBe(3);
		expect(symbols[0].name).toBe("parseUserAuthToken");
		expect(symbols[0].kind).toBe("function");
		expect(symbols[0].subwords).toEqual(["parse", "user", "auth", "token"]);

		expect(symbols[1].name).toBe("UserProfileManager");
		expect(symbols[1].kind).toBe("class");
		expect(symbols[1].subwords).toEqual(["user", "profile", "manager"]);
	});
});

describe("Feature 3.8 & 3.9: Institutional Memory, Writeback, Graph & Contradiction Detection", () => {
	it("writes patterns and anti-patterns with provenance frontmatter", async () => {
		const testDir = createTempDir("writeback");
		const writeback = new HarvestWriteback(testDir);

		const pResult = await writeback.writePattern({
			role: "backend",
			title: "Use connection pooling for Postgres queries",
			taskId: "task-101",
			description: "Always reuse connection pool instances.",
		});

		expect(pResult.action).toBe("created");
		expect(fs.existsSync(pResult.filePath)).toBe(true);
		const content = fs.readFileSync(pResult.filePath, "utf8");
		expect(content).toContain("kind: pattern");
		expect(content).toContain("produced_by: task-101");
		expect(content).toContain("grounding_validated: true");

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("supersedes similar patterns (similarity between 0.50 and 0.85) without destroying history", async () => {
		const testDir = createTempDir("supersede");
		const writeback = new HarvestWriteback(testDir);

		const first = await writeback.writePattern({
			role: "backend",
			title: "Database connection pool configuration",
			taskId: "task-1",
			description: "Configure pool with min 2 max 10.",
		});
		expect(first.action).toBe("created");

		// Second rule with similar title
		const second = await writeback.writePattern({
			role: "backend",
			title: "Database connection pool timeout configuration",
			taskId: "task-2",
			description: "Configure pool with timeout and idle caps.",
		});

		expect(second.action).toBe("superseded");
		const firstContent = fs.readFileSync(first.filePath, "utf8");
		expect(firstContent).toContain("superseded: true");

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("detects contradictions between pattern and anti-pattern sharing >= 2 keywords", async () => {
		const testDir = createTempDir("contradiction");
		const writeback = new HarvestWriteback(testDir);

		await writeback.writePattern({
			role: "backend",
			title: "Always use in-memory caching for session storage",
			taskId: "task-1",
			description: "In-memory caching reduces redis latency.",
		});

		await writeback.writeAntiPattern({
			role: "backend",
			title: "Avoid in-memory caching for session storage",
			taskId: "task-2",
			attemptedAction: "Stored sessions in process RAM.",
			whyItFailed: "Caused memory leaks across clustered processes.",
			correctiveRule: "Use distributed Redis session store.",
		});

		const engine = new ContradictionEngine(testDir);
		const contradictions = engine.detectContradictions();

		expect(contradictions.length).toBeGreaterThan(0);
		expect(contradictions[0].sharedKeywords).toContain("caching");
		expect(contradictions[0].sharedKeywords).toContain("session");
		expect(fs.existsSync(engine.pendingReviewPath)).toBe(true);

		const reviewContent = fs.readFileSync(engine.pendingReviewPath, "utf8");
		expect(reviewContent).toContain("Harvest Knowledge Review Queue");
		expect(reviewContent).toContain("PENDING_REVIEW");

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("knowledge graph preserves directed edges, dangling edges, and BFS traversal", () => {
		const testDir = createTempDir("graph");
		const graph = new HarvestKnowledgeGraph(testDir);

		graph.addNode({ id: "node-A", role: "backend", kind: "pattern", title: "Node A", path: "a.md" });
		graph.addNode({ id: "node-B", role: "backend", kind: "pattern", title: "Node B", path: "b.md" });
		// Dangling target node-C
		graph.addEdge("node-A", "node-B", "depends_on");
		graph.addEdge("node-B", "node-C", "relates_to");

		expect(graph.getOutgoingEdges("node-A").length).toBe(1);
		expect(graph.getOutgoingEdges("node-B").length).toBe(1);

		const reachable = graph.bfs("node-A", 2);
		expect(reachable.some(n => n.id === "node-B")).toBe(true);

		fs.rmSync(testDir, { recursive: true, force: true });
	});
});

describe("Feature 3.10: Automatic Tool-Call Sequence Mining & Skill Automation", () => {
	it("normalizes tool signatures consistently", () => {
		expect(normalizeToolSignature("read", { path: "src/auth/jwt.ts" })).toBe("read:*.ts");
		expect(normalizeToolSignature("write", { path: "src/types.d.ts" })).toBe("write:*.ts");
		expect(normalizeToolSignature("bash", { command: "npm test -- -t auth" })).toBe("bash:npm test");
	});

	it("normalizes tool signatures with replace_file_content, view_file, and TargetFile / AbsolutePath", () => {
		expect(normalizeToolSignature("replace_file_content", { TargetFile: "C:/project/src/index.ts" })).toBe(
			"edit:*.ts",
		);
		expect(normalizeToolSignature("view_file", { AbsolutePath: "/workspace/src/app.py" })).toBe("read:*.py");
		expect(normalizeToolSignature("write_to_file", { TargetFile: "src/main.rs" })).toBe("write:*.rs");
		expect(normalizeToolSignature("ast-edit", { filePath: "src/component.tsx" })).toBe("edit:*.tsx");
		expect(normalizeToolSignature("command", { CommandLine: "cargo test --all" })).toBe("bash:cargo test");
	});

	it("mines repeated sequences and stages proposal requiring human approval", async () => {
		const testDir = createTempDir("skills");
		const automation = new HarvestSkillAutomation(testDir);

		// Execute 3 repetitions of a 3-step workflow
		for (let i = 0; i < 3; i++) {
			automation.recordToolCall("read", { path: "service.ts" });
			automation.recordToolCall("edit", { path: "service.ts" });
			automation.recordToolCall("bash", { command: "npm test" });
		}

		const pending = automation.getPendingProposals();
		expect(pending.length).toBe(1);
		expect(pending[0].occurrences).toBeGreaterThanOrEqual(3);

		const approval = await automation.approveProposal(pending[0].id);
		expect(approval.success).toBe(true);
		expect(fs.existsSync(approval.filePath!)).toBe(true);

		const content = fs.readFileSync(approval.filePath!, "utf8");
		expect(content).toContain("## Required Checklist Sequence");
		expect(content).toContain("- [ ] Step 1: Execute `read:*.ts`");

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("loads approved skills and formats context with query relevance filtering and token budgeting", async () => {
		const testDir = createTempDir("skill-context");
		const skillsDir = path.join(testDir, ".harvest", "skills");
		fs.mkdirSync(skillsDir, { recursive: true });

		fs.writeFileSync(
			path.join(skillsDir, "db-migration.md"),
			`---\nname: db-migration\ndescription: Safely run database migrations with dry-run\n---\n# DB Migration\n`,
			"utf8",
		);
		fs.writeFileSync(
			path.join(skillsDir, "react-component.md"),
			`---\nname: react-component\ndescription: Generate accessible React UI components\n---\n# React Component\n`,
			"utf8",
		);

		const manager = new SkillContextManager(testDir);
		const all = manager.loadSkills();
		expect(all.length).toBe(2);

		// Relevance filtering: query about react should ONLY return react-component, saving tokens
		const relevant = manager.findRelevantSkills("create new react frontend component");
		expect(relevant.length).toBe(1);
		expect(relevant[0].name).toBe("react-component");

		// Irrelevant query returns no skills, preventing token waste
		const none = manager.findRelevantSkills("something completely unrelated like baking cookies");
		expect(none.length).toBe(0);

		// Format prompt with query only includes relevant skill
		const prompt = manager.formatSkillsPrompt({ query: "fix database schema migration" });
		expect(prompt).toContain("<harvest-skills>");
		expect(prompt).toContain("db-migration");
		expect(prompt).not.toContain("react-component");

		// Format prompt with irrelevant query returns empty string
		const emptyPrompt = manager.formatSkillsPrompt({ query: "write a poem" });
		expect(emptyPrompt).toBe("");

		fs.rmSync(testDir, { recursive: true, force: true });
	});

	it("loads directory-based SKILL.md skills and ignores boilerplate stopwords", async () => {
		const testDir = createTempDir("skill-dirs");
		const skillFolder = path.join(testDir, ".harvest", "skills", "semantic-compression");
		fs.mkdirSync(skillFolder, { recursive: true });

		fs.writeFileSync(
			path.join(skillFolder, "SKILL.md"),
			`---\nname: semantic-compression\ndescription: Use when compressing system prompts, tool descriptions, and reducing token bloat.\n---\n# Semantic Compression\n`,
			"utf8",
		);

		const manager = new SkillContextManager(testDir);
		const loaded = manager.loadSkills();
		expect(loaded.length).toBe(1);
		expect(loaded[0].name).toBe("semantic-compression");
		expect(loaded[0].description).toContain("reducing token bloat");

		// "use", "when", "and" are stopwords and should NOT match generic queries
		const stopwordMatches = manager.findRelevantSkills("can you use a helper when needed");
		expect(stopwordMatches.length).toBe(0);

		// Domain keyword match should match accurately
		const domainMatches = manager.findRelevantSkills("compress prompt token bloat");
		expect(domainMatches.length).toBe(1);
		expect(domainMatches[0].name).toBe("semantic-compression");

		fs.rmSync(testDir, { recursive: true, force: true });
	});
});

describe("Feature 3.11: Subagent Delegation & Git Claim Anti-Hallucination", () => {
	it("detects commit assertions in summary text", () => {
		expect(hasGitCommitClaim("I created a commit and pushed to main.")).toBe(true);
		expect(hasGitCommitClaim("I have committed the changes.")).toBe(true);
		expect(hasGitCommitClaim("staged and committed all modified files.")).toBe(true);
		expect(hasGitCommitClaim("I inspected the code and ran all tests successfully.")).toBe(false);
	});

	it("fails verification when commit is claimed but git HEAD did not advance", () => {
		const runner = new HarvestSubagentRunner(".");
		runner.prepareDelegation();

		const res = runner.verifyGitClaim("I implemented the fix and committed the changes to git.");
		expect(res.hasClaim).toBe(true);
		expect(res.verified).toBe(false);
		expect(res.error).toBeDefined();
	});
});

describe("Feature 3.12: Context-Tiered Dynamic Prompt Budgeting", () => {
	it("allocates correct budgets and turn counts across hardware tiers", () => {
		const tiny = getTierBudget(detectContextTier(8_000));
		expect(tiny.tier).toBe("tiny");
		expect(tiny.maxSystemPromptChars).toBe(10_000);
		expect(tiny.turnBudget).toBe(80);

		const small = getTierBudget(detectContextTier(32_000));
		expect(small.tier).toBe("small");
		expect(small.maxSystemPromptChars).toBe(16_000);
		expect(small.turnBudget).toBe(64);

		const standard = getTierBudget(detectContextTier(128_000));
		expect(standard.tier).toBe("standard");
		expect(standard.maxSystemPromptChars).toBe(40_000);
		expect(standard.turnBudget).toBe(48);

		const large = getTierBudget(detectContextTier(1_000_000));
		expect(large.tier).toBe("large");
		expect(large.maxSystemPromptChars).toBe(80_000);
		expect(large.turnBudget).toBe(40);
	});

	it("assembleHarvestSystemPrompt safely handles tiny prompt budgets without emitting broken XML", () => {
		const assembled = assembleHarvestSystemPrompt({
			basePrompt: "Base instructions",
			role: "backend",
			contextWindowTokens: 4_000, // tiny tier
			skillsPrompt: "<harvest-skills>\n- skill1: desc1\n- skill2: desc2\n</harvest-skills>",
			knowledgePrompt: "<harvest-knowledge>\n- pattern1: rule1\n</harvest-knowledge>",
		});

		expect(assembled.prompt).toContain("<harvest-skills>");
		expect(assembled.prompt).toContain("</harvest-skills>");
		expect(assembled.prompt).toContain("<harvest-knowledge>");
		expect(assembled.prompt).toContain("</harvest-knowledge>");
		expect(assembled.contractInjected).toBe(true);
	});
});

describe("Feature 3.13: Deterministic Compaction with Verification Preservation", () => {
	it("extracts read files, modified files, and marks tests as STALE if touched after pass", () => {
		const calls = [
			{ name: "read", args: { path: "src/user.ts" }, timestamp: 10 },
			{ name: "edit", args: { path: "src/user.ts" }, timestamp: 20 },
			{ name: "bash", args: { command: "npm test" }, result: { exitCode: 0 }, timestamp: 30 },
			// File modified AFTER test passed
			{ name: "write", args: { path: "src/auth.ts" }, timestamp: 40 },
		];

		const evidence = extractTranscriptEvidence(calls);
		expect(evidence.readFiles).toEqual(["src/user.ts"]);
		expect(evidence.modifiedFiles).toContain("src/user.ts");
		expect(evidence.modifiedFiles).toContain("src/auth.ts");

		const testRecord = evidence.verificationRecords.find(v => v.command === "npm test");
		expect(testRecord).toBeDefined();
		expect(testRecord!.status).toBe("STALE");
		expect(testRecord!.stalenessReason).toContain("src/auth.ts");
	});

	it("DeterministicCompactor produces formatted summary and preserves evidence tags", () => {
		const compactor = new DeterministicCompactor();
		const result = compactor.compact([
			{ name: "read", args: { path: "index.ts" }, timestamp: 1 },
			{ name: "bash", args: { command: "cargo test" }, result: { exitCode: 1 }, timestamp: 2 },
		]);

		expect(result.formattedSummary).toContain("<compacted-session-evidence");
		expect(result.formattedSummary).toContain("- index.ts");
		expect(result.formattedSummary).toContain("[FAIL] `cargo test`");
		expect(result.formattedSummary).toContain("</compacted-session-evidence>");
	});
});

describe("Feature 3.14: Enterprise Sandbox & AST-Level Security Auditing", () => {
	it("Destructive Command Barrier blocks dangerous commands", () => {
		const sandbox = new SecuritySandbox(".");
		expect(sandbox.checkCommand("rm -rf /").allowed).toBe(false);
		expect(sandbox.checkCommand("rm -rf *").allowed).toBe(false);
		expect(sandbox.checkCommand("mkfs.ext4 /dev/sda1").allowed).toBe(false);
		expect(sandbox.checkCommand(":(){ :|:& };:").allowed).toBe(false);
		expect(sandbox.checkCommand("echo hello > /dev/sda").allowed).toBe(false);

		// Safe commands allowed
		expect(sandbox.checkCommand("git status").allowed).toBe(true);
		expect(sandbox.checkCommand("npm test").allowed).toBe(true);
	});

	it("assertPathJailed rejects path traversal outside workspace root", () => {
		const sandbox = new SecuritySandbox(".");
		const check = sandbox.assertPathJailed("../../../../../etc/shadow");
		expect(check.jailed).toBe(false);
		expect(check.error).toContain("resolves outside workspace root");
	});

	it("audits code for secrets, eval, and SQL injection", () => {
		const sandbox = new SecuritySandbox(".");
		const code = [
			"const token = 'ghp_123456789012345678901234567890123456';",
			"const query = 'SELECT * FROM users WHERE id = ' + userId;",
			"eval(untrustedInput);",
		].join("\n");

		const result = sandbox.auditCode(code);
		expect(result.safe).toBe(false);
		expect(result.findings.some(f => f.ruleId === "secret_api_key")).toBe(true);
		expect(result.findings.some(f => f.ruleId === "sql_concatenation")).toBe(true);
		expect(result.findings.some(f => f.ruleId === "eval_injection")).toBe(true);
	});
});

describe("Feature 3.15: Resilient Provider Normalization & Streaming JSON Repair", () => {
	it("repairJson repairs unescaped control chars and invalid escapes", () => {
		// Control chars inside string
		const raw = '{"code": "line 1\nline 2\ttab"}';
		const repaired = repairJson(raw);
		expect(() => JSON.parse(repaired)).not.toThrow();
		const parsed = JSON.parse(repaired) as { code: string };
		expect(parsed.code).toContain("line 1");
	});

	it("parseStreamingJson parses truncated or incomplete JSON safely", () => {
		const truncated = '{"path": "src/index.ts", "edits": [{"oldText": "foo"';
		const parsed = parseStreamingJson<{ path: string; edits: Array<{ oldText: string }> }>(truncated);
		expect(parsed).toBeDefined();
		expect(parsed.path).toBe("src/index.ts");
	});
});
