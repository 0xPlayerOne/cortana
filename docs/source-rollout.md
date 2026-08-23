# Source rollout plan

This is the operator plan for moving Cortana from a configured connector to a
production-safe source. It is intentionally separate from the code contract:
connector tests prove behavior, but they do not authorize an account or prove
that a user's full corpus is ready for reconciliation.

## Current operator state (2026-08-23; v0.34.34 published and installed runtime)

The local installation remains in manual mode: `ai.cortana.sync` is not installed and no
recurring job is active. The operator has completed the Hermes import/rebuild and retained a
dated rollback backup; active legacy rows, launch agents, and parallel legacy stores are not part
of the live Cortana runtime. Migration compatibility code remains available for older installs.

The current safe rollout is intentionally selective. The earlier v0.34.13 run provided the
initial bounded validation pass for every enabled non-code source using the safe defaults
of 25 documents, 5 MiB, and 60 seconds, followed by a v0.34.15 Personal Gmail retry at the same
document/byte bound with a 120-second cap. Validation does not embed, index, reconcile, or delete
records. Ten sources now return `complete=true`; the three special-workspace Google sources failed
closed because the shared `special.json` OAuth grant returned `invalid_grant`. These records are
bounded evidence, not authorization to reconcile or install recurring sync:

- Apple Notes: `work-notes` 25, `personal-notes` 25, and `special-notes` 8 documents; complete.
- Work Google sources: `work-drive`, `work-gmail`, and `work-calendar`; 25-document bounded
  snapshots; complete.
- Personal Google sources: `personal-drive`, `personal-gmail`, and `personal-calendar` returned
  25-document bounded snapshots and complete=true; Personal Gmail required the 120-second retry cap.
  This confirms the Personal Drive OAuth repair only for a bounded probe, not its 2,000-document
  production budget.
- Buzz: 25 records; complete.
- Special Google sources: `special-drive`, `special-gmail`, and `special-calendar`; failed
  closed with `Google authorization expired or was denied; reauthorize this source`.

After the v0.34.34 installation, the operator retained the three Apple Notes folder-scoped
sources with the current binary. Read-only validation passed for `work-notes`, `personal-notes`, and
`special-notes` at the 25-document/5 MiB/60-second bound. The separate bounded, non-reconciling
trial measurements immediately before the metadata-only release were: `work-notes` 25 documents
(118,540 bytes), `personal-notes` 25 documents (89,645 bytes), and `special-notes` 8 documents
(14,046 bytes). The trials preserved the `work`/`personal`/`special` assignments and made no
deletions. This is fresh current-release connector evidence below the production-budget snapshot;
recurring sync remains uninstalled.

The same bounded smoke budget was then applied to `personal-drive`. Validation passed with 25
documents (234,160 bytes), but the separate non-reconciling trial reached the 60-second safety
bound and failed closed as `budget_exceeded`. It did not authorize reconciliation or recurring
sync, and no deletions were recorded. Personal Drive therefore remains below its configured
2,000-document/128 MiB/900-second production gate until a full-budget run can be completed under
an external watchdog.

On 2026-08-23, the installed v0.34.34 binary retained the bounded validation sweep for all
13 enabled non-code profiles. Ten profiles returned `complete=true` within 25 documents, 5 MiB,
and 60 seconds: all three Apple Notes scopes, all three work Google sources, all three personal
Google sources, and Buzz. The three special Google profiles failed closed with `invalid_grant`.
The matching non-reconciling trials passed for Apple Notes, work/personal Gmail and Calendar, and
Buzz, with zero deletions. Work Drive and Personal Drive both failed closed at the 60-second
budget; the special Google trials were skipped after validation failed. Recurring sync remains
uninstalled, and these bounded records do not authorize reconciliation or a production-budget run.

The published v0.34.34 release boundary retains the bounded four-worker Drive body-fetch pool and regression
coverage from PR #1594. The earlier production-budget Personal Drive run was stopped after 147
documents while a large PDF stalled; it made zero index or reconciliation writes. No current
production-budget validation is claimed for Personal Drive or any other source after this bounded
refresh. Discord and all code/filesystem roots remain disabled by operator choice, Slack is not
configured, and the recurring sync service remains uninstalled until every enabled source has a
fresh complete validation at its configured budget and the special-workspace OAuth grant is repaired.

