import { getProjectDir } from "@harvest/pi-utils";
import { initTheme } from "../modes/theme/theme";
import { Settings } from "./settings";

/** Apply effective appearance preferences without creating persistent settings for standalone commands. */
export async function initConfiguredTheme(settings?: Settings, enableWatcher = false): Promise<void> {
	const preferences = settings ?? (await Settings.loadReadOnly({ cwd: getProjectDir() }));
	await initTheme(
		enableWatcher,
		preferences.get("symbolPreset"),
		preferences.get("colorBlindMode"),
		preferences.get("theme.dark"),
		preferences.get("theme.light"),
	);
}
