//! Shared, non-secret source health types and checks powering both the HTTP
//! status API (`/v1/status`) and the MCP `brain_status` tool.
//!
//! Everything serialized here deliberately omits credential paths, environment
//! variable names, tokens, and raw connector diagnostics.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{Config, SourceConfig};
use crate::source_validation::{self, SourceValidationStatus};

/// Cap on how large an OAuth token file may be before it is treated as
/// unreadable (defense against pathological files).
pub(crate) const MAX_TOKEN_FILE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SourceAuthorizationMethod {
    None,
    Token,
    GoogleOauth,
    GithubOauth,
    DiscordRpc,
    SlackOauth,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceAuthorizationSummary {
    pub method: SourceAuthorizationMethod,
    pub setup_required: bool,
    pub authorized: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceValidationSummary {
    pub source: String,
    pub project: String,
    pub kind: String,
    pub status: String,
    pub validated_at: String,
    pub fresh: bool,
    pub age_seconds: u64,
    /// Whether the validation covered the entire source within its limits.
    /// `false` marks a bounded sample that may authorize only equally bounded
    /// non-reconciling runs; `None` (records persisted before sampling
    /// existed) keeps its legacy full-corpus authority.
    pub complete: Option<bool>,
    pub documents: Option<usize>,
    pub bytes: Option<u64>,
    pub max_documents: usize,
    pub max_bytes: u64,
    pub max_seconds: u64,
    pub error: Option<String>,
    pub error_category: Option<&'static str>,
}

/// Safe, non-secret source configuration shared by the HTTP status API and the
/// MCP `brain_status` tool. This deliberately omits credential paths,
/// environment variable names, tokens, and connector arguments.
#[derive(Clone, Debug, Serialize)]
pub struct ConfiguredSourceStatus {
    pub name: String,
    pub source: String,
    pub kind: String,
    pub project: String,
    pub enabled: bool,
    pub acl: Vec<String>,
    pub max_documents: usize,
    pub max_bytes: u64,
    pub max_duration_seconds: u64,
    pub authorization: SourceAuthorizationSummary,
    pub validation: Option<SourceValidationSummary>,
}

/// Saturating cap for validation-age telemetry in shared status output. The
/// persisted record can be arbitrarily old (or its clock skewed), so the
/// reported age stays bounded and monotonic instead of growing without limit.
pub(crate) const MAX_STATUS_VALIDATION_AGE_SECONDS: u64 = u32::MAX as u64;

fn validation_age_seconds(validated_at: DateTime<Utc>) -> u64 {
    let age = Utc::now()
        .signed_duration_since(validated_at)
        .num_seconds()
        .max(0);
    u64::try_from(age)
        .unwrap_or_default()
        .min(MAX_STATUS_VALIDATION_AGE_SECONDS)
}

/// A validation is current when its age is within the configured freshness
/// bound. `validation_max_age_hours == 0` disables the bound, mirroring the
/// `source-validation` readiness check: an age exactly at the bound still
/// passes, and only an age strictly beyond it is expired. A future
/// `validated_at` (skewed clock) fails a bounded check, matching
/// `require_success`, while an unlimited bound keeps accepting it.
fn validation_is_fresh(validated_at: DateTime<Utc>, max_age_hours: u64) -> bool {
    if max_age_hours == 0 {
        return true;
    }
    !validation_is_future(validated_at)
        && validation_age_seconds(validated_at) <= max_age_hours.saturating_mul(3_600)
}

fn validation_is_future(validated_at: DateTime<Utc>) -> bool {
    Utc::now() < validated_at
}

/// Map a raw connector diagnostic to a bounded, non-secret failure category.
pub fn validation_error_category(error: &str) -> Option<&'static str> {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("timed out") || normalized.contains("timeout") {
        Some("timeout")
    } else if normalized.contains("403 forbidden")
        || normalized.contains("401 unauthorized")
        || (normalized.contains("400 bad request")
            && normalized.contains("oauth2.googleapis.com/token"))
        || normalized.contains("invalid_grant")
        || normalized.contains("invalid_client")
        || normalized.contains("authorization denied")
        || normalized.contains("permission denied")
    {
        Some("authorization")
    } else if normalized.contains("no such file or directory")
        || normalized.contains("does not exist")
        || normalized.contains("not found")
    {
        Some("missing-credential-or-path")
    } else if normalized.contains("exceeds")
        && (normalized.contains("budget") || normalized.contains("bound"))
    {
        Some("budget")
    } else {
        Some("connector")
    }
}

