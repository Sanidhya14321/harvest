import { Args, Command, Flags } from "@harvest/pi-utils/cli";
import { layaHelp as commandHelp } from "../cli/command-help";
import { runLayaCommand, type LayaAction, type LayaCommandArgs } from "../cli/laya-cli";

const ACTIONS: LayaAction[] = ["calibrate", "status", "review-shadow", "recalibrate-subagent"];

export default class Laya extends Command {
	static description = commandHelp.description;
	static args = {
		action: Args.string({
			description: "Action to perform (calibrate | status | review-shadow | recalibrate-subagent)",
			required: false,
			options: ACTIONS,
		}),
	};

	static flags = {
		force: Flags.boolean({ description: "Force recalibration even if hardware signature is unchanged" }),
		json: Flags.boolean({ description: "Output JSON" }),
		yes: Flags.boolean({ char: "y", description: "Confirm applying recalibrated threshold to calibration file" }),
		label: Flags.string({ description: "Label a specific trace: <traceId>:<laya_correct|caller_correct|ambiguous>" }),
		limit: Flags.integer({ description: "Limit number of items to review" }),
	};

	async run(): Promise<void> {
		const { args, flags } = await this.parse(Laya);
		const command: LayaCommandArgs = {
			action: (args.action ?? "status") as LayaAction,
			flags: {
				force: flags.force,
				json: flags.json,
				yes: flags.yes,
				label: flags.label,
				limit: flags.limit,
			},
		};
		await runLayaCommand(command);
	}
}
