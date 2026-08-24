# Source rollout plan

This is the operator plan for moving Cortana from a configured connector to a
production-safe source. It is intentionally separate from the code contract:
connector tests prove behavior, but they do not authorize an account or prove
that a user's full corpus is ready for reconciliation.

## Current operator state (2026-08-24; v0.34.42 published and installed runtime)

The local installation remains in manual mode: `ai.cortana.sync` is not installed and no
recurring job is active. The operator has completed the Hermes import/rebuild and retained a
dated rollback backup; active legacy rows, launch agents, and parallel legacy stores are not part
of the live Cortana runtime. Migration compatibility code remains available for older installs.

The current safe rollout is intentionally selective. The earlier v0.34.13 run provided the
initial bounded validation pass for every enabled non-code source using the safe defaults
of 25 documents, 5 MiB, and 60 seconds, followed by a v0.34.15 Personal Gmail retry at the same
document/byte bound with a 120-second cap. Validation does not embed, index, reconcile, or delete
records. Ten sources now return `complete=true`; the three special-workspace Google sources failed
closed because the shared `special.json` OAuth grant returned `invalid_grant`. The bounded records
are evidence, not authorization to reconcile or install recurring sync; the current production-budget
records below are still not sync authorization:

- Apple Notes: `work-notes` 28 / 122,114 bytes, `personal-notes` 66 / 136,208, and `special-notes`
  8 / 14,046; all are `complete=true` within the configured 2,000-document / 128 MiB / 900-second
  budget, with zero writes.
- Work Google sources now have current production-budget validation: `work-drive` 516 documents /
  4,581,462 bytes at 2,000 / 128 MiB / 900 seconds, `work-gmail` 7,388 / 34,530,230 at 10,000 /
  64 MiB / 600 seconds, and `work-calendar` 2,220 / 1,832,878 at 3,000 / 64 MiB / 300 seconds.
  Each is `complete=true` with zero writes.
- Personal Google sources: `personal-drive` 1,639 / 13,440,509 bytes, `personal-gmail` 431 /
  1,493,536, and `personal-calendar` 1,815 / 360,659; all are `complete=true` within the configured
  2,000-document / 128 MiB / 900-second budget, with zero writes.
- Buzz: 45 records / 375,824 bytes; `complete=true` within the configured 2,000-document / 128 MiB /
  900-second budget, with zero writes.
- Special Google sources: `special-drive`, `special-gmail`, and `special-calendar`; failed
  closed with `Google authorization expired or was denied; reauthorize this source`.

After the v0.34.42 installation, the operator retained the three Apple Notes folder-scoped
sources with the current binary. Read-only validation passed for `work-notes`, `personal-notes`, and
`special-notes` at the 25-document/5 MiB/60-second bound. The separate bounded, non-reconciling
trial measurements immediately before the metadata-only release were: `work-notes` 25 documents
(118,540 bytes), `personal-notes` 25 documents (89,645 bytes), and `special-notes` 8 documents
(14,046 bytes). The trials preserved the `work`/`personal`/`special` assignments and made no
deletions. This is fresh current-release connector evidence below the production-budget snapshot;
recurring sync remains uninstalled.

The installed v0.34.42 binary first refreshed all 13 enabled non-code profiles at the same
read-only 25-document/5 MiB/60-second bound. Ten profiles returned `complete=true`: the three
Apple Notes scopes, all three Work Google scopes, all three Personal Google scopes, and Buzz. The
fresh snapshots were `work-drive` 25/403,588 bytes, `work-gmail` 25/192,005 bytes,
`work-calendar` 25/22,982 bytes, `personal-drive` 25/388,980 bytes, `personal-gmail` 25/67,050
bytes, `personal-calendar` 25/5,008 bytes, and Buzz 25/141,590 bytes. The three Special Google
profiles failed closed with `invalid_grant` and explicitly require reauthorization. Every
validation reported zero document, embedding, and reconciliation writes; the records remain
bounded evidence and do not authorize a production-budget sync or recurring schedule.

The same bounded smoke budget was then applied to `personal-drive`. Validation passed with 25
documents (234,160 bytes), but the separate non-reconciling trial reached the 60-second safety
bound and failed closed as `budget_exceeded`. It did not authorize reconciliation or recurring
sync, and no deletions were recorded. A later current-release read-only production-budget
validation completed 1,639 documents and 13,440,509 bytes, recorded `complete=true`, and made
zero document, embedding, or reconciliation writes. This closes the Personal Drive validation
gate, but does not authorize a trial, reconciliation, or recurring sync by itself.