fn summarize_google_validation_error(error: &str) -> &'static str {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("invalid_grant") || normalized.contains("authorization denied") {
        "Google authorization expired or was denied; reauthorize this source"
    } else if normalized.contains("refusing partial snapshot")
        || normalized.contains("incomplete search")
        || normalized.contains("incomplete")
    {
        "google source snapshot was incomplete"
    } else if normalized.contains("not an object")
        || normalized.contains("no such id")
        || normalized.contains("has no id")
        || normalized.contains("missing id")
    {
        "google source snapshot had malformed records"
    } else if normalized.contains("conversion failed")
        || normalized.contains("detail unavailable")
        || normalized.contains("detail id mismatch")
        || normalized.contains("content unavailable")
        || normalized.contains("no supported content")
    {
        "google source snapshot had incomplete document data"
    } else {
        "source validation failed"
    }
}

/// Build the safe status summary for a persisted validation record.
pub fn validation_summary(
    status: &SourceValidationStatus,
    max_age_hours: u64,
) -> SourceValidationSummary {
    let age_seconds = validation_age_seconds(status.validated_at);
    let error = status.error.as_ref().map(|error| {
        if is_google_source(&status.kind) {
            summarize_google_validation_error(error).into()
        } else {
            "source validation failed".into()
        }
    });
    SourceValidationSummary {
        source: status.source.clone(),
        project: status.project.clone(),
        kind: status.kind.clone(),
        status: status.status.clone(),
        validated_at: status.validated_at.to_rfc3339(),
        fresh: validation_is_fresh(status.validated_at, max_age_hours),
        age_seconds,
        complete: status.complete,
        documents: status.documents,
        bytes: status.bytes,
        max_documents: status.max_documents,
        max_bytes: status.max_bytes,
        max_seconds: status.max_seconds,
        error,
        error_category: status.error.as_deref().and_then(validation_error_category),
    }
}

/// Non-secret status view of one configured source, including its
/// authorization readiness and (before [`refresh_source_validations`]) no
/// validation payload.
pub fn configured_source_status(config: &Config, source: &SourceConfig) -> ConfiguredSourceStatus {
    ConfiguredSourceStatus {
        name: source.name.clone(),
        source: source.source.clone().unwrap_or_else(|| source.name.clone()),
        kind: source.kind.clone(),
        project: source.project.clone(),
        enabled: source.enabled,
        acl: source.effective_acl(),
        max_documents: source
            .max_documents
            .unwrap_or(config.ingestion.max_documents_per_source),
        max_bytes: source
            .max_bytes
            .unwrap_or(config.ingestion.max_bytes_per_source),
        max_duration_seconds: source
            .max_duration_seconds
            .unwrap_or(config.ingestion.max_duration_seconds),
        authorization: source_authorization_summary(config, source),
        validation: None,
    }
}

/// Configuration fingerprints keyed by source name. A persisted validation is
/// only surfaced when its fingerprint still matches the current configuration.
pub fn validation_fingerprints(config: &Config) -> BTreeMap<String, String> {
    config
        .sources
        .iter()
        .filter_map(|source| {
            source_validation::configuration_fingerprint(source)
                .ok()
                .map(|fingerprint| (source.name.clone(), fingerprint))
        })
        .collect()
}

/// Attach persisted validation summaries to each configured source whose
/// configuration fingerprint still matches, computing freshness against
/// `max_age_hours`. Returns a generic message when the persisted validation
/// state cannot be read; callers surface it without details and should treat
/// every validation as absent.
pub fn refresh_source_validations(
    sources: &mut [ConfiguredSourceStatus],
    data_dir: &Path,
    max_age_hours: u64,
    fingerprints: &BTreeMap<String, String>,
) -> Result<(), String> {
    let validations =
        source_validation::load(data_dir).map_err(|_| "source validation state unavailable")?;
    for source in sources {
        source.validation = validations
            .get(&source.name)
            .filter(|validation| {
                fingerprints.get(&source.name).is_some_and(|fingerprint| {
                    validation.configuration_fingerprint.as_deref() == Some(fingerprint.as_str())
                })
            })
            .map(|validation| validation_summary(validation, max_age_hours));
    }
    Ok(())
}

pub(crate) fn is_google_source(kind: &str) -> bool {
    matches!(kind, "google-drive" | "gmail" | "google-calendar")
}

