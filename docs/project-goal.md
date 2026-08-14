# Project goal

Cortana is a private, local-first second brain for a person and the agents that work with them.
It turns approved notes, messages, documents, calendars, and code into one canonical evidence store
that can be searched, browsed, cited, and safely reused across agents.

## The user promise

Install Cortana, create a workspace, approve one source, validate a small read-only sample, and ask
one cited question. The Desktop app should make that path understandable without requiring users to
learn about connectors, embeddings, MCP, or service managers first.

Cortana is intentionally not an unrestricted crawler, hosted personal-data warehouse, or automatic
backup. A new installation is query-only. Account authorization, source reads, embedding work,
reconciliation, recurring sync, shared-agent access, and optional memory providers are separate
explicit decisions.

## What the system provides

- A Tauri Desktop app for workspaces, source setup, service health, progress, backups, updates, and
  an Obsidian-inspired document browser.
- A canonical store with provenance, source/workspace scope, ACL filtering, audit metadata, and
  compatibility migration for the Hermes-era index.
- Configurable connectors for Google Drive, Gmail, Calendar, Apple Notes, GitHub code, filesystem
  and code roots, Discord, Slack, Buzz, and bounded external adapters.
- Local Qwen text/code embeddings or an OpenAI-compatible cloud provider, with content-addressed
  reuse so unchanged material does not pay for another embedding request.
- Hybrid lexical and semantic retrieval through the Desktop UI, MCP, HTTP, and CLI, with bounded
  context bundles, citations, cache telemetry, and a deterministic extractive fallback.
- A paginated hierarchical knowledge graph that exposes workspace, source, and document
  relationships without loading the entire corpus into memory.

## Completion means evidence, not just code

The project is ready for a production claim only when all of these are true:

1. The current source tree and published release have passed the protected staging → main flow,
   the strict release verifier, and the documented archive/package checks.
2. A real supported Desktop build has passed manual acceptance for first-run tooling approval,
   OAuth/browser flows, source controls, workspace isolation, service start/stop, tray/background
   behavior, native dialogs, backup/restore, updater installation, and the supported operating
   system trust requirements.
3. Every enabled source has a fresh, complete validation at its intended full budget. Sampled or
   incomplete validation never authorizes reconciliation or recurring sync.
4. Shared-agent principals have explicit scopes, ACL tests, rotation/revocation procedures, audit
   evidence, and no cross-workspace leakage.
5. Provider-backed retrieval evaluation covers the packaged core and representative approved data;
   fixture-only evaluation remains a useful regression gate, not a production proof.
6. Hindsight and Honcho remain disabled unless live retention, deletion, ACL, idempotence, export,
   and packaged-UI evidence justifies enabling one.
7. Hermes legacy data is retained until Cortana import/rebuild, recovery, and verification are
   complete; cleanup is a separate deliberate operation.

The authoritative current release and open gates live in [Release history](releases.md),
[Evaluation](evaluation.md), [Operations](operations.md), and the [Desktop UX audit](desktop-ux-audit.md).

## Where to start

- End users: [Getting started](getting-started.md).
- Operators: [Operations](operations.md).
- Source setup: [Ingestion](ingestion.md) and [Source rollout](source-rollout.md).
- Agent builders: [Agent integrations](integrations.md) and [Query](query.md).
- Contributors: [Development instructions](../.github/CONTRIBUTING.md).
