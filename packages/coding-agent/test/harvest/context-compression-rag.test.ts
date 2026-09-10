import { describe, expect, it } from "bun:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import {
	assembleHarvestPrompt,
	HarvestContextCompactor,
	HarvestKnowledgeGraph,
	HarvestPromptAssembler,
	HarvestSubagentCoordinator,
	indexGraphToPages,
	indexLocalMemoryToPages,
	indexSkillsToPages,
	LocalMemoryStore,
	MultiAgentContextDistributor,
	safeTruncateXml,
	UnifiedMemoryRetriever,
} from "../../src/core/harvest";
import { getTierBudget } from "../../src/core/harvest/model-tier";
import { assembleHarvestSystemPrompt } from "../../src/core/system-prompt";

function createTempDir(prefix: string): string {
	const tmpBase = os.tmpdir();
	const dir = path.join(tmpBase, `harvest-test-${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`);
	fs.mkdirSync(dir, { recursive: true });
	return dir;
}

describe("Feature: Local Memory Store & Staleness Tracking", () => {
	it("records file operations, working memory notes, and updates staleness on edit", () => {
		const store = new LocalMemoryStore("test-session");

		store.recordReadFile("src/server.ts");
		store.recordReadFile("src/config.ts");
		store.recordSkill("database-migrate");
		store.recordSymbol("handleAuthRequest");

		const note = store.addWorkingMemoryItem(
			"hypothesis",
			"Auth Token Expiration Race",
			"Session token expires before refresh payload reaches server.",
			["auth", "race-condition"],
		);
		expect(note.id).toBeDefined();
		expect(note.category).toBe("hypothesis");

		// Record a passing test at t=100
		store.recordVerification({
			command: "bun test test/auth.test.ts",
			status: "PASS",
			timestamp: 100,
			exitCode: 0,
		});

		let snap = store.snapshot();
		expect(snap.readFiles).toContain("src/server.ts");
		expect(snap.readFiles).toContain("src/config.ts");
		expect(snap.workingMemoryItems.length).toBe(1);
		expect(snap.verificationRecords[0].status).toBe("PASS");

		// Modify a file at t=200 (after the test ran) -> verification becomes STALE
		store.recordModifiedFile("src/server.ts", 200);

		snap = store.snapshot();
		expect(snap.modifiedFiles).toContain("src/server.ts");
		expect(snap.verificationRecords[0].status).toBe("STALE");
		expect(snap.verificationRecords[0].stalenessReason).toContain("'src/server.ts'");
	});

	it("ingests transcript tool calls and extracts file operations and verifications", () => {
		const store = new LocalMemoryStore("transcript-session");

		store.ingestToolCalls([
			{ name: "read", args: { path: "src/index.ts" } },
			{ name: "replace_file_content", args: { TargetFile: "src/user.ts" }, timestamp: 50 },
			{
				name: "bash",
				args: { command: "bun test test/user.test.ts" },
				result: { exitCode: 0, success: true },
				timestamp: 100,
			},
			{ name: "write", args: { path: "src/user.ts" }, timestamp: 150 },
		]);

		const snap = store.snapshot();
		expect(snap.readFiles).toContain("src/index.ts");
		expect(snap.modifiedFiles).toContain("src/user.ts");
		expect(snap.verificationRecords.length).toBe(1);
		// Was modified at t=150 after test at t=100 -> STALE
		expect(snap.verificationRecords[0].status).toBe("STALE");
	});

	it("detects staleness when a verification is recorded with a timestamp earlier than an existing file modification", () => {
		const store = new LocalMemoryStore("out-of-order-session");
		// File modified at t=200
		store.recordModifiedFile("src/auth.ts", 200);

		// Record a verification that was executed at t=100 (prior to file edit)
		store.recordVerification({
			command: "bun test test/auth.test.ts",
			status: "PASS",
			timestamp: 100,
			exitCode: 0,
		});

		const snap = store.snapshot();
		expect(snap.verificationRecords[0].status).toBe("STALE");
		expect(snap.verificationRecords[0].stalenessReason).toContain("'src/auth.ts'");
	});

	it("manages working memory item updates, removals, and category queries", () => {
		const store = new LocalMemoryStore("wm-lifecycle");
		const item = store.addWorkingMemoryItem("hypothesis", "Initial Hypothesis", "Maybe cache is stale.");

		expect(store.getWorkingMemoryItems("hypothesis").length).toBe(1);
		expect(store.getWorkingMemoryItems("goal").length).toBe(0);

		// Update item
		const updated = store.updateWorkingMemoryItem(item.id, {
			category: "decision",
			title: "Confirmed Decision",
			content: "Purge cache on write.",
		});
		expect(updated).toBe(true);

		const decisions = store.getWorkingMemoryItems("decision");
		expect(decisions.length).toBe(1);
		expect(decisions[0].title).toBe("Confirmed Decision");

		// Remove item
		const removed = store.removeWorkingMemoryItem(item.id);
		expect(removed).toBe(true);
		expect(store.getWorkingMemoryItems().length).toBe(0);
	});
});

