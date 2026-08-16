# Cortana Desktop UX audit

This audit maps the current Desktop implementation to the UI/UX objective. It is
intentionally separate from runtime migration work:
legacy scope quarantine remains in place, so changing or deleting indexed data
is not part of a visual/UI change.

## Current release evidence (2026-08-16)

- `v0.32.12` is the current protected source/release, published through the protected promotion
  and Release Please automation. Release-assets workflow `31933279147` completed all 18 assets,
  checksums, updater signatures, manifest, and packaged-core verification. Asset verification does not launch the
  packaged GUI or prove OS-level signing/notarization.
- The supported v0.32.12 Desktop matrix is macOS Apple Silicon (arm64), Linux x86_64, and Windows
  x86_64. No Intel macOS Desktop bundle is published; Intel macOS is an explicit unsupported
  target for this release, not a passing or pending GUI gate. Rosetta or the core archive does not
  provide Intel Desktop evidence.
- The v0.32.12 tag is the fully verified packaged evidence boundary. The audited host
  now runs the installed `/Users/amf/.local/bin/cortana` v0.32.12 with embedding and server services in
  query-only mode; recurring sync remains disabled. Query-only readiness and
  source authorization are separate host checks; source authorization and full-corpus sync were
  not started. The packaged GUI, browser OAuth, tray/menu, native dialogs, updater interaction,
  Developer ID signing, and notarization remain manual gates.
- The installed v0.32.12 host passed the bounded provider-backed `eval --model` evidence
  run on 2026-08-16 in 23,267 ms: retrieval recall, MRR, case pass rate, citation validity,
  planner/synthesis execution, cache reuse, revision invalidation, and bounded provider behavior
  all passed with no fallback. This is provider-backed fixture evidence only; it does not query
  the personal index or prove packaged GUI behavior.
- A historical installed v0.32.2 CLI passed the disposable offline control-plane drill on
  2026-08-15; this evidence predates the current v0.32.12 source and package:
  initialization, bounded two-document ingest, hybrid search/context, metadata-only audit export,
  verified backup, restore into a second temporary data directory, SQLite verification, and
  post-restore search. The drill used only temporary data and does not prove packaged GUI, OAuth,
  tray, native-dialog, updater, or operating-system trust behavior.
- Historical v0.32.6 evidence: the macOS ARM package verifier passed the published updater signature, packaged-core
  offline evaluation, and strict codesign checks. Gatekeeper still rejects the ad-hoc bundle because
  Developer ID signing and notarization are not configured; the GUI was not launched.
- Historical v0.32.6 evidence: the then-installed CLI also passed the disposable offline control-plane
  drill on 2026-08-15:
  bounded two-document ingest, hybrid search/context, metadata-only audit export, verified backup,
  restore into a second temporary data directory, SQLite verification, and post-restore search.
  The drill used only temporary data and does not prove packaged GUI, OAuth, tray, native-dialog,
  updater, or operating-system trust behavior.
- The preceding installed v0.31.16 configured-provider fixture evaluation passed in 14,660 ms with
  planner and synthesis model use, valid citations, cache reuse, and revision invalidation.
  It remains historical synthetic fixture evidence, not a personal-index or packaged-GUI benchmark. The
  opt-in model evaluator remains fail-closed below one minute with a 55-second
  whole-run bound; extractive mode remains the production default and recurring sync
  remains uninstalled because an explicit `readiness --allow-sync-service` check failed closed:
  enabled filesystem/code sources are still bounded samples and connector records are below
  their configured full-sync budgets.
- The complete native Tauri suite passes 130 tests in 2.45 seconds after
  compilation. The current `bun run test` gate passes 263 tests across 24 files
  under the CI-pinned Bun 1.3.14, including the script tests. Its runner groups
  pure suites and executes API-mocking suites in separate Bun processes so one
  mocked bridge cannot leak into another file. The Python package gate passes
  184 tests, including the retired-model runtime guard. These are
  headless source checks and do not
  substitute for the still-unverified interactive packaged GUI flows.
- The published v0.31.12 macOS ARM archive was statically inspected on 2026-08-13 (historical):
  `Contents/MacOS/cortana --version` reports `cortana 0.31.12`, the bundle passes
  strict `codesign --verify --deep --strict`, and `spctl --assess` still rejects
  it because Developer ID signing and notarization are not configured. The
  archive was not launched. The static verifier now selects the host
  architecture (or an explicit `CORTANA_MAC_ARCH` override) and fails closed
  when the release does not publish a matching app archive; v0.31.12 publishes
  only the ARM64 macOS app, so Intel macOS remains an explicit packaging gap.
