# Cortana Desktop UX audit

This document defines the durable packaged-product acceptance contract for Cortana Desktop. It does not track current implementation status, open blockers, or release-specific pass/fail state.

Current Desktop work and evidence belong in [GitHub milestones](https://github.com/0xPlayerOne/cortana/milestones) and [GitHub issues](https://github.com/0xPlayerOne/cortana/issues). Tagged release evidence belongs in [Release history](releases.md).

## Audit boundary

Source tests, web tests, Rust tests, static archive inspection, checksum verification, updater-signature verification, and packaged-core evaluation are necessary but do not prove the packaged GUI.

A supported Desktop claim requires a real package to complete the relevant native flows on the named operating system and architecture.

The manual `Desktop acceptance` workflow adds a narrower published-package host lane: it extracts
or selects the release application, starts it with disposable configuration and data directories,
and stops it after a bounded startup window on macOS Apple Silicon, Linux x86_64, and Windows
x86_64. This proves packaged process startup and isolated state setup only. It does not replace
interactive GUI, browser OAuth, tray/service, native-dialog, updater, accessibility, or
macOS Developer ID/notarization acceptance.

The workflow may verify a historical published tag from a newer verifier checkout. In that case
it explicitly allows source-version drift, records the requested release version in the component
version fields, and preserves the verifier checkout versions plus a drift marker in the evidence.
On Linux, the host-launch lane installs the packaged application's GTK/WebKit runtime dependencies
and runs the AppImage under Xvfb before checking for stable process startup.

## Product principles

- Desktop is the primary human surface, not the canonical database or background-service authority.
- The Rust/Tauri process is the privileged boundary.
- The renderer receives no arbitrary shell, filesystem, credential, backend-address, updater, or service-manager capability.
- Ordinary knowledge, memory, graph, query, and status reads use the authorized local API.
- Privileged operations use typed, allowlisted native commands with argument validation, confirmation where required, atomic writes, and metadata-only audit.
- Closing the window does not silently stop approved background agent access.
- Source authorization, validation, ingestion, reconciliation, scheduling, restore, and update are separate user decisions.

## Acceptance matrix

### Installation and first launch

Verify:

- correct package and architecture selection;
- clean installation and launch;
- application, core, connector, web asset, and package version agreement;
- sidecar and local-runtime discovery;
- optional tooling detection;
- approval-gated tooling installation;
- progress, cancellation, retry, and remediation;
- healthy query-only startup;
- no implicit source authorization, model download, ingestion, schedule, shared principal, or memory write.

Evidence should include the exact package identity, operating system, architecture, application version, and outcome.

### Workspace management

Verify:

- create, rename, switch, and remove workspaces;
- stable generated internal identifiers;
- display name, icon, color, and theme behavior;
- account labels as optional metadata rather than credential authority;
- workspace-specific source assignment;
- no “all workspaces” retrieval path that bypasses isolation;
- state persistence across navigation, refresh, window close, and process restart.

Workspace selection is UX, not authorization. Backend project and ACL checks remain authoritative.

### Source setup and authorization

Verify supported provider-native flows:

- Google browser OAuth and callback;
- GitHub device flow;
- Apple Notes host permission and exact folder routing;
- filesystem/code directory selection;
- Discord Desktop RPC;
- Slack browser OAuth and team/channel assignment;
- Buzz identity/community discovery;
- write-only token or API-key fields.

For each source verify:

- enable/disable only from the source-management model;
- selected workspace and ACL;
- bounded discovery;
- read-only validation;
- visible budgets and write impact;
- explicit initial-sync confirmation;
- progress, cancellation, retry, and recovery;
- no secret returned to the renderer or logs.

Authorization and discovery must not imply validation or synchronization.

### Knowledge browser

Verify:

- workspace and source navigation;
- source/document hierarchy;
- keyset-paginated and virtualized lists;
- exact canonical document view;
- source identity, URI, update time, workspace, ACL context, and metadata;
- backlinks and nearby records;
- integrated scoped search and cited answers;
- result-gated answer, evidence, and timeline views;
- direct links to original sources;
- loading, empty, partial, degraded, offline, revoked, and error states;
- safe rendering of Markdown, HTML-derived content, code, and malformed records.

Previews and derived summaries never replace exact canonical content.

### Graph exploration

Verify:

- bounded initial page;
- progressive node expansion;
- keyset pagination and load-more behavior;
- node and edge type filters;
- local search and focus;
- selected-node relationship explanation;
- provenance and confidence for derived edges;
- navigation to supporting documents, memories, and code;
- cancellation and renderer-memory cleanup;
- full-width graph mode and restoration of the prior workspace layout;
- no unbounded full-corpus load or traversal.

Graph traversal must not expand ACL visibility.

### Native memory

Verify:

- explicit retain/remember, recall, supersede, expiry, export, and forget;
- content type and retention-tier presentation;
- project and ACL scope;
- provenance, confidence, importance, and lifecycle status;
- evidence/memory distinction in answers and ContextBundles;
- tombstone and redaction behavior;
- no automatic source-to-memory copying;
- candidate, consolidation, contradiction, and reflection UX only when the corresponding backend policy is explicitly enabled.
- searchable, virtualized candidate queues for pending, approved, auto-retained, rejected, expired,
  failed, and dead-letter states, with bulk actions capped at 25 records;
- explicit confirmation for canonical promotion, including edit-and-approve, keep-working,
  supersede, and retry actions;
- owner-only persisted pause/resume controls plus bounded retention ceilings, candidate review
  expiry, and active-memory capacity; recurring processing and automatic retention remain disabled
  until an operational scheduler and release gate ship;
- visible provenance, support, classification, policy identity, confidence, sensitivity, expiry,
  attempts, and failure state sufficient to explain why a record exists or was superseded;
- keyboard-operable native controls, announced errors/status changes, responsive layouts, and
  usable large-queue behavior at 200% zoom.

Derived observations or reflection output must not be displayed as canonical memory or source evidence.

### Settings and secrets

Verify:

- complete non-secret configuration coverage;
- local and cloud embedding selection;
- optional query-provider configuration;
- provider-advertised model discovery with custom fallback;
- staleness invalidation when endpoint, mode, or key reference changes;
- write-only secret fields;
- explicit migration to macOS Keychain, Windows Credential Manager, or Linux Secret Service;
- rollback and redacted recovery when native credential storage is unavailable;
- import/export with redaction;
- generation-aware embedding changes;
- automatic restart of only the affected services;
- visible rollback and remediation on failure.

Secret values must not appear in renderer state, configuration exports, audit, crash reports, or logs.

### Services, tray, and background behavior

Verify:

- install, start, stop, restart, inspect, and uninstall per-user services;
- guarded Start All, Stop All, and Restart All;
- recurring synchronization excluded from generic service activation;
- close-to-tray, reopen, explicit quit, and stop-services choice;
- menu-bar status and single-instance behavior;
- login autostart;
- service crash, stale state, port conflict, sleep/wake, logout/login, and restart recovery;
- bounded durable activity for running, completed, failed, and cancelled service actions;
- retry and recovery after timeout, late completion, or a concurrent-action rejection;
- MCP and HTTP availability while the window is closed;
- status derived from durable operational state rather than renderer assumptions.

### Backup, restore, import, and export

Verify native dialogs and privileged file operations for:

- source path selection;
- settings import/export;
- memory export;
- audit export;
- backup destination;
- restore input.

Verify:

- online SQLite snapshot creation;
- independent backup verification;
- explicit restore confirmation;
- recovery copy of replaced data;
- corrupt, incompatible, symlinked, inaccessible, and low-disk failure paths;
- post-restore evidence, memory, ACL, revision, source, and readiness checks;
- rollback instructions.

### Updater

Verify:

- update discovery;
- current and available version;
- release notes and source link;
- explicit approval;
- download progress and cancellation;
- signature and manifest validation;
- archive safety;
- package/core/sidecar version agreement;
- restart and service recovery;
- preserved configuration, index, memory, credentials, backups, and policy;
- network, signature, manifest, disk, partial-download, and interrupted-restart failures;
- upgrade and documented rollback from a supported prior version.

An update must not enable a source, schedule, remote principal, or synthesis policy.

### Visual system and responsive layout

Verify:

- the checked-in shadcn configuration, selected primitive base, icon library, aliases, and Tailwind entrypoint agree with the generated components;
- shared shadcn primitives own ordinary actions, forms, cards, navigation, overlays, menus, status, loading, empty, and error states;
- renderer-specific graph, virtualization, and native-window components compose the shared tokens and controls without recreating parallel primitives;
- every supported theme uses the same semantic token contract and preserves readable hierarchy, contrast, focus, status, and destructive treatments;
- the shell, setup, sources, workspace, search, document, context, graph, memory, services, settings, backup, updater, activity, and recovery surfaces form one coherent visual language;
- phone, tablet, compact desktop, and wide desktop layouts remain usable at 320, 768, 1024, and 1440 CSS pixels;
- content remains operable at 200% zoom without clipped controls, hidden actions, overlapping regions, or two-dimensional scrolling except where the content itself requires it;
- loading, empty, partial, degraded, offline, busy, success, warning, destructive, and error states retain stable layout and clear recovery actions;
- dialogs, sheets, dropdowns, popovers, tooltips, command surfaces, and native-dialog launch points have correct placement, stacking, focus entry, focus return, and dismissal behavior;
- no ordinary surface retains superseded page-local button, form, card, tab, menu, tooltip, dialog, or status styling, and no control falls back to browser-native chrome.

Capture matched before/after screenshots using deterministic fixtures for every primary surface.
The evidence matrix must include all four target widths, the default and accessibility themes at
every width, every other supported theme at desktop and compact widths, representative populated
data, and all loading, empty, error, destructive-confirmation, dialog, sheet, menu, and tooltip
states. A build, component test, or sampled screenshot is not visual acceptance. Review must record
the complete matrix and any intentional renderer-specific exception.

### Accessibility

Verify:

- full keyboard operation;
- visible focus and focus restoration;
- screen-reader names and roles;
- heading and landmark hierarchy;
- alert versus status semantics;
- WCAG 2.2 AA text, non-text, and focus contrast;
- zoom and responsive resizing;
- reduced-motion behavior;
- tooltip accessibility;
- disabled and busy states;
- error recovery without pointer-only interaction;
- minimum 24-by-24 CSS-pixel targets and comfortable touch targets for primary mobile actions;
- no keyboard trap or focused control obscured by sticky or layered content;
- automated axe coverage plus manual VoiceOver, NVDA, or platform-equivalent screen-reader evidence on supported packaged lanes.

Accessibility is a release requirement, not optional polish.

For deterministic knowledge-browser and graph regression coverage, run:

```bash
bun run test:knowledge-accessibility
```

The `Knowledge accessibility` pull-request workflow runs that command in Chromium and uploads a
bounded JSON report plus desktop reduced-motion, desktop 200%-zoom, and mobile screenshots. The
gate exercises keyboard entry and graph selection, WCAG 2.2 AA axe rules, accessible graph-filter
names, live selection status, reduced motion, responsive layout, and console errors against the
provider-free demo fixture. It contains no private source content. It complements the published
package host-launch lane; a passing renderer report is not evidence that VoiceOver, NVDA, native
dialogs, browser OAuth, tray behavior, updater behavior, or operating-system trust passed in a
signed package.

### Security

Verify:

- fixed Tauri capabilities and CSP;
- renderer compromise boundaries;
- no arbitrary shell or filesystem access;
- no secret or private path disclosure;
- path and symlink defenses;
- owner-only files;
- atomic configuration publication;
- metadata-only audit;
- prompt injection treated as untrusted document content;
- account switching and revocation;
- remote bind safeguards;
- update and supply-chain verification.

### Resource and large-corpus behavior

Measure:

- startup and first-ready latency;
- idle and active CPU;
- renderer and native memory;
- source/document list rendering;
- graph open and expansion;
- request count and response bytes;
- database latency;
- cancellation cleanup;
- background-service overhead;
- behavior under slow storage and low-memory conditions.

No primary view may load the full corpus or graph into memory.

## Evidence record

Each packaged acceptance record should include:

- release/tag and package checksum;
- operating system and architecture;
- installation type;
- application/core/connector versions;
- exact acceptance cases run;
- pass/fail/blocked result;
- the complete non-secret visual evidence matrix for renderer changes;
- non-secret logs or recordings where useful;
- known limitations;
- linked issues for failures;
- reviewer and date.

Do not put credentials, source content, private queries, tokens, or local absolute paths in the record.

## Planning boundary

This audit defines what must be tested. [GitHub milestones](https://github.com/0xPlayerOne/cortana/milestones) and [GitHub issues](https://github.com/0xPlayerOne/cortana/issues) own which cases are pending, who owns them, their sequence, blockers, and current evidence.
