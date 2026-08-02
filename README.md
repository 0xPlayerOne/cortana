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
- provenance, access scope, audit events, and observability as core data.

## Quick start

```bash
# From an extracted GitHub release archive (binary, UI, and connector wheel).
./install.sh

# Reproducible per-user install (Rust binary, workspace, connector venv, and macOS services).
./scripts/install-local.sh

# Also install the portable Cortana skill for Codex, Hermes, OpenCode, and
# agents that support the shared ~/.agents/skills convention.
CORTANA_INSTALL_AGENT_INTEGRATIONS=1 ./scripts/install-local.sh

# Or run directly from a checkout.
cargo build --release
bun install --frozen-lockfile
bun run build

./target/release/cortana init

# Verify the configured Qwen/TEI or cloud OpenAI-compatible embedding endpoint.
./target/release/cortana doctor

# Plan and then run bounded ingestion; recurring background sync is opt-in.
./target/release/cortana ingest documents.jsonl
./target/release/cortana sync --source SOURCE --plan
# Fetch and validate one source without embedding, indexing, or reconciliation.
./target/release/cortana validate-source SOURCE --max-documents 25 --max-bytes 10485760 --max-seconds 60
./target/release/cortana sync --source SOURCE
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
See [the ingestion guide](docs/ingestion.md) and
[`config.example.toml`](config.example.toml) for Google Drive, Gmail, Calendar, Apple Notes, Slack,
Discord, Buzz, filesystem/code, and external adapters.
See [the query guide](docs/query.md) for planned retrieval, cited synthesis, local model-gateway
configuration, cloud providers, cache invalidation, and degraded operation.
See the [operations guide](docs/operations.md) for service management, authenticated remote access,
telemetry, backup, restore, and Linux systemd units.
Run the isolated [evaluation and readiness gates](docs/evaluation.md) before enabling synthesis or
recurring ingestion.
See the [desktop architecture](docs/desktop-architecture.md) for the Tauri trust boundary,
background lifecycle, contributor builds, and native release packaging.
See [release history](docs/releases.md) for the automated version-PR policy and transitional
release notes.

### Migrate an existing Hermes second brain

Cortana can copy only reusable Google OAuth tokens and supported chat tokens from Hermes, lock
every migrated credential to mode `0600`, and generate equivalent source configuration. Existing
Chroma and Hindsight indexes are reported and retained until Cortana has imported or rebuilt and
verified their data:

```bash
cortana migrate-hermes \
  --connector-command "$HOME/.local/share/cortana/venv/bin/cortana-connectors" \
  --discord-channel 123456789012345678
cortana doctor
cortana sync
```

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
never prints credential values and recognizes only `DISCORD_BOT_TOKEN` and `SLACK_BOT_TOKEN` from
legacy environment files.

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
configured model produces a citation-validated answer. Every model failure degrades to a
deterministic extractive answer. MCP intentionally exposes raw search and token-bounded context
instead, allowing an agent to orchestrate without paying for an opaque extra synthesis call.
Task-specific `search_code`, `search_messages`, and `who_knows` tools reuse one query embedding
across their configured source group. Ranked passages receive one ACL-checked neighboring chunk
on either side, capped at 16 KiB per result before the independent context-token budget.

The shipped local profile uses SQLite WAL, FTS5, content-addressed incremental updates, a persistent
embedding cache, and an embedding index generation fingerprint. Postgres with `pgvector` is the
planned multi-user store; the canonical model intentionally does not depend on either backend.
OpenAI-compatible embedding APIs make the existing local Qwen/TEI service and cloud embedding
providers interchangeable without mixing vector spaces inside an index generation.

Hindsight is retained as an optional derived memory adapter for temporal/reflection workflows, and
Honcho now has a bounded session adapter behind the same durable outbox. Neither is the system of
record: source evidence, provenance, permissions, and retrieval remain native to Cortana. Both
remain disabled until the versioned evaluation, replacement, and deletion/ACL gates pass; see the [Hindsight
outbox guide](docs/memory-hindsight-outbox.md) and [Honcho adapter contract](docs/memory-honcho.md).

## Development

```bash
rustup show
cargo test

# Workspace
bun install --frozen-lockfile
bun run format
bun run lint
bun run typecheck
bun test
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
