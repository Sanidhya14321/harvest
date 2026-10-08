import { afterEach, beforeEach, describe, expect, test, vi } from "bun:test";
import { stripVTControlCharacters } from "node:util";
import { PluginManager } from "@harvest/pi-coding-agent/extensibility/plugins/manager";
import type { InstalledPlugin } from "@harvest/pi-coding-agent/extensibility/plugins/types";
import { HookSelectorComponent } from "@harvest/pi-coding-agent/modes/components/hook-selector";
import { LoginDialogComponent } from "@harvest/pi-coding-agent/modes/components/login-dialog";
import { LogoutAccountSelectorComponent } from "@harvest/pi-coding-agent/modes/components/logout-account-selector";
import { MCPAddWizard } from "@harvest/pi-coding-agent/modes/components/mcp-add-wizard";
import { PluginDetailComponent, PluginListComponent } from "@harvest/pi-coding-agent/modes/components/plugin-settings";
import { ResetUsageSelectorComponent } from "@harvest/pi-coding-agent/modes/components/reset-usage-selector";
import { createTheme, getBuiltinThemes } from "@harvest/pi-coding-agent/modes/theme/loader";
import { setThemeInstance, theme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { MCPServerConfig } from "@harvest/pi-coding-agent/mcp/types";
import type { ResetUsageAccount } from "@harvest/pi-coding-agent/slash-commands/helpers/reset-usage";
import * as openModule from "@harvest/pi-coding-agent/utils/open";
import { type TUI, visibleWidth } from "@harvest/pi-tui";

const DOWN = "\x1b[B";
const F2 = "\x1bOQ";
const plain = (lines: readonly string[]): string => stripVTControlCharacters(lines.join("\n"));
const click = (line: number): string => `\x1b[<0;3;${line + 1}M`;
let previousTheme = theme;

beforeEach(async () => {
	previousTheme = theme;
	setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "truecolor", symbolPresetOverride: "ascii" }));
});
afterEach(() => {
	vi.restoreAllMocks();
	setThemeInstance(previousTheme);
});

function assertFrame(lines: readonly string[], width: number, height: number): void {
	expect(lines.length).toBeLessThanOrEqual(height);
	for (const line of lines) {
		expect(visibleWidth(line)).toBeLessThanOrEqual(width);
		expect(stripVTControlCharacters(line)).not.toMatch(/[^\x00-\x7f]/);
	}
}

const plugin = (name: string): InstalledPlugin => ({
	name,
	version: "1.0.0",
	path: `/plugins/${name}`,
	enabled: true,
	enabledFeatures: null,
	manifest: { version: "1.0.0", description: "LONGDESCRIPTION".repeat(20), settings: { token: { type: "string" } } },
});

