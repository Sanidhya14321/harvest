import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { resetSettingsForTest, Settings } from "../src/config/settings";
import { COMPOSER_SHAPE_VALUES } from "../src/config/settings-schema";
import { AttachmentChipsBand } from "../src/modes/components/attachment-chips";
import { renderComposerShapePreview } from "../src/modes/components/composer-shape-preview";
import { ErrorBannerComponent } from "../src/modes/components/error-banner";
import { CleansePanelComponent } from "../src/modes/components/cleanse-panel";
import { TranscriptContainer } from "../src/modes/components/transcript-container";
import { WelcomeComponent, gradientLogo, renderWelcomeTip } from "../src/modes/components/welcome";
import { Composer } from "../src/modes/composer";
import { createTheme, getBuiltinThemes } from "../src/modes/theme/loader";
import { getEditorTheme, setThemeInstance, theme, type Theme } from "../src/modes/theme/theme";
import { Container, CURSOR_MARKER, Editor, ImageBudget, Spacer, Text } from "@harvest/pi-tui";
import { VirtualTerminal } from "../../tui/test/virtual-terminal";

let previousTheme: Theme | undefined;
const composers: Composer[] = [];
beforeEach(async () => {
	previousTheme = theme;
	resetSettingsForTest();
	await Settings.init({ inMemory: true });
	setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "none", symbolPresetOverride: "ascii" }));
});
afterEach(() => {
	for (const composer of composers.splice(0)) composer.stop();
	resetSettingsForTest();
	if (previousTheme) setThemeInstance(previousTheme);
});

function mount(fullscreen: boolean): { composer: Composer; error: ErrorBannerComponent } {
	const composer = new Composer({
		terminal: new VirtualTerminal(80, 24),
		preferences: { quiet: true, fullscreen, composerShape: "band", sidebar: "hide" },
	});
	composers.push(composer);
	const transcript = new TranscriptContainer();
	transcript.addChild(new Text("Conversation is still present", 0, 0));
	const errors = new Container();
	const error = new ErrorBannerComponent("Provider failed " + "long-token".repeat(60));
	errors.addChild(error);
	const attachments = new AttachmentChipsBand(composer.editor, new ImageBudget(0), () => {});
	const editorRoot = new Container();
	editorRoot.addChild(new Spacer(1));
	editorRoot.addChild(composer.editor);
	composer.setPinnedErrorContainer(errors);
	composer.setAttachmentContainer(attachments);
	composer.setRuntimeChildren([transcript, errors, attachments, editorRoot]);
	composer.setStatusComponent(new Text("Optional status\n".repeat(30), 0, 0));
	composer.start({ playWelcomeIntro: false });
	composer.editor.insertTextAttachment("pasted body\nsecond line");
	composer.editor.insertText(" draft stays editable");
	return { composer, error };
}

