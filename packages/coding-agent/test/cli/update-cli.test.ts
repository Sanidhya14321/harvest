import { afterEach, describe, expect, it, vi } from "bun:test";
import { getLatestRelease, runUpdateCommand } from "../../src/cli/update-cli";

type FetchInput = string | URL | Request;
type FetchInit = RequestInit | BunFetchRequestInit;

describe("update repository routing", () => {
	for (const [override, repository] of [
		["", "Sanidhya14321/harvest"],
		["fixture-owner/custom-harvest", "fixture-owner/custom-harvest"],
	]) {
		it(`${override ? "keeps an explicit repository override" : "uses the public repository for an empty override"} for asset validation and mise updates`, async () => {
			const moduleUrl = new URL("../../src/cli/update-cli.ts", import.meta.url).href;
			const url = `https://github.com/${repository}/releases/download/v1.2.3/omp-linux-x64`;
			const source = `
				import { buildMiseUpgradeArgs, buildMiseForceInstallArgs, resolveReleaseBinaryAsset } from ${JSON.stringify(moduleUrl)};
				const release = {
					tag_name: "v1.2.3", draft: false, prerelease: false,
					assets: [{ name: "omp-linux-x64", state: "uploaded", size: 1,
						digest: "sha256:" + "a".repeat(64), browser_download_url: ${JSON.stringify(url)} }]
				};
				const asset = resolveReleaseBinaryAsset(release, "v1.2.3", "omp-linux-x64");
				release.assets[0].browser_download_url = "https://github.com/harvest/harvest/releases/download/v1.2.3/omp-linux-x64";
				let rejectedOtherRepository = false;
				try { resolveReleaseBinaryAsset(release, "v1.2.3", "omp-linux-x64"); }
				catch { rejectedOtherRepository = true; }
				process.stdout.write(JSON.stringify({
					url: asset.url,
					upgrade: buildMiseUpgradeArgs(),
					force: buildMiseForceInstallArgs("1.2.3"),
					rejectedOtherRepository
				}));
			`;
			// A fresh process exercises module-load environment defaults without
			// mutating this test runner's shared environment or module registry.
			const child = Bun.spawn([process.execPath, "--eval", source], {
				env: { ...process.env, HARVEST_REPO: override },
				stdout: "pipe",
				stderr: "pipe",
			});
			const [stdout, stderr, exitCode] = await Promise.all([
				new Response(child.stdout).text(),
				new Response(child.stderr).text(),
				child.exited,
			]);
			expect(exitCode, stderr).toBe(0);
			expect(JSON.parse(stdout)).toEqual({
				url,
				upgrade: ["upgrade", `github:${repository}`, "--bump"],
				force: ["install", "--force", `github:${repository}@1.2.3`],
				rejectedOtherRepository: true,
			});
		});
	}
});

describe("runUpdateCommand fetch cancellation", () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("checks release metadata with a timeout signal", async () => {
		let requestSignal: AbortSignal | undefined;
		vi.spyOn(console, "log").mockImplementation(() => {});
		const fetchStub = Object.assign(
			async (_input: FetchInput, init?: FetchInit) => {
				requestSignal = init?.signal ?? undefined;
				return Response.json({ version: "999.0.0" });
			},
			{ preconnect: globalThis.fetch.preconnect },
		);
		vi.spyOn(globalThis, "fetch").mockImplementation(fetchStub);

		await runUpdateCommand({ force: false, check: true });

		expect(requestSignal).toBeInstanceOf(AbortSignal);
	});
});

describe("getLatestRelease rename pointers", () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	function stubRegistry(manifests: Record<string, unknown>): string[] {
		const urls: string[] = [];
		const fetchStub = Object.assign(
			async (input: FetchInput) => {
				const url = String(input);
				urls.push(url);
				let manifest: unknown;
				for (const pkg in manifests) {
					if (url.includes(pkg)) {
						manifest = manifests[pkg];
						break;
					}
				}
				if (!manifest) return new Response(null, { status: 404, statusText: "Not Found" });
				return Response.json(manifest);
			},
			{ preconnect: globalThis.fetch.preconnect },
		);
		vi.spyOn(globalThis, "fetch").mockImplementation(fetchStub);
		return urls;
	}

	it("follows omp.rename to the new package and resolves version, dist, and names from its manifest", async () => {
		const urls = stubRegistry({
			"@new/omp": { version: "999.1.0", omp: { dist: "npm" } },
			"@harvest/pi-coding-agent": {
				version: "999.0.0",
				omp: { dist: "binary", rename: { package: "@new/omp", natives: "@new/natives" } },
			},
		});

		const release = await getLatestRelease();

		expect(release.version).toBe("999.1.0");
		expect(release.dist).toBe("npm");
		expect(release.packages).toEqual({ pkg: "@new/omp", natives: "@new/natives" });
		expect(urls).toEqual([
			"https://registry.npmjs.org/@harvest/pi-coding-agent/latest",
			"https://registry.npmjs.org/@new/omp/latest",
		]);
	});
	it("fetches the canary dist-tag when checking the canary channel", async () => {
		const urls = stubRegistry({
			"@harvest/pi-coding-agent": { version: "999.0.0-canary.1" },
		});

		await getLatestRelease({ channel: "canary" });

		expect(urls).toEqual(["https://registry.npmjs.org/@harvest/pi-coding-agent/canary"]);
	});

	it("ignores a rename pointer that cycles back to an already-visited package", async () => {
		const urls = stubRegistry({
			"@harvest/pi-coding-agent": {
				version: "999.0.0",
				omp: { rename: { package: "@harvest/pi-coding-agent" } },
			},
		});

		const release = await getLatestRelease();

		expect(urls).toHaveLength(1);
		expect(release.version).toBe("999.0.0");
		expect(release.packages).toEqual({ pkg: "@harvest/pi-coding-agent", natives: "@harvest/pi-natives" });
	});
});

describe("getLatestRelease proxy errors", () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("translates Bun's UnsupportedProxyProtocol fetch failure into an actionable CLI message", async () => {
		const fetchStub = Object.assign(
			async () => {
				throw new Error(
					'UnsupportedProxyProtocol fetching "https://registry.npmjs.org/@harvest/pi-coding-agent/latest". ' +
						"For more information, pass `verbose: true` in the second argument to fetch()",
				);
			},
			{ preconnect: globalThis.fetch.preconnect },
		);
		vi.spyOn(globalThis, "fetch").mockImplementation(fetchStub);

		const err = await getLatestRelease({ timeoutMs: 5000 }).then(
			() => null,
			(e: unknown) => e as Error,
		);

		expect(err).toBeInstanceOf(Error);
		// The raw fetch() instruction the CLI user cannot act on must not leak through.
		expect(err?.message).not.toContain("verbose: true");
		expect(err?.message).not.toContain("fetch()");
		// Instead the user gets actionable guidance about supported proxy schemes.
		expect(err?.message).toMatch(/SOCKS/i);
		expect(err?.message).toMatch(/https?:\/\//i);
	});
});
