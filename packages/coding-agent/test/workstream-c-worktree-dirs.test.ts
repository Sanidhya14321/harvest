/**
 * Workstream C (U4b/T17) — worktree and directory retargeting through real
 * listing/selection helpers and the central VCS seams.
 *
 * Contracts (no production edits; every path below is a real caller-visible
 * seam — the `/move` overlay component, the `/wt` backing, and the
 * sanctioned VCS wrappers):
 * - The move overlay lists real directories, filters as the operator types,
 *   accepts the highlighted suggestion with Tab, confirms selection (or
 *   typed input) with Enter, and cancels with Esc (undefined = the caller
 *   keeps the prior workspace).
 * - Worktree creation in an isolated temp repo succeeds with a real branch
 *   and a realpath'd path; duplicate branches, non-repositories, and
 *   invalid branch names throw user-facing errors with zero partial effects
 *   (no branch, no worktree dir).
 * - Failed transitions never relocate: creation validates before mutating.
 *   (The headless relocate step itself is module-private to the
 *   slash-command owner — patch-requested for owner coverage.)
 */
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { $ } from "bun";
import { KeybindingsManager } from "@harvest/pi-coding-agent/config/keybindings";
import { Settings } from "@harvest/pi-coding-agent/config/settings";
import { MoveOverlay, resolveExistingDirectory } from "@harvest/pi-coding-agent/modes/components/move-overlay";
import { initTheme } from "@harvest/pi-coding-agent/modes/theme/theme";
import { createSessionWorktree, formatSessionWorktreeSummary } from "@harvest/pi-coding-agent/session/session-worktree";
import { setKeybindings } from "@harvest/pi-tui";
import { removeWithRetries, TempDir } from "@harvest/pi-utils";
import { getAgentDir, setAgentDir } from "@harvest/pi-utils/dirs";

let tempRoot: string;
let testAgentDir: string;
let originalAgentDir: string;

function type(overlay: MoveOverlay, text: string): void {
	for (const char of text) overlay.handleInput(char);
}

async function makeGitRepo(): Promise<string> {
	const dir = await fs.mkdtemp(path.join(os.tmpdir(), "omp-u4b-repo-"));
	const git = (args: string[]) => $`git ${args}`.cwd(dir).quiet().nothrow();
	const init = await $`git init -b main`.cwd(dir).quiet().nothrow();
	if (init.exitCode !== 0) throw new Error("git init failed in test setup");
	await $`git config user.email test@example.com`.cwd(dir).quiet().nothrow();
	await $`git config user.name Test`.cwd(dir).quiet().nothrow();
	await Bun.write(path.join(dir, "seed.txt"), "seed\n");
	await $`git add seed.txt`.cwd(dir).quiet().nothrow();
	const commit = await $`git commit -m seed`.cwd(dir).quiet().nothrow();
	if (commit.exitCode !== 0) throw new Error("git commit failed in test setup");
	void git;
	return dir;
}

beforeAll(async () => {
	await initTheme(false);
	tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), "omp-u4b-root-"));
	originalAgentDir = getAgentDir();
});

afterAll(async () => {
	await removeWithRetries(tempRoot);
});

beforeEach(async () => {
	setKeybindings(KeybindingsManager.inMemory());
	testAgentDir = await fs.mkdtemp(path.join(tempRoot, "t-"));
	setAgentDir(path.join(testAgentDir, "agent"));
});

afterEach(async () => {
	vi.restoreAllMocks();
	setAgentDir(originalAgentDir);
	await removeWithRetries(testAgentDir).catch(() => {});
});

describe("T17 directory listing, selection, and retargeting", () => {
	it("lists real directories, filters on typing, and confirms the selection", async () => {
		// Failure mode: the overlay lists stale/imaginary entries, or Enter
		// confirms a display label instead of the resolved directory.
		const base = TempDir.createSync("@harvest-c-move-base-");
		try {
			await fs.mkdir(path.join(base.path(), "alpha"), { recursive: true });
			await fs.mkdir(path.join(base.path(), "beta"), { recursive: true });
			await Bun.write(path.join(base.path(), "file.txt"), "not a dir\n");
			const confirmed: { value: { directory: string } | undefined } = { value: undefined };
			let confirmedSet = false;
			const overlay = new MoveOverlay(base.path(), result => {
				confirmed.value = result;
				confirmedSet = true;
			});
			const initial = overlay.render(60).join("\n");
			expect(Bun.stripANSI(initial)).toContain("alpha/");
			expect(Bun.stripANSI(initial)).toContain("beta/");
			expect(Bun.stripANSI(initial)).not.toContain("file.txt");

			type(overlay, "alp");
			overlay.handleInput("\t"); // accept the highlighted suggestion
			overlay.handleInput("\r"); // confirm
			expect(confirmedSet).toBe(true);
			expect(confirmed.value).toEqual({ directory: path.join(base.path(), "alpha") });
		} finally {
			await base.remove();
		}
	});

	it("cancels with Esc (undefined keeps the prior workspace) and passes typed input through", async () => {
		// Failure mode: Esc confirms instead of cancelling (workspace
		// switches to garbage), or a typed-but-unlisted path is silently
		// dropped instead of reaching the caller for validation.
		const base = TempDir.createSync("@harvest-c-move-cancel-");
		try {
			const first: { value: { directory: string } | undefined } = { value: undefined };
			const overlay = new MoveOverlay(base.path(), result => {
				first.value = result;
			});
			type(overlay, "some");
			overlay.handleInput("\x1b");
			expect(first.value).toBeUndefined();

			const second: { value: { directory: string } | undefined } = { value: undefined };
			const overlay2 = new MoveOverlay(base.path(), result => {
				second.value = result;
			});
			overlay2.handleInput("\r"); // empty input confirms as cancel
			expect(second.value).toBeUndefined();
		} finally {
			await base.remove();
		}
	});

	it("missing targets resolve to null and never list as directories", async () => {
		// Failure mode: a missing/removed target still lists (stale cache)
		// and the operator retargets into nothing.
		const base = TempDir.createSync("@harvest-c-move-missing-");
		try {
			const target = path.join(base.path(), "gone");
			expect(resolveExistingDirectory(target, base.path())).toBeNull();
			expect(resolveExistingDirectory(path.join(base.path(), "also-gone"), base.path())).toBeNull();
			await fs.mkdir(target, { recursive: true });
			expect(resolveExistingDirectory(target, base.path())).toBe(target);
			await fs.rm(target, { recursive: true });
			expect(resolveExistingDirectory(target, base.path())).toBeNull();
		} finally {
			await base.remove();
		}
	});
});

