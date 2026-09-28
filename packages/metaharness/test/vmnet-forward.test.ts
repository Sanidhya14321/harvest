import { describe, expect, it } from "bun:test";
import { BENCH_DEFAULT_GATEWAY_TOKEN, isVmnetForwardAuthorized } from "../src/runner";

/**
 * Contracts under test:
 *  - With the default "no-auth" bench token the vmnet forward stays open so
 *    bench containers keep working (documented bench trust boundary).
 *  - With an explicit token, Authorization-less (or wrong-token) requests are
 *    NOT forwarded to the non-loopback bridge address.
 */

function reqWithAuth(header: string | null): Request {
	const headers = new Headers();
	if (header !== null) headers.set("authorization", header);
	return new Request("http://192.168.64.1:4000/v1/models", { headers });
}

describe("vmnet gateway forward auth", () => {
	it("forwards everything with the default bench token (bench trust boundary)", () => {
		expect(isVmnetForwardAuthorized(reqWithAuth(null), BENCH_DEFAULT_GATEWAY_TOKEN)).toBe(true);
		expect(isVmnetForwardAuthorized(reqWithAuth("Bearer whatever"), BENCH_DEFAULT_GATEWAY_TOKEN)).toBe(true);
	});

	it("refuses Authorization-less requests when an explicit token is configured", () => {
		expect(isVmnetForwardAuthorized(reqWithAuth(null), "operator-secret")).toBe(false);
	});

	it("refuses a wrong bearer token when an explicit token is configured", () => {
		expect(isVmnetForwardAuthorized(reqWithAuth("Bearer wrong"), "operator-secret")).toBe(false);
		expect(isVmnetForwardAuthorized(reqWithAuth("Token operator-secret"), "operator-secret")).toBe(false);
	});

	it("forwards the matching bearer token when an explicit token is configured", () => {
		expect(isVmnetForwardAuthorized(reqWithAuth("Bearer operator-secret"), "operator-secret")).toBe(true);
	});
});