describe("composer surface allocation", () => {
	it.each([false, true])("allocates inline cleanse controls before clipping with fullscreen=%s", fullscreen => {
		const composer = new Composer({
			terminal: new VirtualTerminal(80, 24),
			preferences: { quiet: true, fullscreen, composerShape: "band", sidebar: "hide" },
		});
		composers.push(composer);
		const transcript = new TranscriptContainer();
		transcript.addChild(new Text("Conversation", 0, 0));
		const panelRoot = new Container();
		const panel = new CleansePanelComponent({ tui: composer.ui });
		panelRoot.addChild(panel);
		const editorRoot = new Container();
		editorRoot.addChild(composer.editor);
		composer.setRuntimeChildren([transcript, panelRoot, editorRoot]);
		composer.start({ playWelcomeIntro: false });
		composer.editor.insertText("Editable draft");
		try {
			for (let index = 0; index < 14; index++) panel.log(`Checker output ${index}`);
			const small = composer.renderFrame({ columns: 24, rows: 4 }).viewport;
			expect(Bun.stripANSI(small.join("\n"))).toContain("Esc cancel");
			expect(small.some(row => row.includes(CURSOR_MARKER))).toBe(true);
			expect(small.length).toBeLessThanOrEqual(4);
			panel.markError("Checker\tfailed");
			expect(Bun.stripANSI(composer.renderFrame({ columns: 24, rows: 4 }).viewport.join("\n"))).toContain(
				"Esc dismiss",
			);
			const expanded = Bun.stripANSI(composer.renderFrame({ columns: 80, rows: 24 }).viewport.join("\n"));
			expect(expanded.replace(/\s+/g, " ")).toContain("Checker failed");
			expect(composer.editor.getExpandedText()).toBe("Editable draft");
		} finally {
			panel.dispose();
		}
	});
	it.each([false, true])("keeps the cursor and staged draft through tiny resize with fullscreen=%s", fullscreen => {
		const { composer, error } = mount(fullscreen);
		const draft = composer.editor.getExpandedText();
		const chips = composer.editor.composerChips();
		const cursor = composer.editor.getCursor();
		for (const columns of [1, 2, 3, 4]) {
			for (const rows of [1, 2, 3, 4]) {
				const frame = composer.renderFrame({ columns, rows }).viewport;
				expect(frame.length).toBeLessThanOrEqual(rows);
				expect(frame.some(row => row.includes(CURSOR_MARKER))).toBe(true);
				for (const row of frame) expect(Bun.stringWidth(row)).toBeLessThanOrEqual(columns);
			}
		}
		expect(composer.editor.getExpandedText()).toBe(draft);
		expect(composer.editor.composerChips()).toEqual(chips);
		expect(composer.editor.getCursor()).toEqual(cursor);
		const restored = Bun.stripANSI(composer.renderFrame({ columns: 80, rows: 24 }).viewport.join("\n"));
		expect(restored).toContain("Provider failed");
		expect(restored).toContain("pasted body");
		expect(error.render(80).length).toBeLessThanOrEqual(6);
	});

	it("renders every shape preview with the live editor gutter and tiny-width fallback", () => {
		for (const shape of COMPOSER_SHAPE_VALUES) {
			const editor = new Editor(getEditorTheme());
			editor.setBorderStyle(shape);
			editor.setMaxHeight(1);
			editor.setText("Ask anything, edit files, run tools");
			for (const width of [1, 2, 3, 4]) {
				const preview = renderComposerShapePreview(shape, width);
				expect(preview).toEqual(editor.render(width));
				for (const row of preview) expect(Bun.stringWidth(row)).toBeLessThanOrEqual(width);
				expect(Bun.stripANSI(preview.join("\n"))).not.toMatch(/[\u0080-\uFFFF]/);
				expect(preview.join("\n")).not.toMatch(/\x1b\[(?:38|48);/);
			}
		}
	});

	it("updates cached welcome chrome when ASCII or no-color policy changes", () => {
		setThemeInstance(
			createTheme(getBuiltinThemes().harvest!, { mode: "truecolor", symbolPresetOverride: "unicode" }),
		);
		const welcome = new WelcomeComponent("1.0", "model", "provider");
		const colored = welcome.render(80).join("\n");
		expect(colored).toContain("\x1b");
		setThemeInstance(createTheme(getBuiltinThemes().harvest!, { mode: "none", symbolPresetOverride: "ascii" }));
		const plain = welcome.render(80).join("\n");
		expect(plain).not.toMatch(/[│─█╭╮╰╯]|\x1b/);
		expect(gradientLogo(["██"]).join("\n")).toBe("##");
		expect(renderWelcomeTip("Token".repeat(30) + " [NEW]", 24).join("\n")).not.toContain("\x1b");
		for (const width of [1, 2, 3, 4])
			for (const row of welcome.render(width)) expect(Bun.stringWidth(row)).toBeLessThanOrEqual(width);
		welcome.setMaxHeight(2);
		expect(welcome.render(80)).toHaveLength(2);
	});
});