| Source                                                                     | Current evidence                                                                                                                                                                                                                                                                                                            | Next action                                                                                                                                                                                                             |
| -------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Apple Notes (`work-notes`, `personal-notes`, `special-notes`)              | Current v0.34.34 source policy carries forward the v0.34.29 bounded validation and non-reconciling trial evidence for all three folder-scoped sources: 25 `Nifty League` notes in `work` (118,540 bytes), 25 personal notes (89,645 bytes), and 8 `The Pink Binder` notes in `special` (14,046 bytes), with zero deletions. | Keep exact folder filters; repeat validation at the intended production budget before any reconciliation or recurring sync.                                                                                             |
| Google Calendar (`work-calendar`, `personal-calendar`, `special-calendar`) | Current bounded validation returned 25, 25, and an authorization failure respectively. The successful work/personal records are complete only within the 25-document/5 MiB/60-second bound.                                                                                                                                 | Reauthorize the special Google account, then repeat bounded validation and a non-reconciling trial. Keep all runs non-reconciling until production-budget gates close.                                                  |
| Buzz                                                                       | Current bounded validation returned 25 records at 25 documents/5 MiB/60 seconds; complete within that bound.                                                                                                                                                                                                                | Keep the source non-reconciling until the remaining source gates close.                                                                                                                                                 |
| Google Drive/Gmail                                                         | Work Drive/Gmail passed the current bounded validation; current Personal Drive validation passed but its 60-second non-reconciling trial hit `budget_exceeded`; Personal Gmail passed bounded retries, while the three Special Google sources failed closed with `invalid_grant`.                                           | Reauthorize the `special` Google token, then repeat Personal Drive's 2,000-document/128 MiB/900-second validation under an external watchdog. Do not treat bounded probes or historical counts as production snapshots. |
| Discord                                                                    | Disabled by operator decision while the prior bot/RPC authorization is unavailable.                                                                                                                                                                                                                                         | Keep disabled until a fresh owner authorization is completed.                                                                                                                                                           |
| Slack                                                                      | Not configured in this installation.                                                                                                                                                                                                                                                                                        | Remains an optional connector for other users.                                                                                                                                                                          |
| Code/filesystem roots                                                      | Disabled by operator decision to defer the largest syncs.                                                                                                                                                                                                                                                                   | Keep disabled until a separate code-index rollout is approved.                                                                                                                                                          |

A source is eligible for a recurring or reconciling run only when its current record in
`source-validations.json` has `status = "succeeded"`, `complete = true`, the current configuration
fingerprint, and document/byte/time limits at least as large as the requested run. Sampled
(`complete = false`) and legacy-unknown records are valid evidence for bounded non-reconciling
trials only.

The earlier 2026-08-15 Work Drive retry emitted all 478 connector records but failed closed when the local
embedding connection dropped during ingestion. It used `--no-reconcile`, made no deletions, and the
embedding supervisor recovered; query-only readiness passed afterward. A subsequent v0.32.4 run with
`--max-documents 100 --max-bytes 16777216 --max-seconds 300 --no-reconcile --require-validation`
completed 100 unchanged documents with `changed=0` and `deleted=0`. This is a successful bounded
trial and exercises the transport-retry path, but it is not a complete 478-document production trial
and does not authorize reconciliation or recurring sync.

On 2026-08-15 the operator completed a fresh bounded, non-reconciling pass for every enabled
non-code source using 25 documents, 5 MiB, and 60 seconds per source. Work/Personal/Special
Apple Notes, Drive, Gmail, Calendar, and Buzz all succeeded; Special Calendar returned zero
records; every run reported zero deletions. The index ended at 12,123 documents and 42,638
chunks, query-only readiness passed, and the installation remains manual/query-only. These
records are below production budgets, so code roots, Discord, Slack, reconciliation, and
recurring sync remain explicitly gated.

Historical note: on 2026-08-16 a separate read-only Personal Drive probe completed 100 documents
at 390,182 bytes with a 300-second cap and zero index writes. This remains bounded connector
evidence, not a production validation; the current configured gate is 2,000 documents/128 MiB/
900 seconds and still does not authorize reconciliation or recurring sync.

## Per-source rollout matrix

