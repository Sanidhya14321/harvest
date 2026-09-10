/**
 * Unified Diff & Change Formatter for Edit Mutations.
 */

export interface DiffLine {
	readonly type: "add" | "del" | "ctx";
	readonly text: string;
	readonly oldLine?: number;
	readonly newLine?: number;
}

export interface DiffHunk {
	readonly oldStart: number;
	readonly oldCount: number;
	readonly newStart: number;
	readonly newCount: number;
	readonly lines: DiffLine[];
}

/**
 * Generate a clean unified diff string comparing original content to modified content.
 */
export function generateUnifiedDiff(filePath: string, original: string, modified: string): string {
	const origLines = original.split("\n");
	const modLines = modified.split("\n");

	const diffLines: string[] = [
		`--- a/${filePath}`,
		`+++ b/${filePath}`,
	];

	// Find common prefix
	let prefix = 0;
	while (prefix < origLines.length && prefix < modLines.length && origLines[prefix] === modLines[prefix]) {
		prefix++;
	}

	// Find common suffix
	let origSuffix = origLines.length - 1;
	let modSuffix = modLines.length - 1;
	while (origSuffix >= prefix && modSuffix >= prefix && origLines[origSuffix] === modLines[modSuffix]) {
		origSuffix--;
		modSuffix--;
	}

	const contextBefore = Math.max(0, prefix - 3);
	const oldStart = contextBefore + 1;
	const newStart = contextBefore + 1;

	const oldEnd = Math.min(origLines.length, origSuffix + 4);
	const newEnd = Math.min(modLines.length, modSuffix + 4);

	const oldCount = oldEnd - contextBefore;
	const newCount = newEnd - contextBefore;

	diffLines.push(`@@ -${oldStart},${oldCount} +${newStart},${newCount} @@`);

	// Context before
	for (let i = contextBefore; i < prefix; i++) {
		diffLines.push(` ${origLines[i]}`);
	}

	// Deletions
	for (let i = prefix; i <= origSuffix; i++) {
		diffLines.push(`-${origLines[i]}`);
	}

	// Additions
	for (let i = prefix; i <= modSuffix; i++) {
		diffLines.push(`+${modLines[i]}`);
	}

	// Context after
	for (let i = origSuffix + 1; i < oldEnd; i++) {
		diffLines.push(` ${origLines[i]}`);
	}

	return diffLines.join("\n");
}
