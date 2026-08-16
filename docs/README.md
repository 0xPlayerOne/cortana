# Cortana documentation

Cortana is a local-first, agent-native second brain. [Getting started](getting-started.md) is the
shortest path from download to a first bounded query; this index routes you to the right guide
after installation. The [root README](../README.md) explains the project purpose, architecture,
and contributor path.

## Current status

| Area                           | Current boundary                                                                                                                                  |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| Downloadable package           | `v0.33.0`, with release-assets workflow `31969571292` verified: 18 assets, signatures, manifest, and packaged core                                |
| Source checkout                | Tracks the protected v0.33.0 tree; the tag is published and the strict verifier passed                                                            |
| Default runtime                | Query-only; no source authorization, full sync, or recurring schedule is enabled automatically; native memory is explicit-write only              |
| Safe first milestone           | One workspace, one source, bounded validation, one non-reconciling trial, and one cited query                                                     |
| Knowledge browser              | Obsidian-style workspace/source/document navigation with bounded hierarchical graph pages and local type filters                                  |
| Operational visibility         | Query-only service status, health/readiness probes, source progress, and explicit stale-stat warnings during temporary SQLite contention          |
| Still requires host acceptance | Packaged GUI/browser OAuth, tray and native dialogs, updater install, macOS Developer ID/notarization, and complete full-budget source validation |

This table is the documentation boundary for the project. Update it whenever a release, safety gate,
or first-run workflow changes; keep historical measurements in the linked audit and release pages
instead of silently rewriting them.

## Start here

- [Project goal](project-goal.md) — the product purpose, user promise, and evidence-based
  definition of production readiness.
- [Getting started](getting-started.md) — the simple Desktop-first path from download to a cited
  first result.
- [Download and first launch](../README.md#desktop-first-launch-recommended) — the same path in
  the root project overview.
- [Operations](operations.md) — start and stop services, inspect readiness and telemetry, manage
  backups, configure authentication, and recover safely.
- [Agent integrations](integrations.md) — install the portable skill and connect MCP, HTTP, or CLI
  clients with optional scoped principals.

## Configure the system

- [Ingestion](ingestion.md) — source adapters, workspaces, budgets, cursors, ACLs, deletion
  reconciliation, and validation gates.
- [Source rollout plan](source-rollout.md) — per-source authorization, bounded trials, production
  validation, and the explicit recurring-sync gate.
- [Query](query.md) — hybrid retrieval, local Qwen or cloud embeddings, synthesis, cache behavior,
  degraded operation, and the bounded hierarchical knowledge graph.
- [Native memory](memory.md) — memory types, provenance, ACLs, lifecycle, MCP, HTTP, and CLI usage.
- [Evaluation](evaluation.md) — the bounded model, release, readiness, and evidence requirements.
- [Configuration example](../config.example.toml) — a redacted starting point for local, cloud,
  multi-workspace, and local service settings.

## Desktop and architecture

- [Desktop guide](../apps/desktop/README.md) — native settings, source authorization, services,
  tray behavior, updater boundaries, and contributor commands.
- [Desktop architecture](desktop-architecture.md) — Tauri trust boundaries, sidecars, lifecycle,
  and packaging.
- [Desktop UX audit](desktop-ux-audit.md) — current evidence, completed UX requirements, and
  explicitly open manual gates.
- [Memory](memory.md) — the vertically integrated native agent-memory layer.

## Releases and contribution

- [Release history](releases.md) — current release, asset verification, promotion flow, and
  historical recovery notes.
- [Development instructions](../.github/CONTRIBUTING.md) — branch, test, and protected PR flow.
- [Extension points](EXTENSIONS.md) — how repository-owned workflows coexist with Code Foundry.

## Safety boundary

The first-run path is query-only. Documentation examples intentionally use bounded validation and
non-reconciling trial syncs. A failed readiness or source-validation check is a stop condition;
never bypass it by enabling a schedule or increasing limits. Full-corpus validation, recurring
sync, shared-agent principals, and native GUI acceptance each require
their own evidence and explicit operator approval.

## Documentation source of truth

When a user-facing feature, source, release, safety gate, or operating procedure changes, update
the smallest relevant guide and then refresh this index and the root README links if the first-run
path changed. Current-release evidence belongs in [Release history](releases.md),
[Evaluation](evaluation.md), and the [Desktop UX audit](desktop-ux-audit.md); historical evidence
must be labeled historical rather than silently overwritten. The graph contract belongs in
[Query](query.md), and status/readiness semantics belong in [Operations](operations.md), so a
feature should not be documented only in a changelog or UI audit.
