/**
 * Laya Decision Client for Harvest.
 *
 * Communicates with the local Laya decision sidecar over HTTP.
 * Implements strict per-call timeout (~300ms default), non-English detection,
 * batched question execution, temperature-calibrated confidence, and
 * fallback logging.
 */

import * as os from "node:os";
import * as path from "node:path";
import { isRecord, logger } from "@harvest/pi-utils";
import { settings } from "../../config/settings";
import { isBreakerOpen, recordDecisionOutcome } from "./laya-circuit";

export interface LayaQuestionDefinition {
	readonly type: "noul" | "choice" | "score";
	readonly instructions: string;
	readonly criteria?: Record<string, string> | readonly string[];
}

export interface LayaAnswerResult {
	readonly type: string;
	readonly choice?: string;
	readonly answer?: string;
	readonly confidence: number;
	readonly calibratedConfidence?: number;
	readonly noul?: number;
	readonly score?: number;
	readonly action?: Record<string, unknown>;
	readonly probabilities?: Record<string, number>;
}

export interface LayaDecideResponse {
	readonly answers: Record<string, LayaAnswerResult>;
	readonly usage?: {
		readonly input_tokens?: number;
		readonly output_tokens?: number;
	};
	readonly latency_ms: number;
	readonly model: string;
	readonly non_english?: boolean;
}

export interface LayaClientOptions {
	readonly baseUrl?: string;
	readonly timeoutMs?: number;
	readonly autostart?: boolean;
	/** Explicit token overrides LAYA_TOKEN and the local sidecar token file. */
	readonly authToken?: string;
	/** Override the token file path used by a locally managed sidecar. */
	readonly tokenFilePath?: string;
}

export interface DecisionResult<T = LayaAnswerResult> {
	readonly success: boolean;
	readonly fallback: boolean;
	readonly fallbackReason?: string;
	readonly data?: T;
	readonly allAnswers?: Record<string, LayaAnswerResult>;
	readonly latencyMs: number;
}

const DEFAULT_SIDECAR_URL = "http://127.0.0.1:8177";
const DEFAULT_TIMEOUT_MS = 300;

function isProbability(value: unknown): value is number {
	return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 1;
}

/** Reject incomplete or incompatible answers before any agent-loop consumer sees them. */
function isValidDecisionResponse(
	value: unknown,
	questions: Record<string, LayaQuestionDefinition>,
): value is LayaDecideResponse {
	if (
		!isRecord(value) ||
		!isRecord(value.answers) ||
		typeof value.model !== "string" ||
		typeof value.latency_ms !== "number" ||
		!Number.isFinite(value.latency_ms) ||
		value.latency_ms < 0 ||
		(value.non_english !== undefined && typeof value.non_english !== "boolean")
	)
		return false;
	if (value.non_english === true) return true;
	for (const [id, question] of Object.entries(questions)) {
		if (!Object.hasOwn(value.answers, id)) return false;
		const answer = value.answers[id];
		if (
			!isRecord(answer) ||
			answer.type !== question.type ||
			!isProbability(answer.confidence) ||
			(answer.calibratedConfidence !== undefined && !isProbability(answer.calibratedConfidence))
		)
			return false;
		if (question.type === "noul" && !isProbability(answer.noul)) return false;
		if (question.type === "choice") {
			const choices = Array.isArray(question.criteria) ? question.criteria : Object.keys(question.criteria ?? {});
			if (typeof answer.choice !== "string" || !choices.includes(answer.choice)) return false;
		}
		if (question.type === "score") {
			const count = Array.isArray(question.criteria)
				? question.criteria.length
				: Object.keys(question.criteria ?? {}).length;
			if (
				typeof answer.score !== "number" ||
				!Number.isFinite(answer.score) ||
				answer.score < 0 ||
				answer.score > count - 1
			)
				return false;
		}
		if (
			answer.probabilities !== undefined &&
			(!isRecord(answer.probabilities) || !Object.values(answer.probabilities).every(isProbability))
		)
			return false;
	}
	return true;
}

export class LayaClient {
	readonly #baseUrl: string;
	readonly #timeoutMs: number;
	readonly #authToken?: string;
	readonly #tokenFilePath: string;

	constructor(options: LayaClientOptions = {}) {
		let configuredUrl: string | undefined;
		try {
			configuredUrl = settings.get("laya.url");
		} catch {
			// In isolated test environments where settings might not be initialized
		}
		this.#baseUrl = options.baseUrl || process.env.LAYA_SIDECAR_URL || configuredUrl || DEFAULT_SIDECAR_URL;
		const envTimeout = process.env.LAYA_TIMEOUT_MS ? Number.parseInt(process.env.LAYA_TIMEOUT_MS, 10) : undefined;
		this.#timeoutMs =
			options.timeoutMs ?? (envTimeout && !Number.isNaN(envTimeout) ? envTimeout : DEFAULT_TIMEOUT_MS);
		const envToken = process.env.LAYA_TOKEN?.trim();
		this.#authToken = options.authToken ?? (envToken ? envToken : undefined);
		this.#tokenFilePath =
			options.tokenFilePath ?? process.env.LAYA_TOKEN_FILE ?? path.join(os.homedir(), ".harvest", "laya-token");
	}

