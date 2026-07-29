use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub connectors: ConnectorConfig,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EmbeddingConfig {
    #[serde(default = "default_embedding_url")]
    pub base_url: String,
    #[serde(default = "default_embedding_model")]
    pub model: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default = "default_dimension")]
    pub dimension: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConnectorConfig {
    #[serde(default = "default_connector_command")]
    pub command: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SourceConfig {
    pub name: String,
    pub kind: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_project")]
    pub project: String,
    #[serde(default)]
    pub root: Option<PathBuf>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default)]
    pub token: Option<PathBuf>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub command: Vec<String>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            base_url: default_embedding_url(),
            model: default_embedding_model(),
            api_key_env: None,
            dimension: default_dimension(),
        }
    }
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            command: default_connector_command(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            embedding: EmbeddingConfig::default(),
            connectors: ConnectorConfig::default(),
            sources: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = path
            .map(Path::to_path_buf)
            .unwrap_or_else(default_config_path);
        if !path.exists() {
            return Ok(Self::default());
        }
        let body = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        toml::from_str(&body).with_context(|| format!("invalid config {}", path.display()))
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("cortana.sqlite3")
    }
}

pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("cortana/config.toml")
}

fn default_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from(".cortana"))
        .join("cortana")
}

fn default_embedding_url() -> String {
    "http://127.0.0.1:6999/v1".into()
}

fn default_embedding_model() -> String {
    "Qwen/Qwen3-Embedding-0.6B".into()
}

fn default_dimension() -> usize {
    1024
}

fn default_connector_command() -> Vec<String> {
    vec![
        "uv".into(),
        "run".into(),
        "python".into(),
        "-m".into(),
        "cortana.connectors".into(),
    ]
}

fn default_enabled() -> bool {
    true
}

fn default_project() -> String {
    "default".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_configurable_sources() {
        let config: Config = toml::from_str(
            r#"
            [[sources]]
            name = "notes"
            kind = "apple-notes"
            project = "personal"

            [[sources]]
            name = "code"
            kind = "filesystem"
            root = "/tmp/project"
            source = "code"
            "#,
        )
        .expect("valid source config");

        assert_eq!(config.sources.len(), 2);
        assert!(config.sources[0].enabled);
        assert_eq!(config.sources[1].source.as_deref(), Some("code"));
    }
}
