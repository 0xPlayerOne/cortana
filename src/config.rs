use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const MAX_CONFIGURED_SOURCES: usize = 128;
// Buzz community assignment keeps bounded counts and lengths: the identity
// file may hold more communities than any single workspace should index, and
// ids/names are printable strings with a bounded length.
const MAX_BUZZ_COMMUNITIES: usize = 100;
const MAX_BUZZ_COMMUNITY_ID_CHARS: usize = 128;
const MAX_BUZZ_COMMUNITY_NAME_CHARS: usize = 128;
// `chrono::Duration::hours` accepts a signed hour count. Reject values that
// cannot be represented before the freshness bound is converted at runtime.
const MAX_VALIDATION_MAX_AGE_HOURS: u64 = (i64::MAX as u64) / 3_600;
const MAX_SYNC_FRESHNESS_HOURS: u64 = 8_760;
const SUPPORTED_SOURCE_KINDS: &[&str] = &[
    "filesystem",
    "apple-notes",
    "buzz",
    "google-drive",
    "gmail",
    "google-calendar",
    "github",
    "slack",
    "discord",
    "external",
];

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
    pub workspaces: Vec<WorkspaceConfig>,
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
pub struct WorkspaceConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub account_label: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
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
    #[serde(default = "default_validation_max_age_hours")]
    pub validation_max_age_hours: u64,
    /// Maximum age of a successful sync before operational status reports it
    /// as stale. A value of zero disables this health bound.
    #[serde(default = "default_sync_freshness_hours")]
    pub sync_freshness_hours: u64,
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
    /// Explicit Discord server (guild) allowlist assigned to this source's
    /// workspace. Channels selected for indexing remain in `channels`; the
    /// server list records which servers browser authorization assigned to
    /// this workspace so the Desktop chooser can scope channel selection.
    #[serde(default)]
    pub servers: Vec<String>,
    /// Explicit Slack team (workspace) allowlist assigned to this source's
    /// workspace through browser authorization. A Slack user token is scoped
    /// to exactly one workspace, so `teams` holds at most one team id; the
    /// parallel `team_names` records the display names the chooser persisted
    /// so assigned workspaces stay identifiable without re-discovery.
    #[serde(default)]
    pub teams: Vec<String>,
    /// Display names of the Slack teams in `teams`, kept index-aligned.
    #[serde(default)]
    pub team_names: Vec<String>,
    /// Buzz community (team) ids assigned to this source's workspace from the
    /// read-only `agents/teams.json` identity file. This is the generic Buzz
    /// community representation and is separate from Slack's `teams` fields.
    #[serde(default)]
    pub communities: Vec<String>,
    /// Display names of the Buzz communities in `communities`, kept
    /// index-aligned so assigned communities stay identifiable without
    /// re-reading the identity file.
    #[serde(default)]
    pub community_names: Vec<String>,
    /// Explicit `owner/repository` allowlist for GitHub code sources.
    #[serde(default)]
    pub repositories: Vec<String>,
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default)]
    pub token: Option<PathBuf>,
    #[serde(default)]
    pub oauth_client: Option<PathBuf>,
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

