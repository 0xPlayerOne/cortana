# Ingestion

Cortana treats every connector as a snapshot producer. Each successful run emits normalized
`Document` JSON Lines with stable source IDs. Cortana embeds only records whose searchable payload
changed, atomically replaces their chunks, and reconciles records that disappeared from the
completed snapshot. A failed connector never triggers deletion reconciliation.

Connector output is first captured in an owner-only on-disk spool and then ingested in bounded
batches. This preserves complete-snapshot reconciliation without holding a large Drive, Gmail, or
chat export in memory. Temporary spools are removed after success or failure, and reconciliation
uses a temporary SQLite key table rather than an unbounded SQL parameter list.
If one configured source fails, Cortana records the failure, continues syncing the remaining
sources, and exits nonzero after the run so supervisors still detect the partial failure.
Connector subprocesses also have a configurable wall-clock timeout (six hours by default), which
prevents a wedged upstream API from holding the cross-process sync lock indefinitely.
Direct `cortana ingest` and `validate-source` commands take that same `sync.lock` before doing
index or connector work. This prevents a direct JSONL import or validation spool cleanup from
racing a configured sync, backup, restore, or another bounded ingestion job.
Direct JSONL input uses the normal safety ceiling of 2,000 documents, 128 MiB of document
content, 15 minutes, and an 8 MiB maximum line. Treat it as a bounded reviewed import; split a
larger migration into separate batches rather than bypassing the limits.

Every source also has a fail-closed safety preflight. The default ceiling is 2,000 documents,
128 MiB of searchable content, and 15 minutes per source. Filesystem preflight walks metadata
without reading file contents; connector preflight validates the completed owner-only spool before
the first embedding or index write. A source that exceeds any ceiling fails without reconciliation.
Set global defaults in `[ingestion]`, tighter source-specific `max_documents`, `max_bytes`, and
`max_duration_seconds` values on `[[sources]]`, or one-run overrides on the command line.
Ingestion uses one embedding request at a time by default even when interactive queries allow more
concurrency.

Bounded syncs (for example Desktop trial or initial syncs, which never reconcile) pass the run's
document cap upstream to built-in connectors so a Drive listing stops at the permitted scope
instead of emitting a whole page that trips the live output safety bound. Validation applies the
same cap with `--no-cache` so a read-only probe never mutates a persistent cache with a partial
snapshot. Slack and Discord also use their uncached bounded path for trial or initial runs, even
when a cache directory is configured; this prevents a partial page from advancing an incremental
cursor past records that were never enumerated. Complete runs retain their caches, and capped
Google runs never prune cached bodies or messages they did not list. Reconciliation runs never pass
the cap: they always receive the full snapshot and fail closed when it exceeds a ceiling, so a
truncated snapshot can never trigger deletion reconciliation. Arbitrary external commands keep the
plain JSONL contract and never receive connector flags; their output is enforced by the live spool
bound and the spool preflight.

Configured source names are index namespaces. This prevents two Gmail accounts, Drive accounts, or
Slack workspaces from deleting or colliding with one another. The original adapter kind is retained
in metadata for provenance. Names are unique, lower-case, and limited to 64 letters, numbers,
dashes, or underscores; this also keeps per-source cache paths inside Cortana's data directory.

## Configure and run

Start from [`config.example.toml`](../config.example.toml), then run:

```bash
cargo run -- sync
cargo run -- sync --source personal-drive
cargo run -- sync --source work-code --no-reconcile
cargo run -- sync --source work-code --plan
cargo run -- sync --source work-code --max-documents 250 --max-bytes 33554432
```

`--no-reconcile` is useful for an intentionally partial external snapshot. Regular complete
snapshots should reconcile so removed source records do not remain searchable.
`--plan` never starts an external connector or opens the index. For filesystem sources it reports
the metadata-only document and byte scope; for remote connectors it reports the configured budgets
and marks inspection as deferred. An explicitly named disabled source can be planned safely before
it is enabled.

