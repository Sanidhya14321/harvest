import type { TextContent } from "@harvest/pi-ai";
import type { CollabPromptDetails } from "../../collab/protocol";
import type { CustomMessage } from "../../session/messages";
import { UserMessageComponent } from "./user-message";

/**
 * Renders a collab guest prompt on every participant's transcript: a
 * user-message-styled bubble prefixed with the author's name.
 */
export class CollabPromptMessageComponent extends UserMessageComponent {
	constructor(message: CustomMessage<CollabPromptDetails>) {
		const from = message.details?.from?.trim() || "guest";
		const text =
			typeof message.content === "string"
				? message.content
				: message.content
						.filter((content): content is TextContent => content.type === "text")
						.map(content => content.text)
						.join("");
		super(text, false, undefined, { author: from, shellIntegration: false });
	}
}
