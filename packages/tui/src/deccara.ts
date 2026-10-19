/** Shared Rust rectangular-background planner; this module applies UTF-16 cutoffs. */
import { analyzeDeccaraLine, encodeDeccaraRectangle, planDeccaraCuts } from "@harvest/pi-natives";

/** DECSACE protocol delimiters, retained as declarative host constants. */
export const DECSACE_RECT = "\x1b[2*x";
export const DECSACE_DEFAULT = "\x1b[*x";
const SEGMENT_RESET = "\x1b[0m";

export interface BgFillAnalysis {
	/** UTF-16 index immediately after the final content glyph. */
	cut: number;
	/** Zero-based terminal column where trailing padding begins. */
	leftCol: number;
	bg: string;
}

export interface DeccaraPlan {
	texts: string[];
	sequence: string;
}

/** Encode a 1-based inclusive rectangle with validated SGR parameters. */
export function encodeDeccara(top: number, left: number, bottom: number, right: number, sgr: string): string {
	if (typeof encodeDeccaraRectangle !== "function") {
		throw new Error("Native terminal geometry is unavailable; rebuild @harvest/pi-natives.");
	}
	return encodeDeccaraRectangle(top, left, bottom, right, sgr);
}

/** Unsupported rows and unavailable optional native optimizations remain untouched. */
export function analyzeBgFillLine(line: string, width: number): BgFillAnalysis | null {
	if (typeof analyzeDeccaraLine !== "function") return null;
	try {
		return analyzeDeccaraLine(line, width) ?? null;
	} catch {
		return null;
	}
}

/** Plan against an immutable row snapshot; apply cutoffs without UTF-8 conversion. */
export function planDeccaraFills(lines: string[], width: number, firstScreenRow = 0): DeccaraPlan {
	const original = [...lines];
	if (typeof planDeccaraCuts !== "function") return { texts: original, sequence: "" };
	try {
		const plan = planDeccaraCuts(original, width, firstScreenRow);
		return {
			texts: original.map((line, index) => {
				const cut = plan.cuts[index];
				if (cut === undefined || cut < 0) return line;
				return cut === 0 ? "" : line.slice(0, cut) + SEGMENT_RESET;
			}),
			sequence: plan.sequence,
		};
	} catch {
		// This is a byte-saving optimization: failure must never drop renderer rows.
		return { texts: original, sequence: "" };
	}
}
