use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use toml::{Table, Value};

const MAX_WORKSPACES: usize = 3;
const MAX_SOURCES: usize = 128;
const MAX_AUTH_PRINCIPALS: usize = 64;
const MAX_SECRET_BYTES: usize = 16 * 1024;
const MAX_AUDIT_READ_BYTES: u64 = 2 * 1024 * 1024;
const SOURCE_KINDS: &[&str] = &[
    "filesystem",
    "apple-notes",
    "buzz",
    "google-drive",
    "gmail",
    "google-calendar",
    "slack",
    "discord",
    "external",
];

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSettings {
    pub id: String,
    pub name: String,
    pub account_label: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingSettings {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub dimension: usize,
    pub cache_max_entries: usize,
    pub request_timeout_seconds: u64,
    pub request_concurrency: usize,
    pub startup_timeout_seconds: u64,
    pub memory_limit_mb: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QuerySettings {
    pub synthesis_enabled: bool,
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub max_planned_queries: usize,
    pub retrieval_limit: usize,
    pub result_limit: usize,
    pub context_tokens: usize,
    pub output_tokens: usize,
    pub request_timeout_seconds: u64,
    pub answer_timeout_seconds: u64,
    pub request_concurrency: usize,
    pub cache_max_entries: usize,
    pub cache_ttl_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IngestionSettings {
    pub max_documents_per_source: usize,
    pub max_bytes_per_source: u64,
    pub max_duration_seconds: u64,
    pub document_batch_size: usize,
    pub request_concurrency: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSettings {
    pub data_dir: String,
    pub connector_timeout_seconds: u64,
    pub audit_max_events: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSettings {
    pub name: String,
    pub kind: String,
    pub enabled: bool,
    pub project: String,
    pub root: Option<String>,
    pub source: Option<String>,
    pub channels: Vec<String>,
    pub token_env: Option<String>,
    pub token_path: Option<String>,
    pub oauth_client_path: Option<String>,
    pub query: Option<String>,
    pub labels: Vec<String>,
    pub max_content_chars: Option<usize>,
    pub max_documents: Option<usize>,
    pub max_bytes: Option<u64>,
    pub max_duration_seconds: Option<u64>,
    pub exclude: Vec<String>,
    pub acl: Vec<String>,
    pub editable: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthPrincipalSettings {
    pub principal: String,
    pub token_env: String,
    pub scopes: Vec<String>,
    pub acl: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretUpdate {
    pub name: String,
    pub value: Option<String>,
    #[serde(default)]
    pub clear: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsUpdate {
    pub workspaces: Vec<WorkspaceSettings>,
    pub sources: Vec<SourceSettings>,
    pub auth_principals: Vec<AuthPrincipalSettings>,
    pub embedding: EmbeddingSettings,
    pub query: QuerySettings,
    pub ingestion: IngestionSettings,
    pub runtime: RuntimeSettings,
    #[serde(default)]
    pub secrets: Vec<SecretUpdate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecretState {
    pub name: String,
    pub configured: bool,
    pub source: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsSnapshot {
    pub config_path: String,
    pub secret_file_path: String,
    pub needs_setup: bool,
    pub restart_required: bool,
    pub workspaces: Vec<WorkspaceSettings>,
    pub sources: Vec<SourceSettings>,
    pub auth_principals: Vec<AuthPrincipalSettings>,
    pub embedding: EmbeddingSettings,
    pub query: QuerySettings,
    pub ingestion: IngestionSettings,
    pub runtime: RuntimeSettings,
    pub secrets: Vec<SecretState>,
}

pub fn load() -> Result<SettingsSnapshot, String> {
    SettingsStore::default().load()
}

pub fn save(update: SettingsUpdate) -> Result<SettingsSnapshot, String> {
    SettingsStore::default().save(update)
}

pub fn configured_source(name: &str) -> Result<SourceSettings, String> {
    validate_source_name(name)?;
    load()?
        .sources
        .into_iter()
        .find(|source| source.name == name)
        .ok_or_else(|| format!("configured source `{name}` was not found"))
}

pub(crate) fn bearer_for_scope(scope: &str) -> Result<Option<String>, String> {
    bearer_for_scope_at(&default_config_path(), scope)
}

fn bearer_for_scope_at(config_path: &Path, scope: &str) -> Result<Option<String>, String> {
    if !matches!(scope, "query" | "status" | "admin") {
        return Err("unsupported desktop bearer scope".into());
    }
    let root = read_config(config_path)?;
    let principals = configured_auth_principals(&root);
    if principals.is_empty() {
        return Ok(None);
    }
    let secret_path = secret_path(&root, config_path)?;
    let secrets = read_secret_map(&secret_path)?;
    for principal in principals
        .iter()
        .filter(|principal| principal.scopes.iter().any(|value| value == scope))
    {
        if let Some(value) = secrets
            .get(&principal.token_env)
            .cloned()
            .or_else(|| std::env::var(&principal.token_env).ok())
            .filter(|value| !value.is_empty())
        {
            return Ok(Some(value));
        }
    }
    Err(format!(
        "no configured desktop auth principal provides the `{scope}` scope"
    ))
}

pub fn desktop_audit_events(limit: usize) -> Result<Vec<serde_json::Value>, String> {
    desktop_audit_events_at(&default_config_path(), limit)
}

fn desktop_audit_events_at(
    config_path: &Path,
    limit: usize,
) -> Result<Vec<serde_json::Value>, String> {
    if !(1..=500).contains(&limit) {
        return Err("desktop audit limit must be between 1 and 500".into());
    }
    let directory = config_path
        .parent()
        .ok_or_else(|| "config path has no parent".to_string())?;
    let path = directory.join("desktop-audit.jsonl");
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(format!("refusing to use symlinked file {}", path.display()));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("inspect desktop audit log: {error}")),
    }
    let mut file = OpenOptions::new()
        .read(true)
        .open(&path)
        .map_err(|error| format!("open desktop audit log: {error}"))?;
    let length = file
        .metadata()
        .map_err(|error| format!("inspect desktop audit log: {error}"))?
        .len();
    let offset = length.saturating_sub(MAX_AUDIT_READ_BYTES);
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| format!("seek desktop audit log: {error}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_AUDIT_READ_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read desktop audit log: {error}"))?;
    let body = String::from_utf8_lossy(&bytes);
    let mut lines = body.lines();
    if offset > 0 {
        let _ = lines.next();
    }
    Ok(lines
        .rev()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| {
            event.is_object()
                && event
                    .get("secret_values_recorded")
                    .and_then(serde_json::Value::as_bool)
                    == Some(false)
        })
        .take(limit)
        .collect())
}

#[derive(Debug, Clone)]
struct SettingsStore {
    config_path: PathBuf,
}

impl Default for SettingsStore {
    fn default() -> Self {
        Self {
            config_path: default_config_path(),
        }
    }
}

impl SettingsStore {
    fn load(&self) -> Result<SettingsSnapshot, String> {
        let existed = self.config_path.exists();
        let root = read_config(&self.config_path)?;
        let secret_path = secret_path(&root, &self.config_path)?;
        let secrets = read_secret_map(&secret_path)?;
        Ok(snapshot(
            &root,
            &self.config_path,
            &secret_path,
            &secrets,
            !existed,
        ))
    }

    fn save(&self, mut update: SettingsUpdate) -> Result<SettingsSnapshot, String> {
        validate_update(&mut update)?;
        reject_symlink(&self.config_path)?;

        let mut root = read_config(&self.config_path)?;
        let secret_path = secret_path(&root, &self.config_path)?;
        let previous_auth_tokens = configured_auth_principals(&root)
            .into_iter()
            .map(|principal| principal.token_env)
            .collect::<BTreeSet<_>>();
        let next_auth_tokens = update
            .auth_principals
            .iter()
            .map(|principal| principal.token_env.clone())
            .collect::<BTreeSet<_>>();
        let removed_auth_tokens = previous_auth_tokens
            .difference(&next_auth_tokens)
            .cloned()
            .collect::<Vec<_>>();
        validate_mutable_sections(&root)?;
        validate_external_sources(&root, &update.sources)?;
        apply_update(&mut root, &update, &secret_path);

        if !update.secrets.is_empty() || !removed_auth_tokens.is_empty() {
            ensure_managed_secret_path(&secret_path, &self.config_path)?;
            let mut secrets = read_secret_map(&secret_path)?;
            apply_secret_updates(&mut secrets, &update.secrets)?;
            for name in &removed_auth_tokens {
                secrets.remove(name);
            }
            atomic_write(&secret_path, render_secrets(&secrets).as_bytes())?;
        }

        let rendered = toml::to_string_pretty(&root)
            .map_err(|error| format!("serialize settings: {error}"))?;
        if self.config_path.exists() {
            let backup = self.config_path.with_extension("toml.backup");
            fs::copy(&self.config_path, &backup)
                .map_err(|error| format!("back up Cortana settings: {error}"))?;
            set_owner_only(&backup)?;
        }
        atomic_write(&self.config_path, rendered.as_bytes())?;
        append_audit(&self.config_path, &update)?;
        self.load().map(|mut state| {
            state.restart_required = true;
            state
        })
    }
}

fn snapshot(
    root: &Table,
    config_path: &Path,
    secret_path: &Path,
    secrets: &BTreeMap<String, String>,
    needs_setup: bool,
) -> SettingsSnapshot {
    let workspaces = configured_workspaces(root);
    let sources = configured_sources(root);
    let auth_principals = configured_auth_principals(root);
    let embedding_api_key_env = optional_string(root, "embedding", "api_key_env");
    let query_api_key_env = optional_string(root, "query", "api_key_env");
    let mut secret_names = BTreeSet::new();
    secret_names.extend(embedding_api_key_env.iter().cloned());
    secret_names.extend(query_api_key_env.iter().cloned());
    secret_names.extend(
        sources
            .iter()
            .filter_map(|source| source.token_env.as_ref().cloned()),
    );
    secret_names.extend(
        auth_principals
            .iter()
            .map(|principal| principal.token_env.clone()),
    );

    SettingsSnapshot {
        config_path: config_path.display().to_string(),
        secret_file_path: secret_path.display().to_string(),
        needs_setup,
        restart_required: false,
        workspaces,
        sources,
        auth_principals,
        embedding: EmbeddingSettings {
            provider: provider_kind(string(
                root,
                "embedding",
                "base_url",
                "http://127.0.0.1:6999/v1",
            )),
            base_url: string(root, "embedding", "base_url", "http://127.0.0.1:6999/v1"),
            model: string(root, "embedding", "model", "Qwen/Qwen3-Embedding-0.6B"),
            api_key_env: embedding_api_key_env,
            dimension: usize_value(root, "embedding", "dimension", 1024),
            cache_max_entries: usize_value(root, "embedding", "cache_max_entries", 250_000),
            request_timeout_seconds: u64_value(root, "embedding", "request_timeout_seconds", 180),
            request_concurrency: usize_value(root, "embedding", "request_concurrency", 4),
            startup_timeout_seconds: nested_u64(
                root,
                "embedding",
                "service",
                "startup_timeout_seconds",
                120,
            ),
            memory_limit_mb: nested_u64(root, "embedding", "service", "memory_limit_mb", 4096),
        },
        query: QuerySettings {
            synthesis_enabled: bool_value(root, "query", "synthesis_enabled", false),
            provider: provider_kind(string(
                root,
                "query",
                "base_url",
                "http://127.0.0.1:8008/v1",
            )),
            base_url: string(root, "query", "base_url", "http://127.0.0.1:8008/v1"),
            model: string(root, "query", "model", "auto-efficient"),
            api_key_env: query_api_key_env,
            max_planned_queries: usize_value(root, "query", "max_planned_queries", 4),
            retrieval_limit: usize_value(root, "query", "retrieval_limit", 10),
            result_limit: usize_value(root, "query", "result_limit", 20),
            context_tokens: usize_value(root, "query", "context_tokens", 8000),
            output_tokens: usize_value(root, "query", "output_tokens", 1200),
            request_timeout_seconds: u64_value(root, "query", "request_timeout_seconds", 45),
            answer_timeout_seconds: u64_value(root, "query", "answer_timeout_seconds", 55),
            request_concurrency: usize_value(root, "query", "request_concurrency", 4),
            cache_max_entries: usize_value(root, "query", "cache_max_entries", 10_000),
            cache_ttl_seconds: u64_value(root, "query", "cache_ttl_seconds", 3600),
        },
        ingestion: IngestionSettings {
            max_documents_per_source: usize_value(
                root,
                "ingestion",
                "max_documents_per_source",
                2000,
            ),
            max_bytes_per_source: u64_value(
                root,
                "ingestion",
                "max_bytes_per_source",
                128 * 1024 * 1024,
            ),
            max_duration_seconds: u64_value(root, "ingestion", "max_duration_seconds", 900),
            document_batch_size: usize_value(root, "ingestion", "document_batch_size", 16),
            request_concurrency: usize_value(root, "ingestion", "request_concurrency", 1),
        },
        runtime: RuntimeSettings {
            data_dir: top_string(root, "data_dir", &default_data_path().display().to_string()),
            connector_timeout_seconds: u64_value(root, "connectors", "timeout_seconds", 21_600),
            audit_max_events: usize_value(root, "auth", "audit_max_events", 10_000),
        },
        secrets: secret_names
            .into_iter()
            .map(|name| SecretState {
                configured: secrets.contains_key(&name) || std::env::var_os(&name).is_some(),
                source: if secrets.contains_key(&name) {
                    "secret-file"
                } else if std::env::var_os(&name).is_some() {
                    "environment"
                } else {
                    "unset"
                },
                name,
            })
            .collect(),
    }
}

fn configured_auth_principals(root: &Table) -> Vec<AuthPrincipalSettings> {
    table(root, "auth")
        .and_then(|auth| auth.get("tokens"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_table)
                .filter_map(|item| {
                    Some(AuthPrincipalSettings {
                        principal: item.get("principal")?.as_str()?.to_string(),
                        token_env: item.get("token_env")?.as_str()?.to_string(),
                        scopes: table_string_array(item, "scopes"),
                        acl: table_string_array(item, "acl"),
                    })
                })
                .take(MAX_AUTH_PRINCIPALS)
                .collect()
        })
        .unwrap_or_default()
}

fn configured_sources(root: &Table) -> Vec<SourceSettings> {
    root.get("sources")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_table)
                .filter_map(|item| {
                    let kind = item.get("kind")?.as_str()?.to_string();
                    Some(SourceSettings {
                        name: item.get("name")?.as_str()?.to_string(),
                        editable: kind != "external",
                        kind,
                        enabled: item.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                        project: item
                            .get("project")
                            .and_then(Value::as_str)
                            .unwrap_or("default")
                            .to_string(),
                        root: table_optional_string(item, "root"),
                        source: table_optional_string(item, "source"),
                        channels: table_string_array(item, "channels"),
                        token_env: table_optional_string(item, "token_env"),
                        token_path: table_optional_string(item, "token"),
                        oauth_client_path: table_optional_string(item, "oauth_client"),
                        query: table_optional_string(item, "query"),
                        labels: table_string_array(item, "labels"),
                        max_content_chars: table_optional_usize(item, "max_content_chars"),
                        max_documents: table_optional_usize(item, "max_documents"),
                        max_bytes: table_optional_u64(item, "max_bytes"),
                        max_duration_seconds: table_optional_u64(item, "max_duration_seconds"),
                        exclude: table_string_array(item, "exclude"),
                        acl: table_string_array(item, "acl"),
                    })
                })
                .take(MAX_SOURCES)
                .collect()
        })
        .unwrap_or_default()
}

fn configured_workspaces(root: &Table) -> Vec<WorkspaceSettings> {
    let explicit = root
        .get("workspaces")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_table)
                .filter_map(|item| {
                    Some(WorkspaceSettings {
                        id: item.get("id")?.as_str()?.to_string(),
                        name: item.get("name")?.as_str()?.to_string(),
                        account_label: item
                            .get("account_label")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        color: item
                            .get("color")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .take(MAX_WORKSPACES)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !explicit.is_empty() {
        return explicit;
    }

    let mut projects = BTreeSet::new();
    if let Some(sources) = root.get("sources").and_then(Value::as_array) {
        for source in sources {
            if let Some(project) = source
                .as_table()
                .and_then(|table| table.get("project"))
                .and_then(Value::as_str)
            {
                projects.insert(project.to_string());
            }
        }
    }
    let derived = projects
        .into_iter()
        .take(MAX_WORKSPACES)
        .map(|id| WorkspaceSettings {
            name: title_case(&id),
            id,
            account_label: None,
            color: None,
        })
        .collect::<Vec<_>>();
    if !derived.is_empty() {
        return derived;
    }
    ["personal", "work", "special"]
        .into_iter()
        .map(|id| WorkspaceSettings {
            id: id.to_string(),
            name: title_case(id),
            account_label: None,
            color: None,
        })
        .collect()
}

fn validate_update(update: &mut SettingsUpdate) -> Result<(), String> {
    if update.workspaces.is_empty() || update.workspaces.len() > MAX_WORKSPACES {
        return Err(format!(
            "configure between 1 and {MAX_WORKSPACES} workspaces"
        ));
    }
    let mut ids = BTreeSet::new();
    for workspace in &mut update.workspaces {
        workspace.id = workspace.id.trim().to_ascii_lowercase();
        workspace.name = workspace.name.trim().to_string();
        validate_workspace_id(&workspace.id)?;
        bounded_text("workspace name", &workspace.name, 80)?;
        if !ids.insert(workspace.id.clone()) {
            return Err(format!("workspace id `{}` is duplicated", workspace.id));
        }
        if let Some(label) = &mut workspace.account_label {
            *label = label.trim().to_string();
            bounded_text("account label", label, 128)?;
        }
        if let Some(color) = &workspace.color
            && (!color.starts_with('#')
                || color.len() != 7
                || !color[1..]
                    .chars()
                    .all(|character| character.is_ascii_hexdigit()))
        {
            return Err("workspace colors must use #RRGGBB".into());
        }
    }
    validate_sources(&mut update.sources, &ids)?;
    validate_auth_principals(&mut update.auth_principals)?;

    validate_provider("embedding", &update.embedding.provider)?;
    validate_url("embedding", &update.embedding.base_url)?;
    validate_provider_url(
        "embedding",
        &update.embedding.provider,
        &update.embedding.base_url,
    )?;
    bounded_text("embedding model", update.embedding.model.trim(), 256)?;
    validate_optional_env(&update.embedding.api_key_env)?;
    bounded("embedding dimension", update.embedding.dimension, 1, 65_536)?;
    bounded(
        "embedding cache entries",
        update.embedding.cache_max_entries,
        100,
        5_000_000,
    )?;
    bounded(
        "embedding concurrency",
        update.embedding.request_concurrency,
        1,
        64,
    )?;
    bounded_u64(
        "embedding request timeout",
        update.embedding.request_timeout_seconds,
        1,
        3600,
    )?;
    bounded_u64(
        "embedding startup timeout",
        update.embedding.startup_timeout_seconds,
        1,
        3600,
    )?;
    bounded(
        "embedding memory limit",
        update.embedding.memory_limit_mb as usize,
        256,
        262_144,
    )?;

    validate_provider("query", &update.query.provider)?;
    validate_url("query", &update.query.base_url)?;
    validate_provider_url("query", &update.query.provider, &update.query.base_url)?;
    bounded_text("query model", update.query.model.trim(), 256)?;
    validate_optional_env(&update.query.api_key_env)?;
    bounded("planned queries", update.query.max_planned_queries, 1, 8)?;
    bounded("retrieval limit", update.query.retrieval_limit, 1, 100)?;
    bounded("result limit", update.query.result_limit, 1, 50)?;
    bounded("context tokens", update.query.context_tokens, 256, 131_072)?;
    bounded("output tokens", update.query.output_tokens, 64, 32_768)?;
    bounded("query concurrency", update.query.request_concurrency, 1, 32)?;
    bounded_u64(
        "query request timeout",
        update.query.request_timeout_seconds,
        1,
        600,
    )?;
    bounded_u64(
        "answer timeout",
        update.query.answer_timeout_seconds,
        1,
        600,
    )?;
    bounded(
        "query cache entries",
        update.query.cache_max_entries,
        100,
        1_000_000,
    )?;
    bounded_u64(
        "query cache lifetime",
        update.query.cache_ttl_seconds,
        1,
        604_800,
    )?;

    bounded(
        "documents per source",
        update.ingestion.max_documents_per_source,
        1,
        1_000_000,
    )?;
    bounded(
        "document batch size",
        update.ingestion.document_batch_size,
        1,
        2048,
    )?;
    bounded(
        "ingestion concurrency",
        update.ingestion.request_concurrency,
        1,
        32,
    )?;
    bounded_u64(
        "bytes per source",
        update.ingestion.max_bytes_per_source,
        1024,
        1024 * 1024 * 1024 * 1024,
    )?;
    bounded_u64(
        "ingestion duration",
        update.ingestion.max_duration_seconds,
        1,
        86_400,
    )?;
    bounded_u64(
        "connector timeout",
        update.runtime.connector_timeout_seconds,
        1,
        86_400,
    )?;
    bounded(
        "audit events",
        update.runtime.audit_max_events,
        100,
        1_000_000,
    )?;
    let data_path = Path::new(update.runtime.data_dir.trim());
    if !data_path.is_absolute() || data_path.parent().is_none() {
        return Err("data directory must be an absolute non-root path".into());
    }
    update.runtime.data_dir = data_path.display().to_string();
    for secret in &update.secrets {
        validate_env_name(&secret.name)?;
        if secret.clear && secret.value.is_some() {
            return Err(format!(
                "secret `{}` cannot be set and cleared together",
                secret.name
            ));
        }
        if let Some(value) = &secret.value {
            if value.is_empty()
                || value.len() > MAX_SECRET_BYTES
                || value.contains(['\n', '\r', '\0'])
                || value.trim() != value
                || value.starts_with(['"', '\''])
                || value.ends_with(['"', '\''])
            {
                return Err(format!("secret `{}` has an invalid value", secret.name));
            }
        }
    }
    Ok(())
}

fn apply_update(root: &mut Table, update: &SettingsUpdate, secret_path: &Path) {
    root.insert(
        "data_dir".into(),
        Value::String(update.runtime.data_dir.clone()),
    );
    root.insert(
        "workspaces".into(),
        Value::Array(
            update
                .workspaces
                .iter()
                .map(|workspace| {
                    let mut table = Table::new();
                    table.insert("id".into(), Value::String(workspace.id.clone()));
                    table.insert("name".into(), Value::String(workspace.name.clone()));
                    insert_optional_string(&mut table, "account_label", &workspace.account_label);
                    insert_optional_string(&mut table, "color", &workspace.color);
                    Value::Table(table)
                })
                .collect(),
        ),
    );
    apply_sources(root, &update.sources);
    apply_auth_principals(root, &update.auth_principals);

    set_string(root, "embedding", "base_url", &update.embedding.base_url);
    set_string(root, "embedding", "model", &update.embedding.model);
    set_optional_string(
        root,
        "embedding",
        "api_key_env",
        &update.embedding.api_key_env,
    );
    set_integer(
        root,
        "embedding",
        "dimension",
        update.embedding.dimension as i64,
    );
    set_integer(
        root,
        "embedding",
        "cache_max_entries",
        update.embedding.cache_max_entries as i64,
    );
    set_integer(
        root,
        "embedding",
        "request_timeout_seconds",
        update.embedding.request_timeout_seconds as i64,
    );
    set_integer(
        root,
        "embedding",
        "request_concurrency",
        update.embedding.request_concurrency as i64,
    );
    set_nested_integer(
        root,
        "embedding",
        "service",
        "startup_timeout_seconds",
        update.embedding.startup_timeout_seconds as i64,
    );
    set_nested_integer(
        root,
        "embedding",
        "service",
        "memory_limit_mb",
        update.embedding.memory_limit_mb as i64,
    );

    set_bool(
        root,
        "query",
        "synthesis_enabled",
        update.query.synthesis_enabled,
    );
    set_string(root, "query", "base_url", &update.query.base_url);
    set_string(root, "query", "model", &update.query.model);
    set_optional_string(root, "query", "api_key_env", &update.query.api_key_env);
    for (key, value) in [
        (
            "max_planned_queries",
            update.query.max_planned_queries as i64,
        ),
        ("retrieval_limit", update.query.retrieval_limit as i64),
        ("result_limit", update.query.result_limit as i64),
        ("context_tokens", update.query.context_tokens as i64),
        ("output_tokens", update.query.output_tokens as i64),
        (
            "request_timeout_seconds",
            update.query.request_timeout_seconds as i64,
        ),
        (
            "answer_timeout_seconds",
            update.query.answer_timeout_seconds as i64,
        ),
        (
            "request_concurrency",
            update.query.request_concurrency as i64,
        ),
        ("cache_max_entries", update.query.cache_max_entries as i64),
        ("cache_ttl_seconds", update.query.cache_ttl_seconds as i64),
    ] {
        set_integer(root, "query", key, value);
    }

    for (key, value) in [
        (
            "max_documents_per_source",
            update.ingestion.max_documents_per_source as i64,
        ),
        (
            "max_bytes_per_source",
            update.ingestion.max_bytes_per_source as i64,
        ),
        (
            "max_duration_seconds",
            update.ingestion.max_duration_seconds as i64,
        ),
        (
            "document_batch_size",
            update.ingestion.document_batch_size as i64,
        ),
        (
            "request_concurrency",
            update.ingestion.request_concurrency as i64,
        ),
    ] {
        set_integer(root, "ingestion", key, value);
    }
    set_integer(
        root,
        "connectors",
        "timeout_seconds",
        update.runtime.connector_timeout_seconds as i64,
    );
    set_integer(
        root,
        "auth",
        "audit_max_events",
        update.runtime.audit_max_events as i64,
    );
    if !update.secrets.is_empty() {
        set_string(
            root,
            "runtime",
            "env_file",
            &secret_path.display().to_string(),
        );
    }
}

fn apply_auth_principals(root: &mut Table, principals: &[AuthPrincipalSettings]) {
    let rendered = principals
        .iter()
        .map(|principal| {
            let mut table = Table::new();
            table.insert(
                "principal".into(),
                Value::String(principal.principal.clone()),
            );
            table.insert(
                "token_env".into(),
                Value::String(principal.token_env.clone()),
            );
            set_table_string_array(&mut table, "scopes", &principal.scopes);
            set_table_string_array(&mut table, "acl", &principal.acl);
            Value::Table(table)
        })
        .collect();
    mutable_table(root, "auth").insert("tokens".into(), Value::Array(rendered));
}

fn validate_auth_principals(principals: &mut [AuthPrincipalSettings]) -> Result<(), String> {
    if principals.len() > MAX_AUTH_PRINCIPALS {
        return Err(format!(
            "configure no more than {MAX_AUTH_PRINCIPALS} auth principals"
        ));
    }
    let mut names = BTreeSet::new();
    let mut token_envs = BTreeSet::new();
    for principal in principals {
        principal.principal = principal.principal.trim().to_string();
        principal.token_env = principal.token_env.trim().to_string();
        if principal.principal.is_empty()
            || principal.principal.len() > 128
            || !principal.principal.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(character, '-' | '_' | '.' | '@' | ':')
            })
        {
            return Err(
                "auth principal names must use 1-128 letters, numbers, or . _ @ : -".into(),
            );
        }
        if !names.insert(principal.principal.clone()) {
            return Err(format!(
                "auth principal `{}` is duplicated",
                principal.principal
            ));
        }
        validate_env_name(&principal.token_env)?;
        if !token_envs.insert(principal.token_env.clone()) {
            return Err(format!(
                "auth token environment `{}` is reused",
                principal.token_env
            ));
        }
        normalize_string_list("auth scope", &mut principal.scopes, 3, 16)?;
        if principal.scopes.is_empty()
            || principal
                .scopes
                .iter()
                .any(|scope| !matches!(scope.as_str(), "query" | "status" | "admin"))
        {
            return Err(format!(
                "auth principal `{}` must have query, status, or admin scopes",
                principal.principal
            ));
        }
        normalize_string_list("auth ACL", &mut principal.acl, 100, 128)?;
    }
    Ok(())
}

fn apply_sources(root: &mut Table, sources: &[SourceSettings]) {
    let mut existing = root
        .get("sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_table)
        .filter_map(|table| Some((table.get("name")?.as_str()?.to_string(), table.to_owned())))
        .collect::<BTreeMap<_, _>>();
    let rendered = sources
        .iter()
        .map(|source| {
            let mut table = existing.remove(&source.name).unwrap_or_default();
            table.insert("name".into(), Value::String(source.name.clone()));
            table.insert("kind".into(), Value::String(source.kind.clone()));
            table.insert("enabled".into(), Value::Boolean(source.enabled));
            table.insert("project".into(), Value::String(source.project.clone()));
            if source.kind != "external" {
                set_table_optional_string(&mut table, "root", &source.root);
                set_table_optional_string(&mut table, "source", &source.source);
                set_table_string_array(&mut table, "channels", &source.channels);
                set_table_optional_string(&mut table, "token_env", &source.token_env);
                set_table_optional_string(&mut table, "token", &source.token_path);
                set_table_optional_string(&mut table, "oauth_client", &source.oauth_client_path);
                set_table_optional_string(&mut table, "query", &source.query);
                set_table_string_array(&mut table, "labels", &source.labels);
                set_table_optional_integer(
                    &mut table,
                    "max_content_chars",
                    source.max_content_chars.map(|value| value as i64),
                );
                set_table_optional_integer(
                    &mut table,
                    "max_documents",
                    source.max_documents.map(|value| value as i64),
                );
                set_table_optional_integer(
                    &mut table,
                    "max_bytes",
                    source.max_bytes.and_then(|value| i64::try_from(value).ok()),
                );
                set_table_optional_integer(
                    &mut table,
                    "max_duration_seconds",
                    source
                        .max_duration_seconds
                        .and_then(|value| i64::try_from(value).ok()),
                );
                set_table_string_array(&mut table, "exclude", &source.exclude);
                set_table_string_array(&mut table, "acl", &source.acl);
            }
            Value::Table(table)
        })
        .collect();
    root.insert("sources".into(), Value::Array(rendered));
}

fn apply_secret_updates(
    secrets: &mut BTreeMap<String, String>,
    updates: &[SecretUpdate],
) -> Result<(), String> {
    for update in updates {
        if update.clear {
            secrets.remove(&update.name);
        } else if let Some(value) = &update.value {
            secrets.insert(update.name.clone(), value.clone());
        }
    }
    Ok(())
}

pub(crate) fn default_config_path() -> PathBuf {
    std::env::var_os("CORTANA_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
                .unwrap_or_else(|| PathBuf::from(".config"))
                .join("cortana/config.toml")
        })
}

fn default_data_path() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("cortana")
}

fn secret_path(root: &Table, config_path: &Path) -> Result<PathBuf, String> {
    let config_dir = config_path
        .parent()
        .ok_or_else(|| "Cortana config path has no parent directory".to_string())?;
    let path = optional_string(root, "runtime", "env_file")
        .map(PathBuf::from)
        .unwrap_or_else(|| config_dir.join("secrets.env"));
    if !path.is_absolute() {
        return Ok(config_dir.join(path));
    }
    Ok(path)
}

fn ensure_managed_secret_path(path: &Path, config_path: &Path) -> Result<(), String> {
    let config_dir = config_path
        .parent()
        .ok_or_else(|| "Cortana config path has no parent directory".to_string())?;
    let managed_path = config_dir.join("secrets.env");
    if path != managed_path {
        return Err(format!(
            "the configured secret file {} is externally managed; remove runtime.env_file or update it outside Cortana Desktop",
            path.display()
        ));
    }
    Ok(())
}

fn validate_mutable_sections(root: &Table) -> Result<(), String> {
    for section in [
        "embedding",
        "query",
        "ingestion",
        "connectors",
        "auth",
        "runtime",
    ] {
        if root.get(section).is_some_and(|value| !value.is_table()) {
            return Err(format!("settings section `{section}` must be a TOML table"));
        }
    }
    if nested_table(root, "embedding", "service").is_none()
        && table(root, "embedding")
            .and_then(|section| section.get("service"))
            .is_some()
    {
        return Err("settings section `embedding.service` must be a TOML table".into());
    }
    Ok(())
}

fn validate_external_sources(root: &Table, sources: &[SourceSettings]) -> Result<(), String> {
    let existing_external = root
        .get("sources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_table)
        .filter(|source| source.get("kind").and_then(Value::as_str) == Some("external"))
        .filter_map(|source| source.get("name").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    for source in sources.iter().filter(|source| source.kind == "external") {
        if !existing_external.contains(source.name.as_str()) {
            return Err(
                "external command sources can be retained but not created in Cortana Desktop"
                    .into(),
            );
        }
    }
    Ok(())
}

fn validate_sources(
    sources: &mut [SourceSettings],
    workspace_ids: &BTreeSet<String>,
) -> Result<(), String> {
    if sources.len() > MAX_SOURCES {
        return Err(format!("configure no more than {MAX_SOURCES} sources"));
    }
    let mut names = BTreeSet::new();
    for source in sources.iter_mut() {
        source.name = source.name.trim().to_ascii_lowercase();
        source.kind = source.kind.trim().to_ascii_lowercase();
        source.project = source.project.trim().to_ascii_lowercase();
        validate_source_name(&source.name)?;
        if !names.insert(source.name.clone()) {
            return Err(format!("source name `{}` is duplicated", source.name));
        }
        if !SOURCE_KINDS.contains(&source.kind.as_str()) {
            return Err(format!("source `{}` has an unsupported kind", source.name));
        }
        if !workspace_ids.contains(&source.project) {
            return Err(format!(
                "source `{}` uses unknown workspace `{}`",
                source.name, source.project
            ));
        }
        source.editable = source.kind != "external";
        normalize_optional_text(&mut source.root);
        normalize_optional_text(&mut source.source);
        normalize_optional_text(&mut source.token_env);
        normalize_optional_text(&mut source.token_path);
        normalize_optional_text(&mut source.oauth_client_path);
        normalize_optional_text(&mut source.query);
        for (label, value, maximum) in [
            ("source root", source.root.as_deref(), 4096),
            ("source identifier", source.source.as_deref(), 128),
            ("source token environment", source.token_env.as_deref(), 64),
            ("source token path", source.token_path.as_deref(), 4096),
            (
                "source OAuth client path",
                source.oauth_client_path.as_deref(),
                4096,
            ),
            ("source query", source.query.as_deref(), 2048),
        ] {
            if let Some(value) = value {
                bounded_text(label, value, maximum)?;
                if value.contains(['\n', '\r']) {
                    return Err(format!("{label} contains a line break"));
                }
            }
        }
        normalize_string_list("source channel", &mut source.channels, 100, 128)?;
        normalize_string_list("source label", &mut source.labels, 100, 128)?;
        normalize_string_list("source ACL", &mut source.acl, 100, 128)?;
        normalize_string_list("source exclude", &mut source.exclude, 256, 512)?;
        if let Some(token_env) = &source.token_env {
            validate_env_name(token_env)?;
        }
        for exclude in &source.exclude {
            let path = Path::new(exclude);
            if path.is_absolute()
                || path.components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::ParentDir
                            | std::path::Component::RootDir
                            | std::path::Component::Prefix(_)
                    )
                })
            {
                return Err(format!(
                    "source `{}` excludes must be safe relative paths",
                    source.name
                ));
            }
        }
        for (label, value, minimum, maximum) in [
            (
                "content characters",
                source.max_content_chars.map(|value| value as u64),
                1,
                10_000_000,
            ),
            (
                "documents",
                source.max_documents.map(|value| value as u64),
                1,
                1_000_000,
            ),
            ("bytes", source.max_bytes, 1024, 1024 * 1024 * 1024 * 1024),
            ("duration", source.max_duration_seconds, 1, 86_400),
        ] {
            if let Some(value) = value {
                bounded_u64(
                    &format!("source {} {label}", source.name),
                    value,
                    minimum,
                    maximum,
                )?;
            }
        }
        validate_source_paths_and_credentials(source)?;
    }
    let filesystem_roots = sources
        .iter()
        .filter(|source| source.kind == "filesystem")
        .filter_map(|source| source.root.as_deref())
        .map(Path::new)
        .collect::<Vec<_>>();
    for source in sources.iter() {
        for (label, candidate) in [
            ("token", source.token_path.as_deref()),
            ("OAuth client", source.oauth_client_path.as_deref()),
        ] {
            let Some(candidate) = candidate.map(Path::new) else {
                continue;
            };
            if filesystem_roots
                .iter()
                .any(|root| candidate.starts_with(root))
            {
                return Err(format!(
                    "source `{}` {label} path must be outside every filesystem source root",
                    source.name
                ));
            }
        }
    }
    Ok(())
}

fn validate_source_paths_and_credentials(source: &SourceSettings) -> Result<(), String> {
    if let Some(root) = &source.root {
        validate_source_path(&source.name, "root", root)?;
    }
    if let Some(token_path) = &source.token_path {
        validate_source_path(&source.name, "token", token_path)?;
    }
    if let Some(client_path) = &source.oauth_client_path {
        validate_source_path(&source.name, "OAuth client", client_path)?;
    }
    if !source.enabled {
        return Ok(());
    }
    match source.kind.as_str() {
        "filesystem" if source.root.is_none() => {
            Err(format!("filesystem source `{}` needs a root", source.name))
        }
        "slack" | "discord" if source.channels.is_empty() || source.token_env.is_none() => {
            Err(format!(
                "{} source `{}` needs channels and a token environment name",
                source.kind, source.name
            ))
        }
        "google-drive" | "gmail" | "google-calendar"
            if source.token_path.is_none() && source.token_env.is_none() =>
        {
            Err(format!(
                "Google source `{}` needs a token file or token environment name",
                source.name
            ))
        }
        _ => Ok(()),
    }
}

fn validate_source_path(source: &str, label: &str, value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if !path.is_absolute()
        || path.parent().is_none()
        || path.parent().is_none_or(|parent| parent.parent().is_none())
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(format!(
            "source `{source}` {label} must be an absolute path outside the filesystem root"
        ));
    }
    Ok(())
}

