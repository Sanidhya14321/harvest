/**
 * 3-Tier Cascading Edit Matcher with Actionable Near-Miss Diagnostics.
 *
 * Tier 1: Exact LF substring match.
 * Tier 2: Whitespace-flexible line structure match.
 * Tier 3: Line-window fuzzy Levenshtein match with strict accept & margin gates.
 */

export const FUZZY_ACCEPT = 0.88;
export const FUZZY_MARGIN = 0.05;

export interface MatchSuccess {
	readonly matched: true;
	readonly start: number;
	readonly end: number;
	readonly tier: 1 | 2 | 3;
	readonly confidence: number;
	readonly matchedText: string;
}

export interface NearMissDiagnostic {
	readonly closestLineNumber: number;
	readonly similarityPercentage: number;
	readonly excerpt: string;
	readonly reason: string;
}

export interface MatchFailure {
	readonly matched: false;
	readonly diagnostics: NearMissDiagnostic;
}

export type MatchResult = MatchSuccess | MatchFailure;

/** Compute Levenshtein distance between two strings */
export function levenshteinDistance(a: string, b: string): number {
	const m = a.length;
	const n = b.length;

	if (m === 0) return n;
	if (n === 0) return m;

	let prev = Array.from<number>({ length: n + 1 });
	let curr = Array.from<number>({ length: n + 1 });

	for (let j = 0; j <= n; j++) {
		prev[j] = j;
	}

	for (let i = 1; i <= m; i++) {
		curr[0] = i;
		const aChar = a.charCodeAt(i - 1);
		for (let j = 1; j <= n; j++) {
			const bChar = b.charCodeAt(j - 1);
			const cost = aChar === bChar ? 0 : 1;
			curr[j] = Math.min(prev[j] + 1, curr[j - 1] + 1, prev[j - 1] + cost);
		}
		const temp = prev;
		prev = curr;
		curr = temp;
	}

	return prev[n];
}

/** Compute normalized similarity [0, 1] */
export function textSimilarity(a: string, b: string): number {
	const maxLen = Math.max(a.length, b.length);
	if (maxLen === 0) return 1.0;
	const dist = levenshteinDistance(a, b);
	return Math.max(0.0, 1.0 - dist / maxLen);
}

/**
 * Find edit match across 3 cascading tiers.
 */
