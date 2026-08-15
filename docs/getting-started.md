# Getting started

Cortana is a local-first, agent-native second brain. It turns approved notes, messages,
documents, calendars, and code into one searchable evidence store, then exposes the same cited
context through the Desktop app, MCP, HTTP, and CLI.

The normal first-run experience is deliberately safe: Cortana starts in query-only mode. It does
not sign in to an account, download model weights, read a source, index content, or install a
recurring sync until you approve that action. A failed readiness or validation check is a stop
condition, not something to bypass.

## Choose your installation path

- **Desktop release (recommended):** download the matching installer from the [latest GitHub
  release](https://github.com/0xPlayerOne/cortana/releases/latest). This is the simplest path for
  workspace, source, service, and update settings.
- **Core archive:** use the matching platform archive when you want the CLI, MCP, or HTTP service
  without the Desktop shell.
- **Checkout:** use the [contributor setup](../.github/CONTRIBUTING.md) for unreleased source-tree
  work. A checkout may contain hardening that is not present in the latest published package.

The installer preserves an existing configuration, index, backups, and secrets. It does not
authorize an account, download a connector corpus, start a sync, or enable a scheduler by itself.

Before you begin, use a supported release package for your operating system and CPU and keep at
least one backup location available. No account credentials are required for the first launch. On
macOS, an unsigned or non-notarized build may be rejected by Gatekeeper; use the published package
or follow the host-acceptance notes in the [Desktop audit](desktop-ux-audit.md).

## The shortest path to a first result

1. **Download Cortana Desktop.** Open the [latest GitHub release](https://github.com/0xPlayerOne/cortana/releases/latest)
   and choose the installer for your operating system and CPU. The current protected release is
   **v0.32.4**; [Release history](releases.md) records its verification and any later patch.
2. **Install and launch.** Approve the optional tooling that Cortana offers to install. Choose a
   local Qwen-compatible embedding runtime if you want local embeddings; choose a cloud provider
   if you do not want local model tooling.
3. **Create one workspace.** Start with a single workspace such as `work`, `personal`, or
   `special`. Workspaces keep source accounts and retrieval scopes understandable; you can add
   more later.
4. **Configure one source.** In **Settings → Sources**, choose a source, assign it to the
   workspace, and use **Authorize** or **Open provider setup**. For Apple Notes, enter exact
   folder names in **Include Apple Notes folders** or **Exclude Apple Notes folders**; create
   separate Apple Notes sources when folders belong to different workspaces. Credentials stay
   outside the renderer and are never written into the index.
5. **Validate before syncing.** Select **Validate** with the default small budget. Validation is
   read-only: it does not embed, index, reconcile, or delete anything.
6. **Run one bounded initial sync.** After validation succeeds, confirm **Initial sync** with the
   smallest budget. Review the progress and source status, then ask a known question and confirm
   that the answer includes citations.

Apple Notes can be split cleanly across workspaces with exact folder filters. For example, create
one `work` source that includes `Nifty League`, one `special` source that includes `The Pink Binder`,
and a personal source that excludes both folders. If an embedding run is interrupted, Cortana
persists completed documents and resumes only the unfinished tail on the next bounded run.

If you are using the CLI instead of Desktop, confirm the installed version before changing any
configuration:

```bash
cortana --version
cortana doctor
cortana readiness --max-backup-age-hours 48
```

Readiness is read-only. A failure means stop and correct the reported issue; do not increase source
limits, enable a scheduler, or delete the index to force a green result.

Stop after step 6 if you only need a local searchable brain. Recurring sync, shared-agent access,
cloud model use, and optional Hindsight/Honcho memory adapters are separate decisions with their
own safety and evidence gates.

For a first trial, keep the default source budget and stop after one known cited question. Do not
use a full-corpus budget to make a validation pass, and do not treat a sampled filesystem/code
validation as permission for reconciliation or recurring sync.

## What you get

- An Obsidian-inspired Desktop workspace for browsing workspaces, sources, documents, and bounded
  relationships.
- Hybrid retrieval that combines lexical, semantic, IDF, and recency signals with ACL filtering,
  provenance, neighboring context, and citation-ready output.
- A portable agent skill plus MCP, HTTP, and CLI interfaces using the same retrieval contract.
- Incremental, content-addressed embedding reuse for local Qwen or OpenAI-compatible cloud
  embedding providers.
- Query-only background services, backups, audit metadata, and explicit progress/cancellation for
  source jobs.

## What Cortana does not do automatically

- It does not crawl every configured source on first launch.
- It does not enable recurring ingestion when a validation is sampled, incomplete, stale, or
  missing.
- It does not treat Hindsight or Honcho as the source of truth; both are optional and disabled by
  default.
- It does not claim that a static package check proves GUI, browser OAuth, tray, native dialog,
  updater-install, or operating-system signing behavior. See the [Desktop UX audit](desktop-ux-audit.md)
  for current evidence and open manual gates.

## After the first query

- [Desktop guide](../apps/desktop/README.md) — settings, services, tray behavior, updates, and
  native boundaries.
- [Ingestion guide](ingestion.md) — source-specific authorization, budgets, cursors, ACLs, and
  reconciliation.
- [Query guide](query.md) — context bundles, synthesis, embeddings, cache behavior, and fallback.
- [Agent integrations](integrations.md) — install the skill and connect MCP, HTTP, or CLI clients.
- [Operations guide](operations.md) — readiness, backups, recovery, authentication, and service
  management.

The shortest safe success criterion is: the app launches, one source validates, one bounded
non-reconciling trial completes, and a cited query returns the expected document. GUI OAuth,
native dialogs, tray/autostart, updater installation, and macOS Developer ID/notarization remain
separate host-acceptance checks even when headless tests and static package verification pass.

## If something is not ready

Keep the installation query-only and open the relevant status panel. Readiness and source
validation are designed to fail closed. Do not increase limits just to make a check pass, delete
the index, or enable a scheduler before the source has a complete validation at the requested
budget. For contributor and unreleased-build setup, use the [development instructions](../.github/CONTRIBUTING.md)
instead of the end-user path above.
