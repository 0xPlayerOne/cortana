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

Secrets and source credentials remain outside the renderer. Future settings and authorization
commands must preserve this boundary by returning redacted state and accepting named fields
instead of filesystem paths, shell fragments, or arbitrary URLs.

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