`SIGINT` and `SIGTERM` cancel an active source before reconciliation. In-flight connector
subprocesses are terminated, and embedding work is interrupted at a bounded polling interval.
Each completed document is committed as soon as its embedding vectors arrive, so a cancelled
or budget-exceeded run keeps the completed prefix and retries only the unfinished tail on the
next run. A partial snapshot never deletes records from the prior complete snapshot.

The Python adapter process writes only normalized JSON Lines to stdout. Counts and diagnostics go
to stderr, which makes the boundary safe to pipe into `cortana ingest` or supervise independently.
Google Calendar preserves one-off events individually and compacts expanded occurrences into one
stable document per recurring series, including its occurrence count, date range, participants,
and provenance. This prevents daily meetings from consuming thousands of redundant embeddings
without discarding their long-term history.

Google Drive and Gmail keep owner-only derived caches under
`data_dir/connector-cache/<source>/`. Complete runs still list every item ID for correct deletion
reconciliation, but Drive content is downloaded only when its modification timestamp changes and
immutable Gmail message bodies are downloaded only once. The caches are disposable and can always
be rebuilt from Google. First-time Drive content and Gmail detail retrieval use bounded one-worker
and four-worker pools respectively; cache writes and emitted documents remain ordered on the main
connector thread. Drive downloads are processed in 32-file batches, text and export responses are
consumed as bounded head/tail streams, and PDF responses are spooled to a temporary file with a 64 MiB cap
before parsing. PDFs with more than 32 pages use a bounded head/tail sample of at most 32 pages instead of
walking the entire page tree; ordinary PDFs also stop parsing once the 256,000-character extraction bound is
reached. Both paths set `content_truncated=true` and `content_original_chars=null` because the omitted
content was intentionally not counted. Smaller PDFs preserve the exact character count. The cache
preserves this metadata, so a later cache hit does not hide that a provider response was sampled. A
valid PDF with no extractable text is retained with an explicit metadata-only placeholder and
`content_unavailable=true`, so one scanned or malformed PDF cannot abort an otherwise complete listing;
the original Drive link remains the recovery path. Other binary or otherwise unsupported Drive items
are retained with the same explicit metadata-only marker in complete runs; Cortana does not claim to
have extracted their contents. Drive installs pypdf's AES support.
Full, uncapped Gmail runs also persist a private `sync_state` cursor in
`data_dir/connector-cache/<source>/gmail.sqlite3`. For an unfiltered mailbox, later runs use
Gmail history deltas for additions, deletions, and label changes instead of relisting every message.
The cursor is bound to the OAuth account identity and workspace; changing either clears the cached
bodies. Expired cursors rebuild the full snapshot, while bounded validation and filtered runs never
advance the cursor. Delta mutations and cursor advancement commit together, so a failed history
page or message fetch cannot bless a partial snapshot.
Full, uncapped Google Drive runs use the same durable cache contract in
`data_dir/connector-cache/<source>/drive.sqlite3`: the first complete unfiltered listing captures a
Drive `startPageToken`, and later runs request only provider changes, including file additions,
updates, trash/removal events, and shared-drive items. Change application and the replacement
cursor commit atomically before Cortana emits the resulting complete snapshot. A 400/404/410
expired cursor or an account/workspace/query fingerprint change clears only the derived Drive
cache and performs a fresh full listing. Bounded, filtered, or failed runs never advance the
cursor; if Drive does not return a usable start token, Cortana safely falls back to a complete
listing without claiming incremental progress.
Idempotent Google GET/HEAD calls retry bounded transport failures and standard transient HTTP
statuses; a 403 is retried only for Google's explicit rate-limit/backend reasons. Gmail detail
requests also retry a small, bounded 400 window before strict runs fail closed.

