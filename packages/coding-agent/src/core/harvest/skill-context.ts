/**
 * Skill Context Loader.
 *
 * Loads approved skills from .harvest/skills/ and formats them for context injection.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { ContradictionEngine } from "./memory";
import { KnowledgeRetriever, splitMarkdownSections, type SearchResult, type SectionPage } from "./retrieval";

export interface LoadedSkill {
	readonly name: string;
	readonly description: string;
	readonly filePath: string;
	readonly content: string;
}

export interface FormatSkillsOptions {
	readonly query?: string;
	readonly maxSkills?: number;
	readonly maxTotalChars?: number;
}

const STOPWORDS = new Set([
	"about", "all", "also", "and", "any", "are", "been", "can", "could", "does",
	"for", "from", "has", "have", "how", "into", "its", "just", "like", "make",
	"more", "most", "not", "only", "other", "our", "out", "some", "such", "than",
	"that", "the", "their", "them", "then", "there", "these", "they", "this", "use",
	"was", "were", "what", "when", "where", "which", "who", "will", "with", "you", "your",
]);

/** Tokenize text for keyword relevance matching, omitting noise stop words */
function extractKeywords(text: string): Set<string> {
	return new Set(
		text
			.toLowerCase()
			.replace(/[^a-z0-9_\-\s]/g, " ")
			.split(/\s+/)
			.filter((t) => t.length >= 3 && !STOPWORDS.has(t)),
	);
}

/**
 * Extract tags from YAML frontmatter supporting inline, bracketed, or multiline bullet lists.
 */
