# Operations

Operational trust boundaries and incident controls are defined in the [security and trust model](security-model.md).

The supported container/VPS profile, TLS boundary, durable volumes, capacity baseline, and
recovery drills are defined in [Self-hosted deployment](self-hosted.md). Local and self-hosted
profiles use the same single-node store and public provider contracts.

This guide defines the supported operational model for a local Cortana installation. It does not track current milestone status or open work.

Current operational tasks and blockers belong in [GitHub milestones](https://github.com/adea-ai/cortana/milestones) and [GitHub issues](https://github.com/adea-ai/cortana/issues). Tagged operational evidence belongs in [Release history](releases.md).

## Operating model

The accepted primary deployment is one local user with:

- a Tauri Desktop application;
- a local Rust core and HTTP/MCP services;
- an optional local embedding service;
- a Python connector environment;
- a SQLite database in WAL mode;
- per-user service definitions;
- owner-private configuration, credential references, logs, and backups.

The safe installed state is query-only. Server and an optional embedding service may run, while source reads, ingestion, reconciliation, recurring synchronization, provider-backed synthesis, shared principals, restore, and memory writes remain explicit decisions.

## Data and configuration

Keep separate:

- application binaries and packaged resources;
- configuration;
- private environment or secure-storage records;
- SQLite data;
- backups;
- logs;
- service definitions;
- connector caches and temporary spools.

Use restrictive owner permissions. Reject unsafe symlinks and non-regular files where the contract requires them. Publish configuration, service files, and mutable state atomically under the appropriate lock.

Do not copy a live SQLite database as a substitute for the supported backup operation.

## Health and readiness

### Health

Health answers whether a process is alive. It must remain lightweight and must not perform full database integrity, source validation, or provider evaluation.

### Readiness

Readiness is a read-only operational gate. It verifies the configured subset of:

- configuration validity;
- database access and integrity;
- embedding/index fingerprint agreement;
- provider availability;
- API behavior;
- backup freshness and verification;
- source validation and schedule policy;
- query mode and optional grounded provider probe;
- packaged component/version agreement where available.

A readiness failure is a stop condition. It must not repair, migrate, ingest, reconcile, schedule, or delete implicitly.

Typical owner-local checks include:

```bash
cortana --version
cortana doctor
cortana readiness --max-backup-age-hours 48
curl -fsS http://127.0.0.1:7331/healthz
```

Use the exact supported flags from `cortana --help` for the installed version.

## Services

Cortana services are independent:

- core HTTP server;
- local embedding runtime;
- recurring source synchronization;
- backup scheduler.

The Desktop may inspect, install, start, stop, restart, or uninstall approved services without making the window the source of truth.

Operational requirements:

- actions are idempotent;
- data, configuration, credentials, logs, and backups survive ordinary service changes;
- failures name the affected service and provide remediation;
- stale PID or state files are handled safely;
- port conflicts fail visibly;
- start-all does not install or enable recurring synchronization;
- closing Desktop does not silently stop approved agent access;
- stopping a service is explicit and auditable.

## Source operations

Follow [Source rollout](source-rollout.md).

The operator sequence is:

1. configure one source;
2. authorize it;
3. select exact scope;
4. validate read-only;
5. run a bounded non-reconciling trial;
6. inspect progress, indexed evidence, citations, resource use, and backup state;
7. complete validation at the intended budget;
8. separately decide reconciliation and recurring policy.

Never use readiness flags as authorization shortcuts. A passing source gate does not itself permit a schedule or deletion.

## Query and provider operations

Core search and ContextBundle construction do not require a query model.

When changing an embedding provider or model:

- probe the proposed provider;
- compare the configured fingerprint with the active index;
- stage or explicitly migrate the embedding generation;
- rebuild atomically where required;
- retain rollback and backup;
- invalidate derived caches;
- do not treat a model dropdown change as sufficient.

When enabling planner or synthesis:

- use an approved provider and bounded request/response settings;
- run deterministic and approved-corpus evaluation;
- require citation validation;
- preserve extractive fallback;
- record privacy disclosure and opt-in;
- keep provider failure from blocking core retrieval.

## Principals and remote access

With no bearer principals configured, HTTP is loopback-only and uses the implicit local owner.

Shared or remote use requires:

- named principals;
- least-privilege scopes;
- explicit ACL labels;
- secret values stored outside TOML and renderer state;
- rotation and revocation;
- metadata-only audit;
- explicit non-loopback acknowledgement;
- an approved TLS/network boundary.

Scopes are conceptually separate:

- `query`;
- `status`;
- `memory`;
- `admin`.

Do not expose the owner-local CLI as a multi-tenant authorization surface. Shared agents use MCP with a scoped token or bearer-authenticated HTTP.

## Backups

Use the supported online SQLite snapshot operation.

A backup procedure must:

- produce a consistent snapshot;
- include required canonical data, memories, ACLs, revisions, and security configuration references;
- exclude or separately handle replaceable caches according to the contract;
- record application/schema version;
- use owner-restricted storage;
- verify the snapshot independently;
- retain enough history for rollback under the configured policy.

Backup success and backup verification are distinct.

## Restore

Restore is destructive and confirmation-gated.

Before restore:

- stop or quiesce mutating writers;
- verify the input;
- verify compatibility;
- check disk space and permissions;
- create a recovery copy of the active data;
- record the intended target and rollback path.

After restore:

- verify SQLite integrity;
- verify corpus and memory revisions;
- test scoped search and recall;
- verify ACL and principal behavior;
- inspect source and validation status;
- run readiness;
- retain audit evidence.

A corrupt, incompatible, symlinked, or incomplete backup must fail before replacing active data.

## Updates

Desktop updates use signed updater artifacts. A supported update verifies:

- manifest and channel;
- archive safety;
- updater signature;
- checksums and package identity;
- nested binaries and platform signing where required;
- application/core/connector version agreement;
- preserved user data and policy;
- restart and service recovery.

Network, manifest, signature, disk, partial-download, or restart failure must leave the prior installation recoverable.

An update must not authorize a source, enable recurring synchronization, expose remote access, or enable synthesis automatically.

## Logs, metrics, and audit

Operational telemetry may include:

- process and service status;
- source and sync-run counts;
- progress and outcomes;
- cache statistics;
- memory lifecycle counts;
- provider degradation;
- latency and errors;
- backup and update state;
- package version.

Default audit and metrics must exclude:

- query text;
- source content;
- memory content;
- bearer values or hashes;
- provider keys;
- credential paths;
- local absolute paths.

Bound log size and retention. Scrub connector stdout/stderr and provider errors before persistence.

## Failure and recovery

General failure rules:

- operations have independent timeouts and resource limits;
- safe idempotent reads may retry with bounded backoff;
- authorization and policy failures fail fast;
- ambiguous destructive effects stop automatic retry;
- partial ingestion never gains reconciliation authority;
- cancellation stops future work and preserves a valid completed prefix;
- corrupt caches are evicted rather than making canonical retrieval fail;
- provider failure degrades explicitly;
- recovery never broadens scope or disables ACLs.

Use disposable drills and temporary indexes for destructive or adversarial validation whenever possible.

## Migration

Hermes or other legacy migration is recovery-first:

1. inventory legacy services, data, credentials, and skills;
2. create and verify Cortana and legacy rollback backups;
3. import or rebuild under explicit workspace and ACL mapping;
4. verify representative retrieval, memory, and recovery;
5. remove only approved obsolete active runtime components;
6. retain migration helpers and explicitly retained legacy data until separate deletion approval.

A successful migration command does not authorize deletion.

## Incident handling

For suspected corruption, credential exposure, unauthorized access, unsafe deletion, or updater compromise:

- stop the affected mutating operation;
- preserve logs and metadata without exposing content;
- revoke or rotate affected credentials;
- isolate remote access;
- create a verified recovery snapshot where safe;
- identify corpus, memory, source, and package revisions;
- open a security or incident issue through the approved private/public channel;
- do not reconcile or restore until the impact and rollback plan are understood.

## Evidence to retain

Operational evidence should include:

- version and platform;
- command or action class without secrets;
- scope and budgets;
- timestamps and outcome;
- revision or package identifiers;
- non-secret errors;
- backup/restore verification;
- reviewer and approval where required;
- linked issue.

## Planning boundary

This guide defines operations. [GitHub milestones](https://github.com/adea-ai/cortana/milestones) and [GitHub issues](https://github.com/adea-ai/cortana/issues) own current operational work, rollout decisions, owners, blockers, and acceptance status.