describe("Feature: Vectorless RAG with Page Index for Local and Global Memory", () => {
	it("indexes typed knowledge graph nodes and relations into searchable section pages", () => {
		const tmp = createTempDir("graph-rag");
		const graph = new HarvestKnowledgeGraph(tmp);

		graph.addNode({
			id: "auth-oauth2",
			role: "backend",
			kind: "pattern",
			title: "OAuth2 Authorization Flow",
			path: path.join(tmp, ".harvest/backend/patterns/oauth2.md"),
			metadata: { provider: "github" },
		});
		graph.addNode({
			id: "jwt-anti-pattern",
			role: "backend",
			kind: "anti-pattern",
			title: "Raw JWT in LocalStorage",
			path: path.join(tmp, ".harvest/backend/anti-patterns/jwt.md"),
		});
		graph.addEdge("auth-oauth2", "jwt-anti-pattern", "contradicts");

		const pages = indexGraphToPages(graph);
		expect(pages.length).toBe(2);

		const oauthPage = pages.find(p => p.id === "graph://node/auth-oauth2")!;
		expect(oauthPage).toBeDefined();
		expect(oauthPage.title).toContain("OAuth2 Authorization Flow");
		expect(oauthPage.content).toContain("contradicts -> jwt-anti-pattern");
		expect(oauthPage.tokens).toContain("oauth2");
	});

	it("indexes local session memory items into distinct section pages", () => {
		const store = new LocalMemoryStore("local-rag");
		store.recordReadFile("packages/auth/token.ts");
		store.recordModifiedFile("packages/auth/session.ts");
		store.recordVerification({
			command: "bun test packages/auth/test.ts",
			status: "PASS",
			timestamp: 50,
			exitCode: 0,
		});
		store.addWorkingMemoryItem("goal", "Implement PKCE Auth", "Add SHA256 code challenge to OAuth login.");

		const pages = indexLocalMemoryToPages(store);
		expect(pages.some(p => p.id === "local://files/read")).toBe(true);
		expect(pages.some(p => p.id === "local://files/modified")).toBe(true);
		expect(pages.some(p => p.id === "local://verifications/audit")).toBe(true);
		expect(pages.some(p => p.id.startsWith("local://wm/"))).toBe(true);

		const goalPage = pages.find(p => p.id.startsWith("local://wm/"))!;
		expect(goalPage.heading).toBe("Implement PKCE Auth");
		expect(goalPage.tokens).toContain("pkce");
	});

	it("indexes approved skills into section pages", () => {
		const skills = [
			{
				name: "docker-compose-test",
				description: "Run integration tests inside isolated Docker network",
				filePath: "/skills/docker.md",
				content: "Use docker compose up -d and wait for healthchecks.",
			},
		];

		const pages = indexSkillsToPages(skills);
		expect(pages.length).toBe(1);
		expect(pages[0].id).toBe("skill://docker-compose-test");
		expect(pages[0].tokens).toContain("docker");
	});

	it("retrieves and ranks relevant pages across local and global memory with BM25", () => {
		const tmp = createTempDir("unified-retrieval");

		// Global pattern file
		const patternsDir = path.join(tmp, ".harvest", "backend", "patterns");
		fs.mkdirSync(patternsDir, { recursive: true });
		fs.writeFileSync(
			path.join(patternsDir, "jwt-refresh.md"),
			`---\nrole: backend\ntitle: Safe JWT Refresh Strategy\n---\n## Implementation\nRotate refresh tokens on every request and use httpOnly cookies.\n`,
		);

		// Global graph node
		const graph = new HarvestKnowledgeGraph(tmp);
		graph.addNode({
			id: "cookie-security",
			role: "backend",
			kind: "pattern",
			title: "HttpOnly Cookie Storage",
			path: path.join(patternsDir, "cookie-security.md"),
		});

		// Local memory
		const localStore = new LocalMemoryStore("session-rag");
		localStore.recordReadFile("src/auth/jwt.ts");
		localStore.addWorkingMemoryItem(
			"decision",
			"Rotate JWT Refresh Tokens",
			"Decided to adopt httpOnly rotating cookie tokens.",
		);

		const retriever = new UnifiedMemoryRetriever({
			workspaceRoot: tmp,
			graph,
			localMemory: localStore,
		});

		const result = retriever.retrieve("httpOnly cookie refresh tokens");

		expect(result.allResults.length).toBeGreaterThan(0);
		expect(result.localResults.length).toBeGreaterThan(0);
		expect(result.globalResults.length).toBeGreaterThan(0);
		expect(result.formattedContext).toContain("<harvest-retrieved-context>");
		expect(result.formattedContext).toContain("<local-memory>");
		expect(result.formattedContext).toContain("<global-memory>");
		expect(result.formattedContext).toContain("Rotate JWT Refresh Tokens");
	});
});

