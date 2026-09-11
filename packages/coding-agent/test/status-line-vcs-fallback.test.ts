import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { resetSettingsForTest, Settings } from "@harvest/pi-coding-agent/config/settings";
import type { StatusLineSettings } from "@harvest/pi-coding-agent/modes/components/status-line";
import { StatusLineComponent } from "@harvest/pi-coding-agent/modes/components/status-line";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import type { VcsRepo } from "@harvest/pi-natives";
import * as vcs from "@harvest/pi-natives/vcs";
import { getProjectDir, setProjectDir } from "@harvest/pi-utils";

const originalProjectDir = getProjectDir();

const gitSegmentSettings: StatusLineSettings = {
	preset: "custom",
	leftSegments: ["git"],
	rightSegments: ["session_name"],
	separator: "powerline-thin",
	sessionAccent: false,
	transparent: false,
};

function makeSession() {
	return {
		state: { messages: [], model: undefined },
		messages: [],
		model: undefined,
		systemPrompt: [],
		agent: { state: { tools: [] } },
		skills: [],
		isStreaming: false,
		isAutoThinking: false,
		autoResolvedThinkingLevel: () => undefined,
		isFastModeActive: () => false,
		isFastModeEnabled: () => false,
		getGoalModeState: () => null,
		getAsyncJobSnapshot: () => ({ running: [] }),
		modelRegistry: { isUsingOAuth: () => false },
		hookStatuses: () => [],
		planMode: { enabled: false },
		sessionManager: {
			getSessionName: () => "vcs-fallback test",
			getUsageStatistics: () => ({
				input: 0,
				output: 0,
				cacheRead: 0,
				cacheWrite: 0,
				totalTokens: 0,
				orchestrationInput: 0,
				orchestrationOutput: 0,
				orchestrationCacheRead: 0,
				premiumRequests: 0,
				cost: 0,
			}),
		},
	} as unknown as ConstructorParameters<typeof StatusLineComponent>[0];
}

let testDir: string;

describe("StatusLineComponent VCS fallback resilience", () => {
	beforeAll(async () => {
		resetSettingsForTest();
		await Settings.init({ inMemory: true });
		await initTheme();
	});

	afterAll(() => {
		resetSettingsForTest();
		setProjectDir(originalProjectDir);
	});

	beforeEach(async () => {
		testDir = await fs.mkdtemp(path.join(os.tmpdir(), "vcs-fallback-test-"));
		setProjectDir(testDir);
	});

	afterEach(async () => {
		vi.restoreAllMocks();
		setProjectDir(originalProjectDir);
		if (testDir) {
			await fs.rm(testDir, { recursive: true, force: true });
		}
	});

	it("does not throw when repository is an object without asGit method", () => {
		// Simulate a repository object lacking asGit (the exact failure mode that crashed harvest)
		const legacyRepoWithoutAsGit = {
			kind: () => "git",
			root: () => "/test/repo",
			primaryRoot: () => "/test/repo",
			prefixOf: () => null,
			watchTarget: () => "/test/repo/.git/HEAD",
			supports: () => false,
			label: async () => "fallback-branch",
		} as unknown as VcsRepo;

		vi.spyOn(vcs, "repo").mockReturnValue(legacyRepoWithoutAsGit);

		const component = new StatusLineComponent(makeSession());
		component.updateSettings(gitSegmentSettings);

		// Must render cleanly without throwing TypeError: repository.asGit is not a function
		expect(() => component.getTopBorder(80)).not.toThrow();
		expect(() => component.renderBottomBar(80, "full")).not.toThrow();
	});

	it("renders git branch label when asGit() returns a git repository handle", () => {
		const gitRepoHandle = {
			headSync: () => ({ kind: "ref", branch: "feature/resilient", refName: "refs/heads/feature/resilient" }),
			head: async () => ({ kind: "ref", branch: "feature/resilient", refName: "refs/heads/feature/resilient" }),
		};

		const repoWithAsGit = {
			kind: () => "git",
			root: () => "/test/repo",
			primaryRoot: () => "/test/repo",
			prefixOf: () => null,
			watchTarget: () => "/test/repo/.git/HEAD",
			supports: () => true,
			asGit: () => gitRepoHandle,
			label: async () => "feature/resilient",
		} as unknown as VcsRepo;

		vi.spyOn(vcs, "repo").mockReturnValue(repoWithAsGit);
		vi.spyOn(vcs, "gitInfo").mockReturnValue({
			repoRoot: "/test/repo",
			gitDir: "/test/repo/.git",
			commonDir: "/test/repo/.git",
			gitEntryPath: "/test/repo/.git",
			headPath: "/test/repo/.git/HEAD",
			isReftable: false,
		});

		const component = new StatusLineComponent(makeSession());
		component.updateSettings(gitSegmentSettings);

		const border = component.getTopBorder(80);
		expect(border.content).toContain("feature/resilient");
	});
});