describe("allocated interactive dialogs", () => {
	test("login keeps manual input reachable under a long authorization URL and submits the full pasted callback", async () => {
		vi.spyOn(openModule, "openPath").mockImplementation(() => {});
		const dialog = new LoginDialogComponent({ requestRender() {} } as unknown as TUI, "openai-codex", () => {});
		const url = `https://example.com/auth?state=${"a".repeat(300)}&challenge=COMPLETE`;
		dialog.showAuth(url);
		const pending = dialog.showManualInput("Paste callback URL");
		dialog.setMaxHeight(4);
		const callback = `http://localhost/callback?code=${"x".repeat(150)}FINAL`;
		dialog.pasteText(callback);
		const lines = dialog.render(24);
		assertFrame(lines, 24, 4);
		expect(plain(lines)).toContain("FINAL");
		dialog.handleInput(F2);
		const details = dialog.render(24);
		expect(plain(details)).toContain("https://example.com");
		dialog.setMaxHeight(40);
		expect(plain(dialog.render(80)).replace(/\s/g, "")).toContain(url);
		dialog.handleInput(F2);
		dialog.handleInput("\r");
		expect(await pending).toBe(callback);
	});

	test("hook selection survives one-row resizes and mouse hit rows select only a visible option", () => {
		const selected = vi.fn();
		const values = ["First", "Disabled", "Last" + " full payload".repeat(20)];
		const dialog = new HookSelectorComponent("Question", values, selected, () => {}, { disabledIndices: [1] });
		dialog.handleInput(DOWN);
		for (const height of [1, 2, 3, 4]) {
			dialog.setMaxHeight(height);
			const lines = dialog.render(24);
			assertFrame(lines, 24, height);
			expect(plain(lines)).toContain("Last");
		}
		const lines = dialog.render(24);
		const optionLine = lines.findIndex(line => plain([line]).includes("Last"));
		dialog.handleInput(click(lines.length - 1));
		expect(selected).not.toHaveBeenCalled();
		dialog.handleInput(click(optionLine));
		expect(selected).toHaveBeenCalledWith(values[2]);
	});

	test("hook details can reveal a description omitted by the compact choice view", () => {
		const dialog = new HookSelectorComponent(
			"Question",
			[{ label: "Choice", description: "detail ".repeat(50) + "FINALDETAIL" }],
			() => {},
			() => {},
			{ maxVisible: 3 },
		);
		dialog.setMaxHeight(4);
		expect(plain(dialog.render(24))).not.toContain("FINALDETAIL");
		dialog.handleInput(F2);
		dialog.render(24);
		for (let i = 0; i < 50; i++) dialog.handleInput(DOWN);
		const lines = dialog.render(24);
		assertFrame(lines, 24, 4);
		expect(plain(lines)).toContain("FINALDETAIL");
	});

	test("hook refreshes existing choice rows when the active theme switches to color-free output", () => {
		const dialog = new HookSelectorComponent(
			"Question",
			["First", "Second"],
			() => {},
			() => {},
		);
		dialog.render(60);
		dialog.handleInput(DOWN);
		setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "none", symbolPresetOverride: "ascii" }));
		const lines = dialog.render(60);
		expect(lines.join("\n")).not.toMatch(/\x1b\[[0-9;]*m/);
		expect(plain(lines)).toContain("> Second");
	});

	test("MCP wizard keeps inputs and confirmation visible while preserving the complete command configuration", () => {
		const complete = vi.fn<(name: string, config: MCPServerConfig, scope: string) => void>();
		const wizard = new MCPAddWizard(complete, () => {});
		wizard.setMaxHeight(1);
		wizard.handleInput("server");
		wizard.handleInput("\r");
		expect(plain(wizard.render(24))).toContain("stdio");
		wizard.handleInput("\r");
		const command = "/bin/" + "long-command".repeat(20) + "FINAL";
		wizard.handleInput(command);
		assertFrame(wizard.render(3), 3, 1);
		wizard.handleInput("\r");
		wizard.handleInput("--full-argument=value");
		wizard.handleInput("\r");
		wizard.handleInput(DOWN);
		expect(plain(wizard.render(24))).toContain("Project");
		wizard.handleInput("\r");
		expect(plain(wizard.render(24))).toContain("Yes");
		wizard.handleInput("\r");
		expect(complete).toHaveBeenCalledWith(
			"server",
			{ type: "stdio", command, args: ["--full-argument=value"] },
			"project",
		);
	});

	test("MCP transport click coordinates follow the rendered short-window choice", () => {
		const wizard = new MCPAddWizard(
			() => {},
			() => {},
			undefined,
			undefined,
			undefined,
			"server",
		);
		wizard.setMaxHeight(4);
		wizard.handleInput(DOWN);
		const lines = wizard.render(60);
		const httpLine = lines.findIndex(line => plain([line]).includes("http (HTTP"));
		expect(httpLine).toBeGreaterThanOrEqual(0);
		wizard.handleInput(click(httpLine));
		wizard.setMaxHeight(20);
		expect(plain(wizard.render(60))).toContain("Server URL");
	});

	test("reset-account clicks select without spending and confirmation still requires two Enter presses", () => {
		const spend = vi.fn();
		const accounts: ResetUsageAccount[] = [
			{ label: "alice", availableCount: 1, active: true, target: { credentialId: 7 } },
		];
		const dialog = new ResetUsageSelectorComponent(accounts, spend, () => {});
		dialog.setMaxHeight(1);
		assertFrame(dialog.render(24), 24, 1);
		dialog.handleInput(click(0));
		expect(spend).not.toHaveBeenCalled();
		dialog.handleInput("\r");
		expect(plain(dialog.render(24))).toContain("Enter again");
		dialog.setMaxHeight(4);
		expect(plain(dialog.render(60))).toContain("Enter again");
		dialog.handleInput("\r");
		expect(spend).toHaveBeenCalledWith(accounts[0]);
	});

	test("logout pointer coordinates ignore footer rows and return the full selected credential", () => {
		const select = vi.fn();
		const account = {
			credentialId: 7,
			provider: "anthropic",
			label: "alice",
			detail: "long detail".repeat(20),
			type: "oauth" as const,
			active: true,
		};
		const dialog = new LogoutAccountSelectorComponent("Anthropic", [account], select, () => {});
		dialog.setMaxHeight(4);
		const lines = dialog.render(24);
		assertFrame(lines, 24, 4);
		dialog.handleInput(click(lines.length - 1));
		expect(select).not.toHaveBeenCalled();
		const index = lines.findIndex(line => plain([line]).includes("alice"));
		dialog.handleInput(click(index));
		expect(select).toHaveBeenCalledWith(account);
	});

	test("plugin list accepts navigation before its first paint and preserves the selected plugin through a one-row resize", () => {
		const select = vi.fn();
		const first = plugin("first"),
			second = plugin("second");
		const dialog = new PluginListComponent(
			[
				{ kind: "npm", plugin: first },
				{ kind: "npm", plugin: second },
			],
			{ onNpmSelect: select, onMarketplaceSelect: () => {}, onCancel: () => {} },
		);
		dialog.handleInput(DOWN);
		dialog.setMaxHeight(1);
		const lines = dialog.render(24);
		assertFrame(lines, 24, 1);
		expect(plain(lines)).toContain("second");
		dialog.handleInput("\r");
		expect(select).toHaveBeenCalledWith(second);
	});

	test("plugin nested input uses the same allocation and submits the complete setting value", async () => {
		const manager = new PluginManager(process.cwd());
		vi.spyOn(manager, "getPluginSettings").mockResolvedValue({});
		const update = vi.fn();
		const dialog = new PluginDetailComponent(plugin("plugin"), manager, {
			onEnabledChange() {},
			onFeatureChange() {},
			onConfigChange: update,
			onBack() {},
		});
		await Promise.resolve();
		await Promise.resolve();
		dialog.handleInput(DOWN);
		dialog.handleInput("\r");
		dialog.setMaxHeight(1);
		assertFrame(dialog.render(3), 3, 1);
		const value = "TOKEN".repeat(30);
		dialog.handleInput(value);
		dialog.handleInput("\r");
		expect(update).toHaveBeenCalledWith("token", value);
	});
});
