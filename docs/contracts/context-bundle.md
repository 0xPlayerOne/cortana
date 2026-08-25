# ContextBundle contract

`ContextBundle` is the reproducible bridge between Cortana retrieval and an agent. It is evidence
first: source evidence is cited, native memory is visibly separate, and degradation is explicit.
The contract is `cortana.context.v1`.

## Envelope

Every HTTP `/v1/context`, MCP `context`, and CLI `context --json` response contains the same fields:

| Field                        | Type         | Rule                                                            |
| ---------------------------- | ------------ | --------------------------------------------------------------- |
| `contract_version`           | string       | `cortana.context.v1`                                            |
| `context_bundle_id`          | string       | Stable `ctx_` + SHA-256 digest of the canonical payload         |
| `canonical_digest`           | string       | Lowercase SHA-256 of canonical serialization                    |
| `created_at`                 | RFC3339 UTC  | Creation time; not used as identity                             |
| `token_budget`               | integer      | Effective bounded budget, 256–64,000 tokens                     |
| `query`                      | string       | Bounded request query; never included in audit telemetry        |
| `evidence`                   | array        | ACL-filtered source evidence with citations                     |
| `memories`                   | array        | Optional ACL-filtered operational memory, never a citation      |
| `metrics`                    | object       | Retrieved/included/omitted counts and token estimate            |
| `retrieval_mode`             | string       | `hybrid`, `lexical-fallback`, or another registered mode        |
| `degradation`                | object/null  | Stable code and bounded detail when fallback/degraded           |
| `corpus_revision`            | integer      | Canonical corpus revision at build time                         |
| `memory_revision`            | integer/null | Included only when memory scope is granted                      |
| `embedding_fingerprint`      | string/null  | Provider/model generation, never a credential                   |
| `retrieval_contract_version` | string       | `cortana.retrieval.v1`                                          |
| `privacy_scope_digest`       | string       | Hash of normalized project/source/ACL scope, not raw scope data |

The legacy `retrieval_warning` field remains readable for older clients and mirrors the bounded
degradation detail. New clients must use `degradation`.

## Canonical serialization and identity

The digest input is a deterministic JSON object with sorted scope labels and the fields above except
`context_bundle_id` and `canonical_digest`. Evidence and memory array order is the returned stable
ranking order. JSON uses UTF-8, no insignificant whitespace, and the Rust serde field order. A
consumer recomputes the digest before accepting a bundle.

The digest changes when query, evidence, memory, token budget, retrieval mode, scope, embedding
generation, or either canonical revision changes. It does not include credentials, private paths,
raw tokens, unrestricted query history, or unbounded provider output.

## Compatibility and safety

- Additive fields are ignored by v1 readers; required-field or meaning changes require v2.
- Consumers reject an unknown contract version, stale revision, mismatched scope digest, invalid
  digest, unauthorized memory, or degraded bundle when their policy requires complete retrieval.
- A degraded bundle is still useful for explicit fallback paths, but its state must be shown to the
  caller and never cached as a successful provider result.
- Evidence is the only source for factual citations. Memory is operational context and must not be
  rendered as a source citation.