impl SourceConfig {
    /// Return the ACL that should be applied to records emitted by this
    /// source. An omitted ACL is intentionally workspace-private rather than
    /// public; explicit labels can still opt a source into a shared boundary.
    pub fn effective_acl(&self) -> Vec<String> {
        if self.acl.is_empty() {
            return vec![self.project.clone()];
        }
        let mut acl = self.acl.clone();
        let mut seen = HashSet::new();
        acl.retain(|label| seen.insert(label.clone()));
        acl
    }
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
            validation_max_age_hours: default_validation_max_age_hours(),
            sync_freshness_hours: default_sync_freshness_hours(),
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
            workspaces: Vec::new(),
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
        let mut config: Self =
            toml::from_str(&body).with_context(|| format!("invalid config {}", path.display()))?;
        if let Some(env_file) = config.runtime.env_file.as_mut()
            && !env_file.is_absolute()
        {
            let config_dir = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(Path::to_path_buf)
                .context("configuration path has no parent directory")?;
            let config_dir = if config_dir.is_absolute() {
                config_dir
            } else {
                std::env::current_dir()?.join(config_dir)
            };
            *env_file = config_dir.join(&*env_file);
        }
        validate_source_definitions(&config)?;
        Ok(config)
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("cortana.sqlite3")
    }

    pub fn load_environment(&mut self) -> Result<()> {
        let Some(path) = &self.runtime.env_file else {
            return Ok(());
        };
        validate_secret_file(path)?;
        let mut names = HashSet::new();
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
            anyhow::ensure!(
                names.insert(name),
                "duplicate environment variable at {}:{}",
                path.display(),
                line_number + 1
            );
            let value = value.trim().trim_matches(['"', '\'']).to_string();
            anyhow::ensure!(
                !value.contains('\0'),
                "environment variable value contains NUL at {}:{}",
                path.display(),
                line_number + 1
            );
            self.environment.entry(name.to_string()).or_insert(value);
        }
        Ok(())
    }

    pub fn environment_value(&self, name: &str) -> Option<String> {
        std::env::var(name)
            .ok()
            .or_else(|| self.environment.get(name).cloned())
    }
}