- The then-installed v0.31.12 binary passed the disposable offline control-plane
  drill: init, bounded fixture ingest, hybrid search/context, metadata-only
  audit export, verified backup, restore into a second temporary data directory,
  SQLite verification, and post-restore search. The drill touched no live
  indexed data and does not prove packaged GUI, OAuth, tray, or updater flows.
- A bounded end-to-end Discord sync trial on 2026-08-12 covered all three enabled
  authorized sources with one document, a 64 KiB cap, and a 30-second cap per
  source. Each source completed with `changed=0`, `unchanged=1`, and `deleted=0`.
  All three trials were non-reconciling and did not install recurring sync. This
  proves the selected connector-to-embedding-to-index path, not full-corpus
  readiness.
- A historical validation-only source smoke on 2026-08-13 passed the 21 sources
  that were enabled at that time at the same one-document/64 KiB/30-second bounds,
  without embedding, indexing, reconciliation, or scheduler changes. The current
  operator inventory is narrower: 13 sources are enabled (Apple Notes, Drive,
  Gmail, Calendar, and Buzz), while Discord and code/filesystem roots are disabled
  and Slack is unconfigured. The historical sweep is authorization/reachability
  evidence only and must not be read as current source authorization.

The current v0.32.12 source includes the post-v0.31.12 safety lane, which acquires
the global `sync.lock` before mutating CLI startup, bounds direct JSONL imports and custom fixture
parsing before resource-heavy work, fences optional-memory outbox leases, and serializes Desktop
sidecar preparation with atomic publication. Native Desktop settings and schedule writes also share
a per-config cross-process lock. These source-tree protections are covered by focused
regressions; they do not authorize a source, enable recurring sync, or prove the unverified
GUI/browser/tray/dialog/updater gates above.

The v0.32.12 source adds one operational recovery change for the local embedding supervisor:
steady-state checks use the lightweight `/health` endpoint so queued ingestion work cannot look dead;
startup and restart still require a real vector probe. The earlier v0.32.1 Work Drive trials were
cancelled after that older supervisor stalled. After v0.32.2 installation, a foreground Work Drive
trial progressed through bounded unchanged batches before the local embedding health probe timed
out while queued embedding work was still completing; it was cancelled after roughly seven minutes.
The service recovered afterward, and the trial made no deletions or reconciliation. Work
Historical records show Work Drive (478 documents/4,527,663 bytes) and Work Gmail
(7,395 documents/34,494,647 bytes) completing production-budget validation. Work Drive has a successful bounded 100-document
retry, and Work Gmail now has a bounded 100-message pass (`changed=75`, `unchanged=25`,
`deleted=0`); neither source has a complete production-budget trial approved for reconciliation or
recurring sync.
Personal Drive failed its 2,000-document/128 MiB/900-second validation at the 899-second connector
timeout. A follow-up 25-document/5 MiB/60-second validation and non-reconciling trial succeeded
(`changed=1`, `unchanged=24`, `deleted=0`), but that bounded prefix remains below the production gate.

The current validation metadata now contains production-budget complete records for Apple Notes
(28/65/8 documents), Calendar (2,207/1,836/0 events), Buzz (45 records), Work Drive (478
documents/4,527,721 bytes), Work Gmail (7,386 messages), Personal Gmail (427 messages), Special
Gmail (216 messages), and Special Drive (97 documents). Personal Drive's 2,000-document/128 MiB/
1,800-second validation failed closed at the 1,799-second connector deadline under the
then-installed v0.32.9 parser while processing a large PDF.
The recurring sync service remains uninstalled, and no reconciliation or large sync has been run.

Historical bounded source-smoke and non-reconciling trials remain useful connector evidence, but do
not override the current production-budget validation gate. Discord and code/filesystem sources are
disabled by operator choice, Slack is unconfigured, and native Desktop acceptance remains a separate
manual gate.

On 2026-08-15 a second bounded Work Drive retry emitted the complete 478-document connector
snapshot, then failed closed when the local embedding connection closed during ingestion. The run
was non-reconciling, so it made no deletions; completed-prefix writes are retained by the controlled
ingestion contract. The embedding supervisor restarted the router and query-only readiness passed
after recovery. A subsequent v0.32.4 run completed 100 unchanged documents with 0 deletions under
a 300-second bound. This closes the bounded retry observation but does not prove a complete
478-document production trial or authorize reconciliation/recurring sync.

