# Project goal

Cortana is a private, local-first second brain for a person and the agents that work with them. It turns explicitly approved notes, messages, documents, calendars, code, and other sources into one canonical evidence store that can be searched, browsed, cited, and safely reused across supported agents.

## User promise

A new user should be able to:

1. install Cortana Desktop;
2. create a workspace;
3. authorize one source;
4. run a bounded read-only validation;
5. confirm one small non-reconciling initial sync;
6. browse the resulting document;
7. ask one cited question.

The supported path must not require the user to understand connector subprocesses, embedding infrastructure, SQLite, MCP, or operating-system service managers.

## Product promise

Cortana provides:

- a local canonical evidence store with source identity, provenance, workspace scope, ACLs, timestamps, and exact content;
- configurable source connectors with explicit authorization, validation, budgets, incremental ingestion, reconciliation safety, cancellation, and status;
- hybrid semantic and lexical retrieval through Desktop, MCP, HTTP, and CLI;
- bounded ContextBundles that keep source evidence and native memory visibly separate;
- content-addressed embedding reuse and revision-aware derived caches;
- native semantic, episodic, procedural, preference, and expiry-bounded working memory;
- explicit remember, recall, forget, export, expiry, dedupe, supersession, provenance, and backup semantics;
- a polished, responsive, shadcn-based Desktop workspace with one accessible interaction language across sources, documents, graph exploration, memory, services, backups, and updates;
- a narrow agent interface that works across supported harnesses without making any harness the canonical brain.

## Authority boundaries

- External sources remain authoritative for their original content.
- Cortana owns its normalized local evidence, derived indexes, native memory, and ContextBundles.
- Native memory contains bounded conclusions, not full source documents or transcripts.
- Harness scratchpads, Agent HQ ProjectState, Control Plane ContextPackages, workflow checkpoints, and native sessions remain owned by their respective systems.
- Evidence may support citations. Native memory is operational context and cannot substitute for source evidence.
- Retrieval does not mutate evidence, memory, ProjectState, or task state.

## Safety principles

- A fresh installation starts query-only.
- Authorization, validation, ingestion, reconciliation, recurring scheduling, model use, shared-agent access, backup restore, and memory writes are separate explicit actions.
- Partial or failed source operations never gain deletion authority.
- Embedding or synthesis failure degrades safely.
- Workspace and ACL boundaries are enforced before content is serialized.
- Credentials and private content are excluded from default audit telemetry.
- The Desktop renderer receives only narrow, typed, allowlisted native capabilities.
- Derived graph or memory representations retain provenance, scope, confidence where applicable, and invalidation behavior.
- The standalone local product remains useful without Agent HQ, the Control Plane, a hosted service, or an external memory engine.

## Completion criteria

Product claims must be supported by the appropriate evidence lane:

- deterministic tests for contracts and regressions;
- approved-corpus evaluation for representative retrieval and answer quality;
- source-specific validation and bounded trials for connector activation;
- real packaged-app acceptance for Desktop behavior;
- matched responsive and theme screenshot evidence for Desktop visual-system changes;
- artifact signing and operating-system checks for distribution trust;
- backup and recovery drills for durability;
- least-privilege integration tests for shared agents and first-party clients.

The exact live status of those criteria belongs in GitHub, not this document.

## Planning and evidence ownership

[GitHub milestones](https://github.com/adea-ai/cortana/milestones) and [GitHub issues](https://github.com/adea-ai/cortana/issues) are canonical for current scope, sequence, ownership, dates, dependencies, blockers, status, and task-level acceptance evidence.

[Release history](releases.md) owns tagged release evidence. [Evaluation](evaluation.md), [Operations](operations.md), [Source rollout](source-rollout.md), and [Desktop UX audit](desktop-ux-audit.md) define methods and evidence-retention rules.

See [Planning and tracking](planning.md).
