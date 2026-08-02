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
use cortana::{api, google_oauth, mcp, migration, service, source_validation, supervisor};
use fs2::FileExt;
use futures_util::{StreamExt, TryStreamExt, stream};
use serde::Deserialize;
use tokio::process::Command as ProcessCommand;

// Leave half of the default local TEI permits available for interactive agents.
const EMBEDDING_REQUEST_SIZE: usize = 8;
// The CLI context command mirrors the HTTP/MCP context contract defaults.
const DEFAULT_CONTEXT_LIMIT: usize = 10;

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
    /// Run deterministic retrieval and answer quality gates in an isolated temporary index.
    Eval {
        #[arg(long, help = "Use a custom synthetic evaluation fixture")]
        fixture: Option<PathBuf>,
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
            requires = "source",
            help = "Require a matching successful validation at equal or larger limits"
        )]
        require_validation: bool,
    },
    /// Fetch and validate one configured source without embedding or indexing it.
    ValidateSource {
        source: String,
        #[arg(long, help = "Override the document budget for this validation")]
        max_documents: Option<usize>,
        #[arg(long, help = "Override the content-byte budget for this validation")]
        max_bytes: Option<u64>,
        #[arg(long, help = "Override the wall-clock budget for this validation")]
        max_seconds: Option<u64>,
    },
    /// Authorize a configured Google source in the system browser without reading source data.
    AuthorizeGoogle { source: String },
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