## Requirement matrix

| Area                                                   | Current evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | Status            | Follow-up                                                                                                                                                      |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Theme system and readable foreground/background tokens | `apps/web/src/theme.ts`, `apps/web/src/styles/tokens.css`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Done              | Add visual contrast snapshots before adding more themes.                                                                                                       |
| Cortana icon and blue default theme                    | `Navigation.tsx` uses `/app-icon.svg`; blue theme is the default                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | Done              | Verify packaged assets on each signed desktop build.                                                                                                           |
| Consistent buttons and hover tooltips                  | Shared `--btn-*` size/padding tokens drive chrome (34px), compact action (30px), and text action (38px) buttons; workspace/source card controls use the same compact icon contract and the last sharp 4px corners now use `--radius-xs`; the settings banner actions match the secondary-button style                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Done              | Keep the shared contract as new action groups are added; visual snapshot pass before adding themes.                                                            |
| Fast tooltips                                          | `quick-tooltip` uses an 80ms opacity transition and now covers the remaining title-only icon buttons (status bar indicators via a `quick-tooltip--above` variant, source tree, favorites, graph nodes, context panel, workspace/principal remove, path choosers); rows inside scroll containers keep the native title fallback so the tip is never clipped away                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | Done              | Add a first-hover interaction check in the packaged GUI pass.                                                                                                  |
| Background service restart after saving                | `restartAfterSaveIfNeeded` restarts core services and clears the notice on success; failures now keep an alert banner that names the failure with `Retry restart` and `Open services` recovery actions; source-toggle saves from Knowledge also restart in the background and report named failures in the status bar                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Done              | Keep the failure-recovery path covered by regression tests.                                                                                                    |
| Accessible error and status semantics                  | Settings service, database, provider-model, and portable-settings notices use red `role="alert"` semantics only for failures; successful operations and advisory states remain `role="status"`, while the shared safety-note styles keep the visual distinction                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | Done              | Keep new operational failures on the alert contract and warnings on the status contract.                                                                       |
| Human-readable changelog                               | `SafeMarkdown` renders headings, lists, inline code, and safe links                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | Done              | Add coverage for fenced/code-heavy release notes and malformed links.                                                                                          |
| Workspace display name, generated ID, logo, and color  | `WorkspaceSection`, `WorkspaceLogo`, and `workspaceLogoStore.ts`; workspace marks are 20/30/48px across picker, cards, and detail views, and Advanced workspace fields keep an explicit bottom separation before Color                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Done              | Keep the internal ID out of the default form; retain it only under Advanced; preserve the spacing and logo-size regression coverage.                           |
| Workspace account label semantics                      | Label is explicitly optional metadata; OAuth credentials remain source-owned and the field uses a neutral example                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Done              | Derive a provider identity only when authorization metadata is available and explicitly approved.                                                              |
| Workspace-scoped source settings                       | `SourcesSection` now provides workspace tabs, per-workspace source counts, and a Needs assignment quarantine view; add-source targets the selected workspace                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Done              | Keep the tab isolation and assignment warning covered by regression tests.                                                                                     |
| Source logos and compact OAuth/enable actions          | `SourceIcon` and provider-specific actions exist; advanced fields are disclosed and source-card operations use icon-only controls with accessible labels and fast tooltips                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Done              | Add provider-specific discovery actions as OAuth integrations land.                                                                                            |
| OAuth-first source setup                               | Google, GitHub, and Slack use fixed browser OAuth flows; Discord uses the signed-in Desktop RPC client. Every provider has bounded responses and owner-only token files                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Done              | Keep provider discovery/authorization covered by regression tests.                                                                                             |
| Model selectors                                        | Embedding and query settings use a Custom field until the configured provider advertises a bounded model catalog through `cortana provider-models`; only the bundled local Qwen embedding presets remain static, and capabilities are echoed only when the provider explicitly advertises them (never inferred from names)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Done              | Keep the staleness guard (catalog is discarded when the endpoint, mode, or key variable changes) and the custom-model fallback covered by regression tests.    |
| Plugins grouping                                       | Hindsight and Honcho are grouped under Plugins                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Done              | Keep both disabled by default and add live opt-in evaluation before enabling.                                                                                  |
| Settings ordering                                      | Services, Workspaces, Sources, and Readiness are the primary group above the divider                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Done              | Preserve this order as sections grow.                                                                                                                          |
| Knowledge workspace selector                           | Uses workspace logo + display name; no “All workspaces” option                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Done              | Add a compact tab/pill selector when more than one workspace exists.                                                                                           |
| Strict workspace segregation                           | UI requests are scoped to the active workspace; backend/config reject unknown configured source projects. Legacy rows are quarantined and legacy public-ACL rows are now zero (stale corpus remains quarantined).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Done (quarantine) | Keep owner-scoped checks enforced across retrieval/document/search/context/answer/MCP and preserve the quarantine label until an explicit mapping is approved. |
| GitHub repository selection                            | OAuth device flow and bounded repository chooser are implemented and included in release v0.27.2; auth-owner behavior is now released in v0.27.3.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Done              | Keep end-to-end repository selection coverage and release-note alignment for future connector updates.                                                         |
| Discord server/community assignment                    | `cortana authorize-discord SOURCE`, `discord-servers`, and `discord-channels` use the signed-in Discord Desktop client's local RPC socket with bounded, read-only guild/channel discovery. The Desktop chooser persists checked guilds and channels into the per-source `servers`/`channels` fields, which are per-workspace because each Discord source belongs to exactly one workspace. No credential-scraping path is retained.                                                                                                                                                                                                                                                                                                                                                                                                                          | Done (this pass)  | Keep RPC authorization, bounded discovery, and snapshot-based message capture covered by regression tests.                                                     |
| Slack workspace assignment                             | `cortana authorize-slack SOURCE` (Authorization Code + PKCE against the fixed endpoints `https://slack.com/oauth/v2/authorize` and `https://slack.com/api/oauth.v2.access`, with the exact loopback redirect `http://127.0.0.1:47521/callback` registered in the Slack app) and `cortana slack-workspaces SOURCE` (bounded `team.info` result from the stored user token with one-shot refresh when token rotation is enabled) add browser OAuth workspace authorization; the Desktop workspace chooser persists the checked team ids into the per-source `teams` field with display names index-aligned in `team_names`, which is per-workspace because each Slack source belongs to exactly one workspace. `SLACK_BOT_TOKEN` stays the message-sync credential and is never interpreted as a path; token-only setups keep the original behavior unchanged. | Done (this pass)  | Keep OAuth workspace discovery covered by regression tests; channel selection and message sync intentionally stay bot-token based.                             |
| Buzz community assignment                              | `cortana buzz-communities SOURCE` lists the bounded communities recorded in the source's read-only `agents/teams.json` identity file (stable `id` + `name` records; the file must be a regular, non-symlink JSON array bounded at 512 KiB and missing, malformed, or duplicate entries fail closed). The Desktop community chooser persists the checked community ids into the per-source `communities` field with display names index-aligned in `community_names`, which is per-workspace because each Buzz source belongs to exactly one workspace. Discovery is read-only: it never runs ingestion or sync and never infers identity from persona event content; the read-only connector behavior is unchanged.                                                                                                                                          | Done (this pass)  | Keep identity-file validation and per-workspace community assignment covered by regression tests.                                                              |
| Workflow folder removal and tree indentation           | Knowledge uses the workspace-scoped source tree only; the document explorer heading is a `Workspace / Source` breadcrumb (with `Workspace / All sources` while browsing a workspace with no source selected) and virtualized document rows carry the `document-node` indentation class that the stylesheet nests under the source level; no workflow/folder labels exist anywhere in Knowledge display data, all asserted by regression tests in `SourcePanel.test.tsx`                                                                                                                                                                                                                                                                                                                                                                                      | Done              | Re-verify breadcrumb ellipsis and row indentation on the packaged GUI.                                                                                         |
| Apple Notes folder routing                             | Apple Notes sources now expose exact include/exclude folder lists in Desktop and TOML. The connector applies those filters before the document cap and preserves account/folder metadata, so one Notes account can safely feed multiple workspaces without fuzzy matching.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Done (this pass)  | Complete a packaged GUI trial with the user's real folder names before enabling larger or reconciling runs.                                                    |
| Service state from Sources                             | The Knowledge Sources sidebar (`role="switch"` per source) and Settings > Sources (enable checkbox) are the only source enable/disable surfaces; Settings > Services renders process health only (per-service Start/Stop/Restart, Start/Stop/Restart all, install, autostart, and the validation-gated recurring-sync schedule) with a regression test asserting Services never exposes a source enable control                                                                                                                                                                                                                                                                                                                                                                                                                                              | Done              | Keep the Sources-only enablement invariant covered as new connectors land.                                                                                     |
| Result views hidden before search                      | Answer/Evidence/Timeline tabs are result-gated; Graph is a rail action                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Done              | Keep this invariant in regression tests.                                                                                                                       |
| Full-screen Graph alternative                          | Graph is a dedicated rail action; while active the source and context panels collapse (`--source-width`/`--context-width` go to 0) so the graph spans the full workspace width, the duplicate top tab stays removed, and the title-bar source action leaves the graph so the panel is reachable again                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Done              | Keep the full-width layout and panel-restore invariants covered by regression tests.                                                                           |
| Hierarchical Graph exploration                         | The paginated graph view now renders bounded workspace, source, and document nodes; type filters, local text filtering, node-specific icons, selected-node relationship summaries, and explicit load-more pagination keep large corpora responsive without loading the corpus into memory                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Done (this pass)  | Add packaged-GUI visual/performance acceptance for a large corpus.                                                                                             |
| Source icon fidelity                                   | Provider mappings exist for code, Drive (brand mark), calendar, Gmail, Slack, Buzz, Discord, and Apple Notes (brand mark with a `StickyNote` fallback, never the code glyph); a regression test asserts the brand `path` for Notes and Drive and the lucide fallback for glyph-only connectors                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Done              | Keep brand mappings aligned with Simple Icons releases; verify licensing per artifact.                                                                         |

