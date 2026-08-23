# Agent integrations

Cortana exposes the same vertically integrated knowledge and memory pipeline through a portable
agent skill, an MCP stdio server, a loopback HTTP API, and the CLI. Agents should start with the
skill's `context` primitive (MCP,
HTTP, or CLI) and treat `search_code`, `search_messages`, and `who_knows` as targeted evidence
tools; see [the skill](../skills/cortana/SKILL.md) for the full retrieval protocol and
[the query guide](query.md) for pipeline details. This guide covers installation and client
configuration only.

## Current release boundary

The current protected source and published package are `v0.34.36`. The portable skill, MCP, HTTP,
and CLI contracts below are version-aligned with that release. Installing the skill does not expose
credentials, authorize a source, enable recurring sync, or change client configuration implicitly.

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

The MCP server advertises every input property with a strict JSON Schema. In particular, the
`remember.provenance` field accepts arbitrary JSON at runtime but is advertised as an object schema
so strict MCP clients can complete `tools/list` validation before invoking any tool. Cortana keeps
the value unchanged for audit and provenance consumers.
The protocol regression test keeps this advertised schema contract from regressing as new memory
fields are added.

| Interface                                       | Entry points                                                                                                                                                                                                                                          |
| ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| MCP stdio (`cortana --config <path> mcp`)       | `context`, `remember`, `recall`, `forget`, `export_memory`, `search`, `search_code`, `search_messages`, `who_knows`, `brain_status`                                                                                                                   |
| HTTP (`cortana serve --address 127.0.0.1:7331`) | `POST /v1/context`, `POST /v1/memory[/{recall,forget}]`, `GET /v1/memory/export`, `POST /v1/search`, `POST /v1/answer`, `GET /v1/documents[/{id}]`, `GET /v1/graph`, `GET /v1/status`, `GET /v1/audit`, `GET /healthz`, `GET /readyz`, `GET /metrics` |
| CLI (no server required)                        | `cortana context`, `cortana search` (raw-evidence fallback)                                                                                                                                                                                           |

`cortana context QUERY`, `POST /v1/context`, and the MCP `context` tool return the same
citation-ready, token-bounded Markdown bundle with numbered `[n]` citations, the included evidence,
relevant native `memories`, and `retrieved`/`included`/`omitted`/`memories_included`/
`estimated_tokens`/`max_tokens` metrics. Memory writes and redactions remain explicit actions;
ingestion never silently promotes source text into agent memory.

`POST /v1/answer` follows the same boundary: principals with the `memory` scope receive a bounded
`memories` array and the synthesizer may use those entries as operational context, while evidence
alone remains citation-bearing. Query-only principals receive no memory field. Answer-cache keys
include the native memory revision and visible memory ACL, so remember/forget operations cannot
leave an old memory-backed answer cached.

Memory writes may include an optional RFC3339 `valid_until` for short-lived working context. Expired
records are excluded from recall and answer context automatically; durable facts should instead use
an explicit supersession or forget operation. Dedupe and supersession are checked against the
caller’s visible ACL inside the same SQLite transaction, so a scoped agent cannot overwrite or
replace another workspace’s memory by guessing an identifier.
Identical retries with a dedupe key are true no-ops and do not invalidate answer caches.
`brain_status` and `/v1/status` expose active, expired, retracted, superseded, and total native-memory
counts; those lifecycle counts are ACL-scoped for shared principals and complete for owners. Expired
records remain available to scoped export for audit and backup but never enter recall.
Recall ranking is fully local and bounded: exact query coverage is preferred, then lexical
relevance, confidence, importance, and freshness. The returned `relevance_score` is diagnostic
metadata only; ACL, expiry, and lifecycle checks always run independently.

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

Add the `memory` scope when this principal should read, write, or redact native agent memory.
Without it, `context` still returns source evidence but omits the native-memory section; this keeps
query-only agents from receiving operational memory by accident.

The token value lives only in the agent process environment or the private `[runtime].env_file`
(see below) — never in the TOML. How principals are presented per interface:

- **MCP:** pass `--token-env CORTANA_SHARED_AGENT_TOKEN`; the value is read from that environment
  variable and matched against a configured principal. Without `--token-env`, the server runs as
  the local owner (`local-mcp`).
- **HTTP:** send `Authorization: Bearer $CORTANA_SHARED_AGENT_TOKEN`. When any tokens are
  configured they are required for all API routes. `/healthz` remains public liveness; `/readyz`
  is public only on loopback and requires a bearer principal on remote listeners because it probes
  the store and embedding provider.
  `GET /v1/status` requires the `status` scope; `GET /v1/audit` and `GET /metrics` require
  `admin`; every other API route requires `query`.
