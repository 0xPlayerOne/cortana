# Cortana Desktop

Cortana Desktop is the Tauri 2 control plane for the independently runnable Cortana runtime.
It reuses the React/Vite workspace in `apps/web` and talks to the local owner-only API through
narrow Rust commands. The renderer has no arbitrary shell or filesystem capability.

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

Signed, updater-compatible DMG, AppImage/deb, and Windows installers are created only by the
release workflow. That workflow has access to the dedicated updater signing key; local builds do
not require release secrets.

Closing the main window hides it to the tray. Use **Quit Cortana Desktop** from the tray menu to
exit the control plane. This does not stop the independently managed Cortana services.
