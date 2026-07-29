use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cortana::config::{Config, SourceConfig, default_config_path};
use cortana::connectors;
use cortana::embed::{CachedEmbedder, DeterministicEmbedder, Embedder, OpenAiEmbedder};
use cortana::model::Document;
use cortana::retrieval;
use cortana::store::Store;
use cortana::{api, mcp, service, supervisor};

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
    /// Validate configuration, storage, and the embedding provider.
    Doctor,
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
    /// Incrementally ingest a code, notes, transcript, or document tree.
    SyncFiles {
        root: PathBuf,
        #[arg(long, default_value = "files")]
        source: String,
        #[arg(long, default_value = "default")]
        project: String,
    },
    /// Synchronize enabled sources declared in the configuration.
    Sync {
        #[arg(long)]
        source: Option<String>,
        #[arg(long, help = "Keep records missing from a completed source snapshot")]
        no_reconcile: bool,
    },
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
    Mcp,
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
    },
    /// Print current background service states.
    Status,
    /// Stop and remove Cortana's per-user background services.
    Uninstall,
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
    let config_path = cli.config.clone().unwrap_or_else(default_config_path);
    let mut config = Config::load(Some(&config_path))?;
    config.load_environment()?;
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
        Arc::new(OpenAiEmbedder::new(config.embedding.clone(), api_key))
    };
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
            | Command::Service { .. },
        ) => {
            unreachable!()
        }
        Some(Command::Ingest { input }) => ingest(&store, embedder.as_ref(), &input).await,
        Some(Command::SyncFiles {
            root,
            source,
            project,
        }) => {
            let documents = connectors::filesystem_documents(&root, &source, &project)?;
            ingest_documents(&store, embedder.as_ref(), documents).await
        }
        Some(Command::Sync {
            source,
            no_reconcile,
        }) => {
            sync_configured_sources(
                &config,
                &store,
                embedder.as_ref(),
                source.as_deref(),
                !no_reconcile,
            )
            .await
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
            anyhow::ensure!(
                !allow_remote || api_token.is_some(),
                "--allow-remote requires --api-token-env"
            );
            let web_dir = (!no_web).then_some(web_dir);
            api::serve(
                api::AppState::new(store, embedder, api_token),
                &address,
                web_dir.as_deref(),
                allow_remote,
            )
            .await
        }
        Some(Command::Mcp) => mcp::serve(mcp::BrainServer::new(store, embedder)).await,
        Some(Command::Init { .. }) => unreachable!(),
        None => {
            println!("cortana {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
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
                },
            )
        }
        ServiceAction::Status => service::status(),
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
    let mut documents = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        documents.push(serde_json::from_str(&line).context("invalid Document JSONL")?);
    }
    ingest_documents(store, embedder, documents).await
}

async fn ingest_documents(
    store: &Store,
    embedder: &dyn Embedder,
    documents: Vec<Document>,
) -> Result<()> {
    let mut changed = 0;
    let mut unchanged = 0;
    for document in documents {
        if !store.needs_update(&document)? {
            store.refresh_timestamp(&document)?;
            unchanged += 1;
            continue;
        }
        let texts = chunk(&document.content);
        let vectors = embedder.embed(&texts).await?;
        let chunks = texts.into_iter().zip(vectors).collect::<Vec<_>>();
        if store.upsert(&document, &chunks)? {
            changed += 1;
        } else {
            unchanged += 1;
        }
    }
    println!("ingested changed={changed} unchanged={unchanged}");
    Ok(())
}

async fn sync_configured_sources(
    config: &Config,
    store: &Store,
    embedder: &dyn Embedder,
    selected: Option<&str>,
    reconcile: bool,
) -> Result<()> {
    let sources = config
        .sources
        .iter()
        .filter(|source| source.enabled && selected.is_none_or(|name| source.name == name))
        .collect::<Vec<_>>();
    if sources.is_empty() {
        println!("no enabled configured sources matched the selection");
        return Ok(());
    }
    for source in sources {
        let documents = source_documents(config, source)?;
        let seen = documents
            .iter()
            .map(|document| document.source_id.clone())
            .collect::<Vec<_>>();
        let canonical_source = canonical_source(source);
        ingest_documents(store, embedder, documents).await?;
        let deleted = if reconcile {
            store.reconcile(&canonical_source, &source.project, &seen)?
        } else {
            0
        };
        println!("synced source={} deleted={deleted}", source.name);
    }
    Ok(())
}

fn source_documents(config: &Config, source: &SourceConfig) -> Result<Vec<Document>> {
    if source.kind == "filesystem" {
        let root = source
            .root
            .as_ref()
            .with_context(|| format!("source {} requires root", source.name))?;
        let mut documents = connectors::filesystem_documents(
            root,
            source.source.as_deref().unwrap_or(&source.name),
            &source.project,
        )?;
        normalize_documents(&mut documents, source);
        return Ok(documents);
    }
    let mut command = if source.kind == "external" {
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
            source.kind.clone(),
        ]);
        connector_arguments(&mut command, source)?;
        command
    };
    let executable = command.remove(0);
    let output = ProcessCommand::new(&executable)
        .args(&command)
        .envs(&config.environment)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run connector command {executable}"))?;
    anyhow::ensure!(
        output.status.success(),
        "connector {} failed: {}",
        source.name,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let mut documents = String::from_utf8(output.stdout)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).with_context(|| {
                format!("connector {} emitted invalid Document JSONL", source.name)
            })
        })
        .collect::<Result<Vec<Document>>>()?;
    normalize_documents(&mut documents, source);
    Ok(documents)
}

fn normalize_documents(documents: &mut [Document], source: &SourceConfig) {
    let canonical = canonical_source(source);
    for document in documents {
        let connector_kind = document.source.clone();
        document.source.clone_from(&canonical);
        document.project.clone_from(&source.project);
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

fn chunk(content: &str) -> Vec<String> {
    const TARGET: usize = 1_600;
    const OVERLAP: usize = 200;
    let mut output = Vec::new();
    let mut current = String::new();
    for paragraph in content.split("\n\n").filter(|part| !part.trim().is_empty()) {
        if !current.is_empty() && current.len() + paragraph.len() + 2 > TARGET {
            output.push(current.clone());
            let tail = current
                .char_indices()
                .rev()
                .find(|(index, _)| current.len() - index >= OVERLAP)
                .map(|(index, _)| &current[index..])
                .unwrap_or(&current);
            current = format!("{tail}\n\n");
        }
        current.push_str(paragraph.trim());
        current.push_str("\n\n");
    }
    if !current.trim().is_empty() {
        output.push(current.trim().to_string());
    }
    output
}
