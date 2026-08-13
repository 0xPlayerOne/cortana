# Agent integrations

Cortana exposes the same retrieval pipeline through a portable agent skill, an MCP stdio server, a
loopback HTTP API, and the CLI. Agents should start with the skill's `context` primitive (MCP,
HTTP, or CLI) and treat `search_code`, `search_messages`, and `who_knows` as targeted evidence
tools; see [the skill](../skills/cortana/SKILL.md) for the full retrieval protocol and
[the query guide](query.md) for pipeline details. This guide covers installation and client
configuration only.

## Install the portable skill

`scripts/install-agent-integrations.sh` installs the skill into the current Codex and
`~/.agents/skills` roots by default:

```bash
./scripts/install-agent-integrations.sh
```

It installs only the skill files (`SKILL.md` plus `agents/openai.yaml`). MCP client configuration
remains an explicit, one-time setting per client — the script never edits client configuration.
Hermes and OpenCode roots are legacy integrations and are never modified implicitly; add them
explicitly to `CORTANA_SKILL_ROOTS` when those clients are intentionally in scope.
The same install runs automatically when `install-local.sh` is invoked with
`CORTANA_INSTALL_AGENT_INTEGRATIONS=1`.

Defaults, all overridable with environment variables:

| Setting     | Default                                                                                                     |
| ----------- | ----------------------------------------------------------------------------------------------------------- |
| Binary      | `$HOME/.local/bin/cortana` (`CORTANA_BINARY`)                                                               |
| Config      | `$HOME/.config/cortana/config.toml` (`CORTANA_CONFIG`)                                                      |
| Skill roots | `$HOME/.codex/skills:$HOME/.agents/skills` (`CORTANA_SKILL_ROOTS`; Hermes/OpenCode require explicit opt-in) |

The installed skill instructs agents to prefer the configured MCP server first, fall back to
`cortana context`, and only then use raw search. Client configuration examples below use absolute
paths because MCP clients may launch the server from arbitrary working directories.

## Interface overview

MCP and HTTP/CLI share one retrieval contract: queries must be non-empty and at most 16 KiB, scope
filters are bounded, each tool returns at most 50 evidence rows, and the context builder applies an
independent token budget (`--limit` 1–50, `--max-tokens` 256–64,000, defaulting to the configured
`[query].context_tokens` budget of 8,000).

| Interface                                       | Entry points                                                                                                                                                                            |
| ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| MCP stdio (`cortana --config <path> mcp`)       | `context`, `search`, `search_code`, `search_messages`, `who_knows`, `brain_status`                                                                                                      |
| HTTP (`cortana serve --address 127.0.0.1:7331`) | `POST /v1/context`, `POST /v1/search`, `POST /v1/answer`, `GET /v1/documents[/{id}]`, `GET /v1/graph`, `GET /v1/status`, `GET /v1/audit`, `GET /healthz`, `GET /readyz`, `GET /metrics` |
| CLI (no server required)                        | `cortana context`, `cortana search` (raw-evidence fallback)                                                                                                                             |

`cortana context QUERY`, `POST /v1/context`, and the MCP `context` tool return the same
citation-ready, token-bounded Markdown bundle with numbered `[n]` citations, the included evidence
rows, and `retrieved`/`included`/`omitted`/`estimated_tokens`/`max_tokens` metrics.

## Local owner mode versus scoped bearer principals

With no `[[auth.tokens]]` configured, Cortana runs in local owner mode: the loopback-bound HTTP
server and MCP use an unrestricted local principal, and the CLI `context` fallback runs as the
machine user. This is the right model for the owner's own agent sessions on the same machine.

A shared or narrowly scoped agent must use a configured bearer principal instead, so query/status
scopes, document/source ACL labels, and status counters are enforced:

```toml
[auth]
# Metadata-only audit events; query text and evidence content are never stored.
audit_max_events = 10000

[[auth.tokens]]
principal = "shared-agent"
token_env = "CORTANA_SHARED_AGENT_TOKEN"
scopes = ["query", "status"]
acl = ["work", "shared"]
```