describe("Feature: Context Compression with Prompt Budgeting", () => {
	it("compresses local and global memory within tier character budgets and repoints graph edges", () => {
		const tmp = createTempDir("compaction-budget");
		const graph = new HarvestKnowledgeGraph(tmp);

		graph.addNode({
			id: "old-task-node-1",
			role: "backend",
			kind: "task",
			title: "Initial Task Setup",
			path: path.join(tmp, "task1.md"),
		});
		graph.addNode({
			id: "target-service",
			role: "backend",
			kind: "pattern",
			title: "Service Implementation",
			path: path.join(tmp, "service.md"),
		});
		graph.addEdge("old-task-node-1", "target-service", "depends_on");

		const store = new LocalMemoryStore("compact-session");
		store.recordReadFile("src/database.ts");
		store.recordModifiedFile("src/database.ts", 100);
		store.recordVerification({
			command: "bun test test/db.test.ts",
			status: "PASS",
			timestamp: 150,
			exitCode: 0,
		});
		store.addWorkingMemoryItem("finding", "Database Index Missing", "Added compound index on (user_id, created_at).");

		const compactor = new HarvestContextCompactor(tmp, graph);
		const tinyBudget = getTierBudget("tiny");

		const result = compactor.compressContext(store, {
			taskQuery: "database indexing compound performance",
			targetBudget: tinyBudget,
			summaryId: "compacted-db-summary-1",
			compactedNodeIds: ["old-task-node-1"],
			generalSummaryText: "Optimized database queries with compound indexing.",
		});

		expect(result.summaryId).toBe("compacted-db-summary-1");
		expect(result.readFiles).toContain("src/database.ts");
		expect(result.modifiedFiles).toContain("src/database.ts");
		expect(result.verificationRecords.length).toBe(1);
		expect(result.formattedContext).toContain('<harvest-context-compression id="compacted-db-summary-1">');
		expect(result.formattedContext).toContain("Database Index Missing");

		// Graph edge from old-task-node-1 should be repointed to compacted-db-summary-1
		const outgoing = graph.getOutgoingEdges("compacted-db-summary-1");
		expect(outgoing.length).toBe(1);
		expect(outgoing[0].target).toBe("target-service");
		expect(outgoing[0].relation).toBe("depends_on");
	});

	it("preserves verification audit records and modified files when local memory contains many read files", () => {
		const tmp = createTempDir("compaction-priority");
		const store = new LocalMemoryStore("large-session");

		// Simulate 40 read files
		for (let i = 0; i < 40; i++) {
			store.recordReadFile(`src/module-${i}/file-${i}.ts`);
		}
		// Critical modified file
		store.recordModifiedFile("src/core/critical.ts", 100);
		// Critical verification records
		store.recordVerification({
			command: "bun test test/critical.test.ts",
			status: "PASS",
			timestamp: 150,
			exitCode: 0,
		});

		const compactor = new HarvestContextCompactor(tmp);
		const tinyBudget = getTierBudget("tiny");

		const result = compactor.compressContext(store, {
			taskQuery: "critical module implementation and verification",
			targetBudget: tinyBudget,
			summaryId: "compact-priority-1",
		});

		// Total characters strictly within budget
		const maxAllowed = tinyBudget.maxKnowledgeChars + tinyBudget.maxInlineSkillsChars;
		expect(result.totalChars).toBeLessThanOrEqual(maxAllowed);
		expect(result.withinBudget).toBe(true);

		// Critical verification and modified files MUST be preserved
		expect(result.formattedContext).toContain("Verification Audit Records");
		expect(result.formattedContext).toContain("bun test test/critical.test.ts");
		expect(result.formattedContext).toContain("Modified Files");
		expect(result.formattedContext).toContain("src/core/critical.ts");
	});

	it("safely balances nested XML tags and stays strictly within character limits", () => {
		const deeplyNested = `<subagent-scoped-context role="backend"><harvest-retrieved-context><local-memory><active-findings>Important finding content here</active-findings></local-memory></harvest-retrieved-context></subagent-scoped-context>`;

		const truncated = safeTruncateXml(deeplyNested, 120, "subagent-scoped-context");
		expect(truncated.length).toBeLessThanOrEqual(120);
		expect(truncated).toContain("...[budget truncated]");
		expect(truncated).toContain("</subagent-scoped-context>");

		// Assert that open and close tags match in count
		const openCount = (truncated.match(/<[a-zA-Z0-9_-]+(?:\s+[^>]*)?>/g) || []).length;
		const closeCount = (truncated.match(/<\/[a-zA-Z0-9_-]+>/g) || []).length;
		expect(openCount).toBe(closeCount);
	});
});

