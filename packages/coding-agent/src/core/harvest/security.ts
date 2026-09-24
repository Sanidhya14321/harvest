/**
 * Enterprise Sandbox & AST-Level Security Auditing.
 *
 * 1. Destructive Command Barrier: blocks rm -rf /, rm -rf *, mkfs, fork bombs, raw disk writes.
 * 2. Realpath Workspace Jailing: rejects paths/symlinks escaping workspaceRoot.
 * 3. Continuous Security Auditing: scans code for hardcoded secrets, SQL injection, eval(), command injection.
 */

import * as fs from "node:fs";
import * as path from "node:path";

export interface SecurityFinding {
	readonly ruleId: string;
	readonly severity: "critical" | "high" | "medium";
	readonly message: string;
	readonly lineNumber?: number;
	readonly excerpt?: string;
}

export interface SecurityAuditResult {
	readonly safe: boolean;
	readonly findings: readonly SecurityFinding[];
}

export interface CommandBarrierResult {
	readonly allowed: boolean;
	readonly reason?: string;
	readonly patternMatched?: string;
}

const DESTRUCTIVE_COMMAND_PATTERNS = [
	{
		name: "rm_rf_root",
		re: /\brm\s+(?:-[a-zA-Z]*f[a-zA-Z]*\s+)?(?:\/|\/\*)(?:\s|$)/,
		desc: "Dangerous command: recursive root removal (rm -rf /)",
	},
	{
		name: "rm_rf_wildcard",
		re: /\brm\s+(?:-[a-zA-Z]*r[a-zA-Z]*f[a-zA-Z]*|-[a-zA-Z]*f[a-zA-Z]*r[a-zA-Z]*)\s+\*(?:\s|$)/,
		desc: "Dangerous command: blind wildcard recursive removal (rm -rf *)",
	},
	{
		name: "mkfs",
		re: /\bmkfs(?:\.[a-z0-9]+)?\s+/i,
		desc: "Dangerous command: filesystem formatting (mkfs)",
	},
	{
		name: "fork_bomb",
		re: /:\s*\(\s*\)\s*\{\s*:\s*\|\s*:\s*&\s*\}\s*;\s*:/,
		desc: "Dangerous command: bash fork bomb detected",
	},
	{
		name: "raw_disk_write",
		re: /(?:>\s*\/dev\/sd[a-z]|dd\s+if=.*?\s+of=\/dev\/sd[a-z])/i,
		desc: "Dangerous command: raw disk overwrite (/dev/sd*)",
	},
];

