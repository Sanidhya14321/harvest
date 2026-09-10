/**
 * Pre-Read Enforcement & Blind Overwrite Shield.
 *
 * Enforces the invariant that an agent cannot mutate an existing file
 * unless that file has been inspected via the read tool in the current session.
 */

import * as fs from "node:fs";
import * as path from "node:path";

export interface ToolCallPayload {
	readonly name: string;
	readonly args: Record<string, unknown>;
}

export interface MutationCheckResult {
	readonly allowed: boolean;
	readonly reason?: string;
	readonly canonicalPath?: string;
}

export class PreReadEnforcement {
	readonly #readPaths: Set<string> = new Set<string>();
	readonly #workspaceRoot: string;

	constructor(workspaceRoot: string = process.cwd()) {
		this.#workspaceRoot = path.resolve(workspaceRoot);
	}

	/** Canonicalize path, resolving symlinks and casing where possible. */
	canonicalizePath(rawPath: string): string {
		const absolute = path.isAbsolute(rawPath) ? rawPath : path.resolve(this.#workspaceRoot, rawPath);
		try {
			// realpathSync resolves symlinks and canonical casing on Windows/macOS
			return fs.realpathSync.native ? fs.realpathSync.native(absolute) : fs.realpathSync(absolute);
		} catch {
			// If file does not exist, normalize standard path
			return path.normalize(absolute);
		}
	}

	/** Record that a file has been read in the current session. */
	recordRead(filePath: string): void {
		const canonical = this.canonicalizePath(filePath);
		this.#readPaths.add(canonical);
	}

	/** Check if a file has been read in the current session. */
	hasRead(filePath: string): boolean {
		const canonical = this.canonicalizePath(filePath);
		return this.#readPaths.has(canonical);
	}

	/** Get relative path formatted for display. */
	#formatDisplayPath(targetPath: string): string {
		const rel = path.relative(this.#workspaceRoot, targetPath);
		return rel && !rel.startsWith("..") ? rel.replace(/\\/g, "/") : targetPath.replace(/\\/g, "/");
	}

	/**
	 * Check whether a tool call attempting mutation is permitted under pre-read enforcement.
	 */
	checkMutationAllowed(toolName: string, targetPath: string): MutationCheckResult {
		const canonical = this.canonicalizePath(targetPath);
		const displayPath = this.#formatDisplayPath(canonical);

		// Check if file currently exists on disk
		let exists = false;
		try {
			exists = fs.existsSync(canonical) && fs.statSync(canonical).isFile();
		} catch {
			exists = false;
		}

		if (toolName === "edit") {
			if (!exists) {
				return {
					allowed: false,
					reason: `Cannot edit '${displayPath}': file does not exist on disk. Use write to create new files.`,
					canonicalPath: canonical,
				};
			}
			if (!this.#readPaths.has(canonical)) {
				return {
					allowed: false,
					reason: `Blocked edit on '${displayPath}': this file exists but has not been read in this session. Read it first with the read tool, then retry the edit. Editing or overwriting code you have not read risks destroying existing behavior.`,
					canonicalPath: canonical,
				};
			}
			return { allowed: true, canonicalPath: canonical };
		}

		if (toolName === "write") {
			if (exists && !this.#readPaths.has(canonical)) {
				return {
					allowed: false,
					reason: `Blocked write on '${displayPath}': this file exists but has not been read in this session. Read it first with the read tool, then retry the edit. Editing or overwriting code you have not read risks destroying existing behavior.`,
					canonicalPath: canonical,
				};
			}
			return { allowed: true, canonicalPath: canonical };
		}

		return { allowed: true, canonicalPath: canonical };
	}

	/**
	 * Intercept a generic tool call payload.
	 * If the call is read, records the read path.
	 * If the call is write or edit, enforces pre-read check.
	 */
	interceptToolCall(toolCall: ToolCallPayload): { allowed: boolean; error?: string } {
		const name = toolCall.name.toLowerCase();
		const args = toolCall.args ?? {};

		const rawPath =
			(typeof args.path === "string" && args.path) ||
			(typeof args.filePath === "string" && args.filePath) ||
			(typeof args.targetFile === "string" && args.targetFile) ||
			(typeof args.TargetFile === "string" && args.TargetFile) ||
			(typeof args.target_file === "string" && args.target_file) ||
			(typeof args.file === "string" && args.file) ||
			(typeof args.file_path === "string" && args.file_path) ||
			(typeof args.AbsolutePath === "string" && args.AbsolutePath) ||
			null;

		if (!rawPath) {
			return { allowed: true };
		}

		if (name === "read" || name === "view_file" || name === "read_file") {
			this.recordRead(rawPath);
			return { allowed: true };
		}

		if (
			name === "edit" ||
			name === "write" ||
			name === "replace_file_content" ||
			name === "write_to_file" ||
			name === "ast-edit" ||
			name === "ast_edit"
		) {
			const mutationTool = name.includes("write") ? "write" : "edit";
			const check = this.checkMutationAllowed(mutationTool, rawPath);
			if (!check.allowed) {
				return { allowed: false, error: check.reason };
			}
		}

		return { allowed: true };
	}

	clear(): void {
		this.#readPaths.clear();
	}
}
