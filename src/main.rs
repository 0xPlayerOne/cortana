use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use cortana::config::{Config, default_config_path};
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
    let mut changed = 0;
    let mut unchanged = 0;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let document: Document = serde_json::from_str(&line).context("invalid Document JSONL")?;
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
