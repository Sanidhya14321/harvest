/**
 * Show what the read tool will return for a path, URL, or internal URI.
 */

import { Args, Command } from "@harvest/pi-utils/cli";
import { readHelp as commandHelp } from "../cli/command-help";
import { type ReadCommandArgs, runReadCommand } from "../cli/read-cli";
import { initTheme } from "../modes/theme/theme";

export default class Read extends Command {
	static description = commandHelp.description;
	static args = {
		path: Args.string({
			description:
				"Path, URL, or internal URI to read (append :sel for line ranges or raw mode, e.g. src/foo.ts:50-100)",
			required: true,
		}),
	};

	static examples = [
		"harvest read src/foo.ts",
		"harvest read src/foo.ts:50-100",
		"harvest read src/foo.ts:raw",
		"harvest read https://example.com",
		"harvest read harvest://",
		"harvest read issue://123",
		"harvest read path/to/archive.zip:dir/file.ts",
		"harvest read path/to/db.sqlite:users:42",
	];

	async run(): Promise<void> {
		const { args } = await this.parse(Read);
		const cmd: ReadCommandArgs = {
			path: args.path ?? "",
		};
		await initTheme();
		await runReadCommand(cmd);
	}
}