export function parseFrontmatterTags(fm: string): string[] {
	const tags: string[] = [];
	const inlineMatch = fm.match(/^tags:\s*(.+)$/im);
	if (inlineMatch) {
		const raw = inlineMatch[1].trim();
		if (raw.startsWith("[") && raw.endsWith("]")) {
			const items = raw
				.slice(1, -1)
				.split(",")
				.map((s) => s.trim().replace(/^['"]|['"]$/g, "").toLowerCase());
			tags.push(...items.filter(Boolean));
		} else if (!raw.startsWith("-")) {
			const items = raw
				.split(",")
				.map((s) => s.trim().replace(/^['"]|['"]$/g, "").toLowerCase());
			tags.push(...items.filter(Boolean));
		}
	}

	const listMatch = fm.match(/^tags:\s*\n((?:[ \t]*-[ \t]*.+\n?)+)/im);
	if (listMatch) {
		const lines = listMatch[1].split("\n");
		for (const line of lines) {
			const m = line.match(/^[ \t]*-[ \t]*['"]?([^'"]+?)['"]?[ \t]*$/);
			if (m) {
				tags.push(m[1].trim().toLowerCase());
			}
		}
	}

	return Array.from(new Set(tags));
}

export class SkillContextManager {
	readonly #workspaceRoot: string;
	readonly #skillsDir: string;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#workspaceRoot = path.resolve(workspaceRoot);
		this.#skillsDir = path.join(this.#workspaceRoot, ".harvest", "skills");
	}

	get skillsDir(): string {
		return this.#skillsDir;
	}

	loadSkills(): LoadedSkill[] {
		const skills: LoadedSkill[] = [];
		const candidateDirs = [
			this.#skillsDir,
			path.join(this.#workspaceRoot, ".omp", "skills"),
		];

		const targetDir = candidateDirs.find((d) => fs.existsSync(d));
		if (!targetDir) return skills;

		try {
			const entries = fs.readdirSync(targetDir, { withFileTypes: true });
			for (const entry of entries) {
				let fullPath: string | undefined;
				let defaultName: string | undefined;

				if (entry.isDirectory()) {
					const nestedSkill = path.join(targetDir, entry.name, "SKILL.md");
					if (fs.existsSync(nestedSkill)) {
						fullPath = nestedSkill;
						defaultName = entry.name;
					}
				} else if (entry.isFile() && entry.name.endsWith(".md") && entry.name !== "README.md") {
					fullPath = path.join(targetDir, entry.name);
					defaultName = path.basename(entry.name, ".md");
				}

				if (!fullPath || !defaultName) continue;

				try {
					const content = fs.readFileSync(fullPath, "utf8");
					let name = defaultName;
					let description = "";

					if (content.startsWith("---")) {
						const end = content.indexOf("\n---", 3);
						if (end !== -1) {
							const fm = content.slice(3, end);
							const nameMatch = fm.match(/^name:\s*["']?(.*?)["']?$/m);
							if (nameMatch && nameMatch[1].trim()) name = nameMatch[1].trim();
							const descMatch = fm.match(/^description:\s*["']?(.*?)["']?$/m);
							if (descMatch && descMatch[1].trim()) description = descMatch[1].trim();
						}
					}

					skills.push({ name, description, filePath: fullPath, content });
				} catch {}
			}
		} catch {}

		return skills;
	}

	/**
	 * Find skills relevant to a query using keyword overlap.
	 * Returns skills matching query keywords, sorted by relevance score.
	 */
	findRelevantSkills(query: string, maxSkills: number = 2): LoadedSkill[] {
		const allSkills = this.loadSkills();
		if (allSkills.length === 0) return [];

		const queryTokens = extractKeywords(query);
		if (queryTokens.size === 0) return [];

		const scored: Array<{ skill: LoadedSkill; score: number }> = [];

		for (const skill of allSkills) {
			const skillTokens = extractKeywords(`${skill.name} ${skill.description}`);
			let matches = 0;
			for (const token of queryTokens) {
				if (skillTokens.has(token)) {
					matches++;
				}
			}

			// Substring match on name gets a relevance boost
			const lowerQuery = query.toLowerCase();
			if (lowerQuery.includes(skill.name.toLowerCase())) {
				matches += 3;
			}

			if (matches > 0) {
				scored.push({ skill, score: matches });
			}
		}

		scored.sort((a, b) => b.score - a.score);
		return scored.slice(0, maxSkills).map((s) => s.skill);
	}

	/**
	 * Format skills for context injection.
	 * When a query is provided, filters to relevant skills only to avoid token bloat.
	 */
	formatSkillsPrompt(
		optionsOrMaxSkills?: FormatSkillsOptions | number,
		legacyMaxTotalChars?: number,
	): string {
		let query: string | undefined;
		let maxSkills = 2;
		let maxTotalChars = 800;

		if (typeof optionsOrMaxSkills === "number") {
			maxSkills = optionsOrMaxSkills;
			if (typeof legacyMaxTotalChars === "number") {
				maxTotalChars = legacyMaxTotalChars;
			}
		} else if (optionsOrMaxSkills && typeof optionsOrMaxSkills === "object") {
			query = optionsOrMaxSkills.query;
			if (optionsOrMaxSkills.maxSkills !== undefined) maxSkills = optionsOrMaxSkills.maxSkills;
			if (optionsOrMaxSkills.maxTotalChars !== undefined) maxTotalChars = optionsOrMaxSkills.maxTotalChars;
		}

		const skills = query ? this.findRelevantSkills(query, maxSkills) : this.loadSkills().slice(0, maxSkills);
		if (skills.length === 0) return "";

		const lines: string[] = ["<harvest-skills>"];
		let totalChars = 0;

		for (const skill of skills) {
			const item = `- ${skill.name}: ${skill.description || "Reusable workflow"}`;
			if (totalChars + item.length > maxTotalChars) break;
			lines.push(item);
			totalChars += item.length;
		}

		lines.push("</harvest-skills>");
		return lines.length > 2 ? lines.join("\n") : "";
	}

	/**
	 * Filter markdown files in docs/ folder in skills folder by tags,
	 * then rank using vectorless RAG with page index.
	 * Optimizes working memory through tag scoping, deduplication, cache bounding,
	 * and contradiction detection.
	 */
	async findRelevantSkillDocs(
		tags: string[],
		query: string,
		limit: number = 3,
		maxTokens: number = 4000,
	): Promise<SearchResult[]> {
		const candidateDirs = [
			this.#skillsDir,
			path.join(this.#workspaceRoot, ".omp", "skills"),
		];
		const targetDir = candidateDirs.find((d) => fs.existsSync(d));
		if (!targetDir) return [];

		const pages: SectionPage[] = [];

		const processDocsDir = (docsDir: string, skillName: string) => {
			if (!fs.existsSync(docsDir)) return;
			const entries = fs.readdirSync(docsDir, { withFileTypes: true });
			for (const entry of entries) {
				if (entry.isFile() && entry.name.endsWith(".md")) {
					const fullPath = path.join(docsDir, entry.name);
					try {
						const content = fs.readFileSync(fullPath, "utf8");

						// Check tags and superseded in frontmatter
						let hasMatch = tags.length === 0;
						let isSuperseded = false;
						if (content.startsWith("---")) {
							const end = content.indexOf("\n---", 3);
							if (end !== -1) {
								const fm = content.slice(3, end);
								if (/^superseded:\s*true/im.test(fm)) {
									isSuperseded = true;
								}

								if (tags.length > 0) {
									const fileTags = parseFrontmatterTags(fm);
									const lowerQueryTags = tags.map((t) => t.toLowerCase());
									if (lowerQueryTags.some((t) => fileTags.includes(t))) {
										hasMatch = true;
									}
								}
							}
						}

						if (hasMatch && !isSuperseded) {
							const sections = splitMarkdownSections(fullPath, content, skillName);
							pages.push(...sections);
						}
					} catch {}
				}
			}
		};

		// Check skills/docs/ (global docs)
		processDocsDir(path.join(targetDir, "docs"), "global");

		// Check skills/*/docs/ (skill-scoped docs)
		try {
			const entries = fs.readdirSync(targetDir, { withFileTypes: true });
			for (const entry of entries) {
				if (entry.isDirectory() && entry.name !== "docs") {
					processDocsDir(path.join(targetDir, entry.name, "docs"), entry.name);
				}
			}
		} catch {}

		// Run contradiction detection to maintain perfect global memory state
		try {
			const engine = new ContradictionEngine(this.#workspaceRoot);
			engine.detectContradictions();
		} catch {}

		const retriever = new KnowledgeRetriever(this.#workspaceRoot);
		return retriever.searchPages(pages, query, limit, maxTokens);
	}
}
