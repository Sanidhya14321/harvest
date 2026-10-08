# Provider integration review — 2026-10-08

This secondary review inspected the local Claude/Anthropic, OpenAI API-key, and ChatGPT/Codex OAuth integrations while the UI repair continued. The provider layer is wired, and **115 existing mocked/local tests passed with zero failures**. Four integration gaps are recorded below. These results establish local behavior; live account login, entitlements, and inference were not tested.

The audit did not change provider implementation or user configuration, print real credentials, call live provider inference, or capture screenshots. Custom reproductions used fake credentials, replaced provider fetches with mocks, and used `:memory:` auth/cache databases. Existing tests used their own temporary fixtures.

## Supported local paths

| Path | Local evidence |
| --- | --- |
| Anthropic API key → Messages | The catalog declares `ANTHROPIC_API_KEY`; official API requests use `X-Api-Key`. The client tests cover request assembly, retryable statuses, cancellation, timeouts, and bounded error bodies. See [descriptors.ts:134](../packages/catalog/src/provider-models/descriptors.ts#L134), [anthropic.ts:364](../packages/ai/src/providers/anthropic.ts#L364), and [anthropic-client.test.ts](../packages/ai/test/anthropic-client.test.ts). |
| Claude OAuth → Messages | KDL declares browser authorization, PKCE, token exchange, and refresh. Tests cover exchange, refresh headers, bootstrap identity, and OAuth header shaping. See [auth/anthropic.kdl](../packages/catalog/src/compat/rules/auth/anthropic.kdl), [anthropic.ts:335](../packages/ai/src/providers/anthropic.ts#L335), and [anthropic-oauth.test.ts](../packages/ai/test/anthropic-oauth.test.ts). |
| OpenAI API key → Responses | `openai` declares `OPENAI_API_KEY` and uses the Responses transport. Local tests cover terminal status mapping, usage, streamed argument completion, and recovery of omitted events. See [descriptors.ts:378](../packages/catalog/src/provider-models/descriptors.ts#L378), [openai-responses.ts:455](../packages/ai/src/providers/openai-responses.ts#L455), and [openai-responses-stream-terminal.test.ts](../packages/ai/test/openai-responses-stream-terminal.test.ts). |
| ChatGPT OAuth → Codex | Browser/device login declarations store credentials under `openai-codex`. Local tests cover authorization URL fields, tool/request shaping, multi-account discovery, SSE completion/truncation, and WebSocket-to-SSE fallback. See [auth/openai-codex.kdl](../packages/catalog/src/compat/rules/auth/openai-codex.kdl), [auth/openai-codex-device.kdl](../packages/catalog/src/compat/rules/auth/openai-codex-device.kdl), and [openai-codex-stream.test.ts](../packages/ai/test/openai-codex-stream.test.ts). |
| CLI/SDK selection → request auth | Startup checks configured auth without refreshing tokens; actual requests resolve auth through the registry unless the SDK supplies a resolver. Runtime/config keys take precedence over stored OAuth credentials. See [sdk.ts:1502](../packages/coding-agent/src/sdk.ts#L1502), [sdk.ts:3623](../packages/coding-agent/src/sdk.ts#L3623), and [auth-storage.ts:5939](../packages/ai/src/auth-storage.ts#L5939). |

## Findings

### 1. [P1] Codex discovery ignores a configured endpoint

The special Codex discovery descriptor does not pass the provider endpoint or headers into its manager. The manager configuration does not expose those options and calls `fetchCodexModels` without them. Consequently, discovery uses the default ChatGPT backend even when inference has been redirected to a custom gateway.

Evidence: [model-registry.ts:1829](../packages/coding-agent/src/config/model-registry.ts#L1829), [special.ts:40](../packages/catalog/src/provider-models/special.ts#L40), and [special.ts:60](../packages/catalog/src/provider-models/special.ts#L60). Other built-in descriptor paths resolve overrides through [model-registry.ts:954](../packages/coding-agent/src/config/model-registry.ts#L954).

**Confirmed fake reproduction:** construct `ModelRegistry` with an injected fetch recorder and an in-memory cache; register `openai-codex` with `baseUrl: "https://audit-proxy.invalid/backend-api"` and `apiKey: "audit-fake-gateway"`; call `refreshProvider("openai-codex", "online")`. The selected model retained the proxy URL, but the recorded discovery request targeted:

```text
https://chatgpt.com/backend-api/codex/models?client_version=0.153.0
```

Its authorization header contained the fake gateway credential. No request left the mocked transport. With a real override, this routes a custom credential to the wrong host and can discover the wrong catalog.

### 2. [P2] Cancelled Codex device login can still store credentials

Device authorization initialization and polling use global `fetch`, ignoring `OAuthController.fetch`. Their signals contain only a timeout. Cancellation is checked after polling sleeps, but not after a successful poll or before the final exchange; the exchange also uses a timeout-only signal. `AuthStorage.login` subsequently persists the returned credential without a final cancellation check.

Evidence: [openai-codex.ts:217](../packages/ai/src/registry/oauth/openai-codex.ts#L217), [openai-codex.ts:257](../packages/ai/src/registry/oauth/openai-codex.ts#L257), [openai-codex.ts:263](../packages/ai/src/registry/oauth/openai-codex.ts#L263), [openai-codex.ts:297](../packages/ai/src/registry/oauth/openai-codex.ts#L297), and [auth-storage.ts:3180](../packages/ai/src/auth-storage.ts#L3180).

**Confirmed fake reproductions:**

- A pre-aborted controller still caused one global fetch attempt; its injected fetch received zero calls. The global mock rejected immediately, so no network request occurred.
- Replace global fetch and `Bun.sleep` within an isolated process. Return fake device initialization, abort the controller while returning a successful polling response, then return fake token-exchange credentials. `AuthStorage.login("openai-codex-device", ...)` made three mocked requests and stored an OAuth credential despite the aborted controller. Global functions were restored afterward; the auth database was in memory.

### 3. [P2] API-key spending can be labelled as subscription usage

`ModelRegistry.isUsingOAuth` checks whether OAuth credentials exist for the provider, rather than which credential authenticates the request. Runtime/config API-key overrides win request resolution but leave that classification true. Footer billing and advisor attribution consume the classification, so paid API-key activity can appear as subscription usage. An environment-supplied OAuth token without stored OAuth credentials can produce the inverse classification.

Evidence: [model-registry.ts:2521](../packages/coding-agent/src/config/model-registry.ts#L2521), [auth-storage.ts:3006](../packages/ai/src/auth-storage.ts#L3006), [auth-storage.ts:5951](../packages/ai/src/auth-storage.ts#L5951), [footer.ts:215](../packages/coding-agent/src/modes/components/footer.ts#L215), and [session-advisors.ts:1323](../packages/coding-agent/src/session/session-advisors.ts#L1323).

**Confirmed fake reproduction:** seed an unexpired fake Anthropic OAuth credential in memory, then set a fake config API key. `getApiKey("anthropic")` used the config key while `hasOAuth("anthropic")` remained true. The inverse environment-token case is supported by the classification code and was not separately reproduced.

### 4. Latent Codex workspace identity mismatch

OAuth login accepts workspace identity from an `id_token` when the access token lacks it; refresh can also retain a stored `accountId`. The Codex inference transport re-derives identity only from the access-token JWT, so the identity accepted and preserved by auth storage can disappear before request header construction.

Evidence: [openai-codex.ts:65](../packages/ai/src/registry/oauth/openai-codex.ts#L65), [openai-codex.ts:99](../packages/ai/src/registry/oauth/openai-codex.ts#L99), [openai-codex-responses.ts:1440](../packages/ai/src/providers/openai-codex-responses.ts#L1440), and [openai-codex-responses.ts:4423](../packages/ai/src/providers/openai-codex-responses.ts#L4423). The SDK's default request resolver returns key bytes rather than OAuth identity metadata.

**Confirmed fake reproduction:** invoke the profile hook with an opaque fake access token and a fake `id_token` carrying a workspace claim. The hook accepted and returned the workspace identity, while `getCodexAccountId(credentials.access)` returned `undefined`. A request without an explicit account header consequently lacks the derived header. The effect on currently issued live token formats and backend account routing remains unverified.

## Existing test commands and results

Run from the repository root:

```powershell
bun test packages/ai/test/anthropic-oauth.test.ts packages/ai/test/anthropic-client.test.ts packages/ai/test/openai-codex.test.ts packages/ai/test/openai-responses-stream-terminal.test.ts packages/ai/test/auth-storage-config-override.test.ts packages/catalog/test/codex-discovery.test.ts
```

Result: **111 passed, 0 failed, 302 assertions across 6 files**.

```powershell
bun test packages/ai/test/openai-codex-stream.test.ts --test-name-pattern 'streams SSE responses into AssistantMessageEventStream|fails truncated SSE streams that never emit a terminal response event|stops reading SSE responses after a terminal response event|falls back to SSE when websocket connect fails|does not fall back to SSE'
```

Result: **4 passed, 0 failed, 22 assertions, 92 filtered out**. The four matching tests cover SSE normalization, truncated-stream failure, termination without waiting for connection closure, and fallback after WebSocket connection failure.

Total: **115 passed, 0 failed**. The custom reproductions above were additional local probes, not added regression tests.

## Follow-up implementation checklist

- [ ] **Discovery routing:** carry the resolved endpoint and live provider headers through the Codex manager. Regression contract: overriding the Codex endpoint/key sends catalog requests exclusively to that endpoint with the configured auth/header policy, while default configuration still uses the default backend. Preserve multi-account catalog union and failure fallback behavior.
- [ ] **Device login cancellation:** use the injected transport, combine caller cancellation with request timeouts, make polling waits cancellable, and guard exchange/persistence. Regression contracts: a pre-aborted login issues no request; cancellation during initialization, polling, or exchange prevents credential storage; injected fetch handles every device-flow request.
- [ ] **Subscription attribution:** derive billing attribution from the credential actually selected, including runtime/config overrides and session fallback. Regression contracts: API-key overrides display/API-attribute paid usage despite stored OAuth; actual OAuth activity retains subscription attribution; advisor attribution survives runtime teardown.
- [ ] **Workspace identity:** retain resolved OAuth account metadata through normal Codex request auth and transport selection. Regression contract: an account identity accepted from `id_token` or preserved through refresh reaches the request header even when the access token lacks that claim; opaque custom gateway credentials do not inherit an unrelated stored OAuth account header.

## Live-account limits

No real browser/device login, token refresh, account entitlement, current model roster, quota, provider fingerprint acceptance, or inference request was verified. The review establishes local wiring, mocked request/response contracts, and reproducible local defects; it does not certify end-to-end operation for a particular live account.
