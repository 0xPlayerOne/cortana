# Source rollout plan

This is the operator plan for moving Cortana from a configured connector to a
production-safe source. It is intentionally separate from the code contract:
connector tests prove behavior, but they do not authorize an account or prove
that a user's full corpus is ready for reconciliation.

## Current operator state (2026-08-16)

The local installation remains in manual mode: `ai.cortana.sync` is not installed and no
recurring job is active. The operator has completed the Hermes import/rebuild and retained a
dated rollback backup; active legacy rows, launch agents, and parallel legacy stores are not part
of the live Cortana runtime. Migration compatibility code remains available for older installs.

The current safe rollout is intentionally selective. The operator has refreshed the validation
records for the enabled non-code sources at their configured production budgets where the provider
can complete within the bounded connector deadline, using the installed v0.32.9 parser:

- Apple Notes: `work-notes` 28, `personal-notes` 65, and `special-notes` 8 documents; all are
  `complete=true` at 2,000 documents/128 MiB/900 seconds.
- Google Calendar: `work-calendar` 2,207 events at 3,000/64 MiB/300 seconds;
  `personal-calendar` 1,836 events and `special-calendar` 0 events at 2,000/128 MiB/900 seconds;
  all are complete.
- Buzz: 45 records at 2,000/128 MiB/900 seconds; complete.
- Work Gmail: 7,386 messages at 10,000/64 MiB/600 seconds; complete.
- Personal Gmail: 427 messages at 2,000/128 MiB/900 seconds; complete.
- Special Gmail: 216 messages at 2,000/128 MiB/900 seconds; complete.
- Special Drive: 97 documents at 2,000/128 MiB/900 seconds; complete.

Personal Drive's explicit 2,000-document/128 MiB/1,800-second validation is currently running with
the bounded large-PDF parser; it makes no index or reconciliation changes. Work Drive's latest
2,000-document/128 MiB/900-second validation failed closed under the previous connector deadline
and will be retried after Personal Drive completes. These failures do not authorize reconciliation.
Discord and all code/filesystem
roots remain disabled by operator choice, and Slack is not configured. The recurring sync service
remains uninstalled until every enabled source has a fresh complete validation at its configured
budget.

| Source                                                                     | Current evidence                                                                                                                                                                                                                                                                                      | Next action                                                                                                                                                          |
| -------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Apple Notes (`work-notes`, `personal-notes`, `special-notes`)              | Complete folder-scoped validation: 28 `Nifty League` notes in `work`, 65 personal notes after excluding `Nifty League` and Pink Binder folders, and 8 `The Pink Binder` notes in `special`; all meet the 2,000/128 MiB/900-second budget.                                                             | Keep exact folder filters; run a small non-reconciling trial before any future reconciliation.                                                                       |
| Google Calendar (`work-calendar`, `personal-calendar`, `special-calendar`) | Complete production-budget validation returned 2,207, 1,836, and 0 events respectively.                                                                                                                                                                                                               | Keep trials non-reconciling until the remaining source gates close.                                                                                                  |
| Buzz                                                                       | Complete production-budget validation returned 45 records at 2,000/128 MiB/900 seconds.                                                                                                                                                                                                               | Keep the source non-reconciling until the remaining source gates close.                                                                                              |
| Google Drive/Gmail                                                         | Work Gmail is complete at 7,386/34,487,878 bytes; Personal Gmail is complete at 427/1,489,922 bytes; Special Gmail is complete at 216/1,004,868 bytes; Special Drive is complete at 97/290,353 bytes. Personal Drive is running at the 2,000/128 MiB/1,800-second budget; Work Drive remains failed pending retry. | Finish Personal Drive, then retry Work Drive one source at a time with `--no-reconcile`; do not treat capped prefixes or failed runs as production snapshots. |
| Discord                                                                    | Disabled by operator decision while the prior bot/RPC authorization is unavailable.                                                                                                                                                                                                                   | Keep disabled until a fresh owner authorization is completed.                                                                                                        |
| Slack                                                                      | Not configured in this installation.                                                                                                                                                                                                                                                                  | Remains an optional connector for other users.                                                                                                                       |
| Code/filesystem roots                                                      | Disabled by operator decision to defer the largest syncs.                                                                                                                                                                                                                                             | Keep disabled until a separate code-index rollout is approved.                                                                                                       |

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

## Per-source rollout matrix

| Source family   | Authorization boundary                                                                       | Implemented contract                                                                                                                       | Safe next gate                                                                                | Production gate                                                                       |
| --------------- | -------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| Google Drive    | Desktop OAuth client + owner-only refresh-token file                                         | Pagination, bounded exports/PDFs, cache reuse, retries, stale-cache fallback, deletion reconciliation, ACL propagation                     | Authorize one named source, validate with a small budget, then run one `--no-reconcile` trial | Full listing/detail/conversion validation at configured budget with `complete=true`   |
| Gmail           | Desktop OAuth client + owner-only refresh-token file                                         | Listing/detail cursors, bounded detail fetches, transient retries, cache reuse, permission-failure guard, reconciliation, ACL propagation  | Same one-source validation and bounded trial                                                  | Complete message snapshot at configured budget; no unresolved detail or cursor errors |
| Google Calendar | Desktop OAuth client + owner-only refresh-token file                                         | Calendar pagination, recurring-series compaction, bounded events, cursor and listing validation, reconciliation, ACL propagation           | Validate one calendar source before any trial                                                 | Complete event snapshot at configured budget                                          |
| Apple Notes     | macOS Automation permission; no stored credential                                            | Executable/path checks, exact include/exclude folder filters, bounded exports, stable IDs, ACL propagation                                 | Approve host Notes access, validate each folder-scoped source, inspect status                 | Complete Notes validation at the intended full budget for every enabled folder scope  |
| Buzz            | Read-only local `agents/teams.json` identity plus retention data                             | Regular-file/symlink/size guards, bounded community discovery, read-only connector, progress and failure reporting                         | Confirm the identity file and select communities in Desktop                                   | Complete Buzz snapshot at configured budget                                           |
| Discord         | Signed-in Discord Desktop RPC + owner-only OAuth/token files                                 | Bounded guild/channel discovery, workspace assignment, snapshot cache, edit/delete refresh, cancellation, fail-closed RPC deadlines        | Keep Discord Desktop running; authorize one source and validate selected channels             | Complete channel snapshot at configured budget; RPC authorization must remain current |
| Slack           | Browser OAuth user token for workspace assignment plus configured bot token for message sync | PKCE flow, team discovery, bounded channel/message retrieval, cursor/cache refresh, retries, cancellation, reconciliation, ACL propagation | Configure the Slack app callback, authorize one workspace, validate one source                | Complete channel snapshot at configured budget with bot-token access                  |
| Filesystem/code | Owner-selected local roots; no OAuth                                                         | Metadata preflight, stable IDs, bounded reads, sampling, cancellation, ACL propagation, deletion reconciliation for complete snapshots     | Use `--sample` and a small non-reconciling trial                                              | Full-root validation without sampling, then complete reconciling snapshot             |

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
