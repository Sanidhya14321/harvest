import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { SPINNER_GLYPH_ADVANCE_MS, sharedSpinnerFrame } from "@harvest/pi-coding-agent/modes/components/tool-execution";
import { SPINNER_FRAMES } from "@harvest/pi-coding-agent/modes/theme/symbols";
import { getThemeByName } from "@harvest/pi-coding-agent/modes/theme/theme";
import { getConfigRootDir, getCustomThemesDir, removeWithRetries, setAgentDir } from "@harvest/pi-utils";

// Path of the built-in dark theme JSON, used as a known-valid base we can
// extend with custom `symbols.spinnerFrames` shapes.
const DARK_THEME_PATH = path.join(import.meta.dir, "..", "src", "modes", "theme", "dark.json");

const originalAgentDir = process.env.PI_CODING_AGENT_DIR;
const fallbackAgentDir = path.join(getConfigRootDir(), "agent");

let tmpAgentDir: string;

async function writeCustomTheme(name: string, extraSymbols: Record<string, unknown>): Promise<void> {
	const dark = (await Bun.file(DARK_THEME_PATH).json()) as Record<string, unknown>;
	const base = (dark.symbols ?? {}) as Record<string, unknown>;
	const themeJson = {
		...dark,
		name,
		symbols: { ...base, ...extraSymbols },
	};
	const themesDir = getCustomThemesDir();
	await fs.mkdir(themesDir, { recursive: true });
	await Bun.write(path.join(themesDir, `${name}.json`), JSON.stringify(themeJson, null, 2));
}

describe("theme symbols.spinnerFrames", () => {
	beforeEach(async () => {
		tmpAgentDir = await fs.mkdtemp(path.join(os.tmpdir(), "omp-spinner-frames-"));
		setAgentDir(tmpAgentDir);
	});

	afterEach(async () => {
		if (originalAgentDir) {
			setAgentDir(originalAgentDir);
		} else {
			setAgentDir(fallbackAgentDir);
			delete process.env.PI_CODING_AGENT_DIR;
		}
		await removeWithRetries(tmpAgentDir);
	});

	it("flat-array override applies to status, activity, and working spinners", async () => {
		const frames = ["◐", "◓", "◑", "◒"];
		await writeCustomTheme("custom-flat", { spinnerFrames: frames });

		const theme = await getThemeByName("custom-flat");
		expect(theme).toBeDefined();
		expect(theme!.getSpinnerFrames("status")).toEqual(frames);
		expect(theme!.getSpinnerFrames("activity")).toEqual(frames);
		expect(theme!.getSpinnerFrames("working")).toEqual(frames);
		// Default getter is the status spinner.
		expect(theme!.spinnerFrames).toEqual(frames);
	});

	it("object override sets each spinner type independently and falls back to preset", async () => {
		const statusFrames = ["A", "B", "C"];
		// `unicode` preset's activity frames — the default we expect to surface
		// when only `status` is overridden.
		const presetActivity = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
		await writeCustomTheme("custom-status-only", { spinnerFrames: { status: statusFrames } });

		const theme = await getThemeByName("custom-status-only");
		expect(theme).toBeDefined();
		expect(theme!.getSpinnerFrames("status")).toEqual(statusFrames);
		expect(theme!.getSpinnerFrames("activity")).toEqual(presetActivity);
	});

	it("rejects empty arrays and empty objects at validation time", async () => {
		await writeCustomTheme("custom-empty-array", { spinnerFrames: [] });
		await expect(getThemeByName("custom-empty-array")).resolves.toBeUndefined();

		await writeCustomTheme("custom-empty-object", { spinnerFrames: {} });
		await expect(getThemeByName("custom-empty-object")).resolves.toBeUndefined();
	});

	it("falls through to preset frames when `spinnerFrames` is absent", async () => {
		// `dark` ships with `symbols.preset: "unicode"`; we only assert that the
		// default status frames match the preset table when no override is set.
		await writeCustomTheme("custom-no-override", {});

		const theme = await getThemeByName("custom-no-override");
		expect(theme).toBeDefined();
		const status = theme!.getSpinnerFrames("status");
		expect(status.length).toBeGreaterThan(1);
		expect(status).not.toContain("A");
	});

	it("serves width-stable KnightRider working blocks per preset (busy-row)", () => {
		expect(SPINNER_FRAMES.unicode.working.length).toBeGreaterThan(1);
		const widths = new Set(SPINNER_FRAMES.unicode.working.map(frame => Bun.stringWidth(frame)));
		expect(widths.size).toBe(1);
		const asciiWidths = new Set(SPINNER_FRAMES.ascii.working.map(frame => Bun.stringWidth(frame)));
		expect(asciiWidths.size).toBe(1);
		for (const frame of SPINNER_FRAMES.ascii.working) {
			expect(/^[\x20-\x7e]*$/.test(frame)).toBe(true);
		}
	});

	it("accepts a working-only object override (partial-override)", async () => {
		const working = ["1", "2"];
		await writeCustomTheme("custom-working-only", { spinnerFrames: { working } });

		const theme = await getThemeByName("custom-working-only");
		expect(theme).toBeDefined();
		expect(theme!.getSpinnerFrames("working")).toEqual(working);
		expect(theme!.getSpinnerFrames("status").length).toBeGreaterThan(1);
	});

	it("derives live tool spinner frames from a shared clock", () => {
		const frameCount = 4;
		const now = SPINNER_GLYPH_ADVANCE_MS * 3 + 12;

		expect(sharedSpinnerFrame(frameCount, now)).toBe(sharedSpinnerFrame(frameCount, now));
		expect(sharedSpinnerFrame(frameCount, now + SPINNER_GLYPH_ADVANCE_MS)).toBe(
			(sharedSpinnerFrame(frameCount, now) + 1) % frameCount,
		);
		expect(sharedSpinnerFrame(frameCount, SPINNER_GLYPH_ADVANCE_MS * frameCount)).toBe(0);
		expect(sharedSpinnerFrame(0, now)).toBe(0);
	});
});
