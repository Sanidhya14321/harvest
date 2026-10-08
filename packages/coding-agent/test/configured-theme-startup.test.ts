import * as path from "node:path";
import { describe, expect, it } from "bun:test";
import { TempDir } from "@harvest/pi-utils";

interface ThemeProbe {
	name: string;
	prepaint?: string;
	surface: string;
	before?: string;
	after?: string;
}

const settingsModule = new URL("../src/config/settings.ts", import.meta.url).href;
const configModule = new URL("../src/config/theme.ts", import.meta.url).href;
const themeModule = new URL("../src/modes/theme/theme.ts", import.meta.url).href;
const dirsModule = new URL("../../utils/src/dirs.ts", import.meta.url).href;

async function probe(code: string, appearance: "dark" | "light"): Promise<ThemeProbe> {
	const proc = Bun.spawn([process.execPath, "--eval", code], {
		cwd: path.resolve(import.meta.dir, "../../.."),
		env: { ...process.env, FORCE_COLOR: "3", COLORFGBG: appearance === "light" ? "0;15" : "15;0" },
		stdout: "pipe",
		stderr: "pipe",
	});
	const [exit, output, errors] = await Promise.all([
		proc.exited,
		new Response(proc.stdout).text(),
		new Response(proc.stderr).text(),
	]);
	expect(exit, errors).toBe(0);
	return JSON.parse(output) as ThemeProbe;
}

describe("configured terminal initialization", () => {
	it("keeps fresh prepaint and resolved startup surfaces consistent in each appearance slot", async () => {
		const code = `
			import { Settings } from ${JSON.stringify(settingsModule)};
			import { initConfiguredTheme } from ${JSON.stringify(configModule)};
			import { initThemeSync, getCurrentThemeName, getEditorTheme } from ${JSON.stringify(themeModule)};
			initThemeSync();
			const prepaint = getCurrentThemeName();
			const before = getEditorTheme().surfaceColor("draft");
			await initConfiguredTheme(Settings.isolated());
			process.stdout.write(JSON.stringify({name:getCurrentThemeName(),prepaint,surface:getEditorTheme().surfaceColor("draft"),before,after:getEditorTheme().surfaceColor("draft")}));
		`;
		const [dark, light] = await Promise.all([probe(code, "dark"), probe(code, "light")]);
		for (const result of [dark, light]) {
			expect(result.prepaint).toBe(result.name);
			expect(result.before).toBe(result.after);
			expect(Bun.stripANSI(result.surface)).toBe("draft");
		}
		expect(dark.surface).not.toBe(light.surface);
		expect(dark.surface).toContain("48;2;30;30;30");
		expect(light.surface).toContain("48;2;");
	});

	it("renders an explicit older theme from read-only configuration without rewriting it", async () => {
		const directory = TempDir.createSync("@harvest-configured-theme-");
		try {
			const configPath = path.join(directory.path(), "agent", "config.yml");
			const content = "theme:\n  dark: titanium\n  light: light\nsymbolPreset: ascii\n";
			await Bun.write(configPath, content);
			const result = await probe(
				`
				import { initConfiguredTheme } from ${JSON.stringify(configModule)};
				import { getCurrentThemeName, getEditorTheme } from ${JSON.stringify(themeModule)};
				import { setAgentDir, setProjectDir } from ${JSON.stringify(dirsModule)};
				setAgentDir(${JSON.stringify(path.join(directory.path(), "agent"))});
				setProjectDir(${JSON.stringify(directory.path())});
				await initConfiguredTheme();
				process.stdout.write(JSON.stringify({name:getCurrentThemeName(),surface:getEditorTheme().surfaceColor("draft")}));
			`,
				"dark",
			);
			expect(result.name).toBe("titanium");
			expect(result.surface).not.toContain("48;2;30;30;30");
			expect(Bun.stripANSI(result.surface)).toBe("draft");
			expect(await Bun.file(configPath).text()).toBe(content);
			expect(await Bun.file(path.join(directory.path(), "agent", "agent.db")).exists()).toBe(false);
		} finally {
			await directory.remove();
		}
	});
});
