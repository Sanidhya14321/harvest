import { afterEach, describe, expect, test } from "bun:test";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { vcsDiscover, vcsGitDiscover, vcsGitRepoInfo } from "../native/index.js";
import * as vcs from "../native/vcs.js";

const roots: string[] = [];
afterEach(async () => {
	await Promise.all(roots.splice(0).map(root => fs.rm(root, { force: true, recursive: true })));
});

async function createGitRepo(): Promise<string> {
	const root = await fs.mkdtemp(path.join(os.tmpdir(), "pi-natives-fallback-git-"));
	roots.push(root);
	const gitDir = path.join(root, ".git");
	await fs.mkdir(gitDir, { recursive: true });
	await fs.writeFile(path.join(gitDir, "HEAD"), "ref: refs/heads/feature-test\n");
	return root;
}

async function createJjRepo(): Promise<string> {
	const root = await fs.mkdtemp(path.join(os.tmpdir(), "pi-natives-fallback-jj-"));
	roots.push(root);
	const jjRepoDir = path.join(root, ".jj", "repo");
	await fs.mkdir(jjRepoDir, { recursive: true });
	return root;
}

describe("FallbackVcsRepo", () => {
	test("provides asGit() and git repo handles on Git repositories", async () => {
		const root = await createGitRepo();
		const repo = vcsDiscover(root);
		expect(repo).not.toBeNull();
		expect(repo?.kind()).toBe("git");
		expect(typeof repo?.asGit).toBe("function");

		const gitRepo = repo?.asGit();
		expect(gitRepo).not.toBeNull();
		expect(repo?.asJj()).toBeNull();

		// Check headSync shape
		const headState = gitRepo?.headSync();
		expect(headState).toMatchObject({ kind: "ref", branch: "feature-test" });

		// Check label
		const branchLabel = await repo?.label();
		expect(branchLabel).toBe("feature-test");

		// Check capabilities
		expect(repo?.supports("stagedDiff")).toBe(true);
		expect(repo?.supports("revDiff")).toBe(true);
		expect(() => repo?.supports("invalidFeature")).toThrow();

		// Check git info
		const info = vcsGitRepoInfo(root);
		expect(info?.repoRoot).toBe(root);
		expect(info?.headPath).toBe(path.join(root, ".git", "HEAD"));
		expect(info?.isReftable).toBe(false);

		// Check watch target
		expect(repo?.watchTarget()).toBe(path.join(root, ".git", "HEAD"));
	});

	test("provides asJj() and jj workspace handles on Jujutsu workspaces", async () => {
		const root = await createJjRepo();
		const repo = vcsDiscover(root);
		expect(repo).not.toBeNull();
		expect(repo?.kind()).toBe("jj");
		expect(repo?.asGit()).toBeNull();
		expect(repo?.asJj()).not.toBeNull();
		expect(repo?.supports("stagedDiff")).toBe(false);
		expect(repo?.supports("revDiff")).toBe(false);
	});

	test("returns null outside of any repository", async () => {
		const emptyDir = await fs.mkdtemp(path.join(os.tmpdir(), "pi-natives-empty-"));
		roots.push(emptyDir);
		expect(vcsDiscover(emptyDir)).toBeNull();
		expect(vcsGitDiscover(emptyDir)).toBeNull();
		expect(vcs.repo(emptyDir)).toBeNull();
	});
});