| Source family   | Authorization boundary                                                                       | Implemented contract                                                                                                                                                     | Safe next gate                                                                                | Production gate                                                                       |
| --------------- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| Google Drive    | Desktop OAuth client + owner-only refresh-token file                                         | Pagination, provider changes cursor for unfiltered snapshots, bounded exports/PDFs, cache reuse, retries, stale-cache fallback, deletion reconciliation, ACL propagation | Authorize one named source, validate with a small budget, then run one `--no-reconcile` trial | Full listing/detail/conversion validation at configured budget with `complete=true`   |
| Gmail           | Desktop OAuth client + owner-only refresh-token file                                         | Provider history cursor for unfiltered snapshots, bounded detail fetches, transient retries, cache reuse, permission-failure guard, reconciliation, ACL propagation      | Same one-source validation and bounded trial                                                  | Complete message snapshot at configured budget; no unresolved detail or cursor errors |
| Google Calendar | Desktop OAuth client + owner-only refresh-token file                                         | Calendar pagination, recurring-series compaction, bounded events, cursor and listing validation, reconciliation, ACL propagation                                         | Validate one calendar source before any trial                                                 | Complete event snapshot at configured budget                                          |
| Apple Notes     | macOS Automation permission; no stored credential                                            | Executable/path checks, exact include/exclude folder filters, bounded exports, stable IDs, ACL propagation                                                               | Approve host Notes access, validate each folder-scoped source, inspect status                 | Complete Notes validation at the intended full budget for every enabled folder scope  |
| Buzz            | Read-only local `agents/teams.json` identity plus retention data                             | Regular-file/symlink/size guards, bounded community discovery, read-only connector, progress and failure reporting                                                       | Confirm the identity file and select communities in Desktop                                   | Complete Buzz snapshot at configured budget                                           |
| Discord         | Signed-in Discord Desktop RPC + owner-only OAuth/token files                                 | Bounded guild/channel discovery, workspace assignment, snapshot cache, edit/delete refresh, cancellation, fail-closed RPC deadlines                                      | Keep Discord Desktop running; authorize one source and validate selected channels             | Complete channel snapshot at configured budget; RPC authorization must remain current |
| Slack           | Browser OAuth user token for workspace assignment plus configured bot token for message sync | PKCE flow, team discovery, bounded channel/message retrieval, cursor/cache refresh, retries, cancellation, reconciliation, ACL propagation                               | Configure the Slack app callback, authorize one workspace, validate one source                | Complete channel snapshot at configured budget with bot-token access                  |
| Filesystem/code | Owner-selected local roots; no OAuth                                                         | Metadata preflight, stable IDs, bounded reads, sampling, cancellation, ACL propagation, deletion reconciliation for complete snapshots                                   | Use `--sample` and a small non-reconciling trial                                              | Full-root validation without sampling, then complete reconciling snapshot             |

## Repeatable operator sequence

For each source, perform these steps in order and record the result in the
Desktop audit panel or the metadata-only audit export:

1. Confirm the source's workspace, ACL label, root/account, and intended budget.
2. Complete only that source's authorization or host-permission step.
3. Run `cortana validate-source SOURCE` with explicit small limits.
4. Inspect `/v1/status` or Desktop source status; do not proceed on a stale,
   failed, sampled, unknown, or mismatched record.
5. Run one bounded `cortana sync --source SOURCE --require-validation
--no-reconcile` trial and review cited results.
6. Increase limits only after the trial's cursor, cache, ACL, progress,
   cancellation, and failure behavior are understood.
7. Run a complete validation at the intended production budget. Only an explicit
   `complete=true` record permits reconciliation or recurring sync.

The source smoke script is useful for a metadata-only sweep, but it does not
replace per-source authorization or production-budget validation:

```bash
scripts/source-smoke.sh --config "$HOME/.config/cortana/config.toml"
```

Never use `--allow-sync-service` as a shortcut. It acknowledges an intentionally
installed recurring service; it does not authorize a source and must fail closed
until every enabled source meets the complete-validation gate.

## Evidence to retain

Keep the following with each rollout review:

- source name, workspace, kind, and configuration fingerprint;
- authorization method and timestamp, without tokens or private paths;
- validation status, completeness, limits, document/byte counts, and age;
- trial outcome, cursor/cache behavior, ACL scope, cancellation and failure result;
- cited query spot-check and metadata-only audit export;
- explicit approval before enabling a complete reconciliation or recurring job.

This plan does not silently authorize a new source, scheduler, reconciliation, or data deletion.
The Apple Notes rollout above is the completed operator-approved exception; all other production
gates remain explicit and source-scoped so an interrupted run cannot turn into an unbounded sync.
