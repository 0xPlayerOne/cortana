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
metadata-only lifecycle is appended to the Desktop audit log.

Guarded trial sync reuses the source-job boundary but is intentionally distinct from validation.
It requires explicit confirmation and a matching successful validation fingerprint, then invokes
only the fixed `sync --source NAME --require-validation --no-reconcile` shape with limits of 25
documents, 5 MiB, and five minutes. It may write embeddings and documents, so the UI and audit
record say so explicitly. It cannot reconcile deletions, expand its limits, select multiple
sources, or install a recurring sync job.

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
the existing project scope, and source navigation filters to that scope. Editing a workspace may
not orphan a configured source: source assignments must be moved before their workspace ID can be
removed.

Provider secrets are stored in Cortana's managed `secrets.env` file with mode `0600` on Unix. An
existing external `runtime.env_file` remains readable by the runtime but is intentionally
read-only in Desktop.

The source editor supports the native filesystem, Apple Notes, Buzz, Google Drive, Gmail, Google
Calendar, Slack, and Discord connector schemas. It can retain, disable, or remove an existing
external command connector, but cannot create or modify command arrays. Google token files and
OAuth client files and local roots must be absolute non-root paths; Slack and Discord require
explicit channels and a validated environment-variable name. Saving or authorizing source
settings never starts ingestion.

## Lifecycle

There are two distinct lifecycles:

1. The Cortana runtime owns the API, embedding service, opt-in ingestion, and backups.
2. Cortana Desktop owns the window, tray, updater, and user controls.

Closing the main window hides it. The tray continues to report runtime and corpus status. Quitting
from the tray exits only the desktop process; it does not silently stop the runtime or start a
sync. A second desktop launch focuses the existing window.

The Services panel reads a bounded structured report from the bundled sidecar and accepts only
the fixed `embedding`, `server`, `sync`, and `backup` IDs with `start`, `stop`, or `restart`.
Every action requires an explicit confirmation and is audited without command output or secrets.
An uninstalled sync job remains uninstalled and cannot be enabled from this panel. Desktop
autostart is managed separately and does not change runtime-service or ingestion state.

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

On macOS, an unsigned local application bundle can be produced separately:

```bash
bun run desktop:bundle:mac
```
