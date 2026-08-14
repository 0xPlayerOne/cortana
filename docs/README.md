# Cortana documentation

Cortana is a local-first, agent-native second brain. [Getting started](getting-started.md) is the
shortest path from download to a first bounded query; this index routes you to the right guide
after installation. The [root README](../README.md) explains the project purpose, architecture,
and contributor path.

## Start here

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
  and degraded operation.
- [Evaluation](evaluation.md) — the bounded model, release, readiness, and evidence requirements.
- [Configuration example](../config.example.toml) — a redacted starting point for local, cloud,
  multi-workspace, and optional sidecar settings.

## Desktop and architecture

- [Desktop guide](../apps/desktop/README.md) — native settings, source authorization, services,
  tray behavior, updater boundaries, and contributor commands.
- [Desktop architecture](desktop-architecture.md) — Tauri trust boundaries, sidecars, lifecycle,
  and packaging.
- [Desktop UX audit](desktop-ux-audit.md) — current evidence, completed UX requirements, and
  explicitly open manual gates.
- [Memory adapters](memory-hindsight-outbox.md) and [Honcho contract](memory-honcho.md) — optional
  sidecars that remain disabled until their provider and deletion/ACL gates are proven.

## Releases and contribution

- [Release history](releases.md) — current release, asset verification, promotion flow, and
  historical recovery notes.
- [Development instructions](../.github/CONTRIBUTING.md) — branch, test, and protected PR flow.
- [Extension points](EXTENSIONS.md) — how repository-owned workflows coexist with Code Foundry.

## Safety boundary

The first-run path is query-only. Documentation examples intentionally use bounded validation and
non-reconciling trial syncs. A failed readiness or source-validation check is a stop condition;
never bypass it by enabling a schedule or increasing limits. Full-corpus validation, recurring
sync, shared-agent principals, live Hindsight/Honcho use, and native GUI acceptance each require
their own evidence and explicit operator approval.

## Documentation source of truth

When a user-facing feature, source, release, safety gate, or operating procedure changes, update
the smallest relevant guide and then refresh this index and the root README links if the first-run
path changed. Current-release evidence belongs in [Release history](releases.md),
[Evaluation](evaluation.md), and the [Desktop UX audit](desktop-ux-audit.md); historical evidence
must be labeled historical rather than silently overwritten.
