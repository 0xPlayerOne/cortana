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

Secrets and source credentials remain outside the renderer. The settings bridge returns only
configured/unset metadata and accepts write-only named secret updates. It refuses symlinked config
or secret files, externally managed secret paths, insecure remote HTTP endpoints, malformed TOML
sections, broad data roots, and unbounded inputs. It writes owner-only files atomically, keeps a
rollback copy of the previous config, and appends metadata-only audit events.

The renderer can edit typed provider, cache, timeout, ingestion-budget, workspace, and storage
fields. It cannot edit connector command arrays or service commands because those fields cross
directly into process execution. Source authorization and service control use separate typed
native commands.

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

## Lifecycle

There are two distinct lifecycles:

1. The Cortana runtime owns the API, embedding service, opt-in ingestion, and backups.
2. Cortana Desktop owns the window, tray, updater, and user controls.

Closing the main window hides it. The tray continues to report runtime and corpus status. Quitting
from the tray exits only the desktop process; it does not silently stop the runtime or start a
sync. A second desktop launch focuses the existing window.

## Releases

Desktop Rust has its own lockfile so platform WebKit dependencies do not enter the core Rust
workspace or its reusable CI. Pull requests compile the webview and Tauri binary on Linux. Published
releases build the native installers on macOS ARM64, Linux x64, and Windows x64, then sign updater
artifacts with the dedicated Tauri key.

The updater public key is part of the application configuration. Its private key and password are
GitHub Actions secrets. Local contributor builds require neither secret:

```bash
bun run desktop:build
```

On macOS, an unsigned local application bundle can be produced separately:

```bash
bun run desktop:bundle:mac
```