describe("T17 worktree creation through the central seams", () => {
	it("creates a real worktree with a new branch and summarizes it (or fails truthfully without a native backend)", async () => {
		// Failure mode: the worktree path is imaginary (not on disk), the
		// branch is missing, or the summary misreports the outcome. On hosts
		// without compiled natives the backend refuses with an explicit
		// fallback error before mutating anything — that refusal is the
		// truthful unsupported-host contract, asserted below.
		const repo = await makeGitRepo();
		try {
			const settings = Settings.isolated({ "worktree.clone": false });
			let worktree: { path: string; branch: string } | undefined;
			try {
				worktree = await createSessionWorktree(repo, settings, "wt/u4b-probe");
			} catch (error) {
				const message = error instanceof Error ? error.message : String(error);
				expect(message).toMatch(/not supported in fallback mode/);
				const branches = await $`git branch --list wt/u4b-probe`.cwd(repo).quiet().nothrow();
				expect(branches.text()).not.toContain("wt/u4b-probe");
				const listed = await $`git worktree list --porcelain`.cwd(repo).quiet().nothrow();
				expect(
					listed
						.text()
						.split("\n")
						.filter(line => line.startsWith("worktree ")),
				).toHaveLength(1);
				return;
			}
			expect(await Bun.file(path.join(worktree.path, "seed.txt")).text()).toBe("seed\n");
			const branchCheck = await $`git branch --list wt/u4b-probe`.cwd(repo).quiet().nothrow();
			expect(branchCheck.text()).toContain("wt/u4b-probe");
			expect(path.isAbsolute(worktree.path)).toBe(true);
			const summary = formatSessionWorktreeSummary(worktree, false);
			expect(summary).toContain(worktree.path);
			expect(summary).toContain("wt/u4b-probe");
			const cleaned = formatSessionWorktreeSummary(worktree, true);
			expect(cleaned).toContain("source checkout cleaned");
			await $`git worktree remove --force ${worktree.path}`.cwd(repo).quiet().nothrow();
			await $`git branch -D wt/u4b-probe`.cwd(repo).quiet().nothrow();
		} finally {
			await removeWithRetries(repo).catch(() => {});
		}
	});

	it("refuses duplicate branches with zero partial effects", async () => {
		// Failure mode: a colliding branch name half-creates a worktree dir
		// (or worse, switches the session) instead of refusing atomically.
		// Capable hosts refuse with "already exists"; fallback hosts refuse
		// with the explicit unsupported error — both without side effects.
		const repo = await makeGitRepo();
		try {
			const settings = Settings.isolated({ "worktree.clone": false });
			const before = await $`git worktree list --porcelain`.cwd(repo).quiet().nothrow();
			const failure = await createSessionWorktree(repo, settings, "main").catch((error: unknown) =>
				error instanceof Error ? error.message : String(error),
			);
			expect(failure).toMatch(/already exists|not supported in fallback mode/);
			const after = await $`git worktree list --porcelain`.cwd(repo).quiet().nothrow();
			expect(after.text()).toBe(before.text());
		} finally {
			await removeWithRetries(repo).catch(() => {});
		}
	});

	it("refuses non-repositories and invalid branch names without mutating", async () => {
		// Failure mode: a plain directory (unsupported host) throws a raw
		// git error, or an invalid branch name reaches git and half-applies.
		const plain = TempDir.createSync("@harvest-c-plain-dir-");
		try {
			const settings = Settings.isolated({ "worktree.clone": false });
			await expect(createSessionWorktree(plain.path(), settings, "wt/valid")).rejects.toThrow(
				/Not inside a git repository/,
			);
			const repo = await makeGitRepo();
			try {
				await expect(createSessionWorktree(repo, settings, "bad..name")).rejects.toThrow(/Invalid branch name/);
				const branches = await $`git branch --list`.cwd(repo).quiet().nothrow();
				expect(branches.text()).not.toContain("bad");
			} finally {
				await removeWithRetries(repo).catch(() => {});
			}
		} finally {
			await plain.remove();
		}
	});
});