## Production blockers before calling the Desktop complete

1. Complete packaged GUI/browser OAuth, tray/menu, native file-dialog import/export, and
   signed-updater drills on a supported Developer ID/notarized Desktop build. The packaged CLI
   control-plane and backup/restore paths are now verified, and the native acceptance suite covers
   the command handlers; the GUI-only portions remain unverified because no callable Computer Use
   session is available here.
2. Model-backed provider gate: the verified v0.32.12 package passed the credential-free packaged-core
   evaluator and query-only readiness after installation. The current installed v0.32.12 CLI passed
   the bounded provider-backed run on 2026-08-16 in 23,267 ms with planner/synthesis, valid
   citations, cache reuse, and revision invalidation. That run used synthetic fixtures only; no
   provider-backed evaluation against a personal index or the packaged GUI is claimed. Provider
   outages or slow responses still fail closed, and extractive mode remains the safe production
   default. The current package also passes `doctor` and the disposable control-plane/recovery
   drills, while the GUI remains unlaunched.
3. Provider-advertised model metadata is implemented and bounded by
   `cortana provider-models`; keep the provider capability contract covered as
   supported query/answer providers evolve.
4. Discord Desktop RPC authorization/server discovery and per-workspace server/channel
   persistence are implemented, but live Discord authorization is currently disabled by
   operator choice while the previous bot/RPC credential is unavailable. Slack workspace
   discovery and per-workspace team assignment are implemented as an optional connector (the
   `SLACK_BOT_TOKEN` path for channel selection and message sync is preserved and never
   interpreted as a path). Buzz community assignment is also implemented: `cortana
buzz-communities SOURCE` reads the read-only `agents/teams.json` identity file with bounded,
   fail-closed validation, and the Desktop chooser persists per-workspace
   `communities`/`community_names`.
