# Desktop architecture

Cortana Desktop is a Tauri 2 control plane over the independently managed Cortana runtime. The
runtime remains useful to MCP clients and other agents when the desktop window is closed.

## Trust boundary

The webview cannot invoke an arbitrary process, read an arbitrary file, or choose a backend URL.
Its only native data path is a small set of typed Tauri commands. Those commands call the fixed
owner-local endpoint at `http://127.0.0.1:7331`, reject unknown request fields, and bound all query
and scope inputs before forwarding them.

The default capability grants only:

- core window/event functionality;
- autostart enable, disable, and status;
- a desktop-process restart for applying an update;
- updater checks and installation.

The matching core executable is bundled as a Tauri sidecar for each release target. Native Rust
code may invoke only that declared sidecar; the webview receives no shell capability and cannot
replace its path or arguments. The sidecar is included inside the updater-signed desktop artifact,
so Desktop readiness never depends on a separately downloaded Cortana executable.

Secrets and source credentials remain outside the renderer. The settings bridge returns only
configured/unset metadata and accepts write-only named secret updates. It refuses symlinked config
or secret files, externally managed secret paths, insecure remote HTTP endpoints, malformed TOML
sections, broad data roots, and unbounded inputs. It writes owner-only files atomically, keeps a
rollback copy of the previous config, and appends metadata-only audit events.

The renderer can edit typed provider, cache, timeout, ingestion-budget, workspace, and storage
fields. It cannot edit connector command arrays or service commands because those fields cross
directly into process execution. Source authorization and service control use separate typed
native commands.

Corpus browsing also stays behind typed native commands. The renderer may send only bounded
project/source filters, an opaque pagination cursor, or a hex document ID. Native Rust constructs
the fixed loopback URLs and performs the request; the renderer cannot select a host, port, path, or
database file. Original `file:` links take a second native path: the target is canonicalized and
must remain inside a configured filesystem source root, so symlinks cannot escape the indexed
scope. The core API applies the same bearer scope and document ACL policy as retrieval, bounds
pages to 100 summaries and content to 2 MiB, and records metadata-only list/read audits.

Readiness is user-triggered and read-only. It performs bounded version checks for uv and Python
3.11+, locates the managed connector environment, and runs `cortana readiness` through the bundled
sidecar. It rejects oversized or malformed reports and never starts a connector or sync.

Tool installation is a second native boundary. The renderer can request only a fixed tool ID and
must set an explicit approval flag after showing a confirmation. Native code maps that ID to a
platform-specific command, permits one job at a time, bounds and sanitizes returned logs, supports
cancellation, and writes metadata-only audit events beside the active Cortana config. Arbitrary
programs and arguments are never accepted from the renderer.

Source validation is a third native boundary. The renderer sends only an exact configured source
name. Native Rust reloads the owner-local configuration, rejects an unknown name, then constructs
the declared sidecar command with fixed `validate-source` arguments and limits of 25 documents,
5 MiB, and 60 seconds. Only one validation runs at a time; progress is bounded, sanitized, and
cancellable. The command cannot start sync, embedding, indexing, or reconciliation, and its
metadata-only lifecycle is appended to the Desktop audit log. The same command can optionally
validate at one of the fixed initial-sync budget tiers so a later validation-gated initial sync
has a record at equal or larger limits; validation stays read-only and bounded in every tier.

Guarded trial sync reuses the source-job boundary but is intentionally distinct from validation.
It requires explicit confirmation and a matching successful validation fingerprint, then invokes
only the fixed `sync --source NAME --require-validation --no-reconcile` shape with limits of 25
documents, 5 MiB, and five minutes. It may write embeddings and documents, so the UI and audit
record say so explicitly. It cannot reconcile deletions, expand its limits, select multiple
sources, or install a recurring sync job.

Guided initial sync is the next tier of the same source-job boundary and stays a two-step native
protocol. The renderer sends only a configured source name, one of three fixed budget enum values
(100 documents/25 MiB/15 minutes, 500/64 MiB/30 minutes, or 2,000/128 MiB/60 minutes), and a
plan/execute operation; it can never supply CLI flags, raw numbers, or credentials. A plan
request is read-only: native Rust resolves the saved source, returns the exact budget limits that
execution will enforce, reports whether the latest validation record covers that budget, and
issues a short-lived one-shot plan id. Execution requires that plan id plus an explicit approval,
an enabled source, and a successful validation record at equal or larger limits; it then runs the
fixed `sync --source NAME --require-validation --no-reconcile --max-documents N --max-bytes B
--max-seconds S` shape for the selected tier. It reuses the existing single-job lock,
cancellation, bounded sanitized logs, and metadata-only audit lifecycle (including the selected
budget tier), and it never silently escalates beyond the chosen tier. The sidecar's own
`--require-validation` gate remains authoritative even if the Desktop-side coverage hint is
stale or missing.

Source authorization and setup are separate fixed native boundaries. For Google sources, the
renderer can request authorization only for an exact saved source with an absolute token
destination and Desktop OAuth client path. Native Rust invokes the bundled sidecar with the fixed
`authorize-google SOURCE` shape. The sidecar uses Authorization Code + PKCE, a random loopback
port and state value, fixed HTTPS Google endpoints, bounded callback and token-exchange timeouts,
minimum read-only scopes, and owner-only atomic token writes. Tokens and authorization codes never
enter renderer state, logs, command output, or audit records.

