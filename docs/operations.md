# Operations

Cortana is designed to run as a private per-user service. The default server binds only
`127.0.0.1:7331`; use a TLS-terminating reverse proxy for network access. A non-loopback bind is
refused unless both `--allow-remote` and `--api-token-env NAME` are provided. The workspace stores
that bearer token only in browser session storage.

## Health and telemetry

- `GET /healthz` is an unauthenticated process-liveness check.
- `GET /readyz` verifies both the SQLite index and a real embedding request.
- `GET /v1/status` reports source freshness, index counts, runtime counters, and cache telemetry.
- `GET /metrics` exports low-cardinality Prometheus metrics.

HTTP requests emit structured tracing spans to stderr. Set `RUST_LOG`, for example
`RUST_LOG=cortana=debug,tower_http=info`, to change verbosity. Request headers and evidence content
are never logged.

## Backup and recovery

`cortana backup` creates a consistent online SQLite snapshot with `VACUUM INTO`, runs a full
integrity check, and retains the newest 14 scheduled snapshots by default:

```bash
cortana --config ~/.config/cortana/config.toml backup
cortana --config ~/.config/cortana/config.toml verify /path/to/snapshot.sqlite3
```

Stop the server before recovery. Restore requires explicit confirmation and automatically preserves
the previous database as a verified `pre-restore-*.sqlite3` snapshot:

```bash
cortana service uninstall
cortana --config ~/.config/cortana/config.toml restore /path/to/snapshot.sqlite3 --force
cortana --config ~/.config/cortana/config.toml service install --web-dir /path/to/web
```

Test recovery periodically on a copy. A backup that has never been restored is not a proven
recovery path.

## macOS launchd

Build the release binary and workspace first, then install four per-user jobs: local embedding
supervision, the API/workspace, 15-minute ingestion, and daily verified backups.

```bash
cargo build --release
bun run build
./target/release/cortana --config ~/.config/cortana/config.toml service install \
  --web-dir ./apps/web/dist
./target/release/cortana service status
```

Use `--no-embedding-service` for a cloud embedding provider. Logs are written beneath
`data_dir/logs`. `service uninstall` stops and removes only Cortana's four launchd jobs; it does not
delete configuration, data, logs, or backups.

## Linux systemd

Templates live in [`packaging/systemd`](../packaging/systemd). Install the binary, built workspace,
and user-unit files at the paths shown in the templates, then run:

```bash
systemctl --user daemon-reload
systemctl --user enable --now cortana-embedding.service cortana.service
systemctl --user enable --now cortana-sync.timer cortana-backup.timer
```

For cloud embeddings, omit `cortana-embedding.service`. Adjust `ReadWritePaths` when `data_dir`
differs from the XDG default.

## Secrets

An optional `[runtime].env_file` supplies connector, cloud-provider, and HTTP-token environment
variables without putting values in launchd or systemd definitions. On Unix, Cortana refuses to
read this file if any group or other permission bit is set. Use mode `0600`; process environment
variables take precedence.
