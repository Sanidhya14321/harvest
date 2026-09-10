import { resolve as pathResolve, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import * as fs from "node:fs";

const bunStubUrl = pathToFileURL(pathResolve("scripts/bun-stub.mjs")).href;
const bunTestStubUrl = pathToFileURL(pathResolve("scripts/bun-test-stub.mjs")).href;

export async function resolve(specifier, context, nextResolve) {
	if (specifier === "bun") {
		return { url: bunStubUrl, shortCircuit: true };
	}
	if (specifier === "bun:test") {
		return { url: bunTestStubUrl, shortCircuit: true };
	}
	if (specifier.startsWith("bun:")) {
		return { url: bunStubUrl, shortCircuit: true };
	}

	try {
		return await nextResolve(specifier, context);
	} catch (err) {
		if ((err.code === "ERR_MODULE_NOT_FOUND" || err.code === "ERR_UNSUPPORTED_DIR_IMPORT") && context.parentURL) {
			try {
				const parentPath = fileURLToPath(context.parentURL);
				const baseDir = dirname(parentPath);
				const noExt = specifier.endsWith(".js") ? specifier.slice(0, -3) : specifier;
				const candidates = [
					pathResolve(baseDir, specifier + ".ts"),
					pathResolve(baseDir, specifier + ".tsx"),
					pathResolve(baseDir, specifier + ".js"),
					pathResolve(baseDir, noExt + ".ts"),
					pathResolve(baseDir, noExt + ".tsx"),
					pathResolve(baseDir, specifier, "index.ts"),
					pathResolve(baseDir, specifier, "index.js"),
					pathResolve(baseDir, noExt, "index.ts"),
				];
				for (const candidate of candidates) {
					if (fs.existsSync(candidate) && !fs.statSync(candidate).isDirectory()) {
						return {
							url: pathToFileURL(candidate).href,
							shortCircuit: true,
						};
					}
				}
			} catch {}
		}
		throw err;
	}
}

export async function load(url, context, nextLoad) {
	if (url.endsWith(".json")) {
		const content = fs.readFileSync(fileURLToPath(url), "utf8");
		const parsed = JSON.parse(content);
		const namedExports = Object.entries(parsed)
			.filter(([k]) => /^[a-zA-Z_$][a-zA-Z0-9_$]*$/.test(k))
			.map(([k, v]) => `export const ${k} = ${JSON.stringify(v)};`)
			.join("\n");
		return {
			format: "module",
			source: `export default ${content};\n${namedExports}`,
			shortCircuit: true,
		};
	}
	if (url.endsWith(".md") || url.endsWith(".txt") || url.endsWith(".html") || url.endsWith(".css")) {
		const content = fs.readFileSync(fileURLToPath(url), "utf8");
		return {
			format: "module",
			source: `export default ${JSON.stringify(content)};`,
			shortCircuit: true,
		};
	}
	return nextLoad(url, context);
}
