/**
 * Session HTTP surface. Mounted by `ManagerServer` only when
 * `META_SESSIONS=mirror|live`. All handlers are additive; existing
 * `/api/runs` routes are untouched.
 */
import type { SessionService } from "./sessions/session-service";
import type { HarnessSessionStatus } from "./sessions/types";

function isStatus(value: string | null): value is HarnessSessionStatus {
	return value === "pass" || value === "fail" || value === "error" || value === "running";
}

export async function handleSessionRoute(
	service: SessionService,
	request: Request,
	url: URL,
): Promise<Response | null> {
	const p = url.pathname;
	if (p === "/api/sessions" && request.method === "GET") {
		const run = url.searchParams.get("run") ?? undefined;
		const statusParam = url.searchParams.get("status");
		const status = statusParam && isStatus(statusParam) ? statusParam : undefined;
		return Response.json(service.list({ run, status }));
	}
	const ensureMatch = p.match(/^\/api\/sessions\/(.+)\/(ensure-live|fork|resume)$/);
	if (ensureMatch && request.method === "POST") {
		const id = decodeURIComponent(ensureMatch[1]);
		const action = ensureMatch[2];
		try {
			if (action === "ensure-live") return Response.json(await service.ensureLive(id), { status: 201 });
			if (action === "fork") return Response.json(await service.fork(id), { status: 201 });
			return Response.json(await service.resume(id), { status: 201 });
		} catch (err) {
			const message = err instanceof Error ? err.message : String(err);
			return Response.json({ error: message }, { status: 400 });
		}
	}
	const oneMatch = p.match(/^\/api\/sessions\/(.+)$/);
	if (oneMatch && request.method === "GET") {
		const id = decodeURIComponent(oneMatch[1]);
		const session = service.get(id);
		if (!session) return Response.json({ error: "session not found" }, { status: 404 });
		return Response.json(session);
	}
	return null;
}
