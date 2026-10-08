/**
 * Specialist Roles & Shannon Entropy Intent Routing.
 *
 * Implements Harvest's 6 specialist roles with capability policies,
 * directives, and Shannon entropy routing with ambiguity gating.
 */

export type SpecialistRole = "backend" | "frontend" | "qa" | "coordinator" | "bootstrap" | "compaction";

export type CodingIntent = "informational" | "mutation" | "complex";

export interface RoleDefinition {
	readonly id: SpecialistRole;
	readonly name: string;
	readonly description: string;
	readonly capabilities: readonly string[];
	readonly preApprovedCommands: readonly string[];
	readonly toolPolicy: readonly string[];
	readonly directives: readonly string[];
	readonly promptAddendum: string;
}

export const ROUTING_CONFIDENCE_THRESHOLD = 0.6;

export const SPECIALIST_ROLES: Record<SpecialistRole, RoleDefinition> = {
	backend: {
		id: "backend",
		name: "Backend Specialist",
		description: "Specialized in services, APIs, databases, servers, and business logic persistence",
		capabilities: [
			"REST & GraphQL API design and implementation",
			"Database migrations, queries, and connection pools",
			"Authentication, authorization, and session security",
			"Microservices and asynchronous worker queue design",
			"Data integrity, validation, and performance optimization",
		],
		preApprovedCommands: [
			"npm test",
			"npm run build",
			"cargo test",
			"pytest",
			"go test ./...",
			"make",
			"docker compose",
		],
		toolPolicy: ["read", "write", "edit", "bash", "grep", "glob"],
		directives: [
			"Prioritize strong typing, data integrity, and deterministic error handling.",
			"Handle connection pools, concurrency boundaries, and transaction rollbacks cleanly.",
			"Write narrow, isolated tests defending persistence contracts.",
			"Ensure schema validation at all boundary interfaces.",
		],
		promptAddendum:
			"You are operating as the Harvest Backend Specialist. Prioritize data contracts, persistence safety, concurrency guards, and explicit error types. Follow existing API signatures and test every endpoint change.",
	},
	frontend: {
		id: "frontend",
		name: "Frontend Specialist",
		description: "Specialized in UI components, styling, layout, client state, animations, and kinetic typography",
		capabilities: [
			"Single-file component encapsulation and modular styling",
			"Responsive, accessible UI architectures",
			"Kinetic typography and interaction animations",
			"Client state machines and reactive data stores",
			"Direct mutation without conversational overhead",
		],
		preApprovedCommands: ["npm run build", "npm run dev", "vite build", "npx tailwindcss"],
		toolPolicy: ["read", "write", "edit", "bash", "grep", "glob"],
		directives: [
			"Enforce single-file encapsulation and self-contained styling.",
			"Zero external CDN links — bundle or inline all dependencies.",
			"Implement kinetic typography and fluid interactive transitions.",
			"Direct write tool calls without conversational preambles.",
		],
		promptAddendum:
			"You are operating as the Harvest Frontend Specialist. Enforce single-file encapsulation, zero external CDN dependencies, kinetic typography, and fluid visual interactions. Emit direct write/edit tool calls without introductory commentary.",
	},
	qa: {
		id: "qa",
		name: "QA & Verification Specialist",
		description: "Specialized in test suites, regression replication, assertion contracts, and execution grounding",
		capabilities: [
			"Unit, integration, and end-to-end test authoring",
			"Bug replication and minimal reproducer generation",
			"Test coverage auditing and edge case analysis",
			"Execution grounding and test runner diagnostic parsing",
		],
		preApprovedCommands: ["npm test", "vitest", "jest", "pytest", "cargo test", "playwright test"],
		toolPolicy: ["read", "write", "edit", "bash", "grep", "glob"],
		directives: [
			"Always reproduce the failure with a real test before applying any fix.",
			"Defend external contracts and regression boundaries, not internal echoes.",
			"Verify edge cases: empty inputs, boundary values, error paths.",
			"Never weaken or skip an existing test to pass.",
		],
		promptAddendum:
			"You are operating as the Harvest QA Specialist. Reproduce bugs with targeted tests first. Defend contracts, verify edge cases, and run test suites before accepting any solution.",
	},
	coordinator: {
		id: "coordinator",
		name: "Coordinator Specialist",
		description:
			"Specialized in multi-stage decomposition, delegation, architectural roadmaps, and anti-hallucination verification",
		capabilities: [
			"Task breakdown into small verified checkpoints",
			"Subagent delegation and verification auditing",
			"Cross-cutting architectural design and consensus",
			"Git claim verification and progress tracking",
		],
		preApprovedCommands: ["git status", "git diff", "git log"],
		toolPolicy: ["read", "bash", "task", "ask", "grep", "glob"],
		directives: [
			"Decompose complex requests into sequential verifiable milestones.",
			"Assert mechanical proof (git HEAD advance, test pass) before accepting delegate claims.",
			"Never guess when input is underspecified — clarify early.",
		],
		promptAddendum:
			"You are operating as the Harvest Coordinator. Maintain high-level architectural coherence, verify all delegate outputs against git HEAD and test logs, and keep plans grounded in tool evidence.",
	},
	bootstrap: {
		id: "bootstrap",
		name: "Bootstrap Specialist",
		description: "Specialized in project scaffolding, dependency initialization, and environment configuration",
		capabilities: [
			"Monorepo and project scaffolding",
			"Dependency tree initialization and lockfile generation",
			"Toolchain, formatter, and linter configuration",
			"CI workflow and script bootstrapping",
		],
		preApprovedCommands: ["npm init -y", "cargo new", "git init", "pnpm init"],
		toolPolicy: ["read", "write", "bash", "glob"],
		directives: [
			"Create clean, standard, minimal directory hierarchies.",
			"Generate working configurations for test runners and build tools.",
			"Lock dependencies and pin runtime versions explicitly.",
		],
		promptAddendum:
			"You are operating as the Harvest Bootstrap Specialist. Establish clean idiomatic configurations, scaffold runnable skeletons, and wire build/test scripts cleanly.",
	},
	compaction: {
		id: "compaction",
		name: "Compaction Specialist",
		description:
			"Specialized in lossless session history summarization, verification retention, and context preservation",
		capabilities: [
			"Deterministic file operation auditing (read / write / edit)",
			"Test pass/fail status and staleness tracking",
			"Graph topology node repointing",
			"Token budget condensation",
		],
		preApprovedCommands: [],
		toolPolicy: ["read"],
		directives: [
			"Retain complete lists of read and modified files.",
			"Preserve exact pass/fail records and flag post-test edits as STALE.",
			"Do not invent details not present in the original transcript.",
		],
		promptAddendum:
			"You are operating as the Harvest Compaction Specialist. Deterministically summarize transcript events while preserving exact file paths, test verification outcomes, and graph edges.",
	},
};

