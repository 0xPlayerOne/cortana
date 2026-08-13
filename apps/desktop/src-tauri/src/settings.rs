use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use toml::{Table, Value};

const MAX_WORKSPACES: usize = 3;
const MAX_SOURCES: usize = 128;
const MAX_AUTH_PRINCIPALS: usize = 64;
const MAX_SECRET_BYTES: usize = 16 * 1024;
const MAX_AUDIT_READ_BYTES: u64 = 2 * 1024 * 1024;
const DEFAULT_AUDIT_MAX_EVENTS: usize = 10_000;
const MAX_DESKTOP_AUDIT_EVENTS: usize = 1_000_000;
const MAX_PORTABLE_SETTINGS_BYTES: u64 = 2 * 1024 * 1024;
const PORTABLE_SETTINGS_VERSION: u32 = 1;
const SOURCE_KINDS: &[&str] = &[
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

static DESKTOP_AUDIT_COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();

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
pub struct HindsightSettings {
    pub enabled: bool,
    pub provider: String,
    pub base_url: String,
    pub bank: String,
    pub token_env: Option<String>,
    pub optional: bool,
    pub wired_to_ingestion: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HonchoSettings {
    pub enabled: bool,
    pub provider: String,
    pub base_url: String,
    pub workspace_id: String,
    pub peer_id: String,
    pub session_prefix: String,
    pub token_env: Option<String>,
    pub optional: bool,
    pub wired_to_ingestion: bool,
}

impl Default for HonchoSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "honcho".into(),
            base_url: "https://api.honcho.dev".into(),
            workspace_id: "default".into(),
            peer_id: "cortana".into(),
            session_prefix: "cortana".into(),
            token_env: None,
            optional: true,
            wired_to_ingestion: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IngestionSettings {
    pub max_documents_per_source: usize,
    pub max_bytes_per_source: u64,
    pub max_duration_seconds: u64,
    pub document_batch_size: usize,
    pub request_concurrency: usize,
    pub sync_freshness_hours: u64,
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
    #[serde(default)]
    pub repositories: Vec<String>,
    /// Discord servers (guilds) assigned to this source's workspace through
    /// browser authorization. Channel selection stays in `channels`.
    #[serde(default)]
    pub servers: Vec<String>,
    /// Slack team (workspace) ids assigned to this source's workspace through
    /// browser authorization, at most one per source because a Slack user
    /// token is scoped to exactly one workspace.
    #[serde(default)]
    pub teams: Vec<String>,
    /// Slack team display names kept index-aligned with `teams` so assigned
    /// workspaces stay identifiable without re-discovery.
    #[serde(default)]
    pub team_names: Vec<String>,
    /// Buzz community ids assigned to this source's workspace from the
    /// read-only `agents/teams.json` identity file. This is the generic Buzz
    /// community representation and is separate from Slack's `teams` fields.
    #[serde(default)]
    pub communities: Vec<String>,
    /// Buzz community display names kept index-aligned with `communities` so
    /// assigned communities stay identifiable without re-reading the
    /// identity file.
    #[serde(default)]
    pub community_names: Vec<String>,
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
    pub hindsight: HindsightSettings,
    pub honcho: HonchoSettings,
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
    pub secret_file_managed: bool,
    pub embedding_service_program: Option<String>,
    pub needs_setup: bool,
    pub restart_required: bool,
    pub workspaces: Vec<WorkspaceSettings>,
    pub sources: Vec<SourceSettings>,
    pub auth_principals: Vec<AuthPrincipalSettings>,
    pub embedding: EmbeddingSettings,
    pub query: QuerySettings,
    pub hindsight: HindsightSettings,
    pub honcho: HonchoSettings,
    pub ingestion: IngestionSettings,
    pub runtime: RuntimeSettings,
    pub secrets: Vec<SecretState>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortableSettings {
    pub workspaces: Vec<WorkspaceSettings>,
    pub sources: Vec<SourceSettings>,
    pub auth_principals: Vec<AuthPrincipalSettings>,
    pub embedding: EmbeddingSettings,
    pub query: QuerySettings,
    pub hindsight: HindsightSettings,
    #[serde(default)]
    pub honcho: HonchoSettings,
    pub ingestion: IngestionSettings,
    pub runtime: RuntimeSettings,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PortableSettingsFile {
    format_version: u32,
    secrets_included: bool,
    settings: PortableSettings,
}

#[derive(Debug, Clone, Serialize)]
pub struct PortableExport {
    pub path: String,
    pub format_version: u32,
    pub secrets_included: bool,
    pub omitted_external_sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PortableImport {
    pub path: String,
    pub format_version: u32,
    pub secrets_included: bool,
    pub preserved_external_sources: Vec<String>,
    pub settings: PortableSettings,
}

pub fn load() -> Result<SettingsSnapshot, String> {
    SettingsStore::default().load()
}

pub fn save(update: SettingsUpdate) -> Result<SettingsSnapshot, String> {
    SettingsStore::default().save(update)
}

pub(crate) fn configure_connector_command(path: &Path) -> Result<(), String> {
    configure_connector_command_at(&default_config_path(), path)
}

fn configure_connector_command_at(config_path: &Path, path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path.parent().is_none()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err("connector command must be an absolute, normalized path".into());
    }
    let expected_name = if cfg!(windows) {
        "cortana-connectors.exe"
    } else {
        "cortana-connectors"
    };
    if path.file_name().and_then(|name| name.to_str()) != Some(expected_name) {
        return Err("connector command must use the bundled cortana-connectors executable".into());
    }
    reject_symlink(config_path)?;
    let mut root = read_config(config_path)?;
    let connectors = root
        .entry("connectors")
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| "connectors settings must be a TOML table".to_string())?;
    if let Some(existing) = connectors.get("command") {
        match existing {
            Value::Array(values) if !values.is_empty() => {
                if values.iter().any(|value| !value.is_str()) {
                    return Err("existing connector command must contain only strings".into());
                }
                return append_audit_event(
                    config_path,
                    &serde_json::json!({
                        "at_unix_seconds": SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map_err(|error| error.to_string())?
                            .as_secs(),
                        "event": "connectors.command_preserved",
                        "command_configured": false,
                        "secret_values_recorded": false,
                    }),
                );
            }
            Value::Array(_) => {}
            _ => return Err("existing connector command must be a TOML array".into()),
        }
    }
    connectors.insert(
        "command".into(),
        Value::Array(vec![Value::String(path.display().to_string())]),
    );
    let rendered = toml::to_string_pretty(&root)
        .map_err(|error| format!("serialize connector settings: {error}"))?;
    if config_path.exists() {
        let backup = config_path.with_extension("toml.backup");
        reject_symlink(&backup)?;
        fs::copy(config_path, &backup)
            .map_err(|error| format!("back up Cortana settings: {error}"))?;
        set_owner_only(&backup)?;
    }
    atomic_write(config_path, rendered.as_bytes())?;
    append_audit_event(
        config_path,
        &serde_json::json!({
            "at_unix_seconds": SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_secs(),
            "event": "connectors.command_configured",
            "command_configured": true,
            "secret_values_recorded": false,
        }),
    )
}

pub fn export_portable(path: &Path) -> Result<PortableExport, String> {
    export_portable_at(&default_config_path(), path)
}

pub fn import_portable(path: &Path) -> Result<PortableImport, String> {
    import_portable_at(&default_config_path(), path)
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

pub(crate) fn secret_value_for_env(name: &str) -> Result<Option<String>, String> {
    validate_env_name(name)?;
    let config_path = default_config_path();
    let root = read_config(&config_path)?;
    let path = secret_path(&root, &config_path)?;
    let secrets = read_secret_map(&path)?;
    Ok(secrets
        .get(name)
        .cloned()
        .or_else(|| std::env::var(name).ok())
        .filter(|value| !value.is_empty()))
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
        let needs_setup = !existed || configured_sources(&root).is_empty();
        Ok(snapshot(
            &root,
            &self.config_path,
            &secret_path,
            &secrets,
            needs_setup,
        ))
    }

    fn save(&self, mut update: SettingsUpdate) -> Result<SettingsSnapshot, String> {
        reject_symlink(&self.config_path)?;

        let mut root = read_config(&self.config_path)?;
        let visible_workspace_ids = configured_workspaces(&root)
            .into_iter()
            .map(|workspace| workspace.id)
            .collect::<BTreeSet<_>>();
        let legacy_source_scopes = configured_source_projects(&root)
            .difference(&visible_workspace_ids)
            .cloned()
            .collect::<BTreeSet<_>>();
        validate_update_with_legacy_scopes(&mut update, &legacy_source_scopes)?;
        let secret_path = secret_path(&root, &self.config_path)?;
        let previous_secret_names = referenced_secret_names(&root);
        let next_secret_names = update_secret_names(&update);
        let removed_secret_names = previous_secret_names
            .difference(&next_secret_names)
            .cloned()
            .collect::<Vec<_>>();
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
            for name in &removed_secret_names {
                secrets.remove(name);
            }
            atomic_write(&secret_path, render_secrets(&secrets).as_bytes())?;
        } else if !removed_secret_names.is_empty()
            && self
                .config_path
                .parent()
                .is_some_and(|parent| secret_path == parent.join("secrets.env"))
        {
            // Desktop owns the default secret file, so stale values from a
            // removed source/provider reference can be safely retired. An
            // externally managed runtime.env_file is never modified here.
            let mut secrets = read_secret_map(&secret_path)?;
            for name in &removed_secret_names {
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

fn export_portable_at(config_path: &Path, path: &Path) -> Result<PortableExport, String> {
    validate_portable_path(path, false)?;
    let store = SettingsStore {
        config_path: config_path.to_path_buf(),
    };
    let snapshot = store.load()?;
    let omitted_external_sources = snapshot
        .sources
        .iter()
        .filter(|source| source.kind == "external")
        .map(|source| source.name.clone())
        .collect::<Vec<_>>();
    let settings = PortableSettings {
        workspaces: snapshot.workspaces,
        sources: snapshot
            .sources
            .into_iter()
            .filter(|source| source.kind != "external")
            .collect(),
        auth_principals: snapshot.auth_principals,
        embedding: snapshot.embedding,
        query: snapshot.query,
        hindsight: snapshot.hindsight,
        honcho: snapshot.honcho,
        ingestion: snapshot.ingestion,
        runtime: snapshot.runtime,
    };
    let file = PortableSettingsFile {
        format_version: PORTABLE_SETTINGS_VERSION,
        secrets_included: false,
        settings,
    };
    let rendered = serde_json::to_vec_pretty(&file)
        .map_err(|error| format!("serialize portable settings: {error}"))?;
    if rendered.len() as u64 > MAX_PORTABLE_SETTINGS_BYTES {
        return Err("portable settings exceed the 2 MiB export limit".into());
    }
    write_portable_file(path, &rendered)?;
    append_portable_audit(
        config_path,
        "settings.exported",
        omitted_external_sources.len(),
    )?;
    Ok(PortableExport {
        path: path.display().to_string(),
        format_version: PORTABLE_SETTINGS_VERSION,
        secrets_included: false,
        omitted_external_sources,
    })
}

fn import_portable_at(config_path: &Path, path: &Path) -> Result<PortableImport, String> {
    validate_portable_path(path, true)?;
    let metadata =
        fs::metadata(path).map_err(|error| format!("inspect portable settings: {error}"))?;
    if !metadata.is_file() {
        return Err("portable settings must be a regular file".into());
    }
    if metadata.len() > MAX_PORTABLE_SETTINGS_BYTES {
        return Err("portable settings exceed the 2 MiB import limit".into());
    }
    let bytes = fs::read(path).map_err(|error| format!("read portable settings: {error}"))?;
    let file: PortableSettingsFile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse portable settings: {error}"))?;
    if file.format_version != PORTABLE_SETTINGS_VERSION {
        return Err(format!(
            "portable settings format {} is unsupported; expected {}",
            file.format_version, PORTABLE_SETTINGS_VERSION
        ));
    }
    if file.secrets_included {
        return Err("portable settings must never contain secret values".into());
    }
    if file
        .settings
        .sources
        .iter()
        .any(|source| source.kind == "external")
    {
        return Err(
            "portable settings cannot add executable external connectors; retain them locally"
                .into(),
        );
    }

    let store = SettingsStore {
        config_path: config_path.to_path_buf(),
    };
    let current = store.load()?;
    let preserved_external_sources = current
        .sources
        .iter()
        .filter(|source| source.kind == "external")
        .map(|source| source.name.clone())
        .collect::<Vec<_>>();
    let mut update = file.settings.into_update();
    update.sources.extend(
        current
            .sources
            .into_iter()
            .filter(|source| source.kind == "external"),
    );
    let portable_source_scopes = update
        .sources
        .iter()
        .map(|source| source.project.trim().to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    validate_update_with_legacy_scopes(&mut update, &portable_source_scopes)?;
    let root = read_config(config_path)?;
    validate_external_sources(&root, &update.sources)?;
    let settings = PortableSettings::from_update(update);
    append_portable_audit(
        config_path,
        "settings.import.previewed",
        preserved_external_sources.len(),
    )?;
    Ok(PortableImport {
        path: path.display().to_string(),
        format_version: PORTABLE_SETTINGS_VERSION,
        secrets_included: false,
        preserved_external_sources,
        settings,
    })
}

fn workspace_value(workspace: &WorkspaceSettings) -> Value {
    let mut table = Table::new();
    table.insert("id".into(), Value::String(workspace.id.clone()));
    table.insert("name".into(), Value::String(workspace.name.clone()));
    insert_optional_string(&mut table, "account_label", &workspace.account_label);
    insert_optional_string(&mut table, "color", &workspace.color);
    Value::Table(table)
}

/// Keep source scopes that predate the three-workspace Desktop surface.
///
/// The backend permits arbitrary project scopes when no explicit workspace
/// table exists, and existing installations may therefore contain sources
/// such as `community` or `agents`. Desktop intentionally renders only the
/// first three workspaces, but a save must not orphan those source scopes and
/// make the backend reject its own configuration on the next restart.
fn workspace_values(root: &Table, update: &SettingsUpdate) -> Vec<Value> {
    let mut values = update
        .workspaces
        .iter()
        .map(workspace_value)
        .collect::<Vec<_>>();
    let mut known = update
        .workspaces
        .iter()
        .map(|workspace| workspace.id.clone())
        .collect::<BTreeSet<_>>();
    let existing = root
        .get("workspaces")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_table)
        .filter_map(|table| {
            Some(WorkspaceSettings {
                id: table.get("id")?.as_str()?.to_ascii_lowercase(),
                name: table.get("name")?.as_str()?.to_string(),
                account_label: table
                    .get("account_label")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                color: table
                    .get("color")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        })
        .map(|workspace| (workspace.id.clone(), workspace))
        .collect::<BTreeMap<_, _>>();

    let source_scopes = update
        .sources
        .iter()
        .map(|source| source.project.trim().to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    for id in source_scopes {
        if !known.insert(id.clone()) {
            continue;
        }
        let workspace = existing
            .get(&id)
            .cloned()
            .unwrap_or_else(|| WorkspaceSettings {
                id: id.clone(),
                name: title_case(&id),
                account_label: None,
                color: None,
            });
        values.push(workspace_value(&workspace));
    }
    values
}

impl PortableSettings {
    fn into_update(self) -> SettingsUpdate {
        SettingsUpdate {
            workspaces: self.workspaces,
            sources: self.sources,
            auth_principals: self.auth_principals,
            embedding: self.embedding,
            query: self.query,
            hindsight: self.hindsight,
            honcho: self.honcho,
            ingestion: self.ingestion,
            runtime: self.runtime,
            secrets: Vec::new(),
        }
    }

    fn from_update(update: SettingsUpdate) -> Self {
        Self {
            workspaces: update.workspaces,
            sources: update.sources,
            auth_principals: update.auth_principals,
            embedding: update.embedding,
            query: update.query,
            hindsight: update.hindsight,
            honcho: update.honcho,
            ingestion: update.ingestion,
            runtime: update.runtime,
        }
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
    let hindsight_token_env = optional_string(root, "hindsight", "token_env");
    let honcho_token_env = optional_string(root, "honcho", "token_env");
    let embedding_service_program = nested_table(root, "embedding", "service")
        .map(|service| table_string_array(service, "command"))
        .and_then(|command| command.into_iter().next());
    let secret_file_managed = config_path
        .parent()
        .map(|parent| parent.join("secrets.env") == secret_path)
        .unwrap_or(false);
    let mut secret_names = BTreeSet::new();
    secret_names.extend(embedding_api_key_env.iter().cloned());
    secret_names.extend(query_api_key_env.iter().cloned());
    secret_names.extend(hindsight_token_env.iter().cloned());
    secret_names.extend(honcho_token_env.iter().cloned());
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
        secret_file_managed,
        embedding_service_program,
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
                300,
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
        hindsight: HindsightSettings {
            enabled: bool_value(root, "hindsight", "enabled", false),
            provider: string(root, "hindsight", "provider", "hindsight"),
            base_url: string(root, "hindsight", "base_url", "http://127.0.0.1:8888"),
            bank: string(root, "hindsight", "bank", "default"),
            token_env: optional_string(root, "hindsight", "token_env"),
            optional: bool_value(root, "hindsight", "optional", true),
            wired_to_ingestion: bool_value(root, "hindsight", "wired_to_ingestion", false),
        },
        honcho: HonchoSettings {
            enabled: bool_value(root, "honcho", "enabled", false),
            provider: string(root, "honcho", "provider", "honcho"),
            base_url: string(root, "honcho", "base_url", "https://api.honcho.dev"),
            workspace_id: string(root, "honcho", "workspace_id", "default"),
            peer_id: string(root, "honcho", "peer_id", "cortana"),
            session_prefix: string(root, "honcho", "session_prefix", "cortana"),
            token_env: honcho_token_env,
            optional: bool_value(root, "honcho", "optional", true),
            wired_to_ingestion: bool_value(root, "honcho", "wired_to_ingestion", false),
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
            sync_freshness_hours: u64_value(root, "ingestion", "sync_freshness_hours", 48),
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
                        repositories: table_string_array(item, "repositories"),
                        servers: table_string_array(item, "servers"),
                        teams: table_string_array(item, "teams"),
                        team_names: table_string_array(item, "team_names"),
                        communities: table_string_array(item, "communities"),
                        community_names: table_string_array(item, "community_names"),
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

fn configured_source_projects(root: &Table) -> BTreeSet<String> {
    configured_sources(root)
        .into_iter()
        .map(|source| source.project.trim().to_ascii_lowercase())
        .collect()
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
                projects.insert(project.to_ascii_lowercase());
            }
        }
    }
    let fallback = prioritized_workspace_projects(projects);
    if !fallback.is_empty() {
        return fallback;
    }
    ["work", "personal", "special"]
        .into_iter()
        .map(|id| WorkspaceSettings {
            id: id.to_string(),
            name: title_case(id),
            account_label: None,
            color: None,
        })
        .collect()
}

fn prioritized_workspace_projects(projects: BTreeSet<String>) -> Vec<WorkspaceSettings> {
    let mut result = Vec::new();
    let mut remaining = projects;
    for id in ["work", "personal", "special"] {
        if remaining.remove(id) {
            result.push(WorkspaceSettings {
                id: id.to_string(),
                name: title_case(id),
                account_label: None,
                color: None,
            });
        }
    }
    for id in remaining {
        if result.len() >= MAX_WORKSPACES {
            break;
        }
        let name = title_case(&id);
        result.push(WorkspaceSettings {
            id,
            name,
            account_label: None,
            color: None,
        });
    }
    result.truncate(MAX_WORKSPACES);
    result
}

fn referenced_secret_names(root: &Table) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for (section, key) in [
        ("embedding", "api_key_env"),
        ("query", "api_key_env"),
        ("hindsight", "token_env"),
        ("honcho", "token_env"),
    ] {
        if let Some(name) = optional_string(root, section, key) {
            names.insert(name);
        }
    }
    if let Some(sources) = root.get("sources").and_then(Value::as_array) {
        for name in sources
            .iter()
            .filter_map(Value::as_table)
            .filter_map(|source| table_optional_string(source, "token_env"))
        {
            names.insert(name);
        }
    }
    if let Some(principals) = table(root, "auth")
        .and_then(|auth| auth.get("tokens"))
        .and_then(Value::as_array)
    {
        for name in principals
            .iter()
            .filter_map(Value::as_table)
            .filter_map(|principal| table_optional_string(principal, "token_env"))
        {
            names.insert(name);
        }
    }
    names
}

fn update_secret_names(update: &SettingsUpdate) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for name in [
        update.embedding.api_key_env.as_ref(),
        update.query.api_key_env.as_ref(),
        update.hindsight.token_env.as_ref(),
        update.honcho.token_env.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        names.insert(name.clone());
    }
    names.extend(
        update
            .sources
            .iter()
            .filter_map(|source| source.token_env.as_ref().cloned()),
    );
    names.extend(
        update
            .auth_principals
            .iter()
            .map(|principal| principal.token_env.clone()),
    );
    names
}

#[cfg(test)]
fn validate_update(update: &mut SettingsUpdate) -> Result<(), String> {
    validate_update_with_legacy_scopes(update, &BTreeSet::new())
}

fn validate_update_with_legacy_scopes(
    update: &mut SettingsUpdate,
    legacy_source_scopes: &BTreeSet<String>,
) -> Result<(), String> {
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
    validate_sources(&mut update.sources, &ids, legacy_source_scopes)?;
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
        0,
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
        0,
        1_000_000,
    )?;
    bounded_u64(
        "query cache lifetime",
        update.query.cache_ttl_seconds,
        0,
        604_800,
    )?;

    if update.hindsight.provider != "hindsight" {
        return Err("hindsight provider must be `hindsight`".into());
    }
    if !update.hindsight.optional {
        return Err("hindsight is fixed as optional and cannot be changed".into());
    }
    if update.hindsight.wired_to_ingestion {
        return Err("hindsight cannot be wired into normal ingestion by default".into());
    }
    update.hindsight.base_url = update.hindsight.base_url.trim().to_string();
    update.hindsight.bank = update.hindsight.bank.trim().to_string();
    if update.hindsight.enabled {
        if update.hindsight.base_url.is_empty() {
            return Err("hindsight enabled requires a base URL".into());
        }
        if update.hindsight.token_env.is_none() {
            return Err("hindsight enabled requires a token environment variable".into());
        }
        validate_hindsight_bank(&update.hindsight.bank)?;
        validate_hindsight_url("hindsight", &update.hindsight.base_url)?;
    } else {
        if update.hindsight.base_url.is_empty() {
            update.hindsight.base_url = "http://127.0.0.1:8888".into();
        }
        if update.hindsight.bank.is_empty() {
            update.hindsight.bank = "default".into();
        }
        validate_hindsight_url("hindsight", &update.hindsight.base_url)?;
        validate_hindsight_bank(&update.hindsight.bank)?;
    }
    validate_optional_env(&update.hindsight.token_env)?;

    if update.honcho.provider != "honcho" {
        return Err("honcho provider must be `honcho`".into());
    }
    if !update.honcho.optional {
        return Err("honcho is fixed as optional and cannot be changed".into());
    }
    if update.honcho.wired_to_ingestion {
        return Err("honcho cannot be wired into normal ingestion by default".into());
    }
    update.honcho.base_url = update.honcho.base_url.trim().to_string();
    update.honcho.workspace_id = update.honcho.workspace_id.trim().to_string();
    update.honcho.peer_id = update.honcho.peer_id.trim().to_string();
    update.honcho.session_prefix = update.honcho.session_prefix.trim().to_string();
    if update.honcho.enabled && update.honcho.token_env.is_none() {
        return Err("honcho enabled requires a token environment variable".into());
    }
    if update.honcho.base_url.is_empty() {
        update.honcho.base_url = "https://api.honcho.dev".into();
    }
    validate_hindsight_url("honcho", &update.honcho.base_url)?;
    validate_honcho_identifier("honcho workspace", &update.honcho.workspace_id)?;
    validate_honcho_identifier("honcho peer", &update.honcho.peer_id)?;
    validate_honcho_identifier("honcho session prefix", &update.honcho.session_prefix)?;
    validate_optional_env(&update.honcho.token_env)?;

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
        "sync freshness",
        update.ingestion.sync_freshness_hours,
        0,
        8_760,
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
    if !data_path.is_absolute()
        || data_path.parent().is_none()
        || data_path
            .parent()
            .is_none_or(|parent| parent.parent().is_none())
    {
        return Err("data directory must be an absolute non-root path".into());
    }
    update.runtime.data_dir = data_path.display().to_string();
    let referenced_secrets = update_secret_names(update);
    for secret in &update.secrets {
        validate_env_name(&secret.name)?;
        if !referenced_secrets.contains(&secret.name) {
            return Err(format!(
                "secret `{}` is not referenced by the saved settings",
                secret.name
            ));
        }
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
        Value::Array(workspace_values(root, update)),
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

    set_string(root, "hindsight", "provider", &update.hindsight.provider);
    set_bool(root, "hindsight", "enabled", update.hindsight.enabled);
    set_string(root, "hindsight", "base_url", &update.hindsight.base_url);
    set_string(root, "hindsight", "bank", &update.hindsight.bank);
    set_optional_string(root, "hindsight", "token_env", &update.hindsight.token_env);
    set_bool(root, "hindsight", "optional", update.hindsight.optional);
    set_bool(
        root,
        "hindsight",
        "wired_to_ingestion",
        update.hindsight.wired_to_ingestion,
    );

    set_string(root, "honcho", "provider", &update.honcho.provider);
    set_bool(root, "honcho", "enabled", update.honcho.enabled);
    set_string(root, "honcho", "base_url", &update.honcho.base_url);
    set_string(root, "honcho", "workspace_id", &update.honcho.workspace_id);
    set_string(root, "honcho", "peer_id", &update.honcho.peer_id);
    set_string(
        root,
        "honcho",
        "session_prefix",
        &update.honcho.session_prefix,
    );
    set_optional_string(root, "honcho", "token_env", &update.honcho.token_env);
    set_bool(root, "honcho", "optional", update.honcho.optional);
    set_bool(
        root,
        "honcho",
        "wired_to_ingestion",
        update.honcho.wired_to_ingestion,
    );

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
        (
            "sync_freshness_hours",
            update.ingestion.sync_freshness_hours as i64,
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
                set_table_string_array(&mut table, "repositories", &source.repositories);
                set_table_string_array(&mut table, "servers", &source.servers);
                set_table_string_array(&mut table, "teams", &source.teams);
                set_table_string_array(&mut table, "team_names", &source.team_names);
                set_table_string_array(&mut table, "communities", &source.communities);
                set_table_string_array(&mut table, "community_names", &source.community_names);
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
        "hindsight",
        "ingestion",
        "connectors",
        "auth",
        "runtime",
        "honcho",
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
    legacy_source_scopes: &BTreeSet<String>,
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
        validate_workspace_id(&source.project)?;
        if !names.insert(source.name.clone()) {
            return Err(format!("source name `{}` is duplicated", source.name));
        }
        if !SOURCE_KINDS.contains(&source.kind.as_str()) {
            return Err(format!("source `{}` has an unsupported kind", source.name));
        }
        if !workspace_ids.contains(&source.project)
            && !legacy_source_scopes.contains(&source.project)
        {
            return Err(format!(
                "source `{}` uses unknown workspace `{}`",
                source.name, source.project
            ));
        }
        if source.enabled
            && !workspace_ids.contains(&source.project)
            && legacy_source_scopes.contains(&source.project)
        {
            return Err(format!(
                "source `{}` is enabled but not assigned to a visible workspace `{}`; assign it before enabling or syncing",
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
        normalize_string_list("source repository", &mut source.repositories, 32, 256)?;
        normalize_string_list("source server", &mut source.servers, 100, 128)?;
        normalize_string_list("source team", &mut source.teams, 1, 12)?;
        normalize_string_list("source team name", &mut source.team_names, 1, 80)?;
        if source.teams.len() != source.team_names.len() {
            return Err(format!(
                "source `{}` must keep Slack team ids and names aligned",
                source.name
            ));
        }
        normalize_string_list("source community", &mut source.communities, 100, 128)?;
        normalize_string_list(
            "source community name",
            &mut source.community_names,
            100,
            128,
        )?;
        if source.communities.len() != source.community_names.len() {
            return Err(format!(
                "source `{}` must keep Buzz community ids and names aligned",
                source.name
            ));
        }
        if (!source.communities.is_empty() || !source.community_names.is_empty())
            && source.kind != "buzz"
        {
            return Err(format!(
                "source `{}` may assign communities only for Buzz sources; Slack teams use the `teams` fields",
                source.name
            ));
        }
        if source.kind == "github" {
            for repository in &source.repositories {
                if !valid_github_repository(repository) {
                    return Err(format!(
                        "source `{}` has an invalid GitHub repository `{}`; use owner/name",
                        source.name, repository
                    ));
                }
            }
        }
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
        "slack" if source.channels.is_empty() || source.token_env.is_none() => {
            Err(format!(
                "Slack source `{}` needs channels and a token environment name",
                source.name
            ))
        }
        "discord"
            if source.channels.is_empty()
                || source.token_path.is_none()
                || source.oauth_client_path.is_none()
                || source.token_env.is_some() =>
        {
            Err(format!(
                "Discord source `{}` needs channels, a token destination, and an OAuth client path; token environment names are not supported",
                source.name
            ))
        }
        "github"
            if source.repositories.is_empty()
                || (source.token_env.is_none() && source.token_path.is_none()) =>
        {
            Err(format!(
                "GitHub source `{}` needs repositories and a token environment name or token file",
                source.name
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
    reject_symlink(path)?;
    if !path.exists() {
        return Ok(Table::new());
    }
    let body =
        fs::read_to_string(path).map_err(|error| format!("read Cortana settings: {error}"))?;
    toml::from_str(&body).map_err(|error| format!("parse Cortana settings: {error}"))
}

fn read_secret_map(path: &Path) -> Result<BTreeMap<String, String>, String> {
    reject_symlink(path)?;
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
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
    reject_symlink(path)?;
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

fn validate_portable_path(path: &Path, must_exist: bool) -> Result<(), String> {
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
        return Err("portable settings require an absolute non-root path".into());
    }
    if path.extension().and_then(|value| value.to_str()) != Some("json") {
        return Err("portable settings must use a .json file".into());
    }
    reject_symlink(path)?;
    if must_exist && !path.exists() {
        return Err("portable settings file does not exist".into());
    }
    Ok(())
}

fn write_portable_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "portable settings path has no parent".to_string())?;
    if !parent.is_dir() {
        return Err("portable settings parent directory does not exist".into());
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let temp = parent.join(format!(
        ".cortana-portable-{}-{nonce}.tmp",
        std::process::id()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let write_result = (|| {
        let mut file = options
            .open(&temp)
            .map_err(|error| format!("create portable settings: {error}"))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("write portable settings: {error}"))?;
        let backup = if path.exists() {
            let backup = path.with_extension("json.backup");
            reject_symlink(&backup)?;
            fs::copy(path, &backup)
                .map_err(|error| format!("back up portable settings: {error}"))?;
            set_owner_only(&backup)?;
            fs::remove_file(path).map_err(|error| format!("replace portable settings: {error}"))?;
            Some(backup)
        } else {
            None
        };
        if let Err(error) = fs::rename(&temp, path) {
            if let Some(backup) = backup {
                let _ = fs::copy(backup, path);
                let _ = set_owner_only(path);
            }
            return Err(format!("install portable settings: {error}"));
        }
        set_owner_only(path)
    })();
    if write_result.is_err() && temp.exists() {
        let _ = fs::remove_file(&temp);
    }
    write_result
}

fn append_portable_audit(
    config_path: &Path,
    event: &str,
    external_source_count: usize,
) -> Result<(), String> {
    let value = serde_json::json!({
        "at_unix_seconds": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs(),
        "event": event,
        "format_version": PORTABLE_SETTINGS_VERSION,
        "external_source_count": external_source_count,
        "secret_values_recorded": false,
    });
    append_audit_event(config_path, &value)
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
    let max_events = read_config(config_path)
        .ok()
        .map(|root| usize_value(&root, "auth", "audit_max_events", DEFAULT_AUDIT_MAX_EVENTS))
        .unwrap_or(DEFAULT_AUDIT_MAX_EVENTS)
        .min(MAX_DESKTOP_AUDIT_EVENTS);
    if max_events == 0 {
        return Ok(());
    }
    let counts = DESKTOP_AUDIT_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut counts = counts
        .lock()
        .map_err(|_| "desktop audit retention lock poisoned".to_string())?;
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
    drop(file);

    let count = if let Some(count) = counts.get_mut(&path) {
        count
    } else {
        let count = count_audit_lines(&path)?;
        counts.insert(path.clone(), count);
        counts.get_mut(&path).expect("inserted desktop audit count")
    };
    *count = count.saturating_add(1);
    if *count > max_events {
        trim_audit_log(&path, max_events)?;
        *count = max_events;
    }
    set_owner_only(&path)
}

fn count_audit_lines(path: &Path) -> Result<usize, String> {
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("open desktop audit log for retention: {error}")),
    };
    let mut count = 0;
    for line in BufReader::new(file).lines() {
        let line =
            line.map_err(|error| format!("read desktop audit log for retention: {error}"))?;
        if !line.trim().is_empty() {
            count += 1;
        }
    }
    Ok(count)
}

fn trim_audit_log(path: &Path, max_events: usize) -> Result<(), String> {
    if max_events == 0 {
        return Ok(());
    }
    reject_symlink(path)?;
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|error| format!("open desktop audit log for retention: {error}"))?;
    let mut retained = VecDeque::with_capacity(max_events.min(4096));
    for line in BufReader::new(file).lines() {
        let line =
            line.map_err(|error| format!("read desktop audit log for retention: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        if retained.len() == max_events {
            retained.pop_front();
        }
        retained.push_back(line);
    }

    let temporary = path.with_extension(format!("jsonl.trim.{}", std::process::id()));
    reject_symlink(&temporary)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut created_temporary = false;
    let result = (|| -> Result<(), String> {
        let mut output = options
            .open(&temporary)
            .map_err(|error| format!("create desktop audit retention file: {error}"))?;
        created_temporary = true;
        for line in retained {
            writeln!(output, "{line}")
                .map_err(|error| format!("write desktop audit retention file: {error}"))?;
        }
        output
            .sync_data()
            .map_err(|error| format!("sync desktop audit retention file: {error}"))?;
        set_owner_only(&temporary)?;
        #[cfg(windows)]
        if path.exists() {
            fs::remove_file(path).map_err(|error| format!("replace desktop audit log: {error}"))?;
        }
        fs::rename(&temporary, path)
            .map_err(|error| format!("replace desktop audit log: {error}"))?;
        set_owner_only(path)
    })();
    if result.is_err() && created_temporary && temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("refusing to use symlinked file {}", path.display()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect {}: {error}", path.display())),
    }
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
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{name} URL must not include credentials"));
    }
    if url.query().is_some() {
        return Err(format!("{name} URL must not include query parameters"));
    }
    if url.fragment().is_some() {
        return Err(format!("{name} URL must not include a fragment"));
    }
    Ok(())
}

fn validate_hindsight_url(name: &str, value: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(value).map_err(|_| format!("{name} URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{name} URL must use HTTP or HTTPS"));
    }
    let is_loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    if url.scheme() == "http" && !is_loopback {
        return Err(format!("{name} cloud URL must use HTTPS"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{name} URL must not include credentials"));
    }
    if url.query().is_some() {
        return Err(format!("{name} URL must not include query parameters"));
    }
    if url.fragment().is_some() {
        return Err(format!("{name} URL must not include a fragment"));
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

fn validate_hindsight_bank(value: &str) -> Result<(), String> {
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
            "hindsight bank must be 1-64 lowercase letters, numbers, dashes, or underscores".into(),
        );
    }
    Ok(())
}

fn validate_honcho_identifier(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(format!(
            "{name} must be 1-128 letters, numbers, dots, dashes, or underscores"
        ));
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
            repositories: Vec::new(),
            servers: Vec::new(),
            teams: Vec::new(),
            team_names: Vec::new(),
            communities: Vec::new(),
            community_names: Vec::new(),
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
                startup_timeout_seconds: 300,
                memory_limit_mb: 4096,
            },
            query: QuerySettings {
                synthesis_enabled: false,
                provider: "local".into(),
                base_url: "http://127.0.0.1:8080/v1".into(),
                model: "local".into(),
                api_key_env: Some("CORTANA_QUERY_API_KEY".into()),
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
            hindsight: HindsightSettings {
                enabled: false,
                provider: "hindsight".into(),
                base_url: "http://127.0.0.1:8888".into(),
                bank: "default".into(),
                token_env: Some("CORTANA_HINDSIGHT_TOKEN".into()),
                optional: true,
                wired_to_ingestion: false,
            },
            honcho: HonchoSettings {
                enabled: false,
                provider: "honcho".into(),
                base_url: "https://api.honcho.dev".into(),
                workspace_id: "default".into(),
                peer_id: "cortana".into(),
                session_prefix: "cortana".into(),
                token_env: Some("CORTANA_HONCHO_TOKEN".into()),
                optional: true,
                wired_to_ingestion: false,
            },
            ingestion: IngestionSettings {
                max_documents_per_source: 2000,
                max_bytes_per_source: 128 * 1024 * 1024,
                max_duration_seconds: 1800,
                document_batch_size: 64,
                request_concurrency: 2,
                sync_freshness_hours: 48,
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
                SecretUpdate {
                    name: "CORTANA_HINDSIGHT_TOKEN".into(),
                    value: Some("hindsight-secret".into()),
                    clear: false,
                },
                SecretUpdate {
                    name: "CORTANA_HONCHO_TOKEN".into(),
                    value: Some("honcho-secret".into()),
                    clear: false,
                },
            ],
        }
    }

    #[test]
    fn connector_command_configuration_is_atomic_audited_and_keeps_setup_visible() {
        let temp = tempfile::tempdir().expect("temp directory");
        let config_path = temp.path().join("config/config.toml");
        let first = if cfg!(windows) {
            PathBuf::from(
                r"C:\Users\example\.local\share\cortana\venv\Scripts\cortana-connectors.exe",
            )
        } else {
            PathBuf::from("/Users/example/.local/share/cortana/venv/bin/cortana-connectors")
        };
        let second = if cfg!(windows) {
            PathBuf::from(r"C:\opt\cortana\share\cortana\venv\Scripts\cortana-connectors.exe")
        } else {
            PathBuf::from("/opt/cortana/share/cortana/venv/bin/cortana-connectors")
        };

        assert!(configure_connector_command_at(&config_path, &PathBuf::from("relative")).is_err());
        assert!(
            configure_connector_command_at(&config_path, Path::new("/tmp/not-a-connector"))
                .is_err()
        );
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::write(
            &config_path,
            "[runtime]\ndata_dir = \"/tmp/cortana-data\"\n",
        )
        .expect("initial config");
        configure_connector_command_at(&config_path, &first).expect("first connector command");
        let state = SettingsStore {
            config_path: config_path.clone(),
        }
        .load()
        .expect("settings state");
        assert!(state.needs_setup);
        let first_body = fs::read_to_string(&config_path).expect("config body");
        assert!(first_body.contains(&first.display().to_string()));
        configure_connector_command_at(&config_path, &second).expect("second connector command");
        let backup = config_path.with_extension("toml.backup");
        assert!(
            fs::read_to_string(backup)
                .expect("config backup")
                .contains("data_dir")
        );
        assert!(
            fs::read_to_string(&config_path)
                .expect("preserved config")
                .contains(&first.display().to_string())
        );
        let audit = desktop_audit_events_at(&config_path, 10).expect("desktop audit");
        assert_eq!(audit.len(), 2);
        assert!(
            audit
                .iter()
                .all(|event| event["secret_values_recorded"] == false)
        );
    }

    #[test]
    fn desktop_audit_retention_keeps_newest_events_and_private_permissions() {
        let temp = tempfile::tempdir().expect("temp directory");
        let config_path = temp.path().join("config/config.toml");
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::write(&config_path, "[auth]\naudit_max_events = 3\n").expect("audit config");

        for id in 0..5 {
            append_audit_event(
                &config_path,
                &serde_json::json!({
                    "id": id,
                    "event": "test",
                    "secret_values_recorded": false,
                }),
            )
            .expect("append audit event");
        }

        let events = desktop_audit_events_at(&config_path, 10).expect("read audit events");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["id"], 4);
        assert_eq!(events[1]["id"], 3);
        assert_eq!(events[2]["id"], 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(
                    config_path
                        .parent()
                        .expect("config parent")
                        .join("desktop-audit.jsonl")
                )
                .expect("audit metadata")
                .permissions()
                .mode()
                    & 0o777,
                0o600
            );
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
        let secret_names = state
            .secrets
            .iter()
            .map(|secret| secret.name.as_str())
            .collect::<Vec<_>>();
        assert!(secret_names.contains(&"CORTANA_QUERY_API_KEY"));
        assert!(secret_names.contains(&"CORTANA_WORK_AGENT_TOKEN"));
        assert!(secret_names.contains(&"CORTANA_HINDSIGHT_TOKEN"));
        assert!(secret_names.contains(&"CORTANA_HONCHO_TOKEN"));
        assert!(!format!("{state:?}").contains("not-returned"));
        assert!(!format!("{state:?}").contains("private-bearer"));
        assert!(!format!("{state:?}").contains("hindsight-secret"));
        assert!(!format!("{state:?}").contains("honcho-secret"));
        let secret_body =
            fs::read_to_string(temp.path().join("config/secrets.env")).expect("secret file");
        assert!(secret_body.contains("CORTANA_QUERY_API_KEY=not-returned"));
        assert!(secret_body.contains("CORTANA_WORK_AGENT_TOKEN=private-bearer"));
        assert!(secret_body.contains("CORTANA_HINDSIGHT_TOKEN=hindsight-secret"));
        assert!(secret_body.contains("CORTANA_HONCHO_TOKEN=honcho-secret"));
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
    fn retires_secret_file_values_when_references_are_removed() {
        let temp = tempfile::tempdir().expect("temp directory");
        let store = SettingsStore {
            config_path: temp.path().join("config/config.toml"),
        };
        let mut initial = valid_update(temp.path());
        let mut slack = source_settings("team-slack", "slack");
        slack.token_env = Some("CORTANA_SLACK_TOKEN".into());
        initial.sources.push(slack);
        initial.secrets.push(SecretUpdate {
            name: "CORTANA_SLACK_TOKEN".into(),
            value: Some("slack-secret".into()),
            clear: false,
        });
        store.save(initial).expect("save source secret");
        let secret_path = temp.path().join("config/secrets.env");
        assert!(
            fs::read_to_string(&secret_path)
                .expect("secret file")
                .contains("CORTANA_SLACK_TOKEN=slack-secret")
        );

        store
            .save(valid_update(temp.path()))
            .expect("remove source");
        let secret_body = fs::read_to_string(&secret_path).expect("secret file after removal");
        assert!(!secret_body.contains("CORTANA_SLACK_TOKEN"));
        assert!(secret_body.contains("CORTANA_QUERY_API_KEY=not-returned"));
    }

    #[test]
    fn rejects_unbounded_workspaces_insecure_urls_and_secret_newlines() {
        let temp = tempfile::tempdir().expect("temp directory");

        let mut cache_disabled = valid_update(temp.path());
        cache_disabled.embedding.cache_max_entries = 0;
        cache_disabled.query.cache_max_entries = 0;
        cache_disabled.query.cache_ttl_seconds = 0;
        validate_update(&mut cache_disabled).expect("zero cache values are valid opt-outs");

        let mut update = valid_update(temp.path());
        update.workspaces.push(update.workspaces[0].clone());
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.embedding.base_url = "http://example.com/v1".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.embedding.base_url = "https://user:password@example.com/v1".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.query.base_url = "https://api.example.com/v1?api_key=secret".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.query.base_url = "https://api.example.com/v1#fragment".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.secrets[0].value = Some("secret\nINJECTED=yes".into());
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.secrets.push(SecretUpdate {
            name: "CORTANA_UNUSED_TOKEN".into(),
            value: Some("must-not-be-written".into()),
            clear: false,
        });
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.runtime.data_dir = "/Users".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.auth_principals[0].scopes = vec!["root".into()];
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.hindsight.enabled = true;
        update.hindsight.token_env = None;
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.hindsight.base_url = "http://127.0.0.1:9000/v1?x=1".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.hindsight.base_url = "http://hindsight.example/v1".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.hindsight.provider = "openai".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.hindsight.provider = "hindsight".into();
        update.hindsight.optional = false;
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.hindsight.bank = "bad Bank".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.hindsight.base_url = "https://127.0.0.1/v1#section".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.hindsight.token_env = Some("bad-env".into());
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.honcho.enabled = true;
        update.honcho.token_env = None;
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.honcho.base_url = "http://honcho.example/v3".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.honcho.workspace_id = "work space".into();
        assert!(validate_update(&mut update).is_err());

        let mut update = valid_update(temp.path());
        update.honcho.provider = "openai".into();
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
        assert_eq!(state.ingestion.sync_freshness_hours, 48);
        assert_eq!(state.runtime.connector_timeout_seconds, 21_600);
        assert!(!state.hindsight.enabled);
        assert_eq!(state.hindsight.provider, "hindsight");
        assert_eq!(state.hindsight.base_url, "http://127.0.0.1:8888");
        assert_eq!(state.hindsight.bank, "default");
        assert!(state.hindsight.optional);
        assert!(!state.hindsight.wired_to_ingestion);
        assert!(!state.honcho.enabled);
        assert_eq!(state.honcho.provider, "honcho");
        assert_eq!(state.honcho.base_url, "https://api.honcho.dev");
        assert_eq!(state.honcho.workspace_id, "default");
        assert_eq!(state.honcho.peer_id, "cortana");
        assert_eq!(state.honcho.session_prefix, "cortana");
        assert!(state.honcho.optional);
        assert!(!state.honcho.wired_to_ingestion);

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
    fn configured_workspaces_prefer_work_personal_special_before_other_projects() {
        let config: Table = toml::from_str(
            r##"
            [[sources]]
            name = "chat-ops"
            kind = "slack"
            enabled = true
            project = "community"

            [[sources]]
            name = "notes"
            kind = "apple-notes"
            enabled = false
            project = "special"

            [[sources]]
            name = "archive"
            kind = "filesystem"
            enabled = true
            project = "work"

            [[sources]]
            name = "journal"
            kind = "filesystem"
            enabled = true
            project = "agents"

            [[sources]]
            name = "personal-box"
            kind = "filesystem"
            enabled = true
            project = "personal"
            "##,
        )
        .expect("config with implicit workspaces");

        let workspaces = configured_workspaces(&config);
        let ids: Vec<_> = workspaces
            .iter()
            .map(|workspace| workspace.id.as_str())
            .collect();
        assert_eq!(ids, vec!["work", "personal", "special"]);
        assert_eq!(workspaces[2].name, "Special");
    }

    #[test]
    fn settings_save_preserves_legacy_source_scopes_outside_visible_workspaces() {
        let temp = tempfile::tempdir().expect("temp directory");
        let config_path = temp.path().join("config/config.toml");
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::write(
            &config_path,
            r##"
            [[sources]]
            name = "work-notes"
            kind = "filesystem"
            enabled = false
            project = "work"

            [[sources]]
            name = "personal-notes"
            kind = "filesystem"
            enabled = false
            project = "personal"

            [[sources]]
            name = "special-notes"
            kind = "filesystem"
            enabled = false
            project = "special"

            [[sources]]
            name = "community-discord"
            kind = "discord"
            enabled = false
            project = "community"

            [[sources]]
            name = "agent-buzz"
            kind = "buzz"
            enabled = false
            project = "agents"
            "##,
        )
        .expect("legacy source configuration");
        let store = SettingsStore {
            config_path: config_path.clone(),
        };
        let current = store.load().expect("load legacy source configuration");
        assert_eq!(
            current
                .workspaces
                .iter()
                .map(|workspace| workspace.id.as_str())
                .collect::<Vec<_>>(),
            vec!["work", "personal", "special"]
        );

        let mut unsafe_update = valid_update(temp.path());
        unsafe_update.workspaces = current.workspaces.clone();
        unsafe_update.sources = current.sources.clone();
        unsafe_update
            .sources
            .iter_mut()
            .find(|source| source.project == "community")
            .expect("legacy community source")
            .enabled = true;
        let error = store
            .save(unsafe_update)
            .expect_err("enabled legacy sources must be assigned first");
        assert!(error.contains("not assigned to a visible workspace"));

        let mut update = valid_update(temp.path());
        update.workspaces = current.workspaces;
        update.sources = current.sources;
        store
            .save(update)
            .expect("saving visible settings must retain hidden source scopes");

        let saved = fs::read_to_string(&config_path).expect("saved configuration");
        assert!(saved.contains("id = \"community\""));
        assert!(saved.contains("id = \"agents\""));
        assert_eq!(
            store
                .load()
                .expect("reload saved configuration")
                .sources
                .len(),
            5
        );
    }

    #[test]
    fn configured_workspaces_preserve_explicit_workspace_definition() {
        let config: Table = toml::from_str(
            r##"
            [[workspaces]]
            id = "zeta"
            name = "Zeta"

            [[workspaces]]
            id = "alpha"
            name = "Alpha"

            [[sources]]
            name = "chat-ops"
            kind = "slack"
            enabled = true
            project = "work"
            "##,
        )
        .expect("config with explicit workspaces");

        let workspaces = configured_workspaces(&config);
        let ids: Vec<_> = workspaces
            .iter()
            .map(|workspace| workspace.id.as_str())
            .collect();
        assert_eq!(ids, vec!["zeta", "alpha"]);
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
            repositories: Vec::new(),
            servers: Vec::new(),
            teams: Vec::new(),
            team_names: Vec::new(),
            communities: Vec::new(),
            community_names: Vec::new(),
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
    fn discord_settings_use_desktop_rpc_paths_instead_of_token_environment() {
        let temp = tempfile::tempdir().expect("temp directory");
        let token_path = temp.path().join("discord-rpc-token.json");
        let client_path = temp.path().join("discord-rpc-client.json");
        let mut update = valid_update(temp.path());
        update.sources.push(SourceSettings {
            name: "community-discord".into(),
            kind: "discord".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: vec!["123456789012345678".into()],
            repositories: Vec::new(),
            servers: vec!["987654321098765432".into()],
            teams: Vec::new(),
            team_names: Vec::new(),
            communities: Vec::new(),
            community_names: Vec::new(),
            token_env: None,
            token_path: Some(token_path.display().to_string()),
            oauth_client_path: Some(client_path.display().to_string()),
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
        validate_update(&mut update).expect("Desktop RPC Discord source is valid");

        update.sources[0].token_env = Some("DISCORD_TOKEN_ENV_SHOULD_FAIL".into());
        let error = validate_update(&mut update).expect_err("Discord token env must be rejected");
        assert!(error.contains("token environment names are not supported"));
    }

    #[test]
    fn slack_team_assignment_is_bounded_to_one_aligned_team_per_source() {
        let temp = tempfile::tempdir().expect("temp directory");
        let mut update = valid_update(temp.path());
        let mut source = source_settings("team-slack", "slack");
        source.enabled = true;
        source.token_env = Some("SLACK_BOT_TOKEN".into());
        source.channels = vec!["C012345".into()];
        update.sources.push(source.clone());

        // A Slack user token is scoped to exactly one workspace, so the
        // config contract allows at most one team and keeps the display name
        // index-aligned.
        source.teams = vec!["T0123456789".into()];
        source.team_names = vec!["Acme Engineering".into()];
        update.sources[0] = source.clone();
        validate_update(&mut update).expect("one aligned team is valid");

        source.teams = vec!["T0123456789".into(), "T9876543210".into()];
        source.team_names = vec!["Acme Engineering".into(), "Acme Community".into()];
        update.sources[0] = source.clone();
        let error = validate_update(&mut update).expect_err("two teams must fail");
        assert!(error.contains("has too many values"));

        source.teams = vec!["T0123456789".into()];
        source.team_names = Vec::new();
        update.sources[0] = source.clone();
        let error = validate_update(&mut update).expect_err("misaligned names must fail");
        assert!(error.contains("must keep Slack team ids and names aligned"));

        // The stored id stays bounded; oversized ids fail closed.
        source.team_names = vec!["Acme Engineering".into()];
        source.teams = vec!["T".to_string() + &"x".repeat(64)];
        update.sources[0] = source.clone();
        let error = validate_update(&mut update).expect_err("oversized team id must fail");
        assert!(error.contains("must be between 1 and 12 bytes"));
    }

    #[test]
    fn buzz_community_assignment_is_bounded_and_index_aligned() {
        let temp = tempfile::tempdir().expect("temp directory");
        let mut update = valid_update(temp.path());
        let mut source = source_settings("agent-buzz", "buzz");
        source.enabled = true;
        update.sources.push(source.clone());

        // Buzz community assignment is the generic `communities` contract
        // (separate from Slack's `teams`), keeps ids and display names
        // index-aligned, and persists bounded ids and names.
        source.communities = vec!["builtin-team:welcome".into(), "team:research".into()];
        source.community_names = vec!["Welcome Team".into(), "Research".into()];
        update.sources[0] = source.clone();
        validate_update(&mut update).expect("aligned Buzz assignment is valid");

        source.communities = vec!["builtin-team:welcome".into()];
        source.community_names = Vec::new();
        update.sources[0] = source.clone();
        let error = validate_update(&mut update).expect_err("misaligned names must fail");
        assert!(error.contains("must keep Buzz community ids and names aligned"));

        // The stored ids stay bounded; oversized community ids fail closed.
        source.communities = vec!["builtin-team:welcome".into()];
        source.community_names = vec!["Welcome Team".into()];
        source.communities = vec!["x".repeat(129)];
        update.sources[0] = source.clone();
        let error = validate_update(&mut update).expect_err("oversized community id must fail");
        assert!(error.contains("must be between 1 and 128 bytes"));

        // Duplicate community ids fail closed.
        source.communities = vec!["team:research".into(), "team:research".into()];
        source.community_names = vec!["Research".into(), "Research Again".into()];
        update.sources[0] = source.clone();
        let error = validate_update(&mut update).expect_err("duplicate community id must fail");
        assert!(error.contains("contains duplicate values"));

        // The generic fields are reserved for Buzz: Slack sources keep using
        // the `teams` fields, so community assignment on any other kind fails
        // closed instead of being silently reinterpreted.
        source.communities = vec!["team:research".into()];
        source.community_names = vec!["Research".into()];
        let mut slack = source_settings("team-slack", "slack");
        slack.enabled = true;
        slack.token_env = Some("SLACK_BOT_TOKEN".into());
        slack.channels = vec!["C012345".into()];
        slack.communities = source.communities.clone();
        slack.community_names = source.community_names.clone();
        update.sources[0] = slack;
        let error = validate_update(&mut update).expect_err("communities on Slack must fail");
        assert!(error.contains("may assign communities only for Buzz sources"));
    }

    #[test]
    fn github_source_requires_a_repository_allowlist_and_token() {
        let temp = tempfile::tempdir().expect("temp directory");
        let mut update = valid_update(temp.path());
        let mut source = source_settings("github-code", "github");
        source.enabled = true;
        source.repositories = vec!["acme/project".into()];
        source.token_env = Some("GITHUB_TOKEN".into());
        update.sources.push(source.clone());
        validate_update(&mut update).expect("valid GitHub source");

        source.repositories = vec!["https://github.com/acme/project".into()];
        update.sources = vec![source.clone()];
        assert!(validate_update(&mut update).is_err());
        source.repositories = vec!["acme/project".into()];
        source.token_env = None;
        update.sources = vec![source];
        assert!(validate_update(&mut update).is_err());
        let mut token_file_source = source_settings("github-file", "github");
        token_file_source.enabled = true;
        token_file_source.repositories = vec!["acme/project".into()];
        token_file_source.token_path =
            Some(temp.path().join("github-token.json").display().to_string());
        update.sources = vec![token_file_source];
        validate_update(&mut update).expect("GitHub token file is an accepted credential path");
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

    #[test]
    fn portable_settings_redact_secrets_and_preserve_external_connectors() {
        let temp = tempfile::tempdir().expect("temp directory");
        let config_path = temp.path().join("config/config.toml");
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::write(
            &config_path,
            "[[sources]]\nname = \"custom\"\nkind = \"external\"\nenabled = false\nproject = \"work\"\ncommand = [\"/fixed/private-connector\", \"--jsonl\"]\n",
        )
        .expect("external source");
        let store = SettingsStore {
            config_path: config_path.clone(),
        };
        let mut update = valid_update(temp.path());
        update.sources = store.load().expect("load external source").sources;
        store.save(update).expect("save settings with secrets");

        let portable_path = temp.path().join("cortana-settings.json");
        let exported =
            export_portable_at(&config_path, &portable_path).expect("export portable settings");
        assert_eq!(exported.omitted_external_sources, vec!["custom"]);
        assert!(!exported.secrets_included);
        let body = fs::read_to_string(&portable_path).expect("portable settings body");
        assert!(!body.contains("not-returned"));
        assert!(!body.contains("private-bearer"));
        assert!(!body.contains("/fixed/private-connector"));
        assert!(!body.contains("\"kind\": \"external\""));

        export_portable_at(&config_path, &portable_path).expect("replace portable settings");
        let portable_backup = temp.path().join("cortana-settings.json.backup");
        assert!(portable_backup.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&portable_path, &portable_backup] {
                assert_eq!(
                    fs::metadata(path)
                        .expect("portable settings metadata")
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
        }
        let config_before_import =
            fs::read(&config_path).expect("read config before import preview");
        let audit_before_import = temp.path().join("config/desktop-audit.jsonl");
        let audit_lines_before_import = fs::read_to_string(&audit_before_import)
            .map(|body| body.lines().count())
            .unwrap_or_default();
        let imported =
            import_portable_at(&config_path, &portable_path).expect("import portable settings");
        assert_eq!(
            fs::read(&config_path).expect("read config after import preview"),
            config_before_import,
            "import must return a preview without mutating live settings"
        );
        let audit_after_import =
            fs::read_to_string(&audit_before_import).expect("read import preview audit");
        assert_eq!(
            audit_after_import.lines().count(),
            audit_lines_before_import + 1,
            "import preview must append exactly one metadata-only audit event"
        );
        assert!(
            audit_after_import
                .lines()
                .last()
                .expect("import audit line")
                .contains("settings.import.previewed")
        );
        assert_eq!(imported.preserved_external_sources, vec!["custom"]);
        assert!(
            imported
                .settings
                .sources
                .iter()
                .any(|source| source.name == "custom" && source.kind == "external")
        );
        assert!(!format!("{imported:?}").contains("private-bearer"));
        assert!(!format!("{imported:?}").contains("not-returned"));
    }

    #[test]
    fn portable_settings_reject_secrets_oversize_and_symlinks() {
        let temp = tempfile::tempdir().expect("temp directory");
        let config_path = temp.path().join("config/config.toml");
        let store = SettingsStore {
            config_path: config_path.clone(),
        };
        store
            .save(valid_update(temp.path()))
            .expect("save source settings");
        let portable_path = temp.path().join("cortana-settings.json");
        export_portable_at(&config_path, &portable_path).expect("export portable settings");

        let body = fs::read_to_string(&portable_path)
            .expect("portable settings")
            .replacen(
                "\"secrets_included\": false",
                "\"secrets_included\": true",
                1,
            );
        fs::write(&portable_path, body).expect("write unsafe portable settings");
        assert!(
            import_portable_at(&config_path, &portable_path)
                .expect_err("secret-bearing settings must fail")
                .contains("never contain secret values")
        );

        fs::write(
            &portable_path,
            vec![b' '; MAX_PORTABLE_SETTINGS_BYTES as usize + 1],
        )
        .expect("write oversized portable settings");
        assert!(
            import_portable_at(&config_path, &portable_path)
                .expect_err("oversized settings must fail")
                .contains("2 MiB")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = temp.path().join("target.json");
            fs::write(&target, "{}").expect("symlink target");
            let linked = temp.path().join("linked.json");
            symlink(&target, &linked).expect("portable settings symlink");
            assert!(
                import_portable_at(&config_path, &linked)
                    .expect_err("symlinked settings must fail")
                    .contains("symlinked")
            );

            let dangling = temp.path().join("dangling-secrets.env");
            symlink(temp.path().join("missing-secrets.env"), &dangling)
                .expect("dangling secret symlink");
            assert!(
                read_secret_map(&dangling)
                    .expect_err("dangling secret symlink must fail")
                    .contains("symlinked")
            );
        }
    }
}
