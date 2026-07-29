use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cortana::config::{Config, SourceConfig, default_config_path};
use cortana::connectors;
use cortana::embed::{DeterministicEmbedder, Embedder, OpenAiEmbedder};
use cortana::model::Document;
use cortana::retrieval;
use cortana::store::Store;
use cortana::{api, mcp};

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
    Init,
    /// Validate configuration, storage, and the embedding provider.
    Doctor,
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
    },
    /// Serve retrieval tools over MCP stdio.
    Mcp,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if matches!(cli.command, Some(Command::Init)) {
        return init(cli.config);
    }
    let config = Config::load(cli.config.as_deref())?;
    let store = Store::open(&config.database_path())?;
    let embedder: Arc<dyn Embedder> = if cli.offline {
        Arc::new(DeterministicEmbedder::new(256))
    } else {
        Arc::new(OpenAiEmbedder::new(config.embedding.clone()))
    };
    store.ensure_fingerprint(&embedder.fingerprint())?;

    match cli.command {
        Some(Command::Doctor) => doctor(&store, embedder.as_ref()).await,
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
        Some(Command::Serve { address }) => {
            api::serve(api::AppState { store, embedder }, &address).await
        }
        Some(Command::Mcp) => mcp::serve(mcp::BrainServer::new(store, embedder)).await,
        Some(Command::Init) => unreachable!(),
        None => {
            println!("cortana {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

fn init(path: Option<PathBuf>) -> Result<()> {
    let path = path.unwrap_or_else(default_config_path);
    if path.exists() {
        println!("configuration already exists: {}", path.display());
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let config = Config::default();
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
    anyhow::ensure!(
        !sources.is_empty(),
        "no enabled configured sources matched the selection"
    );
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
