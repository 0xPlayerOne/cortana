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

## Download and first launch

Normal users should download the matching package from the
[latest Cortana release](https://github.com/0xPlayerOne/cortana/releases/latest). Install it, launch
the app, and follow the guided **Readiness** panel. The panel is read-only until you approve a
tool installation or a source action: Cortana does not authorize accounts, index content, or start
recurring ingestion on its own. Create a workspace, configure one source, run **Validate**, and
then use the confirmation-gated **Initial sync** for a small trial. The repository
[README](../../README.md#desktop-first-launch-recommended) is the short user path; this file covers
the native control-plane boundary and contributor setup.

For the complete, plain-language checklist, use [Getting started](../../docs/getting-started.md).
The normal user journey is intentionally only six actions: install Desktop, approve optional
tooling, create one workspace, configure one source, validate it, and confirm one bounded initial
sync. Everything else in this file is an operational or contributor reference.

The published package and the source checkout are intentionally versioned separately. Check
`cortana --version` (or the Desktop About panel) before comparing behavior with the
[release history](../../docs/releases.md); source-tree hardening is not downloadable-release
evidence until its package, signatures, and packaged-core checks have been verified.

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
the release workflow. That workflow has access to the dedicated updater signing key. macOS release
builds additionally require the Apple Developer ID signing and notarization secrets; the workflow
fails closed instead of publishing an ad-hoc package when those trust gates are absent. Local builds
do not require release secrets and remain development-only ad-hoc bundles.

The Settings **Readiness** panel runs an explicit, read-only scan. It checks the bundled runtime,
uv, Python 3.11+, the connector environment, the configured local embedding runtime, and the core
production gates without starting an ingestion run. A local embedding provider requires
`text-embeddings-router`; on macOS with Homebrew available, the panel offers the fixed
`text-embeddings-inference` install after confirmation. Cloud embedding providers do not require
that local runtime. Installing the runtime does not download model weights or start a service;
weights are fetched by the embedding service on its first approved start. Fixed tool installers
require a confirmation and native approval, expose bounded logs, and can be cancelled or retried.
Readiness uses only the cryptographically matched sidecar shipped with this Desktop release. If it
is missing, the panel fails closed and asks the user to reinstall; it never uses a separate
`cortana` executable from `PATH`.
On a first launch, the guided setup opens this panel and
runs the read-only scan automatically; every installation still requires separate approval. Tauri
release bundles carry the connector source and metadata needed for the approved **Install** action:
it creates the per-user `~/.local/share/cortana/venv` environment with uv and installs the bounded
`ingestion` dependency set. The bundle never includes credentials or an existing user's venv.
After success, Desktop records the fixed connector executable in the managed config when no
connector command is already configured; existing commands are preserved. The change uses a
rollback copy and metadata-only audit event, so newly configured sources use the installed
environment without a shell or path supplied by the renderer.
Unbundled local release binaries use the generated checkout resource while packaged applications
prefer their embedded resource directory.

If the core readiness report finds a stored embedding generation that differs from the configured
fingerprint, the panel keeps the exact generation details visible and offers a confirmation-gated
**Adopt stored generation** action. It creates a verified backup, clears derived caches, and never
re-embeds or reconciles the corpus; model or dimension changes still require a new generation
rebuild.

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
The Knowledge sidebar mirrors each saved connector with a confirmation-gated enable/disable
switch for future ingestion; it never deletes indexed data and is locked while Settings has an
unsaved draft. Sources that need provider setup or Google authorization also expose the matching
browser action directly in the tree.

Closing the main window hides it to the tray. Use **Quit Cortana Desktop** from the tray menu to
exit the control plane. This does not stop the independently managed Cortana services.
When the window is hidden or unfocused, passive renderer health polling pauses and resumes with an
immediate refresh when the window returns. Installer, source-job, and updater progress remains
owned by the shell and continues while the app is in the background.

The **Services** panel shows structured launchd (macOS), systemd user (Linux), or per-user Task
Scheduler (Windows) state. It can explicitly install the safe query-only embedding/server/backup
set from the bundled runtime and controls installed embedding, server, backup, and sync jobs through
fixed native actions. A separate, confirmation-gated **Enable recurring sync** action re-checks
every enabled source's current validation coverage before installing the schedule. Windows tasks
are created for the logged-in user and do not require administrator access.
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

The Services schedule editor stores validated sync and backup intervals in the owner-only
`service-schedule.toml` beside the active configuration. Saving the schedule never starts a job;
the explicit **Enable recurring sync** action passes those intervals to the bundled runtime. If a
job is already installed, saving changed intervals exposes **Apply recurring sync schedule** until
the updated job is explicitly confirmed.
Portable settings remain redacted and machine-local, so this scheduler file is not included.

## Native acceptance tests

`bun run desktop:test:native` prepares the bundled sidecar and connector resources, then runs the
native acceptance suite: `cargo test native_` in `src-tauri` (a broader `desktop:test` runs the
whole crate). The suite drives the production Tauri command handlers through `tauri::test`
MockRuntime IPC dispatch with temporary config/secret/data directories, exercising settings save
redaction and workspace scopes, read-only autostart status, OAuth/setup fail-closed guards, a real
bounded filesystem validation job through the bundled `cortana` sidecar
(start/status/cancel/terminal result), schedule persistence, the real service status report, tray
close/show policy, and updater fail-closed checks. Sidecar-dependent tests skip with a note when
the sidecar was not prepared.

Headless harness boundaries that are deliberately not faked and remain manual acceptance on a real
desktop session: native file dialogs (settings import/export, path picking), window/tray GUI
events, autostart enable/disable, OAuth browser flows, OS service installation, and signed update
download/install. The tests never perform network requests or modify host configuration.