fn validate_source_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
        || !value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    {
        return Err(
            "source names must be 1-64 lowercase letters, numbers, dashes, or underscores".into(),
        );
    }
    Ok(())
}

fn normalize_optional_text(value: &mut Option<String>) {
    if let Some(inner) = value {
        *inner = inner.trim().to_string();
        if inner.is_empty() {
            *value = None;
        }
    }
}

fn normalize_string_list(
    label: &str,
    values: &mut [String],
    maximum_items: usize,
    maximum_bytes: usize,
) -> Result<(), String> {
    if values.len() > maximum_items {
        return Err(format!("{label} has too many values"));
    }
    let mut unique = BTreeSet::new();
    for value in values.iter_mut() {
        *value = value.trim().to_string();
        bounded_text(label, value, maximum_bytes)?;
        if value.contains(['\n', '\r']) {
            return Err(format!("{label} contains a line break"));
        }
        if !unique.insert(value.clone()) {
            return Err(format!("{label} contains duplicate values"));
        }
    }
    Ok(())
}

fn read_config(path: &Path) -> Result<Table, String> {
    if !path.exists() {
        return Ok(Table::new());
    }
    reject_symlink(path)?;
    let body =
        fs::read_to_string(path).map_err(|error| format!("read Cortana settings: {error}"))?;
    toml::from_str(&body).map_err(|error| format!("parse Cortana settings: {error}"))
}