- **CLI:** the `cortana context` fallback carries no bearer credentials, so it cannot enforce
  `[[auth.tokens]]` principals or document ACL labels. It is owner-local by design and records
  metadata-only audit events under the `local-cli` principal. Shared or narrowly scoped agents
  must use the MCP server with `--token-env` or the bearer-authenticated HTTP API.

Bearer policies are loaded when the HTTP or MCP process starts. The HTTP service also exposes an
owner/admin-only `POST /v1/auth/reload` operation for an atomic policy refresh from its configured
TOML and private environment file. A successful refresh replaces the complete policy at once;
malformed or unreadable credentials leave the last-good HTTP policy active. An MCP process launched
with `--token-env` backed by the private `0600` env file resolves that principal for each tool call,
so replacing or revoking the stable variable takes effect without restarting the stdio process;
malformed or unreadable policy fails closed for that call. A process-environment-only credential is
startup-scoped and must reconnect after its value or variable name changes. The Desktop still marks
access changes as restart-required because its managed service action restarts both transports
deliberately.

Never use a reload to remove the last policy from a non-loopback listener: remote listeners reject
that transition. For a zero-downtime rotation, keep the principal and `token_env` mapping stable,
replace its value in the private env file, and call the reload endpoint with a separate current
admin credential. The old token is rejected immediately after the successful swap; requests already
in flight are not interrupted.

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

### Rotate or revoke an HTTP principal without restarting

The reload endpoint is intentionally narrow and does not accept a token or policy in the request
body. It rereads the configured TOML plus its `0600` environment file and requires the current
principal to have `admin` scope:

```bash
curl -sS -X POST http://127.0.0.1:7331/v1/auth/reload \
  -H "Authorization: Bearer $CORTANA_ADMIN_AGENT_TOKEN"
```

The response is metadata-only. A failed parse, missing secret, invalid scope, or duplicate token
never replaces the active policy; the failure is recorded as an `auth.reload` audit event without
including the error text or credential value. A successful swap records the same action and
immediately invalidates removed token values. MCP processes launched with `--token-env` and a private
env file resolve the new policy on their next tool call; process-environment-only clients must
reconnect after changing the variable or its name.

### Run the disposable shared-agent authorization drill

Before connecting a new agent, run the checked-in drill against a temporary offline index:

```bash
scripts/shared-agent-auth-drill.sh
```

It creates two synthetic work/personal documents, verifies query/status/admin scope separation and
ACL filtering, rotates the query token through `/v1/auth/reload`, confirms the old value is rejected,
and checks that audit responses contain metadata only. It never reads the live config or index,
contacts a provider, authorizes a source, starts recurring sync, or exercises the packaged GUI/MCP
transport. Set `CORTANA_BINARY` for a checkout binary or `CORTANA_KEEP_DRILL=1` to retain the exact
temporary record. The Rust API/MCP test suites remain the authoritative transport-level coverage;
this script is the operator-facing HTTP smoke check.

For the actual stdio transport, run the disposable subprocess drill as well:

```bash
python3 scripts/shared-agent-mcp-drill.py
```

It starts the shipped `cortana mcp` command against a temporary offline index,
checks the initialize/tools/call handshake, proves a work-scoped search cannot
see a personal document, rotates the file-backed principal without restarting
the process, and verifies that an emptied token is rejected. It never reads the
live configuration or index and never contacts a provider.

## Secret handling

- Token and API-key values are read only from the process environment or the private
  `[runtime].env_file` (a `KEY=VALUE` file that Cortana refuses unless its Unix mode is `0600`).
  For connector and provider values such as `api_key_env`, process environment variables take
  precedence over the env file. Bearer policies are intentionally different: `[[auth.tokens]]`
  values prefer the private env file, so a stable `token_env` can be rotated without inheriting a
  stale value from the long-running service environment.
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
| `GET /readyz`       | Loopback readiness without a token; remote listeners require a bearer principal. It performs bounded database stats and an embedding provider probe and returns `503` when unavailable or the stats probe exceeds its timeout                                                                                                                                              |
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
  query contract version, document corpus revision, native memory revision and visible memory ACL
  when enabled, query text plus project/source scope, embedding fingerprint, model endpoint/name,
  and planner/retrieval/context/output bounds; changed or deleted content or memory invalidates
  prior keys. Bounds are configurable via `[query].cache_max_entries` and
  `[query].cache_ttl_seconds` (set either to `0` to skip reads or writes). Temporary planner or
  provider failures are never hidden by a stale cache entry.
- Reusing a bundle within the task is an agent-side practice — the answer cache described above
  covers synthesized `/v1/answer` results only, not `context` bundles. Tune the answer cache
  settings only after the model-backed [evaluation and readiness gates](evaluation.md) pass.
