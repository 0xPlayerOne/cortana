use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use cortana::config::{Config, SourceConfig, default_config_path};
use cortana::connectors;
use cortana::context::{self, ContextBundle};
use cortana::embed::{CachedEmbedder, DeterministicEmbedder, Embedder, OpenAiEmbedder};
use cortana::model::Document;
use cortana::retrieval;
use cortana::store::{Store, SyncRunStatus};
use cortana::{
    api, discord_oauth, github_oauth, google_oauth, mcp, migration, provider_models, service,
    source_status, source_validation, supervisor,
};
use fs2::FileExt;
use futures_util::{StreamExt, TryStreamExt, stream};
use serde::Deserialize;
use tokio::process::Command as ProcessCommand;

// Leave half of the default local TEI permits available for interactive agents.
const EMBEDDING_REQUEST_SIZE: usize = 8;
// The CLI context command mirrors the HTTP/MCP context contract defaults.
const DEFAULT_CONTEXT_LIMIT: usize = 10;
// A plain `validate-source` call must remain a read-only preflight. Callers
// that need coverage for a larger initial or recurring sync must opt into the
// matching explicit limits instead of inheriting the source's write budget.
const VALIDATION_MAX_DOCUMENTS: usize = 25;
const VALIDATION_MAX_BYTES: u64 = 5 * 1024 * 1024;
const VALIDATION_MAX_SECONDS: u64 = 60;
const QUARANTINE_ACL_LABEL: &str = "__quarantine__";

#[derive(Debug, Parser)]
#[command(name = "cortana", version, about = "Agent-native second brain")]
struct Cli {
    #[arg(long, global = true, env = "CORTANA_CONFIG")]
    config: Option<PathBuf>,
    #[arg(long, global = true, help = "Use deterministic local test embeddings")]
    offline: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a safe local configuration and data directory.
    Init {
        #[arg(long)]
        connector_command: Option<PathBuf>,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Securely migrate reusable sources and credentials from a Hermes installation.
    MigrateHermes {
        #[arg(long)]
        hermes_home: Option<PathBuf>,
        #[arg(long)]
        developer_root: Option<PathBuf>,
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        connector_command: Option<PathBuf>,
        #[arg(long = "google-account")]
        google_accounts: Vec<String>,
        #[arg(long = "discord-channel")]
        discord_channels: Vec<String>,
        #[arg(long = "slack-channel")]
        slack_channels: Vec<String>,
        #[arg(
            long,
            help = "Replace an existing Cortana configuration and migrated files"
        )]
        force: bool,
    },
    /// Validate configuration, storage, and the embedding provider.
    Doctor,
    /// Adopt a reviewed embedding generation without rebuilding indexed documents.
    MigrateEmbedding {
        #[arg(long, value_name = "FINGERPRINT")]
        from: String,
        #[arg(long, help = "Confirm the metadata and derived-cache migration")]
        force: bool,
    },
    /// Re-embed every indexed chunk and atomically adopt a new generation.
    RebuildEmbeddings {
        #[arg(long, value_name = "FINGERPRINT")]
        from: String,
        #[arg(
            long,
            help = "Confirm a full-corpus re-embedding and recovery snapshot"
        )]
        force: bool,
    },
    /// Run deterministic retrieval and answer quality gates in an isolated temporary index.
    Eval {
        #[arg(long, help = "Use a custom synthetic evaluation fixture")]
        fixture: Option<PathBuf>,
        #[arg(
            long,
            help = "Evaluate planner+synthesis against the configured query model"
        )]
        model: bool,
    },
    /// Check production dependencies without starting or scheduling ingestion.
    Readiness {
        #[arg(long, default_value = "http://127.0.0.1:7331")]
        api_url: String,
        #[arg(long, default_value_t = 48)]
        max_backup_age_hours: u64,
        #[arg(
            long,
            help = "Acknowledge an explicitly installed recurring sync service"
        )]
        allow_sync_service: bool,
    },
    /// Inspect or migrate legacy public document ACLs.
    Acl {
        #[command(subcommand)]
        action: AclAction,
    },
    /// Export the retained metadata-only audit trail for incident review.
    Audit {
        #[command(subcommand)]
        action: AuditAction,
    },
    /// Create and verify an online SQLite snapshot.
    Backup {
        output: Option<PathBuf>,
        #[arg(long, default_value_t = 14)]
        keep: usize,
    },
    /// Run SQLite's full integrity check against the active index or a backup.
    Verify { input: Option<PathBuf> },
    /// Restore a verified snapshot, retaining a recovery copy of the current index.
    Restore {
        input: PathBuf,
        #[arg(long, help = "Confirm replacement of the current index")]
        force: bool,
    },
    /// Ingest normalized Document records from a JSON Lines file or stdin.
    Ingest {
        #[arg(default_value = "-")]
        input: String,
    },
    /// Import trusted, pre-embedded Document records without calling an embedding provider.
    ImportEmbeddings {
        #[arg(default_value = "-")]
        input: String,
        #[arg(
            long,
            help = "Keep imported records absent from the completed input snapshot"
        )]
        no_reconcile: bool,
    },
    /// Incrementally ingest a code, notes, transcript, or document tree.
    SyncFiles {
        root: PathBuf,
        #[arg(long, default_value = "files")]
        source: String,
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(
            long,
            help = "Inspect filesystem scope and budgets without writing data"
        )]
        plan: bool,
        #[arg(long, help = "Override the document budget for this run")]
        max_documents: Option<usize>,
        #[arg(long, help = "Override the content-byte budget for this run")]
        max_bytes: Option<u64>,
        #[arg(long, help = "Override the wall-clock budget for this run")]
        max_seconds: Option<u64>,
        #[arg(long, help = "Relative path to exclude; may be repeated")]
        exclude: Vec<String>,
    },
    /// Synchronize enabled sources declared in the configuration.
    Sync {
        #[arg(long)]
        source: Option<String>,
        #[arg(long, help = "Keep records missing from a completed source snapshot")]
        no_reconcile: bool,
        #[arg(
            long,
            help = "Inspect source scope and budgets without fetching or writing data"
        )]
        plan: bool,
        #[arg(long, help = "Override the per-source document budget for this run")]
        max_documents: Option<usize>,
        #[arg(
            long,
            help = "Override the per-source content-byte budget for this run"
        )]
        max_bytes: Option<u64>,
        #[arg(long, help = "Override the per-source wall-clock budget for this run")]
        max_seconds: Option<u64>,
        #[arg(
            long,
            help = "Require a current successful validation at equal or larger limits for the selected source, or for every enabled source when --source is omitted"
        )]
        require_validation: bool,
    },
    /// Fetch and validate one configured source without embedding or indexing it.
    /// Omitting overrides uses the safe 25-document, 5 MiB, 60-second preflight bounds.
    ValidateSource {
        source: String,
        #[arg(
            long,
            help = "Override the document budget for this validation (default: 25)"
        )]
        max_documents: Option<usize>,
        #[arg(
            long,
            help = "Override the content-byte budget for this validation (default: 5242880)"
        )]
        max_bytes: Option<u64>,
        #[arg(
            long,
            help = "Override the wall-clock budget for this validation (default: 60 seconds)"
        )]
        max_seconds: Option<u64>,
        #[arg(
            long,
            help = "Filesystem only: validate at most the requested budgets and record a bounded sample when the source is larger; a sampled validation never authorizes a full-corpus sync"
        )]
        sample: bool,
    },
    /// Authorize a configured Google source in the system browser without reading source data.
    AuthorizeGoogle { source: String },
    /// Authorize a configured GitHub source through the device flow without reading repository content.
    AuthorizeGithub { source: String },
    /// Authorize a configured Discord source in the system browser without reading server or channel data.
    AuthorizeDiscord { source: String },
    /// List bounded GitHub repositories visible to a configured source for selection.
    GithubRepositories { source: String },
    /// List bounded Discord guilds and channels visible to a configured source for selection.
    DiscordChannels { source: String },
    /// List the models advertised by the configured OpenAI-compatible provider for selection.
    ProviderModels {
        #[arg(long, value_enum)]
        kind: ProviderModelsKind,
    },
    /// List bounded Discord servers (guilds) the authorized user belongs to for per-workspace assignment.
    DiscordServers { source: String },
    /// Search indexed evidence with semantic and lexical rank fusion.
    Search {
        query: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: usize,
    },
    /// Build a token-bounded, citation-ready context bundle for agents.
    Context {
        query: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(
            long,
            default_value_t = DEFAULT_CONTEXT_LIMIT,
            value_parser = clap::builder::RangedU64ValueParser::<usize>::new()
                .range(1u64..=retrieval::MAX_RESULT_LIMIT as u64)
        )]
        limit: usize,
        #[arg(
            long,
            value_parser = clap::builder::RangedU64ValueParser::<usize>::new()
                .range(context::MIN_CONTEXT_TOKENS as u64..=context::MAX_CONTEXT_TOKENS as u64)
        )]
        max_tokens: Option<usize>,
    },
    /// Serve the HTTP query API.
    Serve {
        #[arg(long, default_value = "127.0.0.1:7331")]
        address: String,
        #[arg(long, default_value = "apps/web/dist")]
        web_dir: PathBuf,
        #[arg(long, help = "Serve only the JSON API")]
        no_web: bool,
        #[arg(
            long,
            help = "Permit a bearer-authenticated non-loopback bind; terminate TLS upstream"
        )]
        allow_remote: bool,
        #[arg(
            long,
            env = "CORTANA_API_TOKEN_ENV",
            help = "Environment variable containing the HTTP bearer token"
        )]
        api_token_env: Option<String>,
    },
    /// Serve retrieval tools over MCP stdio.
    Mcp {
        #[arg(
            long,
            help = "Environment variable containing a configured scoped agent token"
        )]
        token_env: Option<String>,
    },
    /// Supervise the configured local OpenAI-compatible embedding process.
    EmbeddingService,
    /// Install, inspect, or remove the per-user background services.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ProviderModelsKind {
    Embedding,
    Query,
}

#[derive(Debug, Subcommand)]
enum ServiceAction {
    /// Install and immediately bootstrap per-user macOS launchd, Linux systemd, or Windows Task Scheduler jobs.
    Install {
        #[arg(long, default_value = "apps/web/dist")]
        web_dir: PathBuf,
        #[arg(
            long,
            help = "Install the API service without serving the workspace assets"
        )]
        no_web: bool,
        #[arg(long)]
        working_directory: Option<PathBuf>,
        #[arg(long, default_value_t = 900)]
        sync_seconds: u64,
        #[arg(long, default_value_t = 86_400)]
        backup_seconds: u64,
        #[arg(long)]
        no_embedding_service: bool,
        #[arg(
            long,
            help = "Install the recurring sync job (disabled by default for safe query-only operation)"
        )]
        enable_sync_service: bool,
    },
    /// Print current background service states.
    Status {
        #[arg(long, help = "Print a machine-readable service report")]
        json: bool,
    },
    /// Start one installed background service.
    Start { service: ServiceName },
    /// Stop one background service without removing its configuration.
    Stop { service: ServiceName },
    /// Restart one installed background service.
    Restart { service: ServiceName },
    /// Stop and remove Cortana's per-user background services.
    Uninstall,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ServiceName {
    Embedding,
    Server,
    Sync,
    Backup,
}

impl ServiceName {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Embedding => "embedding",
            Self::Server => "server",
            Self::Sync => "sync",
            Self::Backup => "backup",
        }
    }
}

#[derive(Debug, Subcommand)]
enum AclAction {
    /// Report public rows and preview explicit project-to-label mappings.
    Plan {
        #[arg(long = "project", value_name = "PROJECT=LABEL[,LABEL]")]
        projects: Vec<String>,
        #[arg(
            long,
            help = "Include every unmapped public project in the quarantine plan"
        )]
        quarantine_unmapped: bool,
    },
    /// Apply explicit project ACLs after source defaults agree.
    Apply {
        #[arg(long = "project", value_name = "PROJECT=LABEL[,LABEL]")]
        projects: Vec<String>,
        #[arg(
            long,
            help = "Assign unmapped public projects to the reserved quarantine ACL"
        )]
        quarantine_unmapped: bool,
        #[arg(long, help = "Confirm mutation of legacy public ACL rows")]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
enum AuditAction {
    /// Write retained audit metadata as JSON or newline-delimited JSON.
    Export {
        /// Destination path; omit to write the export to stdout.
        output: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = AuditExportFormat::Jsonl)]
        format: AuditExportFormat,
        #[arg(long, value_name = "COUNT")]
        limit: Option<usize>,
        #[arg(long, help = "Replace an existing destination file")]
        force: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AuditExportFormat {
    Json,
    Jsonl,
}

const MAX_AUDIT_EXPORT_EVENTS: usize = 100_000;

