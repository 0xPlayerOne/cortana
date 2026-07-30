# Agent Instructions

These instructions are the repository-level operating contract for coding agents. They complement `.github/CONTRIBUTING.md`.

## Architecture

Cortana is a local-first agent-native second brain with three language stacks:

- **Rust** (`src/`, `Cargo.toml`): Core runtime — CLI, HTTP API, MCP server, storage (SQLite WAL + FTS5), embedding pipeline, retrieval, service supervision. Edition 2024, toolchain `1.88.0`.
- **TypeScript** (`apps/web/`): Vite + React 19 workspace UI. Bun workspace with `@cortana/web`.
- **Python** (`src/cortana/`): Connector SDK — integrations for Google, Slack, Discord, Apple Notes, Buzz, chat, filesystem. Packaged as a wheel with `hatchling`, exposed via `cortana-connectors` entrypoint.

The workspace and MCP server serve the same retrieval pipeline; the Vite dev server proxies `/v1` and `/healthz` to the Rust server at `127.0.0.1:7331`.

## Commands

```bash
# Rust
cargo test
cargo build --release
./target/release/cortana doctor
./target/release/cortana serve --address 127.0.0.1:7331

# Offline mode uses deterministic local embeddings — a different fingerprint from production.
# Offline and production indexes cannot be mixed.
./target/release/cortana --offline search "query"

# Workspace (Bun, not npm)
bun install --frozen-lockfile
bun run format        # prettier --check .
bun run format:write  # prettier --write .
bun run lint          # eslint apps/web/src
bun run typecheck     # tsc -b --pretty false
bun test              # bun test (not Vitest)
bun run build         # tsc -b && vite build

# Dev server (proxies API to the Rust server)
bun run dev           # vite, listens on 127.0.0.1:4173

# Connector SDK (uv, not pip)
uv sync --all-extras
uv run pytest
uv run ruff check .
uv run mypy
```

Use `--frozen-lockfile` with `bun install`. The lockfile `bun.lock` must stay in sync; do not regenerate it without cause.

## Validation order

```bash
bun run format && bun run lint && bun run typecheck && bun test
cargo fmt --check && cargo clippy -- -D warnings && cargo test
uv run ruff check . && uv run mypy && uv run pytest
```

CI runs these via `code-foundry` reusable workflows (`.github/code-foundry.yml`, `v0.27.17`). Do not edit CI YAML files directly — they are thin wrappers around the shared workflow.

## Conventions that differ from defaults

- **No semicolons** in TS/JS/TSX. Single quotes, trailing commas in ES5 positions, print width 100 (`.prettierrc`).
- **No Vitest.** Bun's native test runner (`bun test`) is required.
- **Prefer `bun` over `npm`** for all package management and scripts.
- **Rust edition 2024.** Features in `Cargo.toml` may be newer than stable defaults.
- **SQLite WAL mode** with `bundled` feature — no external SQLite dependency.
- **Embedding dimension** is 1024 (Qwen/Qwen3-Embedding-0.6B). Fingerprint prevents mixing vector spaces.
- **Sync operations** acquire an exclusive `fs2` lock (`sync.lock`). Only one sync can run at a time.
- **Embedding concurrency** is bounded to 8 requests per batch with 4 concurrent requests (half of local TEI defaults).
- **Connector spools** are written to `$DATA_DIR/staging/connector-*.jsonl` with mode `0600` and cleaned up after use.

## Branching and releases

Branch from `staging`, target PRs at `staging`. `main` is the protected release branch. Merge strategy is `rebase`.

Use Conventional Commits: `fix:` → patch, `feat:` → minor, `!` or `BREAKING CHANGE:` → major. Release automation (`release-please`) runs after changes reach `main`. Release type is `rust` in `code-foundry.yml`.

## Gotchas

- `bun install` without `--frozen-lockfile` will mutate `bun.lock`. Always use `--frozen-lockfile`.
- `cortana init` skips if the config file already exists; it does not overwrite.
- `migrate-hermes` refuses to replace an existing config without `--force`. It never prints credential values.
- `import-embeddings` validates every vector's fingerprint and dimension. A truncated or invalid export cannot delete previously imported data.
- `serve --allow-remote` requires `--api-token-env`; the token env var must be set.
- Changing the embedding model invalidates the index fingerprint. Re-ingestion is required.
- Python tests use `pythonpath = ["src"]` and `testpaths = ["tests"]` in pytest config. Coverage must stay at or above 85% (branch coverage).

## Completion report

End every agent task with:

```text
Summary:
Files changed:
Validation:
Skipped checks:
Risks or follow-up:
Branch/PR:
```