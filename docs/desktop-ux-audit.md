# Cortana Desktop UX audit

This audit maps the current Desktop implementation to the UI/UX objective. It is
intentionally separate from runtime migration work:
legacy scope quarantine remains in place, so changing or deleting indexed data
is not part of a visual/UI change.

## Requirement matrix

| Area                                                   | Current evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | Status            | Follow-up                                                                                                                                                      |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Theme system and readable foreground/background tokens | `apps/web/src/theme.ts`, `apps/web/src/styles/tokens.css`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Done              | Add visual contrast snapshots before adding more themes.                                                                                                       |
| Cortana icon and blue default theme                    | `Navigation.tsx` uses `/app-icon.svg`; blue theme is the default                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | Done              | Verify packaged assets on each signed desktop build.                                                                                                           |
| Consistent buttons and hover tooltips                  | Shared `--btn-*` size/padding tokens drive chrome (34px), compact action (30px), and text action (38px) buttons; workspace/source card controls use the same compact icon contract and the last sharp 4px corners now use `--radius-xs`; the settings banner actions match the secondary-button style                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Done              | Keep the shared contract as new action groups are added; visual snapshot pass before adding themes.                                                            |
| Fast tooltips                                          | `quick-tooltip` uses an 80ms opacity transition and now covers the remaining title-only icon buttons (status bar indicators via a `quick-tooltip--above` variant, source tree, favorites, graph nodes, context panel, workspace/principal remove, path choosers); rows inside scroll containers keep the native title fallback so the tip is never clipped away                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | Done              | Add a first-hover interaction check in the packaged GUI pass.                                                                                                  |
| Background service restart after saving                | `restartAfterSaveIfNeeded` restarts core services and clears the notice on success; failures now keep an alert banner that names the failure with `Retry restart` and `Open services` recovery actions; source-toggle saves from Knowledge also restart in the background and report named failures in the status bar                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Done              | Keep the failure-recovery path covered by regression tests.                                                                                                    |
| Human-readable changelog                               | `SafeMarkdown` renders headings, lists, inline code, and safe links                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | Done              | Add coverage for fenced/code-heavy release notes and malformed links.                                                                                          |
| Workspace display name, generated ID, logo, and color  | `WorkspaceSection`, `WorkspaceLogo`, and `workspaceLogoStore.ts`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | Done              | Keep the internal ID out of the default form; retain it only under Advanced.                                                                                   |
| Workspace account label semantics                      | Label is explicitly optional metadata; OAuth credentials remain source-owned and the field uses a neutral example                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Done              | Derive a provider identity only when authorization metadata is available and explicitly approved.                                                              |
| Workspace-scoped source settings                       | `SourcesSection` now provides workspace tabs, per-workspace source counts, and a Needs assignment quarantine view; add-source targets the selected workspace                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Done              | Keep the tab isolation and assignment warning covered by regression tests.                                                                                     |
| Source logos and compact OAuth/enable actions          | `SourceIcon` and provider-specific actions exist; advanced fields are disclosed and source-card operations use icon-only controls with accessible labels and fast tooltips                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Done              | Add provider-specific discovery actions as OAuth integrations land.                                                                                            |
| OAuth-first source setup                               | Google, GitHub, and Slack use fixed browser OAuth flows; Discord uses the signed-in Desktop RPC client. Every provider has bounded responses and owner-only token files                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Done              | Keep provider discovery/authorization covered by regression tests.                                                                                             |
| Model selectors                                        | Embedding and query settings have local/cloud catalog selects with a Custom option, plus an opt-in refresh that lists the models the configured provider advertises via `cortana provider-models`; local Qwen presets and a saved custom model are preserved whenever discovery is unavailable, and capabilities are echoed only when the provider explicitly advertises them (never inferred from names)                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Done              | Keep the staleness guard (catalog is discarded when the endpoint, mode, or key variable changes) and the custom-model fallback covered by regression tests.    |
| Plugins grouping                                       | Hindsight and Honcho are grouped under Plugins                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Done              | Keep both disabled by default and add live opt-in evaluation before enabling.                                                                                  |
| Settings ordering                                      | Services, Workspaces, Sources, and Readiness are the primary group above the divider                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Done              | Preserve this order as sections grow.                                                                                                                          |
| Knowledge workspace selector                           | Uses workspace logo + display name; no “All workspaces” option                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Done              | Add a compact tab/pill selector when more than one workspace exists.                                                                                           |
| Strict workspace segregation                           | UI requests are scoped to the active workspace; backend/config reject unknown configured source projects. Legacy rows are quarantined and legacy public-ACL rows are now zero (stale corpus remains quarantined).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Done (quarantine) | Keep owner-scoped checks enforced across retrieval/document/search/context/answer/MCP and preserve the quarantine label until an explicit mapping is approved. |
| GitHub repository selection                            | OAuth device flow and bounded repository chooser are implemented and included in release v0.27.2; auth-owner behavior is now released in v0.27.3.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | Done              | Keep end-to-end repository selection coverage and release-note alignment for future connector updates.                                                         |
| Discord server/community assignment                    | `cortana authorize-discord SOURCE`, `discord-servers`, and `discord-channels` use the signed-in Discord Desktop client's local RPC socket with bounded, read-only guild/channel discovery. The Desktop chooser persists checked guilds and channels into the per-source `servers`/`channels` fields, which are per-workspace because each Discord source belongs to exactly one workspace. No credential-scraping path is retained.                                                                                                                                                                                                                                                                                                                                                                                                                          | Done (this pass)  | Keep RPC authorization, bounded discovery, and snapshot-based message capture covered by regression tests.                                                     |
| Slack workspace assignment                             | `cortana authorize-slack SOURCE` (Authorization Code + PKCE against the fixed endpoints `https://slack.com/oauth/v2/authorize` and `https://slack.com/api/oauth.v2.access`, with the exact loopback redirect `http://127.0.0.1:47521/callback` registered in the Slack app) and `cortana slack-workspaces SOURCE` (bounded `team.info` result from the stored user token with one-shot refresh when token rotation is enabled) add browser OAuth workspace authorization; the Desktop workspace chooser persists the checked team ids into the per-source `teams` field with display names index-aligned in `team_names`, which is per-workspace because each Slack source belongs to exactly one workspace. `SLACK_BOT_TOKEN` stays the message-sync credential and is never interpreted as a path; token-only setups keep the original behavior unchanged. | Done (this pass)  | Keep OAuth workspace discovery covered by regression tests; channel selection and message sync intentionally stay bot-token based.                             |
| Buzz community assignment                              | `cortana buzz-communities SOURCE` lists the bounded communities recorded in the source's read-only `agents/teams.json` identity file (stable `id` + `name` records; the file must be a regular, non-symlink JSON array bounded at 512 KiB and missing, malformed, or duplicate entries fail closed). The Desktop community chooser persists the checked community ids into the per-source `communities` field with display names index-aligned in `community_names`, which is per-workspace because each Buzz source belongs to exactly one workspace. Discovery is read-only: it never runs ingestion or sync and never infers identity from persona event content; the read-only connector behavior is unchanged.                                                                                                                                          | Done (this pass)  | Keep identity-file validation and per-workspace community assignment covered by regression tests.                                                              |
| Workflow folder removal and tree indentation           | Knowledge uses the workspace-scoped source tree only; the document explorer heading is a `Workspace / Source` breadcrumb (with `Workspace / All sources` while browsing a workspace with no source selected) and virtualized document rows carry the `document-node` indentation class that the stylesheet nests under the source level; no workflow/folder labels exist anywhere in Knowledge display data, all asserted by regression tests in `SourcePanel.test.tsx`                                                                                                                                                                                                                                                                                                                                                                                      | Done              | Re-verify breadcrumb ellipsis and row indentation on the packaged GUI.                                                                                         |
| Service state from Sources                             | The Knowledge Sources sidebar (`role="switch"` per source) and Settings > Sources (enable checkbox) are the only source enable/disable surfaces; Settings > Services renders process health only (per-service Start/Stop/Restart, Start/Stop/Restart all, install, autostart, and the validation-gated recurring-sync schedule) with a regression test asserting Services never exposes a source enable control                                                                                                                                                                                                                                                                                                                                                                                                                                              | Done              | Keep the Sources-only enablement invariant covered as new connectors land.                                                                                     |
| Result views hidden before search                      | Answer/Evidence/Timeline tabs are result-gated; Graph is a rail action                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Done              | Keep this invariant in regression tests.                                                                                                                       |
| Full-screen Graph alternative                          | Graph is a dedicated rail action; while active the source and context panels collapse (`--source-width`/`--context-width` go to 0) so the graph spans the full workspace width, the duplicate top tab stays removed, and the title-bar source action leaves the graph so the panel is reachable again                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Done              | Keep the full-width layout and panel-restore invariants covered by regression tests.                                                                           |
| Source icon fidelity                                   | Provider mappings exist for code, Drive (brand mark), calendar, Gmail, Slack, Buzz, Discord, and Apple Notes (brand mark with a `StickyNote` fallback, never the code glyph); a regression test asserts the brand `path` for Notes and Drive and the lucide fallback for glyph-only connectors                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Done              | Keep brand mappings aligned with Simple Icons releases; verify licensing per artifact.                                                                         |