Complete, reconciling Google runs fail closed on unresolved listing, detail, or conversion data
so a truncated snapshot can never reconcile as if it were whole. Drive rejects an
`incompleteSearch` listing, malformed file records, missing or unparsable `modifiedTime`, invalid
pagination cursors, and content that cannot be downloaded without a cached copy. Unsupported or
empty file bodies are retained with `content_unavailable=true` and an explicit recovery marker;
they are not treated as extracted text. PDFs with no extractable text use the explicit placeholder
described above.
Gmail rejects malformed message listings, invalid pagination cursors, cached or
fresh detail IDs that do not match the listed ID, any message detail that is denied or unavailable
between list and detail requests, and messages that fail document conversion. Calendar applies the
same strict listing, event, and cursor checks and paginates the calendar list. The one tolerated
omission on a complete Drive run is a file whose content download failed but which still has a
prior cached body: the cached body is emitted, marked `content_stale` in metadata, and diagnostics
expose only the exception class.

Bounded trial or initial syncs never reconcile, so they keep the diagnostic skip behavior instead:
malformed records, unsupported content, and isolated denied or unavailable message details are
skipped with stderr diagnostics rather than aborting the trial. Gmail bounded runs still refuse
when more than 10% of an uncached page (with a minimum allowance of ten messages) is denied, so
Cortana cannot reconcile against a broad permission failure even on a bounded run.

Discord also keeps an owner-only derived cache. After the first complete channel snapshot,
scheduled runs request only messages after the newest cached snowflake. A complete refresh runs
daily to capture edits and deletions, while every emitted snapshot remains complete for safe
reconciliation.

Slack uses the same complete-snapshot contract. Its owner-only cache stores normalized thread
payloads and the newest parent timestamp per channel; ordinary runs request only messages newer
than that cursor, then emit the complete cached channel snapshot so reconciliation remains safe.
At most once per day Slack performs a full history refresh to capture edits and deletions. Bounded
validation disables the cache, and a bounded trial or initial sync never prunes cached threads it
did not enumerate.

GitHub code sources use an explicit repository allowlist and the GitHub REST API. Each selected
`owner/repository` is resolved to its default branch, enumerated through the Git tree API, and
limited to UTF-8 text/code files; generated trees, vendored dependencies, binary blobs, and files
larger than 512 KiB are skipped. A truncated tree fails closed rather than pretending that a
partial repository is a complete snapshot. The connector does not search the account, follow
repository links, or clone arbitrary URLs. Bounded validation and trial syncs stop before the
configured document cap and never reconcile deletions.

## Credentials

- Slack tokens are read from the configured environment-variable name. Slack browser OAuth
  adds a separate owner-only user-token file for workspace assignment; the Slack bot token
  remains the operational message-sync credential.
- Discord uses the signed-in Discord Desktop client through its local RPC socket. Cortana stores
  the resulting owner-only RPC access token in the configured `token` file and never accepts a
  pasted Discord credential or an environment-variable token.
- GitHub code sources read either a personal/GitHub App access token from the configured
  environment-variable name or a private OAuth token JSON file, and always require an explicit
  repository allowlist. `cortana authorize-github SOURCE` uses the configured GitHub OAuth client
  id JSON and device flow, while `cortana github-repositories SOURCE` returns a bounded safe
  repository list for Desktop selection. The Desktop chooser persists only explicitly checked
  `owner/repository` entries. Neither command reads repository content.
- `cortana discord-channels SOURCE` returns a bounded, read-only guild and channel list through
  the running Discord Desktop RPC client. It never accepts a pasted credential, starts ingestion, or
  prints the RPC token. The Desktop chooser persists only explicitly checked channel snowflake
  IDs; the renderer can send only the source name and selected channel IDs.
- `cortana provider-models --kind embedding|query` returns the bounded model catalog the
  configured OpenAI-compatible provider advertises through `/models` (sanitized ids plus explicit
  capability metadata only when the provider publishes it). The Desktop model selectors use it
  when available; local Qwen embedding presets remain available for the bundled local path, while
  cloud and query models stay as the saved custom value until the provider advertises them. The
  call is read-only, never follows redirects, uses a strict timeout, and never prints the provider
  API key.
