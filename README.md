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

Use the published build unless you are developing Cortana itself. Do not run both installation
methods.

### Install the published build

1. Install [`uv`](https://docs.astral.sh/uv/getting-started/installation/). Cortana uses it for
   its connector environment.
2. Open the [Cortana releases](https://github.com/0xPlayerOne/cortana/releases) page and download
   the `.tar.gz` file for your operating system and CPU. For example, Apple Silicon uses
   `aarch64-apple-darwin`.
3. Extract the downloaded file and enter the extracted directory. The directory name starts with
   `cortana-v`.

   ```bash
   tar -xzf cortana-v<version>-<platform>.tar.gz
   cd cortana-v<version>-<platform>
   ```

4. Run the installer from that directory:

   ```bash
   ./install.sh
   ```

   This installs the `cortana` command, web UI, connector environment, and initial configuration
   under your user account. The `install.sh` file is included in release archives; it is not part
   of a Git checkout.

5. Configure the embedding provider and the sources you want to index in
   `~/.config/cortana/config.toml`. Use [`config.example.toml`](config.example.toml) as the
   reference. Cortana does not download an embedding model. The default local profile expects a
   `text-embeddings-router` process on `127.0.0.1:6999`; alternatively configure an accessible
   OpenAI-compatible embedding endpoint.
6. Check the configuration and embedding endpoint:

   ```bash
   cortana doctor
   ```

7. Run the first ingestion manually after configuring sources:

   ```bash
   cortana sync
   ```

8. Open the workspace at <http://127.0.0.1:7331>.

On macOS, the installer also starts four per-user jobs: the embedding supervisor, the API and web
UI, ingestion every 15 minutes, and verified backups once a day. It does not configure agent MCP
clients. On Linux, the installer installs the files but does not register background services; use
the [Linux service instructions](docs/operations.md#linux-systemd).

To install the bundled agent skill after installation, run the copy of
`scripts/install-agent-integrations.sh` included in the extracted release directory with
`CORTANA_INSTALL_AGENT_INTEGRATIONS=1 ./install.sh`, or configure your MCP client to run
`cortana --config ~/.config/cortana/config.toml mcp`.

### Build from a Git checkout (developers only)

From the repository root, run `./scripts/install-local.sh`. It builds the Rust binary and web UI,
creates the connector environment, initializes configuration when needed, and installs the same
macOS background jobs. It does not install an embedding model or configure sources. Continue with
steps 5–8 above, then use the [development commands](#development) when working on the code.

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
