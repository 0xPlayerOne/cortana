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

# Ingest normalized documents, then retrieve structured cited evidence.
./target/release/cortana ingest documents.jsonl
./target/release/cortana sync
./target/release/cortana search "how do releases work?" --project engineering

# Agent transport and workspace API use the identical retrieval pipeline.
./target/release/cortana mcp
# `bun run build` packages the Obsidian-like workspace served at this address.
./target/release/cortana serve --address 127.0.0.1:7331
# API-only deployments can opt out of static workspace serving.
./target/release/cortana serve --address 127.0.0.1:7331 --no-web

# Retrieve the same citation-ready, token-bounded bundle used by the workspace and MCP.
curl -sS http://127.0.0.1:7331/v1/context \
  -H 'content-type: application/json' \
  -d '{"query":"how do releases work?","project":"engineering","max_tokens":8000}'
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
See the [operations guide](docs/operations.md) for service management, authenticated remote access,
telemetry, backup, restore, and Linux systemd units.
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

The shipped local profile uses SQLite WAL, FTS5, content-addressed incremental updates, a persistent
embedding cache, and an embedding index generation fingerprint. Postgres with `pgvector` is the
planned multi-user store; the canonical model intentionally does not depend on either backend.
OpenAI-compatible embedding APIs make the existing local Qwen/TEI service and cloud embedding
providers interchangeable without mixing vector spaces inside an index generation.

Hindsight is retained as an optional derived memory adapter for temporal/reflection workflows. It
is not the system of record: source evidence, provenance, permissions, and retrieval remain native
to Cortana. Honcho will be evaluated behind the same optional memory interface after the canonical
pipeline is measurable.

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
