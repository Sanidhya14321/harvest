#!/usr/bin/env bun
/**
 * Changelog document parsing and rendering shared by `rewrite-changelog.ts`.
 *
 * The changelog fixer itself (promotion, duplicate pruning, baseline pinning,
 * recovery and size collapse) moved to `cargo xtask fix-changelogs`; this module
 * keeps only the parser/renderer until `rewrite-changelog.ts` migrates too.
 */

import * as path from "node:path";
import { $, Glob } from "bun";

const CHANGELOG_GLOB = "packages/*/CHANGELOG.md";

export interface NumberedLine {
	text: string;
	lineNumber: number;
}

export interface Subsection {
	title: string;
	lines: NumberedLine[];
}

export interface ReleaseSection {
	heading: string;
	title: string;
	leadingLines: NumberedLine[];
	subsections: Subsection[];
}

export interface ChangelogDocument {
	prefixLines: NumberedLine[];
	sections: ReleaseSection[];
}

export interface ParsedItem {
	startLine: number;
	endLine: number;
	lines: string[];
}

function isReleaseHeading(line: string): boolean {
	return /^## \[[^\]]+\]/.test(line);
}

function isSubsectionHeading(line: string): boolean {
	return /^###\s+\S/.test(line);
}

function parseReleaseTitle(heading: string): string {
	const match = heading.match(/^## \[([^\]]+)\]/);
	return match?.[1] ?? heading.replace(/^##\s+/, "").trim();
}

function parseSubsectionTitle(heading: string): string {
	return heading.replace(/^###\s+/, "").trim();
}

function isListItemLine(line: string): boolean {
	return line.trimStart().startsWith("- ");
}

function splitContentLines(content: string): string[] {
	const normalized = content.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
	if (normalized.endsWith("\n")) {
		return normalized.slice(0, -1).split("\n");
	}
	return normalized.split("\n");
}

function createNumberedLine(text: string, lineNumber: number): NumberedLine {
	return { text, lineNumber };
}

export function parseChangelog(content: string): ChangelogDocument {
	const lines = splitContentLines(content);
	const numberedLines = lines.map((text, index) => createNumberedLine(text, index + 1));
	const prefixLines: NumberedLine[] = [];
	const sections: ReleaseSection[] = [];
	let index = 0;

	while (index < numberedLines.length && !isReleaseHeading(numberedLines[index]?.text ?? "")) {
		const line = numberedLines[index];
		if (line) prefixLines.push(line);
		index++;
	}

	while (index < numberedLines.length) {
		const headingLine = numberedLines[index];
		if (!headingLine) break;
		index++;

		const bodyLines: NumberedLine[] = [];
		while (index < numberedLines.length && !isReleaseHeading(numberedLines[index]?.text ?? "")) {
			const line = numberedLines[index];
			if (line) bodyLines.push(line);
			index++;
		}

		sections.push(parseReleaseSection(headingLine.text, bodyLines));
	}

	return { prefixLines, sections };
}

function parseReleaseSection(heading: string, bodyLines: readonly NumberedLine[]): ReleaseSection {
	const leadingLines: NumberedLine[] = [];
	const subsections: Subsection[] = [];
	let index = 0;

	while (index < bodyLines.length && !isSubsectionHeading(bodyLines[index]?.text ?? "")) {
		const line = bodyLines[index];
		if (line) leadingLines.push(line);
		index++;
	}

	while (index < bodyLines.length) {
		const headingLine = bodyLines[index];
		if (!headingLine) break;
		index++;

		const lines: NumberedLine[] = [];
		while (index < bodyLines.length && !isSubsectionHeading(bodyLines[index]?.text ?? "")) {
			const line = bodyLines[index];
			if (line) lines.push(line);
			index++;
		}

		subsections.push({ title: parseSubsectionTitle(headingLine.text), lines });
	}

	return {
		heading,
		title: parseReleaseTitle(heading),
		leadingLines,
		subsections,
	};
}

function trimBlankLines(lines: readonly string[]): string[] {
	let start = 0;
	let end = lines.length;
	while (start < end && lines[start]?.trim() === "") start++;
	while (end > start && lines[end - 1]?.trim() === "") end--;
	return lines.slice(start, end);
}

function numberedText(lines: readonly NumberedLine[]): string[] {
	return lines.map(line => line.text);
}

export function parseItems(lines: readonly NumberedLine[]): ParsedItem[] {
	const items: ParsedItem[] = [];
	let index = 0;

	while (index < lines.length) {
		const line = lines[index];
		if (!line || !isListItemLine(line.text)) {
			index++;
			continue;
		}

		const start = index;
		index++;
		while (index < lines.length && !isListItemLine(lines[index]?.text ?? "")) {
			index++;
		}

		const itemLines = lines.slice(start, index);
		const firstLine = itemLines[0];
		const lastLine = itemLines[itemLines.length - 1];
		if (firstLine && lastLine) {
			items.push({
				startLine: firstLine.lineNumber,
				endLine: lastLine.lineNumber,
				lines: trimBlankLines(numberedText(itemLines)),
			});
		}
	}

	return items;
}

export function renderChangelog(document: ChangelogDocument): string {
	const output: string[] = [];
	const prefix = trimBlankLines(numberedText(document.prefixLines));
	if (prefix.length > 0) {
		output.push(...prefix, "");
	}

	for (const section of document.sections) {
		output.push(section.heading);
		const leading = trimBlankLines(numberedText(section.leadingLines));
		if (leading.length > 0) {
			output.push("", ...leading);
		}

		for (const subsection of section.subsections) {
			const lines = trimBlankLines(numberedText(subsection.lines));
			if (lines.length === 0) continue;
			output.push("", `### ${subsection.title}`, "", ...lines);
		}

		output.push("");
	}

	while (output.length > 0 && output[output.length - 1] === "") {
		output.pop();
	}
	return `${output.join("\n")}\n`;
}

async function git(args: readonly string[], cwd: string): Promise<string> {
	const result = await $`git -c core.fsmonitor=false -c core.untrackedCache=false -c fetch.pruneTags=false ${args}`
		.cwd(cwd)
		.quiet();
	return result.text();
}

export async function resolveRepoRoot(repoRoot: string | undefined): Promise<string> {
	if (repoRoot) return path.resolve(repoRoot);
	return (await git(["rev-parse", "--show-toplevel"], process.cwd())).trim();
}

export async function changelogPaths(repoRoot: string): Promise<string[]> {
	const glob = new Glob(CHANGELOG_GLOB);
	const paths: string[] = [];
	for await (const changelogPath of glob.scan(repoRoot)) {
		paths.push(path.isAbsolute(changelogPath) ? path.relative(repoRoot, changelogPath) : changelogPath);
	}
	paths.sort();
	return paths;
}
