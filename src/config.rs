use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
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

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            embedding: EmbeddingConfig::default(),
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
