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
   when known. Its token-bounded Markdown is ready to place directly into the working context and
   cite with `[n]`.
2. Use `search_code` for repository and filesystem evidence, `search_messages` for Gmail, Slack,
   Discord, and Buzz evidence, and `who_knows` when identifying source-backed expertise. Use generic
   `search` only for another focused pass, an explicit source, or exact debugging details.
3. Use source filters for exact configured source names. Inspect `brain_status` when source names
   or index freshness are uncertain.
4. Reuse a context bundle within the same task. Cortana persistently caches query and ingestion
   embeddings, but avoiding redundant retrieval also saves ranking and context-window work.
5. Treat returned rows as evidence, preserving their source URI and timestamp. Prefer exact lexical
   evidence for identifiers and errors; use semantic evidence for paraphrases.
6. Do not persist secrets, credentials, private keys, raw authentication material, or copied
   Cortana evidence outside the task unless the user asks.
7. If evidence conflicts, prefer newer authoritative sources and disclose the conflict.

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
to the configured principal, enforces query/status scopes and document ACL labels inside MCP, and
records only metadata-only audit events under that principal name.

For an HTTP-only client, send:

```json
{
  "method": "POST",
  "url": "http://127.0.0.1:7331/v1/context",
  "json": {
    "query": "the concrete question",
    "project": "optional-project",
    "max_tokens": 4000
  }
}
```

For a CLI-only client (no MCP integration and no server running), run:

```bash
cortana context "the concrete question" --project optional-project --max-tokens 4000
```

The command prints stable JSON containing the Markdown context (with `[n]` citations), the
included evidence rows, and retrieval/metrics fields (`retrieved`, `included`, `omitted`,
`estimated_tokens`, `max_tokens`). Omit `--max-tokens` to use the configured `[query].context_tokens`
budget. Use `--offline` for the deterministic embedding path.

The CLI fallback is owner-local: it carries no bearer credentials, so it cannot enforce scoped
`[[auth.tokens]]` principals or document ACL labels. Shared or narrowly scoped agents must use the
MCP server with `--token-env` or the bearer-authenticated HTTP API. CLI `context` calls are
recorded in the metadata-only audit trail under the `local-cli` principal (action
`local-cli/context`); query text and evidence content are never stored.