The token value lives only in the agent process environment or the private `[runtime].env_file`
(see below) — never in the TOML. How principals are presented per interface:

- **MCP:** pass `--token-env CORTANA_SHARED_AGENT_TOKEN`; the value is read from that environment
  variable and matched against a configured principal. Without `--token-env`, the server runs as
  the local owner (`local-mcp`).
- **HTTP:** send `Authorization: Bearer $CORTANA_SHARED_AGENT_TOKEN`. When any tokens are
  configured they are required for all API routes; `/healthz` and `/readyz` stay public.
  `GET /v1/status` requires the `status` scope; `GET /v1/audit` and `GET /metrics` require
  `admin`; every other API route requires `query`.
- **CLI:** the `cortana context` fallback carries no bearer credentials, so it cannot enforce
  `[[auth.tokens]]` principals or document ACL labels. It is owner-local by design and records
  metadata-only audit events under the `local-cli` principal. Shared or narrowly scoped agents
  must use the MCP server with `--token-env` or the bearer-authenticated HTTP API.

Bearer policies are loaded when the HTTP or MCP process starts. Adding, rotating, or revoking a
shared principal therefore takes effect after restarting the affected process; the desktop marks
these settings as restart-required and restarts core services in the background. Keep the previous
principal until a bounded request with the replacement succeeds and the restart has completed.

`serve` binds loopback by default. `--allow-remote` is refused unless bearer principals are configured
via `[[auth.tokens]]`; terminate TLS upstream when exposing an authenticated endpoint beyond loopback.

## Client configuration

All examples use the stdio invocation `cortana --config /absolute/path/to/cortana.toml mcp`,
optionally with `--token-env CORTANA_SHARED_AGENT_TOKEN` appended for a scoped principal.
Replace `/absolute/path/to/cortana` and `/absolute/path/to/cortana.toml` with the installed
locations from the table above.

### Codex

Append to `~/.codex/config.toml`:

```toml
[mcp_servers.cortana]
command = "/absolute/path/to/cortana"
args = ["--config", "/absolute/path/to/cortana.toml", "mcp"]
enabled = true
startup_timeout_sec = 30
```

### Hermes

Add under `mcp_servers:` in `~/.hermes/config.yaml`:

```yaml
mcp_servers:
  cortana:
    command: /absolute/path/to/cortana
    args:
      - --config
      - /absolute/path/to/cortana.toml
      - mcp
    connect_timeout: 30
    timeout: 120
```

### Buzz

Buzz-managed agents spawn an MCP stdio server from the agent's configured MCP command. Point it at
the Cortana binary with the same stdio invocation as the command string:

```text
/absolute/path/to/cortana --config /absolute/path/to/cortana.toml mcp
```

For a scoped agent, keep the token in the agent's environment and append
`--token-env CORTANA_SHARED_AGENT_TOKEN`.

### Generic MCP clients

Clients that take a JSON server descriptor (for example OpenCode, Claude Desktop, or any MCP
stdio-capable client) use the same command/args pair:

```json
{
  "mcpServers": {
    "cortana": {
      "command": "/absolute/path/to/cortana",
      "args": ["--config", "/absolute/path/to/cortana.toml", "mcp"],
      "env": {
        "CORTANA_SHARED_AGENT_TOKEN": "${CORTANA_SHARED_AGENT_TOKEN}"
      }
    }
  }
}
```

The `env` map is only needed for a scoped `[[auth.tokens]]` principal and only if the client does
not otherwise inherit the process environment. Without a token, this is the local owner's
unrestricted profile.

### HTTP-only clients

A client without MCP integration can call the loopback API directly. The local owner may omit the
`Authorization` header when no tokens are configured; shared agents must send the bearer header:

```bash
curl -sS http://127.0.0.1:7331/v1/context \
  -H "Authorization: Bearer $CORTANA_SHARED_AGENT_TOKEN" \
  -H 'content-type: application/json' \
  -d '{"query":"the concrete question","project":"optional-project","max_tokens":4000}'
```

