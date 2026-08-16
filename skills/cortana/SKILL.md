---
name: cortana
description: Retrieve durable personal, project, communication, calendar, email, document, and code context from Cortana. Use before broad or costly discovery when a task depends on prior decisions, preferences, project history, cross-repository code, messages, notes, meetings, or long-term agent memory.
---

# Cortana retrieval

Use the configured Cortana MCP server first. If the client has no MCP integration, prefer the CLI
fallback `cortana context "<concrete question>"` — optionally `--project`, `--source`, `--limit`
(1–50), and `--max-tokens` (256–64,000, defaulting to the configured context budget) — which
returns the same citation-ready bundle as the MCP/HTTP endpoints without a running server. The
local HTTP API at `http://127.0.0.1:7331/v1/context` is equivalent. Use `cortana search` only as a
raw-evidence fallback.

1. Start with `context` (MCP, CLI, or HTTP) using the user's concrete terms and the current project
   when known. Its token-bounded Markdown contains numbered source evidence plus a separate native
   memory section when matching durable agent context exists. Cite only the numbered evidence with
   `[n]`; treat memories as scoped operational context.
2. Use `search_code` for repository and filesystem evidence, `search_messages` for Gmail, Slack,
   Discord, and Buzz evidence, and `who_knows` when identifying source-backed expertise. Use generic
   `search` only for another focused pass, an explicit source, or exact debugging details.
3. Use source filters for exact configured source names. Inspect `brain_status` when source names,
   configured-but-not-yet-indexed sources, or index freshness are uncertain; it reports the
   configured source inventory without exposing credentials, including the cumulative retrieval
   fallback counter.
4. Reuse a context bundle within the same task. Cortana persistently caches query and ingestion
   embeddings, but avoiding redundant retrieval also saves ranking and context-window work.
5. Treat returned rows as evidence, preserving their source URI and timestamp. Prefer exact lexical
   evidence for identifiers and errors; use semantic evidence for paraphrases.
6. Do not persist secrets, credentials, private keys, raw authentication material, or copied
   Cortana evidence outside the task unless the user asks.
7. If evidence conflicts, prefer newer authoritative sources and disclose the conflict.
8. Use `remember` only for an explicit, bounded conclusion, preference, procedure, episode, or
   working-state update. Include a stable `dedupe_key` and provenance when possible; never copy
   an entire source document into memory. Set an RFC3339 `valid_until` on short-lived working
   context so expired state is excluded automatically. Dedupe keys and supersession targets stay
   within the selected workspace, even for the owner. Use `recall` for memory-only retrieval and
   `forget` when the user withdraws a memory.

When using `/v1/answer`, treat any returned `memories` as operational context rather than citations;
the answer must remain grounded in numbered evidence. Shared agents need the `memory` scope for
those entries, while query-only agents intentionally receive evidence-only answers.

For an MCP client, configure:

```json
{
  "command": "cortana",
  "args": ["--config", "/absolute/path/to/cortana.toml", "mcp"]
}
```

Use an absolute config path because MCP clients may launch from arbitrary working directories.
The no-token form is the local owner's unrestricted profile. For a shared or narrowly scoped
agent, define a `[[auth.tokens]]` principal in Cortana's configuration and pass its environment
variable name:

```json
{
  "command": "cortana",
  "args": [
    "--config",
    "/absolute/path/to/cortana.toml",
    "mcp",
    "--token-env",
    "CORTANA_SHARED_AGENT_TOKEN"
  ]
}
```

The token value stays in the agent process environment/private Cortana env file. Cortana maps it
to the configured principal, enforces query/status scopes and document/source ACL labels inside
MCP, scopes status counters and source inventory, and records only metadata-only audit events under
that principal name.

The MCP server also exposes native `remember`, `recall`, `forget`, and `export_memory` tools. They
use the same workspace ACLs and audit trail as document retrieval; `context` automatically includes
relevant native memories without exposing retracted or out-of-scope records.

For an HTTP-only client, send:

```json
{
  "method": "POST",
  "url": "http://127.0.0.1:7331/v1/context",
  "headers": {
    "Authorization": "Bearer $CORTANA_SHARED_AGENT_TOKEN",
    "Content-Type": "application/json"
  },
  "json": {
    "query": "the concrete question",
    "project": "optional-project",
    "max_tokens": 4000
  }
}
```

Use the `Authorization` header for a configured shared principal; do not send a token in the
request body or URL. HTTP and MCP share the same retrieval limits: queries must be non-empty and
at most 16 KiB, and each tool returns at most 50 evidence rows. HTTP search preserves its
evidence-array response shape and exposes `x-cortana-retrieval-mode` and
`x-cortana-retrieval-degraded` headers. A local owner-only HTTP server may
omit the header when no `[[auth.tokens]]` are configured, but shared agents must use a scoped
bearer principal so ACL labels and status/query permissions are enforced.

For a CLI-only client (no MCP integration and no server running), run:

```bash
cortana context "the concrete question" --project optional-project --max-tokens 4000
```

The command prints stable JSON containing the Markdown context (with `[n]` citations), the
included evidence rows, and retrieval/metrics fields (`retrieved`, `included`, `omitted`,
`memories_retrieved`, `memories_included`, `memories_omitted`, `estimated_tokens`, `max_tokens`,
and `retrieval_mode`). If the embedding provider is unavailable
or exceeds the interactive budget, `retrieval_mode` is `lexical-fallback` and the bundle includes
a non-secret `retrieval_warning`; disclose that degradation to the user instead of presenting it
as semantic retrieval. Omit `--max-tokens` to use the configured `[query].context_tokens` budget.
Use `--offline` for the deterministic embedding path.

The CLI fallback is owner-local: it carries no bearer credentials, so it cannot enforce scoped
`[[auth.tokens]]` principals or document ACL labels. Shared or narrowly scoped agents must use the
MCP server with `--token-env` or the bearer-authenticated HTTP API. CLI `context` calls are
recorded in the metadata-only audit trail under the `local-cli` principal (action
`local-cli/context`); query text and evidence content are never stored.