5. The memory-provider decision is recorded: keep Hindsight and Honcho as
   disabled-by-default optional adapters. Cortana's canonical store remains the
   source of truth; Hindsight is the replacement-capable sidecar and Honcho is
   an append-only experimental sink. Neither is enabled for personal data until
   provider ACL, deletion, export, and packaged-UI gates are explicitly proven. The fresh offline
   comparative fixture (`uv run cortana-memory-eval`) reports `material_gain=true`, with recall and
   MRR gains of `0.375`; that is useful evidence for a future opt-in review, not live-provider proof.
6. Complete source authorization and full validation coverage before recurring sync. The current
   operator installation has 13 enabled sources: Apple Notes, Drive, Gmail, Calendar, and Buzz;
   Discord and all code/filesystem roots are disabled by operator choice, and Slack is not
   configured. Apple Notes has complete folder-scoped validation and bounded no-reconcile
   snapshots; Calendar has complete validation with bounded 100-event Work, Personal, and Special
   trials; Buzz has a
   completed bounded no-reconcile snapshot. Historical records show Special Drive with a
   production-budget validation (97 documents, 290,353 bytes) and a completed 97-document
   non-reconciling trial with zero deletions. Current Work Drive and Work Gmail records show
   production-budget validation (478/4,527,721 bytes and 7,386/34,487,878 bytes respectively).
   The earlier v0.32.2 Work Drive trial was cancelled while queued embedding work was still
   completing, but the later complete 478-document validation is now authoritative. Personal
   Drive failed both its earlier 1,799-second and later configured 899-second connector deadlines;
   bounded 25-document/5 MiB validation and non-reconciling trial evidence remains below the
   configured production budget. Personal Gmail now has
   production-budget validation
   (430 documents/1,563,456 bytes) plus a bounded 100-document-cap trial with zero deletions,
   and Special Gmail has historical production-budget validation (214 documents/995,335 bytes)
   plus the same bounded trial result. These capped prefixes prove selected
   connector behavior, not full-corpus readiness. Recurring sync must remain uninstalled until
   every enabled source has a fresh `complete=true` validation at its configured production
   budget.

