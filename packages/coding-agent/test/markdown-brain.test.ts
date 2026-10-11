import { afterEach, beforeEach, describe, expect, it, vi } from "bun:test";
import type { AgentMessage } from "@harvest/pi-agent-core";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { createMarkdownBrain, MarkdownBrain } from "../src/core/harvest/brain";

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

	it("keeps deterministic lexical/graph ordering with no model round trip", async () => {
		await Bun.write(path.join(root, "a.md"), "## Database\nDatabase database database transactions.");
		await Bun.write(path.join(root, "b.md"), "## Database\nDatabase backups restore recovery.");
		const brain = new MarkdownBrain([{ scope: "project", directory: root }]);
		const lexical = await brain.retrieve("database");
		expect(lexical.length).toBeGreaterThan(0);
		// The deprecated rerank flag is accepted but ignored: identical order.
		expect((await brain.retrieve("database", { rerank: true })).map(page => page.id)).toEqual(
			lexical.map(page => page.id),
		);
	});

	it("excludes out-of-scope knowledge when scopes are restricted", async () => {
		await Bun.write(
			path.join(root, "project", "deploy.md"),
			"## Deployment\nDeploy releases through the staging cluster.",
		);
		await Bun.write(path.join(root, "user", "deploy.md"), "## Deployment\nDeploy personal apps using containers.");
		const brain = new MarkdownBrain([
			{ scope: "project", directory: path.join(root, "project") },
			{ scope: "user", directory: path.join(root, "user") },
		]);
		expect((await brain.retrieve("deploy")).map(page => page.scope).sort()).toEqual(["project", "user"]);
		const projectOnly = await brain.retrieve("deploy", { scopes: ["project"] });
		expect(projectOnly.length).toBeGreaterThan(0);
		expect(projectOnly.every(page => page.scope === "project")).toBe(true);
	});

	it("ignores the removed rerank latency budget and keeps lexical order", async () => {
		await Bun.write(path.join(root, "a.md"), "## Database\nDatabase database database transactions.");
		await Bun.write(path.join(root, "b.md"), "## Database\nDatabase backups restore recovery.");
		const brain = new MarkdownBrain([{ scope: "project", directory: root }]);
		const lexical = await brain.retrieve("database");
		expect((await brain.retrieve("database", { rerank: true, rerankTimeoutMs: 137 })).map(page => page.id)).toEqual(
			lexical.map(page => page.id),
		);
	});

	it("Rust retrieval filters scope before ranking and throughout graph expansion", async () => {
		for (let index = 0; index < 8; index++) {
			await Bun.write(path.join(root, "project", `crowd-${index}.md`), "## Deploy\nDeploy deploy deploy.");
		}
		await Bun.write(
			path.join(root, "user", "lesson.md"),
			"---\nid: user-lesson\nrelates_to: [project:private]\n---\n## Deploy\nDeploy after confirming the release.",
		);
		await Bun.write(path.join(root, "project", "private.md"), "---\nid: private\n---\nPrivate project instruction.");
		const brain = new MarkdownBrain([
			{ scope: "project", directory: path.join(root, "project") },
			{ scope: "user", directory: path.join(root, "user") },
		]);
		const pages = await brain.retrieve("deploy", { scopes: ["user"] });
		expect(pages.map(page => page.documentId)).toEqual(["user:user-lesson"]);
		expect(pages[0].content).toContain("confirming the release");
		expect(await brain.retrieve("deploy", { scopes: [] })).toEqual([]);
	});

	it("preserves filename graph links and stable section identities across queries", async () => {
		await Bun.write(
			path.join(root, "deploy.md"),
			"---\nrelates_to: notes.md\n---\n## Deploy\nDeploy releases.\n## Review\nReview releases.",
		);
		await Bun.write(path.join(root, "notes.md"), "## Checklist\nConfirm rollback instructions.");
		const brain = new MarkdownBrain([{ scope: "project", directory: root }]);
		const deploy = await brain.retrieve("deploy");
		expect(deploy.map(page => page.documentId)).toContain("project:notes.md");
		const review = await brain.retrieve("review");
		const reviewPage = review.find(page => page.heading === "Review");
		const mixed = await brain.retrieve("deploy review");
		expect(mixed.find(page => page.heading === "Review")?.id).toBe(reviewPage?.id);
		expect(reviewPage?.tokens).toContain("review");
	});

	it("retired generated lessons become ineligible without waiting for the authored-page cache", async () => {
		const id = `learned-${"a".repeat(64)}`;
		const content = `---\nid: ${id}\nownership: generated\n---\n## Deploy\nDeploy after verifying the release.\n`;
		await Bun.write(path.join(root, ".harvest", "brain", "lesson.md"), content);
		const receipt = {
			id,
			fingerprint: "a".repeat(64),
			path: ".harvest/brain/lesson.md",
			file_hash: new Bun.CryptoHasher("sha256").update(content).digest("hex"),
			created_at: 1,
			status: "active",
		};
		const state = {
			version: 1,
			ledger: { version: 1, events: {} },
			records: { [id]: receipt },
			archives: {},
			leases: [],
		};
		const statePath = path.join(root, ".harvest", "brain-state", "state.json");
		await Bun.write(statePath, JSON.stringify(state));
		const brain = createMarkdownBrain(root, path.join(root, "absent-user"));
		expect((await brain.retrieve("deploy"))[0].documentId).toBe(`project:${id}`);
		receipt.status = "archived";
		await Bun.write(statePath, JSON.stringify(state));
		expect(await brain.retrieve("deploy")).toEqual([]);
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

	it("returns no pages for an aborted signal and keeps indexed content retrievable", async () => {
		await Bun.write(path.join(root, "a.md"), "## Database\nDatabase secret-value alpha.");
		await Bun.write(path.join(root, "b.md"), "## Database\nDatabase secret-value beta.");
		const brain = new MarkdownBrain([{ scope: "user", directory: root }]);
		expect(await brain.retrieve("database", { rerank: true, signal: AbortSignal.abort() })).toEqual([]);
		const pages = await brain.retrieve("database secret-value", {
			sanitize: text => text.replaceAll("secret-value", "redacted"),
		});
		expect(pages.length).toBeGreaterThan(0);
	});

	it("keeps the removed rerank-cache hook as a harmless no-op", async () => {
		const brain = new MarkdownBrain([{ scope: "project", directory: root }]);
		brain.clearRerankCache();
		expect(await brain.retrieve("database")).toEqual([]);
	});
});
