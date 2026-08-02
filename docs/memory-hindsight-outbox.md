# Milestone: Optional Hindsight Outbox Sidecar

Cortana’s default ingestion and query path remains canonical and unchanged.

This milestone adds an **optional** Hindsight sidecar package under `src/cortana/memory`
that is intentionally disabled by default:

- no change to normal source ingestion flows,
- no automatic retention queue population from the sync pipeline,
- no production defaults enabling the provider.

## Design

- `MemoryDocument` models a canonical source update with a deterministic, stable document id
  derived from `project/source/source_id`.
- `Outbox` stores sync work items in durable SQLite with:
  - schema versioning,
  - idempotent enqueue with upsert on `(operation, document_id)`,
  - `pending`/`in_flight`/`succeeded`/`dead_letter` states,
  - lease/claim semantics,
  - bounded retry attempts and backoff,
  - retry/reconciliation exports and stats.
- `MemorySyncWorker` drains only due rows, calls provider operations, and marks entries
  succeeded or failed according to transient vs non-transient outcomes.
- `HindsightHttpProvider` uses documented retain/delete endpoints with bank scope and
  preserves a configured HTTP base URL.

## Hindsight contract notes

- retain: `POST /v1/default/banks/{bank}/memories/retain`
- delete: `DELETE /v1/default/banks/{bank}/documents/{document_id}`

`retain` payload uses a string `context` (or omits it), with structured `metadata` and
`tags`.

## Why no bulk sync

No bulk copy of the canonical source corpus is performed. Hindsight only receives
canonical-document operations (retain/delete) for selected memory events so Cortana remains
source-of-truth for evidence, provenance, and retention.

## Honcho status

Honcho remains deferred to a later milestone; only this optional derived sidecar is introduced
as a non-default integration until evaluation gates are met.