- `cortana authorize-discord SOURCE` asks the running Discord Desktop client to authorize the
  configured application with the `rpc`, `identify`, and `messages.read` scopes, exchanges the
  returned code at Discord's token endpoint, and stores the resulting token in the private source
  token file. `cortana discord-servers SOURCE` lists the bounded guilds visible to that authorized
  desktop client through RPC, and `cortana discord-channels SOURCE` lists their channels. Server
  selection is persisted per source in `servers`; each Discord source belongs to exactly one
  workspace. RPC does not expose a paginated historical-message command, so ingestion captures
  the bounded messages returned by channel reads and upserts later snapshots without
  pruning records it could not enumerate.
- `cortana authorize-slack SOURCE` runs Slack browser OAuth (Authorization Code + PKCE against
  the fixed endpoints `https://slack.com/oauth/v2/authorize` and
  `https://slack.com/api/oauth.v2.access`) with the source's OAuth client JSON and stores the
  resulting user token in the source's private token file. `cortana slack-workspaces SOURCE`
  lists the bounded workspace (team) that token is scoped to against `https://slack.com/api/team.info`,
  refreshing the token once when it is expired or rejected. Slack validates redirect URIs
  exactly, so the loopback callback uses the fixed port `47521`
  (`http://127.0.0.1:47521/callback`) and the operator registers that exact URL in the Slack
  app. Team selection is persisted per source in `teams` (with display names kept index-aligned
  in `team_names`); each Slack source belongs to exactly one workspace, so that is the
  per-workspace workspace assignment. Channel listing and message sync remain bot-token based
  via `SLACK_BOT_TOKEN`; a source with no OAuth setup keeps the plain token behavior unchanged.
- `cortana buzz-communities SOURCE` lists the bounded communities recorded in the source's
  read-only `agents/teams.json` identity file under the configured Buzz data directory. The
  file must be a regular, non-symlink JSON array bounded at 512 KiB; each record must carry
  stable string `id` and `name` fields, and missing, malformed, or duplicate entries fail
  closed. The command never runs ingestion, never starts a sync, never touches the retention
  database or agent logs, and never infers identity from persona event content. Community
  selection is persisted per source in `communities` (with display names kept index-aligned
  in `community_names`); each Buzz source belongs to exactly one workspace, so that is the
  per-workspace community assignment. Identity is a local, read-only file: the connector's
  read-only behavior is unchanged.
- Google Drive, Gmail, and Calendar accept an OAuth token JSON path. Desktop authorization uses a
  Google **Desktop app** OAuth client JSON, Authorization Code + PKCE, a random loopback callback,
  and the minimum read-only scopes required by the Google sources that share that token. Refresh
  data is updated atomically and the token file is forced to mode `0600`. Existing token and OAuth
  client files must be regular, non-symlink files with owner-only permissions on Unix.
- A Google source may use `token_env` instead of `token` when the named environment value contains
  an absolute OAuth token JSON path. The Desktop editor stores that path value write-only in its
  managed secret file; it does not accept inline token JSON.
- Google Calendar full snapshots persist the provider's `nextSyncToken` and the complete event
  snapshot in the source's private `connector-cache/<source>/calendar.sqlite3` cache. Later full
  runs request only the provider delta, apply additions, updates, and deletions to the cached
  snapshot, and emit the resulting complete snapshot so reconciliation remains safe. The cursor
  is scoped to the token file, workspace, and query configuration; a changed scope or expired
  Google token causes a clean full rebuild. Bounded validation and capped non-reconciling trials
  never read or advance this cache.
- Apple Notes uses the local macOS Notes automation permission and stores no credential. Desktop's
  **Grant Apple Notes access** action opens macOS Privacy & Security → Automation; allow Cortana
  (or the invoking terminal) to control Notes before validating. Each Apple Notes source may set
  exact `folders` to include or `exclude_folders` to omit. Use separate sources when folders belong
  to different workspaces; an empty include list means all folders unless exclusions are present.
  Folder metadata is retained on every indexed note.
- Buzz opens the retention database read-only. Community identity comes from the
  read-only `agents/teams.json` file; see `cortana buzz-communities SOURCE` above.

Never place secret values in `config.toml`, logs, or the repository. Use a secret manager,
launchd/systemd environment file with restrictive permissions, or the host platform's secret
injection.