fn validate_source_definitions(config: &Config) -> Result<()> {
    anyhow::ensure!(
        config.ingestion.validation_max_age_hours <= MAX_VALIDATION_MAX_AGE_HOURS,
        "ingestion validation_max_age_hours exceeds the supported maximum"
    );
    anyhow::ensure!(
        config.ingestion.sync_freshness_hours <= MAX_SYNC_FRESHNESS_HOURS,
        "ingestion sync_freshness_hours exceeds the supported maximum"
    );
    anyhow::ensure!(
        config.sources.len() <= MAX_CONFIGURED_SOURCES,
        "configured sources exceed the {MAX_CONFIGURED_SOURCES} source safety limit"
    );
    let mut workspace_ids = HashSet::new();
    for workspace in &config.workspaces {
        anyhow::ensure!(
            !workspace.id.trim().is_empty(),
            "workspace ids must not be empty"
        );
        anyhow::ensure!(
            workspace_ids.insert(workspace.id.as_str()),
            "workspace id `{}` is duplicated",
            workspace.id
        );
    }
    let mut source_names = HashSet::new();
    let mut source_scopes = HashSet::new();
    for source in &config.sources {
        anyhow::ensure!(
            !source.name.is_empty()
                && source.name.len() <= 64
                && source.name.chars().all(|character| {
                    character.is_ascii_lowercase()
                        || character.is_ascii_digit()
                        || matches!(character, '-' | '_')
                })
                && source.name.chars().next().is_some_and(|character| {
                    character.is_ascii_lowercase() || character.is_ascii_digit()
                }),
            "source names must be 1-64 lowercase letters, numbers, dashes, or underscores: {}",
            source.name
        );
        anyhow::ensure!(
            source_names.insert(source.name.as_str()),
            "source name `{}` is duplicated",
            source.name
        );
        let canonical_source = source.source.as_deref().unwrap_or(&source.name);
        anyhow::ensure!(
            !canonical_source.is_empty()
                && canonical_source == canonical_source.trim()
                && !canonical_source.trim().is_empty()
                && canonical_source.len() <= 128
                && !canonical_source.chars().any(char::is_control),
            "source `{}` has an invalid canonical identifier",
            source.name
        );
        anyhow::ensure!(
            source_scopes.insert((source.project.as_str(), canonical_source)),
            "source identifier `{canonical_source}` is duplicated in project `{}`",
            source.project
        );
        anyhow::ensure!(
            SUPPORTED_SOURCE_KINDS.contains(&source.kind.as_str()),
            "source `{}` has unsupported kind `{}`",
            source.name,
            source.kind
        );
        anyhow::ensure!(
            !source.project.trim().is_empty(),
            "source `{}` requires a non-empty project",
            source.name
        );
        if source.kind == "github" {
            if source.enabled {
                anyhow::ensure!(
                    !source.repositories.is_empty(),
                    "source `{}` requires at least one GitHub repository",
                    source.name
                );
                anyhow::ensure!(
                    source
                        .token_env
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
                        || source.token.is_some(),
                    "source `{}` requires a GitHub token file or token environment variable",
                    source.name
                );
            }
            for repository in &source.repositories {
                anyhow::ensure!(
                    valid_github_repository(repository),
                    "source `{}` has an invalid GitHub repository `{}`; use owner/name",
                    source.name,
                    repository
                );
            }
        }
        if source.kind == "discord" && source.enabled {
            anyhow::ensure!(
                source.token.is_some() && source.oauth_client.is_some(),
                "source `{}` requires a Discord RPC token file and OAuth client path",
                source.name
            );
            anyhow::ensure!(
                source.token_env.is_none(),
                "source `{}` cannot use token_env; Discord requires Desktop RPC paths",
                source.name
            );
        }
        if !workspace_ids.is_empty() {
            anyhow::ensure!(
                workspace_ids.contains(source.project.as_str()),
                "source `{}` uses unknown workspace `{}`",
                source.name,
                source.project
            );
        }
        if !source.communities.is_empty() || !source.community_names.is_empty() {
            anyhow::ensure!(
                source.kind == "buzz",
                "source `{}` may assign communities only for kind `buzz`; Slack teams use the `teams` fields",
                source.name
            );
        }
        anyhow::ensure!(
            source.communities.len() <= MAX_BUZZ_COMMUNITIES,
            "source `{}` assigns more than {MAX_BUZZ_COMMUNITIES} Buzz communities",
            source.name
        );
        anyhow::ensure!(
            source.community_names.len() == source.communities.len(),
            "source `{}` must keep Buzz community ids and names index-aligned",
            source.name
        );
        let mut community_ids = HashSet::new();
        for (index, community) in source.communities.iter().enumerate() {
            anyhow::ensure!(
                !community.is_empty()
                    && community.len() <= MAX_BUZZ_COMMUNITY_ID_CHARS
                    && community == community.trim()
                    && !community.chars().any(char::is_control),
                "source `{}` has an invalid Buzz community id at index {index}",
                source.name
            );
            anyhow::ensure!(
                community_ids.insert(community.as_str()),
                "source `{}` assigns Buzz community `{}` more than once",
                source.name,
                community
            );
            let name = &source.community_names[index];
            anyhow::ensure!(
                !name.trim().is_empty()
                    && name.len() <= MAX_BUZZ_COMMUNITY_NAME_CHARS
                    && !name.chars().any(char::is_control),
                "source `{}` has an invalid Buzz community name at index {index}",
                source.name
            );
        }
    }
    Ok(())
}

fn valid_github_repository(value: &str) -> bool {
    let mut parts = value.trim().split('/');
    let owner = parts.next().unwrap_or_default();
    let repository = parts.next().unwrap_or_default();
    parts.next().is_none()
        && !owner.is_empty()
        && !repository.is_empty()
        && owner.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        && repository.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

/// Validate a provider base URL before a client can send credentials or
/// document-derived content. Local HTTP is limited to loopback; remote
/// providers must use HTTPS and may not hide credentials in URL components.
pub fn validate_provider_base_url(name: &str, value: &str) -> Result<()> {
    let url =
        reqwest::Url::parse(value).with_context(|| format!("{name} provider URL is invalid"))?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https"),
        "{name} provider URL must use HTTP or HTTPS"
    );
    anyhow::ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "{name} provider URL must not include credentials, query parameters, or a fragment"
    );
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    anyhow::ensure!(
        url.scheme() != "http" || loopback,
        "{name} remote provider URL must use HTTPS"
    );
    Ok(())
}