fn read_secret_map(path: &Path) -> Result<BTreeMap<String, String>, String> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    reject_symlink(path)?;
    let body = fs::read_to_string(path).map_err(|error| format!("read secret file: {error}"))?;
    let mut values = BTreeMap::new();
    for (line_number, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = line
            .split_once('=')
            .ok_or_else(|| format!("invalid secret file line {}", line_number + 1))?;
        let name = name.trim().to_string();
        validate_env_name(&name)?;
        values.insert(name, value.trim().trim_matches(['"', '\'']).to_string());
    }
    Ok(values)
}

fn render_secrets(values: &BTreeMap<String, String>) -> String {
    let mut rendered =
        String::from("# Managed by Cortana Desktop. Values are never returned to the webview.\n");
    for (name, value) in values {
        rendered.push_str(name);
        rendered.push('=');
        rendered.push_str(value);
        rendered.push('\n');
    }
    rendered
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| format!("create settings directory: {error}"))?;
    set_directory_owner_only(parent)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let temp = parent.join(format!(
        ".cortana-settings-{}-{nonce}.tmp",
        std::process::id()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .map_err(|error| format!("create temporary settings: {error}"))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("write settings: {error}"))?;
    fs::rename(&temp, path).map_err(|error| format!("replace settings: {error}"))?;
    set_owner_only(path)
}

