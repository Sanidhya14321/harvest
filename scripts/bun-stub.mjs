import * as fs from "node:fs";

export const $ = () => ({
	cwd: () => ({ quiet: () => ({ nothrow: () => Promise.resolve({ exitCode: 0, text: () => "" }) }) }),
	quiet: () => ({ nothrow: () => Promise.resolve({ exitCode: 0, text: () => "" }) }),
	nothrow: () => Promise.resolve({ exitCode: 0, text: () => "" }),
});

export const file = (p) => ({
	text: () => fs.promises.readFile(p, "utf8"),
	json: async () => JSON.parse(await fs.promises.readFile(p, "utf8")),
	exists: async () => fs.existsSync(p),
});

export const write = (p, data) => fs.promises.writeFile(p, data, "utf8");
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export const semver = {
	order: () => 0,
};
export const stringWidth = (s) => s.length;
export const wrapAnsi = (s) => s;
