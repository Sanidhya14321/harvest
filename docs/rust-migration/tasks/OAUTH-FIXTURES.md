# control:oauth-fixtures — capture failures at the current refresh boundary

Prerequisite: `control:playbook`. Source:
`packages/ai/test/auth-storage-oauth-refresh-race.test.ts` remains owned legacy
test code; this repair does not mark that source or any provider migrated.

The broad transition gate exposed two repeatable failures, also reproduced in
an isolated serial file run. The fixtures mocked `getOAuthApiKey`, but current
AuthStorage refreshes expired credentials earlier through its per-instance
`refreshOAuthCredential` seam. Consequently those mocks did not create the
intended definitive refresh failure and compare-and-swap race.

Contract/destination: preserve every assertion about peer rotation, real failure,
CAS invocation, persistent rows and disable events. Inject the intended failure
through the existing AuthStorageOptions refresh hook before seeding credentials,
so the same instance still holds the stale snapshot. Keep actual SQLite/CAS code
and all production logic unchanged. Share fixture construction; keep other
registered-provider tests on the normal refresh path. Do not contact real OAuth
endpoints with fake credentials or substitute fixture success for live evidence.

Steps: reproduce isolated failures; move the three existing refresh fixtures to
the correct injection seam; format/type-check; run all 19 refresh-race contracts
and the full AI workspace suite. Record unrelated harness/environment failures
separately. Global FORCE_COLOR must not be imposed on tests requiring silent
stderr. Retain the broader transition's failed receipt until a real rerun passes.
No legacy deletion or new persisted format is authorized.

```sh
bun test packages/ai/test/auth-storage-oauth-refresh-race.test.ts
bun scripts/ci-test-ts.ts workspace
```

Use the repository formatter and TypeScript check for the changed test. Completion
requires all original race assertions to pass and honest full-suite results.
Recovery: revert only this fixture repair and task evidence, keeping the production
refresh implementation and all user credentials unchanged. The eventual Rust
credential port must preserve these failure, ownership and cancellation contracts.