### Authorize Google sources

Create a Desktop app OAuth client in Google Cloud, enable only the APIs the selected sources need,
then configure both absolute paths:

```toml
[[sources]]
name = "personal-drive"
kind = "google-drive"
project = "personal"
token = "/Users/example/.config/cortana/google-personal-token.json"
oauth_client = "/Users/example/.config/cortana/google-desktop-client.json"
```

Save the source before choosing **Authorize** in Desktop, or run:

```bash
cortana authorize-google personal-drive
```

Cortana opens Google's consent page in the system browser and waits up to five minutes for the
loopback callback. The command never prints tokens. Sources sharing the same token file are
authorized together so the stored grant contains the union of their minimum read-only scopes.
Use separate token paths for different Google accounts or trust domains. The OAuth client file is
configuration, not a user token, but Cortana still rejects symlinks, broad permissions, and
oversized client files.
The token destination must be outside a filesystem source root.
When `token_env` is used instead of `token`, its configured value is the private token destination;
Desktop authorization can create or update that file after the OAuth client path is supplied.

Authorization does not validate, sync, embed, index, or reconcile the source. After consent,
run the bounded validation described below. Google may not return a new refresh token on a later
grant; Cortana preserves the existing refresh token in that case.

### Authorize Discord sources

Discord authorization uses the signed-in **Discord Desktop** client and assigns servers to a
workspace without adding a bot to any server. Create an OAuth application in the
[Discord developer portal](https://discord.com/developers/applications), register the exact
desktop redirect URI `http://127.0.0.1/callback` under OAuth2, and save a JSON file with the
application's client id and client secret:

```json
{ "client_id": "123456789012345678", "client_secret": "application-secret" }
```

Configure an absolute OAuth client path and an absolute private user-token destination on the
source (the Desktop token file picker creates the destination before authorization):

```toml
[[sources]]
name = "community-discord"
kind = "discord"
project = "community"
channels = ["123456789012345678"]
servers = ["987654321098765432"]
token = "/Users/example/.config/cortana/discord-rpc-token.json"
oauth_client = "/Users/example/.config/cortana/discord-rpc-client.json"
```

Save the source before choosing **Authorize** in Desktop, or run:

```bash
cortana authorize-discord community-discord
```

Cortana asks the running Discord Desktop client to show its consent prompt, exchanges the returned
code, and stores the access token in an owner-only file; it never prints the token. The Desktop
server chooser then persists only explicitly checked guild snowflake IDs into `servers` (per
source, so per workspace). The OAuth client file is configuration, not a user token, but Cortana
still rejects symlinks, broad permissions, and oversized files, and the token destination must be
outside every filesystem source root. Discord must remain running whenever discovery or message
ingestion reads the local RPC socket.

Authorization does not validate, sync, embed, index, or reconcile the source. A Discord source
without both explicit RPC paths fails closed and cannot fall back to a bot or personal token.

### Authorize Slack sources

Slack browser authorization assigns **workspaces** (teams) to a workspace; it does not replace
the bot token. The Python connector keeps reading the configured `SLACK_BOT_TOKEN` environment
variable for channel selection and message sync, and that variable is a credential, never a
path. The user token from browser OAuth carries the minimal `team:read` scope, which is what
`team.info` needs to identify the workspace the token is scoped to (a Slack user token is always
scoped to exactly one workspace). Create an OAuth app in the
[Slack API app management](https://api.slack.com/apps) console, add the exact loopback redirect
URL `http://127.0.0.1:47521/callback` under **OAuth & Permissions → Redirect URLs** (Slack does
not accept wildcard ports, so the callback uses this one fixed port), and save a JSON file with
the app's client id (plus the optional client secret for confidential apps):

```json
{ "client_id": "1234567890123.9876543210987", "client_secret": "optional-secret" }
```

Configure an absolute OAuth client path and an absolute private user-token destination on the
source (the Desktop token file picker creates the destination before authorization):

```toml
[[sources]]
name = "team-slack"
kind = "slack"
project = "work"
channels = ["C0123456789"]
teams = ["T0123456789"]
team_names = ["Acme Engineering"]
token_env = "SLACK_BOT_TOKEN"
token = "/Users/example/.config/cortana/slack-user-token.json"
oauth_client = "/Users/example/.config/cortana/slack-oauth-client.json"
```

Save the source before choosing **Authorize** in Desktop, or run:

```bash
cortana authorize-slack team-slack
```

Cortana opens Slack's authorization page in the system browser and waits up to five minutes for
the loopback callback. It stores the user token in an owner-only file and never prints it.
`cortana slack-workspaces SOURCE` then lists the single workspace that token is scoped to; the
Desktop workspace chooser persists only explicitly checked team ids into `teams` with the
display names kept index-aligned in `team_names` (per source, so per workspace). When the app
enables token rotation, discovery refreshes the token once when it is expired or rejected and
persists the refresh atomically; without rotation the token is long-lived. The OAuth client file
is configuration, not a user token, but Cortana still rejects symlinks, broad permissions, and
oversized files, and the token destination must be outside every filesystem source root.

Authorization does not validate, sync, embed, index, or reconcile the source. Sources without an
OAuth client or token file keep the original token-only behavior unchanged.

## Connector contract

Before the first sync of any configured source, validate it with explicit small bounds:

```bash
cortana validate-source SOURCE_NAME \
  --max-documents 25 \
  --max-bytes 10485760 \
  --max-seconds 60
```

When the three overrides are omitted, the CLI applies the same safe defaults: 25 documents,
5 MiB, and 60 seconds. Use larger explicit limits only when you are deliberately proving coverage
for a matching guided initial-sync or recurring-sync budget.

Filesystem sources larger than the requested budgets fail closed by default: a root whose
preflight walk exceeds the document, byte, or wall-clock limits is rejected, because a truncated
validation must never bless a full-corpus sync. To validate a broad code or notes root with a
small budget, opt in explicitly with `--sample`:

```bash
cortana validate-source CODE_ROOT --sample \
  --max-documents 25 \
  --max-bytes 5242880 \
  --max-seconds 60
```

A sampled validation walks at most the requested document, byte, and wall-clock limits and
records the outcome in `source-validations.json` with `complete: false`; if the sample happens to
cover the whole root, it records `complete: true` and keeps full-corpus authority. Only an explicit
`complete: true` validation authorizes a recurring or full-corpus sync. Records written before the
completeness marker existed have unknown scope and must be revalidated; they never inherit
full-corpus authority. A sampled record satisfies an explicitly equal-or-smaller non-reconciling trial or initial sync
(`sync --require-validation --no-reconcile` with matching limits), but it can never authorize a
reconciling sync, the recurring-sync job, or the `--allow-sync-service` readiness gate, which all
require a complete validation at equal or larger limits. `--sample` applies only to filesystem
sources; connector validations already run a capped snapshot and are rejected if the flag is
supplied.

Validation can target a disabled source by exact name. It fetches only that connector snapshot,
enforces the wall-clock and live stdout/stderr spool bounds, parses every emitted document, then
deletes the private spool. It never opens the index, embeds content, or reconciles records. The
latest metadata-only outcome is written atomically to the owner-only
`data_dir/source-validations.json` file so operators can distinguish a proven connector from one
that is merely configured. This record contains counts, limits, timestamps, and a bounded error
summary—never credentials, source content, or connector output.
Filesystem validation performs the same bounded preflight walk used by sync, so start with a
narrow root and conservative limits, or pass `--sample` to validate an oversized root as a
bounded sample as described above. Desktop runs filesystem validations in sample mode at one of
the guided initial-sync budget tiers (100, 500, or 2,000 documents with matching byte and duration
limits) so the resulting record covers a subsequent non-reconciling initial sync; the limits shown
in `source-validations.json` always reflect the validation that actually ran, and a Desktop
initial-sync plan reports whether the covering validation was complete or a bounded sample.

Desktop exposes a separately confirmed guarded trial sync after validation. It invokes the fixed
equivalent of:

```bash
cortana sync --source SOURCE \
  --require-validation --no-reconcile \
  --max-documents 25 --max-bytes 5242880 --max-seconds 300
```

`--require-validation` fails before opening the index or embedding provider unless the selected
source is enabled and its latest validation succeeded for the exact current source configuration
at equal or larger document and byte limits. A reconciling run additionally requires the
validation to be complete: a bounded sample recorded with `--sample` is rejected for any sync
that deletes records absent from its snapshot, while a non-reconciling trial may rely on a
matching sample. Omitting `--source` widens the same check to every
enabled source; the installed recurring sync job always invokes this all-sources form, so each
scheduled run re-checks validation freshness and budgets before any connector is contacted. The
validation record stores only a one-way
configuration fingerprint. Trial sync may embed and index committed batches, but it never deletes
records absent from the bounded snapshot. Cancellation preserves already committed batches.

Desktop adds a guided initial sync on top of the same boundary for first-time ingestion. It offers
exactly three fixed budget tiers — 100 documents/25 MiB/15 minutes, 500/64 MiB/30 minutes, or
2,000/128 MiB/60 minutes — and the renderer can select only the tier enum, never raw flags or
numbers. The flow is plan-then-confirm: a read-only plan request resolves the saved source and
returns the exact limits that execution will enforce, and execution requires that plan plus an
explicit confirmation and a successful validation recorded at equal or larger limits (run
`validate-source` with the same budget, or use the Desktop **Validate for this budget** action).
Execution invokes the fixed equivalent of:

```bash
cortana sync --source SOURCE \
  --require-validation --no-reconcile \
  --max-documents 100 --max-bytes 26214400 --max-seconds 900
```

with the numbers of the selected tier. It runs under the same single-job lock with cancellation
and metadata-only audit events that include the tier, and it never escalates beyond the selected
budget. Desktop initial syncs and trial syncs never reconcile deletions; a complete CLI or
scheduled sync remains the reconciliation path.

External connectors must emit one JSON object per line:

```json
{
  "source": "upstream",
  "source_id": "stable-123",
  "title": "Example",
  "content": "Evidence body",
  "uri": "https://example.test/item/123",
  "updated_at": "2026-07-29T12:00:00Z",
  "project": "work",
  "acl": [],
  "metadata": {}
}
```

`source_id` must remain stable across runs. `content` must be plain searchable text. Put
provenance, channel/account identifiers, participants, and source-specific fields in `metadata`;
never place credentials there.

Set `acl` on a source to apply explicit access labels to every document that connector emits
without its own ACL. When omitted, the source automatically inherits its `project` workspace label;
new source records are therefore private to that workspace by default. A document with one or more
labels is returned only to a query principal with at least one matching label; the implicit loopback
owner can access all labels. Use stable trust-domain labels such as `personal`, `work`, or `shared`,
not user-controlled channel names. Existing empty-ACL rows are legacy public data and must be
reviewed with `cortana acl plan` before enabling shared principals.

## Pre-embedded import

`cortana import-embeddings` accepts trusted JSON Lines for migrations from a compatible vector
store. Every record declares `embedding_fingerprint`, one normalized `document`, and one or more
`chunks` containing text and a vector. Cortana rejects the stream on the first fingerprint,
dimension, empty-chunk, or JSON mismatch. Valid vectors are also written to the persistent
embedding cache, allowing later native source syncs to reuse identical chunks.

The exporter terminates the stream with a completion record containing the exact document count.
Cortana requires that trailer before it reconciles each `(source, project)` represented by the
stream, so a broken or truncated pipe cannot delete an earlier snapshot. Pass
`--no-reconcile` only when intentionally importing a partial snapshot. Do not import vectors whose
model, dimensions, or preprocessing are uncertain; rebuild those records through normal ingestion.

When changing the embedding model for an existing index, `cortana rebuild-embeddings --from ...
--force` provides the safe full-corpus path. It preserves canonical documents, ACLs, provenance,
and chunk text while replacing only their vectors, with an atomic generation swap after all chunks
are embedded successfully.
