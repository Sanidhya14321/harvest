# Shared legacy credential prerequisite

ID application:legacy-provider-credentials; planned, review pending. Prerequisites
control:playbook, review:catalog-providers. Capability surface:oauth-credentials.
The active conversion owner reserves interfaces; no second credential store may
be introduced for trace reports or any other leaf consumer.

Named blocker: the native configuration/value/provider builder supports literal,
exact environment and opt-in command sources but does not discover/refresh the
legacy agent.db stored/OAuth records. Metaharness trace-report.ts remains the
explicit compatibility consumer; native trace-report migration cannot delete it
until this shared boundary and installed credential checks exist. This is not a
missing user approval and does not block independent native-profile consumers.

Review inputs packages/ai/src/auth-storage.ts, auth/sqlite-credential-store.ts,
auth-broker/discover.ts, auth-broker/refresher.ts, auth-retry.ts, existing provider
OAuth/account implementations, config agent-directory selection, native
configuration/value/provider owners and control:oauth-fixtures evidence. Existing
parent ledger ownership stays unchanged until those contracts are reviewed.

Before ready: define supported persisted tables and ownership/locking, stored-key
selection and env precedence, legacy agent directory discovery, OAuth refresh and
rotation/error/expiry contracts, broker boundaries, redaction, cancellation,
crash-safe writes, data migration/recovery and all native consumers. Pick the
shared Rust storage/provider destination after that review; do not guess a schema
or put one-off SQLite access in trace_report. Preserve existing user database.

Implement one shared credential owner, connect native model builders and all
named consumers, run real local data-format/lock/recovery and refresh HTTP
contracts plus applicable installed/live checks, then move compatibility consumers
and delete their legacy sources. No retirement of stored/OAuth behavior is
approved. Record exact remaining callers and source-bound receipts. Restore the
previous shared credential path on failure; never erase/replace the user DB.
Use cargo xtask test for the eventual affected packages; exact selectors and
installed check commands must be reviewed before ready. No Rust implementation,
behavioral completion, source removal or installation evidence is claimed here.