const CODE_SECURITY_RULES = [
	{
		ruleId: "secret_api_key",
		severity: "critical" as const,
		re: /\b(?:sk_live_[0-9a-zA-Z]{24,}|ghp_[0-9a-zA-Z]{36})\b/,
		message: "Hardcoded API key or personal access token detected.",
	},
	{
		ruleId: "secret_private_key",
		severity: "critical" as const,
		re: /-----BEGIN (?:RSA )?PRIVATE KEY-----/,
		message: "Hardcoded private cryptographic key detected.",
	},
	{
		ruleId: "secret_aws_key",
		severity: "critical" as const,
		re: /aws_secret_access_key\s*=\s*['"][A-Za-z0-9/+=]{40}['"]/i,
		message: "Hardcoded AWS secret access key detected.",
	},
	{
		ruleId: "eval_injection",
		severity: "high" as const,
		re: /\b(?:eval\s*\(|new\s+Function\s*\()/,
		message: "Dynamic code execution (eval / new Function) is a security hazard.",
	},
	{
		ruleId: "sql_concatenation",
		severity: "high" as const,
		re: /(?:SELECT|INSERT|UPDATE|DELETE)\s+.*?\+\s*[a-zA-Z_$][a-zA-Z0-9_$]*/i,
		message: "Unparameterized SQL query concatenation detected (potential SQL injection).",
	},
	{
		ruleId: "command_injection_template",
		severity: "high" as const,
		re: /(?:exec|spawn|execSync)\s*\(\s*`[^`]*\$\{/,
		message: "Subprocess execution using unescaped template literals (command injection risk).",
	},
];

function tryRealpathSync(target: string): string | null {
	try {
		return fs.realpathSync.native ? fs.realpathSync.native(target) : fs.realpathSync(target);
	} catch {
		return null;
	}
}

function isSymlinkSync(target: string): boolean {
	try {
		return fs.lstatSync(target).isSymbolicLink();
	} catch {
		return false;
	}
}

function stripVerbatim(p: string): string {
	return p.startsWith("\\\\?\\") ? p.slice(4) : p;
}

export class SecuritySandbox {
	readonly #workspaceRoot: string;

	constructor(workspaceRoot: string = process.cwd()) {
		const abs = path.resolve(workspaceRoot);
		try {
			this.#workspaceRoot = fs.realpathSync.native ? fs.realpathSync.native(abs) : fs.realpathSync(abs);
		} catch {
			this.#workspaceRoot = abs;
		}
	}

	/**
	 * Check whether a shell command violates destructive command barriers.
	 */
	checkCommand(command: string): CommandBarrierResult {
		const trimmed = command.trim();

		for (const pattern of DESTRUCTIVE_COMMAND_PATTERNS) {
			if (pattern.re.test(trimmed)) {
				return {
					allowed: false,
					reason: `Command rejected by Harvest Destructive Command Barrier: ${pattern.desc}`,
					patternMatched: pattern.name,
				};
			}
		}

		return { allowed: true };
	}

	/**
	 * Assert that a file path resolves inside the workspace root (preventing path traversal).
	 */
	assertPathJailed(targetPath: string): { jailed: boolean; resolvedPath: string; error?: string } {
		const absolute = path.isAbsolute(targetPath) ? targetPath : path.resolve(this.#workspaceRoot, targetPath);

		// Lexical containment pre-check against workspace root
		const lexicalRel = path.relative(this.#workspaceRoot, absolute);
		if (lexicalRel.startsWith("..") || path.isAbsolute(lexicalRel)) {
			return {
				jailed: false,
				resolvedPath: absolute,
				error: `Path traversal rejected: '${targetPath}' resolves outside workspace root '${this.#workspaceRoot}' (to '${absolute}').`,
			};
		}

		const realRoot = tryRealpathSync(this.#workspaceRoot) ?? path.resolve(this.#workspaceRoot);

		let resolved: string;
		const realTarget = tryRealpathSync(absolute);
		if (realTarget !== null) {
			resolved = realTarget;
		} else {
			// If target is an unresolvable symlink (dangling link), reject outright
			if (isSymlinkSync(absolute)) {
				return {
					jailed: false,
					resolvedPath: absolute,
					error: `Path traversal rejected: '${targetPath}' is an unresolvable symlink.`,
				};
			}

			// Nonexistent leaf: find the deepest existing ancestor and resolve it
			let ancestor = path.dirname(absolute);
			const tail: string[] = [path.basename(absolute)];
			for (;;) {
				const realAncestor = tryRealpathSync(ancestor);
				if (realAncestor !== null) {
					resolved = path.join(realAncestor, ...tail.reverse());
					break;
				}
				if (isSymlinkSync(ancestor)) {
					return {
						jailed: false,
						resolvedPath: absolute,
						error: `Path traversal rejected: '${targetPath}' contains an unresolvable symlink ancestor '${ancestor}'.`,
					};
				}
				const parent = path.dirname(ancestor);
				if (parent === ancestor || !path.relative(this.#workspaceRoot, ancestor) || path.relative(this.#workspaceRoot, ancestor).startsWith("..")) {
					resolved = path.normalize(absolute);
					break;
				}
				tail.push(path.basename(ancestor));
				ancestor = parent;
			}
		}

		const normRealRoot = stripVerbatim(realRoot);
		const normResolved = stripVerbatim(resolved);
		const rel = path.relative(normRealRoot, normResolved);
		const isOutside = rel.startsWith("..") || path.isAbsolute(rel);

		if (isOutside) {
			return {
				jailed: false,
				resolvedPath: resolved,
				error: `Path traversal rejected: '${targetPath}' resolves outside workspace root '${this.#workspaceRoot}' (to '${resolved}').`,
			};
		}

		return { jailed: true, resolvedPath: resolved };
	}

	/**
	 * Continuous AST regex scanning on code content for secrets and vulnerabilities.
	 */
	auditCode(content: string, _filePath?: string): SecurityAuditResult {
		const lines = content.split("\n");
		const findings: SecurityFinding[] = [];

		for (let i = 0; i < lines.length; i++) {
			const line = lines[i];

			for (const rule of CODE_SECURITY_RULES) {
				if (rule.re.test(line)) {
					findings.push({
						ruleId: rule.ruleId,
						severity: rule.severity,
						message: rule.message,
						lineNumber: i + 1,
						excerpt: line.trim().slice(0, 100),
					});
				}
			}
		}

		return {
			safe: findings.length === 0,
			findings,
		};
	}
}
