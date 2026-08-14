# Source rollout plan

This is the operator plan for moving Cortana from a configured connector to a
production-safe source. It is intentionally separate from the code contract:
connector tests prove behavior, but they do not authorize an account or prove
that a user's full corpus is ready for reconciliation.

## Current safety state

The local installation remains query-only. Do not install `ai.cortana.sync`, run
a full sync, or delete retained Hermes data from this plan. A source is eligible
for a recurring or reconciling run only when its current record in
`source-validations.json` has `status = "succeeded"`, `complete = true`, the
current configuration fingerprint, and document/byte/time limits at least as
large as the requested run. Sampled (`complete = false`) and legacy-unknown
records are valid evidence for bounded non-reconciling trials only.

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

This plan does not authorize any source, sync, scheduler, or data deletion. It
exists to make the remaining user-controlled production gates unambiguous.