## Production blockers before calling the Desktop complete

1. Complete packaged GUI/browser OAuth, tray/menu, native file-dialog import/export, and
   signed-updater drills on a supported Developer ID/notarized Desktop build. The packaged CLI
   control-plane and backup/restore paths are now verified, and the native acceptance suite covers
   the command handlers; the GUI-only portions remain unverified because no callable Computer Use
   session is available here.
2. Prove the persistent configured query provider with a model-backed evaluation. **Closed for the
   current installed core:** the configured `auto-free` provider passed the fixture-only gate in
   13,779 ms with planner and synthesis model use, valid citations, cache reuse, and revision
   invalidation. This remains fixture-only evidence, not packaged-app proof. Keep extractive mode
   as the safe production default because model synthesis remains opt-in; the installed v0.30.3
   core also passed `doctor`, readiness, and the disposable packaged control-plane and recovery
   drills, while the GUI remains unlaunched.
3. Provider-advertised model metadata is implemented and bounded by
   `cortana provider-models`; keep the provider capability contract covered as
   supported query/answer providers evolve.
4. Discord Desktop RPC authorization/server discovery and per-workspace server/channel
   persistence landed in this pass. Slack workspace discovery and per-workspace
   team assignment landed alongside it (the `SLACK_BOT_TOKEN` path for channel
   selection and message sync is preserved and never interpreted as a path).
   Buzz community assignment landed in the following pass: `cortana buzz-communities SOURCE`
   reads the read-only `agents/teams.json` identity
   file with bounded, fail-closed validation, and the Desktop chooser persists
   per-workspace `communities`/`community_names`.
