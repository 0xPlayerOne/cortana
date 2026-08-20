# Cortana

Cortana is a local-first, agent-native second brain. It continuously indexes the places where
knowledge is already created, retrieves evidence with hybrid search, and exposes the same narrow,
structured primitives through an MCP server, HTTP API, CLI, and human workspace.

The project follows the production lessons in Cerebras' “How We Built Our Knowledge Base”:

- one canonical evidence schema across heterogeneous sources;
- incremental ingestion with stable source IDs and deletion reconciliation;
- persistent content-addressed embedding reuse across ingestion and queries;
- lexical, semantic, IDF, and recency signals fused before reranking;
- source-level deduplication and surrounding-context expansion;
- project-scoped retrieval instead of unbounded “search everything”;
- small, low-latency MCP primitives that leave orchestration to the calling agent;
- planner → concurrent retrieval → synthesis for the human UI;
- an Obsidian-inspired browser with workspace/source/document navigation and a bounded hierarchical
  knowledge graph;
- provenance, access scope, audit events, and observability as core data.

## What Cortana is

Cortana is a private, local-first knowledge system for people and the agents that work with them.
It turns approved documents, messages, notes, calendars, and code into one searchable evidence
store, then exposes the same cited context through the Desktop app, MCP, HTTP, and CLI. It is
intended to become a durable second brain without sending a personal corpus to a hosted database.

Cortana is not an automatic backup or an unrestricted crawler. A new installation starts in
query-only mode: it does not authorize accounts, download model weights, index data, or install a
recurring sync until you explicitly approve each step. A failed readiness or source-validation
check is a safety stop, not an invitation to bypass the gate.

If you remember only one thing: install Desktop, create one workspace, validate one source, run
one small initial sync, and ask one cited question. The complete first-run checklist is in
[Getting started](docs/getting-started.md).

The product purpose and evidence-based definition of “production ready” are kept in the
[Project goal](docs/project-goal.md); release-specific proof and open gates remain in the linked
release, evaluation, operations, and Desktop audit pages.

### The simple user path

If you are new to Cortana, you do not need to understand connectors, embeddings, MCP, or service
files before trying it:

