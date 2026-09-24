import { Args, Command, Flags } from "@harvest/pi-utils/cli";
import { getProjectDir } from "@harvest/pi-utils";
import { Settings } from "../config/settings";
import { layaHelp as commandHelp } from "../cli/command-help";
import { runLayaCommand, type LayaAction, type LayaCommandArgs } from "../cli/laya-cli";

const ACTIONS: LayaAction[] = ["setup", "calibrate", "status", "review-shadow", "recalibrate-subagent"];

export default class Laya extends Command {
	static description = commandHelp.description;
	static args = {
		action: Args.string({
			description: "Action to perform (setup | calibrate | status | review-shadow | recalibrate-subagent)",
			required: false,
			options: ACTIONS,
		}),
	};

	static flags = {
		force: Flags.boolean({ description: "Force recalibration even if hardware signature is unchanged" }),
		reinstall: Flags.boolean({ description: "Force reinstallation of dependencies and virtual environment" }),
		port: Flags.integer({ description: "Port for Laya sidecar (default: 8177)" }),
		url: Flags.string({ description: "Base URL for Laya sidecar (default: http://127.0.0.1:8177)" }),
		json: Flags.boolean({ description: "Output JSON" }),
		yes: Flags.boolean({ char: "y", description: "Confirm applying recalibrated threshold or auto-agree during setup" }),
		label: Flags.string({ description: "Label a specific trace: <traceId>:<laya_correct|caller_correct|ambiguous>" }),
		limit: Flags.integer({ description: "Limit number of items to review" }),
	};

	async run(): Promise<void> {
		try {
			await Settings.init({ cwd: getProjectDir() });
		} catch {
			// Non-fatal if settings already initialized or in test environment
		}
		const { args, flags } = await this.parse(Laya);
		const command: LayaCommandArgs = {
			action: (args.action ?? "status") as LayaAction,
			flags: {
				force: flags.force,
				reinstall: flags.reinstall,
				port: flags.port,
				url: flags.url,
				json: flags.json,
				yes: flags.yes,
				label: flags.label,
				limit: flags.limit,
			},
		};
		await runLayaCommand(command);
	}
}
