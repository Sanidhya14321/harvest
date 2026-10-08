import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import * as os from "node:os";
import { SkillContextManager } from "../../src/core/harvest/skill-context";

describe("SkillContextManager - findRelevantSkillDocs", () => {
	let tempWorkspace: string;
	let manager: SkillContextManager;
	let skillsDir: string;

	beforeEach(async () => {
		tempWorkspace = await fs.mkdtemp(path.join(os.tmpdir(), "harvest-skill-docs-test-"));
		manager = new SkillContextManager(tempWorkspace);
		skillsDir = path.join(tempWorkspace, ".harvest", "skills");
		await fs.mkdir(skillsDir, { recursive: true });
	});

	afterEach(async () => {
		await fs.rm(tempWorkspace, { recursive: true, force: true });
	});

	it("should find docs in global skills/docs/ matching tags", async () => {
		const docsDir = path.join(skillsDir, "docs");
		await fs.mkdir(docsDir, { recursive: true });

		const docPath = path.join(docsDir, "guide.md");
		await fs.writeFile(
			docPath,
			`---
tags: frontend, react
---
## React Best Practices
Always use hooks for state.`,
		);

		const results = await manager.findRelevantSkillDocs(["react"], "best practices for state");
		expect(results.length).toBeGreaterThan(0);
		expect(results[0].page.heading).toBe("React Best Practices");
		expect(results[0].page.role).toBe("global");
	});

	it("should find docs in skills/*/docs/ matching tags", async () => {
		const skillDocsDir = path.join(skillsDir, "my-skill", "docs");
		await fs.mkdir(skillDocsDir, { recursive: true });

		const docPath = path.join(skillDocsDir, "usage.md");
		await fs.writeFile(
			docPath,
			`---
tags: backend, api
---
## API Usage
Make sure to authenticate requests.`,
		);

		const results = await manager.findRelevantSkillDocs(["backend"], "authenticate api");
		expect(results.length).toBeGreaterThan(0);
		expect(results[0].page.heading).toBe("API Usage");
		expect(results[0].page.role).toBe("my-skill");
	});

	it("should not return docs that do not match the tags", async () => {
		const docsDir = path.join(skillsDir, "docs");
		await fs.mkdir(docsDir, { recursive: true });

		const docPath = path.join(docsDir, "guide.md");
		await fs.writeFile(
			docPath,
			`---
tags: frontend
---
## HTML Guide
Use semantic tags.`,
		);

		const results = await manager.findRelevantSkillDocs(["backend"], "html semantic tags");
		expect(results.length).toBe(0);
	});

	it("should apply Okapi BM25 ranking correctly", async () => {
		const skillDocsDir = path.join(skillsDir, "my-skill", "docs");
		await fs.mkdir(skillDocsDir, { recursive: true });

		const docPath = path.join(skillDocsDir, "usage.md");
		await fs.writeFile(
			docPath,
			`---
tags: general
---
## Section One
This section mentions authentication once.

## Section Two
This section talks about authentication multiple times because authentication is important for authentication.`,
		);

		const results = await manager.findRelevantSkillDocs(["general"], "authentication");
		expect(results.length).toBe(2);
		// Section Two should rank higher because it mentions the keyword more times
		expect(results[0].page.heading).toBe("Section Two");
		expect(results[1].page.heading).toBe("Section One");
	});

	it("should skip documents marked as superseded", async () => {
		const docsDir = path.join(skillsDir, "docs");
		await fs.mkdir(docsDir, { recursive: true });

		const docPath = path.join(docsDir, "old-guide.md");
		await fs.writeFile(
			docPath,
			`---
tags: frontend
superseded: true
---
## Old React Best Practices
Use class components.`,
		);

		const results = await manager.findRelevantSkillDocs(["frontend"], "react components");
		expect(results.length).toBe(0);
	});

	it("should deduplicate sections with identical content", async () => {
		const skillDocsDir = path.join(skillsDir, "my-skill", "docs");
		await fs.mkdir(skillDocsDir, { recursive: true });

		const docPath1 = path.join(skillDocsDir, "file1.md");
		await fs.writeFile(
			docPath1,
			`---
tags: test
---
## Same Section
This is some identical text.`,
		);
		const docPath2 = path.join(skillDocsDir, "file2.md");
		await fs.writeFile(
			docPath2,
			`---
tags: test
---
## Same Section
This is some identical text.`,
		);

		const results = await manager.findRelevantSkillDocs(["test"], "identical text");
		// Should only return 1 result since the content is identical
		expect(results.length).toBe(1);
	});

	it("should bound the cache using maxTokens", async () => {
		const skillDocsDir = path.join(skillsDir, "my-skill", "docs");
		await fs.mkdir(skillDocsDir, { recursive: true });

		const docPath = path.join(skillDocsDir, "long-doc.md");
		await fs.writeFile(
			docPath,
			`---
tags: cache
---
## First
token token token token token token token token token token

## Second
token token token token token token token token token token

## Third
token token token token token token token token token token`,
		);

		// With maxTokens = 15, we can only fit one section (each has 10 valid tokens > 2 chars)
		const results = await manager.findRelevantSkillDocs(["cache"], "token", 10, 15);
		expect(results.length).toBe(1);

		// With maxTokens = 25, we can fit two sections
		const results2 = await manager.findRelevantSkillDocs(["cache"], "token", 10, 25);
		expect(results2.length).toBe(2);
	});

	it("should support bracketed and multiline bullet tag formats", async () => {
		const skillDocsDir = path.join(skillsDir, "formats-skill", "docs");
		await fs.mkdir(skillDocsDir, { recursive: true });

		const docPath1 = path.join(skillDocsDir, "bracket.md");
		await fs.writeFile(
			docPath1,
			`---
tags: [database, postgres]
---
## Database Connection
Pooling postgres connections.`,
		);

		const docPath2 = path.join(skillDocsDir, "multiline.md");
		await fs.writeFile(
			docPath2,
			`---
tags:
  - telemetry
  - tracing
---
## Telemetry Setup
OpenTelemetry tracer initialization.`,
		);

		const res1 = await manager.findRelevantSkillDocs(["postgres"], "pooling connections");
		expect(res1.length).toBe(1);
		expect(res1[0].page.heading).toBe("Database Connection");

		const res2 = await manager.findRelevantSkillDocs(["tracing"], "opentelemetry");
		expect(res2.length).toBe(1);
		expect(res2[0].page.heading).toBe("Telemetry Setup");
	});
});