fn append_audit(config_path: &Path, update: &SettingsUpdate) -> Result<(), String> {
    let at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let secret_names = update
        .secrets
        .iter()
        .map(|secret| secret.name.as_str())
        .collect::<Vec<_>>();
    let event = serde_json::json!({
        "at_unix_seconds": at,
        "event": "settings.updated",
        "workspace_ids": update.workspaces.iter().map(|workspace| workspace.id.as_str()).collect::<Vec<_>>(),
        "source_names": update.sources.iter().map(|source| source.name.as_str()).collect::<Vec<_>>(),
        "enabled_source_names": update.sources.iter().filter(|source| source.enabled).map(|source| source.name.as_str()).collect::<Vec<_>>(),
        "auth_principal_names": update.auth_principals.iter().map(|principal| principal.principal.as_str()).collect::<Vec<_>>(),
        "auth_token_environment_names": update.auth_principals.iter().map(|principal| principal.token_env.as_str()).collect::<Vec<_>>(),
        "secret_names": secret_names,
        "secret_values_recorded": false,
    });
    append_audit_event(config_path, &event)
}

pub(crate) fn append_audit_event(
    config_path: &Path,
    event: &serde_json::Value,
) -> Result<(), String> {
    let directory = config_path
        .parent()
        .ok_or_else(|| "config path has no parent".to_string())?;
    fs::create_dir_all(directory).map_err(|error| format!("create settings directory: {error}"))?;
    set_directory_owner_only(directory)?;
    let path = directory.join("desktop-audit.jsonl");
    reject_symlink(&path)?;
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| format!("open desktop audit log: {error}"))?;
    writeln!(file, "{event}").map_err(|error| format!("append desktop audit log: {error}"))?;
    file.sync_data()
        .map_err(|error| format!("sync desktop audit log: {error}"))?;
    set_owner_only(&path)
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    if path.exists()
        && fs::symlink_metadata(path)
            .map_err(|error| format!("inspect {}: {error}", path.display()))?
            .file_type()
            .is_symlink()
    {
        return Err(format!("refusing to use symlinked file {}", path.display()));
    }
    Ok(())
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("secure {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn set_owner_only(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_directory_owner_only(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("secure {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn set_directory_owner_only(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn provider_kind(base_url: String) -> String {
    let loopback = reqwest::Url::parse(&base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1"));
    if loopback {
        "local".into()
    } else {
        "cloud".into()
    }
}

fn validate_provider(name: &str, provider: &str) -> Result<(), String> {
    if matches!(provider, "local" | "cloud") {
        Ok(())
    } else {
        Err(format!("{name} provider must be local or cloud"))
    }
}

fn validate_url(name: &str, value: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(value).map_err(|_| format!("{name} URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{name} URL must use HTTP or HTTPS"));
    }
    let is_loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    if url.scheme() == "http" && !is_loopback {
        return Err(format!("{name} cloud URL must use HTTPS"));
    }
    Ok(())
}

fn validate_provider_url(name: &str, provider: &str, value: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(value).map_err(|_| format!("{name} URL is invalid"))?;
    let is_loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    match (provider, is_loopback) {
        ("local", true) | ("cloud", false) => Ok(()),
        ("local", false) => Err(format!("{name} local provider must use a loopback URL")),
        ("cloud", true) => Err(format!("{name} cloud provider must not use a loopback URL")),
        _ => Err(format!("{name} provider must be local or cloud")),
    }
}

fn validate_workspace_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 32
        || !value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '_')
        })
        || !value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
    {
        return Err(
            "workspace ids must be 1-32 lowercase letters, numbers, dashes, or underscores".into(),
        );
    }
    Ok(())
}

