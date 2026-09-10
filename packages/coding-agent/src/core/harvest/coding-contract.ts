/**
 * The Grounded Coding Task Contract.
 *
 * Enforces mechanical invariants on the model turn loop:
 * decomposition into small checkpoints, pre-inspection, minimal change,
 * test verification before finishing, and preference for tool evidence.
 */

export const GROUNDED_CODING_CONTRACT = `<coding-contract>
0. Decompose the request into small checkpoints; finish and verify one coherent checkpoint at a time.
1. Inspect the target files plus their nearest tests, types, configuration, and call sites before editing.
2. Implement the smallest complete change that satisfies the request and preserves established interfaces and conventions.
3. Run the narrowest relevant test, typecheck, lint, or build command. If it fails, read the exact diagnostic, fix the cause, and rerun it.
4. Finish only when every requested behavior is implemented and verification succeeded. If verification is impossible, state the precise blocker.
Prefer evidence from tools over assumptions.
</coding-contract>`;

export function formatCodingContract(): string {
	return GROUNDED_CODING_CONTRACT;
}

export function shouldInjectCodingContract(intent?: string): boolean {
	return intent !== "informational";
}

export function injectCodingContract(systemPrompt: string, intent?: string): string {
	if (!shouldInjectCodingContract(intent)) {
		return systemPrompt;
	}
	if (systemPrompt.includes("<coding-contract>")) {
		return systemPrompt;
	}
	return `${systemPrompt.trimEnd()}\n\n${GROUNDED_CODING_CONTRACT}\n`;
}
