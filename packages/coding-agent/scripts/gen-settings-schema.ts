/**
 * Emit the settings schema (type, default, enum values, credential marker per
 * path) as JSON for the Rust runtime. Regenerate with
 * `bun run gen:settings-schema` after editing `src/config/settings-schema.ts`.
 */
import * as path from "node:path";
import { isCredential, SETTINGS_SCHEMA, type SettingPath } from "../src/config/settings-schema";

const out: Record<string, { type: string; default?: unknown; values?: readonly string[]; credential?: true }> = {};
for (const key of Object.keys(SETTINGS_SCHEMA).sort() as SettingPath[]) {
	const def = SETTINGS_SCHEMA[key] as { type: string; default?: unknown; values?: readonly string[] };
	out[key] = {
		type: def.type,
		...(def.default !== undefined ? { default: def.default } : {}),
		...(def.values ? { values: def.values } : {}),
		...(isCredential(key) ? { credential: true as const } : {}),
	};
}
const target = path.join(import.meta.dir, "../../../rust-agent/src/settings-schema.json");
await Bun.write(target, `${JSON.stringify(out, null, "\t")}\n`);
console.log(`wrote ${Object.keys(out).length} settings to ${path.relative(process.cwd(), target)}`);