Never put the token in the request body or URL. The equivalent CLI fallback (no running server)
is:

```bash
cortana context "the concrete question" --project optional-project --max-tokens 4000
```

## Secret handling

- Token and API-key values are read only from the process environment or the private
  `[runtime].env_file` (a `KEY=VALUE` file that Cortana refuses unless its Unix mode is `0600`).
  Process environment variables take precedence over the env file.
- The TOML config stores only environment variable names (`token_env`, `api_key_env`), never
  values. Reference `CORTANA_EMBEDDING_API_KEY` and `CORTANA_QUERY_API_KEY` the same way for
  embedding and query model endpoints.
- MCP receives tokens via `--token-env`; HTTP via the `Authorization` header only. Never commit
  tokens, private env files, or machine-specific paths.
- The audit trail records metadata only (principal, action, scope, outcome, result count,
  latency): query text and evidence content are never written to audit events.

## Health and status

| Check               | What it verifies                                                                                                                                                                                                                                                                                                                                                           |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cortana doctor`    | Configuration, storage, and the configured embedding provider                                                                                                                                                                                                                                                                                                              |
| `cortana readiness` | Read-only production gate: database integrity, embedding availability, embedding/index generation compatibility, backup freshness, query mode, recurring-sync state; when synthesis is enabled it performs a minimal grounded completion and fails closed on endpoint or citation-contract failures                                                                        |
| `GET /healthz`      | Public liveness (`{"status":"ok"}`), no token required                                                                                                                                                                                                                                                                                                                     |
| `GET /readyz`       | Public readiness: bounded database stats and an embedding provider probe; `503` when unavailable or the stats probe exceeds its timeout                                                                                                                                                                                                                                    |
| `GET /v1/status`    | Bounded index and ingestion status counters, filtered to the principal's ACL when scoped; requires the `status` scope                                                                                                                                                                                                                                                      |
| `GET /metrics`      | Low-cardinality Prometheus metrics using the same bounded database-stats probe; requires the `admin` scope                                                                                                                                                                                                                                                                 |
| MCP `brain_status`  | Configured source inventory — names, kinds, projects, enabled state, ACL labels, per-source authorization readiness (method, `authorized`, `setup_required`), and validation status (freshness, document/byte counts, generic error category) — without exposing credentials, token paths, environment variable names, or raw diagnostics; filtered by the principal's ACL |

Inspect `brain_status` when source names, configured-but-not-yet-indexed sources, or index
freshness are uncertain. `cortana doctor` and `cortana readiness` run offline against the local
index and never start or schedule ingestion; recurring sync remains opt-in and validation-gated
(readiness reports it with `--allow-sync-service`).

## Cache-aware context usage

- Cortana persistently caches query and ingestion embeddings (content-addressed, bounded by
  `[embedding].cache_max_entries`), so repeated retrieval does not re-embed. The
  `retrieved`/`included`/`omitted`/`estimated_tokens` metrics in every context bundle show what
  the token budget kept; use them to size follow-up `--max-tokens` requests.
- Reuse a context bundle within the same task. Avoiding redundant retrieval also saves ranking and
  context-window work, even when embeddings are already cached.
- Synthesized answers are cached server-side only when citation-validated. Cache keys include the
  query contract version, corpus revision, query text plus project/source scope, embedding
  fingerprint, model endpoint/name, and planner/retrieval/context/output bounds; changed or deleted
  content invalidates prior keys. Bounds are configurable via `[query].cache_max_entries` and
  `[query].cache_ttl_seconds` (set either to `0` to skip reads or writes). Temporary planner or
  provider failures are never hidden by a stale cache entry.
- Reusing a bundle within the task is an agent-side practice — the answer cache described above
  covers synthesized `/v1/answer` results only, not `context` bundles. Tune the answer cache
  settings only after the model-backed [evaluation and readiness gates](evaluation.md) pass.