describe("Feature: Prompt Assembler with Memory & Tier Budgeting", () => {
	it("assembles complete prompt injecting coding contract, role directives, and scoped memory", () => {
		const tmp = createTempDir("prompt-assemble");
		const store = new LocalMemoryStore("prompt-session");
		store.recordReadFile("src/api/routes.ts");
		store.recordModifiedFile("src/api/routes.ts");
		store.addWorkingMemoryItem("goal", "API Rate Limiting", "Implement token bucket rate limiter middleware.");

		const assembler = new HarvestPromptAssembler(tmp);
		const res = assembler.assemble({
			basePrompt: "You are an autonomous engineering assistant.",
			role: "backend",
			intent: "mutation",
			taskQuery: "API rate limiting token bucket",
			localMemory: store,
			contextWindowTokens: 32_000, // small tier
		});

		expect(res.role).toBe("backend");
		expect(res.tierBudget.tier).toBe("small");
		expect(res.contractInjected).toBe(true);
		expect(res.prompt).toContain("<coding-contract>");
		expect(res.prompt).toContain("# Role Directive: Backend Specialist");
		expect(res.prompt).toContain("<harvest-context-compression");
		expect(res.prompt).toContain("API Rate Limiting");
		expect(res.prompt.length).toBeLessThanOrEqual(res.tierBudget.maxSystemPromptChars);

		// Direct functional export
		const directRes = assembleHarvestPrompt({
			basePrompt: "Functional assembler test.",
			role: "backend",
			intent: "mutation",
			taskQuery: "API rate limiting",
			localMemory: store,
			workspaceRoot: tmp,
		});
		expect(directRes.role).toBe("backend");
		expect(directRes.contractInjected).toBe(true);
	});

	it("integrates seamlessly through assembleHarvestSystemPrompt facade", () => {
		const tmp = createTempDir("facade-assemble");
		const store = new LocalMemoryStore("facade-session");
		store.addWorkingMemoryItem("note", "Cache Key Strategy", "Use sha256 of request query as redis cache key.");

		const assembled = assembleHarvestSystemPrompt({
			basePrompt: "Base prompt instructions.",
			role: "backend",
			intent: "complex",
			taskQuery: "cache key redis query",
			localMemory: store,
			workspaceRoot: tmp,
			contextWindowTokens: 128_000, // standard tier
		});

		expect(assembled.role).toBe("backend");
		expect(assembled.tierBudget.tier).toBe("standard");
		expect(assembled.contractInjected).toBe(true);
		expect(assembled.prompt).toContain("Cache Key Strategy");
	});
});