const ROLE_KEYWORDS: Record<SpecialistRole, readonly string[]> = {
	backend: [
		"api",
		"backend",
		"server",
		"database",
		"endpoint",
		"sql",
		"db",
		"auth",
		"route",
		"model",
		"service",
		"query",
		"graphql",
		"rest",
		"controller",
		"middleware",
		"migration",
		"handler",
		"redis",
		"postgres",
	],
	frontend: [
		"ui",
		"frontend",
		"component",
		"css",
		"html",
		"style",
		"react",
		"vue",
		"svelte",
		"layout",
		"typography",
		"button",
		"form",
		"dom",
		"page",
		"view",
		"tailwind",
		"animation",
		"responsive",
		"dialog",
		"modal",
	],
	qa: [
		"test",
		"spec",
		"assert",
		"coverage",
		"qa",
		"jest",
		"vitest",
		"pytest",
		"benchmark",
		"verify",
		"regression",
		"e2e",
		"mock",
		"stub",
		"fixture",
	],
	coordinator: [
		"plan",
		"coordinate",
		"orchestrate",
		"subagent",
		"delegate",
		"workflow",
		"architecture",
		"stages",
		"roadmap",
		"review",
		"decompose",
	],
	bootstrap: [
		"init",
		"scaffold",
		"setup",
		"bootstrap",
		"create project",
		"new repo",
		"boilerplate",
		"skeleton",
		"initialize",
	],
	compaction: ["summarize", "compact", "prune", "shrink context", "condense", "compress session"],
};

const MUTATION_VERBS = [
	"add",
	"create",
	"update",
	"fix",
	"refactor",
	"implement",
	"remove",
	"edit",
	"change",
	"rewrite",
	"delete",
	"write",
	"patch",
	"build",
	"modify",
];

const COMPLEX_TRIGGERS = [
	"migrate",
	"design and implement",
	"orchestrate",
	"multi-stage",
	"end-to-end",
	"bootstrap",
	"coordinate",
	"from scratch",
	"complete redesign",
];

const INFORMATIONAL_TRIGGERS = [
	"what is",
	"how does",
	"explain",
	"why",
	"inspect",
	"read",
	"show me",
	"find",
	"where is",
	"describe",
];

/**
 * Classify user prompt intent into informational, mutation, or complex.
 */
export function classifyIntent(promptText: string): CodingIntent {
	const lower = promptText.toLowerCase().trim();

	for (const trigger of COMPLEX_TRIGGERS) {
		if (lower.includes(trigger)) return "complex";
	}

	for (const verb of MUTATION_VERBS) {
		const re = new RegExp(`\\b${verb}\\b`, "i");
		if (re.test(lower)) return "mutation";
	}

	for (const trigger of INFORMATIONAL_TRIGGERS) {
		if (lower.includes(trigger)) return "informational";
	}

	return "mutation";
}

