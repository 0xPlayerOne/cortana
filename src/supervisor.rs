use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::{Client, Url};
use tokio::process::{Child, Command};

use crate::config::Config;

pub async fn run_embedding(config: &Config) -> Result<()> {
    let (program, arguments) = embedding_command(config)?;
    let probe_url = embedding_probe_url(config)?;
    let client = Client::builder().timeout(Duration::from_secs(2)).build()?;
    tracing::info!(
        program,
        model = config.embedding.model,
        %probe_url,
        "starting local embedding service"
    );
    let mut child = Command::new(&program)
        .args(&arguments)
        .envs(&config.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("failed to start embedding service {program}"))?;

    let startup = Duration::from_secs(config.embedding.service.startup_timeout_seconds);
    let started = tokio::time::Instant::now();
    loop {
        if healthy(&client, &probe_url, config).await {
            tracing::info!(pid = child.id(), "embedding service is healthy");
            break;
        }
        if let Some(status) = child.try_wait()? {
            anyhow::bail!("embedding service exited before becoming healthy: {status}");
        }
        anyhow::ensure!(
            started.elapsed() < startup,
            "embedding service did not become healthy within {} seconds",
            startup.as_secs()
        );
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(1)) => {}
            () = shutdown_signal() => {
                stop(&mut child).await;
                return Ok(());
            }
        }
    }

    let mut interval = tokio::time::interval(Duration::from_secs(10));
    loop {
        tokio::select! {
            () = shutdown_signal() => {
                stop(&mut child).await;
                return Ok(());
            }
            _ = interval.tick() => {
                if let Some(status) = child.try_wait()? {
                    anyhow::bail!("embedding service exited: {status}");
                }
                let limit = config.embedding.service.memory_limit_mb;
                if limit > 0
                    && resident_memory_mb(child.id()).await.is_some_and(|memory| memory > limit)
                {
                    stop(&mut child).await;
                    anyhow::bail!("embedding service exceeded its {limit} MiB memory limit");
                }
                if !healthy(&client, &probe_url, config).await {
                    tracing::warn!("real embedding probe failed");
                }
            }
        }
    }
}

pub fn uses_local_service(config: &Config) -> bool {
    !config.embedding.service.command.is_empty()
        || Url::parse(&config.embedding.base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .is_some_and(|host| matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1"))
}

fn embedding_command(config: &Config) -> Result<(String, Vec<String>)> {
    if let Some((program, arguments)) = config.embedding.service.command.split_first() {
        return Ok((program.clone(), arguments.to_vec()));
    }
    let url = Url::parse(&config.embedding.base_url)?;
    let host = url
        .host_str()
        .context("embedding base URL does not contain a host")?;
    anyhow::ensure!(
        matches!(host, "127.0.0.1" | "localhost" | "::1"),
        "automatic embedding service can only bind a loopback base URL"
    );
    let port = url.port_or_known_default().unwrap_or(80);
    Ok((
        "text-embeddings-router".into(),
        vec![
            "--model-id".into(),
            config.embedding.model.clone(),
            "--dtype".into(),
            "float16".into(),
            "--hostname".into(),
            host.into(),
            "--port".into(),
            port.to_string(),
            "--max-batch-tokens".into(),
            "4096".into(),
            "--max-batch-requests".into(),
            "16".into(),
            "--max-concurrent-requests".into(),
            "128".into(),
        ],
    ))
}

fn embedding_probe_url(config: &Config) -> Result<Url> {
    let mut url = Url::parse(&config.embedding.base_url)?;
    let path = format!("{}/embeddings", url.path().trim_end_matches('/'));
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

async fn healthy(client: &Client, url: &Url, config: &Config) -> bool {
    let request = serde_json::json!({
        "model": config.embedding.model,
        "input": ["__cortana_probe__"],
    });
    let Ok(response) = client.post(url.clone()).json(&request).send().await else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    response
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|value| value["data"][0]["embedding"].as_array().map(Vec::len))
        == Some(config.embedding.dimension)
}

async fn resident_memory_mb(pid: Option<u32>) -> Option<u64> {
    let output = Command::new("ps")
        .args(["-p", &pid?.to_string(), "-o", "rss="])
        .output()
        .await
        .ok()?;
    let kib = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(kib.div_ceil(1024))
}

async fn stop(child: &mut Child) {
    tracing::info!(pid = child.id(), "stopping embedding service");
    let _ = child.kill().await;
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install terminate handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_local_tei_command_and_probe_url() {
        let config = Config::default();
        assert!(uses_local_service(&config));
        let (program, arguments) = embedding_command(&config).expect("command");
        assert_eq!(program, "text-embeddings-router");
        assert!(arguments.windows(2).any(|pair| pair == ["--port", "6999"]));
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--max-concurrent-requests", "128"])
        );
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--max-batch-tokens", "4096"])
        );
        assert_eq!(
            embedding_probe_url(&config).expect("probe").as_str(),
            "http://127.0.0.1:6999/v1/embeddings"
        );
    }
}