#[derive(Debug, Subcommand)]
enum ServiceAction {
    /// Install and immediately bootstrap macOS launchd jobs.
    Install {
        #[arg(long, default_value = "apps/web/dist")]
        web_dir: PathBuf,
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
    },
    /// Apply explicit project ACLs after source defaults agree.
    Apply {
        #[arg(long = "project", value_name = "PROJECT=LABEL[,LABEL]")]
        projects: Vec<String>,
        #[arg(long, help = "Confirm mutation of legacy public ACL rows")]
        force: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
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
    if let Some(Command::Eval { fixture }) = cli.command.as_ref() {
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
    }) = cli.command.as_ref()
    {
        return validate_configured_source(
            &config,
            source,
            SyncOverrides {
                max_documents: *max_documents,
                max_bytes: *max_bytes,
                max_seconds: *max_seconds,
            },
        )
        .await;
    }
    if let Some(Command::AuthorizeGoogle { source }) = cli.command.as_ref() {
        let outcome = google_oauth::authorize(&config, source).await?;
        println!("{}", serde_json::to_string(&outcome)?);
        return Ok(());
    }
    if let Some(Command::Sync {
        source,
        plan: false,
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
            | Command::Acl { .. },
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
                sync_source_documents(&config, &store, embedder.as_ref(), &source, &control).await;
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
            let configured_sources = config
                .sources
                .iter()
                .map(|source| mcp::ConfiguredSourceStatus {
                    name: source.name.clone(),
                    source: source.source.clone().unwrap_or_else(|| source.name.clone()),
                    kind: source.kind.clone(),
                    project: source.project.clone(),
                    enabled: source.enabled,
                })
                .collect();
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
            | Command::Eval { .. }
            | Command::AuthorizeGoogle { .. }
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

fn mcp_source_groups(config: &Config) -> (Vec<String>, Vec<String>) {
    let mut code = Vec::new();
    let mut messages = Vec::new();
    for source in config.sources.iter().filter(|source| source.enabled) {
        let stored_source = source.source.clone().unwrap_or_else(|| source.name.clone());
        match source.kind.as_str() {
            "filesystem" => code.push(stored_source),
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

#[derive(Clone, Copy, Debug, serde::Serialize)]
struct SourceLimits {
    max_documents: usize,
    max_bytes: u64,
    max_seconds: u64,
    document_batch_size: usize,
    request_concurrency: usize,
}

impl SourceLimits {
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
    let (values, apply, force) = match action {
        AclAction::Plan { projects } => (projects, false, false),
        AclAction::Apply { projects, force } => (projects, true, *force),
    };
    let mappings = parse_project_acl_mappings(values)?;
    let public = store.public_acl_summary()?;
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
    println!(
        "{}",
        serde_json::json!({
            "applied": false,
            "public": public,
            "proposed": proposed,
            "source_alignment_errors": alignment_errors,
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
        let mut configured = source.acl.clone();
        configured.sort();
        configured.dedup();
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
) -> Result<()> {
    let source = config
        .sources
        .iter()
        .find(|source| source.name == selected)
        .with_context(|| format!("configured source {selected} was not found"))?;
    let limits = SourceLimits::resolve(config, source, overrides)?;
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
            )?;
            return Ok(serde_json::json!({
                "documents": scope.documents,
                "bytes": scope.bytes,
                "inspection": "filesystem preflight"
            }));
        }
        cleanup_connector_spools(&config.data_dir)?;
        let (spool, diagnostics) = run_connector_to_spool(config, source, &control).await?;
        let validation = validate_connector_spool(&spool, source, &control);
        let _ = std::fs::remove_file(&spool);
        let _ = std::fs::remove_file(&diagnostics);
        let scope = validation?;
        Ok(serde_json::json!({
            "documents": scope.documents,
            "bytes": scope.bytes,
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
) -> Result<()> {
    let selected = selected.context("--require-validation requires --source")?;
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
            working_directory,
            sync_seconds,
            backup_seconds,
            no_embedding_service,
            enable_sync_service,
        } => {
            let web_dir = web_dir.canonicalize().with_context(|| {
                format!("workspace directory does not exist: {}", web_dir.display())
            })?;
            let working_directory = working_directory
                .clone()
                .unwrap_or(std::env::current_dir()?)
                .canonicalize()?;
            service::install(
                config,
                service::InstallOptions {
                    config: &config_path.canonicalize()?,
                    web_dir: &web_dir,
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
    anyhow::ensure!(
        !vectors[0].is_empty(),
        "embedding provider returned an empty vector"
    );
    let _ = store.all_chunks(None, None)?;
    println!(
        "cortana: healthy (embedding={}, dimension={})",
        embedder.fingerprint(),
        vectors[0].len()
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
            let scope = sync_source_documents(config, store, embedder, source, &control).await?;
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
    if message.contains("cancel") {
        SyncRunStatus::Cancelled
    } else if message.contains("budget") || message.contains("safety bound") {
        SyncRunStatus::BudgetExceeded
    } else {
        SyncRunStatus::Failed
    }
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
        )?;
        tracing::info!(
            source = source.name,
            documents = plan.documents,
            bytes = plan.bytes,
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
        let mut batch = Vec::with_capacity(control.limits.document_batch_size);
        let mut content_bytes = 0_u64;
        for document in documents {
            control.check(&source.name)?;
            let mut document = document?;
            normalize_documents(std::slice::from_mut(&mut document), source);
            content_bytes = content_bytes
                .saturating_add(u64::try_from(document.content.len()).unwrap_or(u64::MAX));
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

    let (spool, diagnostics) = run_connector_to_spool(config, source, control).await?;
    let result = async {
        let scope = validate_connector_spool(&spool, source, control)?;
        let reader = std::io::BufReader::new(
            std::fs::File::open(&spool)
                .with_context(|| format!("failed to open {}", spool.display()))?,
        );
        let mut seen = Vec::new();
        let mut batch = Vec::with_capacity(control.limits.document_batch_size);
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
) -> Result<(PathBuf, PathBuf)> {
    let staging = prepare_connector_staging(&config.data_dir, true)?;
    let identifier = uuid::Uuid::new_v4();
    let spool = staging.join(format!("connector-{identifier}.jsonl"));
    let diagnostics = staging.join(format!("connector-{identifier}.stderr"));
    let stdout = private_file(&spool)?;
    let stderr = private_file(&diagnostics)?;
    let mut command = configured_connector_command(config, source)?;
    let executable = command.remove(0);
    let child = ProcessCommand::new(&executable)
        .args(&command)
        .envs(&config.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
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
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = std::fs::remove_file(&spool);
            let _ = std::fs::remove_file(&diagnostics);
            anyhow::bail!("connector {} cancelled before reconciliation", source.name);
        }
        let spool_bytes = std::fs::metadata(&spool).map_or(0, |metadata| metadata.len());
        let diagnostic_bytes = std::fs::metadata(&diagnostics).map_or(0, |metadata| metadata.len());
        if spool_bytes > maximum_spool_bytes || diagnostic_bytes > MAXIMUM_DIAGNOSTIC_BYTES {
            let _ = child.kill().await;
            let _ = child.wait().await;
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
            let _ = child.kill().await;
            let _ = child.wait().await;
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

fn configured_connector_command(config: &Config, source: &SourceConfig) -> Result<Vec<String>> {
    let command = if source.kind == "external" {
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
            source.kind.clone(),
        ]);
        connector_arguments(&mut command, source)?;
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
            document.acl.clone_from(&source.acl);
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
        Cancellation, Cli, Command, DEFAULT_CONTEXT_LIMIT, SourceControl, SourceLimits, SyncLock,
        chunk, cleanup_connector_spools, context_bundle, ingest_documents, private_file,
        run_connector_to_spool,
    };
    use cortana::config::{Config, SourceConfig};
    use cortana::embed::{DeterministicEmbedder, Embedder};
    use cortana::model::Document;
    use cortana::store::Store;

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
        let mut config = Config::default();
        config.data_dir = directory.path().to_path_buf();
        config.connectors.timeout_seconds = 2;
        let source = SourceConfig {
            name: "slow-external".into(),
            kind: "external".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
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

        let result = run_connector_to_spool(&config, &source, &control).await;
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