Provider setup links are selected by native code from a fixed allowlist and opened in the system
browser; the renderer cannot supply a URL. File and folder selection also stays native. The
renderer requests one of three fixed picker kinds—source directory, OAuth client JSON, or Google
token destination—and receives a validated absolute path. It has no general filesystem
permission.

## Workspaces and settings

Workspaces are query/project scopes inside one canonical database, rather than separate indexes.
This keeps embedding and query caches reusable while allowing agents and people to filter work,
personal, or special context. The first desktop iteration permits one to three workspaces; the
storage and API contract remain list-based so that limit can be raised without a data migration.

Workspace metadata is exposed by `/v1/status`. Desktop search sends the selected workspace ID as
the existing project scope, and source navigation plus canonical document pagination filter to
that scope. The sidebar groups paginated documents under workspace and source nodes; selecting a
document opens canonical content rather than a retrieved chunk. Editing a workspace may not orphan
a configured source: source assignments must be moved before their workspace ID can be removed.

Provider secrets are stored in Cortana's managed `secrets.env` file with mode `0600` on Unix. An
existing external `runtime.env_file` remains readable by the runtime but is intentionally
read-only in Desktop.

The source editor supports the native filesystem, Apple Notes, Buzz, Google Drive, Gmail, Google
Calendar, Slack, and Discord connector schemas. It can retain, disable, or remove an existing
external command connector, but cannot create or modify command arrays. Google token files and
OAuth client files and local roots must be absolute non-root paths; Slack and Discord require
explicit channels and a validated environment-variable name. Saving or authorizing source
settings never starts ingestion.

First launch enters a guided checklist and automatically runs only the read-only readiness scan.
Missing tool installers remain fixed native jobs that require explicit approval and expose bounded
progress, cancellation, and retry state.

Settings portability uses a versioned, size-bounded JSON envelope selected through the native file
dialog. Exports exclude secret values and executable connector commands. Imports reject unknown
fields, embedded-secret declarations, unsupported versions, oversized files, symlinks, and
unvalidated values. They return an unsaved preview to the renderer, preserve locally configured
external connectors, and rely on the existing owner-only backup-and-replace path only after the
user saves.

## Lifecycle

There are two distinct lifecycles:

1. The Cortana runtime owns the API, embedding service, opt-in ingestion, and backups.
2. Cortana Desktop owns the window, tray, updater, and user controls.

Closing the main window hides it. The tray continues to report runtime, corpus, and bounded
ingestion status. Quitting from the tray exits only the desktop process; it does not silently stop
the runtime or start a sync. A second desktop launch focuses the existing window.

The Services panel reads a bounded structured report from the bundled sidecar and can explicitly
install the safe query-only service set (embedding when the configured provider is local, server,
and backup) through the fixed `service install --no-web` command. It accepts only the fixed
`embedding`, `server`, `sync`, and `backup` IDs with `start`, `stop`, or `restart`.
Every action requires an explicit confirmation and is audited without command output or secrets.
An uninstalled sync job remains uninstalled and cannot be enabled from this panel. Desktop
autostart is managed separately and does not change runtime-service or ingestion state.

Start All, Stop All, and Restart All are narrower than the individual controls: they operate only
on `embedding` and `server`, in dependency-safe order. They always exclude `sync` and `backup`, so
a whole-app action cannot schedule ingestion or trigger a backup.

Desktop manages named bearer principals without exposing credentials to the renderer. The webview
edits only principal names, environment-variable references, scopes, and ACL labels. Write-only
values go to the owner-only managed secret file. Native loopback requests resolve a matching
private credential for `query`, `status`, or `admin` immediately before sending the request;
bearer values never enter IPC responses or renderer state.

The Audit panel combines the API's bounded metadata-only retrieval events with a bounded tail of
the owner-only Desktop action log. Runtime unavailability does not prevent local action events
from being inspected. Both views exclude query text, document content, command output, and secret
values.

## Releases

Desktop Rust has its own lockfile so platform WebKit dependencies do not enter the core Rust
workspace or its reusable CI. Pull requests compile the webview and Tauri binary on Linux. Published
releases build the native installers and matching sidecars on macOS ARM64, Linux x64, and Windows
x64, then sign updater artifacts with the dedicated Tauri key.

The updater public key is part of the application configuration. Its private key and password are
GitHub Actions secrets. Local contributor builds require neither secret:

```bash
bun run desktop:build
```

Updater signing authenticates the downloaded update payload; it is distinct from operating-system
publisher trust. Until Apple Developer ID credentials and notarization are configured, macOS
artifacts remain ad-hoc code-signed and Gatekeeper does not treat them as notarized applications.

Update checks, downloads, signature verification, installation, and restart requests run in native
Rust. The renderer can display version metadata, release notes, the compiled changelog, and bounded
download progress, but cannot override the feed URL, signature key, download URL, or expected
version. Installation requires explicit confirmation and verifies the announced signature before
replacing the app.

On macOS, an unsigned local application bundle can be produced separately:

```bash
bun run desktop:bundle:mac
```
