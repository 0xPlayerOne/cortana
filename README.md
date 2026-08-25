# Cortana

Cortana is a local-first, agent-native second brain for people and the AI agents that work with them. It turns explicitly approved notes, messages, documents, calendars, code, and other sources into one canonical evidence store; maintains a separate native memory lifecycle for bounded conclusions; and compiles cited, token-bounded context for supported agents.

Cortana is independently usable through:

- a Tauri 2 Desktop application;
- an MCP stdio server;
- a loopback or explicitly secured HTTP API;
- a local-owner CLI.

It is not an unrestricted crawler, implicit backup service, agent harness, or hosted personal-data warehouse. A new installation starts query-only. Source authorization, validation, ingestion, reconciliation, recurring synchronization, model use, shared-agent access, and memory writes are separate explicit decisions.

## Product model

Cortana separates three authorities and one derived output:

- **Evidence** — source-backed documents, messages, notes, calendar records, and code. Evidence retains stable source identity, provenance, workspace scope, update time, and ACLs.
- **Native memory** — bounded conclusions deliberately retained by a user or authorized agent. Memory has its own provenance, confidence, importance, expiry, supersession, redaction, and revision lifecycle.
- **External task state** — harness scratchpads, Agent HQ ProjectState, Control Plane ContextPackages, workflow checkpoints, and conversation history remain owned by their respective systems.
- **ContextBundle** — a disposable, token-bounded compilation of authorized evidence and relevant memory for one query or objective.

The governing rule is:

> Sources teach Cortana. Agents remember into Cortana. Harnesses retrieve bounded context from Cortana.

## Safe first run

1. Download the matching package from the [latest release](https://github.com/0xPlayerOne/cortana/releases/latest).
2. Launch Cortana Desktop and approve only the optional tooling you need.
3. Create a workspace.
4. Configure and authorize one source.
5. Run a bounded read-only validation.
6. Confirm one small non-reconciling initial sync.
7. Browse the indexed document and ask one cited question.

The detailed path is in [Getting started](docs/getting-started.md).

## Core capabilities

### Knowledge ingestion

Cortana normalizes approved sources into one workspace-scoped `Document` contract. Built-in source classes include Google Drive, Gmail, Google Calendar, Apple Notes, GitHub, filesystem/code roots, Slack, Discord, Buzz, and external commands that emit the normalized JSONL contract.

Authorization, discovery, validation, trial sync, complete validation, reconciliation, scheduling, and disabling are distinct lifecycle steps. Only a fresh, complete, configuration-matched snapshot can authorize reconciliation.

### Retrieval and context

Cortana combines semantic vector retrieval with SQLite FTS5 lexical retrieval, query-term coverage, recency, deterministic fusion, canonical-source deduplication, and bounded neighboring context. Embedding failure degrades to marked lexical fallback rather than failing the query or fabricating semantic results.

`context` is the primary agent primitive. It returns:

- numbered source evidence;
- separately labeled native memory;
- applied scope and budget;
- inclusion and omission metrics;
- retrieval mode and degradation warnings;
- estimated token use.

### Native memory

The public memory operations are:

- `remember` — retain one bounded provenance-bearing conclusion;
- `recall` — retrieve authorized active memory;
- `forget` — redact one memory while preserving a minimal tombstone;
- `export_memory` — export bounded visible records and tombstones.

Semantic, episodic, procedural, and preference describe durable content types. Working state is short-lived and normally expiry-bounded. Source ingestion never silently becomes native memory, and memory never satisfies source-citation requirements.

### Desktop operations

The Desktop application provides first-run setup, workspaces, sources, authorization, validation, progress, document and graph browsing, memory controls, provider settings, services, schedules, backups, restore, updates, tray behavior, and native dialogs through a narrow Rust/Tauri privilege boundary.

The Desktop window is not the source of truth for background services. Closing the window may leave approved local services running; stopping them is explicit.

## Agent interfaces

The MCP server exposes narrow tools for evidence retrieval, context compilation, code and message search, expertise discovery, memory lifecycle, export, and status. HTTP and CLI surfaces use the same store, authorization, retrieval, and memory contracts.

Shared or least-privilege agents must use a scoped bearer principal through MCP or HTTP. The owner-local CLI is not a multi-tenant authorization surface.

See [Agent integrations](docs/integrations.md) and [Query](docs/query.md).

## Planning and evidence ownership

[GitHub milestones](https://github.com/0xPlayerOne/cortana/milestones) and [GitHub issues](https://github.com/0xPlayerOne/cortana/issues) are the only authority for current scope, sequencing, ownership, dates, dependencies, blockers, status, and task-level acceptance evidence.

Durable documentation has separate ownership:

- [Project goal](docs/project-goal.md) defines the stable user promise.
- [Documentation map](docs/README.md) explains document ownership.
- [Release history](docs/releases.md) preserves tagged release evidence.
- [Evaluation](docs/evaluation.md) defines evaluation methods and evidence rules.
- [Operations](docs/operations.md) defines supported operational procedures.
- [Source rollout](docs/source-rollout.md) defines the source-activation procedure.
- [Desktop UX audit](docs/desktop-ux-audit.md) defines packaged-product acceptance.
- ADRs record durable architecture decisions.

See [Planning and tracking](docs/planning.md).

## Development

Read [AGENTS.md](AGENTS.md) and [.github/CONTRIBUTING.md](.github/CONTRIBUTING.md) before changing the repository.

The normal change path is:

1. create one focused branch from `main`;
2. implement and test one issue-sized change;
3. open a pull request to `main`;
4. let required checks validate the exact tree;
5. merge only after review and CI.

Do not authorize live sources, run broad ingestion, install recurring synchronization, change embedding generations, restore backups, expose remote access, or delete migration data merely because a development task mentions those capabilities.

The Code Foundry test categories are available locally as well as in CI:

```sh
bun run test:unit        # JavaScript + non-integration Python + docs
bun run test:integration # installer and packaging/release boundaries
bun run test:smoke       # bounded source/runtime probes
```

JavaScript suites run in stable, isolated groups with a default cap of two
parallel groups. Set `CORTANA_TEST_MAX_PARALLEL=1` when debugging a flaky suite
or raise it only on a runner with enough CPU and memory headroom.

## License

Cortana is licensed under [Apache License 2.0](LICENSE).
