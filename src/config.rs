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
    pub ingestion: IngestionConfig,
    #[serde(default)]
    pub query: QueryConfig,
    #[serde(default)]
    pub auth: AuthConfig,
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
    #[serde(default = "default_embedding_request_concurrency")]
    pub request_concurrency: usize,
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
pub struct IngestionConfig {
    #[serde(default = "default_ingestion_max_documents")]
    pub max_documents_per_source: usize,
    #[serde(default = "default_ingestion_max_bytes")]
    pub max_bytes_per_source: u64,
    #[serde(default = "default_ingestion_max_duration")]
    pub max_duration_seconds: u64,
    #[serde(default = "default_ingestion_document_batch_size")]
    pub document_batch_size: usize,
    #[serde(default = "default_ingestion_request_concurrency")]
    pub request_concurrency: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QueryConfig {
    #[serde(default)]
    pub synthesis_enabled: bool,
    #[serde(default = "default_query_model_url")]
    pub base_url: String,
    #[serde(default = "default_query_model")]
    pub model: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default = "default_query_max_planned_queries")]
    pub max_planned_queries: usize,
    #[serde(default = "default_query_retrieval_limit")]
    pub retrieval_limit: usize,
    #[serde(default = "default_query_result_limit")]
    pub result_limit: usize,
    #[serde(default = "default_query_context_tokens")]
    pub context_tokens: usize,
    #[serde(default = "default_query_output_tokens")]
    pub output_tokens: usize,
    #[serde(default = "default_query_timeout")]
    pub request_timeout_seconds: u64,
    #[serde(default = "default_answer_timeout")]
    pub answer_timeout_seconds: u64,
    #[serde(default = "default_query_concurrency")]
    pub request_concurrency: usize,
    #[serde(default = "default_query_cache_entries")]
    pub cache_max_entries: usize,
    #[serde(default = "default_query_cache_ttl")]
    pub cache_ttl_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AuthConfig {
    #[serde(default = "default_audit_max_events")]
    pub audit_max_events: usize,
    #[serde(default)]
    pub tokens: Vec<AuthTokenConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AuthTokenConfig {
    pub principal: String,
    pub token_env: String,
    #[serde(default = "default_auth_scopes")]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub acl: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConnectorConfig {
    #[serde(default = "default_connector_command")]
    pub command: Vec<String>,
    #[serde(default = "default_connector_timeout")]
    pub timeout_seconds: u64,
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
    pub max_content_chars: Option<usize>,
    #[serde(default)]
    pub max_documents: Option<usize>,
    #[serde(default)]
    pub max_bytes: Option<u64>,
    #[serde(default)]
    pub max_duration_seconds: Option<u64>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub acl: Vec<String>,
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
            request_concurrency: default_embedding_request_concurrency(),
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

impl Default for IngestionConfig {
    fn default() -> Self {
        Self {
            max_documents_per_source: default_ingestion_max_documents(),
            max_bytes_per_source: default_ingestion_max_bytes(),
            max_duration_seconds: default_ingestion_max_duration(),
            document_batch_size: default_ingestion_document_batch_size(),
            request_concurrency: default_ingestion_request_concurrency(),
        }
    }
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            synthesis_enabled: false,
            base_url: default_query_model_url(),
            model: default_query_model(),
            api_key_env: None,
            max_planned_queries: default_query_max_planned_queries(),
            retrieval_limit: default_query_retrieval_limit(),
            result_limit: default_query_result_limit(),
            context_tokens: default_query_context_tokens(),
            output_tokens: default_query_output_tokens(),
            request_timeout_seconds: default_query_timeout(),
            answer_timeout_seconds: default_answer_timeout(),
            request_concurrency: default_query_concurrency(),
            cache_max_entries: default_query_cache_entries(),
            cache_ttl_seconds: default_query_cache_ttl(),
        }
    }
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            command: default_connector_command(),
            timeout_seconds: default_connector_timeout(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            audit_max_events: default_audit_max_events(),
            tokens: Vec::new(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            embedding: EmbeddingConfig::default(),
            ingestion: IngestionConfig::default(),
            query: QueryConfig::default(),
            auth: AuthConfig::default(),
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

fn default_query_model_url() -> String {
    "http://127.0.0.1:8008/v1".into()
}

fn default_query_model() -> String {
    "auto-efficient".into()
}

const fn default_query_max_planned_queries() -> usize {
    4
}

const fn default_query_retrieval_limit() -> usize {
    10
}

const fn default_query_result_limit() -> usize {
    20
}

const fn default_query_context_tokens() -> usize {
    8_000
}

const fn default_query_output_tokens() -> usize {
    1_200
}

const fn default_query_timeout() -> u64 {
    45
}

const fn default_answer_timeout() -> u64 {
    55
}

const fn default_query_concurrency() -> usize {
    4
}

const fn default_query_cache_entries() -> usize {
    10_000
}

const fn default_query_cache_ttl() -> u64 {
    3_600
}

const fn default_audit_max_events() -> usize {
    10_000
}

fn default_auth_scopes() -> Vec<String> {
    vec!["query".into(), "status".into()]
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

fn default_embedding_request_concurrency() -> usize {
    4
}

fn default_embedding_startup_timeout() -> u64 {
    120
}

fn default_embedding_memory_limit() -> u64 {
    4_096
}

fn default_ingestion_max_documents() -> usize {
    2_000
}

fn default_ingestion_max_bytes() -> u64 {
    128 * 1024 * 1024
}

fn default_ingestion_max_duration() -> u64 {
    15 * 60
}

fn default_ingestion_document_batch_size() -> usize {
    16
}

fn default_ingestion_request_concurrency() -> usize {
    1
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

fn default_connector_timeout() -> u64 {
    6 * 60 * 60
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
            max_content_chars = 12345
            max_documents = 100
            max_bytes = 2048
            max_duration_seconds = 30

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
        assert_eq!(config.sources[0].max_content_chars, Some(12_345));
        assert_eq!(config.sources[0].max_documents, Some(100));
        assert_eq!(config.sources[0].max_bytes, Some(2_048));
        assert_eq!(config.sources[0].max_duration_seconds, Some(30));
        assert_eq!(config.sources[1].source.as_deref(), Some("code"));
        assert_eq!(config.ingestion.max_documents_per_source, 2_000);
        assert_eq!(config.ingestion.max_bytes_per_source, 128 * 1024 * 1024);
        assert_eq!(config.ingestion.request_concurrency, 1);
    }

    #[test]
    fn reserves_enough_memory_for_the_default_local_embedding_model() {
        assert_eq!(Config::default().embedding.service.memory_limit_mb, 4_096);
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