describe("Feature: Subagent Spawning & Multi-Agent Context Distribution", () => {
	it("prepares scoped delegation context without parent transcript dumps", () => {
		const tmp = createTempDir("subagent-delegation");
		const parentStore = new LocalMemoryStore("parent-session");

		// Parent has inspected multiple files
		parentStore.recordReadFile("src/backend/auth.ts");
		parentStore.recordReadFile("src/frontend/components/Button.tsx");
		parentStore.recordReadFile("src/frontend/components/Modal.tsx");
		parentStore.recordModifiedFile("src/frontend/components/Button.tsx");

		// Parent has multiple working memory notes
		parentStore.addWorkingMemoryItem(
			"finding",
			"Modal UI Glitch",
			"Tailwind flex wrap causes modal overflow on mobile.",
		);
		parentStore.addWorkingMemoryItem("finding", "Auth DB Timeout", "Database connection pool exhausted.");

		const coordinator = new HarvestSubagentCoordinator(tmp);

		// Delegate specifically to frontend specialist
		const delegation = coordinator.prepareScopedDelegation({
			parentRole: "coordinator",
			targetRole: "frontend",
			assignment: "Fix modal flex wrap styling in Modal.tsx to prevent mobile overflow",
			targetTier: "small",
			parentLocalMemory: parentStore,
			workspaceRoot: tmp,
		});

		expect(delegation.targetRole).toBe("frontend");
		expect(delegation.tierBudget.tier).toBe("small");
		expect(delegation.scopedPromptContext).toContain('<subagent-scoped-context role="frontend" tier="small">');
		expect(delegation.scopedPromptContext).toContain("# Delegation Directive: Frontend Specialist");
		expect(delegation.scopedPromptContext).toContain("Modal.tsx");
		// Should include relevant frontend files but NOT unrelated backend auth
		expect(delegation.relevantFiles).toContain("src/frontend/components/Modal.tsx");
		expect(delegation.relevantFiles).not.toContain("src/backend/auth.ts");
	});

	it("completes delegation and ingests subagent results into parent memory without transcript explosion", () => {
		const tmp = createTempDir("subagent-complete");
		const parentStore = new LocalMemoryStore("parent-main");
		const coordinator = new HarvestSubagentCoordinator(tmp);

		const result = coordinator.completeDelegation(
			"Refactored Modal.tsx to use responsive flex-col on mobile screens.",
			{
				subagentRole: "frontend",
				subagentModifiedFiles: ["src/frontend/components/Modal.tsx"],
				subagentReadFiles: ["src/frontend/components/Button.tsx"],
				parentLocalMemory: parentStore,
			},
		);

		expect(result.success).toBe(true);
		expect(result.synthesizedSummary).toContain("[Subagent Delegation Completed: frontend]");
		expect(result.stagedModifications).toContain("src/frontend/components/Modal.tsx");

		// Parent memory received updated files and structured note
		const parentSnap = parentStore.snapshot();
		expect(parentSnap.readFiles).toContain("src/frontend/components/Button.tsx");
		expect(parentSnap.modifiedFiles).toContain("src/frontend/components/Modal.tsx");
		expect(parentSnap.workingMemoryItems.some(n => n.title.includes("Subagent [frontend] Completion"))).toBe(true);
	});

	it("distributes scoped contexts across multiple specialist agents concurrently", () => {
		const tmp = createTempDir("multi-agent-dist");
		const parentStore = new LocalMemoryStore("parent-dist");
		parentStore.recordReadFile("src/api/payment.ts");
		parentStore.recordReadFile("src/ui/Checkout.tsx");

		const distributor = new MultiAgentContextDistributor(tmp);

		const contexts = distributor.distribute([
			{
				parentRole: "coordinator",
				targetRole: "backend",
				assignment: "Implement stripe webhook verification in payment.ts",
				targetTier: "small",
				parentLocalMemory: parentStore,
				workspaceRoot: tmp,
			},
			{
				parentRole: "coordinator",
				targetRole: "frontend",
				assignment: "Create checkout payment form in Checkout.tsx",
				targetTier: "small",
				parentLocalMemory: parentStore,
				workspaceRoot: tmp,
			},
			{
				parentRole: "coordinator",
				targetRole: "qa",
				assignment: "Write end-to-end checkout test suite",
				targetTier: "small",
				parentLocalMemory: parentStore,
				workspaceRoot: tmp,
			},
		]);

		expect(contexts.length).toBe(3);
		expect(contexts[0].targetRole).toBe("backend");
		expect(contexts[0].scopedPromptContext).toContain("Backend Specialist");
		expect(contexts[0].relevantFiles).toContain("src/api/payment.ts");

		expect(contexts[1].targetRole).toBe("frontend");
		expect(contexts[1].scopedPromptContext).toContain("Frontend Specialist");
		expect(contexts[1].relevantFiles).toContain("src/ui/Checkout.tsx");

		expect(contexts[2].targetRole).toBe("qa");
		expect(contexts[2].scopedPromptContext).toContain("QA & Verification Specialist");
	});

	it("prevents parent memory file list leaks into subagent prompt context", () => {
		const tmp = createTempDir("subagent-no-leak");
		const parentStore = new LocalMemoryStore("parent-large");

		// Parent has 30 unrelated files
		for (let i = 0; i < 30; i++) {
			parentStore.recordReadFile(`src/unrelated/legacy-${i}.ts`);
		}
		// Parent has 1 relevant file
		parentStore.recordReadFile("src/payment/stripe.ts");
		parentStore.recordModifiedFile("src/payment/stripe.ts");

		const coordinator = new HarvestSubagentCoordinator(tmp);
		const delegation = coordinator.prepareScopedDelegation({
			parentRole: "coordinator",
			targetRole: "backend",
			assignment: "Refactor Stripe webhook signature validation in stripe.ts",
			targetTier: "tiny",
			parentLocalMemory: parentStore,
			workspaceRoot: tmp,
		});

		expect(delegation.scopedPromptContext).toContain("src/payment/stripe.ts");
		// Unrelated files must NOT leak into the scoped context
		expect(delegation.scopedPromptContext).not.toContain("legacy-0.ts");
		expect(delegation.scopedPromptContext).not.toContain("legacy-10.ts");
		// Ensure tags are balanced even on tiny tier
		expect(delegation.scopedPromptContext).toContain("</subagent-scoped-context>");
	});

	it("detects preflight file conflicts and synthesizes multi-agent delegation results", () => {
		const tmp = createTempDir("multi-agent-conflicts");
		const parentStore = new LocalMemoryStore("parent-multi");
		parentStore.recordModifiedFile("src/common/schema.ts");
		parentStore.recordModifiedFile("src/frontend/App.tsx");
		parentStore.recordModifiedFile("src/backend/server.ts");

		const distributor = new MultiAgentContextDistributor(tmp);

		// Both backend and frontend target schema.ts
		const conflicts = distributor.detectPreflightConflicts([
			{
				parentRole: "coordinator",
				targetRole: "backend",
				assignment: "Update schema.ts definitions for auth API",
				targetTier: "small",
				parentLocalMemory: parentStore,
				workspaceRoot: tmp,
			},
			{
				parentRole: "coordinator",
				targetRole: "frontend",
				assignment: "Consume schema.ts in UI forms",
				targetTier: "small",
				parentLocalMemory: parentStore,
				workspaceRoot: tmp,
			},
		]);

		expect(conflicts.length).toBe(1);
		expect(conflicts[0].file).toBe("src/common/schema.ts");
		expect(conflicts[0].roles).toContain("backend");
		expect(conflicts[0].roles).toContain("frontend");

		// Synthesize results from both subagents
		const report = distributor.synthesizeMultiAgentResults(
			[
				{
					role: "backend",
					summary: "Added user schema types to schema.ts",
					modifiedFiles: ["src/common/schema.ts", "src/backend/server.ts"],
				},
				{
					role: "frontend",
					summary: "Updated form fields with schema.ts types",
					modifiedFiles: ["src/common/schema.ts", "src/frontend/App.tsx"],
				},
			],
			parentStore,
		);

		expect(report.totalDelegations).toBe(2);
		expect(report.successful).toBe(2);
		expect(report.hasConflicts).toBe(true);
		expect(report.fileConflicts.length).toBe(1);
		expect(report.fileConflicts[0].filePath).toBe("src/common/schema.ts");
		expect(report.stagedModifications).toContain("src/common/schema.ts");
		expect(report.stagedModifications).toContain("src/backend/server.ts");
		expect(report.stagedModifications).toContain("src/frontend/App.tsx");
		expect(report.synthesizedSummary).toContain("Warning: Conflicting Modifications Detected");

		// Ingested into parent local memory
		const snap = parentStore.snapshot();
		expect(snap.modifiedFiles).toContain("src/backend/server.ts");
		expect(snap.modifiedFiles).toContain("src/frontend/App.tsx");
	});
});
