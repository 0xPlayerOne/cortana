# Self-hosted single-node deployment

The supported self-hosted profile runs the same Cortana binary, SQLite store, ContextBundle,
memory, MCP, HTTP, and CLI contracts as Local. It is a user-controlled single node for a private
workstation, home server, or Linux VPS. It is not managed hosting, multi-writer SQLite, cross-device
brain synchronization, team tenancy, or a remote connector fleet.

## Requirements and measured baseline

The image supports `linux/amd64` and `linux/arm64`. The M8 reference build was measured on an arm64
Docker host with an empty synthetic corpus: the image was 87,706,821 bytes, the healthy service used
3.406 MiB resident memory and 10 PIDs at idle, and liveness became available within the first
one-second probe interval. These numbers exclude an external embedding service and corpus growth.

Use at least 1 CPU, 1 GiB RAM, and 10 GiB durable disk for a small corpus with an external embedding
provider. The reference Compose limit and recommendation are 2 CPUs and 2 GiB for Cortana, plus the
embedding provider's own measured allocation. Reserve disk for the image, SQLite/WAL growth, model
assets, and at least two verified backups; a practical starting allocation is 20 GiB plus three
times the expected canonical database size. Re-measure with the approved corpus before raising
ingestion limits.

## Generic Linux and Hostinger-compatible path

Install Docker Engine with the Compose plugin, clone or download the matching release deployment
directory, and work from `deploy/self-hosted`. Hostinger's Docker-capable VPS uses the same steps;
point the domain's DNS record at the VPS before enabling the TLS overlay.

1. Copy `env.example` outside source control, replace the owner token with
   `openssl rand -base64 48`, and load it with `set -a; . /private/path/cortana.env; set +a`.
2. Pin `CORTANA_VERSION` to a release tag. Review `config.toml`. Its default embedding endpoint is
   an owner-operated OpenAI-compatible service on the Docker host. Selecting an HTTPS cloud
   embedding provider is an explicit corpus-upload decision.
3. For private host-only access, run `docker compose -f compose.yaml up -d`. The API is published
   only on `127.0.0.1:7331`; reach it through an SSH tunnel or another owner-approved private path.
4. For public-network transport, set `CORTANA_DOMAIN` and run
   `docker compose -f compose.yaml -f compose.tls.yaml up -d`. The overlay removes the host API
   port and exposes only Caddy on 80/443. Every non-liveness Cortana request still requires a scoped
   bearer principal.

The container binds `0.0.0.0` only inside its private Compose network and starts with
`--allow-remote`; Cortana refuses that mode unless at least one bearer principal resolves. The
process runs as UID/GID 10001, drops Linux capabilities, uses a read-only root filesystem, limits
PIDs/log rotation/CPU/RAM, and receives graceful termination through `tini` with a 45-second drain.

## Persistent state and single-writer rule

`cortana_data` owns the canonical SQLite database, WAL, source validation state, and other required
operational metadata. `cortana_backups` is mounted at the database backup directory, and
`cortana_models` is reserved for optional owner-approved local model assets. `config.toml` is a
read-only bind mount; credentials arrive only through the runtime environment or a supported local
secret mechanism.

Only one Cortana process may mount a canonical data volume read-write. NFS/SMB/shared-filesystem
SQLite and active-active replicas are unsupported and must be rejected operationally. A read-only
backup copy is not a second live writer.

## Operations and drills

Use `Authorization: Bearer $CORTANA_OWNER_TOKEN` for authenticated probes. `/healthz` is liveness;
`/readyz`, `/v1/status`, and `/v1/provider/capabilities` require the bearer on this remote listener.
Logs go to Docker's bounded JSON log driver.

- Backup: `docker compose exec cortana cortana --config /etc/cortana/config.toml backup --keep 14`.
  Copy a verified snapshot off-host after creation.
- Verify: `docker compose exec cortana cortana --config /etc/cortana/config.toml verify` and verify
  the off-host snapshot separately before relying on it.
- Restore: stop Cortana, retain the current data volume, run the documented `restore --force`
  command against a verified snapshot in a one-off container, then start and probe readiness.
- Update: create and verify a backup, pin the new release tag, pull, recreate, and run provider
  conformance before deleting the old image.
- Rollback: stop, restore the pre-update snapshot if a migration changed the database, pin the prior
  release tag, recreate, and rerun conformance. Never start old and new versions on one volume.
- Restart: write a synthetic memory with a dedupe key, restart the container or host, then recall it
  and compare its ID/revision. The M8 reference drill preserved the same memory ID across restart.
- Corruption: run `verify` against a disposable known-invalid fixture. It must fail closed with
  `file is not a database`; never test by corrupting the active volume.
- Low disk: monitor the data and backup volumes, stop ingestion before free space falls below the
  larger of 2 GiB or twice the latest verified backup, create off-host capacity, then resume. A
  failed backup is not a valid recovery point.

Container recreation and host restart preserve evidence, memory, ACLs, revisions, source
configuration, and audit/security state because those records live in the durable volumes and
read-only config, not the container layer.
