export interface BoxSymbols {
	topLeft: string;
	topRight: string;
	bottomLeft: string;
	bottomRight: string;
	horizontal: string;
	vertical: string;
	teeDown: string;
	teeUp: string;
	teeLeft: string;
	teeRight: string;
	cross: string;
}

export interface SymbolTheme {
	cursor: string;
	inputCursor: string;
	boxRound: Omit<BoxSymbols, "teeDown" | "teeUp" | "teeLeft" | "teeRight" | "cross">;
	boxSharp: BoxSymbols;
	table: BoxSymbols;
	quoteBorder: string;
	hrChar: string;
	/** Chip glyph drawn (painted with the referenced color) before inline hex colors. */
	colorSwatch?: string;
	spinnerFrames: string[];
	/** Optional application chrome; omitted values retain legacy symbol defaults. */
	scrollbar?: { track: string; thumb: string };
	composer?: { rail: string; leftCap: string; rightCap: string; promptGutter: string; bandGutter: string };
}

/** Backward-compatible built-in composer chrome for SDK themes without explicit shape symbols. */
export function resolveComposerSymbols(
	symbols?: SymbolTheme,
	box?: SymbolTheme["boxRound"],
): NonNullable<SymbolTheme["composer"]> {
	if (symbols?.composer) return symbols.composer;
	const activeBox = box ?? symbols?.boxRound;
	const ascii = activeBox?.vertical === "|";
	return {
		rail: ascii ? "|" : "▎",
		leftCap: ascii ? "|" : "▐",
		rightCap: ascii ? "|" : "▌",
		promptGutter: `${symbols?.cursor ?? (ascii ? ">" : "❯")} `,
		bandGutter: `${activeBox?.bottomLeft ?? "╰"}${activeBox?.horizontal ?? "─"} `,
	};
}
