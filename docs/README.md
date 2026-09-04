# Cortana documentation

Cortana documentation is divided by authority. Each document should define one durable concern and avoid duplicating current planning or release status.

## Start here

| Need                                                    | Canonical document                                                                                                                |
| ------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Product purpose and user promise                        | [Project goal](project-goal.md)                                                                                                   |
| Install and first use                                   | [Getting started](getting-started.md)                                                                                             |
| Current work, ownership, sequence, blockers, and status | [GitHub milestones](https://github.com/adea-ai/cortana/milestones) and [GitHub issues](https://github.com/adea-ai/cortana/issues) |
| Tagged release evidence                                 | [Release history](releases.md)                                                                                                    |
| Source lifecycle and connector rules                    | [Ingestion](ingestion.md)                                                                                                         |
| Source activation procedure                             | [Source rollout](source-rollout.md)                                                                                               |
| Retrieval, context, synthesis, and caching              | [Query](query.md)                                                                                                                 |
| Native memory lifecycle                                 | [Memory](memory.md)                                                                                                               |
| Agent setup and interface use                           | [Integrations](integrations.md)                                                                                                   |
| Desktop architecture and privilege boundary             | [Desktop architecture](desktop-architecture.md)                                                                                   |
| Desktop packaged-product acceptance                     | [Desktop UX audit](desktop-ux-audit.md)                                                                                           |
| Desktop shadcn migration baseline and architecture      | [Desktop shadcn migration record](desktop-shadcn-migration.md)                                                                    |
| Services, readiness, backup, restore, and recovery      | [Operations](operations.md)                                                                                                       |
| Derived Obsidian-compatible Markdown export             | [Derived Obsidian vault](obsidian-vault.md)                                                                                       |
| Evaluation methods and evidence                         | [Evaluation](evaluation.md)                                                                                                       |
| Planning and documentation ownership                    | [Planning and tracking](planning.md)                                                                                              |
| Architecture diagrams                                   | [Architecture](architecture/)                                                                                                     |
| Canonical entities and persistence                      | [Core entity contract](contracts/core-entities.md)                                                                                |
| Structured retrieval units                              | [Structured chunking](chunking.md)                                                                                                |
| Revision-aware code indexes and retrieval               | [Code intelligence](code-intelligence.md)                                                                                         |
| ContextBundle identity and pinning                      | [ContextBundle contract](contracts/context-bundle.md)                                                                             |
| Connector and reconciliation semantics                  | [Connector contract](contracts/connectors.md)                                                                                     |
| Workspace, principal, and ACL mapping                   | [Identity contract](contracts/identity.md)                                                                                        |
| Public API compatibility                                | [Public API contract](contracts/public-api.md)                                                                                    |
| Knowledge graph identity and relationship semantics     | [Knowledge graph contract](contracts/knowledge-graph.md)                                                                          |
| ContextProvider compatibility and fixtures              | [ContextProvider contract](contracts/context-provider.md)                                                                         |
| Supported single-node container and VPS operations      | [Self-hosted deployment](self-hosted.md)                                                                                          |
| Security assets and trust boundaries                    | [Security and trust model](security-model.md)                                                                                     |
| Native memory taxonomy                                  | [Memory contract](contracts/memory.md)                                                                                            |
| Bounded non-mutating memory reasoning                   | [Reflection contract](contracts/reflection.md)                                                                                    |
| Extension policy                                        | [Extensions](EXTENSIONS.md)                                                                                                       |

## Authority model

### GitHub

[GitHub milestones](https://github.com/adea-ai/cortana/milestones) and [GitHub issues](https://github.com/adea-ai/cortana/issues) are canonical for:

- current product and implementation status;
- milestone scope and sequence;
- ownership and assignees;
- dates and dependencies;
- blockers and risks;
- task-level acceptance criteria and evidence;
- superseded or deferred work.

Canonical documents must not maintain parallel milestone lists, current-status tables, or issue backlogs.

### Product and architecture documents

Product documents define what and why. Technical design documents define how. Specifications define exact schemas and contracts. ADRs explain durable decisions and their consequences.

These documents may define acceptance requirements, safety gates, invariants, and ownership boundaries. They must not claim that a gate currently passes unless they link to immutable release or test evidence owned elsewhere.

### Release and evidence documents

[Release history](releases.md) preserves immutable, version-specific evidence. [Evaluation](evaluation.md), [Operations](operations.md), [Source rollout](source-rollout.md), and [Desktop UX audit](desktop-ux-audit.md) define methods and the evidence that must be retained.

Dated evidence may be linked from a release record or GitHub issue. It should not become a competing roadmap.

## Documentation rules

- Keep source-backed evidence separate from native memory.
- Keep release/package integrity separate from packaged GUI and operating-system acceptance.
- Keep authorization, validation, ingestion, reconciliation, scheduling, and deletion as distinct actions.
- Mark derived or inferred graph and memory representations explicitly.
- Preserve exact source provenance and stable terminology.
- Link the first meaningful reference to the owning document instead of redefining it.
- Move unresolved implementation work to GitHub issues.
- Record durable architecture changes in ADRs.
- Do not include credentials, private paths, query text, source content, or personal evaluation manifests in documentation or default audit output.

## Planning links

- [Milestones](https://github.com/adea-ai/cortana/milestones)
- [Issues](https://github.com/adea-ai/cortana/issues)
- [Pull requests](https://github.com/adea-ai/cortana/pulls)
- [Latest release](https://github.com/adea-ai/cortana/releases/latest)