1. Download the Desktop installer from the [latest release](https://github.com/0xPlayerOne/cortana/releases/latest).
2. Approve the optional local tooling, or choose a cloud embedding provider.
3. Create one workspace and configure one source.
4. Run **Validate**, then confirm one small **Initial sync**.
5. Ask a question and check that the answer includes citations.

Everything else is an operator or contributor concern. Cortana remains query-only until each
source, service, and recurring-sync action is explicitly approved.

### Project purpose and current safety boundary

Cortana exists to give people and their agents one private, cited memory across notes, messages,
documents, calendars, and code. The canonical store, permissions, provenance, retrieval, MCP
tools, Desktop workspace, and CLI are one system; connectors are replaceable input adapters, not
separate databases. Local Qwen embeddings and OpenAI-compatible cloud providers share the same
contract, while content-addressed caching avoids repeating work when source content is unchanged.

The protected source and latest published release are **v0.34.0**. Release-assets workflow
`31975576411` completed all platform packages and the strict 18-asset verifier: all 18 release
assets, checksums, updater signatures, manifest, and packaged-core checks passed. The v0.34.0
package carries the production safety hardening from the protected `staging` → `main` flow:
mutating CLI startup and direct JSONL imports serialize on the global lock, imports and
evaluation fixtures have explicit resource ceilings, remote `/readyz` requires scoped bearer
access, native memory writes are idempotent and fenced, Desktop settings and schedules serialize
through a shared per-config lock, and Desktop sidecars publish atomically. v0.32.6 and earlier
release records remain historical evidence.

The verified host installation now reports `cortana 0.34.0`, matching the current published package;
the embedding and HTTP services are running in query-only mode and recurring sync remains
uninstalled. The provider-backed evaluation records below were collected under the earlier
v0.32.12 installation and remain historical fixture evidence, not a v0.34.0 personal-index proof.
When a checkout and downloaded application report different versions, trust the application version
for end-user behavior and use [Release history](docs/releases.md) to determine which source-tree
hardening has shipped.

## Download the latest release

For normal use, download Cortana from the
[latest GitHub release](https://github.com/0xPlayerOne/cortana/releases/latest) and choose the
package for your operating system and CPU. The current protected release is **v0.34.0**. Its
release-assets workflow is the active archive, checksum, updater-signature, and credential-free
packaged-core verification gate. The Desktop app still has
separate manual gates for macOS Developer ID notarization and first-run operating-system
interactions; those limits are documented in the
[Desktop audit](docs/desktop-ux-audit.md).

The v0.34.0 Desktop support matrix is **macOS Apple Silicon (arm64), Linux x86_64, and Windows
x86_64**. v0.34.0 does not publish an Intel macOS Desktop bundle; Intel macOS is unsupported for
this release. Rosetta execution or a core archive is not evidence of Intel Desktop support. A
future Intel policy change requires a matching signed bundle, updater signature, installer
verification, and native acceptance evidence.

### Desktop first launch (recommended)

1. Download and install the matching Desktop package from the release page.
2. Launch Cortana Desktop and approve only the tooling it offers to install (uv, Python, or the
   local embedding runtime). Cloud embeddings do not require the local embedding runtime.
3. In **Settings → Workspaces**, create or select a workspace, then configure one source and use
   **Authorize** or **Open provider setup**.
4. Run **Validate** with the small default budget. Validation is read-only and does not index or
   reconcile anything.
5. After the result is healthy, use the explicitly confirmed **Initial sync** action for a bounded
   trial. Review the source status and a cited query before increasing its budget or enabling a
   recurring schedule.

The Desktop app can remain in the tray while Cortana's local services run in the background. The
same installation can be used by agents through the optional Cortana skill and MCP integration;
agent configuration remains an explicit, one-time choice.

If you only want to use Cortana, stop here. The terminal commands below are for operators,
recovery, and contributors; they are not required for a normal Desktop installation.

### Choose the path that fits you

| If you are...                         | Start here                                                                           | What it covers                                                               |
| ------------------------------------- | ------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------- |
| Installing Cortana for the first time | [Getting started](docs/getting-started.md)                                           | Download, setup approval, one source, validation, and a safe trial sync      |
| Operating an existing installation    | [Operations guide](docs/operations.md)                                               | Services, readiness, backups, authentication, audit, and recovery            |
| Connecting an agent                   | [Agent integrations](docs/integrations.md)                                           | The portable skill, MCP, HTTP, CLI, and scoped principals                    |
| Adding or validating sources          | [Ingestion guide](docs/ingestion.md)                                                 | Source contracts, cursors, ACLs, budgets, and reconciliation safety          |
| Tuning retrieval or embeddings        | [Query guide](docs/query.md)                                                         | Hybrid retrieval, Qwen or cloud embeddings, synthesis, caching, and fallback |
| Contributing to Cortana               | [Development](#development) and [Desktop architecture](docs/desktop-architecture.md) | Local builds, tests, Tauri boundaries, and release packaging                 |

The safest first milestone is deliberately small: install the Desktop app, validate one source,
run one bounded non-reconciling trial, confirm a cited query, then write one explicit native memory.
Do not enable recurring ingestion until the relevant production gates in the [evaluation guide](docs/evaluation.md)
are complete.

## Quick start

Use the numbered CLI path below when you prefer terminal control, are recovering an installation,
or are contributing from a checkout. The release installer installs the published application
bundle; the checkout installer builds the application and connector runtime from source. Neither
path downloads embedding weights, authorizes a source, performs a first sync, or enables recurring
ingestion automatically.

### 1. Install the application

Choose one path:

1. **Release archive (recommended for normal use).** Extract a matching GitHub release archive and
   run `./install.sh`.
2. **Git checkout (contributors or unreleased changes).** Run `./scripts/install-local.sh` from the
   checkout. Set `CORTANA_INSTALL_SERVICE=0` when you want to install files without starting the
   per-user services yet.
3. **Agent integration (optional).** Add
   `CORTANA_INSTALL_AGENT_INTEGRATIONS=1` to the checkout installer, or run
   `./scripts/install-agent-integrations.sh` after installation. MCP client configuration remains
   an explicit agent-side step.

The installer preserves an existing configuration and index. It does not overwrite secrets, run a
connector, rebuild embeddings, or delete data. After installation, keep the application in its
safe query-only state until the source checks in the next steps pass.

### 2. Initialize and check the local runtime

From a checkout, use the built binary; from a release install, use the installed `cortana` command:

```bash
# A release install normally needs only the installed `cortana` command:
cortana --version
cortana init
cortana doctor
cortana readiness --max-backup-age-hours 48
```

`doctor` checks configuration and dependencies. `readiness` is read-only and confirms the database,
embedding provider, API, backup freshness, and that recurring sync is not installed. A readiness
failure is a stop sign; it never repairs the index implicitly. SQLite integrity and backup
verification run on dedicated blocking threads and are bounded to 240 seconds by default; use
`--storage-timeout-seconds <seconds>` (1–300) to set an explicit bound. A timeout fails readiness
closed.

### 3. Authorize and validate one source

Authorize only the source you intend to use, then run a small read-only validation. Google OAuth is
started with `cortana authorize-google SOURCE`; GitHub uses `cortana authorize-github SOURCE`
with a configured device-flow client id and private token destination; Discord uses
`cortana authorize-discord SOURCE` through the running Discord Desktop client with a configured
RPC client JSON and private RPC token destination to assign servers per workspace; Slack uses
`cortana authorize-slack SOURCE` with a
configured OAuth client JSON and private user-token destination to assign workspaces per
workspace (`cortana slack-workspaces SOURCE` lists the assigned workspace); Buzz uses
`cortana buzz-communities SOURCE` to list the bounded communities recorded in its read-only
`agents/teams.json` identity file for per-workspace assignment; Apple Notes uses the host
permission; token-backed sources read only the configured environment variable. Apple Notes
sources can be split across workspaces with exact `folders` and `exclude_folders` lists. For
example, create one Apple Notes source for `Nifty League` in `work`, one for `The Pink Binder`
in `special`, and a personal source with those folders excluded. Validation never embeds,
indexes, or reconciles data:

```bash
cortana validate-source SOURCE \
  --max-documents 25 \
  --max-bytes 5242880 \
  --max-seconds 60
```

Review the source result in Desktop or `/v1/status` before proceeding. Do not use a production-sized
budget as a validation shortcut. A filesystem root larger than the requested budgets fails closed
unless you explicitly pass `--sample`, which records a bounded sample that can authorize only an
equally bounded non-reconciling trial sync — never a full-corpus or recurring sync.

For a repeatable operator check across the configured sources, use the bounded smoke harness. It
reads only source names and kinds from the TOML file, never prints credentials, and exits nonzero
when authorization or validation fails. Filesystem/code validations pass `--sample`, so an
oversized root records a bounded sample instead of failing; connector sources keep ordinary
fail-closed validation:

```bash
scripts/source-smoke.sh --config "$HOME/.config/cortana/config.toml"
```

### 4. Run one bounded sync

Plan the source first, then run a deliberately small non-reconciling trial. A trial cannot delete
records that are missing from a partial snapshot:

```bash
cortana sync --source SOURCE --plan
cortana sync --source SOURCE \
  --max-documents 25 \
  --max-bytes 5242880 \
  --max-seconds 60 \
  --no-reconcile
```

Inspect the Desktop source panel or `/v1/status` and query a known item. Only after the connector,
cursor, ACL, and cache behavior is verified should you choose a larger complete snapshot. Recurring
sync remains a separate confirmation-gated operation.

To run the same bounded trial for every connector source after validation, add `--sync`. Trials
always pass `--no-reconcile` and use the same budgets as the validation, so a filesystem/code
trial (enabled with `--include-filesystem`) can rely on the matching `--sample` validation while
never authorizing a full-corpus sync:

```bash
scripts/source-smoke.sh --sync
```

Explicit trial probes retry only transient transport, timeout, rate-limit, and 5xx failures
once by default. Set `CORTANA_SOURCE_SMOKE_SYNC_ATTEMPTS=1` to disable retries (or `3` for the
maximum); authorization and configuration failures remain fail-fast.

### 5. Start, stop, and uninstall safely

The service commands affect only Cortana's per-user jobs and preserve configuration, data, logs,
and backups:

```bash
cortana service status --json
cortana service stop server
cortana service stop embedding
cortana service uninstall
```

Use `cortana service start NAME` or `cortana service install --web-dir PATH` to resume the core
services. Never remove the data directory as part of an ordinary uninstall; take and verify a
backup first if the index is no longer needed.

To verify recovery without replacing the live index, run the disposable drill from the checkout or
release archive:

```bash
CORTANA_CONFIG="$HOME/.config/cortana/config.toml" scripts/backup-restore-drill.sh
```

The remaining commands below are contributor and recovery examples. New users should use the
Desktop path above unless they specifically need terminal control.

```bash
# From an extracted GitHub release archive (binary, UI, and connector wheel).
./install.sh

# Reproducible per-user install (Rust binary, workspace, connector venv, and macOS services).
./scripts/install-local.sh

# Also install the portable Cortana skill for Codex and agents that support the
# shared ~/.agents/skills convention. Hermes/OpenCode roots require an explicit
# CORTANA_SKILL_ROOTS override so unrelated harnesses are never changed implicitly.
CORTANA_INSTALL_AGENT_INTEGRATIONS=1 ./scripts/install-local.sh

# Or run directly from a checkout.
cargo build --release
bun install --frozen-lockfile
bun run build

./target/release/cortana init

# Verify the configured Qwen/TEI or cloud OpenAI-compatible embedding endpoint.
./target/release/cortana doctor

# Plan and then run bounded ingestion; recurring background sync is opt-in.
# Direct JSONL ingest is capped at 2,000 documents, 128 MiB, 15 minutes, and 8 MiB per line.
# Split larger reviewed imports into separate batches.
./target/release/cortana ingest documents.jsonl
./target/release/cortana sync --source SOURCE --plan
# Fetch and validate one source without embedding, indexing, or reconciliation.
./target/release/cortana validate-source SOURCE --max-documents 25 --max-bytes 10485760 --max-seconds 60
# Run only the explicitly bounded, non-reconciling trial covered by that validation.
./target/release/cortana sync --source SOURCE --require-validation --no-reconcile \
  --max-documents 25 --max-bytes 5242880 --max-seconds 60
./target/release/cortana search "how do releases work?" --project engineering
# Same citation-ready, token-bounded bundle as MCP/HTTP, without a running server.
./target/release/cortana context "how do releases work?" --project engineering

# Agent transport, the workspace API, and the CLI use the identical retrieval pipeline.
./target/release/cortana mcp
# `bun run build` packages the Obsidian-like workspace served at this address.
./target/release/cortana serve --address 127.0.0.1:7331
# API-only deployments can opt out of static workspace serving.
./target/release/cortana serve --address 127.0.0.1:7331 --no-web

# Retrieve the same citation-ready, token-bounded bundle used by the workspace and MCP.
curl -sS http://127.0.0.1:7331/v1/context \
  -H 'content-type: application/json' \
  -d '{"query":"how do releases work?","project":"engineering","max_tokens":8000}'

# Human-facing planned answer. This stays extractive unless [query].synthesis_enabled is true.
curl -sS http://127.0.0.1:7331/v1/answer \
  -H 'content-type: application/json' \
  -d '{"query":"how do releases work?","project":"engineering"}'

# ACL-filtered canonical documents use bounded keyset pagination.
curl -sS 'http://127.0.0.1:7331/v1/documents?project=engineering&limit=50'
```

For an existing installation, run `./scripts/install-agent-integrations.sh`.
The script installs only skill files; MCP client configuration remains an
explicit, one-time agent setting. Point the MCP command at the installed
`cortana` binary with `--config <path> mcp`.

Use `--offline` for a deterministic, zero-network evaluation index. Offline and production
embeddings are intentionally fingerprinted as different index generations and cannot be mixed.
If only an endpoint fingerprint changed and you have verified that the stored vectors are still
the same model, dimension, and vector space, use the explicit `migrate-embedding --from ...
--force` command to adopt the generation without a corpus rebuild; otherwise rebuild or import
vectors into a new generation. For a true model or preprocessing change, use the guarded
`rebuild-embeddings --from ... --force` command: it re-embeds every stored chunk behind a
recovery snapshot and only swaps the live vectors after the entire corpus succeeds.
See [the ingestion guide](docs/ingestion.md) and
[`config.example.toml`](config.example.toml) for Google Drive, Gmail, Calendar, Apple Notes,
GitHub code, Slack, Discord, Buzz, and filesystem/code sources.
See [the query guide](docs/query.md) for hybrid retrieval, cited synthesis, local model-gateway
configuration, cloud providers, cache invalidation, and degraded operation.
See the [agent integration guide](docs/integrations.md) for Codex, Hermes, Buzz, MCP, HTTP, and
CLI setup with scoped principals, native memory, and cache-aware context retrieval.
See the [operations guide](docs/operations.md) for service management, authenticated remote access,
telemetry, backup, restore, and Linux systemd units.
Run the isolated [evaluation and readiness gates](docs/evaluation.md) before enabling synthesis or
recurring ingestion.
See the [desktop architecture](docs/desktop-architecture.md) for the Tauri trust boundary,
background lifecycle, contributor builds, and native release packaging.
See [release history](docs/releases.md) for the automated version-PR policy and transitional
release notes.
See the [documentation index](docs/README.md) for the complete guide map.

### Migrate an existing Hermes second brain

Cortana can copy only reusable Google OAuth tokens and supported chat tokens from Hermes, lock
every migrated credential to mode `0600`, and generate equivalent source configuration. Existing
Chroma indexes are reported and retained until Cortana has imported or rebuilt and
verified their data:

```bash
cortana migrate-hermes \
  --connector-command "$HOME/.local/share/cortana/venv/bin/cortana-connectors"
cortana doctor
cortana sync
```

The migration stages credentials, configuration, and its report before publishing them. If any
publication step fails, previously existing files are restored and temporary migration files are
removed, so a partial migration cannot leave the active installation in a mixed state.

When the legacy Chroma collections use the configured embedding fingerprint, migrate their
existing vectors without an expensive re-embedding pass:

```bash
"$HOME/.hermes/code-index-venv/bin/python" scripts/export_chroma.py \
  --chroma-dir "$HOME/.hermes/code-index/chroma" |
  cortana import-embeddings -
```

The import is streaming, validates every vector's model fingerprint and dimension, seeds the
shared embedding cache, and reconciles only after the complete input is read. A truncated or
invalid export therefore cannot delete the prior imported snapshot.

Migration refuses to replace an existing Cortana configuration unless `--force` is explicit. It
never prints credential values and recognizes only `SLACK_BOT_TOKEN` from legacy environment files;
legacy Discord environment credentials are intentionally not migrated.

## Architecture

```text
Sources -> adapters -> normalized documents -> chunk/distill -> embeddings
                            |                         |
                            v                         v
                     canonical store <---- lexical + vector indexes
                            |
                  project/access scoped retrieval
                     /             |             \
                   MCP           HTTP/UI         CLI
```

The core runtime is Rust: service supervision, normalization contracts, storage, indexing,
retrieval, HTTP, MCP, and the CLI. Python is an isolated connector SDK for integrations where
mature vendor libraries or macOS automation are the safer boundary.

The human workspace calls `/v1/answer`: a bounded planner can fan out up to eight focused searches,
the runtime executes them concurrently, reciprocal-rank fusion deduplicates evidence, and a
configured model produces a citation-validated answer. Authorized principals also receive matching
native memory as separate operational context; numbered source evidence remains the citation
authority. Every model failure degrades to a deterministic extractive answer. MCP intentionally
exposes raw search and token-bounded context as well, allowing an agent to orchestrate without
paying for an opaque extra synthesis call.
Task-specific `search_code`, `search_messages`, and `who_knows` tools reuse one query embedding
across their configured source group. Ranked passages receive one ACL-checked neighboring chunk
on either side, capped at 16 KiB per result before the independent context-token budget.

The shipped local profile uses SQLite WAL, FTS5, content-addressed incremental updates, a persistent
embedding cache, and an embedding index generation fingerprint. Postgres with `pgvector` is the
planned multi-user store; the canonical model intentionally does not depend on either backend.
OpenAI-compatible embedding APIs make the existing local Qwen/TEI service and cloud embedding
providers interchangeable without mixing vector spaces inside an index generation.

Agentic memory is native to Cortana's canonical SQLite store. Explicit `remember`, `recall`,
`forget`, expiry, supersession, and scoped export operations share the same provenance, workspace
ACL, audit, backup, and cache-revision semantics as knowledge retrieval. See
[Native agentic memory](docs/memory.md) for the built-in operational memory layer. Recall stays
provider-free: local FTS candidates are ranked by query coverage, lexical match, salience, and
freshness, with ACL and lifecycle checks applied before results are returned.

## Development

```bash
rustup show
cargo test

# Workspace
bun install --frozen-lockfile
bun run format
bun run lint
bun run type-check
bun run test
bun run build

# Tauri 2 desktop
bun run desktop:test
bun run --cwd apps/desktop clippy
bun run desktop:build
# macOS-only unsigned local application bundle
bun run desktop:bundle:mac

# Connector SDK
uv sync --all-extras
uv run pytest
uv run ruff check .
uv run mypy
```

Never commit source credentials or personal data. Local configuration will live outside the
checkout and environment examples contain names only.

## License

AGPL-3.0-only.
