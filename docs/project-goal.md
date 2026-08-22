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
reconciliation, recurring sync, shared-agent access, and explicit memory writes are separate
explicit decisions.

## What the system provides

- A Tauri Desktop app for workspaces, source setup, service health, progress, backups, updates, and
  an Obsidian-inspired document browser.
- A canonical store with provenance, source/workspace scope, ACL filtering, audit metadata, and
  compatibility migration for the Hermes-era index.
- Configurable connectors for Google Drive, Gmail, Calendar, Apple Notes, GitHub code, filesystem
  and code roots, Discord, Slack, and Buzz.
- Local Qwen text/code embeddings or an OpenAI-compatible cloud provider, with content-addressed
  reuse so unchanged material does not pay for another embedding request.
- Hybrid lexical and semantic retrieval through the Desktop UI, MCP, HTTP, and CLI, with bounded
  context bundles, citations, cache telemetry, and a deterministic extractive fallback.
- A vertically integrated native agentic-memory layer in the same store for explicit semantic,
  episodic, procedural, preference, and working memories, with expiry, supersession, ACL
  enforcement, audit metadata, scoped export, and cache invalidation shared with knowledge
  retrieval. Cortana's own store is the sole supported memory engine for this release.
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
   fixture-only evaluation remains a useful regression gate, not a production proof. The repository
   ships a bounded read-only live-index harness (`scripts/evaluate-live-index.py`) for this
   operator-controlled evidence; the first retrieval-only run is recorded in the evaluation
   guide, while approved-corpus answer/synthesis evidence is still required before claiming this
   gate fully closed.
6. Native memory retention, deletion, ACL, idempotence, export, and packaged-UI evidence are
   covered by the same canonical-store contract.
7. Hermes migration compatibility remains available for explicit imports, but the live installation
   must not run a parallel legacy stack. Active legacy rows, launch agents, and machine-level
   directories are removed only after import/rebuild and a verified rollback backup; the migration
   helpers remain in the source tree for controlled recovery of older installations.

The authoritative current release and open gates live in [Release history](releases.md),
[Evaluation](evaluation.md), [Operations](operations.md), and the [Desktop UX audit](desktop-ux-audit.md).

## Release boundary

The published `v0.34.19` tag is the current source/release boundary. Its release-assets workflow
completed the strict 18-asset verifier, including checksums, updater signatures,
manifest, and packaged-core evidence. A source checkout can contain validated changes
that are not yet in the installer; only a tag with recorded verifier evidence should be presented
as downloadable release behavior.

## Where to start

- End users: [Getting started](getting-started.md).
- Operators: [Operations](operations.md).
- Source setup: [Ingestion](ingestion.md) and [Source rollout](source-rollout.md).
- Agent builders: [Agent integrations](integrations.md) and [Query](query.md).
- Contributors: [Development instructions](../.github/CONTRIBUTING.md).
