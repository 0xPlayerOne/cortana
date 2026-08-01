# Cortana Desktop

Cortana Desktop is the Tauri 2 control plane for the independently runnable Cortana runtime.
It reuses the React/Vite workspace in `apps/web`, ships the matching Cortana core binary as a
platform-specific sidecar, and talks to the local owner-only API through narrow Rust commands.
The renderer has no arbitrary shell or filesystem capability.

Source setup is explicit and non-ingesting. The source editor can open fixed official provider
setup pages, pick source/token/client paths through native dialogs, authorize saved Google sources
through the bundled sidecar, and run bounded validation. Provider credentials never pass through
the webview.

The knowledge view provides a workspace/source/document tree backed by ACL-filtered, keyset
pagination. Selecting a document opens bounded canonical content and its original-source link.
The webview reaches this data only through fixed typed loopback commands; it cannot open the index
database or select a backend URL.

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
logs, and can be cancelled or retried. On a first launch, the guided setup opens this panel and
runs the read-only scan automatically; every installation still requires separate approval.

The Settings **Sources** panel edits typed connector configuration, workspace assignments,
credential references, and per-source safety budgets. Secret values are write-only. External
command connectors remain read-only because their command arrays cross the native process
boundary. After saving, **Validate** runs the bundled runtime with fixed limits of 25 documents,
5 MiB, and 60 seconds. It can be cancelled, never embeds or indexes content, and records only a
metadata outcome. **Initial sync** plans first: it resolves one of three fixed budgets (100
documents/25 MiB/15 minutes, 500/64 MiB/30 minutes, or 2,000/128 MiB/60 minutes), requires a
read-only validation at equal or larger limits (with a **Validate for this budget** action when
the latest record is smaller), and then runs a separately confirmed, validation-gated,
no-reconcile sync through the same cancellable source-job boundary with visible progress and
metadata-only audit events.

Closing the main window hides it to the tray. Use **Quit Cortana Desktop** from the tray menu to
exit the control plane. This does not stop the independently managed Cortana services.

The **Services** panel shows structured launchd state and controls installed embedding, server,
backup, and (only when explicitly installed elsewhere) sync jobs through fixed native actions.
Desktop-at-login controls only the tray application and never installs or starts ingestion.

Aggregate service actions control only embedding and server; recurring sync and backup are always
excluded. **Updates** uses the fixed signed GitHub feed, reports download progress, and requires
confirmation before native signature verification, installation, and restart. **Access** manages
named scopes and ACL labels with write-only bearer values. **Audit** displays bounded metadata-only
runtime and Desktop events without queries, content, command logs, or credentials.

The **Advanced** panel can export a versioned JSON settings backup or import one as an unsaved
preview. Portable files never include secret values or executable external-connector commands.
Import preserves existing external connectors, validates the complete bounded settings contract,
and writes nothing until **Save changes** creates the normal rollback copy.
