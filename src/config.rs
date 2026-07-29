use std::collections::HashMap;
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
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    #[serde(skip)]
    pub environment: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub env_file: Option<PathBuf>,
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
    #[serde(default = "default_embedding_cache_entries")]
    pub cache_max_entries: usize,
    #[serde(default = "default_embedding_request_timeout")]
    pub request_timeout_seconds: u64,
    #[serde(default)]
    pub service: EmbeddingServiceConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EmbeddingServiceConfig {
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default = "default_embedding_startup_timeout")]
    pub startup_timeout_seconds: u64,
    #[serde(default = "default_embedding_memory_limit")]
    pub memory_limit_mb: u64,
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
    pub exclude: Vec<String>,
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
            cache_max_entries: default_embedding_cache_entries(),
            request_timeout_seconds: default_embedding_request_timeout(),
            service: EmbeddingServiceConfig::default(),
        }
    }
}

impl Default for EmbeddingServiceConfig {
    fn default() -> Self {
        Self {
            command: Vec::new(),
            startup_timeout_seconds: default_embedding_startup_timeout(),
            memory_limit_mb: default_embedding_memory_limit(),
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
            runtime: RuntimeConfig::default(),
            sources: Vec::new(),
            environment: HashMap::new(),
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

    pub fn load_environment(&mut self) -> Result<()> {
        let Some(path) = &self.runtime.env_file else {
            return Ok(());
        };
        validate_secret_file(path)?;
        for (line_number, raw) in std::fs::read_to_string(path)?.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (name, value) = line.split_once('=').with_context(|| {
                format!(
                    "invalid environment entry at {}:{}",
                    path.display(),
                    line_number + 1
                )
            })?;
            let name = name.trim();
            anyhow::ensure!(
                !name.is_empty()
                    && name
                        .chars()
                        .all(|character| character == '_' || character.is_ascii_alphanumeric()),
                "invalid environment variable name at {}:{}",
                path.display(),
                line_number + 1
            );
            self.environment
                .entry(name.to_string())
                .or_insert_with(|| value.trim().trim_matches(['"', '\'']).to_string());
        }
        Ok(())
    }

    pub fn environment_value(&self, name: &str) -> Option<String> {
        std::env::var(name)
            .ok()
            .or_else(|| self.environment.get(name).cloned())
    }
}

fn validate_secret_file(path: &Path) -> Result<()> {
    anyhow::ensure!(
        path.is_file(),
        "environment file is missing: {}",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = std::fs::metadata(path)?.permissions().mode();
        anyhow::ensure!(
            mode & 0o077 == 0,
            "environment file must not be accessible by group or others: {}",
            path.display()
        );
    }
    Ok(())
}

pub fn default_config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("cortana/config.toml")
}

fn default_data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
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

fn default_embedding_cache_entries() -> usize {
    250_000
}

fn default_embedding_request_timeout() -> u64 {
    180
}

fn default_embedding_startup_timeout() -> u64 {
    120
}

fn default_embedding_memory_limit() -> u64 {
    2_048
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

    #[cfg(unix)]
    #[test]
    fn rejects_environment_files_with_broad_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("secrets.env");
        std::fs::write(&path, "CORTANA_TEST_VALUE=secret\n").expect("write secrets");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("set permissions");
        let mut config = Config {
            runtime: RuntimeConfig {
                env_file: Some(path),
            },
            ..Config::default()
        };
        assert!(config.load_environment().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn loads_private_environment_file_without_mutating_process_environment() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("secrets.env");
        std::fs::write(&path, "CORTANA_TEST_PRIVATE=loaded\n").expect("write secrets");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("set permissions");
        let mut config = Config {
            runtime: RuntimeConfig {
                env_file: Some(path),
            },
            ..Config::default()
        };
        config.load_environment().expect("load environment");
        assert_eq!(
            config.environment.get("CORTANA_TEST_PRIVATE"),
            Some(&"loaded".into())
        );
        assert!(std::env::var_os("CORTANA_TEST_PRIVATE").is_none());
    }
}
