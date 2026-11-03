/**
 * Emit the compact provider/model routing table the Rust runtime uses to
 * resolve `provider/model` selectors: OpenAI-compatible chat and Anthropic
 * Messages models with concrete (placeholder-free) base URLs.
 * Regenerate with `bun run gen:rust-catalog` after `gen:models`.
 */
import * as path from "node:path";
import models from "../src/models.json" with { type: "json" };
import { CATALOG_PROVIDERS } from "../src/provider-models/descriptors";

type Entry = { api: string; baseUrl: string; contextWindow?: number; maxTokens?: number };
const envVars = new Map<string, readonly string[]>();
for (const provider of CATALOG_PROVIDERS) if (provider.envVars?.length) envVars.set(provider.id, provider.envVars);

const WIRES: Record<string, string | undefined> = { "openai-completions": undefined, "anthropic-messages": "anthropic" };
type Row = { baseUrl: string; wire?: string; contextWindow?: number; maxTokens?: number };
const out: Record<string, { envVars: readonly string[]; models: Record<string, Row> }> = {};
for (const provider of Object.keys(models).sort()) {
	const table = (models as Record<string, Record<string, Entry>>)[provider];
	const rows: (typeof out)[string]["models"] = {};
	for (const id of Object.keys(table).sort()) {
		const model = table[id];
		if (!(model.api in WIRES) || !model.baseUrl || /[{<]/.test(model.baseUrl)) continue;
		const wire = WIRES[model.api];
		rows[id] = { baseUrl: model.baseUrl, ...(wire ? { wire } : {}), contextWindow: model.contextWindow, maxTokens: model.maxTokens };
	}
	if (Object.keys(rows).length) out[provider] = { envVars: envVars.get(provider) ?? [], models: rows };
}
const target = path.join(import.meta.dir, "../../../rust-agent/src/catalog.json");
await Bun.write(target, `${JSON.stringify(out)}\n`);
console.log(`wrote ${Object.keys(out).length} providers to ${path.relative(process.cwd(), target)}`);
