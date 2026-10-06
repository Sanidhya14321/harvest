/**
 * Registry coverage: every TUI builtin slash command is discoverable through
 * the palette assembly with its argument/subcommand intent intact.
 *
 * Contracts:
 * - No builtin command silently drops from the palette merge (first source
 *   wins on duplicates; builtins lead).
 * - Commands that take arguments/subcommands prepare `/name ` drafts instead
 *   of executing incomplete input; runnable commands keep `run` intent.
 * - Argument hints survive the merge so the palette can advertise them.
 * - Palette filtering still finds every merged command by name.
 *
 * Real registries and components only. Failure modes are named in each case.
 */
import { beforeAll, describe, expect, it } from "bun:test";
import {
	CommandPaletteComponent,
	mergePaletteItems,
	type PaletteCommandSource,
} from "@harvest/pi-coding-agent/modes/components/command-palette";
import { BUILTIN_SLASH_COMMANDS } from "@harvest/pi-coding-agent/slash-commands/builtin-registry";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";

beforeAll(async () => {
	await initTheme();
});

function builtinSources(): PaletteCommandSource[] {
	return BUILTIN_SLASH_COMMANDS.map(cmd => ({
		name: cmd.name,
		description: cmd.description,
		group: "builtin",
		allowArgs: cmd.allowArgs,
		hasSubcommands: (cmd.subcommands?.length ?? 0) > 0,
		inlineHint: cmd.inlineHint,
	}));
}

describe("command registry coverage", () => {
	it("enumerates a non-empty live command registry (stale-count detector)", () => {
		expect(BUILTIN_SLASH_COMMANDS.length).toBeGreaterThan(0);
	});

	it("exposes every builtin command to the palette merge (silent-drop)", () => {
		const items = mergePaletteItems(builtinSources(), []);
		const ids = new Set(items.map(item => item.id));
		const missing = BUILTIN_SLASH_COMMANDS.map(cmd => `/${cmd.name}`).filter(id => !ids.has(id));
		expect(missing).toEqual([]);
	});

	it("marks argument-taking commands as drafts, runnable ones as run (incomplete-execution)", () => {
		const items = mergePaletteItems(builtinSources(), []);
		const byId = new Map(items.map(item => [item.id, item]));
		for (const cmd of BUILTIN_SLASH_COMMANDS) {
			const item = byId.get(`/${cmd.name}`);
			if (!item) continue;
			const takesArgs = cmd.allowArgs === true || (cmd.subcommands?.length ?? 0) > 0;
			if (takesArgs) {
				expect(item.intent).toBe("draft");
			} else {
				expect(item.intent ?? "run").toBe("run");
			}
		}
	});

	it("preserves inline argument hints through the merge (hint-loss)", () => {
		const withHints = BUILTIN_SLASH_COMMANDS.filter(cmd => cmd.inlineHint);
		expect(withHints.length).toBeGreaterThan(0);
		const items = mergePaletteItems(builtinSources(), []);
		const byId = new Map(items.map(item => [item.id, item]));
		for (const cmd of withHints) {
			expect(byId.get(`/${cmd.name}`)?.argHint).toBe(cmd.inlineHint);
		}
	});

	it("lets the first source win on duplicate names (extension-shadow leak)", () => {
		const items = mergePaletteItems(builtinSources(), [
			{ name: BUILTIN_SLASH_COMMANDS[0]!.name, description: "shadow", group: "extension" },
		]);
		const first = items.find(item => item.id === `/${BUILTIN_SLASH_COMMANDS[0]!.name}`);
		expect(first?.group).toBe("builtin");
	});

	it("finds every merged command through palette filtering (undiscoverable command)", () => {
		const palette = new CommandPaletteComponent();
		const extra: PaletteCommandSource[] = [
			{ name: "ext:deploy", description: "extension command", group: "extension", allowArgs: true },
			{ name: "notes.md", description: "file command", group: "file" },
		];
		palette.setItems(mergePaletteItems(builtinSources(), extra));
		for (const cmd of BUILTIN_SLASH_COMMANDS) {
			palette.setQuery(`/${cmd.name}`);
			const found = palette.filtered().some(item => item.id === `/${cmd.name}`);
			expect(found).toBe(true);
		}
		palette.setQuery("/ext:dep");
		expect(palette.filtered().some(item => item.id === "/ext:deploy")).toBe(true);
	});
});
