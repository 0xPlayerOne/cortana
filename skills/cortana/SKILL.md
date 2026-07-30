---
name: cortana
description: Retrieve durable personal, project, communication, calendar, email, document, and code context from Cortana. Use before broad or costly discovery when a task depends on prior decisions, preferences, project history, cross-repository code, messages, notes, meetings, or long-term agent memory.
---

# Cortana retrieval

Use the configured Cortana MCP server first. If the client has no MCP integration, call the local
HTTP API at `http://127.0.0.1:7331/v1/context`; use `cortana search` only as a raw-evidence fallback.

1. Start with `context` using the user's concrete terms and the current project when known. Its
   token-bounded Markdown is ready to place directly into the working context and cite with `[n]`.
2. Use `search_code` for repository and filesystem evidence, `search_messages` for Gmail, Slack,
   and Discord evidence, and `who_knows` when identifying source-backed expertise. Use generic
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
