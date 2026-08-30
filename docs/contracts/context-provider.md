# ContextProvider contract

Contract version: `cortana.provider.v1`. Portable fixture version:
`cortana.provider-fixtures.v1`.

This contract exposes Cortana as an optional, provider-neutral source of bounded evidence and
explicit native-memory effects. It does not expose SQLite, connector credentials, filesystem paths,
native Rust types, consumer execution state, or a generic command runner. Agent HQ, Control Plane,
and third-party clients consume this public boundary without becoming Cortana dependencies.

## Identity and mapping

An `ExternalWorkspaceMapping` binds an opaque consumer identity and external workspace identity to
one approved Cortana project and a narrowed ACL set. Consumer, workspace, mapping, principal, and
Cortana project identifiers remain independent. A mapping starts `pending_approval`, may become
`active`, and can be revoked or orphaned without deleting either product's data. Reconnecting an
orphaned capability returns it to approval instead of silently restoring access. Wildcard ACLs,
paths, URLs, database names, and credentials are invalid mapping fields.

Integration principal roles are `query_only`, `query_and_memory`, `status_only`, and
`integration_admin`. Their serialized records contain an opaque credential reference, never a
bearer value or digest. Principals have bounded ACLs, revision, status, optional RFC3339 expiry, and
immediate revocation semantics. Runtime bearer values remain in the process environment or the
private local secret store described in the [identity contract](identity.md).

## Operations and transports

The operations are context, targeted evidence search, status, optional memory recall, explicit
memory write, and authoritative memory-write status. `GET /v1/provider/capabilities` and the MCP
`provider_capabilities` tool return the supported operations, transports, and hard limits.

- `direct_local` is the co-located loopback, stdio, or owner-approved local-service path.
- `scoped_http` is the bearer-authenticated HTTP path used by a different process or the supported
  single-node self-hosted profile.
- `remote_broker` is an allowlisted envelope for those same operations. Its cloud-shaped state may
  carry opaque request, mapping, principal, and connection references, but it cannot select an
  endpoint, executable, path, database, project, or credential.

All transports preserve the same provider request and result meaning. A transport change may alter
availability or latency; it never broadens scope, changes ContextBundle identity, promotes memory,
or changes citation semantics. Requests carry a contract version, request identity, mapping and
principal references, approved project, privacy-safe scope digest, operation, token/byte/time
limits, and an idempotency key for writes.

## Context validation and pins

Consumers validate the ContextBundle contract version, canonical digest and bundle ID, approved
privacy-scope digest, minimum corpus revision, optional memory revision, embedding fingerprint,
retrieval contract version, token budget, degradation, and omission metrics before use. A pin stores
only those non-secret replay fields and creation time. It excludes query text, rendered context,
credentials, paths, and unrestricted private content.

Changed revisions cannot hide behind an old pin. Scope mismatch, digest mismatch, incompatible
contract, stale revision, over-budget content, malformed metadata, and disallowed degradation fail
with stable `ValidationCode` values. Memory stays visibly separate from evidence and cannot satisfy
external-source citation requirements.

## Outcomes and effect policy

Provider outcomes are `ok`, `unavailable`, `unauthorized`, `stale`, `degraded`, `insufficient`,
`over_budget`, `incompatible`, `cancelled`, `rate_limited`, `timed_out`, `host_offline`, and
`ambiguous_write`. Each result is versioned and includes transport, retryability, whether user
action is required, an optional safe message/retry hint, and an optional result. It never substitutes
a stale cache, broadens scope, or uploads the local brain.

Consumers independently choose whether to continue without Cortana, request user action, use
another already-authorized provider, use an explicitly valid pin, or fail their own context
requirement. Cortana does not own that execution policy.

Retrieval mutates neither evidence, memory, nor consumer state. A consumer-owned canonical-state
promotion and a Cortana memory write are separate effects; success of one never implies the other.
Duplicate reads may reuse their authoritative result. Every write requires an idempotency key.
After an ambiguous write, automatic execution stops until the memory-write-status operation returns
an authoritative outcome.

## Fixtures and compatibility

The portable artifact is
[`tests/fixtures/provider-conformance-v1.json`](../../tests/fixtures/provider-conformance-v1.json).
It covers Local and self-hosted single-node deployments, every transport, evidence-only and
memory-enabled bundles, degradation, staleness, budgets, empty and contradictory results,
cross-scope rejection, principal failures, restarts, disconnect/reconnect, replay, and arbitrary
endpoint/path attempts. The artifact's compatibility window is explicit so independent consumers
can reject stale fixtures without importing private Cortana code.