#[tokio::main]
async fn main() -> Result<()> {
    configure_desktop_process_group();
    init_tracing();
    let cli = Cli::parse();
    if let Some(Command::Init {
        connector_command,
        data_dir,
    }) = cli.command.as_ref()
    {
        return init(
            cli.config,
            connector_command.as_deref(),
            data_dir.as_deref(),
        );
    }
    if let Some(Command::MigrateHermes {
        hermes_home,
        developer_root,
        data_dir,
        connector_command,
        google_accounts,
        discord_channels,
        slack_channels,
        force,
    }) = cli.command.as_ref()
    {
        let home = dirs::home_dir().context("cannot resolve the home directory")?;
        let config_path = cli.config.clone().unwrap_or_else(default_config_path);
        migration::migrate_hermes(&migration::HermesMigrationOptions {
            config_path,
            hermes_home: hermes_home.clone().unwrap_or_else(|| home.join(".hermes")),
            developer_root: developer_root
                .clone()
                .unwrap_or_else(|| home.join("Developer")),
            data_dir: data_dir.clone(),
            connector_command: connector_command.clone(),
            google_accounts: google_accounts.clone(),
            discord_channels: discord_channels.clone(),
            slack_channels: slack_channels.clone(),
            force: *force,
        })
        .map_err(|_| anyhow::anyhow!("Hermes migration failed"))?;
        println!("migrated Hermes configuration");
        return Ok(());
    }
    if let Some(Command::Eval { fixture, model }) = cli.command.as_ref() {
        if !*model {
            let report = match fixture {
                Some(path) => cortana::evaluation::run(path).await?,
                None => cortana::evaluation::run_default().await?,
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
            anyhow::ensure!(report.passed, "deterministic evaluation thresholds failed");
            return Ok(());
        }

        let config_path = cli.config.clone().unwrap_or_else(default_config_path);
        let mut config = Config::load(Some(&config_path))?;
        config.load_environment()?;
        let query_api_key = config
            .query
            .api_key_env
            .as_deref()
            .map(|name| {
                config.environment_value(name).with_context(|| {
                    format!("query API key environment variable {name} is not set")
                })
            })
            .transpose()?;
        // `eval --model` is itself the opt-in quality gate for the real
        // planner+synthesis path, so a production config that keeps the safe
        // runtime default (`synthesis_enabled = false`) must not block it.
        // Validate the configured provider (API key above; base URL and model
        // here) before enabling synthesis only on this in-memory copy; the
        // on-disk config and the runtime default are left untouched.
        cortana::answer::validate_query_provider(&config.query)?;
        config.query.synthesis_enabled = true;
        let report = match fixture {
            Some(path) => {
                cortana::evaluation::run_with_config(path, &config.query, query_api_key).await?
            }
            None => {
                cortana::evaluation::run_with_model_default(&config.query, query_api_key).await?
            }
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        anyhow::ensure!(report.passed, "model-backed evaluation thresholds failed");
        return Ok(());
    }
    let config_path = cli.config.clone().unwrap_or_else(default_config_path);
    let mut config = Config::load(Some(&config_path))?;
    config.load_environment()?;
    if let Some(Command::Sync {
        source,
        plan: true,
        max_documents,
        max_bytes,
        max_seconds,
        ..
    }) = cli.command.as_ref()
    {
        return plan_configured_sources(
            &config,
            source.as_deref(),
            SyncOverrides {
                max_documents: *max_documents,
                max_bytes: *max_bytes,
                max_seconds: *max_seconds,
            },
        );
    }
    if let Some(Command::SyncFiles {
        root,
        source,
        project,
        plan: true,
        max_documents,
        max_bytes,
        max_seconds,
        exclude,
    }) = cli.command.as_ref()
    {
        let source =
            ad_hoc_filesystem_source(root.clone(), source.clone(), project.clone(), exclude);
        let limits = SourceLimits::resolve(
            &config,
            &source,
            SyncOverrides {
                max_documents: *max_documents,
                max_bytes: *max_bytes,
                max_seconds: *max_seconds,
            },
        )?;
        let scope = connectors::filesystem_plan(
            root,
            exclude,
            limits.max_documents,
            limits.max_bytes,
            Duration::from_secs(limits.max_seconds),
            false,
        )?;
        println!(
            "{}",
            serde_json::json!({
                "source": source.name,
                "kind": "filesystem",
                "project": source.project,
                "limits": limits,
                "scope": scope,
            })
        );
        return Ok(());
    }
    if let Some(Command::ValidateSource {
        source,
        max_documents,
        max_bytes,
        max_seconds,
        sample,
    }) = cli.command.as_ref()
    {
        return validate_configured_source(
            &config,
            source,
            validation_overrides(*max_documents, *max_bytes, *max_seconds),
            *sample,
        )
        .await;
    }
    if let Some(Command::AuthorizeGoogle { source }) = cli.command.as_ref() {
        let outcome = google_oauth::authorize(&config, source).await?;
        println!("{}", serde_json::to_string(&outcome)?);
        return Ok(());
    }
    if let Some(Command::AuthorizeGithub { source }) = cli.command.as_ref() {
        let outcome = github_oauth::authorize(&config, source).await?;
        println!("{}", serde_json::to_string(&outcome)?);
        return Ok(());
    }
    if let Some(Command::AuthorizeDiscord { source }) = cli.command.as_ref() {
        let outcome = discord_oauth::authorize(&config, source).await?;
        println!("{}", serde_json::to_string(&outcome)?);
        return Ok(());
    }
    if let Some(Command::GithubRepositories { source }) = cli.command.as_ref() {
        let repositories = github_oauth::list_repositories(&config, source).await?;
        println!("{}", serde_json::to_string(&repositories)?);
        return Ok(());
    }
    if let Some(Command::DiscordChannels { source }) = cli.command.as_ref() {
        let channels = cortana::discord::list_channels(&config, source).await?;
        println!("{}", serde_json::to_string(&channels)?);
        return Ok(());
    }
    if let Some(Command::ProviderModels { kind }) = cli.command.as_ref() {
        let kind = match kind {
            ProviderModelsKind::Embedding => provider_models::ModelKind::Embedding,
            ProviderModelsKind::Query => provider_models::ModelKind::Query,
        };
        let models = provider_models::list_provider_models(&config, kind).await?;
        println!("{}", serde_json::to_string(&models)?);
        return Ok(());
    }
    if let Some(Command::DiscordServers { source }) = cli.command.as_ref() {
        let servers = discord_oauth::list_servers(&config, source).await?;
        println!("{}", serde_json::to_string(&servers)?);
        return Ok(());
    }
    if let Some(Command::Sync {
        source,
        plan: false,
        no_reconcile,
        max_documents,
        max_bytes,
        max_seconds,
        require_validation: true,
        ..
    }) = cli.command.as_ref()
    {
        require_sync_validation(
            &config,
            source.as_deref(),
            SyncOverrides {
                max_documents: *max_documents,
                max_bytes: *max_bytes,
                max_seconds: *max_seconds,
            },
            !*no_reconcile,
        )?;
    }
    match &cli.command {
        Some(Command::Backup { output, keep }) => {
            return backup_database(&config, output.as_deref(), *keep);
        }
        Some(Command::Verify { input: Some(input) }) => return verify_database(input),
        Some(Command::Verify { input: None }) => {
            return verify_database(&config.database_path());
        }
        Some(Command::Restore { input, force }) => {
            return restore_database(&config, input, *force);
        }
        Some(Command::EmbeddingService) => return supervisor::run_embedding(&config).await,
        Some(Command::Service { action }) => {
            return manage_service(&config, &config_path, action);
        }
        _ => {}
    }
    let store = Store::open(&config.database_path())?;
    if let Some(Command::Acl { action }) = cli.command.as_ref() {
        return manage_acl(&config, &store, action);
    }
    if let Some(Command::Audit { action }) = cli.command.as_ref() {
        return manage_audit(&config, &store, action);
    }
    let cache_max_entries = config.embedding.cache_max_entries;
    let base_embedder: Arc<dyn Embedder> = if cli.offline {
        Arc::new(DeterministicEmbedder::new(256))
    } else {
        let api_key = config
            .embedding
            .api_key_env
            .as_deref()
            .map(|name| {
                config
                    .environment_value(name)
                    .with_context(|| format!("{name} is not set"))
            })
            .transpose()?;
        Arc::new(OpenAiEmbedder::new(config.embedding.clone(), api_key)?)
    };
    if let Some(Command::Readiness {
        api_url,
        max_backup_age_hours,
        allow_sync_service,
    }) = cli.command.as_ref()
    {
        let report = cortana::readiness::run(
            &config,
            &store,
            base_embedder.as_ref(),
            api_url,
            *max_backup_age_hours,
            *allow_sync_service,
        )
        .await;
        println!("{}", serde_json::to_string_pretty(&report)?);
        anyhow::ensure!(report.passed, "production readiness checks failed");
        return Ok(());
    }
    if let Some(Command::MigrateEmbedding { from, force }) = cli.command.as_ref() {
        return migrate_embedding_generation(&config, &store, base_embedder.as_ref(), from, *force);
    }
    if let Some(Command::RebuildEmbeddings { from, force }) = cli.command.as_ref() {
        let rebuild_embedder: Arc<dyn Embedder> = Arc::new(CachedEmbedder::with_limit(
            store.clone(),
            base_embedder.clone(),
            cache_max_entries,
        ));
        return rebuild_embedding_generation(
            &config,
            &store,
            rebuild_embedder.as_ref(),
            from,
            *force,
        )
        .await;
    }
    store.ensure_fingerprint(&base_embedder.fingerprint())?;
    let embedder: Arc<dyn Embedder> = Arc::new(CachedEmbedder::with_limit(
        store.clone(),
        base_embedder,
        cache_max_entries,
    ));

    match cli.command {
        Some(Command::Doctor) => doctor(&store, embedder.as_ref()).await,
        Some(
            Command::Backup { .. }
            | Command::Verify { .. }
            | Command::Restore { .. }
            | Command::EmbeddingService
            | Command::Service { .. }
            | Command::Readiness { .. }
            | Command::Acl { .. }
            | Command::Audit { .. }
            | Command::ProviderModels { .. },
        ) => {
            unreachable!()
        }
        Some(Command::Ingest { input }) => ingest(&store, embedder.as_ref(), &input).await,
        Some(Command::ImportEmbeddings {
            input,
            no_reconcile,
        }) => {
            let _lock = SyncLock::acquire(&config.data_dir.join("sync.lock"))?;
            import_embeddings(
                &store,
                embedder.as_ref(),
                &input,
                config.embedding.dimension,
                config.embedding.cache_max_entries,
                !no_reconcile,
            )
        }
        Some(Command::SyncFiles {
            root,
            source,
            project,
            plan: false,
            max_documents,
            max_bytes,
            max_seconds,
            exclude,
        }) => {
            let _lock = SyncLock::acquire(&config.data_dir.join("sync.lock"))?;
            let recovered = store.recover_interrupted_syncs()?;
            if recovered > 0 {
                tracing::info!(
                    recovered,
                    "cancelled sync runs orphaned by an interrupted process"
                );
            }
            let source = ad_hoc_filesystem_source(root, source, project, &exclude);
            let limits = SourceLimits::resolve(
                &config,
                &source,
                SyncOverrides {
                    max_documents,
                    max_bytes,
                    max_seconds,
                },
            )?;
            let cancellation = Cancellation::install();
            let control = SourceControl {
                limits,
                started: Instant::now(),
                cancellation: &cancellation,
            };
            let canonical_source = canonical_source(&source);
            let run_id = store.begin_sync(
                &canonical_source,
                &source.project,
                limits.max_documents,
                limits.max_bytes,
                limits.max_seconds,
            )?;
            let result =
                sync_source_documents(&config, &store, embedder.as_ref(), &source, &control, false)
                    .await;
            let result = match result {
                Ok(scope) => {
                    store.finish_sync(
                        &run_id,
                        SyncRunStatus::Succeeded,
                        Some(scope.documents),
                        Some(scope.bytes),
                        Some(0),
                    )?;
                    Ok(())
                }
                Err(error) => {
                    store.finish_sync(&run_id, failure_status(&error), None, None, None)?;
                    Err(error)
                }
            };
            cancellation.stop();
            result
        }
        Some(Command::Sync {
            source,
            no_reconcile,
            plan: false,
            max_documents,
            max_bytes,
            max_seconds,
            require_validation: _,
        }) => {
            let _lock = SyncLock::acquire(&config.data_dir.join("sync.lock"))?;
            let recovered = store.recover_interrupted_syncs()?;
            if recovered > 0 {
                tracing::info!(
                    recovered,
                    "cancelled sync runs orphaned by an interrupted process"
                );
            }
            let cancellation = Cancellation::install();
            let result = sync_configured_sources(
                &config,
                &store,
                embedder.as_ref(),
                source.as_deref(),
                !no_reconcile,
                SyncOverrides {
                    max_documents,
                    max_bytes,
                    max_seconds,
                },
                &cancellation,
            )
            .await;
            cancellation.stop();
            result
        }
        Some(Command::Search {
            query,
            project,
            source,
            limit,
        }) => {
            search(
                &store,
                embedder.as_ref(),
                &query,
                project.as_deref(),
                source.as_deref(),
                limit,
            )
            .await
        }
        Some(Command::Context {
            query,
            project,
            source,
            limit,
            max_tokens,
        }) => {
            let started = Instant::now();
            let project = project.as_deref();
            let source = source.as_deref();
            let result = context_bundle(
                &store,
                &embedder,
                &query,
                project,
                source,
                limit,
                max_tokens.unwrap_or(config.query.context_tokens),
            )
            .await;
            match result {
                Ok(bundle) => {
                    record_cli_context_audit(
                        &store,
                        config.auth.audit_max_events,
                        project,
                        source,
                        "succeeded",
                        Some(bundle.evidence.len()),
                        started,
                    );
                    let mut stdout = std::io::BufWriter::new(std::io::stdout());
                    serde_json::to_writer_pretty(&mut stdout, &bundle)?;
                    writeln!(stdout)?;
                    Ok(())
                }
                Err(error) => {
                    record_cli_context_audit(
                        &store,
                        config.auth.audit_max_events,
                        project,
                        source,
                        "failed",
                        None,
                        started,
                    );
                    Err(error)
                }
            }
        }
        Some(Command::Serve {
            address,
            web_dir,
            no_web,
            allow_remote,
            api_token_env,
        }) => {
            let api_token = api_token_env
                .as_deref()
                .map(|name| {
                    config.environment_value(name).with_context(|| {
                        format!("HTTP token environment variable {name} is not set")
                    })
                })
                .transpose()?;
            let auth = cortana::auth::AuthPolicy::from_config(&config, api_token)?;
            anyhow::ensure!(
                !allow_remote || auth.requires_token(),
                "--allow-remote requires --api-token-env or [[auth.tokens]]"
            );
            let web_dir = (!no_web).then_some(web_dir);
            let query_api_key = config
                .query
                .api_key_env
                .as_deref()
                .map(|name| {
                    config.environment_value(name).with_context(|| {
                        format!("query API key environment variable {name} is not set")
                    })
                })
                .transpose()?;
            let query_model = cortana::answer::configured_model(&config.query, query_api_key)?;
            let answer = cortana::answer::AnswerEngine::new(
                store.clone(),
                embedder.clone(),
                query_model,
                config.query.clone(),
            );
            api::serve(
                api::AppState::new(store, embedder, None)
                    .with_config(&config, service::sync_job_installed())
                    .with_answer_engine(answer)
                    .with_auth_policy(auth),
                &address,
                web_dir.as_deref(),
                allow_remote,
            )
            .await
        }
        Some(Command::Mcp { token_env }) => {
            let (code_sources, message_sources) = mcp_source_groups(&config);
            let mut configured_sources = config
                .sources
                .iter()
                .map(|source| source_status::configured_source_status(&config, source))
                .collect::<Vec<_>>();
            let validation_fingerprints = source_status::validation_fingerprints(&config);
            if let Err(message) = source_status::refresh_source_validations(
                &mut configured_sources,
                &config.data_dir,
                config.ingestion.validation_max_age_hours,
                &validation_fingerprints,
            ) {
                tracing::warn!(%message, "failed to load source validation state for MCP status");
            }
            let principal = if let Some(name) = token_env {
                let token = config
                    .environment_value(&name)
                    .with_context(|| format!("MCP token environment variable {name} is not set"))?;
                let auth = cortana::auth::AuthPolicy::from_config(&config, None)?;
                auth.authenticate(&token)
                    .context("MCP token does not match a configured [[auth.tokens]] principal")?
            } else {
                cortana::auth::Principal::local("local-mcp")
            };
            mcp::serve(
                mcp::BrainServer::new(store, embedder)
                    .with_principal(principal)
                    .with_audit_limit(config.auth.audit_max_events)
                    .with_source_groups(code_sources, message_sources)
                    .with_configured_sources(configured_sources),
            )
            .await
        }
        Some(
            Command::Init { .. }
            | Command::MigrateHermes { .. }
            | Command::MigrateEmbedding { .. }
            | Command::RebuildEmbeddings { .. }
            | Command::Eval { .. }
            | Command::AuthorizeGoogle { .. }
            | Command::AuthorizeGithub { .. }
            | Command::AuthorizeDiscord { .. }
            | Command::GithubRepositories { .. }
            | Command::DiscordChannels { .. }
            | Command::DiscordServers { .. }
            | Command::ValidateSource { .. }
            | Command::Sync { plan: true, .. }
            | Command::SyncFiles { plan: true, .. },
        ) => unreachable!(),
        None => {
            println!("cortana {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

fn configure_desktop_process_group() {
    #[cfg(unix)]
    if std::env::var_os("CORTANA_DESKTOP_PROCESS_GROUP").is_some() {
        // Desktop source jobs are cancelled as a unit. Keep the wrapper and
        // its connector children in one process group without changing the
        // terminal job-control behavior of normal CLI invocations.
        let result = unsafe { libc::setpgid(0, 0) };
        if result != 0 {
            eprintln!(
                "warning: could not isolate Desktop source job process group: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}

fn mcp_source_groups(config: &Config) -> (Vec<String>, Vec<String>) {
    let mut code = Vec::new();
    let mut messages = Vec::new();
    for source in config.sources.iter().filter(|source| source.enabled) {
        let stored_source = source.source.clone().unwrap_or_else(|| source.name.clone());
        match source.kind.as_str() {
            "filesystem" | "github" => code.push(stored_source),
            "buzz" | "gmail" | "slack" | "discord" => messages.push(stored_source),
            _ => {}
        }
    }
    (code, messages)
}

struct SyncLock {
    _file: std::fs::File,
}

#[derive(Clone, Copy, Debug, Default)]
struct SyncOverrides {
    max_documents: Option<usize>,
    max_bytes: Option<u64>,
    max_seconds: Option<u64>,
}

fn validation_overrides(
    max_documents: Option<usize>,
    max_bytes: Option<u64>,
    max_seconds: Option<u64>,
) -> SyncOverrides {
    SyncOverrides {
        max_documents: Some(max_documents.unwrap_or(VALIDATION_MAX_DOCUMENTS)),
        max_bytes: Some(max_bytes.unwrap_or(VALIDATION_MAX_BYTES)),
        max_seconds: Some(max_seconds.unwrap_or(VALIDATION_MAX_SECONDS)),
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
struct SourceLimits {
    max_documents: usize,
    max_bytes: u64,
    max_seconds: u64,
    document_batch_size: usize,
    request_concurrency: usize,
}

impl SourceLimits {
    // Safe preallocation bound for an ingestion batch. A bounded run can never
    // hold more than `max_documents` documents in flight, so an arbitrarily
    // large configured `document_batch_size` must not turn into an oversized
    // `Vec::with_capacity` allocation. Capacity is only an allocation hint;
    // flushing still follows `document_batch_size`.
    fn batch_capacity(&self) -> usize {
        self.document_batch_size.min(self.max_documents).max(1)
    }

    fn resolve(config: &Config, source: &SourceConfig, overrides: SyncOverrides) -> Result<Self> {
        let limits = Self {
            max_documents: overrides
                .max_documents
                .or(source.max_documents)
                .unwrap_or(config.ingestion.max_documents_per_source),
            max_bytes: overrides
                .max_bytes
                .or(source.max_bytes)
                .unwrap_or(config.ingestion.max_bytes_per_source),
            max_seconds: overrides
                .max_seconds
                .or(source.max_duration_seconds)
                .unwrap_or(config.ingestion.max_duration_seconds),
            document_batch_size: config.ingestion.document_batch_size,
            request_concurrency: config
                .ingestion
                .request_concurrency
                .min(config.embedding.request_concurrency),
        };
        anyhow::ensure!(
            limits.max_documents > 0,
            "source {} requires a positive document budget",
            source.name
        );
        anyhow::ensure!(
            limits.max_bytes > 0,
            "source {} requires a positive byte budget",
            source.name
        );
        anyhow::ensure!(
            limits.max_seconds > 0,
            "source {} requires a positive duration budget",
            source.name
        );
        anyhow::ensure!(
            limits.document_batch_size > 0,
            "ingestion document_batch_size must be positive"
        );
        anyhow::ensure!(
            limits.request_concurrency > 0,
            "ingestion request_concurrency and embedding request_concurrency must be positive"
        );
        Ok(limits)
    }
}

struct Cancellation {
    requested: Arc<AtomicBool>,
    listener: tokio::task::JoinHandle<()>,
}

impl Cancellation {
    fn install() -> Self {
        let requested = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&requested);
        let listener = tokio::spawn(async move {
            wait_for_shutdown_signal().await;
            signal.store(true, Ordering::Release);
            tracing::warn!("ingestion cancellation requested");
        });
        Self {
            requested,
            listener,
        }
    }

    fn inert() -> Self {
        Self {
            requested: Arc::new(AtomicBool::new(false)),
            listener: tokio::spawn(std::future::pending()),
        }
    }

    fn is_requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }

    fn stop(self) {
        self.listener.abort();
    }
}

async fn wait_for_shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

struct SourceControl<'a> {
    limits: SourceLimits,
    started: Instant,
    cancellation: &'a Cancellation,
}

impl SourceControl<'_> {
    fn check(&self, source: &str) -> Result<()> {
        anyhow::ensure!(
            !self.cancellation.is_requested(),
            "source {source} cancelled before reconciliation"
        );
        anyhow::ensure!(
            self.started.elapsed() <= Duration::from_secs(self.limits.max_seconds),
            "source {source} exceeded the {} second budget before reconciliation",
            self.limits.max_seconds
        );
        Ok(())
    }

    fn remaining(&self, source: &str) -> Result<Duration> {
        self.check(source)?;
        Ok(Duration::from_secs(self.limits.max_seconds).saturating_sub(self.started.elapsed()))
    }
}

fn plan_configured_sources(
    config: &Config,
    selected: Option<&str>,
    overrides: SyncOverrides,
) -> Result<()> {
    let sources = config
        .sources
        .iter()
        .filter(|source| selected.map_or(source.enabled, |name| source.name == name))
        .collect::<Vec<_>>();
    anyhow::ensure!(
        !sources.is_empty(),
        "no configured sources matched the selection"
    );
    for source in sources {
        let limits = SourceLimits::resolve(config, source, overrides)?;
        let scope = if source.kind == "filesystem" {
            let root = source
                .root
                .as_ref()
                .with_context(|| format!("source {} requires root", source.name))?;
            serde_json::to_value(connectors::filesystem_plan(
                root,
                &source.exclude,
                limits.max_documents,
                limits.max_bytes,
                Duration::from_secs(limits.max_seconds),
                false,
            )?)?
        } else {
            serde_json::json!({
                "inspection": "deferred",
                "reason": "plan mode never calls external connectors"
            })
        };
        println!(
            "{}",
            serde_json::json!({
                "source": source.name,
                "kind": source.kind,
                "enabled": source.enabled,
                "project": source.project,
                "limits": limits,
                "scope": scope,
            })
        );
    }
    Ok(())
}

fn manage_acl(config: &Config, store: &Store, action: &AclAction) -> Result<()> {
    let (values, apply, force, quarantine_unmapped) = match action {
        AclAction::Plan {
            projects,
            quarantine_unmapped,
        } => (projects, false, false, *quarantine_unmapped),
        AclAction::Apply {
            projects,
            quarantine_unmapped,
            force,
        } => (projects, true, *force, *quarantine_unmapped),
    };
    let explicit_mappings = parse_project_acl_mappings(values)?;
    let public = store.public_acl_summary()?;
    let mut mappings = explicit_mappings.clone();
    if quarantine_unmapped {
        mappings.extend(
            public
                .iter()
                .filter(|summary| {
                    !explicit_mappings
                        .iter()
                        .any(|(project, _)| project == &summary.project)
                })
                .map(|summary| {
                    (
                        summary.project.clone(),
                        vec![QUARANTINE_ACL_LABEL.to_string()],
                    )
                }),
        );
    }
    let alignment_errors = acl_alignment_errors(config, &mappings);
    if apply {
        anyhow::ensure!(force, "ACL apply requires --force");
        anyhow::ensure!(
            !mappings.is_empty(),
            "ACL apply requires --project mappings"
        );
        anyhow::ensure!(
            alignment_errors.is_empty(),
            "configured source ACLs do not match the requested migration: {}",
            alignment_errors.join("; ")
        );
        let changed = store.backfill_project_acls(&mappings)?;
        println!(
            "{}",
            serde_json::json!({
                "applied": true,
                "documents_changed": changed,
                "corpus_revision": store.corpus_revision()?,
                "remaining_public": store.public_acl_summary()?,
                "quarantine_unmapped": quarantine_unmapped,
            })
        );
        return Ok(());
    }
    let proposed = mappings
        .iter()
        .map(|(project, labels)| {
            let documents = public
                .iter()
                .find(|summary| &summary.project == project)
                .map_or(0, |summary| summary.documents);
            serde_json::json!({
                "project": project,
                "labels": labels,
                "documents": documents,
            })
        })
        .collect::<Vec<_>>();
    let unmapped_public = public
        .iter()
        .filter(|summary| {
            !explicit_mappings
                .iter()
                .any(|(project, _)| project == &summary.project)
        })
        .map(|summary| summary.project.clone())
        .collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::json!({
            "applied": false,
            "public": public,
            "proposed": proposed,
            "source_alignment_errors": alignment_errors,
            "quarantine_unmapped": quarantine_unmapped,
            "unmapped_public": unmapped_public,
        })
    );
    Ok(())
}

fn parse_project_acl_mappings(values: &[String]) -> Result<Vec<(String, Vec<String>)>> {
    let mut mappings = Vec::new();
    for value in values {
        let (project, labels) = value.split_once('=').with_context(|| {
            format!("invalid ACL mapping {value}; expected PROJECT=LABEL[,LABEL]")
        })?;
        let project = project.trim();
        anyhow::ensure!(!project.is_empty(), "ACL project must not be empty");
        anyhow::ensure!(
            !mappings.iter().any(|(existing, _)| existing == project),
            "duplicate ACL project mapping {project}"
        );
        let mut labels = labels
            .split(',')
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        labels.sort();
        labels.dedup();
        anyhow::ensure!(!labels.is_empty(), "ACL labels must not be empty");
        anyhow::ensure!(
            !labels.iter().any(|label| label == "*"),
            "ACL migration cannot assign the reserved owner wildcard"
        );
        anyhow::ensure!(
            !labels.iter().any(|label| label == QUARANTINE_ACL_LABEL),
            "ACL migration cannot assign the reserved quarantine label"
        );
        mappings.push((project.to_string(), labels));
    }
    Ok(mappings)
}

fn acl_alignment_errors(config: &Config, mappings: &[(String, Vec<String>)]) -> Vec<String> {
    let mut errors = Vec::new();
    for source in &config.sources {
        let Some((_, expected)) = mappings
            .iter()
            .find(|(project, _)| project == &source.project)
        else {
            continue;
        };
        let configured = source.effective_acl();
        if &configured != expected {
            errors.push(format!(
                "{} has acl={configured:?}, expected {expected:?}",
                source.name
            ));
        }
    }
    errors
}

async fn validate_configured_source(
    config: &Config,
    selected: &str,
    overrides: SyncOverrides,
    sample: bool,
) -> Result<()> {
    let source = config
        .sources
        .iter()
        .find(|source| source.name == selected)
        .with_context(|| format!("configured source {selected} was not found"))?;
    let limits = SourceLimits::resolve(config, source, overrides)?;
    anyhow::ensure!(
        !sample || source.kind == "filesystem",
        "source {} kind {} does not support --sample; only filesystem validation can record a bounded sample",
        source.name,
        source.kind
    );
    let cancellation = Cancellation::inert();
    let control = SourceControl {
        limits,
        started: Instant::now(),
        cancellation: &cancellation,
    };
    let validation: Result<serde_json::Value> = async {
        if source.kind == "filesystem" {
            let root = source
                .root
                .as_ref()
                .with_context(|| format!("source {} requires root", source.name))?;
            let scope = connectors::filesystem_plan(
                root,
                &source.exclude,
                limits.max_documents,
                limits.max_bytes,
                control.remaining(&source.name)?,
                sample,
            )?;
            return Ok(serde_json::json!({
                "documents": scope.documents,
                "bytes": scope.bytes,
                "complete": scope.complete,
                "inspection": if scope.complete {
                    "filesystem preflight"
                } else {
                    "filesystem sample"
                }
            }));
        }
        cleanup_connector_spools(&config.data_dir)?;
        let (spool, diagnostics) = run_connector_to_spool(
            config,
            source,
            &control,
            Some(control.limits.max_documents),
            true,
        )
        .await?;
        let validation = validate_connector_spool(&spool, source, &control);
        let _ = std::fs::remove_file(&spool);
        let _ = std::fs::remove_file(&diagnostics);
        let scope = validation?;
        Ok(serde_json::json!({
            "documents": scope.documents,
            "bytes": scope.bytes,
            "complete": true,
            "inspection": "connector snapshot"
        }))
    }
    .await;
    cancellation.stop();
    let validated_at = chrono::Utc::now();
    let result = match validation {
        Ok(result) => result,
        Err(error) => {
            let status = source_validation::SourceValidationStatus {
                source: source.name.clone(),
                project: source.project.clone(),
                kind: source.kind.clone(),
                status: "failed".into(),
                validated_at,
                documents: None,
                bytes: None,
                max_documents: limits.max_documents,
                max_bytes: limits.max_bytes,
                max_seconds: limits.max_seconds,
                configuration_fingerprint: source_validation::configuration_fingerprint(source)
                    .ok(),
                complete: None,
                error: Some(error.to_string()),
            };
            if let Err(state_error) = source_validation::record(&config.data_dir, status) {
                eprintln!("failed to persist source validation outcome: {state_error}");
            }
            return Err(error);
        }
    };
    source_validation::record(
        &config.data_dir,
        source_validation::SourceValidationStatus {
            source: source.name.clone(),
            project: source.project.clone(),
            kind: source.kind.clone(),
            status: "succeeded".into(),
            validated_at,
            documents: result
                .get("documents")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok()),
            bytes: result.get("bytes").and_then(serde_json::Value::as_u64),
            max_documents: limits.max_documents,
            max_bytes: limits.max_bytes,
            max_seconds: limits.max_seconds,
            configuration_fingerprint: Some(source_validation::configuration_fingerprint(source)?),
            complete: result.get("complete").and_then(serde_json::Value::as_bool),
            error: None,
        },
    )?;
    println!(
        "{}",
        serde_json::json!({
            "source": source.name,
            "kind": source.kind,
            "enabled": source.enabled,
            "project": source.project,
            "limits": limits,
            "validated": true,
            "writes": {
                "documents": 0,
                "embeddings": 0,
                "reconciliations": 0
            },
            "scope": result,
        })
    );
    Ok(())
}

fn require_sync_validation(
    config: &Config,
    selected: Option<&str>,
    overrides: SyncOverrides,
    reconcile: bool,
) -> Result<()> {
    let Some(selected) = selected else {
        return require_enabled_sources_validated(config, overrides, reconcile);
    };
    let source = config
        .sources
        .iter()
        .find(|candidate| candidate.name == selected)
        .with_context(|| format!("configured source {selected} was not found"))?;
    anyhow::ensure!(
        source.enabled,
        "source {selected} must be enabled before sync"
    );
    let limits = SourceLimits::resolve(config, source, overrides)?;
    source_validation::require_success(
        &config.data_dir,
        source,
        limits.max_documents,
        limits.max_bytes,
        limits.max_seconds,
        chrono::Duration::hours(config.ingestion.validation_max_age_hours as i64),
        reconcile,
    )
}

fn ad_hoc_filesystem_source(
    root: PathBuf,
    source: String,
    project: String,
    exclude: &[String],
) -> SourceConfig {
    SourceConfig {
        name: source.clone(),
        kind: "filesystem".into(),
        enabled: true,
        project,
        root: Some(root),
        source: Some(source),
        channels: Vec::new(),
        servers: Vec::new(),
        repositories: Vec::new(),
        token_env: None,
        token: None,
        oauth_client: None,
        query: None,
        labels: Vec::new(),
        max_content_chars: None,
        max_documents: None,
        max_bytes: None,
        max_duration_seconds: None,
        exclude: exclude.to_vec(),
        command: Vec::new(),
        acl: Vec::new(),
    }
}

impl SyncLock {
    fn acquire(path: &std::path::Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        configure_no_follow(&mut options);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(path)
            .with_context(|| format!("failed to open sync lock {}", path.display()))?;
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.is_file(),
            "sync lock is not a regular file: {}",
            path.display()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            anyhow::ensure!(
                metadata.uid() == unsafe { libc::geteuid() },
                "sync lock is not owned by the current user: {}",
                path.display()
            );
            anyhow::ensure!(
                metadata.nlink() == 1,
                "sync lock has multiple hard links: {}",
                path.display()
            );
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        FileExt::try_lock_exclusive(&file).with_context(|| {
            format!(
                "another Cortana sync is already active (lock: {})",
                path.display()
            )
        })?;
        Ok(Self { _file: file })
    }
}

fn manage_service(
    config: &Config,
    config_path: &std::path::Path,
    action: &ServiceAction,
) -> Result<()> {
    match action {
        ServiceAction::Install {
            web_dir,
            no_web,
            working_directory,
            sync_seconds,
            backup_seconds,
            no_embedding_service,
            enable_sync_service,
        } => {
            if *enable_sync_service {
                ensure_recurring_sync_validated(config)?;
            }
            let web_dir = if *no_web {
                None
            } else {
                Some(web_dir.canonicalize().with_context(|| {
                    format!("workspace directory does not exist: {}", web_dir.display())
                })?)
            };
            let working_directory = working_directory
                .clone()
                .unwrap_or(std::env::current_dir()?)
                .canonicalize()?;
            service::install(
                config,
                service::InstallOptions {
                    config: &config_path.canonicalize()?,
                    web_dir: web_dir.as_deref(),
                    no_web: *no_web,
                    working_directory: &working_directory,
                    sync_seconds: *sync_seconds,
                    backup_seconds: *backup_seconds,
                    install_embedding: !no_embedding_service
                        && supervisor::uses_local_service(config),
                    install_sync: *enable_sync_service,
                },
            )
        }
        ServiceAction::Status { json } => {
            let report = service::status()?;
            if *json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                for item in report.services {
                    println!(
                        "{}: {}",
                        item.label,
                        item.state.as_deref().unwrap_or(if item.installed {
                            "not loaded"
                        } else {
                            "not installed"
                        })
                    );
                }
            }
            Ok(())
        }
        ServiceAction::Start { service: name } => service::start(name.as_str()),
        ServiceAction::Stop { service: name } => service::stop(name.as_str()),
        ServiceAction::Restart { service: name } => service::restart(name.as_str()),
        ServiceAction::Uninstall => service::uninstall(),
    }
}

fn manage_audit(config: &Config, store: &Store, action: &AuditAction) -> Result<()> {
    match action {
        AuditAction::Export {
            output,
            format,
            limit,
            force,
        } => {
            let limit = limit.unwrap_or(config.auth.audit_max_events);
            anyhow::ensure!(
                limit <= MAX_AUDIT_EXPORT_EVENTS,
                "audit export limit cannot exceed {MAX_AUDIT_EXPORT_EVENTS} events"
            );
            let events = store.audit_events_for_export(limit)?;
            let count = events.len();
            if let Some(path) = output {
                reject_symlink_path(path)?;
                anyhow::ensure!(
                    *force || !path.exists(),
                    "audit export destination already exists: {}; rerun with --force",
                    path.display()
                );
                let mut options = std::fs::OpenOptions::new();
                options.write(true);
                if *force {
                    options.create(true).truncate(true);
                } else {
                    options.create_new(true);
                }
                configure_no_follow(&mut options);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let file = options
                    .open(path)
                    .with_context(|| format!("failed to create audit export {}", path.display()))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
                }
                let mut writer = std::io::BufWriter::new(file);
                write_audit_export(&mut writer, &events, *format)?;
                writer.flush()?;
                println!("audit export wrote {count} events: {}", path.display());
            } else {
                let mut writer = std::io::BufWriter::new(std::io::stdout());
                write_audit_export(&mut writer, &events, *format)?;
                writer.flush()?;
            }
            Ok(())
        }
    }
}

fn write_audit_export(
    writer: &mut impl Write,
    events: &[cortana::store::AuditEvent],
    format: AuditExportFormat,
) -> Result<()> {
    match format {
        AuditExportFormat::Json => {
            serde_json::to_writer_pretty(&mut *writer, events)?;
            writeln!(writer)?;
        }
        AuditExportFormat::Jsonl => {
            for event in events {
                serde_json::to_writer(&mut *writer, event)?;
                writeln!(writer)?;
            }
        }
    }
    Ok(())
}

fn ensure_recurring_sync_validated(config: &Config) -> Result<()> {
    // The recurring job runs a full-corpus (reconciling) sync, so only a
    // complete validation may bless it; sampled records never qualify.
    require_enabled_sources_validated(config, SyncOverrides::default(), true)
}

/// Re-check that every enabled source has a current successful validation at
/// equal or larger budgets than its resolved run limits.
///
/// The installed recurring sync job invokes this gate on every scheduled run
/// (`sync --require-validation` without `--source`) so a validation that
/// lapsed, failed, lost its configuration fingerprint, or covers smaller
/// budgets than the configured limits fails the run fast instead of ingesting
/// against it; `service install --enable-sync-service` applies the same gate
/// once before scheduling the job. `reconcile` marks whether the guarded run
/// will delete records absent from its snapshot: reconciling runs require a
/// complete validation, while an explicitly bounded non-reconciling run may
/// rely on a matching successful sample validation.
fn require_enabled_sources_validated(
    config: &Config,
    overrides: SyncOverrides,
    reconcile: bool,
) -> Result<()> {
    let mut checked = 0usize;
    for source in config.sources.iter().filter(|source| source.enabled) {
        checked += 1;
        let limits = SourceLimits::resolve(config, source, overrides)
            .with_context(|| format!("invalid recurring sync budget for {}", source.name))?;
        source_validation::require_success(
            &config.data_dir,
            source,
            limits.max_documents,
            limits.max_bytes,
            limits.max_seconds,
            chrono::Duration::hours(config.ingestion.validation_max_age_hours as i64),
            reconcile,
        )
        .with_context(|| {
            format!(
                "recurring sync requires a current successful validation for source {}",
                source.name
            )
        })?;
    }
    anyhow::ensure!(
        checked > 0,
        "recurring sync requires at least one enabled source"
    );
    Ok(())
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("cortana=info,tower_http=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn backup_database(config: &Config, output: Option<&std::path::Path>, keep: usize) -> Result<()> {
    let directory = config.data_dir.join("backups");
    let destination = output.map(PathBuf::from).unwrap_or_else(|| {
        directory.join(format!(
            "cortana-{}.sqlite3",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        ))
    });
    let store = Store::open(&config.database_path())?;
    store.integrity_check()?;
    store.backup(&destination)?;
    if destination.parent() == Some(directory.as_path()) {
        prune_backups(&directory, keep, Some(&destination))?;
    }
    println!("backup verified: {}", destination.display());
    Ok(())
}

fn migrate_embedding_generation(
    config: &Config,
    store: &Store,
    base_embedder: &dyn Embedder,
    from: &str,
    force: bool,
) -> Result<()> {
    let target = base_embedder.fingerprint();
    let _lock = SyncLock::acquire(&config.data_dir.join("sync.lock"))?;
    let current = store
        .stats()?
        .embedding_fingerprint
        .context("the index has no embedding generation; initialize it before migrating")?;
    anyhow::ensure!(
        current == from,
        "embedding generation does not match --from (expected: {from}; actual: {current})"
    );
    if current == target {
        println!("embedding generation already matches configured provider: {target}");
        return Ok(());
    }
    anyhow::ensure!(
        force,
        "embedding migration changes index metadata and clears derived caches; rerun with --force after reviewing the exact --from fingerprint"
    );
    store.integrity_check()?;

    let backup = config.data_dir.join("backups").join(format!(
        "cortana-embedding-migration-{}-{}.sqlite3",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4()
    ));
    store.backup(&backup)?;
    store.migrate_embedding_fingerprint(&current, &target)?;
    if let Err(error) = store.record_audit(
        "local-cli",
        "embedding-generation-migrate",
        None,
        None,
        "ok",
        None,
        0,
        config.auth.audit_max_events,
    ) {
        tracing::warn!(%error, "embedding generation migration audit write failed");
    }
    println!("embedding generation migrated");
    println!("  from: {current}");
    println!("  to: {target}");
    println!("  verified backup: {}", backup.display());
    println!("  indexed documents were not rebuilt; derived caches were cleared");
    Ok(())
}

async fn rebuild_embedding_generation(
    config: &Config,
    store: &Store,
    embedder: &dyn Embedder,
    from: &str,
    force: bool,
) -> Result<()> {
    anyhow::ensure!(
        force,
        "embedding rebuild re-embeds the entire index and creates a recovery snapshot; rerun with --force after reviewing the target provider"
    );
    let target = embedder.fingerprint();
    let _lock = SyncLock::acquire(&config.data_dir.join("sync.lock"))?;
    let current = store
        .stats()?
        .embedding_fingerprint
        .context("the index has no embedding generation; initialize it before rebuilding")?;
    anyhow::ensure!(
        current == from,
        "embedding generation does not match --from (expected: {from}; actual: {current})"
    );
    anyhow::ensure!(
        current != target,
        "embedding generation already matches configured provider: {target}"
    );

    // Probe before creating a recovery snapshot or staging table so a missing
    // local/cloud provider fails without changing the index.
    embedder
        .probe()
        .await
        .context("embedding provider probe failed")?;
    store.integrity_check()?;
    let backup = config.data_dir.join("backups").join(format!(
        "cortana-embedding-rebuild-{}-{}.sqlite3",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4()
    ));
    store.backup(&backup)?;
    let expected = store.begin_embedding_rebuild(&current, &target)?;
    let rebuild = async {
        let concurrency = embedder.request_concurrency().max(1);
        let mut after = None;
        let mut processed = 0_usize;
        loop {
            let page = store.embedding_rebuild_chunks(after.as_deref(), 256)?;
            if page.is_empty() {
                break;
            }
            let requests = page
                .iter()
                .map(|(_, content)| content.clone())
                .collect::<Vec<_>>();
            let batches = stream::iter(requests.chunks(EMBEDDING_REQUEST_SIZE))
                .map(|request| embedder.embed(request))
                .buffered(concurrency)
                .try_collect::<Vec<_>>()
                .await?;
            let vectors = batches.into_iter().flatten().collect::<Vec<_>>();
            anyhow::ensure!(
                vectors.len() == page.len(),
                "embedding provider returned an unexpected vector count during rebuild"
            );
            let staged = page
                .into_iter()
                .zip(vectors)
                .map(|((chunk_id, _), vector)| (chunk_id, vector))
                .collect::<Vec<_>>();
            after = staged.last().map(|(chunk_id, _)| chunk_id.clone());
            store.stage_embedding_rebuild(&staged)?;
            processed = processed.saturating_add(staged.len());
            if processed.is_multiple_of(1_000) || processed == expected {
                eprintln!("rebuilding embeddings: {processed}/{expected} chunks");
            }
        }
        anyhow::ensure!(
            processed == expected,
            "embedding rebuild scanned {processed} of {expected} chunks"
        );
        Ok::<usize, anyhow::Error>(processed)
    }
    .await;
    let processed = match rebuild {
        Ok(processed) => processed,
        Err(error) => {
            if let Err(discard_error) = store.discard_embedding_rebuild() {
                tracing::warn!(%discard_error, "embedding rebuild cleanup failed");
            }
            return Err(error);
        }
    };
    let committed = store.commit_embedding_rebuild(&current, &target)?;
    anyhow::ensure!(
        committed == processed,
        "embedding rebuild commit count mismatch"
    );
    if let Err(error) = store.record_audit(
        "local-cli",
        "embedding-generation-rebuild",
        None,
        None,
        "ok",
        Some(committed),
        0,
        config.auth.audit_max_events,
    ) {
        tracing::warn!(%error, "embedding generation rebuild audit write failed");
    }
    println!("embedding generation rebuilt");
    println!("  from: {current}");
    println!("  to: {target}");
    println!("  chunks: {committed}");
    println!("  verified backup: {}", backup.display());
    Ok(())
}

fn verify_database(path: &std::path::Path) -> Result<()> {
    Store::verify(path)?;
    println!("database verified: {}", path.display());
    Ok(())
}

fn restore_database(config: &Config, input: &std::path::Path, force: bool) -> Result<()> {
    let database = config.database_path();
    anyhow::ensure!(
        force || !database.exists(),
        "restore would replace {}; rerun with --force after stopping Cortana",
        database.display()
    );
    let recovery = database.exists().then(|| {
        config.data_dir.join("backups").join(format!(
            "pre-restore-{}.sqlite3",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        ))
    });
    Store::restore(&database, input, recovery.as_deref())?;
    println!("database restored from {}", input.display());
    if let Some(path) = recovery {
        println!("previous index retained at {}", path.display());
    }
    Ok(())
}

fn prune_backups(
    directory: &std::path::Path,
    keep: usize,
    protected: Option<&std::path::Path>,
) -> Result<()> {
    if keep == 0 || !directory.is_dir() {
        return Ok(());
    }
    let mut backups = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "sqlite3")
                && protected.is_none_or(|protected| path != protected)
        })
        .collect::<Vec<_>>();
    backups.sort();
    let remove = backups.len().saturating_sub(keep.saturating_sub(1));
    for path in backups.into_iter().take(remove) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn init(
    path: Option<PathBuf>,
    connector_command: Option<&std::path::Path>,
    data_dir: Option<&std::path::Path>,
) -> Result<()> {
    let path = path.unwrap_or_else(default_config_path);
    if path.exists() {
        println!("configuration already exists: {}", path.display());
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut config = Config::default();
    if let Some(data_dir) = data_dir {
        config.data_dir = data_dir.to_path_buf();
    }
    if let Some(command) = connector_command {
        config.connectors.command = vec![command.display().to_string()];
    }
    std::fs::create_dir_all(&config.data_dir)?;
    let body = toml::to_string_pretty(&config)?;
    std::fs::write(&path, body)?;
    println!("created {}", path.display());
    Ok(())
}

async fn doctor(store: &Store, embedder: &dyn Embedder) -> Result<()> {
    let vectors = embedder.embed(&["health check".to_string()]).await?;
    let vector = vectors
        .first()
        .context("embedding provider returned no vector")?;
    anyhow::ensure!(
        !vector.is_empty(),
        "embedding provider returned an empty vector"
    );
    let _ = store.all_chunks(None, None)?;
    println!(
        "cortana: healthy (embedding={}, dimension={})",
        embedder.fingerprint(),
        vector.len()
    );
    Ok(())
}

async fn ingest(store: &Store, embedder: &dyn Embedder, input: &str) -> Result<()> {
    let reader: Box<dyn BufRead> = if input == "-" {
        Box::new(std::io::BufReader::new(std::io::stdin()))
    } else {
        Box::new(std::io::BufReader::new(
            std::fs::File::open(input).with_context(|| format!("failed to open {input}"))?,
        ))
    };
    let mut documents = Vec::with_capacity(64);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        documents.push(serde_json::from_str(&line).context("invalid Document JSONL")?);
        if documents.len() == 64 {
            ingest_documents(store, embedder, std::mem::take(&mut documents)).await?;
        }
    }
    if !documents.is_empty() {
        ingest_documents(store, embedder, documents).await?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct EmbeddedImportChunk {
    content: String,
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum EmbeddedImportLine {
    Document {
        embedding_fingerprint: String,
        document: Box<Document>,
        chunks: Vec<EmbeddedImportChunk>,
    },
    Complete {
        records: usize,
    },
}

fn import_embeddings(
    store: &Store,
    embedder: &dyn Embedder,
    input: &str,
    dimension: usize,
    cache_max_entries: usize,
    reconcile: bool,
) -> Result<()> {
    let reader: Box<dyn BufRead> = if input == "-" {
        Box::new(std::io::BufReader::new(std::io::stdin()))
    } else {
        Box::new(std::io::BufReader::new(
            std::fs::File::open(input).with_context(|| format!("failed to open {input}"))?,
        ))
    };
    let expected_fingerprint = embedder.fingerprint();
    let mut imported = 0_usize;
    let mut unchanged = 0_usize;
    let mut completed = false;
    let mut seen = std::collections::HashMap::<(String, String), Vec<String>>::new();
    for (line_number, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: EmbeddedImportLine = serde_json::from_str(&line)
            .with_context(|| format!("invalid embedded JSONL at line {}", line_number + 1))?;
        let (embedding_fingerprint, document, embedded_chunks) = match record {
            EmbeddedImportLine::Document {
                embedding_fingerprint,
                document,
                chunks,
            } => (embedding_fingerprint, document, chunks),
            EmbeddedImportLine::Complete { records } => {
                anyhow::ensure!(
                    !completed,
                    "duplicate embedded import completion record at line {}",
                    line_number + 1
                );
                anyhow::ensure!(
                    records == imported + unchanged,
                    "embedded import is incomplete: expected {records} records, received {}",
                    imported + unchanged
                );
                completed = true;
                continue;
            }
        };
        anyhow::ensure!(
            !completed,
            "embedded import contains data after its completion record at line {}",
            line_number + 1
        );
        anyhow::ensure!(
            embedding_fingerprint == expected_fingerprint,
            "embedding fingerprint mismatch at line {}: expected {}",
            line_number + 1,
            expected_fingerprint
        );
        anyhow::ensure!(
            !embedded_chunks.is_empty(),
            "embedded record has no chunks at line {}",
            line_number + 1
        );
        let chunks = embedded_chunks
            .into_iter()
            .map(|chunk| {
                anyhow::ensure!(
                    !chunk.content.trim().is_empty(),
                    "embedded record has an empty chunk at line {}",
                    line_number + 1
                );
                anyhow::ensure!(
                    chunk.embedding.len() == dimension,
                    "embedding dimension mismatch at line {}: expected {}",
                    line_number + 1,
                    dimension
                );
                store.cache_embedding(&expected_fingerprint, &chunk.content, &chunk.embedding)?;
                Ok((chunk.content, chunk.embedding))
            })
            .collect::<Result<Vec<_>>>()?;
        let key = (document.source.clone(), document.project.clone());
        seen.entry(key)
            .or_default()
            .push(document.source_id.clone());
        if store.upsert(&document, &chunks)? {
            imported += 1;
        } else {
            unchanged += 1;
        }
        if (imported + unchanged).is_multiple_of(1_000) {
            eprintln!("imported embedded records: changed={imported} unchanged={unchanged}");
        }
    }
    anyhow::ensure!(
        completed,
        "embedded import ended without a valid completion record; no reconciliation was performed"
    );
    let mut deleted = 0_usize;
    if reconcile {
        for ((source, project), source_ids) in seen {
            deleted += store.reconcile(&source, &project, &source_ids)?;
        }
    }
    store.prune_embedding_cache(cache_max_entries)?;
    println!("imported embeddings changed={imported} unchanged={unchanged} deleted={deleted}");
    Ok(())
}

async fn ingest_documents(
    store: &Store,
    embedder: &dyn Embedder,
    documents: Vec<Document>,
) -> Result<()> {
    let cancellation = Cancellation::inert();
    let control = SourceControl {
        limits: SourceLimits {
            max_documents: usize::MAX,
            max_bytes: u64::MAX,
            max_seconds: 24 * 60 * 60,
            document_batch_size: usize::MAX,
            request_concurrency: embedder.request_concurrency().max(1),
        },
        started: Instant::now(),
        cancellation: &cancellation,
    };
    let result =
        ingest_documents_controlled(store, embedder, documents, "direct-ingest", &control).await;
    cancellation.stop();
    result
}

async fn ingest_documents_controlled(
    store: &Store,
    embedder: &dyn Embedder,
    documents: Vec<Document>,
    source: &str,
    control: &SourceControl<'_>,
) -> Result<()> {
    let mut changed = 0;
    let mut unchanged = 0;
    let mut pending = Vec::new();
    let mut pending_chunks = 0;
    for document in documents {
        control.check(source)?;
        if !store.needs_update(&document)? {
            store.refresh_timestamp(&document)?;
            unchanged += 1;
            continue;
        }
        let texts = chunk(&document.content);
        pending_chunks += texts.len();
        pending.push((document, texts));
        if pending_chunks >= EMBEDDING_REQUEST_SIZE * control.limits.request_concurrency {
            flush_ingest_batch(
                store,
                embedder,
                &mut pending,
                &mut changed,
                &mut unchanged,
                source,
                control,
            )
            .await?;
            pending_chunks = 0;
        }
    }
    flush_ingest_batch(
        store,
        embedder,
        &mut pending,
        &mut changed,
        &mut unchanged,
        source,
        control,
    )
    .await?;
    println!("ingested changed={changed} unchanged={unchanged}");
    Ok(())
}

async fn flush_ingest_batch(
    store: &Store,
    embedder: &dyn Embedder,
    pending: &mut Vec<(Document, Vec<String>)>,
    changed: &mut usize,
    unchanged: &mut usize,
    source: &str,
    control: &SourceControl<'_>,
) -> Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    control.check(source)?;
    let input = pending
        .iter()
        .flat_map(|(_, texts)| texts.iter().cloned())
        .collect::<Vec<_>>();
    let embedding_requests = stream::iter(input.chunks(EMBEDDING_REQUEST_SIZE))
        .map(|request| embedder.embed(request))
        .buffered(control.limits.request_concurrency)
        .try_collect::<Vec<_>>();
    let remaining = control.remaining(source)?;
    let cancellation = wait_for_cancellation(control.cancellation);
    let batches = tokio::select! {
        result = tokio::time::timeout(remaining, embedding_requests) => {
            result
                .with_context(|| format!("source {source} exceeded its duration budget during embedding"))??
        }
        () = cancellation => {
            anyhow::bail!("source {source} cancelled during embedding before reconciliation")
        }
    };
    control.check(source)?;
    let all_vectors = batches.into_iter().flatten().collect::<Vec<_>>();
    let mut vectors = all_vectors.into_iter();
    for (document, texts) in pending.drain(..) {
        let chunks = texts
            .into_iter()
            .map(|text| {
                let vector = vectors
                    .next()
                    .context("embedding provider returned too few vectors")?;
                Ok((text, vector))
            })
            .collect::<Result<Vec<_>>>()?;
        if store.upsert(&document, &chunks)? {
            *changed += 1;
        } else {
            *unchanged += 1;
        }
    }
    anyhow::ensure!(
        vectors.next().is_none(),
        "embedding provider returned too many vectors"
    );
    tracing::info!(
        changed = *changed,
        unchanged = *unchanged,
        "ingestion batch committed"
    );
    Ok(())
}

async fn wait_for_cancellation(cancellation: &Cancellation) {
    while !cancellation.is_requested() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn sync_configured_sources(
    config: &Config,
    store: &Store,
    embedder: &dyn Embedder,
    selected: Option<&str>,
    reconcile: bool,
    overrides: SyncOverrides,
    cancellation: &Cancellation,
) -> Result<()> {
    cleanup_connector_spools(&config.data_dir)?;
    let sources = config
        .sources
        .iter()
        .filter(|source| source.enabled && selected.is_none_or(|name| source.name == name))
        .collect::<Vec<_>>();
    if sources.is_empty() {
        anyhow::bail!("no enabled configured sources matched the selection");
    }
    let mut failures = Vec::new();
    for source in sources {
        let limits = SourceLimits::resolve(config, source, overrides)?;
        let control = SourceControl {
            limits,
            started: Instant::now(),
            cancellation,
        };
        let canonical_source = canonical_source(source);
        let run_id = store.begin_sync(
            &canonical_source,
            &source.project,
            limits.max_documents,
            limits.max_bytes,
            limits.max_seconds,
        )?;
        let result = async {
            let scope =
                sync_source_documents(config, store, embedder, source, &control, reconcile).await?;
            control.check(&source.name)?;
            let deleted = if reconcile {
                store.reconcile(&canonical_source, &source.project, &scope.seen)?
            } else {
                0
            };
            Ok::<_, anyhow::Error>((scope, deleted))
        }
        .await;
        match result {
            Ok((scope, deleted)) => {
                store.finish_sync(
                    &run_id,
                    SyncRunStatus::Succeeded,
                    Some(scope.documents),
                    Some(scope.bytes),
                    Some(deleted),
                )?;
                println!("synced source={} deleted={deleted}", source.name);
            }
            Err(error) => {
                store.finish_sync(&run_id, failure_status(&error), None, None, None)?;
                eprintln!("source sync failed: source={} error={error:#}", source.name);
                failures.push(source.name.clone());
            }
        }
    }
    anyhow::ensure!(
        failures.is_empty(),
        "source sync failed for: {}",
        failures.join(", ")
    );
    Ok(())
}

fn failure_status(error: &anyhow::Error) -> SyncRunStatus {
    let message = format!("{error:#}").to_lowercase();
    if is_cancelled_status(&message) {
        SyncRunStatus::Cancelled
    } else if is_budget_exceeded(&message) {
        SyncRunStatus::BudgetExceeded
    } else {
        SyncRunStatus::Failed
    }
}

fn is_cancelled_status(message: &str) -> bool {
    message.contains("cancel")
}

fn is_budget_exceeded(message: &str) -> bool {
    const BUDGET_EXCEEDED_MARKERS: [&str; 6] = [
        "budget",
        "safety bound",
        "timed out after",
        "timed out before",
        "exceeded duration budget",
        "connector timed out",
    ];

    BUDGET_EXCEEDED_MARKERS
        .into_iter()
        .any(|marker| message.contains(marker))
}
fn cleanup_connector_spools(data_dir: &std::path::Path) -> Result<usize> {
    let staging = prepare_connector_staging(data_dir, false)?;
    if !staging.is_dir() {
        return Ok(0);
    }
    let mut removed = 0;
    for entry in std::fs::read_dir(&staging)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type()?.is_file()
            && name.starts_with("connector-")
            && (name.ends_with(".jsonl") || name.ends_with(".stderr"))
        {
            std::fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(removed, "removed stale connector spools");
    }
    Ok(removed)
}

async fn sync_source_documents(
    config: &Config,
    store: &Store,
    embedder: &dyn Embedder,
    source: &SourceConfig,
    control: &SourceControl<'_>,
    reconcile: bool,
) -> Result<SyncScope> {
    if source.kind == "filesystem" {
        let root = source
            .root
            .as_ref()
            .with_context(|| format!("source {} requires root", source.name))?;
        let plan = connectors::filesystem_plan(
            root,
            &source.exclude,
            control.limits.max_documents,
            control.limits.max_bytes,
            control.remaining(&source.name)?,
            !reconcile,
        )?;
        tracing::info!(
            source = source.name,
            documents = plan.documents,
            bytes = plan.bytes,
            complete = plan.complete,
            "filesystem source passed ingestion preflight"
        );
        control.check(&source.name)?;
        let documents = connectors::filesystem_document_iter(
            root,
            source.source.as_deref().unwrap_or(&source.name),
            &source.project,
            &source.exclude,
        )?;
        let mut seen = Vec::new();
        let mut batch = Vec::with_capacity(control.limits.batch_capacity());
        let mut content_bytes = 0_u64;
        for document in documents {
            control.check(&source.name)?;
            let mut document = document?;
            normalize_documents(std::slice::from_mut(&mut document), source);
            let document_bytes = u64::try_from(document.content.len()).unwrap_or(u64::MAX);
            if reconcile {
                content_bytes = content_bytes.saturating_add(document_bytes);
                anyhow::ensure!(
                    seen.len() < control.limits.max_documents,
                    "source {} exceeds the {} document budget",
                    source.name,
                    control.limits.max_documents
                );
                anyhow::ensure!(
                    content_bytes <= control.limits.max_bytes,
                    "source {} exceeds the {} byte budget",
                    source.name,
                    control.limits.max_bytes
                );
            } else if seen.len() >= control.limits.max_documents
                || content_bytes.saturating_add(document_bytes) > control.limits.max_bytes
            {
                // A bounded non-reconciling run stops at its budgets instead
                // of failing, mirroring the capped connector snapshot below:
                // the partial snapshot never deletes records absent from it.
                break;
            } else {
                content_bytes = content_bytes.saturating_add(document_bytes);
            }
            seen.push(document.source_id.clone());
            batch.push(document);
            if batch.len() >= control.limits.document_batch_size {
                ingest_documents_controlled(
                    store,
                    embedder,
                    std::mem::take(&mut batch),
                    &source.name,
                    control,
                )
                .await?;
            }
        }
        if !batch.is_empty() {
            ingest_documents_controlled(store, embedder, batch, &source.name, control).await?;
        }
        return Ok(SyncScope {
            documents: seen.len(),
            bytes: content_bytes,
            seen,
        });
    }

    // A capped connector snapshot is partial by definition, so upstream
    // document limiting is only ever applied to runs that will not reconcile.
    // Reconciliation runs keep the uncapped full snapshot so the fail-closed
    // validation below still rejects a source that exceeds its budget instead
    // of silently truncating and deleting the remainder of the index.
    let document_cap = (!reconcile).then_some(control.limits.max_documents);
    let (spool, diagnostics) =
        run_connector_to_spool(config, source, control, document_cap, false).await?;
    let result = async {
        let scope = validate_connector_spool(&spool, source, control)?;
        let reader = std::io::BufReader::new(
            std::fs::File::open(&spool)
                .with_context(|| format!("failed to open {}", spool.display()))?,
        );
        let mut seen = Vec::new();
        let mut batch = Vec::with_capacity(control.limits.batch_capacity());
        for line in reader.lines() {
            control.check(&source.name)?;
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let mut document: Document = serde_json::from_str(&line).with_context(|| {
                format!("connector {} emitted invalid Document JSONL", source.name)
            })?;
            normalize_documents(std::slice::from_mut(&mut document), source);
            seen.push(document.source_id.clone());
            batch.push(document);
            if batch.len() >= control.limits.document_batch_size {
                ingest_documents_controlled(
                    store,
                    embedder,
                    std::mem::take(&mut batch),
                    &source.name,
                    control,
                )
                .await?;
            }
        }
        if !batch.is_empty() {
            ingest_documents_controlled(store, embedder, batch, &source.name, control).await?;
        }
        Ok(SyncScope { seen, ..scope })
    }
    .await;
    let _ = std::fs::remove_file(&spool);
    let _ = std::fs::remove_file(&diagnostics);
    result
}

struct SyncScope {
    seen: Vec<String>,
    documents: usize,
    bytes: u64,
}

fn validate_connector_spool(
    spool: &std::path::Path,
    source: &SourceConfig,
    control: &SourceControl<'_>,
) -> Result<SyncScope> {
    let maximum_spool_bytes = maximum_connector_spool_bytes(&control.limits);
    let spool_bytes = std::fs::metadata(spool)?.len();
    anyhow::ensure!(
        spool_bytes <= maximum_spool_bytes,
        "source {} spool exceeds the {} byte safety bound",
        source.name,
        maximum_spool_bytes
    );
    let reader = std::io::BufReader::new(std::fs::File::open(spool)?);
    let mut documents = 0_usize;
    let mut content_bytes = 0_u64;
    for line in reader.lines() {
        control.check(&source.name)?;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let document: Document = serde_json::from_str(&line)
            .with_context(|| format!("connector {} emitted invalid Document JSONL", source.name))?;
        documents = documents.saturating_add(1);
        content_bytes =
            content_bytes.saturating_add(u64::try_from(document.content.len()).unwrap_or(u64::MAX));
        anyhow::ensure!(
            documents <= control.limits.max_documents,
            "source {} exceeds the {} document budget",
            source.name,
            control.limits.max_documents
        );
        anyhow::ensure!(
            content_bytes <= control.limits.max_bytes,
            "source {} exceeds the {} byte budget",
            source.name,
            control.limits.max_bytes
        );
    }
    tracing::info!(
        source = source.name,
        documents,
        content_bytes,
        "connector source passed ingestion preflight"
    );
    Ok(SyncScope {
        seen: Vec::new(),
        documents,
        bytes: content_bytes,
    })
}

async fn run_connector_to_spool(
    config: &Config,
    source: &SourceConfig,
    control: &SourceControl<'_>,
    document_cap: Option<usize>,
    no_cache: bool,
) -> Result<(PathBuf, PathBuf)> {
    let staging = prepare_connector_staging(&config.data_dir, true)?;
    let identifier = uuid::Uuid::new_v4();
    let spool = staging.join(format!("connector-{identifier}.jsonl"));
    let diagnostics = staging.join(format!("connector-{identifier}.stderr"));
    let stdout = private_file(&spool)?;
    let stderr = private_file(&diagnostics)?;
    let mut command = configured_connector_command(config, source, document_cap, no_cache)?;
    let executable = command.remove(0);
    let mut process = ProcessCommand::new(&executable);
    process
        .args(&command)
        .envs(&config.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    if std::env::var_os("CORTANA_DESKTOP_PROCESS_GROUP").is_none() {
        // Normal CLI/service runs isolate the connector so cancellation can
        // terminate helpers without touching the caller's process group.
        process.process_group(0);
    }
    let child = process
        .spawn()
        .with_context(|| format!("failed to run connector command {executable}"));
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            let _ = std::fs::remove_file(&spool);
            let _ = std::fs::remove_file(&diagnostics);
            return Err(error);
        }
    };
    let timeout = Duration::from_secs(config.connectors.timeout_seconds.max(1))
        .min(control.remaining(&source.name)?);
    let maximum_spool_bytes = maximum_connector_spool_bytes(&control.limits);
    const MAXIMUM_DIAGNOSTIC_BYTES: u64 = 16 * 1024 * 1024;
    let started = std::time::Instant::now();
    let status = loop {
        if control.cancellation.is_requested() {
            terminate_connector(&mut child).await;
            let _ = std::fs::remove_file(&spool);
            let _ = std::fs::remove_file(&diagnostics);
            anyhow::bail!("connector {} cancelled before reconciliation", source.name);
        }
        let spool_bytes = std::fs::metadata(&spool).map_or(0, |metadata| metadata.len());
        let diagnostic_bytes = std::fs::metadata(&diagnostics).map_or(0, |metadata| metadata.len());
        if spool_bytes > maximum_spool_bytes || diagnostic_bytes > MAXIMUM_DIAGNOSTIC_BYTES {
            terminate_connector(&mut child).await;
            let _ = std::fs::remove_file(&spool);
            let _ = std::fs::remove_file(&diagnostics);
            anyhow::bail!(
                "connector {} exceeded its live output safety bound",
                source.name
            );
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            terminate_connector(&mut child).await;
            let _ = std::fs::remove_file(&spool);
            let _ = std::fs::remove_file(&diagnostics);
            anyhow::bail!(
                "connector {} timed out after {} seconds",
                source.name,
                timeout.as_secs()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    if !status.success() {
        let message = std::fs::read_to_string(&diagnostics).unwrap_or_default();
        let _ = std::fs::remove_file(&spool);
        let _ = std::fs::remove_file(&diagnostics);
        anyhow::bail!(
            "connector {} failed: {}",
            source.name,
            message.trim().chars().take(16_384).collect::<String>()
        );
    }
    Ok((spool, diagnostics))
}

async fn terminate_connector(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id().filter(|pid| *pid > 0 && *pid <= i32::MAX as u32) {
        // Normal CLI/service runs make the connector a process-group leader,
        // so a negative PID terminates helpers spawned by it too. Desktop
        // source jobs inherit their wrapper's isolated group and are killed
        // as a unit by the native Desktop cancellation path instead.
        unsafe {
            let _ = libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn prepare_connector_staging(data_dir: &std::path::Path, create: bool) -> Result<PathBuf> {
    reject_symlink_path(data_dir)?;
    let staging = data_dir.join("staging");
    reject_symlink_path(&staging)?;
    if create {
        std::fs::create_dir_all(&staging)?;
    }
    for path in [data_dir, staging.as_path()] {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("inspect connector staging directory {}", path.display())
                });
            }
        };
        anyhow::ensure!(
            metadata.is_dir(),
            "connector staging path is not a directory: {}",
            path.display()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            anyhow::ensure!(
                metadata.uid() == unsafe { libc::geteuid() },
                "connector staging path is not owned by the current user: {}",
                path.display()
            );
            if create {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
            }
        }
    }
    Ok(staging)
}

fn reject_symlink_path(path: &std::path::Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "refusing to use symlinked connector staging path {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn maximum_connector_spool_bytes(limits: &SourceLimits) -> u64 {
    limits.max_bytes.saturating_add(
        u64::try_from(limits.max_documents)
            .unwrap_or(u64::MAX)
            .saturating_mul(64 * 1024),
    )
}

fn private_file(path: &std::path::Path) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    configure_no_follow(&mut options);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("failed to create private spool {}", path.display()))
}

#[cfg(unix)]
fn configure_no_follow(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
}

#[cfg(not(unix))]
fn configure_no_follow(_options: &mut std::fs::OpenOptions) {}

fn configured_connector_command(
    config: &Config,
    source: &SourceConfig,
    max_documents: Option<usize>,
    no_cache: bool,
) -> Result<Vec<String>> {
    let command = if source.kind == "external" {
        // Arbitrary external commands keep the plain contract: one JSON
        // object per line on stdout, no connector-specific flags appended.
        anyhow::ensure!(
            !source.command.is_empty(),
            "external source {} requires command",
            source.name
        );
        source.command.clone()
    } else {
        let mut command = config.connectors.command.clone();
        command.extend([
            "--project".into(),
            source.project.clone(),
            "--cache-dir".into(),
            config
                .data_dir
                .join("connector-cache")
                .join(&source.name)
                .display()
                .to_string(),
        ]);
        if let Some(max_documents) = max_documents {
            // Root-level connector options and must precede the subcommand for
            // argparse to accept them. Validation disables persistent caches
            // because a read-only probe must not mutate a partial snapshot;
            // bounded sync passes the same cap without --no-cache so real
            // ingestion still reads and extends the derived caches.
            if no_cache {
                command.push("--no-cache".into());
            }
            command.extend(["--max-documents".into(), max_documents.to_string()]);
        }
        command.push(source.kind.clone());
        connector_arguments(&mut command, source)?;
        if let Some(max_documents) = max_documents {
            // Drive also needs an explicit subcommand cap because it downloads
            // a whole listing page before yielding JSONL; without this, a
            // bounded run can fetch 1,000 files and hit the wall-clock or live
            // output safety bound before the first permitted document is
            // consumed.
            if source.kind == "google-drive" {
                command.extend(["--max-documents".into(), max_documents.to_string()]);
            }
        }
        command
    };
    anyhow::ensure!(
        !command.is_empty(),
        "connector command for {} is empty",
        source.name
    );
    Ok(command)
}

fn normalize_documents(documents: &mut [Document], source: &SourceConfig) {
    let canonical = canonical_source(source);
    for document in documents {
        let connector_kind = document.source.clone();
        document.source.clone_from(&canonical);
        document.project.clone_from(&source.project);
        if document.acl.is_empty() {
            document.acl = source.effective_acl();
        }
        if !document.metadata.is_object() {
            document.metadata = serde_json::json!({});
        }
        let metadata = document
            .metadata
            .as_object_mut()
            .expect("metadata was normalized to an object");
        metadata
            .entry("connector_kind")
            .or_insert(serde_json::Value::String(connector_kind));
        metadata
            .entry("configured_source")
            .or_insert(serde_json::Value::String(source.name.clone()));
    }
}

fn connector_arguments(command: &mut Vec<String>, source: &SourceConfig) -> Result<()> {
    if let Some(root) = &source.root {
        command.extend(["--root".into(), root.display().to_string()]);
    }
    for channel in &source.channels {
        command.extend(["--channel".into(), channel.clone()]);
    }
    for repository in &source.repositories {
        command.extend(["--repo".into(), repository.clone()]);
    }
    if let Some(token_env) = &source.token_env {
        command.extend(["--token-env".into(), token_env.clone()]);
    }
    if let Some(token) = &source.token {
        command.extend(["--token".into(), token.display().to_string()]);
    }
    if let Some(query) = &source.query {
        command.extend(["--query".into(), query.clone()]);
    }
    for label in &source.labels {
        command.extend(["--label".into(), label.clone()]);
    }
    if let Some(max_content_chars) = source.max_content_chars {
        anyhow::ensure!(
            source.kind == "google-drive",
            "source {} only supports max_content_chars for google-drive",
            source.name
        );
        anyhow::ensure!(
            max_content_chars > 0,
            "source {} requires max_content_chars greater than zero",
            source.name
        );
        command.extend(["--max-content-chars".into(), max_content_chars.to_string()]);
    }
    anyhow::ensure!(
        !matches!(source.kind.as_str(), "slack" | "discord") || !source.channels.is_empty(),
        "source {} requires at least one channel",
        source.name
    );
    anyhow::ensure!(
        source.kind != "github" || !source.repositories.is_empty(),
        "source {} requires at least one GitHub repository",
        source.name
    );
    Ok(())
}

fn canonical_source(source: &SourceConfig) -> String {
    source.source.as_deref().unwrap_or(&source.name).into()
}

async fn search(
    store: &Store,
    embedder: &dyn Embedder,
    query: &str,
    project: Option<&str>,
    source: Option<&str>,
    limit: usize,
) -> Result<()> {
    let embedding = embedder.embed(&[query.to_string()]).await?.remove(0);
    let evidence = retrieval::search(store, query, &embedding, project, source, limit)?;
    let mut stdout = std::io::BufWriter::new(std::io::stdout());
    serde_json::to_writer_pretty(&mut stdout, &evidence)?;
    writeln!(stdout)?;
    Ok(())
}

async fn context_bundle(
    store: &Store,
    embedder: &Arc<dyn Embedder>,
    query: &str,
    project: Option<&str>,
    source: Option<&str>,
    limit: usize,
    max_tokens: usize,
) -> Result<ContextBundle> {
    let evidence = retrieval::retrieve(store, embedder, query, project, source, limit).await?;
    Ok(context::build(query, &evidence, max_tokens))
}

/// Metadata-only audit trail for CLI context requests. The query text and
/// evidence content are never written, and an unavailable audit store never
/// fails the command.
fn record_cli_context_audit(
    store: &Store,
    max_events: usize,
    project: Option<&str>,
    source: Option<&str>,
    outcome: &str,
    result_count: Option<usize>,
    started: Instant,
) {
    if let Err(error) = store.record_audit(
        "local-cli",
        "local-cli/context",
        project,
        source,
        outcome,
        result_count,
        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        max_events,
    ) {
        tracing::warn!(%error, "CLI context audit write failed");
    }
}

fn chunk(content: &str) -> Vec<String> {
    const TARGET: usize = 1_600;
    const OVERLAP: usize = 200;
    let mut output = Vec::new();
    let mut start = 0;
    while start < content.len() {
        while start < content.len() && !content.is_char_boundary(start) {
            start += 1;
        }
        let hard_end = (start + TARGET).min(content.len());
        let mut end = hard_end;
        while end > start && !content.is_char_boundary(end) {
            end -= 1;
        }
        if end < content.len() {
            let window = &content[start..end];
            let preferred_floor = window.len() / 2;
            end = window
                .rfind("\n\n")
                .filter(|position| *position >= preferred_floor)
                .map(|position| start + position + 2)
                .or_else(|| {
                    window
                        .rfind('\n')
                        .filter(|position| *position >= preferred_floor)
                        .map(|position| start + position + 1)
                })
                .unwrap_or(end);
        }
        let text = content[start..end].trim();
        if !text.is_empty() {
            output.push(text.to_string());
        }
        if end == content.len() {
            break;
        }
        let mut next = end.saturating_sub(OVERLAP);
        while next < end && !content.is_char_boundary(next) {
            next += 1;
        }
        if next <= start {
            next = end;
        }
        start = next;
    }
    output
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use anyhow::Result;
    use async_trait::async_trait;
    use chrono::Utc;
    use clap::Parser;

    use super::{
        AclAction, Cancellation, Cli, Command, DEFAULT_CONTEXT_LIMIT, SourceControl, SourceLimits,
        SyncLock, SyncOverrides, SyncRunStatus, chunk, cleanup_connector_spools,
        configured_connector_command, context_bundle, ensure_recurring_sync_validated,
        failure_status, ingest_documents, is_budget_exceeded, private_file,
        require_sync_validation, run_connector_to_spool, validate_configured_source,
        validate_connector_spool, validation_overrides,
    };
    use cortana::config::{Config, SourceConfig};
    use cortana::embed::{DeterministicEmbedder, Embedder};
    use cortana::model::Document;
    use cortana::source_validation::{SourceValidationStatus, configuration_fingerprint, record};
    use cortana::store::Store;

    #[test]
    fn acl_plan_can_include_unmapped_public_projects_in_quarantine() {
        let cli = Cli::try_parse_from([
            "cortana",
            "acl",
            "plan",
            "--project",
            "work=work",
            "--quarantine-unmapped",
        ])
        .expect("ACL plan command");
        match cli.command {
            Some(Command::Acl {
                action:
                    AclAction::Plan {
                        projects,
                        quarantine_unmapped,
                    },
            }) => {
                assert_eq!(projects, vec!["work=work"]);
                assert!(quarantine_unmapped);
            }
            _ => panic!("expected the ACL plan subcommand"),
        }
    }

    #[test]
    fn validate_source_defaults_to_safe_read_only_bounds() {
        let defaults = validation_overrides(None, None, None);
        assert_eq!(defaults.max_documents, Some(25));
        assert_eq!(defaults.max_bytes, Some(5 * 1024 * 1024));
        assert_eq!(defaults.max_seconds, Some(60));

        let explicit = validation_overrides(Some(100), Some(64 * 1024 * 1024), Some(900));
        assert_eq!(explicit.max_documents, Some(100));
        assert_eq!(explicit.max_bytes, Some(64 * 1024 * 1024));
        assert_eq!(explicit.max_seconds, Some(900));
    }

    #[test]
    fn failure_status_categorizes_sync_errors_by_retry_profile() {
        use anyhow::anyhow;

        match failure_status(&anyhow!("operation cancelled by operator")) {
            SyncRunStatus::Cancelled => {}
            _ => panic!("expected cancelled classification"),
        }
        match failure_status(&anyhow!("connector upstream timed out after 300 seconds")) {
            SyncRunStatus::BudgetExceeded => {}
            _ => panic!("expected budget_exceeded classification"),
        }
        match failure_status(&anyhow!("source work-doc exceeded the 60 second budget")) {
            SyncRunStatus::BudgetExceeded => {}
            _ => panic!("expected budget_exceeded classification"),
        };
        match failure_status(&anyhow!("unexpected ingestion pipeline failure")) {
            SyncRunStatus::Failed => {}
            _ => panic!("expected failed classification"),
        };
    }

    #[test]
    fn budget_exceeded_markers_are_narrow_and_stable() {
        assert!(is_budget_exceeded(
            "connector sync timed out after 120 seconds"
        ));
        assert!(is_budget_exceeded(
            "source-work budget exceeded during reconcile"
        ));
        assert!(!is_budget_exceeded("source-work was cancelled by user"));
        assert!(!is_budget_exceeded("network retryable connection reset"));
    }

    #[test]
    fn batch_capacity_is_bounded_by_the_document_budget() {
        // An arbitrarily large configured batch size must never become an
        // oversized preallocation on a run with a small document budget.
        let tiny_budget = SourceLimits {
            max_documents: 1,
            max_bytes: 1024,
            max_seconds: 60,
            document_batch_size: usize::MAX,
            request_concurrency: 1,
        };
        assert_eq!(tiny_budget.batch_capacity(), 1);

        // max_documents=10 with a batch size larger than the cap: the batch
        // preallocates for the cap, never beyond it.
        let bounded = SourceLimits {
            max_documents: 10,
            max_bytes: 1024,
            max_seconds: 60,
            document_batch_size: 16,
            request_concurrency: 1,
        };
        assert_eq!(bounded.batch_capacity(), 10);

        // Capacity is only an allocation hint: when the budget permits, the
        // full configured batch size is still used for preallocation.
        let unbounded = SourceLimits {
            max_documents: usize::MAX,
            max_bytes: u64::MAX,
            max_seconds: 3600,
            document_batch_size: 16,
            request_concurrency: 1,
        };
        assert_eq!(unbounded.batch_capacity(), 16);
    }

    #[tokio::test]
    async fn connector_spool_validation_rejects_snapshots_over_the_document_budget() {
        // Reconciliation runs validate the full uncapped spool and must keep
        // failing closed when a source exceeds its document budget instead of
        // silently truncating and deleting the remainder of the index.
        let directory = tempfile::tempdir().expect("temporary directory");
        let spool = directory.path().join("connector.jsonl");
        let mut lines = String::new();
        for index in 0..11 {
            let document = Document {
                source: "connector".into(),
                source_id: format!("doc-{index}"),
                title: "Document".into(),
                content: "body".into(),
                uri: None,
                updated_at: Utc::now(),
                project: "work".into(),
                acl: Vec::new(),
                metadata: serde_json::json!({}),
            };
            lines.push_str(&serde_json::to_string(&document).expect("serialize"));
            lines.push('\n');
        }
        std::fs::write(&spool, lines).expect("spool");

        let source = SourceConfig {
            name: "over-budget".into(),
            kind: "external".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            repositories: Vec::new(),
            token_env: None,
            token: None,
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        };
        let cancellation = Cancellation::inert();
        let control = SourceControl {
            limits: SourceLimits {
                max_documents: 10,
                max_bytes: 1024 * 1024,
                max_seconds: 60,
                document_batch_size: 16,
                request_concurrency: 1,
            },
            started: std::time::Instant::now(),
            cancellation: &cancellation,
        };

        let error = validate_connector_spool(&spool, &source, &control)
            .err()
            .expect("over-budget spool must be rejected");
        assert!(
            format!("{error:#}").contains("document budget"),
            "unexpected error: {error:#}"
        );
        cancellation.stop();
    }

    #[test]
    fn bounded_validation_caps_builtin_drive_without_mutating_cache() {
        let config = Config::default();
        let source = SourceConfig {
            name: "work-drive".into(),
            kind: "google-drive".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            repositories: Vec::new(),
            token_env: None,
            token: Some("/tmp/google-token.json".into()),
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        };

        let command = configured_connector_command(&config, &source, Some(25), true)
            .expect("bounded connector command");
        let no_cache = command
            .iter()
            .position(|argument| argument == "--no-cache")
            .expect("validation must disable persistent caches");
        let subcommand = command
            .iter()
            .position(|argument| argument == "google-drive")
            .expect("Drive connector subcommand");
        assert!(no_cache < subcommand);
        assert_eq!(
            command
                .windows(2)
                .find(|window| window[0] == "--max-documents")
                .map(|window| window[1].as_str()),
            Some("25")
        );
    }

    #[test]
    fn bounded_sync_caps_builtin_drive_without_no_cache() {
        let config = Config::default();
        let source = SourceConfig {
            name: "work-drive".into(),
            kind: "google-drive".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            repositories: Vec::new(),
            token_env: None,
            token: Some("/tmp/google-token.json".into()),
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        };

        let command = configured_connector_command(&config, &source, Some(1), false)
            .expect("bounded sync connector command");
        assert!(
            command.iter().all(|argument| argument != "--no-cache"),
            "a bounded sync must not disable persistent caches"
        );
        assert_eq!(
            command
                .windows(2)
                .filter(|window| window[0] == "--max-documents")
                .map(|window| window[1].as_str())
                .collect::<Vec<_>>(),
            vec!["1", "1"],
            "the cap must be passed both at root level and to the Drive subcommand"
        );
        let subcommand = command
            .iter()
            .position(|argument| argument == "google-drive")
            .expect("Drive connector subcommand");
        assert!(
            command[subcommand + 1..]
                .windows(2)
                .any(|window| window[0] == "--max-documents" && window[1] == "1"),
            "the Drive subcommand cap must follow the subcommand"
        );
    }

    #[test]
    fn github_connector_commands_include_only_explicit_repositories() {
        let config = Config::default();
        let source = SourceConfig {
            name: "work-github".into(),
            kind: "github".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            repositories: vec!["acme/one".into(), "acme/two".into()],
            token_env: Some("GITHUB_TOKEN".into()),
            token: None,
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        };
        let command = configured_connector_command(&config, &source, Some(25), true)
            .expect("GitHub connector command");
        assert!(
            command
                .windows(2)
                .any(|pair| pair[0] == "--repo" && pair[1] == "acme/one")
        );
        assert!(
            command
                .windows(2)
                .any(|pair| pair[0] == "--repo" && pair[1] == "acme/two")
        );
        assert!(
            command
                .windows(2)
                .any(|pair| pair[0] == "--token-env" && pair[1] == "GITHUB_TOKEN")
        );
        assert!(command.iter().any(|argument| argument == "github"));
    }

    #[test]
    fn external_connector_commands_never_receive_budget_flags() {
        let config = Config::default();
        let source = SourceConfig {
            name: "external-demo".into(),
            kind: "external".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            repositories: Vec::new(),
            token_env: None,
            token: None,
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: vec![
                "/usr/bin/upstream-sync".into(),
                "--profile".into(),
                "prod".into(),
            ],
            acl: Vec::new(),
        };

        let command = configured_connector_command(&config, &source, Some(25), true)
            .expect("external connector command");
        assert_eq!(
            command, source.command,
            "external commands must keep their exact invocation"
        );
    }

    #[test]
    fn recurring_sync_requires_current_validation_for_each_enabled_source() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let mut config = Config {
            data_dir: directory.path().to_path_buf(),
            ..Config::default()
        };
        config.sources.push(SourceConfig {
            name: "work-code".into(),
            kind: "filesystem".into(),
            enabled: true,
            project: "work".into(),
            root: Some(directory.path().join("code")),
            source: Some("work-code".into()),
            channels: Vec::new(),
            servers: Vec::new(),
            repositories: Vec::new(),
            token_env: None,
            token: None,
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        });

        let error = ensure_recurring_sync_validated(&config)
            .expect_err("missing source validation must block recurring sync");
        assert!(error.to_string().contains("work-code"));
        assert!(error.to_string().contains("current successful validation"));
    }

    #[test]
    fn recurring_sync_requires_a_fresh_validation_not_just_a_successful_one() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let mut config = Config {
            data_dir: directory.path().to_path_buf(),
            ..Config::default()
        };
        let source = SourceConfig {
            name: "work-code".into(),
            kind: "filesystem".into(),
            enabled: true,
            project: "work".into(),
            root: Some(directory.path().join("code")),
            source: Some("work-code".into()),
            channels: Vec::new(),
            servers: Vec::new(),
            repositories: Vec::new(),
            token_env: None,
            token: None,
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        };
        config.sources.push(source.clone());
        let limits = SourceLimits::resolve(&config, &source, SyncOverrides::default())
            .expect("resolved budgets");
        let mut status = cortana::source_validation::SourceValidationStatus {
            source: source.name.clone(),
            project: source.project.clone(),
            kind: source.kind.clone(),
            status: "succeeded".into(),
            validated_at: chrono::Utc::now() - chrono::Duration::days(30),
            documents: Some(1),
            bytes: Some(8),
            max_documents: limits.max_documents,
            max_bytes: limits.max_bytes,
            max_seconds: limits.max_seconds,
            configuration_fingerprint: Some(
                cortana::source_validation::configuration_fingerprint(&source).unwrap(),
            ),
            complete: None,
            error: None,
        };
        cortana::source_validation::record(directory.path(), status.clone())
            .expect("lapsed validation");

        let error = ensure_recurring_sync_validated(&config)
            .expect_err("a 30-day-old validation must not bless recurring sync");
        let message = format!("{error:#}");
        assert!(message.contains("30 days old"));
        assert!(message.contains("re-run validate-source"));

        status.validated_at = chrono::Utc::now();
        cortana::source_validation::record(directory.path(), status).expect("fresh validation");
        ensure_recurring_sync_validated(&config)
            .expect("a fresh validation must bless recurring sync");
    }

    fn filesystem_source(name: &str) -> SourceConfig {
        SourceConfig {
            name: name.into(),
            kind: "filesystem".into(),
            enabled: true,
            project: "work".into(),
            root: Some(std::env::temp_dir()),
            source: Some(name.into()),
            channels: Vec::new(),
            servers: Vec::new(),
            repositories: Vec::new(),
            token_env: None,
            token: None,
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: Some(25),
            max_bytes: Some(1024),
            max_duration_seconds: Some(60),
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        }
    }

    fn record_success(
        data_dir: &std::path::Path,
        source: &SourceConfig,
        max_documents: usize,
        max_bytes: u64,
        max_seconds: u64,
    ) {
        record(
            data_dir,
            SourceValidationStatus {
                source: source.name.clone(),
                project: source.project.clone(),
                kind: source.kind.clone(),
                status: "succeeded".into(),
                validated_at: Utc::now(),
                documents: Some(1),
                bytes: Some(64),
                max_documents,
                max_bytes,
                max_seconds,
                configuration_fingerprint: Some(configuration_fingerprint(source).unwrap()),
                complete: None,
                error: None,
            },
        )
        .expect("record validation");
    }

    #[test]
    fn guarded_sync_without_source_rejects_missing_or_stale_validations() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let mut config = Config {
            data_dir: directory.path().to_path_buf(),
            ..Config::default()
        };
        let current = filesystem_source("work-code");
        let stale = filesystem_source("notes");
        config.sources.push(current.clone());
        config.sources.push(stale.clone());

        // The scheduled guard must fail as soon as any enabled source lacks a
        // current successful validation.
        let error = require_sync_validation(&config, None, SyncOverrides::default(), true)
            .expect_err("guarded run must reject a missing validation");
        assert!(format!("{error:#}").contains("work-code"));
        assert!(format!("{error:#}").contains("has not been validated"));

        record_success(directory.path(), &current, 25, 1024, 60);
        // The second source was validated at budgets below its configured limits.
        record_success(directory.path(), &stale, 10, 512, 30);
        let error = require_sync_validation(&config, None, SyncOverrides::default(), true)
            .expect_err("guarded run must reject a stale validation budget");
        assert!(format!("{error:#}").contains("notes"));
        assert!(format!("{error:#}").contains("smaller"));
    }

    #[test]
    fn guarded_sync_without_source_passes_when_validations_cover_resolved_limits() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let mut config = Config {
            data_dir: directory.path().to_path_buf(),
            ..Config::default()
        };
        let first = filesystem_source("work-code");
        let second = filesystem_source("notes");
        config.sources.push(first.clone());
        config.sources.push(second.clone());
        record_success(directory.path(), &first, 25, 1024, 60);
        record_success(directory.path(), &second, 25, 1024, 60);
        require_sync_validation(&config, None, SyncOverrides::default(), true)
            .expect("every enabled source is current at its configured budgets");

        // Run-level budget overrides raise the required validation coverage for
        // every source, exactly like the sync run they guard.
        let overrides = SyncOverrides {
            max_documents: Some(100),
            max_bytes: Some(2048),
            max_seconds: Some(300),
        };
        let error = require_sync_validation(&config, None, overrides, true)
            .expect_err("run-level overrides must be covered by validation");
        assert!(format!("{error:#}").contains("work-code"));
        assert!(format!("{error:#}").contains("smaller"));

        record_success(directory.path(), &first, 100, 2048, 300);
        record_success(directory.path(), &second, 100, 2048, 300);
        require_sync_validation(&config, None, overrides, true)
            .expect("re-validated sources cover the run-level limits");
    }

    #[test]
    fn sampled_validation_blesses_only_equally_bounded_non_reconciling_syncs() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let mut config = Config {
            data_dir: directory.path().to_path_buf(),
            ..Config::default()
        };
        let source = filesystem_source("work-code");
        config.sources.push(source.clone());
        record(
            directory.path(),
            SourceValidationStatus {
                source: source.name.clone(),
                project: source.project.clone(),
                kind: source.kind.clone(),
                status: "succeeded".into(),
                validated_at: Utc::now(),
                documents: Some(1),
                bytes: Some(64),
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 60,
                configuration_fingerprint: Some(configuration_fingerprint(&source).unwrap()),
                complete: Some(false),
                error: None,
            },
        )
        .expect("record sampled validation");

        require_sync_validation(&config, Some("work-code"), SyncOverrides::default(), false)
            .expect("an equally bounded non-reconciling trial sync accepts a sample");

        let error =
            require_sync_validation(&config, Some("work-code"), SyncOverrides::default(), true)
                .expect_err("a reconciling sync must reject a sampled validation");
        assert!(format!("{error:#}").contains("bounded sample"));
        assert!(format!("{error:#}").contains("--sample"));

        let error = require_sync_validation(&config, None, SyncOverrides::default(), true)
            .expect_err("the all-sources gate must reject a sampled validation");
        assert!(format!("{error:#}").contains("work-code"));
    }

    #[test]
    fn validate_source_command_parses_the_sample_flag() {
        let cli = Cli::try_parse_from(["cortana", "validate-source", "work-code", "--sample"])
            .expect("validate-source with --sample");
        match cli.command.expect("command") {
            Command::ValidateSource { sample, .. } => assert!(sample),
            other => panic!("unexpected command: {other:?}"),
        }

        let cli = Cli::try_parse_from(["cortana", "validate-source", "work-code"])
            .expect("plain validate-source");
        match cli.command.expect("command") {
            Command::ValidateSource { sample, .. } => assert!(!sample),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[tokio::test]
    async fn filesystem_validation_records_a_partial_scope_only_when_sampled() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path().join("root");
        std::fs::create_dir_all(&root).expect("source root");
        std::fs::write(root.join("one.rs"), "aa").expect("first file");
        std::fs::write(root.join("two.rs"), "bb").expect("second file");
        let data_dir = directory.path().join("data");
        let mut config = Config {
            data_dir: data_dir.clone(),
            ..Config::default()
        };
        config.sources.push(SourceConfig {
            name: "work-code".into(),
            kind: "filesystem".into(),
            enabled: true,
            project: "work".into(),
            root: Some(root),
            source: Some("work-code".into()),
            channels: Vec::new(),
            servers: Vec::new(),
            repositories: Vec::new(),
            token_env: None,
            token: None,
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        });

        // A root larger than the budget fails closed without --sample.
        let error = validate_configured_source(
            &config,
            "work-code",
            validation_overrides(Some(1), None, None),
            false,
        )
        .await
        .expect_err("an oversized root must fail a plain validation");
        assert!(format!("{error:#}").contains("1 document budget"));

        // The same bounded validation records a partial sample with --sample.
        validate_configured_source(
            &config,
            "work-code",
            validation_overrides(Some(1), None, None),
            true,
        )
        .await
        .expect("a sampled validation accepts an oversized root");
        let validations = cortana::source_validation::load(&data_dir).expect("validation state");
        let record = validations
            .get("work-code")
            .expect("persisted validation record");
        assert_eq!(record.status, "succeeded");
        assert_eq!(record.documents, Some(1));
        assert_eq!(record.complete, Some(false));

        // A root that fits the budget records a complete validation even with
        // --sample, so it keeps full-corpus authority.
        validate_configured_source(
            &config,
            "work-code",
            validation_overrides(Some(10), None, None),
            true,
        )
        .await
        .expect("a sample covering the whole corpus succeeds");
        let validations = cortana::source_validation::load(&data_dir).expect("validation state");
        assert_eq!(validations["work-code"].complete, Some(true));
        assert_eq!(validations["work-code"].documents, Some(2));

        // --sample is rejected for non-filesystem kinds before any connector
        // is contacted.
        let mut remote = SourceConfig {
            kind: "google-drive".into(),
            ..config.sources[0].clone()
        };
        remote.name = "drive".into();
        config.sources.push(remote);
        let error = validate_configured_source(
            &config,
            "drive",
            validation_overrides(None, None, None),
            true,
        )
        .await
        .expect_err("--sample must be rejected for connector sources");
        assert!(format!("{error:#}").contains("does not support --sample"));
    }

    struct BatchRecordingEmbedder {
        maximum: AtomicUsize,
    }

    struct ConcurrencyRecordingEmbedder {
        active: AtomicUsize,
        maximum: AtomicUsize,
    }

    #[async_trait]
    impl Embedder for BatchRecordingEmbedder {
        async fn embed(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
            self.maximum.fetch_max(input.len(), Ordering::SeqCst);
            Ok(input.iter().map(|_| vec![1.0]).collect())
        }

        fn fingerprint(&self) -> String {
            "recording:1".into()
        }
    }

    #[async_trait]
    impl Embedder for ConcurrencyRecordingEmbedder {
        async fn embed(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(input.iter().map(|_| vec![1.0]).collect())
        }

        fn fingerprint(&self) -> String {
            "concurrency-recording:1".into()
        }

        fn request_concurrency(&self) -> usize {
            4
        }
    }

    #[test]
    fn chunk_bounds_unbroken_content_and_preserves_unicode() {
        let content = format!("{}{}", "a".repeat(5_000), "🧠".repeat(300));
        let chunks = chunk(&content);

        assert!(chunks.len() > 3);
        assert!(chunks.iter().all(|item| item.len() <= 1_600));
        assert!(chunks.iter().any(|item| item.contains('🧠')));
    }

    #[cfg(unix)]
    #[test]
    fn connector_spools_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("private.jsonl");
        drop(private_file(&path).expect("private spool"));

        let mode = std::fs::metadata(path)
            .expect("spool metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn stale_connector_spools_are_removed_without_touching_other_files() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let staging = directory.path().join("staging");
        std::fs::create_dir(&staging).expect("staging directory");
        std::fs::write(staging.join("connector-old.jsonl"), "private").expect("stale spool");
        std::fs::write(staging.join("connector-old.stderr"), "diagnostic")
            .expect("stale diagnostics");
        std::fs::write(staging.join("retain.txt"), "unrelated").expect("retained file");

        assert_eq!(
            cleanup_connector_spools(directory.path()).expect("cleanup"),
            2
        );
        assert!(staging.join("retain.txt").is_file());
        assert_eq!(
            cleanup_connector_spools(directory.path()).expect("repeat cleanup"),
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn connector_staging_rejects_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory");
        let external = directory.path().join("external");
        std::fs::create_dir(&external).expect("external staging");
        symlink(&external, directory.path().join("staging")).expect("staging symlink");

        let error = cleanup_connector_spools(directory.path())
            .expect_err("connector staging symlink must fail");
        assert!(
            error
                .to_string()
                .contains("symlinked connector staging path")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn connector_polling_does_not_block_other_async_tasks() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let mut config = Config {
            data_dir: directory.path().to_path_buf(),
            ..Config::default()
        };
        config.connectors.timeout_seconds = 2;
        let source = SourceConfig {
            name: "slow-external".into(),
            kind: "external".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            repositories: Vec::new(),
            token_env: None,
            token: None,
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: Some(1),
            max_bytes: Some(1024),
            max_duration_seconds: Some(2),
            exclude: Vec::new(),
            command: vec!["sh".into(), "-c".into(), "sleep 0.2; printf done".into()],
            acl: Vec::new(),
        };
        let cancellation = Cancellation::inert();
        let control = SourceControl {
            limits: SourceLimits {
                max_documents: 1,
                max_bytes: 1024,
                max_seconds: 2,
                document_batch_size: 1,
                request_concurrency: 1,
            },
            started: std::time::Instant::now(),
            cancellation: &cancellation,
        };
        let ticks = Arc::new(AtomicUsize::new(0));
        let tick_counter = Arc::clone(&ticks);
        let heartbeat = tokio::spawn(async move {
            for _ in 0..10 {
                tokio::time::sleep(Duration::from_millis(20)).await;
                tick_counter.fetch_add(1, Ordering::SeqCst);
            }
        });

        let result = run_connector_to_spool(&config, &source, &control, None, false).await;
        heartbeat.abort();
        cancellation.stop();

        let (spool, diagnostics) = result.expect("connector should finish");
        let _ = std::fs::remove_file(spool);
        let _ = std::fs::remove_file(diagnostics);
        assert!(
            ticks.load(Ordering::SeqCst) > 0,
            "connector polling blocked the Tokio runtime"
        );
    }

    #[tokio::test]
    async fn oversized_documents_respect_embedding_provider_batch_bound() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let embedder = BatchRecordingEmbedder {
            maximum: AtomicUsize::new(0),
        };
        let document = Document {
            source: "test".into(),
            source_id: "large".into(),
            title: "Large".into(),
            content: "large document content\n".repeat(2_000),
            uri: None,
            updated_at: Utc::now(),
            project: "test".into(),
            acl: Vec::new(),
            metadata: serde_json::json!({}),
        };

        ingest_documents(&store, &embedder, vec![document])
            .await
            .expect("ingest");

        assert_eq!(embedder.maximum.load(Ordering::SeqCst), 8);
    }

    #[tokio::test]
    async fn ingestion_uses_bounded_embedding_request_concurrency() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let embedder = ConcurrencyRecordingEmbedder {
            active: AtomicUsize::new(0),
            maximum: AtomicUsize::new(0),
        };
        let document = Document {
            source: "test".into(),
            source_id: "concurrent".into(),
            title: "Concurrent".into(),
            content: "concurrent embedding content\n".repeat(2_000),
            uri: None,
            updated_at: Utc::now(),
            project: "test".into(),
            acl: Vec::new(),
            metadata: serde_json::json!({}),
        };

        ingest_documents(&store, &embedder, vec![document])
            .await
            .expect("ingest");

        assert_eq!(embedder.maximum.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn sync_lock_prevents_overlapping_processes_and_recovers_on_drop() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("sync.lock");
        let first = SyncLock::acquire(&path).expect("first lock");

        assert!(SyncLock::acquire(&path).is_err());
        drop(first);
        SyncLock::acquire(&path).expect("lock after release");
    }

    #[cfg(unix)]
    #[test]
    fn sync_lock_rejects_symlinked_paths() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory");
        let target = directory.path().join("outside.lock");
        let link = directory.path().join("sync.lock");
        std::fs::write(&target, b"not a lock").expect("target lock");
        symlink(&target, &link).expect("lock symlink");

        let error = SyncLock::acquire(&link)
            .err()
            .expect("sync lock must not follow a symlink");
        assert!(error.to_string().contains("sync lock"));
    }

    #[test]
    fn context_command_parses_scopes_and_explicit_bounds() {
        let cli = Cli::try_parse_from([
            "cortana",
            "context",
            "how do releases work?",
            "--project",
            "engineering",
            "--source",
            "runbooks",
            "--limit",
            "5",
            "--max-tokens",
            "4096",
        ])
        .expect("context command");
        match cli.command {
            Some(Command::Context {
                query,
                project,
                source,
                limit,
                max_tokens,
            }) => {
                assert_eq!(query, "how do releases work?");
                assert_eq!(project.as_deref(), Some("engineering"));
                assert_eq!(source.as_deref(), Some("runbooks"));
                assert_eq!(limit, 5);
                assert_eq!(max_tokens, Some(4096));
            }
            _ => panic!("expected the context subcommand"),
        }
    }

    #[test]
    fn context_command_applies_contract_defaults() {
        let cli = Cli::try_parse_from(["cortana", "context", "releases"]).expect("context command");
        match cli.command {
            Some(Command::Context {
                query,
                project,
                source,
                limit,
                max_tokens,
            }) => {
                assert_eq!(query, "releases");
                assert_eq!(project, None);
                assert_eq!(source, None);
                assert_eq!(limit, DEFAULT_CONTEXT_LIMIT);
                assert_eq!(max_tokens, None);
            }
            _ => panic!("expected the context subcommand"),
        }
    }

    #[test]
    fn context_command_rejects_out_of_contract_bounds() {
        for limit in ["0", "51"] {
            let error = Cli::try_parse_from(["cortana", "context", "query", "--limit", limit])
                .expect_err("out-of-range limit");
            assert!(error.to_string().contains("is not in 1..=50"));
        }
        for tokens in ["255", "64001"] {
            let error =
                Cli::try_parse_from(["cortana", "context", "query", "--max-tokens", tokens])
                    .expect_err("out-of-range max tokens");
            assert!(error.to_string().contains("is not in 256..=64000"));
        }
    }

    #[test]
    fn migrate_embedding_command_parses_exact_source_and_confirmation() {
        let cli = Cli::try_parse_from([
            "cortana",
            "migrate-embedding",
            "--from",
            "legacy:model:16",
            "--force",
        ])
        .expect("embedding migration command");
        match cli.command {
            Some(Command::MigrateEmbedding { from, force }) => {
                assert_eq!(from, "legacy:model:16");
                assert!(force);
            }
            _ => panic!("expected the migrate-embedding subcommand"),
        }
    }

    #[test]
    fn rebuild_embeddings_command_requires_exact_source_and_confirmation() {
        let cli = Cli::try_parse_from([
            "cortana",
            "rebuild-embeddings",
            "--from",
            "legacy:model:16",
            "--force",
        ])
        .expect("embedding rebuild command");
        match cli.command {
            Some(Command::RebuildEmbeddings { from, force }) => {
                assert_eq!(from, "legacy:model:16");
                assert!(force);
            }
            _ => panic!("expected the rebuild-embeddings subcommand"),
        }
    }

    #[tokio::test]
    async fn context_bundle_output_is_stable_cited_json() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));
        store
            .ensure_fingerprint(&embedder.fingerprint())
            .expect("fingerprint");
        let content = "Deploy after validation.".to_string();
        let vector = embedder
            .embed(std::slice::from_ref(&content))
            .await
            .expect("embedding")
            .remove(0);
        store
            .upsert(
                &Document {
                    source: "notes".into(),
                    source_id: "runbook".into(),
                    title: "Release runbook".into(),
                    content: content.clone(),
                    uri: Some("file:///runbook.md".into()),
                    updated_at: Utc::now(),
                    project: "engineering".into(),
                    acl: Vec::new(),
                    metadata: serde_json::json!({}),
                },
                &[(content, vector)],
            )
            .expect("upsert");

        let bundle = context_bundle(
            &store,
            &embedder,
            "deploy",
            Some("engineering"),
            None,
            10,
            2_000,
        )
        .await
        .expect("context bundle");
        assert!(bundle.context.contains("### [1] Release runbook"));
        assert!(bundle.context.contains("Cite sources with [n]"));
        assert_eq!(bundle.evidence.len(), 1);
        assert_eq!(bundle.metrics.retrieved, 1);
        assert_eq!(bundle.metrics.included, 1);
        assert_eq!(bundle.metrics.omitted, 0);
        assert_eq!(bundle.metrics.max_tokens, 2_000);

        let json = serde_json::to_string_pretty(&bundle).expect("serialized bundle");
        for field in [
            "\"query\"",
            "\"context\"",
            "\"evidence\"",
            "\"metrics\"",
            "\"estimated_tokens\"",
            "\"max_tokens\"",
        ] {
            assert!(json.contains(field), "missing {field} in {json}");
        }
        assert!(json.contains("### [1] Release runbook"));
    }
}