export function findEditMatch(sourceContent: string, searchBlock: string): MatchResult {
	if (!searchBlock) {
		return {
			matched: false,
			diagnostics: {
				closestLineNumber: 1,
				similarityPercentage: 0,
				excerpt: "",
				reason: "Search block is empty.",
			},
		};
	}

	// Canonicalize LF
	const canonicalSource = sourceContent.replace(/\r\n/g, "\n");
	const canonicalSearch = searchBlock.replace(/\r\n/g, "\n");

	// -------------------------------------------------------------
	// Tier 1: Exact LF Match
	// -------------------------------------------------------------
	const exactIndex = canonicalSource.indexOf(canonicalSearch);
	if (exactIndex !== -1) {
		const secondIndex = canonicalSource.indexOf(canonicalSearch, exactIndex + 1);
		if (secondIndex === -1) {
			return {
				matched: true,
				start: exactIndex,
				end: exactIndex + canonicalSearch.length,
				tier: 1,
				confidence: 1.0,
				matchedText: canonicalSource.slice(exactIndex, exactIndex + canonicalSearch.length),
			};
		}
		// Ambiguous exact matches: return failure with explanation
		const lineNumber = canonicalSource.slice(0, exactIndex).split("\n").length;
		return {
			matched: false,
			diagnostics: {
				closestLineNumber: lineNumber,
				similarityPercentage: 100,
				excerpt: canonicalSearch.slice(0, 100),
				reason: `Multiple exact matches found in file (first at line ${lineNumber}). Provide more surrounding context to disambiguate.`,
			},
		};
	}

	// -------------------------------------------------------------
	// Tier 2: Whitespace-Flexible Match
	// -------------------------------------------------------------
	// Build regex by replacing runs of non-newline whitespace with [^\S\r\n]+
	const searchLines = canonicalSearch.split("\n");
	const regexParts = searchLines.map((line) => {
		const trimmed = line.trim();
		if (!trimmed) return "[^\\S\\r\\n]*";
		const tokens = trimmed.split(/[^\S\r\n]+/);
		const escaped = tokens.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
		return "[^\\S\\r\\n]*" + escaped.join("[^\\S\\r\\n]+") + "[^\\S\\r\\n]*";
	});

	const whitespaceRegex = new RegExp(regexParts.join("\\n"), "g");
	const tier2Matches: RegExpExecArray[] = [];
	let match: RegExpExecArray | null;

	while ((match = whitespaceRegex.exec(canonicalSource)) !== null) {
		tier2Matches.push(match);
		if (match.index === whitespaceRegex.lastIndex) {
			whitespaceRegex.lastIndex++;
		}
	}

	if (tier2Matches.length === 1) {
		const m = tier2Matches[0];
		return {
			matched: true,
			start: m.index,
			end: m.index + m[0].length,
			tier: 2,
			confidence: 0.95,
			matchedText: m[0],
		};
	}

	if (tier2Matches.length > 1) {
		const lineNum = canonicalSource.slice(0, tier2Matches[0].index).split("\n").length;
		return {
			matched: false,
			diagnostics: {
				closestLineNumber: lineNum,
				similarityPercentage: 95,
				excerpt: tier2Matches[0][0].slice(0, 100),
				reason: `Ambiguous match: ${tier2Matches.length} matches found under whitespace-flexible matching. Include more surrounding lines.`,
			},
		};
	}

	// -------------------------------------------------------------
	// Tier 3: Line-Window Fuzzy Matching
	// -------------------------------------------------------------
	const sourceLines = canonicalSource.split("\n");
	const targetLineCount = searchLines.length;

	// Calculate line offsets for start/end character index calculation
	const lineStarts: number[] = [0];
	for (let i = 0; i < sourceLines.length; i++) {
		lineStarts.push(lineStarts[i] + sourceLines[i].length + 1);
	}

	interface Candidate {
		startLine: number;
		endLine: number;
		score: number;
		text: string;
		startChar: number;
		endChar: number;
	}

	const candidates: Candidate[] = [];

	// Sliding window over source lines
	for (let i = 0; i <= sourceLines.length - targetLineCount; i++) {
		const windowLines = sourceLines.slice(i, i + targetLineCount);
		const windowText = windowLines.join("\n");

		// Anchoring check: look for at least one anchor line with >= 5 chars having high similarity
		let hasAnchor = false;
		for (let j = 0; j < targetLineCount; j++) {
			const sLine = searchLines[j].trim();
			const wLine = windowLines[j].trim();
			if (sLine.length >= 5 && wLine.length >= 5 && textSimilarity(sLine, wLine) >= 0.8) {
				hasAnchor = true;
				break;
			}
		}

		if (hasAnchor) {
			const sim = textSimilarity(canonicalSearch, windowText);
			const startChar = lineStarts[i];
			const endChar = lineStarts[i + targetLineCount] - 1;
			candidates.push({
				startLine: i + 1,
				endLine: i + targetLineCount,
				score: sim,
				text: windowText,
				startChar,
				endChar: Math.min(endChar, canonicalSource.length),
			});
		}
	}

	candidates.sort((a, b) => b.score - a.score);

	if (candidates.length > 0) {
		const best = candidates[0];
		const secondBestScore = candidates.length > 1 ? candidates[1].score : 0;
		const margin = best.score - secondBestScore;

		if (best.score >= FUZZY_ACCEPT && (candidates.length === 1 || margin >= FUZZY_MARGIN)) {
			return {
				matched: true,
				start: best.startChar,
				end: best.endChar,
				tier: 3,
				confidence: best.score,
				matchedText: best.text,
			};
		}

		// Near-miss diagnostic
		const simPct = Math.round(best.score * 100);
		const reason =
			best.score < FUZZY_ACCEPT
				? `Closest candidate at line ${best.startLine} matched with only ${simPct}% similarity (threshold is ${Math.round(FUZZY_ACCEPT * 100)}%). Check line edits or formatting.`
				: `Ambiguous fuzzy match at line ${best.startLine} (${simPct}%) vs second runner-up (${Math.round(secondBestScore * 100)}%). Margin ${Math.round(margin * 100)}% is below required ${Math.round(FUZZY_MARGIN * 100)}%. Provide additional unique surrounding lines.`;

		return {
			matched: false,
			diagnostics: {
				closestLineNumber: best.startLine,
				similarityPercentage: simPct,
				excerpt: best.text.split("\n").slice(0, 3).join("\n"),
				reason,
			},
		};
	}

	// Fallback nearest line inspection
	let closestLine = 1;
	let maxLineSim = 0;
	const firstSearchLine = searchLines[0].trim();

	for (let i = 0; i < sourceLines.length; i++) {
		const sim = textSimilarity(firstSearchLine, sourceLines[i].trim());
		if (sim > maxLineSim) {
			maxLineSim = sim;
			closestLine = i + 1;
		}
	}

	return {
		matched: false,
		diagnostics: {
			closestLineNumber: closestLine,
			similarityPercentage: Math.round(maxLineSim * 100),
			excerpt: sourceLines.slice(closestLine - 1, closestLine + 2).join("\n"),
			reason: `Could not find candidate match for search block. Closest matching line was line ${closestLine} (${Math.round(maxLineSim * 100)}% similarity to first line).`,
		},
	};
}
