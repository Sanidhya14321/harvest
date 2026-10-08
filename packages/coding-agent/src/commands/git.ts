/**
 * Fullscreen interactive git TUI: split-pane diff viewer with minimap, file
 * staging sidebar, and generated or manual commit composer.
 */

import { getProjectDir } from "@harvest/pi-utils";
import { Args, Command, Flags } from "@harvest/pi-utils/cli";
import { gitHelp as commandHelp } from "../cli/command-help";
import { runGitTui } from "../cli/git-tui";
import { Settings, settings } from "../config/settings";
import { initConfiguredTheme } from "../config/theme";

export default class Git extends Command {
	static description = commandHelp.description;

	static args = {
		revision: Args.string({
			description: "Pin the view to one commit (any revision, e.g. HEAD~2 or a sha)",
			required: false,
		}),
	};

	static flags = {
		dir: Flags.string({ char: "C", description: "Run in another directory instead of the current one" }),
	};

	static examples = ["harvest git", "harvest git HEAD~2", "harvest git -C ~/projects/app"];

	async run(): Promise<void> {
		const { args, flags } = await this.parse(Git);
		if (process.stdout.isTTY !== true || process.stdin.isTTY !== true) {
			console.error("harvest git is interactive and requires a TTY");
			process.exit(1);
		}
		// Load settings first so the user's configured theme/symbol preset apply
		// exactly like interactive mode (bare initTheme falls back to built-ins).
		await Settings.init({ cwd: getProjectDir() });
		await initConfiguredTheme(settings);
		await runGitTui({ cwd: flags.dir, revision: args.revision });
	}
}
