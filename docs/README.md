# Cortana documentation

Cortana is a local-first, agent-native second brain. The root [README](../README.md) is the
shortest path from download to a first bounded query; this index routes you to the right guide
after installation.

## Start here

- [Download and first launch](../README.md#desktop-first-launch-recommended) — install the Desktop
  app, approve tooling, configure one source, validate it, and run a safe trial.
- [Operations](operations.md) — start and stop services, inspect readiness and telemetry, manage
  backups, configure authentication, and recover safely.
- [Agent integrations](integrations.md) — install the portable skill and connect MCP, HTTP, or CLI
  clients with optional scoped principals.

## Configure the system

- [Ingestion](ingestion.md) — source adapters, workspaces, budgets, cursors, ACLs, deletion
  reconciliation, and validation gates.
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