fn regular_file_ready(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| {
        if !metadata.file_type().is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o077 == 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
}

fn google_token_env_ready(config: &Config, name: &str) -> bool {
    config
        .environment_value(name)
        .as_deref()
        .is_some_and(token_destination_value_ready)
}

fn token_destination_value_ready(value: &str) -> bool {
    let path = Path::new(value.trim());
    path.is_absolute() && secure_regular_file_ready(path)
}

fn google_token_file_ready(path: &Path) -> bool {
    if !secure_regular_file_ready(path) {
        return false;
    }
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(_) => return false,
    };
    let mut bytes = Vec::new();
    if file
        .by_ref()
        .take((MAX_TOKEN_FILE_BYTES as u64) + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return false;
    }
    if bytes.len() > MAX_TOKEN_FILE_BYTES {
        return false;
    }
    let token = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(serde_json::Value::Object(token)) => token,
        _ => return false,
    };
    let has_access_token = token
        .get("token")
        .or_else(|| token.get("access_token"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|token| !token.trim().is_empty());
    let has_refresh_token = token
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|token| !token.trim().is_empty());
    let has_client_id = token
        .get("client_id")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| !value.trim().is_empty());
    has_access_token || (has_refresh_token && has_client_id)
}

fn github_token_file_ready(path: &Path) -> bool {
    if !secure_regular_file_ready(path) {
        return false;
    }
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(_) => return false,
    };
    let mut bytes = Vec::new();
    if file
        .by_ref()
        .take((MAX_TOKEN_FILE_BYTES as u64) + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > MAX_TOKEN_FILE_BYTES
    {
        return false;
    }
    let token = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(serde_json::Value::Object(token)) => token,
        _ => return false,
    };
    token
        .get("access_token")
        .or_else(|| token.get("token"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(valid_github_bearer)
}

fn github_token_env_ready(config: &Config, name: &str) -> bool {
    config
        .environment_value(name)
        .is_some_and(|value| valid_github_bearer(&value))
}

fn valid_github_bearer(value: &str) -> bool {
    let trimmed = value.trim();
    value == trimmed
        && !trimmed.is_empty()
        && trimmed.len() <= 16 * 1024
        && !trimmed
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

/// A Discord Desktop RPC token file is ready when it is a private regular
/// file whose JSON carries a plausible bearer access token. Discord never
/// falls back to an environment-variable or bot credential.
fn discord_token_file_ready(path: &Path) -> bool {
    if !secure_regular_file_ready(path) {
        return false;
    }
    let mut file = match std::fs::File::open(path) {
        Ok(file) => std::io::BufReader::new(file),
        Err(_) => return false,
    };
    let mut bytes = Vec::new();
    if file
        .by_ref()
        .take((MAX_TOKEN_FILE_BYTES as u64) + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > MAX_TOKEN_FILE_BYTES
    {
        return false;
    }
    let token = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(serde_json::Value::Object(token)) => token,
        _ => return false,
    };
    token
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .is_some_and(valid_github_bearer)
}

fn slack_token_file_ready(path: &Path) -> bool {
    // Slack user tokens are stored in the same owner-only JSON contract as
    // Discord's: a bounded `access_token` field is enough to consider the
    // browser-authorization path complete.
    discord_token_file_ready(path)
}

fn secure_regular_file_ready(path: &Path) -> bool {
    regular_file_ready(path) && private_path_components_ready(path)
}

fn private_path_components_ready(path: &Path) -> bool {
    let mut current = path.to_path_buf();
    loop {
        match std::fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.file_type().is_symlink() && !is_allowed_system_alias(&current) =>
            {
                return false;
            }
            Ok(metadata) if current == path => {
                if !metadata.is_file() {
                    return false;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if metadata.permissions().mode() & 0o077 != 0 {
                        return false;
                    }
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
            Err(_) => return false,
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    true
}

fn is_allowed_system_alias(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        path == Path::new("/tmp") || path == Path::new("/var") || path == Path::new("/etc")
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

/// Authorization readiness of a configured source, without ever revealing how
/// credentials are stored or where they live.
pub fn source_authorization_summary(
    config: &Config,
    source: &SourceConfig,
) -> SourceAuthorizationSummary {
    if is_google_source(&source.kind) {
        let oauth_client_ready = source
            .oauth_client
            .as_ref()
            .is_some_and(|path| secure_regular_file_ready(path.as_path()));
        let token_env_ready = source
            .token_env
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty() && google_token_env_ready(config, name));
        let token_file_ready = source
            .token
            .as_ref()
            .is_some_and(|path| google_token_file_ready(path.as_path()));
        // Google may still use a token environment variable. Discord does not:
        // its RPC token destination is always the explicit `token` field.
        let token_destination_ready = source.token.as_deref().is_some_and(token_destination_ready);
        SourceAuthorizationSummary {
            method: SourceAuthorizationMethod::GoogleOauth,
            // A migrated/private token file is a complete authorization path on
            // its own. Requiring an OAuth client in that case makes an already
            // authorized Google source appear unhealthy in the desktop status
            // panel and incorrectly invites the user to repeat setup.
            setup_required: !(token_env_ready
                || token_file_ready
                || (oauth_client_ready && token_destination_ready)),
            authorized: token_env_ready || token_file_ready,
        }
    } else if source.kind == "github" {
        let oauth_client_ready = source
            .oauth_client
            .as_ref()
            .is_some_and(|path| secure_regular_file_ready(path.as_path()));
        let token_env_ready = source
            .token_env
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty() && github_token_env_ready(config, name));
        let token_file_ready = source
            .token
            .as_ref()
            .is_some_and(|path| github_token_file_ready(path.as_path()));
        let token_destination_ready = source.token.as_deref().is_some_and(token_destination_ready);
        SourceAuthorizationSummary {
            method: SourceAuthorizationMethod::GithubOauth,
            setup_required: !(token_env_ready
                || token_file_ready
                || (oauth_client_ready && token_destination_ready)),
            authorized: token_env_ready || token_file_ready,
        }
    } else if source.kind == "slack" && (source.oauth_client.is_some() || source.token.is_some()) {
        // Browser OAuth for Slack assigns workspaces (teams) to this source's
        // workspace. The bot token environment variable remains the
        // operational sync credential and is reported by the token branch
        // below when no OAuth paths are configured; it is a credential, never
        // a path, so the user-token destination is always the explicit
        // `token` field.
        let oauth_client_ready = source
            .oauth_client
            .as_ref()
            .is_some_and(|path| secure_regular_file_ready(path.as_path()));
        let token_file_ready = source
            .token
            .as_ref()
            .is_some_and(|path| slack_token_file_ready(path.as_path()));
        let token_destination_ready = source.token.as_deref().is_some_and(token_destination_ready);
        SourceAuthorizationSummary {
            method: SourceAuthorizationMethod::SlackOauth,
            setup_required: !(token_file_ready || (oauth_client_ready && token_destination_ready)),
            authorized: token_file_ready,
        }
    } else if source.kind == "discord" {
        // Discord Desktop RPC is the sole Discord authorization path. The
        // user token is explicitly stored and is never sourced from an
        // environment variable or interpreted as a bot credential.
        let oauth_client_ready = source
            .oauth_client
            .as_ref()
            .is_some_and(|path| secure_regular_file_ready(path.as_path()));
        let token_file_ready = source
            .token
            .as_ref()
            .is_some_and(|path| discord_token_file_ready(path.as_path()));
        let token_destination_ready = source.token.as_deref().is_some_and(token_destination_ready);
        SourceAuthorizationSummary {
            method: SourceAuthorizationMethod::DiscordRpc,
            setup_required: !(token_file_ready || (oauth_client_ready && token_destination_ready)),
            authorized: token_file_ready,
        }
    } else if source.token_env.is_some() || source.token.is_some() {
        let token_env_ready = source.token_env.as_deref().is_some_and(|name| {
            !name.trim().is_empty()
                && config
                    .environment_value(name)
                    .is_some_and(|value| !value.is_empty())
        });
        let token_file_ready = source
            .token
            .as_ref()
            .is_some_and(|path| secure_regular_file_ready(path.as_path()));
        SourceAuthorizationSummary {
            method: SourceAuthorizationMethod::Token,
            setup_required: !token_env_ready && !token_file_ready,
            authorized: token_env_ready || token_file_ready,
        }
    } else {
        SourceAuthorizationSummary {
            method: SourceAuthorizationMethod::None,
            setup_required: false,
            authorized: true,
        }
    }
}

fn token_destination_ready(path: &Path) -> bool {
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
        return false;
    }
    let mut current = path.to_path_buf();
    loop {
        match std::fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.file_type().is_symlink() && !is_allowed_system_alias(&current) =>
            {
                return false;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    true
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{validation_error_category, validation_summary};
    use crate::config::SourceConfig;
    use crate::source_validation::SourceValidationStatus;

    fn source(
        kind: &str,
        token: Option<std::path::PathBuf>,
        oauth_client: Option<std::path::PathBuf>,
    ) -> SourceConfig {
        SourceConfig {
            name: "community".into(),
            kind: kind.into(),
            enabled: true,
            project: "community".into(),
            root: None,
            source: None,
            channels: vec!["175928847299117064".into()],
            servers: Vec::new(),
            teams: Vec::new(),
            team_names: Vec::new(),
            communities: Vec::new(),
            community_names: Vec::new(),
            repositories: Vec::new(),
            token_env: (kind == "slack").then_some("SLACK_BOT_TOKEN".into()),
            token,
            oauth_client,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        }
    }

    #[test]
    fn discord_rpc_authorization_is_reported_only_when_rpc_paths_exist() {
        use crate::source_status::SourceAuthorizationMethod;

        let directory = tempfile::tempdir().unwrap();
        let token_path = directory.path().join("discord-token.json");
        std::fs::write(&token_path, "{\"access_token\":\"valid-token\"}").unwrap();
        #[cfg(unix)]
        crate::oauth_common::set_owner_only(&token_path).unwrap();
        let client_path = directory.path().join("discord-oauth-client.json");
        std::fs::write(&client_path, "{\"client_id\":\"client-id\"}").unwrap();
        #[cfg(unix)]
        crate::oauth_common::set_owner_only(&client_path).unwrap();

        // A Discord source without explicit RPC paths is not authorized and
        // must not fall back to a token environment variable.
        let mut config = crate::config::Config::default();
        config.sources.push(source("discord", None, None));
        let summary = super::source_authorization_summary(&config, &config.sources[0]);
        assert_eq!(summary.method, SourceAuthorizationMethod::DiscordRpc);
        assert!(!summary.authorized);
        assert!(summary.setup_required);

        // RPC paths report the stored user token state without contacting
        // Discord Desktop RPC or requiring a client prompt during status generation.
        config.sources[0] = source(
            "discord",
            Some(token_path.clone()),
            Some(client_path.clone()),
        );
        let summary = super::source_authorization_summary(&config, &config.sources[0]);
        assert_eq!(summary.method, SourceAuthorizationMethod::DiscordRpc);
        assert!(summary.authorized);
        assert!(!summary.setup_required);

        // Without the user token file, desktop authorization is still
        // required but no setup remains: the client file and token
        // destination are both configured.
        std::fs::remove_file(&token_path).unwrap();
        config.sources[0] = source("discord", Some(token_path), Some(client_path));
        let summary = super::source_authorization_summary(&config, &config.sources[0]);
        assert_eq!(summary.method, SourceAuthorizationMethod::DiscordRpc);
        assert!(!summary.authorized);
        assert!(!summary.setup_required);

        let json = serde_json::to_value(&summary).expect("summary serializes");
        assert_eq!(
            json.get("method"),
            Some(&serde_json::Value::String("discord_rpc".into()))
        );
    }

    #[test]
    fn slack_oauth_authorization_is_reported_only_when_oauth_paths_exist() {
        use crate::source_status::SourceAuthorizationMethod;

        let directory = tempfile::tempdir().unwrap();
        let token_path = directory.path().join("slack-token.json");
        std::fs::write(&token_path, "{\"access_token\":\"valid-token\"}").unwrap();
        #[cfg(unix)]
        crate::oauth_common::set_owner_only(&token_path).unwrap();
        let client_path = directory.path().join("slack-oauth-client.json");
        std::fs::write(&client_path, "{\"client_id\":\"client-id\"}").unwrap();
        #[cfg(unix)]
        crate::oauth_common::set_owner_only(&client_path).unwrap();

        // Bot-token-only sources keep the plain token method (fallback): the
        // bot token environment variable is a credential, never a path. The
        // the fixture supplies Slack's operational bot credential for the
        // token-only branch.
        let mut config = crate::config::Config::default();
        config.sources.push(source("slack", None, None));
        config
            .environment
            .insert("SLACK_BOT_TOKEN".into(), "bot-token".into());
        let summary = super::source_authorization_summary(&config, &config.sources[0]);
        assert_eq!(summary.method, SourceAuthorizationMethod::Token);
        assert!(summary.authorized);

        // OAuth paths flip the method to slack_oauth and gate
        // authorization on the stored user token file.
        config.sources[0] = source("slack", Some(token_path.clone()), Some(client_path.clone()));
        let summary = super::source_authorization_summary(&config, &config.sources[0]);
        assert_eq!(summary.method, SourceAuthorizationMethod::SlackOauth);
        assert!(summary.authorized);
        assert!(!summary.setup_required);

        // Without the user token file, browser authorization is still
        // required but no setup remains: the client file and the token
        // destination are both configured.
        std::fs::remove_file(&token_path).unwrap();
        config.sources[0] = source("slack", Some(token_path), Some(client_path));
        let summary = super::source_authorization_summary(&config, &config.sources[0]);
        assert_eq!(summary.method, SourceAuthorizationMethod::SlackOauth);
        assert!(!summary.authorized);
        assert!(!summary.setup_required);

        let json = serde_json::to_value(&summary).expect("summary serializes");
        assert_eq!(
            json.get("method"),
            Some(&serde_json::Value::String("slack_oauth".into()))
        );
    }

    #[test]
    fn slack_oauth_authorization_summary_does_not_use_bot_token_env_as_user_token_destination() {
        use crate::source_status::SourceAuthorizationMethod;

        let directory = tempfile::tempdir().unwrap();
        let client_path = directory.path().join("slack-oauth-client.json");
        let token_path = directory.path().join("fake-bot-token.json");

        std::fs::write(&client_path, "{\"client_id\":\"client-id\"}").unwrap();
        std::fs::write(&token_path, "not-a-token").unwrap();
        #[cfg(unix)]
        {
            crate::oauth_common::set_owner_only(&client_path).unwrap();
            crate::oauth_common::set_owner_only(&token_path).unwrap();
        }

        let mut config = crate::config::Config::default();
        config.environment.insert(
            "SLACK_BOT_TOKEN".into(),
            token_path.to_string_lossy().into(),
        );
        config
            .sources
            .push(source("slack", None, Some(client_path)));

        let summary = super::source_authorization_summary(&config, &config.sources[0]);
        assert_eq!(summary.method, SourceAuthorizationMethod::SlackOauth);
        assert!(!summary.authorized);
        assert!(summary.setup_required);
    }

    fn validated_status(complete: Option<bool>) -> SourceValidationStatus {
        SourceValidationStatus {
            source: "docs".into(),
            project: "agents".into(),
            kind: "filesystem".into(),
            status: "succeeded".into(),
            validated_at: Utc::now(),
            documents: Some(12),
            bytes: Some(512),
            max_documents: 25,
            max_bytes: 1024,
            max_seconds: 60,
            configuration_fingerprint: None,
            complete,
            error: None,
        }
    }

    #[test]
    fn summary_exposes_a_bounded_sample_as_incomplete() {
        let summary = validation_summary(&validated_status(Some(false)), 168);
        assert_eq!(summary.complete, Some(false));
        let json = serde_json::to_value(&summary).expect("summary serializes");
        assert_eq!(json.get("complete"), Some(&serde_json::Value::Bool(false)));
    }

    #[test]
    fn oauth_token_refresh_failures_are_authorization_categories() {
        for error in [
            "Client error '400 Bad Request' for url 'https://oauth2.googleapis.com/token'",
            "Google token exchange failed: invalid_grant",
            "provider returned invalid_client",
        ] {
            assert_eq!(
                validation_error_category(error),
                Some("authorization"),
                "OAuth failure should be actionable: {error}"
            );
        }
    }

    #[test]
    fn google_oauth_summary_requests_reauthorization_without_provider_detail() {
        let mut status = validated_status(None);
        status.kind = "google-calendar".into();
        status.status = "failed".into();
        status.error = Some("Google OAuth refresh failed (400: invalid_grant)".into());

        let summary = validation_summary(&status, 168);
        assert_eq!(
            summary.error.as_deref(),
            Some("Google authorization expired or was denied; reauthorize this source")
        );
        assert_eq!(summary.error_category, Some("authorization"));
    }

    #[test]
    fn summary_keeps_legacy_records_without_completeness() {
        let summary = validation_summary(&validated_status(None), 168);
        assert_eq!(summary.complete, None);
        let json = serde_json::to_value(&summary).expect("summary serializes");
        // A legacy record keeps a null completeness marker; consumers treat it
        // exactly like the pre-sampling era.
        assert_eq!(json.get("complete"), Some(&serde_json::Value::Null));

        // The persisted record round-trips without a completeness key, so
        // records written before sampling existed stay compatible.
        let encoded = serde_json::to_string(&validated_status(None)).expect("record serializes");
        assert!(!encoded.contains("\"complete\""));
        let decoded: SourceValidationStatus =
            serde_json::from_str(&encoded).expect("legacy record deserializes");
        assert_eq!(decoded.complete, None);
    }
}
