# Source rollout

This document defines the durable procedure for moving a configured connector into approved Cortana use. It does not list the current enabled inventory or act as a roadmap.

Current source work, owners, blockers, rollout decisions, and acceptance evidence belong in [GitHub milestones](https://github.com/adea-ai/cortana/milestones) and [GitHub issues](https://github.com/adea-ai/cortana/issues). Tagged and dated evidence belongs in [Release history](releases.md) or the owning issue.

## Core rule

Connector implementation proves that a source type can satisfy Cortana's contract. It does not authorize an account, source scope, ingestion run, reconciliation, or recurring schedule.

The lifecycle is:

1. configure;
2. authorize;
3. discover and select scope;
4. validate read-only;
5. run a bounded non-reconciling trial;
6. inspect indexed evidence and operations;
7. complete validation at the intended production budget;
8. decide reconciliation policy;
9. decide recurring synchronization policy;
10. disable, revoke, or remove safely when needed.

Each step is explicit and independently auditable.

## Source identity and scope

Each configured source has:

- one stable source name;
- one connector kind;
- one workspace/project;
- one approved account, root, repository, folder, calendar, channel, community, or equivalent scope;
- one default ACL or a narrower source-provided ACL;
- bounded document, byte, time, response, spool, and concurrency settings;
- credential references or a provider-native authorization record;
- a configuration fingerprint.

Changing scope, credentials, inclusion rules, budgets, or other material configuration invalidates prior validation authority.

## Configure

Configuration creates or updates the source definition without reading or indexing source content.

Review:

- workspace/project assignment;
- ACL label;
- account or host boundary;
- exact include/exclude rules;
- intended production budget;
- connector-specific credentials or native authorization references;
- whether the source is enabled for validation or ingestion.

Disabling a source stops future reads. It does not silently delete indexed evidence.

## Authorize

Authorization uses the provider-specific mechanism:

- browser OAuth or callback;
- device flow;
- Desktop RPC;
- host permission;
- environment-backed token;
- owner-selected filesystem path;
- read-only local application identity.

Authorization must not perform ingestion, embeddings, reconciliation, or scheduling. Secrets remain provider-, runtime-, keychain-, environment-, or owner-private-file owned according to the connector contract.

## Discover and select scope

Where the provider supports discovery, the user selects the exact approved scope before validation:

- Drive folders or account;
- repositories;
- Notes folders;
- calendars;
- Slack teams and channels;
- Discord servers and channels;
- Buzz communities;
- filesystem/code roots.

Discovery responses are bounded and contain no reusable secrets. Selection is workspace-scoped and cannot grant visibility to another workspace.

## Read-only validation

Validation exercises authorization, configuration, listing, parsing, conversion, cursor behavior, and resource limits without:

- writing documents;
- creating embeddings;
- changing the index;
- reconciling deletions;
- installing a schedule.

A validation record includes:

- source and configuration fingerprint;
- status and completion state;
- document and byte counts;
- applied budgets;
- start and completion times;
- connector outcome;
- zero-write confirmation;
- freshness.

A sampled or capped validation may authorize only an equal-or-smaller non-reconciling trial. It never authorizes complete ingestion, reconciliation, or recurring synchronization.

## Bounded initial trial

The initial trial must:

- name one approved source;
- require matching validation;
- use explicit document, byte, and time budgets;
- use non-reconciling mode;
- state expected write impact;
- have a verified backup and rollback procedure;
- record progress, retries, cancellation, resource use, and results.

Review after the run:

- changed, unchanged, failed, cancelled, budget-exceeded, and deleted counts;
- cursor and cache behavior;
- embedding reuse;
- source identity and links;
- exact canonical content;
- workspace and ACL isolation;
- cited retrieval;
- service health and query availability;
- database and backup verification.

A partial prefix may remain indexed when it is valid. A partial, failed, cancelled, timed-out, capped, sampled, or explicitly non-reconciling run must report zero authoritative deletions.

## Complete validation

Before any complete or recurring policy is considered, the source must have a fresh validation that:

- covers the intended complete scope;
- uses budgets at least as large as the proposed run;
- reports `complete=true`;
- matches the current configuration fingerprint;
- has no unresolved authorization, detail-fetch, cursor, conversion, or scope errors.

Complete validation proves only that the source can be read under that configuration. It does not itself authorize ingestion, reconciliation, or scheduling.

## Reconciliation

Reconciliation removes canonical records that are absent from a complete authoritative snapshot. It is a destructive capability and requires all of the following:

- explicit operator approval;
- a fresh matching complete validation;
- a complete reconciling run;
- stable source/source_id identity;
- no sampled, capped, failed, cancelled, timed-out, or unresolved provider state;
- backup and rollback evidence;
- deletion counts and audit evidence.

One source failure cannot authorize deletion for another source. Cursor optimization may reduce reads but cannot weaken complete-snapshot guarantees.

## Recurring synchronization

Recurring synchronization is a separate policy decision. The policy defines:

- eligible sources;
- cadence and jitter;
- quiet hours;
- concurrency and resource ceilings;
- validation freshness;
- backup prerequisites;
- retry and backoff;
- notification and escalation;
- reconciliation mode;
- canary activation;
- pause, resume, disable, and uninstall behavior.

Installing general Cortana services must not install recurring synchronization implicitly. Disabling the scheduler preserves the index, source configuration, credentials, and manual query operation.

## Failure and recovery

Connectors may retry only safe idempotent reads for bounded transport, timeout, rate-limit, and approved provider failures.

On failure or cancellation:

- stop future work;
- record the explicit outcome;
- preserve valid completed-prefix writes;
- release locks and bounded spools;
- retain query availability where possible;
- make no authoritative deletions;
- provide a safe retry or remediation path.

Authorization, ACL, configuration, scope, and validation failures fail fast and closed.

## Source-family contract

| Source family    | Authorization boundary                        | Scope examples                                 | Required production evidence                                                                       |
| ---------------- | --------------------------------------------- | ---------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Google Drive     | Browser OAuth and owner-private refresh token | Account, folder, exported documents            | Complete listing/detail/conversion validation, bounded trial, ACL and source-link checks           |
| Gmail            | Browser OAuth and owner-private refresh token | Account, labels or provider-supported snapshot | Complete message/detail validation, history/cursor checks, bounded trial, thread/provenance checks |
| Google Calendar  | Browser OAuth and owner-private refresh token | Account and selected calendars                 | Complete event validation, recurring-series bounds, empty-snapshot distinction, bounded trial      |
| Apple Notes      | Host-native permission                        | Exact included and excluded folders            | Complete folder-scoped validation, exact routing, bounded trial                                    |
| GitHub           | Device flow or token reference                | Selected repositories                          | Repository selection, revision identity, bounded trial, code provenance                            |
| Filesystem/code  | Owner-selected local root                     | Exact roots and excludes                       | Full-root validation without sampling before complete policy; generated/vendor/worktree exclusions |
| Slack            | Browser OAuth plus message-sync credential    | Team and channels                              | Team/channel authorization, complete validation, bounded message trial                             |
| Discord          | Signed-in Desktop RPC and private token state | Servers and channels                           | Current authorization, complete selected-channel validation, bounded trial                         |
| Buzz             | Read-only local identity/application data     | Communities                                    | Identity-file validation, selected-community validation, bounded trial                             |
| External command | Explicit executable and config                | Adapter-defined                                | Connector certification, JSONL conformance, budgets, cancellation, completeness, secret handling   |

## Evidence to retain

Retain non-secret evidence sufficient to reproduce the decision:

- source name, kind, workspace, and ACL;
- configuration fingerprint;
- authorization method and time;
- validation completeness, budgets, counts, and age;
- trial command or request envelope;
- changed/unchanged/deleted counts;
- progress, cancellation, retry, and failure results;
- resource and cache observations;
- cited spot checks;
- backup and rollback verification;
- explicit approval for reconciliation or recurring scheduling.

Do not retain raw tokens, credential paths, source content, private query text, or unnecessary absolute paths in issues or audit metadata.

## Planning boundary

The procedure above is durable. The current source inventory, pending trials, activation order, owners, and blockers belong only in [GitHub milestones](https://github.com/adea-ai/cortana/milestones) and [GitHub issues](https://github.com/adea-ai/cortana/issues).