fn validate_secret_file(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).with_context(|| {
        format!(
            "environment file is missing or inaccessible: {}",
            path.display()
        )
    })?;
    anyhow::ensure!(
        !metadata.file_type().is_symlink(),
        "environment file must not be a symlink: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.file_type().is_file(),
        "environment file is not a regular file: {}",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = metadata.permissions().mode();
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
    // The first Qwen/Metal load can take several minutes on a cold cache.
    300
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

/// A successful source validation stays current for this long. The install
/// gate for recurring sync, `sync --require-validation`, and the readiness
/// `source-validation` check all refuse a validation older than the bound so a
/// revoked credential or changed scope cannot keep a stale record blessing the
/// schedule. `0` disables the freshness bound for read-only/manual checks;
/// recurring sync rejects it before installation or reconciliation.
fn default_validation_max_age_hours() -> u64 {
    168
}

fn default_sync_freshness_hours() -> u64 {
    48
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

            [[sources]]
            name = "github-code"
            kind = "github"
            project = "work"
            repositories = ["Acme/Project"]
            token_env = "GITHUB_TOKEN"

            [[sources]]
            name = "community"
            kind = "discord"
            project = "community"
            channels = ["175928847299117064"]
            servers = ["175928847299117063"]
            token = "/tmp/cortana/discord-rpc-token.json"
            oauth_client = "/tmp/cortana/discord-rpc-client.json"

            [[sources]]
            name = "team-slack"
            kind = "slack"
            project = "work"
            channels = ["C0123456789"]
            teams = ["T0123456789"]
            team_names = ["Acme Engineering"]
            token_env = "SLACK_BOT_TOKEN"

            [[sources]]
            name = "agent-buzz"
            kind = "buzz"
            project = "agents"
            root = "/Users/example/Library/Application Support/xyz.block.buzz.app"
            communities = ["builtin-team:welcome", "team:research"]
            community_names = ["Welcome Team", "Research"]
            "#,
        )
        .expect("valid source config");

        assert_eq!(config.sources.len(), 6);
        assert!(config.sources[0].enabled);
        assert_eq!(config.sources[0].max_content_chars, Some(12_345));
        assert_eq!(config.sources[0].max_documents, Some(100));
        assert_eq!(config.sources[0].max_bytes, Some(2_048));
        assert_eq!(config.sources[0].max_duration_seconds, Some(30));
        assert_eq!(config.sources[1].source.as_deref(), Some("code"));
        assert_eq!(config.sources[2].repositories, ["Acme/Project"]);
        assert_eq!(
            config.sources[3].servers,
            ["175928847299117063"],
            "per-workspace Discord server assignment must round-trip"
        );
        assert_eq!(config.sources[3].channels, ["175928847299117064"]);
        assert_eq!(
            config.sources[4].teams,
            ["T0123456789"],
            "per-workspace Slack team assignment must round-trip"
        );
        assert_eq!(
            config.sources[4].team_names,
            ["Acme Engineering"],
            "persisted Slack team names must round-trip"
        );
        assert_eq!(config.sources[4].channels, ["C0123456789"]);
        assert_eq!(
            config.sources[5].communities,
            ["builtin-team:welcome", "team:research"],
            "per-workspace Buzz community assignment must round-trip"
        );
        assert_eq!(
            config.sources[5].community_names,
            ["Welcome Team", "Research"],
            "persisted Buzz community names must round-trip"
        );
        assert_eq!(config.ingestion.max_documents_per_source, 2_000);
        assert_eq!(config.ingestion.max_bytes_per_source, 128 * 1024 * 1024);
        assert_eq!(config.ingestion.request_concurrency, 1);
        assert_eq!(config.ingestion.validation_max_age_hours, 168);
        assert_eq!(config.ingestion.sync_freshness_hours, 48);
        validate_source_definitions(&config).expect("source definitions are safe");
    }

    #[test]
    fn enabled_github_sources_require_a_safe_allowlist_and_token_name() {
        let mut config = Config::default();
        config.sources.push(SourceConfig {
            name: "github-code".into(),
            kind: "github".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            teams: Vec::new(),
            team_names: Vec::new(),
            communities: Vec::new(),
            community_names: Vec::new(),
            repositories: vec!["acme/project".into()],
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
        });
        validate_source_definitions(&config).expect("valid GitHub source");

        config.sources[0].repositories = vec!["https://github.com/acme/project".into()];
        assert!(validate_source_definitions(&config).is_err());
        config.sources[0].repositories = vec!["acme/project".into()];
        config.sources[0].token_env = None;
        assert!(validate_source_definitions(&config).is_err());
        config.sources[0].token = Some(PathBuf::from(
            "/Users/example/.config/cortana/github-token.json",
        ));
        validate_source_definitions(&config)
            .expect("GitHub token file is an accepted credential path");
    }

    #[test]
    fn enabled_discord_sources_require_desktop_rpc_paths_without_token_env() {
        let mut config = Config::default();
        config.sources.push(SourceConfig {
            name: "community".into(),
            kind: "discord".into(),
            enabled: true,
            project: "community".into(),
            root: None,
            source: None,
            channels: vec!["123456789".into()],
            servers: Vec::new(),
            teams: Vec::new(),
            team_names: Vec::new(),
            communities: Vec::new(),
            community_names: Vec::new(),
            repositories: Vec::new(),
            token_env: Some("DISCORD_LEGACY_TOKEN_ENV".into()),
            token: Some(PathBuf::from(
                "/Users/example/.config/cortana/discord-token.json",
            )),
            oauth_client: Some(PathBuf::from(
                "/Users/example/.config/cortana/discord-client.json",
            )),
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
        assert!(
            validate_source_definitions(&config)
                .expect_err("legacy Discord token configuration must be rejected")
                .to_string()
                .contains("cannot use token_env")
        );
        config.sources[0].token_env = None;
        validate_source_definitions(&config).expect("Desktop RPC paths are valid");
    }

    #[test]
    fn rejects_ambiguous_or_unsafe_source_definitions() {
        for source_block in [
            r#"
            [[sources]]
            name = "../escape"
            kind = "filesystem"
            project = "work"
            "#,
            r#"
            [[sources]]
            name = "notes"
            kind = "filesystem"
            project = "work"

            [[sources]]
            name = "notes"
            kind = "filesystem"
            project = "personal"
            "#,
            r#"
            [[sources]]
            name = "drive"
            kind = "filesystem"
            project = "work"
            source = "shared"

            [[sources]]
            name = "code"
            kind = "filesystem"
            project = "work"
            source = "shared"
            "#,
            r#"
            [[sources]]
            name = "notes"
            kind = "future-connector"
            project = "work"
            "#,
            r#"
            [[sources]]
            name = "notes"
            kind = "filesystem"
            project = "work"
            source = " notes "
            "#,
            r#"
            [[sources]]
            name = "notes"
            kind = "filesystem"
            project = "work"
            source = "   "
            "#,
            r#"
            [[sources]]
            name = "notes"
            kind = "filesystem"
            project = "work"
            source = "line\nbreak"
            "#,
            r#"
            [[sources]]
            name = "agent-buzz"
            kind = "buzz"
            project = "agents"
            communities = ["team:research"]
            "#,
            r#"
            [[sources]]
            name = "agent-buzz"
            kind = "buzz"
            project = "agents"
            communities = ["team:research"]
            community_names = []
            "#,
            r#"
            [[sources]]
            name = "agent-buzz"
            kind = "buzz"
            project = "agents"
            communities = ["team:research", "team:research"]
            community_names = ["Research", "Research Again"]
            "#,
            r#"
            [[sources]]
            name = "agent-buzz"
            kind = "buzz"
            project = "agents"
            communities = ["team:research"]
            community_names = [""]
            "#,
            r#"
            [[sources]]
            name = "team-slack"
            kind = "slack"
            project = "work"
            communities = ["team:research"]
            community_names = ["Research"]
            "#,
        ] {
            let config: Config = toml::from_str(source_block).expect("fixture config");
            assert!(validate_source_definitions(&config).is_err());
        }
    }

    #[test]
    fn rejects_unrepresentable_validation_freshness_bounds() {
        let mut config = Config::default();
        config.ingestion.validation_max_age_hours = MAX_VALIDATION_MAX_AGE_HOURS + 1;
        let error = validate_source_definitions(&config).expect_err("bound must fit chrono");
        assert!(error.to_string().contains("validation_max_age_hours"));
    }

    #[test]
    fn rejects_unrepresentable_sync_freshness_bounds() {
        let mut config = Config::default();
        config.ingestion.sync_freshness_hours = MAX_SYNC_FRESHNESS_HOURS + 1;
        let error = validate_source_definitions(&config).expect_err("bound must fit chrono");
        assert!(error.to_string().contains("sync_freshness_hours"));
    }

    #[test]
    fn reserves_enough_memory_for_the_default_local_embedding_model() {
        assert_eq!(Config::default().embedding.service.memory_limit_mb, 4_096);
    }

    #[test]
    fn allows_time_for_the_default_local_embedding_model_to_start() {
        assert_eq!(
            Config::default().embedding.service.startup_timeout_seconds,
            300
        );
    }

    #[test]
    fn provider_urls_allow_loopback_http_and_require_secure_remote_transport() {
        assert!(validate_provider_base_url("embedding", "http://127.0.0.1:6999/v1").is_ok());
        assert!(validate_provider_base_url("query", "https://api.example.test/v1").is_ok());
        for value in [
            "http://api.example.test/v1",
            "https://user:secret@api.example.test/v1",
            "https://api.example.test/v1?token=secret",
            "https://api.example.test/v1#fragment",
        ] {
            assert!(
                validate_provider_base_url("provider", value).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn parses_workspace_metadata_without_changing_source_scopes() {
        let config: Config = toml::from_str(
            r##"
            [[workspaces]]
            id = "personal"
            name = "Personal"
            account_label = "me@example.com"
            color = "#E8A83B"

            [[sources]]
            name = "mail"
            kind = "gmail"
            project = "personal"
            "##,
        )
        .expect("valid workspace configuration");

        assert_eq!(config.workspaces.len(), 1);
        assert_eq!(config.workspaces[0].id, "personal");
        assert_eq!(
            config.workspaces[0].account_label.as_deref(),
            Some("me@example.com")
        );
        assert_eq!(config.sources[0].project, "personal");
    }

    #[test]
    fn buzz_community_assignment_is_bounded_and_aligned() {
        let mut config = Config::default();
        config.sources.push(SourceConfig {
            name: "agent-buzz".into(),
            kind: "buzz".into(),
            enabled: true,
            project: "agents".into(),
            root: Some(PathBuf::from(
                "/Users/example/Library/Application Support/xyz.block.buzz.app",
            )),
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            teams: Vec::new(),
            team_names: Vec::new(),
            communities: vec!["builtin-team:welcome".into(), "team:research".into()],
            community_names: vec!["Welcome Team".into(), "Research".into()],
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
        validate_source_definitions(&config).expect("aligned Buzz assignment is valid");

        config.sources[0].communities.push("x".repeat(129));
        config.sources[0].community_names.push("Oversized".into());
        let error = validate_source_definitions(&config).expect_err("oversized id must fail");
        assert!(error.to_string().contains("invalid Buzz community id"));
        config.sources[0].communities.pop();
        config.sources[0].community_names.pop();

        config.sources[0].community_names = Vec::new();
        let error = validate_source_definitions(&config).expect_err("misaligned names must fail");
        assert!(error.to_string().contains("index-aligned"));
        config.sources[0].community_names = vec!["Welcome Team".into(), "Research".into()];

        config.sources[0].communities[0] = "team:research".into();
        let error = validate_source_definitions(&config).expect_err("duplicate id must fail");
        assert!(error.to_string().contains("more than once"));
        config.sources[0].communities[0] = "builtin-team:welcome".into();

        config.sources[0].kind = "slack".into();
        let error = validate_source_definitions(&config).expect_err("non-buzz kind must fail");
        assert!(error.to_string().contains("only for kind `buzz`"));

        config.sources[0].kind = "buzz".into();
        config.sources[0].communities = (0..=MAX_BUZZ_COMMUNITIES)
            .map(|index| format!("team-{index:03}"))
            .collect();
        config.sources[0].community_names = config.sources[0]
            .communities
            .iter()
            .map(|id| format!("Team {id}"))
            .collect();
        let error = validate_source_definitions(&config).expect_err("count bound must fail");
        assert!(error.to_string().contains("more than 100 Buzz communities"));
    }

    #[test]
    fn source_acl_defaults_to_its_workspace_and_preserves_explicit_labels() {
        let mut source = SourceConfig {
            name: "notes".into(),
            kind: "apple-notes".into(),
            enabled: true,
            project: "personal".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            teams: Vec::new(),
            team_names: Vec::new(),
            communities: Vec::new(),
            community_names: Vec::new(),
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
        assert_eq!(source.effective_acl(), vec!["personal"]);

        source.acl = vec!["shared".into(), "personal".into(), "shared".into()];
        assert_eq!(source.effective_acl(), vec!["shared", "personal"]);
    }

    #[cfg(unix)]
    #[test]
    fn resolves_relative_environment_files_against_the_config_directory() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let config_path = directory.path().join("config.toml");
        let secret_path = directory.path().join("secrets.env");
        std::fs::write(&secret_path, "CORTANA_RELATIVE_SECRET=loaded\n").expect("write secrets");
        std::fs::set_permissions(&secret_path, std::fs::Permissions::from_mode(0o600))
            .expect("secure secrets");
        std::fs::write(&config_path, "[runtime]\nenv_file = \"secrets.env\"\n")
            .expect("write config");

        let mut config = Config::load(Some(&config_path)).expect("load config");
        config.load_environment().expect("load relative secrets");
        assert_eq!(
            config.runtime.env_file.as_deref(),
            Some(secret_path.as_path())
        );
        assert_eq!(
            config.environment.get("CORTANA_RELATIVE_SECRET"),
            Some(&"loaded".into())
        );
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

    #[cfg(unix)]
    #[test]
    fn rejects_ambiguous_or_untransportable_environment_entries() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("secrets.env");
        std::fs::write(
            &path,
            b"CORTANA_DUPLICATE=first\nCORTANA_DUPLICATE=second\n",
        )
        .expect("write duplicate secrets");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("secure duplicate fixture");
        let mut config = Config {
            runtime: RuntimeConfig {
                env_file: Some(path.clone()),
            },
            ..Config::default()
        };
        let duplicate = config
            .load_environment()
            .expect_err("duplicate environment names must fail closed");
        assert!(
            duplicate
                .to_string()
                .contains("duplicate environment variable")
        );

        std::fs::write(&path, b"CORTANA_NUL=bad\0value\n").expect("write NUL secret");
        let nul = config
            .load_environment()
            .expect_err("NUL environment values cannot cross a process boundary");
        assert!(nul.to_string().contains("contains NUL"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_environment_files() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let directory = tempfile::tempdir().expect("temporary directory");
        let target = directory.path().join("real-secrets.env");
        std::fs::write(&target, "CORTANA_TEST_LINKED=secret\n").expect("write secrets");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))
            .expect("set permissions");
        let linked = directory.path().join("secrets.env");
        symlink(&target, &linked).expect("symlink secrets");
        let mut config = Config {
            runtime: RuntimeConfig {
                env_file: Some(linked),
            },
            ..Config::default()
        };

        let error = config
            .load_environment()
            .expect_err("symlinked environment files must fail");
        assert!(error.to_string().contains("must not be a symlink"));
    }
}