fn validate_optional_env(value: &Option<String>) -> Result<(), String> {
    if let Some(value) = value {
        validate_env_name(value)?;
    }
    Ok(())
}

fn validate_env_name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || !value.chars().all(|character| {
            character == '_' || character.is_ascii_uppercase() || character.is_ascii_digit()
        })
        || !value
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_uppercase())
    {
        return Err(format!(
            "`{value}` is not a valid environment variable name"
        ));
    }
    Ok(())
}

fn bounded(name: &str, value: usize, minimum: usize, maximum: usize) -> Result<(), String> {
    if (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(format!("{name} must be between {minimum} and {maximum}"))
    }
}

fn bounded_u64(name: &str, value: u64, minimum: u64, maximum: u64) -> Result<(), String> {
    if (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(format!("{name} must be between {minimum} and {maximum}"))
    }
}

fn bounded_text(name: &str, value: &str, maximum: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        Err(format!("{name} must be between 1 and {maximum} bytes"))
    } else {
        Ok(())
    }
}

fn table<'a>(root: &'a Table, section: &str) -> Option<&'a Table> {
    root.get(section).and_then(Value::as_table)
}

fn nested_table<'a>(root: &'a Table, section: &str, nested: &str) -> Option<&'a Table> {
    table(root, section)?.get(nested).and_then(Value::as_table)
}