5. The memory-provider decision is recorded: keep Hindsight and Honcho as
   disabled-by-default optional adapters. Cortana's canonical store remains the
   source of truth; Hindsight is the replacement-capable sidecar and Honcho is
   an append-only experimental sink. Neither is enabled for personal data until
   provider ACL, deletion, export, and packaged-UI gates are explicitly proven. The fresh offline
   comparative fixture (`uv run cortana-memory-eval`) reports `material_gain=true`, with recall and
   MRR gains of `0.375`; that is useful evidence for a future opt-in review, not live-provider proof.
6. Complete source authorization and full validation coverage before recurring sync: the bounded
   `personal-calendar` validation now succeeds and refreshes its owner-only Google token, but the
   four enabled Discord sources still fail closed because the configured Desktop RPC OAuth client
   and token files are absent. Filesystem/code sources are either bounded samples
   (`complete=false`) or legacy records without an explicit completeness marker. Both states
   fail closed; recurring sync must remain uninstalled until Discord authorization is completed
   and every enabled source has a current `complete=true` validation at its configured budget.

## Evidence limits

### Stale/provider audit (2026-08-11)

- A tracked-source scan found no Spark model, provider, configuration, or dependency. The only
  remaining `Spark` matches are Lucide `Sparkles` icons used by the Query navigation surface.
- Rust `clippy --all-targets --all-features -- -D warnings`, Python Ruff/format/mypy, the web
  type-check, and ESLint all pass on this tree. No generated build/cache directory is tracked.
- Desktop Cargo helpers now restore the Release Please lockfile marker even when Cargo rewrites the
  lockfile; `scripts/desktop-lockfile.test.mjs` covers missing and already-present annotations.
- Remaining `legacy` references are active migration, ACL-quarantine, and embedding-generation
  safety paths. They are not dead Spark-era code; deleting them before existing configurations are
  migrated would orphan source scopes or weaken the fail-closed migration boundary.

