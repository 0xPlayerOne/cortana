---
name: cortana
description: Retrieve durable personal, project, communication, and code context from Cortana before broad or costly discovery.
---

# Cortana retrieval

Use the configured Cortana MCP server when a task depends on prior decisions, personal preferences,
project history, code across repositories, messages, notes, email, or documents.

1. Start with `context` using the user's concrete terms and the current project when known. Its
   token-bounded Markdown is ready to place directly into the working context and cite with `[n]`.
2. Use `search` only when you need raw evidence rows, a second focused pass, or exact debugging
   details not present in the first bundle.
3. Use source filters for exact configured source names. Inspect `brain_status` when source names
   or index freshness are uncertain.
4. Reuse a context bundle within the same task. Cortana persistently caches query and ingestion
   embeddings, but avoiding redundant retrieval also saves ranking and context-window work.
5. Treat returned rows as evidence, preserving their source URI and timestamp. Prefer exact lexical
   evidence for identifiers and errors; use semantic evidence for paraphrases.
6. Do not persist secrets, credentials, private keys, raw authentication material, or copied
   Cortana evidence outside the task unless the user asks.
7. If evidence conflicts, prefer newer authoritative sources and disclose the conflict.

The MCP configuration is:

```json
{
  "command": "cortana",
  "args": ["--config", "/absolute/path/to/cortana.toml", "mcp"]
}
```

Use an absolute config path because MCP clients may launch from arbitrary working directories.
