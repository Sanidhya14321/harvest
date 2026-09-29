import { afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import type { AgentMessage } from "@harvest/pi-agent-core";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { createMarkdownBrain, MarkdownBrain } from "../src/core/harvest/brain";
import { LayaClient } from "../src/core/harvest/laya-client";

let root: string;
beforeEach(async () => {
	root = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-brain-"));
});
afterEach(async () => {
	vi.restoreAllMocks();
	await fs.rm(root, { recursive: true, force: true });
});

describe("Markdown brains", () => {
	it("rejects a brain directory junction that escapes the project jail", async () => {
		const project = path.join(root, "project");
		const outside = path.join(root, "outside");
		await fs.mkdir(path.join(project, ".harvest"), { recursive: true });
		await Bun.write(path.join(outside, "secret.md"), "## Database\nDatabase private outside material.");
		await fs.symlink(outside, path.join(project, ".harvest", "brain"), "junction");
		expect(await createMarkdownBrain(project, path.join(root, "agent")).retrieve("database")).toEqual([]);
	});

	it("retrieves for the latest user request rather than the original session topic", async () => {
		await Bun.write(path.join(root, "database.md"), "## Database\nDatabase schema migrations.");
		await Bun.write(path.join(root, "ui.md"), "## Interface\nAccessibility keyboard navigation.");
		const brain = new MarkdownBrain([{ scope: "project", directory: root }]);
		const messages: AgentMessage[] = [
			{ role: "user", timestamp: 1, content: "database" },
			{ role: "user", timestamp: 2, content: "accessibility" },
		];
		const transformed = await brain.transform(messages);
		expect(transformed[0]).toBe(messages[0]);
		expect(JSON.stringify(transformed[1])).toContain("keyboard navigation");
		expect(JSON.stringify(transformed[1])).not.toContain("schema migrations");
	});
	it("retrieves project and user pages with distinct identities and follows explicit graph dependencies", async () => {
		await Bun.write(
			path.join(root, "project", "deploy.md"),
			"---\nid: deploy\ndepends_on: [user:preferences]\n---\n## Deployment\nDeploy releases through the staging cluster.",
		);
		await Bun.write(
			path.join(root, "user", "preferences.md"),
			"---\nid: preferences\n---\n## Communication\nExplain tradeoffs before expensive work.",
		);
		await Bun.write(path.join(root, "user", "deploy.md"), "## Deployment\nDeploy personal apps using containers.");
		const brain = new MarkdownBrain([
			{ scope: "project", directory: path.join(root, "project") },
			{ scope: "user", directory: path.join(root, "user") },
		]);
		const pages = await brain.retrieve("deploy");
		expect(pages.map(page => page.documentId)).toContain("user:preferences");
		expect(
			pages
				.filter(page => page.heading === "Deployment")
				.map(page => page.scope)
				.sort(),
		).toEqual(["project", "user"]);
		expect(new Set(pages.map(page => page.id)).size).toBe(pages.length);
	});

	it("refresh replaces changed pages and removes deleted or superseded knowledge", async () => {
		const file = path.join(root, "brain", "architecture.md");
		const brain = new MarkdownBrain([{ scope: "project", directory: path.dirname(file) }]);
		await Bun.write(file, "## Database\nSQLite stores events.");
		expect((await brain.retrieve("SQLite"))[0].content).toContain("SQLite");
		await Bun.write(file, "## Database\nPostgres stores events now.");
		await brain.refresh();
		expect(await brain.retrieve("SQLite")).toEqual([]);
		expect((await brain.retrieve("Postgres"))[0].content).toContain("Postgres");
		await Bun.write(file, "---\nsuperseded: true\n---\nPostgres stores events now.");
		await brain.refresh();
		expect(await brain.retrieve("Postgres")).toEqual([]);
		await fs.rm(file);
		await brain.refresh();
		expect(await brain.retrieve("events")).toEqual([]);
	});

	it("uses one batched Laya call to rerank and keeps lexical ordering when the sidecar fails", async () => {
		await Bun.write(path.join(root, "a.md"), "## Database\nDatabase database database transactions.");
		await Bun.write(path.join(root, "b.md"), "## Database\nDatabase backups restore recovery.");
		const brain = new MarkdownBrain([{ scope: "project", directory: root }]);
		const lexical = await brain.retrieve("database");
		const client = new LayaClient();
		const decide = vi.spyOn(client, "decide").mockResolvedValue({
			success: true,
			fallback: false,
			latencyMs: 1,
			data: {
				brain_0: { type: "score", score: 0, confidence: 1 },
				brain_1: { type: "score", score: 3, confidence: 1 },
			},
		});
		expect((await brain.retrieve("database", { client, rerank: true }))[0].id).toBe(lexical[1].id);
		expect(decide).toHaveBeenCalledTimes(1);
		decide.mockResolvedValue({ success: false, fallback: true, fallbackReason: "timeout", latencyMs: 1 });
		expect((await brain.retrieve("database", { client, rerank: true })).map(page => page.id)).toEqual(
			lexical.map(page => page.id),
		);
		decide.mockResolvedValue({
			success: true,
			fallback: false,
			latencyMs: 1,
			data: { brain_0: { type: "score", score: Number.NaN, confidence: 1 } },
		});
		expect((await brain.retrieve("database", { client, rerank: true })).map(page => page.id)).toEqual(
			lexical.map(page => page.id),
		);
	});

	it("bounds injected context and preserves user images and tool protocol without modifying the transcript", async () => {
		await Bun.write(path.join(root, "database.md"), `## Database\n${"database transactions ".repeat(1000)}`);
		const brain = new MarkdownBrain([{ scope: "project", directory: root }]);
		const messages: AgentMessage[] = [
			{
				role: "user",
				timestamp: 1,
				content: [
					{ type: "text", text: "database" },
					{ type: "image", data: "fixture", mimeType: "image/png" },
				],
			},
			{
				role: "toolResult",
				timestamp: 2,
				toolCallId: "call-1",
				toolName: "read",
				isError: false,
				content: [{ type: "text", text: "result" }],
			},
		];
		const original = structuredClone(messages);
		const transformed = await brain.transform(messages, { maxChars: 1200 });
		expect(messages).toEqual(original);
		expect(transformed[1]).toBe(messages[1]);
		const user = transformed[0];
		if (user.role !== "user" || !Array.isArray(user.content)) throw new Error("Missing user content");
		expect(user.content[1]).toEqual({ type: "image", data: "fixture", mimeType: "image/png" });
		const context = user.content[2];
		if (context.type !== "text") throw new Error("Missing retrieved context");
		expect(context.text).toContain("Source:");
		expect(context.text.length).toBeLessThanOrEqual(1200);
	});

	it("isolates project roots while sharing user knowledge and discovers local skills", async () => {
		const first = path.join(root, "first");
		const second = path.join(root, "second");
		const user = path.join(root, "agent");
		await Bun.write(path.join(first, ".harvest", "brain", "context.md"), "## Build\nAurora compilation uses Bun.");
		await Bun.write(
			path.join(first, ".agents", "skills", "deploy", "SKILL.md"),
			"---\nname: deployment\n---\n## Build\nAurora compilation requires a staging check.",
		);
		await Bun.write(path.join(user, "brain", "preferences.md"), "## Tools\nAurora prefers concise progress updates.");
		const firstPages = await createMarkdownBrain(first, user).retrieve("Aurora");
		expect(firstPages.filter(page => page.scope === "project")).toHaveLength(2);
		const withoutSkills = await createMarkdownBrain(first, user, false).retrieve("Aurora");
		expect(withoutSkills.filter(page => page.scope === "project")).toHaveLength(1);
		expect(withoutSkills.some(page => page.filePath.endsWith("SKILL.md"))).toBe(false);
		const secondPages = await createMarkdownBrain(second, user).retrieve("Aurora");
		expect(secondPages.map(page => page.scope)).toEqual(["user"]);
	});

	it("cancellation bypasses sidecar work and secret sanitization applies to scoring payloads", async () => {
		await Bun.write(path.join(root, "a.md"), "## Database\nDatabase secret-value alpha.");
		await Bun.write(path.join(root, "b.md"), "## Database\nDatabase secret-value beta.");
		const brain = new MarkdownBrain([{ scope: "user", directory: root }]);
		const client = new LayaClient();
		const decide = vi.spyOn(client, "decide").mockResolvedValue({ success: false, fallback: true, latencyMs: 0 });
		expect(await brain.retrieve("database", { rerank: true, client, signal: AbortSignal.abort() })).toEqual([]);
		expect(decide).not.toHaveBeenCalled();
		await brain.retrieve("database secret-value", {
			rerank: true,
			client,
			sanitize: text => text.replaceAll("secret-value", "redacted"),
		});
		expect(JSON.stringify(decide.mock.calls[0][0])).not.toContain("secret-value");
		expect(JSON.stringify(decide.mock.calls[0][0])).toContain("redacted");
	});
});