- The current main line is v0.30.3 at tag commit `0c793c6` (`v0.30.3`), published from
  release-assets workflow `31481537396`. The workflow completed all five platform jobs, and the
  strict verifier passed all 18 required core, desktop, signature, checksum, and updater assets,
  including the repaired Windows installers. The installed CLI `/Users/amf/.local/bin/cortana`
  reports `cortana 0.30.3`; the packaged Desktop app was not launched. The current
  model/evaluation evidence remains fixture-only evidence, while GUI/browser OAuth, tray, native
  dialogs, and notarization remain manual gates.
- Historical v0.30.0 and v0.30.2 evidence remains useful for release investigations, but it must not
  be read as current-release proof; v0.30.3 is the verified cross-platform release.
- A static drill of the published `Cortana_0.29.64_aarch64.app.tar.gz` archive found the expected
  `Cortana.app` bundle, executable, and `Info.plist` version `0.29.64`; `codesign --verify --deep
--strict` passed. This proves archive integrity and local signature structure only: the app was
  not launched, notarization was not assessed, and tray, native dialogs, OAuth, and updater UI
  remain manual gates.
- Historical local developer-bundle checks at v0.29.69 regenerated the expected arm64 app and
  connector sidecars, but used the deliberate `bundle:mac --no-sign` path. They are retained only
  as historical evidence; strict signature validation is not claimed for that artifact and the
  v0.30.3 release asset is authoritative.
- A static check of the published v0.30.3 macOS ARM app archive reports
  `CFBundleShortVersionString=0.30.3` and passes `codesign --verify --deep --strict`. `spctl --assess`
  exits 3 because Developer ID signing/notarization is not configured; the app was not launched.
- A headless v0.29.66 macOS ARM packaged-app drill verified the published app archive's minisign
  signature, safe tar members, `Cortana.app` bundle, `Info.plist` version `0.29.66`, and
  `codesign --verify --deep --strict`. `spctl --assess` rejects the ad-hoc bundle (exit 3) because
  Developer ID signing/notarization is not configured. The v0.29.66 `latest.json` contains all
  required platform entries and passed the full updater-manifest and signature gate; the app was
  not launched.
  The full `cortana readiness` scan is a read-only operational check because it includes roughly
  1 GB of SQLite integrity and backup scanning; the latest installed v0.30.3 run completed successfully.
  That fresh query-only run passed database integrity, embedding/index generation, embedding
  provider, ACL, API liveness, backup freshness, extractive query mode, and confirmed that the
  recurring sync service is not installed.
  The current-source native Desktop suite on v0.30.3 passes all
  129 tests. The
  local developer bundle is intentionally unsigned (`bundle:mac --no-sign`); strict `codesign`
  verification fails as expected and no `TeamIdentifier` is present. Developer ID
  signing/notarization remains a release blocker; the previous v0.29.50 bundle is retained at
  `/Users/amf/.Trash/Cortana.app.backup-v0.29.50-20260810-2205`
  for recovery (older v0.29.38, v0.29.37, v0.29.33, v0.29.31, v0.29.29, v0.29.28, v0.29.27,
  v0.29.26, v0.29.24, v0.29.23, v0.29.22, v0.29.20, v0.29.19, and v0.29.14 backups remain
  available as well).
- The focused Desktop web gate passes 160 tests across 9 files, and the isolated full web suite
  passes 255 tests across 22 files (latest run: 60.34 seconds, 1,264 assertions). The Python suite
  passes 160 tests, `bun run type-check`, `uv lock --check`, and the current source formatting/lint
  gates pass. These are per-suite figures, not a deduplicated aggregate. The root `test` script now
  runs Bun with isolated, single-worker file execution so file-local API mocks cannot leak between
  OAuth suites or race the desktop pagination tests. The current-source native Desktop suite passes
  all 129 tests; the focused `native_` subset passes 24 tests (105 filtered). These counts were
  refreshed against the v0.30.3 functional tree without launching the Desktop app.
- The current Rust library suite on v0.30.3 passes 253 tests with
  no failures;
  this is a separate core-runtime count and is not added to the Desktop-native count above.
