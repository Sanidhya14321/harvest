/**
 * Laya Decision Client for Harvest.
 *
 * Communicates with the local Laya decision sidecar over HTTP.
 * Implements strict per-call timeout (~300ms default), non-English detection,
 * batched question execution, temperature-calibrated confidence, and
 * fallback logging.
 */

import { logger } from "@harvest/pi-utils";
import { settings } from "../../config/settings";

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

export class LayaClient {
	readonly #baseUrl: string;
	readonly #timeoutMs: number;

	constructor(options: LayaClientOptions = {}) {
		let configuredUrl: string | undefined;
		try {
			configuredUrl = settings.get("laya.url");
		} catch {
			// In isolated test environments where settings might not be initialized
		}
		this.#baseUrl = options.baseUrl || process.env.LAYA_SIDECAR_URL || configuredUrl || DEFAULT_SIDECAR_URL;
		const envTimeout = process.env.LAYA_TIMEOUT_MS ? Number.parseInt(process.env.LAYA_TIMEOUT_MS, 10) : undefined;
		this.#timeoutMs = options.timeoutMs ?? (envTimeout && !Number.isNaN(envTimeout) ? envTimeout : DEFAULT_TIMEOUT_MS);
	}

	get baseUrl(): string {
		return this.#baseUrl;
	}

	get timeoutMs(): number {
		return this.#timeoutMs;
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
	 */
	async decide(
		state: string | Record<string, unknown> | unknown[],
		questions: Record<string, LayaQuestionDefinition>,
		metadata: { callSite: string; sessionId?: string; timeoutMs?: number; signal?: AbortSignal } = { callSite: "unknown" },
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
			const response = await fetch(`${this.#baseUrl}/v1/decide`, {
				method: "POST",
				headers: {
					"Content-Type": "application/json",
				},
				body: JSON.stringify({
					state,
					questions,
					metadata: {
						call_site: metadata.callSite,
						session_id: metadata.sessionId,
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

			const data = (await response.json()) as LayaDecideResponse;
			const latencyMs = performance.now() - startTime;

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

export function getLayaClient(): LayaClient {
	if (!defaultClient) {
		defaultClient = new LayaClient();
	}
	return defaultClient;
}
