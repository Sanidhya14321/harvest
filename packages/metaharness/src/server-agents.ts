/**
 * Agent HTTP surface. Mounted by `ManagerServer` only when `META_AGENTS=on`.
 * Additive only; benchmark runner ownership stays in `ManagerServer`.
 */
import { assertSafeAgentId, type AgentService, type SpawnAgentRequest } from "./agents/agent-service";
import type { HarnessAgentStatus } from "./agents/types";

function isStatus(value: string | null): value is HarnessAgentStatus {
	return value === "running" || value === "idle" || value === "parked" || value === "aborted";
}

export async function handleAgentRoute(service: AgentService, request: Request, url: URL): Promise<Response | null> {
	const p = url.pathname;
	if (p === "/api/agents" && request.method === "GET") {
		const run = url.searchParams.get("run") ?? undefined;
		const statusParam = url.searchParams.get("status");
		const status = statusParam && isStatus(statusParam) ? statusParam : undefined;
		return Response.json(service.list({ run, status }));
	}
	if (p === "/api/agents" && request.method === "POST") {
		const body = (await request.json()) as Partial<SpawnAgentRequest>;
		if (!body.run || typeof body.run !== "string") {
			return Response.json({ error: "run is required" }, { status: 400 });
		}
		try {
			const agent = await service.spawn({
				run: body.run,
				trial: typeof body.trial === "string" ? body.trial : undefined,
				displayName: typeof body.displayName === "string" ? body.displayName : undefined,
				kind: body.kind === "benchmark-runner" ? "benchmark-runner" : "trial-worker",
				command: Array.isArray(body.command)
					? body.command.filter((c): c is string => typeof c === "string")
					: undefined,
				cwd: typeof body.cwd === "string" ? body.cwd : undefined,
			});
			return Response.json(agent, { status: 201 });
		} catch (err) {
			const message = err instanceof Error ? err.message : String(err);
			return Response.json({ error: message }, { status: 400 });
		}
	}
	const actionMatch = p.match(/^\/api\/agents\/(.+)\/(cancel|park|revive)$/);
	if (actionMatch && request.method === "POST") {
		const id = decodeURIComponent(actionMatch[1]);
		try {
			assertSafeAgentId(id);
		} catch (err) {
			const message = err instanceof Error ? err.message : String(err);
			return Response.json({ error: message }, { status: 400 });
		}
		try {
			const action = actionMatch[2];
			if (action === "cancel") {
				const cancelled = await service.cancel(id);
				if (!cancelled) return Response.json({ error: "agent not found" }, { status: 404 });
				return Response.json({ id, cancelled: true });
			}
			if (action === "park") return Response.json(await service.park(id));
			return Response.json(await service.revive(id));
		} catch (err) {
			const message = err instanceof Error ? err.message : String(err);
			return Response.json({ error: message }, { status: 400 });
		}
	}
	const oneMatch = p.match(/^\/api\/agents\/(.+)$/);
	if (oneMatch && request.method === "GET") {
		const id = decodeURIComponent(oneMatch[1]);
		const agent = service.get(id);
		if (!agent) return Response.json({ error: "agent not found" }, { status: 404 });
		return Response.json(agent);
	}
	return null;
}