- The protected promotion workflow remains authoritative: feature PRs target `staging`, then a
  separate staging-to-main promotion produces the release on `main`. Desktop checks remain
  headless CI evidence; they do not claim packaged GUI, browser, OS-service, or signed-updater
  behavior. The older v0.29.8 readiness figures are historical and are not re-asserted.
- The remote branch policy now matches that staging-release flow: the active `code-foundry-main`
  and `code-foundry-staging` rulesets block deletion and non-fast-forward updates, require
  `Validation / Gate`, and require `Tauri 2 / Linux` for protected promotion; staging permits
  only squash feature merges. PR #600 is the sole staging input for this Discord change after
  the superseded direct-main duplicate was closed. This is repository-policy evidence, not a
  packaged GUI or manual-drill result.
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
  refresh token still fails closed and requests reauthorization. The local connector environment
  was updated from v0.29.68 to v0.29.69; all four enabled Discord sources reached the expected
  missing OAuth-client-file guard instead of the stale CLI parser. Discord authorization and
  recurring sync therefore remain uninstalled and disabled until the owner supplies the Desktop
  RPC client/token files and every enabled source has current complete validation coverage.
- The current Desktop readiness source now compares the installed connector version with the
  bundled Cortana sidecar and marks a stale or unreadable connector unavailable before source jobs
  start. The regression suite covers matching and mismatching release versions.
- A tracked-history `gitleaks detect --redact` scan covered 970 commits and found no secrets.
- Release v0.29.61 also carries the fail-closed recurring-sync freshness guard across every
  reconciling path: the all-source gate, single-source `sync --require-validation`, and
  `readiness --allow-sync-service` reject `validation_max_age_hours = 0`; targeted Rust tests cover
  each path. Query-only/manual checks continue to permit an unbounded age without installing sync.
- A current installed-core model-backed evaluation ran against the configured provider without
  opening a personal index or starting sync/connectors and passed in 13,779 ms with planner and
  synthesis model use, bounded planning, valid citations, cache reuse, and revision invalidation.
  The source at `339240e` and packaged v0.29.31 passed historical runs,
  but the installed v0.29.33 evaluator failed closed twice after the planner call because the
  provider appended an uncited attribution line to the synthesis response (8,313 ms and 13,398 ms).
  The latest source run passed planner+synthesis citation validation in 22,866 ms after the bounded
  output cap was raised for gateway reasoning. The latest installed v0.29.60 core binary passed
  the current planner+synthesis citation validation with cache reuse and revision invalidation in
  17,928 ms; the prior cache-warm v0.29.60 run passed in 10,323 ms. The fresh installed v0.30.3
  rerun passed in 13,779 ms with the same planner, synthesis, citation, cache, and revision checks.
  Extractive mode remains the safe production default because synthesis is still an explicit opt-in.
- The current runtime status remains safely closed for recurring sync: ingestion is `manual`, the
  sync service is not installed, and all four enabled Discord sources fail closed until their
  owner-only RPC OAuth client/token files exist. Filesystem and code sources remain bounded samples
  (`complete=false`) or legacy records with unknown completeness; neither state authorizes a
  full-corpus or recurring run without a fresh explicit `complete=true` validation.
- A current `readiness --allow-sync-service` probe correctly failed closed without installing or
  starting sync: connector validations were below configured budgets, filesystem/code validations
  were sampled, and every Discord validation was unsuccessful. Query-only readiness still passes.
- A current v0.30.3 packaged control-plane drill passed verified backup creation, disposable restore,
  SQLite verification, and cleanup. It also passed offline
  init, bounded fixture ingestion, search/context, metadata-only audit export, backup, restore, and
  post-restore search; neither drill touched indexed personal data.
- The current source-native headless acceptance suite passes without starting Tauri: the 129 native
  tests cover OAuth guards, tray/background lifecycle, updater guards, settings import/export,
  backup/restore, and source validation. They complement the 252 web tests and do not substitute for
  the still-unverified interactive packaged GUI flows.
- Packaged-app GUI/browser OAuth, tray/menu, native file-dialog import/export, and signed updater
  interactions remain unverified because no callable Computer Use session was available. Native
  handler tests, packaged CLI control-plane, and packaged backup/restore evidence are recorded
  separately above.
- Hindsight and Honcho remain disabled-by-default optional adapters; Cortana's canonical store
  remains the source of truth until provider ACL, deletion, export, and packaged-UI gates are
  explicitly proven.
