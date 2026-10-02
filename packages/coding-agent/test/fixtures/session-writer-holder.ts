import { SessionManager } from "../../src/session/session-manager";

const [file, ready] = process.argv.slice(2);
if (!file || !ready) throw new Error("Expected session and readiness paths");
const manager = await SessionManager.open(file, undefined, undefined, { suppressBreadcrumb: true });
manager.appendMessage({ role: "user", content: "child committed", timestamp: Date.now() });
await manager.flush();
await Bun.write(ready, "ready");
await Bun.sleep(120_000);
await manager.close();