## Evidence limits

### Historical/provider audit (archived evidence through v0.30.10)

The evidence in this section is retained for incident and migration history. It
does not describe the current v0.32.12 source or the historical v0.32.6 core; use the
current-release section above for sign-off status.

- A tracked-source scan found no Spark model, provider, configuration, or dependency. The only
  remaining `Spark` matches are Lucide `Sparkles` icons used by the Query navigation surface.
- Rust `clippy --all-targets --all-features -- -D warnings`, Python Ruff/format/mypy, the web
  type-check, and ESLint all pass on this tree. No generated build/cache directory is tracked.
- Desktop Cargo helpers now restore the Release Please lockfile marker even when Cargo rewrites the
  lockfile; `scripts/desktop-lockfile.test.mjs` covers missing and already-present annotations.
- Remaining `legacy` references are active migration, ACL-quarantine, and embedding-generation
  safety paths. They are not dead Spark-era code; deleting them before existing configurations are
  migrated would orphan source scopes or weaken the fail-closed migration boundary.

- The v0.30.10 release snapshot (tag commit `b46dda8`, workflow `31515684053`)
  is historical evidence. It completed its then-current asset and signature
  checks, and the then-installed CLI reported `cortana 0.30.10`; neither proves
  the current `v0.32.12` source or packaged Desktop behavior. The verified
  `v0.32.11` asset workflow is historical; the active `v0.32.12` workflow is recorded in the release section above.
- Historical v0.30.0, v0.30.2, and v0.30.7 evidence remains useful for release
  investigations, but it must not be read as current-release proof.
- A static drill of the published `Cortana_0.29.64_aarch64.app.tar.gz` archive found the expected
  `Cortana.app` bundle, executable, and `Info.plist` version `0.29.64`; `codesign --verify --deep
--strict` passed. This proves archive integrity and local signature structure only: the app was
  not launched, notarization was not assessed, and tray, native dialogs, OAuth, and updater UI
  remain manual gates.
- Historical local developer-bundle checks at v0.29.69 regenerated the expected arm64 app and
  connector sidecars, but used the deliberate `bundle:mac --no-sign` path. They are retained only
  as historical evidence; strict signature validation is not claimed for that artifact and the
  v0.30.10 release asset is authoritative.
- A static check of the published v0.30.3 macOS ARM app archive reports
  `CFBundleShortVersionString=0.30.3` and passes `codesign --verify --deep --strict`. `spctl --assess`
  exits 3 because Developer ID signing/notarization is not configured; the app was not launched.
- A headless v0.29.66 macOS ARM packaged-app drill verified the published app archive's minisign
  signature, safe tar members, `Cortana.app` bundle, `Info.plist` version `0.29.66`, and
  `codesign --verify --deep --strict`. `spctl --assess` rejects the ad-hoc bundle (exit 3) because
  Developer ID signing/notarization is not configured. The v0.29.66 `latest.json` contains all
  required platform entries and passed the full updater-manifest and signature gate; the app was
  not launched.
- The full `cortana readiness` scan is a read-only operational check because it includes roughly
  1 GB of SQLite integrity and backup scanning; the then-installed v0.30.10 run completed successfully.
  That fresh query-only run passed database integrity, embedding/index generation, embedding
  provider, ACL, API liveness, backup freshness, extractive query mode, and confirmed that the
  recurring sync service is not installed.
- The v0.30.10 source tree's native Desktop suite passed all
  129 tests. The
  local developer bundle is intentionally unsigned (`bundle:mac --no-sign`); strict `codesign`
  verification fails as expected and no `TeamIdentifier` is present. Developer ID
  signing/notarization remains a release blocker. No unsigned historical developer bundle is
  treated as current recovery evidence; the verified v0.30.10 release assets and installed CLI are
  authoritative.