	get baseUrl(): string {
		return this.#baseUrl;
	}

	get timeoutMs(): number {
		return this.#timeoutMs;
	}

	async #resolveAuthToken(): Promise<string | undefined> {
		if (this.#authToken) return this.#authToken;
		// The managed sidecar binds to loopback. Never send its file token to a
		// configured remote URL, even if that URL was supplied by an environment variable.
		let host: string;
		try {
			host = new URL(this.#baseUrl).hostname;
		} catch {
			return undefined;
		}
		if (host !== "127.0.0.1" && host !== "localhost" && host !== "[::1]") return undefined;
		try {
			// The Python service rotates this token on restart. Read it per call so
			// a long-running Harvest session follows the new process automatically.
			return (await Bun.file(this.#tokenFilePath).text()).trim() || undefined;
		} catch {
			return undefined;
		}
	}

	/**
	 * Fast heuristic to detect non-English input before sending to English-only checkpoint.
	 * Returns false if high ratio of non-ASCII / non-Latin characters or known non-English markers.
	 */
	isEnglish(text: string): boolean {
		if (!text || text.trim().length === 0) return true;

		// Count non-ASCII characters
		let nonAsciiCount = 0;
		for (let i = 0; i < text.length; i++) {
			if (text.charCodeAt(i) > 127) {
				nonAsciiCount++;
			}
		}

		const nonAsciiRatio = nonAsciiCount / text.length;
		// If more than 15% non-ASCII, treat as non-English
		if (nonAsciiRatio > 0.15) {
			return false;
		}

		return true;
	}

	/**
	 * Check health of the sidecar service.
	 */
	async isHealthy(): Promise<boolean> {
		try {
			const controller = new AbortController();
			const timer = setTimeout(() => controller.abort(), Math.max(this.#timeoutMs, 1000));
			const res = await fetch(`${this.#baseUrl}/health`, {
				method: "GET",
				signal: controller.signal,
			});
			clearTimeout(timer);
			if (!res.ok) return false;
			const body = (await res.json()) as { ready?: boolean };
			return body.ready === true;
		} catch {
			return false;
		}
	}

	/**
	 * Retrieve hardware detection result, device details, and signature from sidecar.
	 */
	async getHardwareInfo(): Promise<import("./laya-calibration").HardwareInfo | null> {
		try {
			const controller = new AbortController();
			const timer = setTimeout(() => controller.abort(), Math.max(this.#timeoutMs, 2000));
			const res = await fetch(`${this.#baseUrl}/v1/hardware`, {
				method: "GET",
				signal: controller.signal,
			});
			clearTimeout(timer);
			if (!res.ok) return null;
			return (await res.json()) as import("./laya-calibration").HardwareInfo;
		} catch {
			return null;
		}
	}

	/**
	 * Send batch of typed decision questions against state.
	 *
	 * Optional decision points fail fast while the circuit breaker is open
	 * (no HTTP round trip); required gating and infrastructure calls always
	 * attempt. Every settled outcome feeds per-call-site statistics.
	 */
	async decide(
		state: string | Record<string, unknown> | unknown[],
		questions: Record<string, LayaQuestionDefinition>,
		metadata: {
			callSite: string;
			sessionId?: string;
			timeoutMs?: number;
			signal?: AbortSignal;
			authToken?: string;
		} = { callSite: "unknown" },
	): Promise<DecisionResult<Record<string, LayaAnswerResult>>> {
		if (isBreakerOpen(metadata.callSite)) {
			return {
				success: false,
				fallback: true,
				fallbackReason: "breaker_open_sidecar_overloaded",
				latencyMs: 0,
			};
		}
		const result = await this.#decideInner(state, questions, metadata);
		recordDecisionOutcome(metadata.callSite, {
			success: result.success,
			fallbackReason: result.fallbackReason,
			latencyMs: result.latencyMs,
		});
		return result;
	}

	async #decideInner(
		state: string | Record<string, unknown> | unknown[],
		questions: Record<string, LayaQuestionDefinition>,
		metadata: {
			callSite: string;
			sessionId?: string;
			timeoutMs?: number;
			signal?: AbortSignal;
			authToken?: string;
		},
	): Promise<DecisionResult<Record<string, LayaAnswerResult>>> {
		const startTime = performance.now();

		// Upstream language gating: bypass Laya for non-English inputs
		const stateText = typeof state === "string" ? state : JSON.stringify(state);
		if (!this.isEnglish(stateText)) {
			const latencyMs = performance.now() - startTime;
			logger.info("Laya decision bypassed: non-English input detected", {
				callSite: metadata.callSite,
				sample: stateText.slice(0, 50),
			});
			return {
				success: false,
				fallback: true,
				fallbackReason: "non_english_input",
				latencyMs,
			};
		}

		if (metadata.signal?.aborted) {
			const latencyMs = performance.now() - startTime;
			return {
				success: false,
				fallback: true,
				fallbackReason: "operation_cancelled",
				latencyMs,
			};
		}

		const controller = new AbortController();
		const effectiveTimeout = metadata.timeoutMs ?? this.#timeoutMs;
		const timeoutId = setTimeout(() => {
			controller.abort();
		}, effectiveTimeout);

		const onAbort = () => controller.abort(metadata.signal?.reason);
		if (metadata.signal) {
			metadata.signal.addEventListener("abort", onAbort, { once: true });
		}

		try {
			const headers: Record<string, string> = { "Content-Type": "application/json" };
			const authToken = metadata.authToken ?? (await this.#resolveAuthToken());
			if (authToken) headers.Authorization = `Bearer ${authToken}`;
			const response = await fetch(`${this.#baseUrl}/v1/decide`, {
				method: "POST",
				headers,
				body: JSON.stringify({
					state,
					questions,
					metadata: {
						call_site: metadata.callSite,
						session_id: metadata.sessionId,
						request_timeout_ms: Math.max(1, effectiveTimeout - (performance.now() - startTime)),
					},
				}),
				signal: controller.signal,
			});

			if (!response.ok) {
				const latencyMs = performance.now() - startTime;
				logger.warn("Laya sidecar returned error status", {
					status: response.status,
					callSite: metadata.callSite,
					latencyMs,
				});
				return {
					success: false,
					fallback: true,
					fallbackReason: `http_status_${response.status}`,
					latencyMs,
				};
			}

			const data: unknown = await response.json();
			const latencyMs = performance.now() - startTime;
			if (!isValidDecisionResponse(data, questions)) {
				logger.warn("Laya sidecar returned an invalid decision contract", { callSite: metadata.callSite });
				return { success: false, fallback: true, fallbackReason: "invalid_decision_response", latencyMs };
			}

			if (data.non_english) {
				logger.info("Laya sidecar flagged state as non-English", {
					callSite: metadata.callSite,
				});
				return {
					success: false,
					fallback: true,
					fallbackReason: "non_english_state",
					latencyMs,
				};
			}

			return {
				success: true,
				fallback: false,
				data: data.answers,
				allAnswers: data.answers,
				latencyMs,
			};
		} catch (err) {
			const latencyMs = performance.now() - startTime;
			const isAbort = (err as Error)?.name === "AbortError";
			const wasUserCancelled = metadata.signal?.aborted;
			const reason = wasUserCancelled
				? "operation_cancelled"
				: isAbort
					? `timeout_exceeded_${effectiveTimeout}ms`
					: `connection_error: ${(err as Error)?.message}`;

			logger.warn(`Laya sidecar call failed, triggering fallback [${metadata.callSite}]`, {
				reason,
				latencyMs,
			});

			return {
				success: false,
				fallback: true,
				fallbackReason: reason,
				latencyMs,
			};
		} finally {
			clearTimeout(timeoutId);
			if (metadata.signal) {
				metadata.signal.removeEventListener("abort", onAbort);
			}
		}
	}
}

// Global default singleton
let defaultClient: LayaClient | undefined;

/** Resolve the sidecar URL from explicit arg, env, settings, then default — the same precedence as the constructor. */
function resolveEffectiveBaseUrl(): string {
	let configuredUrl: string | undefined;
	try {
		configuredUrl = settings.get("laya.url");
	} catch {
		// In isolated test environments where settings might not be initialized
	}
	return process.env.LAYA_SIDECAR_URL || configuredUrl || DEFAULT_SIDECAR_URL;
}

export function getLayaClient(baseUrl?: string): LayaClient {
	if (baseUrl) {
		// Explicit callers (setup probing a fallback port, calibration) get a
		// client for exactly that URL without disturbing the shared default.
		return new LayaClient({ baseUrl });
	}
	// Recompute every call: setup may have resolved a port conflict and
	// persisted another `laya.url` since the singleton was built. A stale
	// default would otherwise keep hitting the old port until restart.
	const effective = resolveEffectiveBaseUrl();
	if (!defaultClient || defaultClient.baseUrl !== effective) {
		defaultClient = new LayaClient();
	}
	return defaultClient;
}

/** Drop the shared default client (tests; the next call rebuilds it). */
export function resetLayaClient(): void {
	defaultClient = undefined;
}
