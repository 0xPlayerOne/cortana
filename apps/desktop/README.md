# Cortana Desktop

Cortana Desktop is the Tauri 2 control plane for the independently runnable Cortana runtime.
It reuses the React/Vite workspace in `apps/web`, ships the matching Cortana core binary as a
platform-specific sidecar, and talks to the local owner-only API through narrow Rust commands.
The renderer has no arbitrary shell or filesystem capability.

## Development

Start the installed Cortana services, then run:

```bash
bun run desktop:dev
```

Compile the production binary without creating an installer:

```bash
bun run desktop:build
```

On macOS, build an unsigned local `.app` bundle with:

```bash
bun run desktop:bundle:mac
```

Updater-signed DMG/application archives, AppImage/deb, and Windows installers are created only by
the release workflow. That workflow has access to the dedicated updater signing key; local builds
do not require release secrets. macOS Developer ID signing and notarization require separate Apple
credentials and are reported independently from the updater signature.

The Settings **Readiness** panel runs an explicit, read-only scan. It checks the bundled runtime,
uv, Python 3.11+, the connector environment, and the core production gates without starting an
ingestion run. Fixed tool installers require a confirmation and native approval, expose bounded
logs, and can be cancelled or retried.

The Settings **Sources** panel edits typed connector configuration, workspace assignments,
credential references, and per-source safety budgets. Secret values are write-only. External
command connectors remain read-only because their command arrays cross the native process
boundary. After saving, **Validate** runs the bundled runtime with fixed limits of 25 documents,
5 MiB, and 60 seconds. It can be cancelled, never embeds or indexes content, and records only a
metadata outcome.

Closing the main window hides it to the tray. Use **Quit Cortana Desktop** from the tray menu to
exit the control plane. This does not stop the independently managed Cortana services.