- The historical focused Desktop web gate passed 160 tests across 9 files, and the isolated full
  Bun suite passed 258 tests across 22 files (that run: 71.84 seconds, 1,272 assertions, including
  the desktop lockfile helper regression). The Python suite passed 160 tests, `bun run type-check`,
  `uv lock --check`, and the source formatting/lint gates passed. These are per-suite figures, not
  a deduplicated aggregate. The root `test` script now runs Bun with isolated, single-worker file
  execution so file-local API mocks cannot leak between OAuth suites or race the desktop pagination
  tests. The v0.30.10 source tree's native Desktop suite passed all 129 tests; the focused `native_`
  subset passed 24 tests (105 filtered). These counts were refreshed against that historical source
  tree without launching the Desktop app. The full
  run still emits non-fatal React act diagnostics from a few asynchronous shell tests; these are
  separate from pass/fail results and do not constitute packaged-GUI evidence. These are headless assertions and
  do not count as a successful production-GUI drill.
- The historical v0.30.10 Rust library suite passed 253 tests with
  no failures;
  this is a separate core-runtime count and is not added to the Desktop-native count above.
- The protected promotion workflow remains authoritative: feature PRs target `staging`, then a
  separate staging-to-main promotion produces the release on `main`. Desktop checks remain
  headless CI evidence; they do not claim packaged GUI, browser, OS-service, or signed-updater
  behavior. The older v0.29.8 readiness figures are historical and are not re-asserted.
- The remote branch policy now matches that staging-release flow: the active `code-foundry-main`
  and `code-foundry-staging` rulesets block deletion and non-fast-forward updates, require
  `Validation / Gate`, and require `Tauri 2 / Linux` for protected promotion; staging permits
  only squash feature merges. This is repository-policy evidence, not a packaged GUI or
  manual-drill result.
- Full `cortana readiness` is a read-only, comprehensive check that includes roughly 1 GB of
  SQLite integrity and backup scanning. In the observed run, the database integrity scan took
  about 130 seconds and the backup scan about 80 seconds. `GET /healthz` is only an
  unauthenticated process-liveness check and must not be treated as full readiness evidence. The
  previous comprehensive readiness run against the installed v0.29.54 configuration passed
  database integrity, embedding generation/provider, ACL, query API, and verified-backup freshness
  (24 hours within a 48-hour bound),
  query mode, and the safe query-only state with recurring sync not installed; it did not invoke
  source validation because `--allow-sync-service` was not supplied.
- A historical `cortana readiness --allow-sync-service` run failed closed without contacting any
  connector: every legacy/filesystem/code validation was a bounded sample or below the configured
  full-corpus budget, and `personal-calendar` had no successful validation. This is the expected
  safety result; recurring sync remains uninstalled until complete validation and the missing Google
  token are repaired.
- A historical v0.29.64 headless `scripts/source-smoke.sh` validation-only pass used one document,
  65,536 bytes, and a 30-second per-source cap: 11 of 12 enabled sources passed. `personal-calendar`
  failed closed as `authorization denied` because its Google refresh token is expired; no sync was
  requested and recurring sync remains uninstalled.
- The matching v0.29.64 `--sync --include-filesystem` trial passed the same bounded,
  `--no-reconcile --require-validation` operation for all 11 authorized sources. The calendar trial
  was skipped after its failed validation, and the command did not install or enable recurring sync.
- A bounded validation of `personal-calendar` on the functional tree carried into v0.30.0 (1
  document, 65,536 bytes, 30
  seconds) succeeded without writing documents, embeddings, or reconciliations and refreshed the
  owner-only Google token through its configured refresh path. Discord Desktop RPC tokens now
  refresh atomically from their owner-only refresh token before expiry; an expired token without a
  refresh token still fails closed and requests reauthorization. On 2026-08-11, Discord
  authorization succeeded for the Nifty League Team, Nifty League, and The Pink Binder sources;
  the current validation-only smoke used a one-document cap per enabled source without writes. The
  personal AMF source remains disabled, and recurring sync remains uninstalled until full
  complete validation coverage exists for every enabled source.
- The current Desktop readiness source now compares the installed connector version with the
  bundled Cortana sidecar and marks a stale or unreadable connector unavailable before source jobs
  start. The regression suite covers matching and mismatching release versions.