/**
 * Compute normalized Shannon entropy and confidence over score distribution.
 *
 * H(S) = - 1 / log2(k) * sum(p_i * log2(p_i))
 * Confidence C = 1 - H(S)
 */
export function calculateEntropy(
	scores: Record<string, number>,
	totalClasses?: number,
): { entropy: number; confidence: number } {
	const positiveEntries = Object.entries(scores).filter(([, score]) => score > 0);
	const positiveCount = positiveEntries.length;

	if (positiveCount <= 1) {
		return { entropy: 0, confidence: positiveCount === 1 ? 1.0 : 0.0 };
	}

	const sum = positiveEntries.reduce((acc, [, score]) => acc + score, 0);
	if (sum <= 0) {
		return { entropy: 1.0, confidence: 0.0 };
	}

	let h = 0;
	for (const [, score] of positiveEntries) {
		const p = score / sum;
		if (p > 0) {
			h -= p * Math.log2(p);
		}
	}

	// As per spec (Section 4.2): H = -sum(p * log2 p) / log2(N) where N is the
	// dimension of the Score Distribution Vector (5 routing roles).
	const k = totalClasses && totalClasses > 1 ? totalClasses : positiveCount;
	const maxEntropy = Math.log2(k);
	const normalizedEntropy = maxEntropy > 0 ? Math.min(1.0, Math.max(0.0, h / maxEntropy)) : 0;
	const confidence = Math.max(0.0, Math.min(1.0, 1.0 - normalizedEntropy));

	return {
		entropy: normalizedEntropy,
		confidence,
	};
}

export interface RoleRoutingResult {
	readonly role: SpecialistRole;
	readonly confidence: number;
	readonly isAmbiguous: boolean;
	readonly clarificationQuestion?: string;
	readonly topRoles: readonly SpecialistRole[];
	readonly scores: Record<SpecialistRole, number>;
	readonly intent: CodingIntent;
}

/**
 * Route a user prompt to a specialist role using word-boundary keyword hits
 * and Shannon entropy confidence gating.
 */
export function routeRole(promptText: string): RoleRoutingResult {
	const intent = classifyIntent(promptText);
	const scores: Record<SpecialistRole, number> = {
		backend: 0,
		frontend: 0,
		qa: 0,
		coordinator: 0,
		bootstrap: 0,
		compaction: 0,
	};

	const lower = promptText.toLowerCase();

	for (const [role, keywords] of Object.entries(ROLE_KEYWORDS) as Array<[SpecialistRole, readonly string[]]>) {
		let count = 0;
		for (const kw of keywords) {
			const escaped = kw.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
			const re = new RegExp(`\\b${escaped}\\b`, "gi");
			const matches = lower.match(re);
			if (matches) {
				count += matches.length;
			}
		}
		scores[role] = count;
	}

	// Sort roles by score descending
	const sortedRoles = (Object.keys(scores) as SpecialistRole[]).sort((a, b) => scores[b] - scores[a]);
	const highestScore = scores[sortedRoles[0]];

	if (highestScore === 0) {
		// Default to coordinator for complex/informational, backend for mutation
		const defaultRole: SpecialistRole = intent === "complex" ? "coordinator" : "backend";
		return {
			role: defaultRole,
			confidence: 1.0,
			isAmbiguous: false,
			topRoles: [defaultRole],
			scores,
			intent,
		};
	}

	// As per Section 4.2: Score Distribution Vector across the 5 routing roles
	const ROUTING_VECTOR_SIZE = 5;
	const { confidence } = calculateEntropy(scores, ROUTING_VECTOR_SIZE);
	const positiveRoles = sortedRoles.filter(r => scores[r] > 0);
	const isAmbiguous = confidence < ROUTING_CONFIDENCE_THRESHOLD && positiveRoles.length > 1;

	let clarificationQuestion: string | undefined;
	if (isAmbiguous) {
		const topCandidates = positiveRoles.slice(0, 2);
		const candidateNames = topCandidates.map(r => `\`${r}\` (${SPECIALIST_ROLES[r].name})`).join(" vs ");
		clarificationQuestion = `Your request contains multi-domain keywords spanning ${candidateNames}. Which specialist role should lead this task?`;
	}

	return {
		role: sortedRoles[0],
		confidence,
		isAmbiguous,
		clarificationQuestion,
		topRoles: positiveRoles.slice(0, 3),
		scores,
		intent,
	};
}

export function getRoleDefinition(role: SpecialistRole): RoleDefinition {
	return SPECIALIST_ROLES[role] ?? SPECIALIST_ROLES.backend;
}
