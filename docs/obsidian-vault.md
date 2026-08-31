# Derived Obsidian vault

Cortana can materialize explicitly selected, authorized workspaces as an Obsidian-compatible
Markdown directory. The vault is a derived, read-only projection: Cortana's SQLite store remains
canonical, and edits made in Obsidian are never ingested, reconciled, backed up, or converted into
native memory.

## Export and open a vault

Use Desktop Settings > Advanced > Derived Obsidian vault to choose an empty directory, select each
workspace, preview the export, and then confirm it. The equivalent owner-local CLI command is:

```sh
cortana export-vault "/absolute/path/Cortana Vault" \
  --workspace work \
  --workspace research
```

In Obsidian, choose **Open folder as vault** and select `Cortana Vault`. Markdown is organized by
workspace, source, and a safe source-owned folder name when present. Stable filenames and
frontmatter retain Cortana document identity, canonical revision, timestamp, ACL labels, source
URI, and provenance. Safe HTTP(S) attachment references from the canonical `attachments` metadata
field are retained as links; local paths, credentials, runtime paths, and unsupported URI schemes
are not exported. This version does not copy remote attachment bytes.

Use `--dry-run` or Desktop Preview to inspect counts and paths without creating the destination.
Repeated exports retain unchanged files without content rewrites. Progress and cancellation are
available in Desktop; `SIGINT` or `SIGTERM` cancels the CLI before publication.

## Explicit recurring schedule

Scheduling is disabled by default. Install it only after reviewing the exact destination,
workspace list, and interval:

```sh
cortana service install \
  --enable-vault-service \
  --vault-output "/absolute/path/Cortana Vault" \
  --vault-workspace work \
  --vault-workspace research \
  --vault-seconds 86400
```

This installs the platform's per-user `vault` service alongside the selected safe service set.
Inspect it with `cortana service status`, trigger it with `cortana service start vault`, stop it with
`cortana service stop vault`, or remove all Cortana per-user services with
`cortana service uninstall`. Re-running `service install` without the explicit vault flags removes
the recurring vault job; it never silently preserves or creates a schedule.

## Rename, deletion, failure, and recovery

`.cortana-vault.json` is an owner-private manifest. It maps canonical document IDs to generated
paths and revisions so rename, supersession, and deletion are handled on the next complete export.
The manifest is an implementation detail and must not be shared as public content.

Publication is atomic. While rebuilding, Cortana stages a complete replacement and leaves the
current vault usable. After a successful changed export, the immediately previous complete vault
is retained beside it as `.Cortana Vault.cortana-previous`. A cancelled or failed export removes
its staging directory and does not replace the complete vault. The previous directory is a
reversible one-generation recovery point, not a backup of canonical evidence.

To rebuild, close Obsidian, move the generated vault and its adjacent previous-vault directory to
Trash, then run the export again from Cortana's canonical store. To remove the integration, stop or
remove the `vault` service first and move both generated directories to Trash. Never point source
ingestion at an exported vault; doing so would create a feedback loop and a false second authority.