On 2026-08-24, the installed v0.34.42 binary retained the bounded validation sweep for all
13 enabled non-code profiles. Ten profiles returned `complete=true` within 25 documents, 5 MiB,
and 60 seconds: all three Apple Notes scopes, all three work Google sources, all three personal
Google sources, and Buzz. The three special Google profiles failed closed with `invalid_grant`.
The matching non-reconciling trials passed for Apple Notes, work/personal Gmail and Calendar, and
Buzz, with zero deletions. The earlier Work Drive 60-second trial failed closed, but later
production-budget validations completed successfully for every non-special enabled source;
the special Google trials were skipped after validation failed. Recurring sync remains uninstalled
because the special OAuth grant is expired and source authorization/recurring enablement remain
explicit operator decisions.

The published v0.34.42 release boundary retains the bounded four-worker Drive body-fetch pool and regression
coverage from PR #1594. Current production-budget validations include Personal Drive (1,639 documents /
13,440,509 bytes), Work Drive (516 / 4,581,462), Work Gmail (7,388 / 34,530,230), Work Calendar
(2,220 / 1,832,878), Personal Calendar (1,815 / 360,659), Apple Notes (`work-notes` 28 / 122,114;
`personal-notes` 66 / 136,208; `special-notes` 8 / 14,046), and Buzz (45 / 375,824), all with zero
index or reconciliation writes. Every non-special enabled source now has a current complete
production-budget record; the special-workspace OAuth grant is expired. Discord and all code/filesystem roots remain disabled by
operator choice, Slack is not configured, and the recurring sync service remains uninstalled until every
enabled source has a fresh complete validation at its configured budget.

| Source                                                                     | Current evidence                                                                                                                                                                                                                                                                           | Next action                                                                                                                                                                           |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Apple Notes (`work-notes`, `personal-notes`, `special-notes`)              | Current v0.34.42 production-budget validation returned `work-notes` 28 / 122,114 bytes, `personal-notes` 66 / 136,208, and `special-notes` 8 / 14,046, all complete with zero writes; exact folder routing is preserved.                                                                   | Keep exact folder filters; retain non-reconciling mode until the operator explicitly approves a trial and recurring policy.                                                           |
| Google Calendar (`work-calendar`, `personal-calendar`, `special-calendar`) | `work-calendar` passed 2,220 records / 1,832,878 bytes within 3,000 / 64 MiB / 300 seconds; `personal-calendar` passed 1,815 / 360,659 within 2,000 / 128 MiB / 900 seconds; `special-calendar` fails closed with `invalid_grant`.                                                         | Reauthorize the special Google account, then validate the remaining special scope and run a non-reconciling trial. Keep all runs non-reconciling until production-budget gates close. |
| Buzz                                                                       | Current v0.34.42 production-budget validation returned 45 records / 375,824 bytes, complete within 2,000 / 128 MiB / 900 seconds, with zero writes.                                                                                                                                        | Keep the source non-reconciling until the operator explicitly approves a trial and recurring policy.                                                                                  |
| Google Drive/Gmail                                                         | `work-drive` passed 516 / 4,581,462; `work-gmail` 7,388 / 34,530,230; Personal Drive 1,639 / 13,440,509; Personal Gmail 431 / 1,493,536. Each is complete within its configured production budget and made zero writes; the three Special Google sources fail closed with `invalid_grant`. | Reauthorize the `special` Google token. Keep all runs non-reconciling until authorization and recurring policy are explicitly approved.                                               |
| Discord                                                                    | Disabled by operator decision while the prior bot/RPC authorization is unavailable.                                                                                                                                                                                                        | Keep disabled until a fresh owner authorization is completed.                                                                                                                         |
| Slack                                                                      | Not configured in this installation.                                                                                                                                                                                                                                                       | Remains an optional connector for other users.                                                                                                                                        |
| Code/filesystem roots                                                      | Disabled by operator decision to defer the largest syncs.                                                                                                                                                                                                                                  | Keep disabled until a separate code-index rollout is approved.                                                                                                                        |

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
