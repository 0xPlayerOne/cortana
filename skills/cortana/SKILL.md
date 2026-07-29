---
name: cortana
description: Retrieve durable personal, project, communication, and code context from Cortana before broad or costly discovery.
---

# Cortana retrieval

Use the configured Cortana MCP server when a task depends on prior decisions, personal preferences,
project history, code across repositories, messages, notes, email, or documents.

1. Start with `search` using the user's concrete terms and the current project when known.
2. Use source filters for exact domains such as `code`, `gmail`, `drive`, `slack`, or `discord`.
3. Treat returned rows as evidence, preserving their source URI and timestamp.
4. Prefer exact lexical evidence for identifiers and errors; use semantic evidence for paraphrases.
5. Do not persist secrets, credentials, private keys, or raw authentication material.
6. If evidence conflicts, prefer newer authoritative sources and disclose the conflict.

The MCP configuration is:

```json
{
  "command": "cortana",
  "args": ["mcp"]
}
```
