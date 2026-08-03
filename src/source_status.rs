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

/// Cap on how large a Google token file may be before it is treated as
/// unreadable (defense against pathological files).
pub(crate) const MAX_GOOGLE_TOKEN_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceAuthorizationMethod {
    None,
    Token,
    GoogleOauth,
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
    if normalized.contains("refusing partial snapshot")
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
        acl: source.acl.clone(),
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
        .is_some_and(google_token_destination_value_ready)
}

fn google_token_destination_value_ready(value: &str) -> bool {
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
        .take((MAX_GOOGLE_TOKEN_BYTES as u64) + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return false;
    }
    if bytes.len() > MAX_GOOGLE_TOKEN_BYTES {
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
        let token_destination_ready = source
            .token
            .as_deref()
            .is_some_and(google_token_destination_ready)
            || source
                .token_env
                .as_deref()
                .and_then(|name| config.environment_value(name))
                .as_deref()
                .is_some_and(google_token_destination_value_ready);
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

fn google_token_destination_ready(path: &Path) -> bool {
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
