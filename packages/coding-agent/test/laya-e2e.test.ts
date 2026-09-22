/**
 * End-to-End Test Suite for Laya Local Decision Layer in Harvest.
 *
 * Covers:
 * 1. Normal low-risk tasks (no behavior change vs baseline).
 * 2. Tasks triggering gated high-risk tools (fail-closed, feeds into approval).
 * 3. Ambiguous / low-confidence state handling (escalation, no silent wrong decisions).
 * 4. Sidecar-down resilience (fail CLOSED for gating, fail OPEN for routing & completion).
 * 5. Non-English input bypass (bypasses English-only ModernBERT checkpoint).
 * 6. Latency deltas & benchmarking.
 */

import { describe, expect, it } from "bun:test";
import { LayaClient } from "../src/core/harvest/laya-client";
import { checkToolCallGating } from "../src/core/harvest/laya-gating";
import { routeModelWithLaya, routeSpecialistRoleWithLaya } from "../src/core/harvest/laya-routing";
import { checkCompletionWithLaya } from "../src/core/harvest/laya-completion";

const LIVE_SIDECAR_URL = process.env.LAYA_SIDECAR_URL || "http://127.0.0.1:8177";
const DEAD_SIDECAR_URL = "http://127.0.0.1:9999";

describe("Laya Local Decision Layer in Harvest (End-to-End)", () => {
	const liveClient = new LayaClient({ baseUrl: LIVE_SIDECAR_URL, timeoutMs: 8000 });
	const deadClient = new LayaClient({ baseUrl: DEAD_SIDECAR_URL, timeoutMs: 100 });

	// --- Scenario 1: Normal Low-Risk Tasks ---
	describe("Scenario 1: Normal Low-Risk Tasks", () => {
		it("bypasses gating entirely for read-only tools without sidecar call", async () => {
			const start = performance.now();
			const readGating = await checkToolCallGating("read", { path: "src/index.ts" }, { client: liveClient });
			const globGating = await checkToolCallGating("glob", { pattern: "**/*.ts" }, { client: liveClient });
			const grepGating = await checkToolCallGating("grep", { query: "export" }, { client: liveClient });
			const latencyMs = performance.now() - start;

			expect(readGating.isHighRiskTool).toBe(false);
			expect(readGating.requireApproval).toBe(false);
			expect(globGating.isHighRiskTool).toBe(false);
			expect(globGating.requireApproval).toBe(false);
			expect(grepGating.isHighRiskTool).toBe(false);
			expect(grepGating.requireApproval).toBe(false);
			// Low-risk bypass is instantaneous (< 10ms for all 3)
			expect(latencyMs).toBeLessThan(15);
		});
	});

	// --- Scenario 2: Gated High-Risk Tools ---
	describe("Scenario 2: Gated High-Risk Tools & Approval Flow", () => {
		it("detects destructive commands and requires approval", async () => {
			const isLive = await liveClient.isHealthy();
			if (!isLive) {
				console.warn("Live sidecar not running on 8177, skipping live inference assertion");
				return;
			}

			const start = performance.now();
			const result = await checkToolCallGating(
				"bash",
				{ command: "rm -rf /projects/harvest/dist && rm -f package-lock.json" },
				{ client: liveClient },
			);
			const elapsed = performance.now() - start;

			expect(result.isHighRiskTool).toBe(true);
			expect(result.requireApproval).toBe(true);
			expect(result.fallback).toBe(false);
			expect(result.irreversibleScore).toBeGreaterThanOrEqual(0.40);
			expect(result.confidence).toBeGreaterThan(0.5);
			console.log(`[Scenario 2: Destructive Bash] Latency: ${elapsed.toFixed(1)}ms, noul: ${result.irreversibleScore}`);
		});

		it("classifies benign non-destructive commands as safe", async () => {
			const isLive = await liveClient.isHealthy();
			if (!isLive) return;

			const result = await checkToolCallGating(
				"bash",
				{ command: "git status && git log -n 5" },
				{ client: liveClient },
			);

			expect(result.isHighRiskTool).toBe(true);
			expect(result.fallback).toBe(false);
			// git status is non-destructive
			expect(result.irreversibleScore).toBeLessThan(0.40);
			expect(result.requireApproval).toBe(false);
		});
	});

	// --- Scenario 3: Ambiguous / Low-Confidence State ---
	describe("Scenario 3: Ambiguous & Low-Confidence State Handling", () => {
		it("does not blindly auto-approve ambiguous state", async () => {
			// When command has ambiguous flags or ambiguous file target, fail-closed policy requires approval
			const result = await checkToolCallGating(
				"bash",
				{ command: "chmod 777 /var/run/docker.sock && echo done" },
				{ client: liveClient },
			);

			expect(result.isHighRiskTool).toBe(true);
			expect(result.requireApproval).toBe(true);
		});

		it("routes ambiguous multi-specialist tasks with choice criteria", async () => {
			const isLive = await liveClient.isHealthy();
			if (!isLive) return;

			const result = await routeSpecialistRoleWithLaya(
				"Implement database migration for postgres user table and create react modal form",
				["backend", "frontend"],
				{ client: liveClient, defaultRole: "backend" },
			);

			expect(result.fallback).toBe(false);
			expect(["backend", "frontend"]).toContain(result.selectedRole);
		});
	});

	// --- Scenario 4: Sidecar-Down Resilience (Fail CLOSED vs Fail OPEN) ---
	describe("Scenario 4: Sidecar-Down Fallback Contracts", () => {
		it("Call Site 1 (Tool-Call Gating): FAILS CLOSED when sidecar is down", async () => {
			const start = performance.now();
			const result = await checkToolCallGating(
				"bash",
				{ command: "echo test" },
				{ client: deadClient },
			);
			const elapsed = performance.now() - start;

			expect(result.isHighRiskTool).toBe(true);
			// Mandatory Contract: Tool-call gating MUST fail CLOSED (require approval)
			expect(result.requireApproval).toBe(true);
			expect(result.fallback).toBe(true);
			expect(result.reason).toContain("fallback");
			expect(elapsed).toBeLessThan(350); // fast timeout
		});

		it("Call Site 2 (Model Routing): FAILS OPEN when sidecar is down", async () => {
			const result = await routeModelWithLaya(
				"Refactor the authentication middleware connection pool",
				{ client: deadClient, defaultRole: "default" },
			);

			// Mandatory Contract: Model routing MUST fail OPEN to existing default
			expect(result.selectedRole).toBe("default");
			expect(result.fallback).toBe(true);
		});

		it("Call Site 3 (Completion Checks): FAILS OPEN when sidecar is down", async () => {
			const result = await checkCompletionWithLaya(
				{ command: "npm test", output: "Tests passed cleanly" },
				{ client: deadClient },
			);

			// Mandatory Contract: Completion checks MUST fail OPEN to existing full-LLM evaluation
			expect(result.isSuccess).toBe(true);
			expect(result.isPrematureStop).toBe(false);
			expect(result.fallback).toBe(true);
		});
	});

	// --- Scenario 5: Non-English Input Bypass ---
	describe("Scenario 5: Non-English Input Bypass", () => {
		it("client-side language guard immediately bypasses Laya on non-English text", () => {
			expect(liveClient.isEnglish("Hello world, please run git status.")).toBe(true);
			expect(liveClient.isEnglish("Bonjour tout le monde, veuillez supprimer tous les fichiers.")).toBe(true); // short latin
			expect(liveClient.isEnglish("こんにちは世界、テストを実行してください。")).toBe(false); // CJK
			expect(liveClient.isEnglish("Привет мир, удали эти файлы сейчас.")).toBe(false); // Cyrillic
			expect(liveClient.isEnglish("مرحبا بالعالم، قم بتشغيل الاختبار")).toBe(false); // Arabic
		});

		it("bypasses Laya without sending non-English text to checkpoint", async () => {
			const result = await checkToolCallGating(
				"bash",
				{ command: "echo 'Привет мир, пожалуйста удали это'" },
				{ client: liveClient },
			);

			expect(result.fallback).toBe(true);
			expect(result.requireApproval).toBe(true); // fail closed
			expect(result.reason).toBe("fallback_non_english_input");
		});

		it("model routing falls open on non-English prompt", async () => {
			const result = await routeModelWithLaya(
				"请实现一个新的用户认证模块并编写单元测试",
				{ client: liveClient, defaultRole: "default" },
			);

			expect(result.fallback).toBe(true);
			expect(result.selectedRole).toBe("default");
			expect(result.fallbackReason).toBe("non_english_input");
		});
	});

	// --- Latency Benchmarking & Performance ---
	describe("Performance & Latency Deltas", () => {
		it("measures decision call latency on live sidecar vs fallback", async () => {
			const isLive = await liveClient.isHealthy();
			if (!isLive) return;

			// Live call
			const t0 = performance.now();
			const liveDecision = await liveClient.decide(
				"npm run build",
				{
					build_check: {
						type: "noul",
						instructions: "does this command perform destructive deletions?",
					},
				},
				{ callSite: "benchmark" },
			);
			const liveDuration = performance.now() - t0;

			// Fallback call (dead endpoint)
			const t1 = performance.now();
			const deadDecision = await deadClient.decide(
				"npm run build",
				{
					build_check: {
						type: "noul",
						instructions: "does this command perform destructive deletions?",
					},
				},
				{ callSite: "benchmark" },
			);
			const fallbackDuration = performance.now() - t1;

			expect(liveDecision.success).toBe(true);
			expect(deadDecision.fallback).toBe(true);
			console.log(`[Benchmark] Live Sidecar: ${liveDuration.toFixed(1)}ms | Fallback: ${fallbackDuration.toFixed(1)}ms`);
		});
	});
});