fn top_string(root: &Table, key: &str, default: &str) -> String {
    root.get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn string(root: &Table, section: &str, key: &str, default: &str) -> String {
    table(root, section)
        .and_then(|table| table.get(key))
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn optional_string(root: &Table, section: &str, key: &str) -> Option<String> {
    table(root, section)
        .and_then(|table| table.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn table_optional_string(table: &Table, key: &str) -> Option<String> {
    table.get(key).and_then(Value::as_str).map(str::to_string)
}

fn table_string_array(table: &Table, key: &str) -> Vec<String> {
    table
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn table_optional_u64(table: &Table, key: &str) -> Option<u64> {
    table
        .get(key)
        .and_then(Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
}

fn table_optional_usize(table: &Table, key: &str) -> Option<usize> {
    table_optional_u64(table, key).and_then(|value| usize::try_from(value).ok())
}

fn usize_value(root: &Table, section: &str, key: &str, default: usize) -> usize {
    u64_value(root, section, key, default as u64) as usize
}

fn u64_value(root: &Table, section: &str, key: &str, default: u64) -> u64 {
    table(root, section)
        .and_then(|table| table.get(key))
        .and_then(Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(default)
}

fn nested_u64(root: &Table, section: &str, nested: &str, key: &str, default: u64) -> u64 {
    nested_table(root, section, nested)
        .and_then(|table| table.get(key))
        .and_then(Value::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(default)
}

fn bool_value(root: &Table, section: &str, key: &str, default: bool) -> bool {
    table(root, section)
        .and_then(|table| table.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(default)
}

fn mutable_table<'a>(root: &'a mut Table, section: &str) -> &'a mut Table {
    root.entry(section.to_string())
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .expect("settings section must be a table")
}

fn mutable_nested_table<'a>(root: &'a mut Table, section: &str, nested: &str) -> &'a mut Table {
    mutable_table(root, section)
        .entry(nested.to_string())
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .expect("nested settings section must be a table")
}

fn set_string(root: &mut Table, section: &str, key: &str, value: &str) {
    mutable_table(root, section).insert(key.into(), Value::String(value.into()));
}

fn set_optional_string(root: &mut Table, section: &str, key: &str, value: &Option<String>) {
    let table = mutable_table(root, section);
    if let Some(value) = value.as_ref().filter(|value| !value.is_empty()) {
        table.insert(key.into(), Value::String(value.clone()));
    } else {
        table.remove(key);
    }
}

fn insert_optional_string(table: &mut Table, key: &str, value: &Option<String>) {
    if let Some(value) = value.as_ref().filter(|value| !value.is_empty()) {
        table.insert(key.into(), Value::String(value.clone()));
    }
}

fn set_table_optional_string(table: &mut Table, key: &str, value: &Option<String>) {
    if let Some(value) = value.as_ref().filter(|value| !value.is_empty()) {
        table.insert(key.into(), Value::String(value.clone()));
    } else {
        table.remove(key);
    }
}

fn set_table_string_array(table: &mut Table, key: &str, values: &[String]) {
    if values.is_empty() {
        table.remove(key);
    } else {
        table.insert(
            key.into(),
            Value::Array(values.iter().cloned().map(Value::String).collect()),
        );
    }
}

fn set_table_optional_integer(table: &mut Table, key: &str, value: Option<i64>) {
    if let Some(value) = value {
        table.insert(key.into(), Value::Integer(value));
    } else {
        table.remove(key);
    }
}

fn set_integer(root: &mut Table, section: &str, key: &str, value: i64) {
    mutable_table(root, section).insert(key.into(), Value::Integer(value));
}

fn set_nested_integer(root: &mut Table, section: &str, nested: &str, key: &str, value: i64) {
    mutable_nested_table(root, section, nested).insert(key.into(), Value::Integer(value));
}

fn set_bool(root: &mut Table, section: &str, key: &str, value: bool) {
    mutable_table(root, section).insert(key.into(), Value::Boolean(value));
}

fn title_case(value: &str) -> String {
    let mut characters = value.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_settings(name: &str, kind: &str) -> SourceSettings {
        SourceSettings {
            name: name.into(),
            kind: kind.into(),
            enabled: false,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            token_env: None,
            token_path: None,
            oauth_client_path: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            acl: Vec::new(),
            editable: true,
        }
    }

    fn valid_update(root: &Path) -> SettingsUpdate {
        SettingsUpdate {
            workspaces: vec![WorkspaceSettings {
                id: "work".into(),
                name: "Work".into(),
                account_label: Some("team@example.com".into()),
                color: Some("#E8A83B".into()),
            }],
            sources: Vec::new(),
            auth_principals: vec![AuthPrincipalSettings {
                principal: "work-agent".into(),
                token_env: "CORTANA_WORK_AGENT_TOKEN".into(),
                scopes: vec!["query".into(), "status".into()],
                acl: vec!["work".into()],
            }],
            embedding: EmbeddingSettings {
                provider: "local".into(),
                base_url: "http://127.0.0.1:6999/v1".into(),
                model: "Qwen/Qwen3-Embedding-0.6B".into(),
                api_key_env: None,
                dimension: 1024,
                cache_max_entries: 250_000,
                request_timeout_seconds: 180,
                request_concurrency: 4,
                startup_timeout_seconds: 120,
                memory_limit_mb: 4096,
            },
            query: QuerySettings {
                synthesis_enabled: false,
                provider: "local".into(),
                base_url: "http://127.0.0.1:8080/v1".into(),
                model: "local".into(),
                api_key_env: None,
                max_planned_queries: 4,
                retrieval_limit: 20,
                result_limit: 8,
                context_tokens: 8000,
                output_tokens: 1200,
                request_timeout_seconds: 30,
                answer_timeout_seconds: 65,
                request_concurrency: 4,
                cache_max_entries: 10_000,
                cache_ttl_seconds: 3600,
            },
            ingestion: IngestionSettings {
                max_documents_per_source: 2000,
                max_bytes_per_source: 128 * 1024 * 1024,
                max_duration_seconds: 1800,
                document_batch_size: 64,
                request_concurrency: 2,
            },
            runtime: RuntimeSettings {
                data_dir: root.join("data").display().to_string(),
                connector_timeout_seconds: 3600,
                audit_max_events: 10_000,
            },
            secrets: vec![
                SecretUpdate {
                    name: "CORTANA_QUERY_API_KEY".into(),
                    value: Some("not-returned".into()),
                    clear: false,
                },
                SecretUpdate {
                    name: "CORTANA_WORK_AGENT_TOKEN".into(),
                    value: Some("private-bearer".into()),
                    clear: false,
                },
            ],
        }
    }

    #[test]
    fn saves_owner_only_settings_and_never_returns_secret_values() {
        let temp = tempfile::tempdir().expect("temp directory");
        let store = SettingsStore {
            config_path: temp.path().join("config/config.toml"),
        };
        let state = store
            .save(valid_update(temp.path()))
            .expect("save settings");
        assert_eq!(state.workspaces[0].id, "work");
        assert_eq!(state.auth_principals[0].principal, "work-agent");
        assert!(!format!("{state:?}").contains("not-returned"));
        assert!(!format!("{state:?}").contains("private-bearer"));
        let secret_body =
            fs::read_to_string(temp.path().join("config/secrets.env")).expect("secret file");
        assert!(secret_body.contains("CORTANA_QUERY_API_KEY=not-returned"));
        assert!(secret_body.contains("CORTANA_WORK_AGENT_TOKEN=private-bearer"));
        assert_eq!(
            bearer_for_scope_at(&store.config_path, "query").expect("query bearer"),
            Some("private-bearer".into())
        );
        assert!(bearer_for_scope_at(&store.config_path, "admin").is_err());
        let audit = desktop_audit_events_at(&store.config_path, 10).expect("desktop audit");
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0]["event"], "settings.updated");
        assert_eq!(audit[0]["secret_values_recorded"], false);

        let mut removal = valid_update(temp.path());
        removal.auth_principals.clear();
        removal.secrets.clear();
        store.save(removal).expect("remove auth principal");
        let secret_body =
            fs::read_to_string(temp.path().join("config/secrets.env")).expect("secret file");
        assert!(!secret_body.contains("CORTANA_WORK_AGENT_TOKEN"));
        assert!(secret_body.contains("CORTANA_QUERY_API_KEY=not-returned"));
        assert!(temp.path().join("config/desktop-audit.jsonl").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&store.config_path)
                    .expect("config metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn rejects_unbounded_workspaces_insecure_urls_and_secret_newlines() {
        let temp = tempfile::tempdir().expect("temp directory");
        let mut update = valid_update(temp.path());
        update.workspaces.push(update.workspaces[0].clone());
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.embedding.base_url = "http://example.com/v1".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.secrets[0].value = Some("secret\nINJECTED=yes".into());
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.auth_principals[0].scopes = vec!["root".into()];
        assert!(validate_update(&mut update).is_err());
    }

    #[test]
    fn refuses_to_mutate_externally_managed_secrets_or_malformed_sections() {
        let temp = tempfile::tempdir().expect("temp directory");
        let config_path = temp.path().join("config/config.toml");
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::write(
            &config_path,
            format!(
                "[runtime]\nenv_file = \"{}\"\n",
                temp.path().join("external.env").display()
            ),
        )
        .expect("external secret config");
        let store = SettingsStore {
            config_path: config_path.clone(),
        };
        let error = store
            .save(valid_update(temp.path()))
            .expect_err("external secrets must not be mutated");
        assert!(error.contains("externally managed"));
        assert!(!temp.path().join("external.env").exists());

        fs::write(&config_path, "embedding = \"invalid\"\n").expect("malformed section");
        let error = store
            .save(valid_update(temp.path()))
            .expect_err("malformed sections must not panic");
        assert!(error.contains("must be a TOML table"));
    }

    #[test]
    fn defaults_match_the_core_runtime_and_preserve_source_scopes() {
        let temp = tempfile::tempdir().expect("temp directory");
        let store = SettingsStore {
            config_path: temp.path().join("config/config.toml"),
        };
        let state = store.load().expect("default settings");
        assert_eq!(state.query.base_url, "http://127.0.0.1:8008/v1");
        assert_eq!(state.query.model, "auto-efficient");
        assert_eq!(state.query.retrieval_limit, 10);
        assert_eq!(state.query.result_limit, 20);
        assert_eq!(state.ingestion.max_duration_seconds, 900);
        assert_eq!(state.ingestion.document_batch_size, 16);
        assert_eq!(state.ingestion.request_concurrency, 1);
        assert_eq!(state.runtime.connector_timeout_seconds, 21_600);

        fs::create_dir_all(store.config_path.parent().expect("config parent"))
            .expect("config directory");
        fs::write(
            &store.config_path,
            "[[sources]]\nname = \"mail\"\nkind = \"gmail\"\nenabled = false\nproject = \"personal\"\n",
        )
        .expect("source config");
        let mut update = valid_update(temp.path());
        update.sources = store.load().expect("configured source").sources;
        let error = store
            .save(update.clone())
            .expect_err("source scope cannot be orphaned");
        assert!(error.contains("personal"));
        update.workspaces[0].id = "personal".into();
        store.save(update).expect("matching source scope");
    }

    #[test]
    fn source_settings_preserve_external_commands_and_validate_credentials() {
        let temp = tempfile::tempdir().expect("temp directory");
        let store = SettingsStore {
            config_path: temp.path().join("config/config.toml"),
        };
        fs::create_dir_all(store.config_path.parent().expect("config parent"))
            .expect("config directory");
        fs::write(
            &store.config_path,
            "[[sources]]\nname = \"custom\"\nkind = \"external\"\nenabled = false\nproject = \"work\"\ncommand = [\"/fixed/connector\", \"--jsonl\"]\n",
        )
        .expect("external source");
        let mut update = valid_update(temp.path());
        update.sources = store.load().expect("load source").sources;
        assert!(!update.sources[0].editable);
        store.save(update).expect("retain external source");
        let rendered = fs::read_to_string(&store.config_path).expect("saved config");
        assert!(rendered.contains("/fixed/connector"));

        let mut update = valid_update(temp.path());
        update.sources.push(SourceSettings {
            name: "team-slack".into(),
            kind: "slack".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: vec!["C012345".into()],
            token_env: None,
            token_path: None,
            oauth_client_path: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            acl: vec!["work".into()],
            editable: true,
        });
        let error = validate_update(&mut update).expect_err("enabled Slack needs credentials");
        assert!(error.contains("token environment"));
    }

    #[test]
    fn source_credentials_must_stay_outside_indexed_roots() {
        let temp = tempfile::tempdir().expect("temp directory");
        let root = temp.path().join("documents");
        let mut filesystem = source_settings("documents", "filesystem");
        filesystem.root = Some(root.display().to_string());
        let mut google = source_settings("drive", "google-drive");
        google.token_path = Some(root.join("token.json").display().to_string());
        google.oauth_client_path = Some(
            temp.path()
                .join("private/client.json")
                .display()
                .to_string(),
        );
        let mut update = valid_update(temp.path());
        update.sources = vec![filesystem, google];

        let error = validate_update(&mut update).expect_err("credential path must not be indexed");
        assert!(error.contains("outside every filesystem source root"));
    }
}
