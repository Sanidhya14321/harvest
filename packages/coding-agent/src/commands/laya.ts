import { Args, Command, Flags } from "@harvest/pi-utils/cli";
import { layaHelp as commandHelp } from "../cli/command-help";
import { runLayaCommand, type LayaAction, type LayaCommandArgs } from "../cli/laya-cli";

const ACTIONS: LayaAction[] = ["calibrate", "status"];

export default class Laya extends Command {
	static description = commandHelp.description;
	static args = {
		action: Args.string({
			description: "Action to perform (calibrate | status)",
			required: false,
			options: ACTIONS,
		}),
	};

	static flags = {
		force: Flags.boolean({ description: "Force recalibration even if hardware signature is unchanged" }),
		json: Flags.boolean({ description: "Output JSON" }),
	};

	async run(): Promise<void> {
		const { args, flags } = await this.parse(Laya);
		const command: LayaCommandArgs = {
			action: (args.action ?? "status") as LayaAction,
			flags: {
				force: flags.force,
				json: flags.json,
			},
		};
		await runLayaCommand(command);
	}
}