- A tracked-history `gitleaks detect --redact` scan covered 970 commits and found no secrets.
- Release v0.29.61 also carries the fail-closed recurring-sync freshness guard across every
  reconciling path: the all-source gate, single-source `sync --require-validation`, and
  `readiness --allow-sync-service` reject `validation_max_age_hours = 0`; targeted Rust tests cover
  each path. Query-only/manual checks continue to permit an unbounded age without installing sync.
- Historical installed-core model-backed evaluations ran against the configured provider without
  opening a personal index or starting sync/connectors. The source at `339240e` and packaged
  v0.29.31 passed historical runs,
  but the installed v0.29.33 evaluator failed closed twice after the planner call because the
  provider appended an uncited attribution line to the synthesis response (8,313 ms and 13,398 ms).
  The latest source run passed planner+synthesis citation validation in 22,866 ms after the bounded
  output cap was raised for gateway reasoning. The latest installed v0.29.60 core binary passed
  the current planner+synthesis citation validation with cache reuse and revision invalidation in
  17,928 ms; the prior cache-warm v0.29.60 run passed in 10,323 ms. The prior installed v0.30.6
  rerun passed in 19,954 ms with the same planner, synthesis, citation, cache, and revision
  checks. A fresh installed v0.30.10 rerun passed in 15,107 ms with planner and synthesis model
  use, valid citations, cache reuse, and revision invalidation; retrieval recall, MRR, case pass
  rate, and citation validity were all 1.0 within the 30,000 ms answer deadline. The latest
  2026-08-12 rerun also passed in 11,100 ms; the earlier 15,107 ms result is historical. Earlier
  provider-unavailable attempts remain historical fail-closed evidence, and extractive mode
  remains the safe production default because synthesis is still an explicit opt-in.
- That historical runtime snapshot remained safely closed for recurring sync: ingestion was `manual`, the
  sync service is not installed, and the configured inventory has 22 sources with 13 enabled.
  The enabled set is Apple Notes, Drive, Gmail, Calendar, and Buzz; Discord and all code/filesystem
  roots are disabled, and Slack is unconfigured. The enabled records are fresh bounded evidence,
  but eight Drive/Gmail/Calendar records were below their configured production budgets. These
  records do not authorize a full-corpus or recurring run without fresh validation at the configured
  limits.
- That historical query-only readiness probe passed database integrity, embedding/index/provider health,
  ACL, API liveness, fresh verified backup, extractive mode, and confirms sync is not installed.
  The matching historical `readiness --allow-sync-service` probe failed closed because every enabled
  connector was below its configured full-sync budget and filesystem/code records were bounded
  samples; no sync service was installed or started. Recurring mode must stay fail-closed until
  every enabled source covers its configured full-sync budget.
- The 2026-08-12 validation-only `scripts/source-smoke.sh` run is historical: it passed the 21
  sources enabled at that time at the bounded one-document/65,536-byte/30-second budget, including
  the then-authorized Discord sources, with no trial sync, embeddings, or reconciliation. It confirms
  only the authorization and connector reachability of that earlier inventory; it does not authorize
  the current source set or recurring sync.
- A subsequent one-document/65,536-byte non-reconciling trial showed the full connector-to-index
  path is sensitive to the per-source wall-clock budget: Personal Drive and Personal Gmail both
  completed after their validation and trial windows were raised to 180 seconds, while the same
  sources exceeded the tighter 30-second embedding window. The Discord pending-1 trial was
  cancelled before completion when its RPC channel walk outlasted the bounded operator probe.
  No trial reconciled or deleted indexed records; this evidence does not authorize recurring sync.
- A historical v0.30.10 packaged control-plane drill passed verified backup creation, disposable restore,
  SQLite verification, and cleanup. It also passed offline
  init, bounded fixture ingestion, search/context, metadata-only audit export, backup, restore, and
  post-restore search; neither drill touched indexed personal data.
- The current source-native headless acceptance suite passes without starting Tauri: the 130 native
  tests cover OAuth guards, tray/background lifecycle, updater guards, settings import/export,
  backup/restore, and source validation. They complement the 258 Bun tests and do not substitute for
  the still-unverified interactive packaged GUI flows.
- Packaged-app GUI/browser OAuth, tray/menu, native file-dialog import/export, and signed updater
  interactions remain unverified because no callable Computer Use session was available. Native
  handler tests, packaged CLI control-plane, and packaged backup/restore evidence are recorded
  separately above.
- Hindsight and Honcho remain disabled-by-default optional adapters; Cortana's canonical store
  remains the source of truth until provider ACL, deletion, export, and packaged-UI gates are
  explicitly proven.
