import { beforeAll, expect, it } from "bun:test";
import { stripVTControlCharacters } from "node:util";
import { SessionTabStrip } from "../../../src/modes/components/session-tab-strip";
import { initTheme } from "../../../src/modes/theme/theme";
import { SessionTabs } from "../../../src/session/session-tabs";

beforeAll(async () => {
	await initTheme(false);
});

it("shows open session titles and keeps control characters out of the tab strip", () => {
	const tabs = new SessionTabs();
	tabs.open("/work/first.jsonl", "First task");
	tabs.open("/work/second.jsonl", "Second\t\x1b[31mtask");
	const strip = new SessionTabStrip(
		tabs,
		() => "/work/second.jsonl",
		async () => {},
	);
	const text = stripVTControlCharacters(strip.render(80).join("\n"));
	expect(text).toContain("First task");
	expect(text).toContain("Second task");
	expect(text).not.toContain("\t");
});

it("keeps the active tab visible when many sessions are open", () => {
	const tabs = new SessionTabs();
	for (let index = 1; index <= 12; index++) tabs.open(`/work/${index}.jsonl`, `Task ${index}`);
	const strip = new SessionTabStrip(
		tabs,
		() => "/work/12.jsonl",
		async () => {},
	);
	const text = stripVTControlCharacters(strip.render(60).join("\n"));
	expect(text).toContain("Task 12");
	expect(text).not.toContain("Task 1 ");
});
