use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use reqwest::{Client, Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{
    AppHandle, Manager, State, Wry,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_autostart::ManagerExt;

mod backups;
mod hindsight;
mod honcho;
mod installer;
mod paths;
mod provider_models;
mod readiness;
mod schedule;
mod scheduled_services;
mod services;
mod settings;
mod source_jobs;
mod updater;

const BACKEND_ORIGIN: &str = "http://127.0.0.1:7331";
const MAIN_WINDOW: &str = "main";
const MAX_QUERY_LENGTH: usize = 16_384;
const MAX_SCOPE_LENGTH: usize = 256;
const MAX_DOCUMENT_CURSOR_LENGTH: usize = 1024;
const MAX_DOCUMENT_ID_LENGTH: usize = 128;
const MAX_BACKEND_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const PROJECT_URL: &str = "https://github.com/0xPlayerOne/cortana";
static QUITTING: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct BackendClient {
    http: Client,
}

impl BackendClient {
    fn new() -> Result<Self, reqwest::Error> {
        Ok(Self {
            http: Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(65))
                .build()?,
        })
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let url = Url::parse(&format!("{BACKEND_ORIGIN}{path}"))
            .map_err(|error| format!("invalid fixed Cortana runtime URL: {error}"))?;
        self.request_url(method, url, body).await
    }

    async fn request_url(
        &self,
        method: Method,
        url: Url,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let scope = match url.path() {
            "/metrics" | "/v1/audit" => "admin",
            "/v1/status" => "status",
            _ => "query",
        };
        let mut request = self.http.request(method, url);
        if let Some(token) = desktop_bearer_for_scope(scope)? {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|error| format!("Cortana runtime is unavailable: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "Cortana runtime request failed with status {}",
                status.as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BACKEND_RESPONSE_BYTES as u64)
        {
            return Err(format!(
                "Cortana runtime response exceeded the {MAX_BACKEND_RESPONSE_BYTES} byte Desktop safety limit"
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("read Cortana runtime response: {error}"))?
        {
            if body.len().saturating_add(chunk.len()) > MAX_BACKEND_RESPONSE_BYTES {
                return Err(format!(
                    "Cortana runtime response exceeded the {MAX_BACKEND_RESPONSE_BYTES} byte Desktop safety limit"
                ));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body)
            .map_err(|error| format!("Cortana runtime returned an invalid response: {error}"))
    }
}

/// Desktop is the owner-local control plane. If a named auth principal carries
/// both `admin` and the requested scope, prefer it for loopback requests so the
/// UI does not accidentally render a narrow agent's ACL as the whole corpus.
/// Fall back to the requested scope when no shared-scope owner credential is
/// configured.
fn desktop_bearer_for_scope(scope: &str) -> Result<Option<String>, String> {
    if scope != "admin" {
        if let Ok(snapshot) = settings::load() {
            for principal in snapshot
                .auth_principals
                .iter()
                .filter(|principal| principal_supports_owner_scope(principal, scope))
            {
                if let Ok(Some(token)) = settings::secret_value_for_env(&principal.token_env) {
                    return Ok(Some(token));
                }
            }
        }
    }
    settings::bearer_for_scope(scope)
}

fn principal_supports_owner_scope(
    principal: &settings::AuthPrincipalSettings,
    scope: &str,
) -> bool {
    principal.scopes.iter().any(|value| value == "admin")
        && principal.scopes.iter().any(|value| value == scope)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RetrievalRequest {
    query: String,
    project: Option<String>,
    source: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentListRequest {
    project: Option<String>,
    source: Option<String>,
    query: Option<String>,
    cursor: Option<String>,
    limit: usize,
}

#[derive(Debug, Serialize)]
struct DesktopInfo {
    desktop_version: &'static str,
    backend_origin: &'static str,
    autostart_enabled: bool,
    platform: &'static str,
}

#[derive(Clone)]
struct TrayStatus {
    health: MenuItem<Wry>,
    corpus: MenuItem<Wry>,
    ingestion: MenuItem<Wry>,
    source_jobs: MenuItem<Wry>,
}

#[tauri::command]
async fn brain_status(backend: State<'_, BackendClient>) -> Result<Value, String> {
    backend.request(Method::GET, "/v1/status", None).await
}

#[tauri::command]
async fn brain_answer(
    backend: State<'_, BackendClient>,
    request: RetrievalRequest,
) -> Result<Value, String> {
    validate_retrieval_request(&request)?;
    backend
        .request(
            Method::POST,
            "/v1/answer",
            Some(serde_json::to_value(request).map_err(|error| error.to_string())?),
        )
        .await
}

#[tauri::command]
async fn brain_context(
    backend: State<'_, BackendClient>,
    request: RetrievalRequest,
) -> Result<Value, String> {
    validate_retrieval_request(&request)?;
    let mut value = serde_json::to_value(request).map_err(|error| error.to_string())?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "invalid context request".to_string())?;
    object.insert("limit".into(), 20.into());
    object.insert("max_tokens".into(), 8_000.into());
    backend
        .request(Method::POST, "/v1/context", Some(value))
        .await
}

#[tauri::command]
async fn brain_documents(
    backend: State<'_, BackendClient>,
    request: DocumentListRequest,
) -> Result<Value, String> {
    validate_document_list_request(&request)?;
    let mut url = Url::parse(BACKEND_ORIGIN)
        .map_err(|error| format!("invalid fixed Cortana runtime URL: {error}"))?;
    url.set_path("/v1/documents");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("limit", &request.limit.to_string());
        if let Some(project) = request.project {
            query.append_pair("project", &project);
        }
        if let Some(source) = request.source {
            query.append_pair("source", &source);
        }
        if let Some(document_query) = request.query {
            query.append_pair("query", &document_query);
        }
        if let Some(cursor) = request.cursor {
            query.append_pair("cursor", &cursor);
        }
    }
    backend.request_url(Method::GET, url, None).await
}

#[tauri::command]
async fn brain_graph(
    backend: State<'_, BackendClient>,
    request: DocumentListRequest,
) -> Result<Value, String> {
    validate_document_list_request(&request)?;
    let mut url = Url::parse(BACKEND_ORIGIN)
        .map_err(|error| format!("invalid fixed Cortana runtime URL: {error}"))?;
    url.set_path("/v1/graph");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("limit", &request.limit.to_string());
        if let Some(project) = request.project {
            query.append_pair("project", &project);
        }
        if let Some(source) = request.source {
            query.append_pair("source", &source);
        }
        if let Some(document_query) = request.query {
            query.append_pair("query", &document_query);
        }
        if let Some(cursor) = request.cursor {
            query.append_pair("cursor", &cursor);
        }
    }
    backend.request_url(Method::GET, url, None).await
}

#[tauri::command]
async fn brain_document(backend: State<'_, BackendClient>, id: String) -> Result<Value, String> {
    validate_document_id(&id)?;
    backend
        .request(Method::GET, &format!("/v1/documents/{id}"), None)
        .await
}

#[tauri::command]
async fn brain_audit(backend: State<'_, BackendClient>, limit: usize) -> Result<Value, String> {
    if !(1..=500).contains(&limit) {
        return Err("runtime audit limit must be between 1 and 500".into());
    }
    let mut url = Url::parse(BACKEND_ORIGIN)
        .map_err(|error| format!("invalid fixed Cortana runtime URL: {error}"))?;
    url.set_path("/v1/audit");
    url.query_pairs_mut()
        .append_pair("limit", &limit.to_string());
    backend.request_url(Method::GET, url, None).await
}

#[tauri::command]
fn desktop_audit(limit: usize) -> Result<Vec<Value>, String> {
    settings::desktop_audit_events(limit)
}

#[tauri::command]
fn desktop_project_open() -> Result<(), String> {
    open::that(PROJECT_URL).map_err(|error| format!("open Cortana project page: {error}"))
}

#[tauri::command]
fn desktop_secret_file_open() -> Result<(), String> {
    let path = settings::load()?.secret_file_path;
    if !std::path::Path::new(&path).is_file() {
        return Err("secret file is unavailable".into());
    }
    open::that_detached(&path).map_err(|error| format!("open secret file: {error}"))?;
    let event = serde_json::json!({
        "at_unix_seconds": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "event": "desktop.secret_file.opened",
        "secret_values_recorded": false,
    });
    let _ = settings::append_audit_event(&settings::default_config_path(), &event);
    Ok(())
}

fn validate_external_url(url: &str) -> Result<(), String> {
    let parsed = Url::parse(url).map_err(|error| format!("invalid URL: {error}"))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("external links must not contain embedded credentials".into());
    }
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        "mailto" => Ok(()),
        "slack" => validate_slack_url(&parsed),
        "notes" => validate_notes_url(&parsed),
        "buzz" => validate_buzz_url(&parsed),
        "file" => {
            if parsed
                .host_str()
                .is_some_and(|host| !host.is_empty() && host != "localhost")
            {
                return Err("file links must be local paths".into());
            }
            if parsed.query().is_some() || parsed.fragment().is_some() {
                return Err("file links must not contain query or fragment data".into());
            }
            parsed
                .to_file_path()
                .map_err(|_| "file links must contain an absolute local path".to_string())?;
            Ok(())
        }
        _ => Err(format!("unsupported URL scheme: {}", parsed.scheme())),
    }
}

fn validate_slack_url(url: &Url) -> Result<(), String> {
    if url.host_str() != Some("channel") || !url.path().is_empty() || url.fragment().is_some() {
        return Err("Slack links must target a channel deep link".into());
    }
    let mut team = None;
    let mut channel = None;
    let mut message = None;
    for (key, value) in url.query_pairs() {
        let slot = match key.as_ref() {
            "team" => &mut team,
            "id" => &mut channel,
            "message" => &mut message,
            _ => return Err("Slack links contain unsupported query data".into()),
        };
        if slot.replace(value.into_owned()).is_some() {
            return Err("Slack links must not repeat query fields".into());
        }
    }
    let channel = channel.ok_or_else(|| "Slack links must include a channel id".to_string())?;
    if !valid_slack_identifier(&channel) {
        return Err("Slack links contain an invalid channel id".into());
    }
    let message =
        message.ok_or_else(|| "Slack links must include a message timestamp".to_string())?;
    if !valid_slack_timestamp(&message) {
        return Err("Slack links contain an invalid message timestamp".into());
    }
    if team
        .as_deref()
        .is_some_and(|value| !value.is_empty() && !valid_slack_identifier(value))
    {
        return Err("Slack links contain an invalid team id".into());
    }
    Ok(())
}

fn validate_notes_url(url: &Url) -> Result<(), String> {
    if url
        .host_str()
        .is_none_or(|host| !host.eq_ignore_ascii_case("shownote"))
        || !url.path().is_empty()
        || url.fragment().is_some()
    {
        return Err("Apple Notes links must target a note deep link".into());
    }
    let mut identifier = None;
    for (key, value) in url.query_pairs() {
        if key != "identifier" || identifier.replace(value.into_owned()).is_some() {
            return Err("Apple Notes links contain unsupported query data".into());
        }
    }
    let identifier =
        identifier.ok_or_else(|| "Apple Notes links must include a note identifier".to_string())?;
    validate_custom_link_value(&identifier, 1024, true)
        .then_some(())
        .ok_or_else(|| "Apple Notes links contain an invalid note identifier".into())
}

fn validate_buzz_url(url: &Url) -> Result<(), String> {
    if url
        .host_str()
        .is_none_or(|host| !host.eq_ignore_ascii_case("persona"))
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Buzz links must target a persona deep link".into());
    }
    if contains_forbidden_encoded_path_byte(url.path()) {
        return Err("Buzz links contain an invalid encoded path segment".into());
    }
    let segments = url
        .path_segments()
        .ok_or_else(|| "Buzz links must contain persona path segments".to_string())?
        .collect::<Vec<_>>();
    if segments.len() != 2
        || segments
            .iter()
            .any(|segment| !validate_custom_link_value(segment, 256, false))
    {
        return Err("Buzz links contain invalid persona data".into());
    }
    Ok(())
}

fn contains_forbidden_encoded_path_byte(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return true;
        }
        let Some(high) = (bytes[index + 1] as char).to_digit(16) else {
            return true;
        };
        let Some(low) = (bytes[index + 2] as char).to_digit(16) else {
            return true;
        };
        let decoded = (high * 16 + low) as u8;
        if decoded == b'/' || decoded == b'\\' || decoded < 0x20 || decoded == 0x7f {
            return true;
        }
        index += 3;
    }
    false
}

fn validate_custom_link_value(value: &str, maximum_length: usize, allow_slash: bool) -> bool {
    !value.is_empty()
        && value.len() <= maximum_length
        && (allow_slash || !value.contains('/'))
        && !value.chars().any(|character| character.is_control())
}

fn valid_slack_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_slack_timestamp(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    if let Some((whole, fraction)) = value.split_once('.') {
        !whole.is_empty()
            && !fraction.is_empty()
            && whole.bytes().all(|byte| byte.is_ascii_digit())
            && fraction.bytes().all(|byte| byte.is_ascii_digit())
    } else {
        value.bytes().all(|byte| byte.is_ascii_digit())
    }
}

#[tauri::command]
fn desktop_url_open(url: String) -> Result<(), String> {
    validate_external_url(&url)?;
    let parsed = Url::parse(&url).map_err(|error| format!("invalid URL: {error}"))?;
    if parsed.scheme() == "file" {
        let target = configured_file_target(&parsed)?;
        return open::that_detached(target).map_err(|error| format!("open local source: {error}"));
    }
    open::that_detached(url).map_err(|error| format!("open external URL: {error}"))
}

fn configured_file_target(url: &Url) -> Result<PathBuf, String> {
    let target = url
        .to_file_path()
        .map_err(|_| "file links must contain an absolute local path".to_string())?;
    let target =
        fs::canonicalize(&target).map_err(|error| format!("resolve local source path: {error}"))?;
    let settings = settings::load()?;
    let roots = settings
        .sources
        .iter()
        .filter(|source| source.kind == "filesystem")
        .filter_map(|source| source.root.as_deref())
        .map(Path::new)
        .filter_map(|root| fs::canonicalize(root).ok())
        .collect::<Vec<_>>();
    if is_within_filesystem_root(&target, &roots) {
        return Ok(target);
    }
    Err("local source links must stay inside a configured filesystem source root".into())
}

fn is_within_filesystem_root(target: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| target.starts_with(root))
}

#[tauri::command]
fn desktop_info<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> DesktopInfo {
    DesktopInfo {
        desktop_version: env!("CARGO_PKG_VERSION"),
        backend_origin: BACKEND_ORIGIN,
        autostart_enabled: app.autolaunch().is_enabled().unwrap_or(false),
        platform: std::env::consts::OS,
    }
}

#[tauri::command]
fn desktop_autostart_set(app: AppHandle, enabled: bool) -> Result<DesktopInfo, String> {
    if enabled {
        app.autolaunch()
            .enable()
            .map_err(|error| format!("enable Desktop at login: {error}"))?;
    } else {
        app.autolaunch()
            .disable()
            .map_err(|error| format!("disable Desktop at login: {error}"))?;
    }
    let event = serde_json::json!({
        "at_unix_seconds": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "event": "desktop.autostart.changed",
        "enabled": enabled,
        "secret_values_recorded": false,
    });
    let _ = settings::append_audit_event(&settings::default_config_path(), &event);
    Ok(desktop_info(app))
}

#[tauri::command]
async fn desktop_services_status<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<services::ServiceReport, String> {
    services::status(&app).await
}

#[tauri::command]
async fn desktop_services_install(
    app: AppHandle,
    approved: bool,
) -> Result<services::ServiceReport, String> {
    scheduled_services::install_core(&app, approved).await
}

#[tauri::command]
async fn desktop_services_install_sync(
    app: AppHandle,
    approved: bool,
) -> Result<services::ServiceReport, String> {
    scheduled_services::install_sync(&app, approved).await
}

#[tauri::command]
fn desktop_schedule_get() -> Result<schedule::ScheduleSettings, String> {
    schedule::load()
}

#[tauri::command]
fn desktop_schedule_save(
    schedule: schedule::ScheduleSettings,
) -> Result<schedule::ScheduleSettings, String> {
    schedule::save(schedule)
}

#[tauri::command]
async fn desktop_hindsight_status() -> Result<hindsight::HindsightStatus, String> {
    hindsight::status().await
}

#[tauri::command]
async fn desktop_honcho_status() -> Result<honcho::HonchoStatus, String> {
    honcho::status().await
}

#[tauri::command]
async fn desktop_service_action(
    app: AppHandle,
    service: String,
    action: String,
    approved: bool,
) -> Result<services::ServiceReport, String> {
    services::action(&app, &service, &action, approved).await
}

#[tauri::command]
async fn desktop_services_action_all(
    app: AppHandle,
    action: String,
    approved: bool,
) -> Result<services::ServiceReport, String> {
    services::action_all(&app, &action, approved).await
}

#[tauri::command]
async fn desktop_database_backup(
    app: AppHandle,
    approved: bool,
) -> Result<Option<backups::DatabaseActionResult>, String> {
    backups::backup(&app, approved).await
}

#[tauri::command]
async fn desktop_database_restore(
    app: AppHandle,
    approved: bool,
) -> Result<Option<backups::DatabaseActionResult>, String> {
    backups::restore(&app, approved).await
}

#[tauri::command]
fn desktop_update_status(updater: State<'_, updater::UpdaterState>) -> updater::UpdateSnapshot {
    updater.status()
}

#[tauri::command]
async fn desktop_update_check<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    updater: State<'_, updater::UpdaterState>,
) -> Result<updater::UpdateSnapshot, String> {
    updater.check(&app).await
}

#[tauri::command]
async fn desktop_update_install(
    app: AppHandle,
    updater: State<'_, updater::UpdaterState>,
    expected_version: String,
    approved: bool,
    restart: bool,
) -> Result<updater::UpdateSnapshot, String> {
    updater
        .install(&app, &expected_version, approved, restart)
        .await
}

#[tauri::command]
fn desktop_settings_get() -> Result<settings::SettingsSnapshot, String> {
    settings::load()
}

#[tauri::command]
fn desktop_settings_save(
    update: settings::SettingsUpdate,
) -> Result<settings::SettingsSnapshot, String> {
    settings::save(update)
}

#[tauri::command]
async fn desktop_settings_export(
    app: AppHandle,
) -> Result<Option<settings::PortableExport>, String> {
    let Some(path) = paths::pick(app, "settings-export").await? else {
        return Ok(None);
    };
    settings::export_portable(std::path::Path::new(&path)).map(Some)
}

#[tauri::command]
async fn desktop_settings_import(
    app: AppHandle,
) -> Result<Option<settings::PortableImport>, String> {
    let Some(path) = paths::pick(app, "settings-import").await? else {
        return Ok(None);
    };
    settings::import_portable(std::path::Path::new(&path)).map(Some)
}

#[tauri::command]
fn desktop_installer_start(
    app: AppHandle,
    installer: State<'_, installer::InstallerState>,
    tool: String,
    approved: bool,
) -> Result<installer::InstallJobSnapshot, String> {
    installer.start_with_app(Some(&app), &tool, approved)
}

#[tauri::command]
fn desktop_installer_status(
    installer: State<'_, installer::InstallerState>,
    id: String,
) -> Result<installer::InstallJobSnapshot, String> {
    installer.status(&id)
}

#[tauri::command]
fn desktop_installer_cancel(
    installer: State<'_, installer::InstallerState>,
    id: String,
) -> Result<installer::InstallJobSnapshot, String> {
    installer.cancel(&id)
}

#[tauri::command]
fn desktop_source_validation_start<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    jobs: State<'_, source_jobs::SourceJobState>,
    source: String,
    budget: Option<source_jobs::InitialSyncBudget>,
) -> Result<source_jobs::SourceJobSnapshot, String> {
    jobs.start_validation(&app, &source, budget)
}

#[tauri::command]
fn desktop_source_authorization_start<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    jobs: State<'_, source_jobs::SourceJobState>,
    source: String,
) -> Result<source_jobs::SourceJobSnapshot, String> {
    jobs.start_authorization(&app, &source)
}

#[tauri::command]
fn desktop_source_trial_sync_start(
    app: AppHandle,
    jobs: State<'_, source_jobs::SourceJobState>,
    source: String,
    approved: bool,
) -> Result<source_jobs::SourceJobSnapshot, String> {
    jobs.start_trial_sync(&app, &source, approved)
}

#[tauri::command]
fn desktop_source_setup_open(source: String) -> Result<source_jobs::SetupOpenOutcome, String> {
    source_jobs::open_setup(&source)
}

#[tauri::command]
async fn desktop_github_repositories<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    source: String,
) -> Result<Value, String> {
    source_jobs::list_github_repositories(&app, &source).await
}

#[tauri::command]
async fn desktop_discord_channels<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    source: String,
) -> Result<Value, String> {
    source_jobs::list_discord_channels(&app, &source).await
}

#[tauri::command]
async fn desktop_discord_servers<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    source: String,
) -> Result<Value, String> {
    source_jobs::list_discord_servers(&app, &source).await
}

#[tauri::command]
async fn desktop_slack_workspaces<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    source: String,
) -> Result<Value, String> {
    source_jobs::list_slack_workspaces(&app, &source).await
}

#[tauri::command]
async fn desktop_buzz_communities<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    source: String,
) -> Result<Value, String> {
    source_jobs::list_buzz_communities(&app, &source).await
}

#[tauri::command]
fn desktop_source_initial_sync<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    jobs: State<'_, source_jobs::SourceJobState>,
    source: String,
    budget: source_jobs::InitialSyncBudget,
    operation: source_jobs::InitialSyncOperation,
    plan_id: String,
    approved: bool,
) -> Result<source_jobs::InitialSyncOutcome, String> {
    match operation {
        source_jobs::InitialSyncOperation::Plan => jobs
            .plan_initial_sync(&source, budget)
            .map(source_jobs::InitialSyncOutcome::Plan),
        source_jobs::InitialSyncOperation::Execute => jobs
            .execute_initial_sync(&app, &source, budget, &plan_id, approved)
            .map(source_jobs::InitialSyncOutcome::Job),
    }
}

#[tauri::command]
fn desktop_source_validation_status(
    jobs: State<'_, source_jobs::SourceJobState>,
    id: String,
) -> Result<source_jobs::SourceJobSnapshot, String> {
    jobs.status(&id)
}

#[tauri::command]
fn desktop_source_jobs_status(
    jobs: State<'_, source_jobs::SourceJobState>,
) -> Result<Vec<source_jobs::SourceJobSnapshot>, String> {
    jobs.snapshots()
}

#[tauri::command]
fn desktop_source_validation_cancel(
    jobs: State<'_, source_jobs::SourceJobState>,
    id: String,
) -> Result<source_jobs::SourceJobSnapshot, String> {
    jobs.cancel(&id)
}

#[tauri::command]
async fn desktop_provider_models<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    kind: String,
) -> Result<Value, String> {
    provider_models::list_provider_models(&app, &kind).await
}

#[tauri::command]
async fn desktop_readiness_scan(app: AppHandle) -> readiness::ReadinessSnapshot {
    readiness::scan(&app).await
}

#[tauri::command]
async fn desktop_embedding_generation_migrate(
    app: AppHandle,
    from: String,
    approved: bool,
) -> Result<String, String> {
    if !approved {
        return Err("embedding generation migration requires explicit approval".into());
    }
    readiness::migrate_embedding_generation(&app, &from).await
}

#[tauri::command]
async fn desktop_path_pick(app: AppHandle, kind: String) -> Result<Option<String>, String> {
    paths::pick(app, &kind).await
}

fn validate_retrieval_request(request: &RetrievalRequest) -> Result<(), String> {
    let query = request.query.trim();
    if query.is_empty() {
        return Err("query must not be empty".into());
    }
    if query.len() > MAX_QUERY_LENGTH {
        return Err(format!(
            "query exceeds the {MAX_QUERY_LENGTH} byte desktop safety limit"
        ));
    }
    for (name, value) in [
        ("project", request.project.as_deref()),
        ("source", request.source.as_deref()),
    ] {
        if value.is_some_and(|value| {
            value.is_empty()
                || value.len() > MAX_SCOPE_LENGTH
                || value.chars().any(|character| character.is_control())
        }) {
            return Err(format!("{name} must contain 1 to {MAX_SCOPE_LENGTH} bytes"));
        }
    }
    Ok(())
}

fn validate_document_list_request(request: &DocumentListRequest) -> Result<(), String> {
    if !(1..=100).contains(&request.limit) {
        return Err("document page limit must be between 1 and 100".into());
    }
    for (name, value) in [
        ("project", request.project.as_deref()),
        ("source", request.source.as_deref()),
    ] {
        if value.is_some_and(|value| {
            value.is_empty()
                || value.len() > MAX_SCOPE_LENGTH
                || value.chars().any(|character| character.is_control())
        }) {
            return Err(format!("{name} must contain 1 to {MAX_SCOPE_LENGTH} bytes"));
        }
    }
    if request.query.as_ref().is_some_and(|query| {
        query.len() > MAX_SCOPE_LENGTH
            || query.trim().is_empty()
            || query.chars().any(|character| character.is_control())
    }) {
        return Err(format!("query must contain 1 to {MAX_SCOPE_LENGTH} bytes"));
    }
    if request
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.is_empty() || cursor.len() > MAX_DOCUMENT_CURSOR_LENGTH)
    {
        return Err("invalid document cursor".into());
    }
    Ok(())
}

fn validate_document_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > MAX_DOCUMENT_ID_LENGTH
        || !id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("invalid document id".into());
    }
    Ok(())
}

fn show_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Tray menu dispatch. Extracted from the tray builder so the "show"
/// wiring and the explicit-quit flag handoff are testable over the mock
/// runtime. The "quit" arm needs a real event loop (`app.exit`), so it is
/// exercised only by manual acceptance on a real desktop session.
fn on_tray_menu_event<R: tauri::Runtime>(app: &tauri::AppHandle<R>, event_id: &str) {
    match event_id {
        "show" => show_main_window(app),
        "quit" => {
            QUITTING.store(true, Ordering::SeqCst);
            app.exit(0);
        }
        _ => {}
    }
}

/// Close policy for the tray-resident app: while the tray is running, closing
/// the main window hides it instead of quitting; an explicit tray quit clears
/// the flag so a later close request is allowed through and the app exits.
fn should_hide_main_window_on_close(window_label: &str, quitting: bool) -> bool {
    window_label == MAIN_WINDOW && !quitting
}

fn install_tray(app: &mut tauri::App) -> tauri::Result<TrayStatus> {
    let health = MenuItem::with_id(app, "health", "Runtime: checking", false, None::<&str>)?;
    let corpus = MenuItem::with_id(app, "corpus", "Corpus: checking", false, None::<&str>)?;
    let ingestion =
        MenuItem::with_id(app, "ingestion", "Ingestion: checking", false, None::<&str>)?;
    let source_jobs = MenuItem::with_id(
        app,
        "source-jobs",
        "Source jobs: checking",
        false,
        None::<&str>,
    )?;
    let show = MenuItem::with_id(app, "show", "Show Cortana", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Cortana Desktop", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&health, &corpus, &ingestion, &source_jobs, &show, &quit],
    )?;

    let mut builder = TrayIconBuilder::with_id("cortana")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Cortana second brain")
        .on_menu_event(|app, event| on_tray_menu_event(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;

    Ok(TrayStatus {
        health,
        corpus,
        ingestion,
        source_jobs,
    })
}

fn ingestion_label(status: &Value) -> String {
    let runs = status
        .get("sync_runs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let running = runs
        .iter()
        .filter(|run| run.get("status").and_then(Value::as_str) == Some("running"))
        .count();
    if running > 0 {
        return format!("Ingestion: {running} running");
    }
    let attention = runs
        .iter()
        .filter(|run| {
            matches!(
                run.get("status").and_then(Value::as_str),
                Some("failed" | "cancelled" | "budget_exceeded")
            )
        })
        .count();
    if attention > 0 {
        return format!("Ingestion: {attention} need attention");
    }
    if status
        .get("ingestion")
        .and_then(|value| value.get("scheduled"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "Ingestion: scheduled".into()
    } else {
        "Ingestion: manual".into()
    }
}

async fn refresh_tray(
    backend: &BackendClient,
    jobs: &source_jobs::SourceJobState,
    tray: &TrayStatus,
) {
    let source_label = jobs
        .snapshots()
        .map(|snapshots| source_jobs_label(&snapshots))
        .unwrap_or_else(|_| "Source jobs: unavailable".to_string());
    let _ = tray.source_jobs.set_text(source_label);
    match backend.request(Method::GET, "/v1/status", None).await {
        Ok(status) => {
            let documents = status
                .get("documents")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let chunks = status
                .get("chunks")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let _ = tray.health.set_text("Runtime: online");
            let _ = tray
                .corpus
                .set_text(format!("Corpus: {documents} docs · {chunks} chunks"));
            let _ = tray.ingestion.set_text(ingestion_label(&status));
        }
        Err(_) => {
            let _ = tray.health.set_text("Runtime: offline");
            let _ = tray.corpus.set_text("Corpus: unavailable");
            let _ = tray.ingestion.set_text("Ingestion: unavailable");
        }
    }
}

fn source_jobs_label(snapshots: &[source_jobs::SourceJobSnapshot]) -> String {
    let active = snapshots
        .iter()
        .filter(|job| matches!(job.status, "running" | "cancelling"))
        .count();
    if active > 0 {
        return format!("Source jobs: {active} active");
    }

    // Snapshots are newest first. Count only the latest terminal result for
    // each source so an old failure does not keep the tray in an attention
    // state after a later successful run.
    let mut seen_sources = BTreeSet::new();
    let attention = snapshots
        .iter()
        .filter(|job| seen_sources.insert((job.project.as_str(), job.source.as_str())))
        .filter(|job| matches!(job.status, "failed" | "cancelled" | "budget_exceeded"))
        .count();
    if attention > 0 {
        format!("Source jobs: {attention} need attention")
    } else {
        "Source jobs: idle".to_string()
    }
}

pub fn run() {
    let backend = BackendClient::new().expect("build loopback Cortana client");
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(backend.clone())
        .manage(installer::InstallerState::default())
        .manage(source_jobs::SourceJobState::default())
        .manage(updater::UpdaterState::default())
        .invoke_handler(tauri::generate_handler![
            brain_status,
            brain_answer,
            brain_context,
            brain_documents,
            brain_document,
            brain_graph,
            brain_audit,
            desktop_audit,
            desktop_project_open,
            desktop_secret_file_open,
            desktop_url_open,
            desktop_info,
            desktop_autostart_set,
            desktop_services_status,
            desktop_services_install,
            desktop_services_install_sync,
            desktop_schedule_get,
            desktop_schedule_save,
            desktop_hindsight_status,
            desktop_honcho_status,
            desktop_service_action,
            desktop_services_action_all,
            desktop_database_backup,
            desktop_database_restore,
            desktop_update_status,
            desktop_update_check,
            desktop_update_install,
            desktop_settings_get,
            desktop_settings_save,
            desktop_settings_export,
            desktop_settings_import,
            desktop_path_pick,
            desktop_readiness_scan,
            desktop_embedding_generation_migrate,
            desktop_installer_start,
            desktop_installer_status,
            desktop_installer_cancel,
            desktop_source_validation_start,
            desktop_source_authorization_start,
            desktop_source_trial_sync_start,
            desktop_source_setup_open,
            desktop_github_repositories,
            desktop_discord_channels,
            desktop_provider_models,
            desktop_discord_servers,
            desktop_slack_workspaces,
            desktop_buzz_communities,
            desktop_source_initial_sync,
            desktop_source_validation_status,
            desktop_source_jobs_status,
            desktop_source_validation_cancel
        ])
        .setup(move |app| {
            let tray = install_tray(app)?;
            let backend = backend.clone();
            let jobs = app.state::<source_jobs::SourceJobState>().inner().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    refresh_tray(&backend, &jobs, &tray).await;
                    tokio::time::sleep(Duration::from_secs(15)).await;
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if should_hide_main_window_on_close(window.label(), QUITTING.load(Ordering::SeqCst))
                && let tauri::WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("run Cortana desktop");
}

#[cfg(test)]
mod tests {
    use super::*;

    // Native acceptance suite.
    //
    // These tests drive the production Tauri command handlers through
    // `tauri::test::MockRuntime` IPC dispatch — the same typed command surface
    // the webview uses — with all state confined to temporary config, secret,
    // and data directories (`CORTANA_CONFIG` override). Sidecar-backed tests
    // spawn the real bundled `cortana` CLI and skip with a note when the
    // sidecar has not been prepared (`bun run desktop:test:native` prepares
    // it). No test performs network requests or touches host configuration.
    //
    // Platform-only boundaries that the headless harness deliberately does
    // not fake (they need a real desktop session / OS): native file dialogs
    // (settings import/export, path picking), window/tray GUI creation and
    // close-request events, autostart enable/disable writes, OAuth browser
    // flows, OS service installation, and signed update download/install.

    fn ipc_request(command: &str) -> tauri::webview::InvokeRequest {
        ipc_request_with(command, tauri::ipc::InvokeBody::default())
    }

    fn ipc_request_with(
        command: &str,
        body: tauri::ipc::InvokeBody,
    ) -> tauri::webview::InvokeRequest {
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: if cfg!(any(windows, target_os = "android")) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .expect("mock Tauri URL"),
            body,
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        }
    }

    use serde_json::json;
    use std::{env, ffi::OsString, sync::Mutex};

    /// Process-wide environment mutation is a global side effect in Rust tests.
    ///
    /// The production code resolves the settings root through `settings::default_config_path`,
    /// which reads `CORTANA_CONFIG` when set. That makes path injection feasible only via
    /// this variable in tests. We therefore serialize env reads/writes in-process so the
    /// override stays scoped to this test and can be deterministic for tests running in the
    /// same process; this cannot protect against cross-process test invocation with a shared
    /// process environment.
    static CORTANA_CONFIG_LOCK: Mutex<()> = Mutex::new(());

    struct CortanaConfigScope {
        previous: Option<OsString>,
    }

    impl CortanaConfigScope {
        fn with(path: &Path) -> Self {
            let previous = env::var_os("CORTANA_CONFIG");
            // SAFETY: environment mutation is process-global; guarded by `CORTANA_CONFIG_LOCK`
            // and restored on scope drop.
            unsafe { env::set_var("CORTANA_CONFIG", path) };
            Self { previous }
        }
    }

    impl Drop for CortanaConfigScope {
        fn drop(&mut self) {
            // SAFETY: environment mutation is process-global; guarded by `CORTANA_CONFIG_LOCK`
            // and restored here for this scope.
            match self.previous.clone() {
                Some(previous) => unsafe { env::set_var("CORTANA_CONFIG", previous) },
                None => unsafe { env::remove_var("CORTANA_CONFIG") },
            }
        }
    }

    fn with_cortana_config_override<T, F>(path: &Path, operation: F) -> T
    where
        F: FnOnce() -> T,
    {
        let _lock = CORTANA_CONFIG_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _scope = CortanaConfigScope::with(path);
        operation()
    }

    /// Mock app used by the native acceptance suite.
    ///
    /// Registers the same plugins the production `run()` builder uses where
    /// they are safe headlessly: the shell plugin (so sidecar commands can
    /// spawn the real bundled CLI) and the updater plugin (configured with
    /// empty endpoints so checks fail closed deterministically instead of
    /// hitting the network).
    fn ipc_test_app() -> tauri::App<tauri::test::MockRuntime> {
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().plugins.0.insert(
            "updater".into(),
            serde_json::json!({ "endpoints": [], "pubkey": "" }),
        );
        tauri::test::mock_builder()
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                None,
            ))
            .plugin(tauri_plugin_shell::init())
            .plugin(tauri_plugin_updater::Builder::new().build())
            .manage(updater::UpdaterState::default())
            .manage(source_jobs::SourceJobState::default())
            .invoke_handler(tauri::generate_handler![
                desktop_schedule_get,
                desktop_schedule_save,
                desktop_services_status,
                desktop_info,
                desktop_settings_get,
                desktop_settings_save,
                desktop_source_authorization_start,
                desktop_source_initial_sync,
                desktop_source_setup_open,
                desktop_github_repositories,
                desktop_discord_channels,
                desktop_provider_models,
                desktop_discord_servers,
                desktop_slack_workspaces,
                desktop_buzz_communities,
                desktop_source_jobs_status,
                desktop_source_validation_cancel,
                desktop_source_validation_start,
                desktop_source_validation_status,
                desktop_update_check,
                desktop_update_status
            ])
            .build(context)
            .expect("build mock desktop app")
    }

    fn invoke_json<W>(webview: &W, command: &str) -> Result<Value, Value>
    where
        W: AsRef<tauri::Webview<tauri::test::MockRuntime>>,
    {
        tauri::test::get_ipc_response(webview, ipc_request(command)).map(|body| {
            body.deserialize::<Value>()
                .expect("deserialize IPC response")
        })
    }

    fn invoke_json_with<W>(webview: &W, command: &str, payload: Value) -> Result<Value, Value>
    where
        W: AsRef<tauri::Webview<tauri::test::MockRuntime>>,
    {
        tauri::test::get_ipc_response(webview, ipc_request_with(command, payload.into())).map(
            |body| {
                body.deserialize::<Value>()
                    .expect("deserialize IPC response")
            },
        )
    }

    /// Mirrors the shell plugin's sidecar resolution: next to the test
    /// executable (up one level when cargo placed the test binary in `deps`).
    /// `prepare:sidecar` stores the host binary in the root target directory,
    /// while this standalone Tauri crate may use its own target directory.
    /// Copy it into the path the shell plugin resolves so the acceptance test
    /// exercises the real CLI rather than silently skipping it.
    fn bundled_sidecar_available() -> bool {
        let exe = std::env::current_exe().expect("current test executable");
        let directory = exe.parent().expect("test executable parent");
        let base = if directory.ends_with("deps") {
            directory.parent().unwrap_or(directory)
        } else {
            directory
        };
        let sidecar = if cfg!(windows) {
            base.join("cortana.exe")
        } else {
            base.join("cortana")
        };
        if sidecar.is_file() {
            return true;
        }
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let binaries = manifest.join("binaries");
        let candidate = fs::read_dir(&binaries)
            .ok()
            .into_iter()
            .flat_map(|entries| entries.flatten())
            .map(|entry| entry.path())
            .find(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("cortana-"))
            });
        let Some(candidate) = candidate else {
            return false;
        };
        if let Some(parent) = sidecar.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::copy(candidate, &sidecar).is_ok() && sidecar.is_file()
    }

    /// Bounded polling helper for source job status over IPC. The production
    /// watcher drives the real sidecar on the global async runtime, so the
    /// test thread only observes the typed command surface.
    fn wait_for_terminal_job<W>(webview: &W, id: &str, timeout: std::time::Duration) -> Value
    where
        W: AsRef<tauri::Webview<tauri::test::MockRuntime>>,
    {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let response = invoke_json_with(
                webview,
                "desktop_source_validation_status",
                json!({ "id": id }),
            )
            .expect("source job status IPC");
            let status = response["status"].as_str().expect("job status string");
            if !matches!(status, "running" | "cancelling") {
                return response;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "source job `{id}` did not finish within {timeout:?}: {response}"
            );
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }

    /// Temp config + data dir + filesystem source root for acceptance tests.
    /// All side effects stay inside the temp directory; the CLI sidecar reads
    /// the same file through the inherited `CORTANA_CONFIG` override.
    struct NativeFixture {
        _temp: tempfile::TempDir,
        config: PathBuf,
        data_dir: PathBuf,
        root: PathBuf,
    }

    fn toml_string(value: &str) -> String {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }

    fn filesystem_fixture(source_name: &str, enabled: bool) -> NativeFixture {
        let temp = tempfile::tempdir().expect("temporary fixture directory");
        let config_dir = temp.path().join("cortana");
        fs::create_dir_all(&config_dir).expect("fixture config directory");
        let config = config_dir.join("config.toml");
        let data_dir = config_dir.join("data");
        let root = config_dir.join("notes");
        fs::create_dir_all(&root).expect("fixture source root");
        fs::write(
            &config,
            format!(
                "data_dir = {}\n\n[query]\napi_key_env = \"CORTANA_TEST_QUERY_API_KEY\"\n\n[[sources]]\nname = {}\nkind = \"filesystem\"\nenabled = {}\nproject = \"work\"\nroot = {}\n",
                toml_string(&data_dir.display().to_string()),
                toml_string(source_name),
                enabled,
                toml_string(&root.display().to_string()),
            ),
        )
        .expect("fixture config");
        NativeFixture {
            _temp: temp,
            config,
            data_dir,
            root,
        }
    }

    #[test]
    fn native_ipc_dispatches_source_job_status() {
        let app = ipc_test_app();
        let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
            .build()
            .expect("build mock desktop window");

        let response = invoke_json(&window, "desktop_source_jobs_status").expect("IPC response");
        assert_eq!(response, Value::Array(Vec::new()));
    }

    #[test]
    fn native_ipc_dispatches_default_schedule_when_missing() {
        let temp = tempfile::tempdir().expect("temporary config directory");
        let config_path = temp.path().join("cortana/config.toml");

        with_cortana_config_override(&config_path, || {
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let response = invoke_json(&window, "desktop_schedule_get").expect("IPC response");
            assert_eq!(response["sync_interval_seconds"], 900);
            assert_eq!(response["backup_interval_seconds"], 86400);
        });
    }

    #[test]
    fn native_ipc_dispatches_idle_update_status_without_restart() {
        let app = ipc_test_app();
        let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
            .build()
            .expect("build mock desktop window");

        let response = invoke_json(&window, "desktop_update_status").expect("IPC response");
        assert_eq!(response["phase"], "idle");
        assert_eq!(response["restart_required"], false);
    }

    #[test]
    fn native_desktop_info_reports_autostart_without_mutating_host_state() {
        let app = ipc_test_app();
        let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
            .build()
            .expect("build mock desktop window");

        let info = invoke_json(&window, "desktop_info").expect("desktop info IPC");
        assert_eq!(info["platform"], std::env::consts::OS);
        assert!(
            info["desktop_version"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert!(info["autostart_enabled"].is_boolean());
    }

    #[test]
    fn native_google_authorization_and_setup_fail_closed_before_browser_launch() {
        let temp = tempfile::tempdir().expect("temporary config directory");
        let config_path = temp.path().join("cortana/config.toml");
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::create_dir_all(temp.path().join("cortana/notes")).expect("notes directory");
        fs::write(
            &config_path,
            format!(
                "data_dir = {}\n\n[[sources]]\nname = \"work-drive\"\nkind = \"google-drive\"\nenabled = true\nproject = \"work\"\n\n[[sources]]\nname = \"work-notes\"\nkind = \"filesystem\"\nenabled = true\nproject = \"work\"\nroot = {}\n",
                toml_string(&temp.path().join("cortana/data").display().to_string()),
                toml_string(&temp.path().join("cortana/notes").display().to_string()),
            ),
        )
        .expect("test config");

        with_cortana_config_override(&config_path, || {
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let authorization = invoke_json_with(
                &window,
                "desktop_source_authorization_start",
                json!({ "source": "work-drive" }),
            )
            .expect_err("OAuth must reject incomplete setup without opening a browser");
            assert!(
                authorization
                    .as_str()
                    .unwrap_or_default()
                    .contains("save a Google token destination")
            );

            let setup = invoke_json_with(
                &window,
                "desktop_source_setup_open",
                json!({ "source": "work-notes" }),
            )
            .expect_err("filesystem setup must fail closed without opening a browser");
            assert!(
                setup
                    .as_str()
                    .unwrap_or_default()
                    .contains("does not have a browser-based account setup page")
            );
        });
    }

    #[test]
    fn native_discord_channel_discovery_fails_closed_for_other_kinds() {
        let fixture = filesystem_fixture("work-notes", true);
        with_cortana_config_override(&fixture.config, || {
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let error = invoke_json_with(
                &window,
                "desktop_discord_channels",
                json!({ "source": "work-notes" }),
            )
            .expect_err("non-Discord sources must fail closed before spawning the sidecar");
            assert!(
                error
                    .as_str()
                    .unwrap_or_default()
                    .contains("only for Discord sources")
            );
        });
    }

    #[test]
    fn native_discord_channel_discovery_fails_closed_without_rpc_credentials() {
        let temp = tempfile::tempdir().expect("temporary config directory");
        let config_path = temp.path().join("cortana/config.toml");
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::write(
            &config_path,
            format!(
                "data_dir = {}\n\n[[sources]]\nname = \"community\"\nkind = \"discord\"\nenabled = true\nproject = \"work\"\ntoken = {:?}\noauth_client = {:?}\n",
                toml_string(&temp.path().join("cortana/data").display().to_string()),
                temp.path().join("cortana/discord-token.json"),
                temp.path().join("cortana/discord-client.json"),
            ),
        )
        .expect("test config");

        with_cortana_config_override(&config_path, || {
            if !bundled_sidecar_available() {
                eprintln!(
                    "SKIP: bundled `cortana` sidecar is missing next to the test executable; \
                     run `bun run desktop:test:native` to prepare it"
                );
                return;
            }
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            // Discovery must fail before any network request when Discord
            // Desktop RPC credentials are not available.
            let error = invoke_json_with(
                &window,
                "desktop_discord_channels",
                json!({ "source": "community" }),
            )
            .expect_err("missing RPC credentials must fail closed without network access");
            let message = error.as_str().unwrap_or_default();
            assert!(
                message.contains("Discord"),
                "unexpected discovery error: {message}"
            );
            assert!(!message.contains("DISCORD_LEGACY_TOKEN_ENV"));
        });
    }

    #[test]
    fn native_discord_server_discovery_fails_closed_for_other_kinds() {
        let fixture = filesystem_fixture("work-notes", true);
        with_cortana_config_override(&fixture.config, || {
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let error = invoke_json_with(
                &window,
                "desktop_discord_servers",
                json!({ "source": "work-notes" }),
            )
            .expect_err("non-Discord sources must fail closed before spawning the sidecar");
            assert!(
                error
                    .as_str()
                    .unwrap_or_default()
                    .contains("only for Discord sources")
            );
        });
    }

    #[test]
    fn native_discord_server_discovery_fails_closed_without_authorization() {
        let temp = tempfile::tempdir().expect("temporary config directory");
        let config_path = temp.path().join("cortana/config.toml");
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::write(
            &config_path,
            format!(
                "data_dir = {}\n\n[[sources]]\nname = \"community\"\nkind = \"discord\"\nenabled = true\nproject = \"work\"\ntoken = \"/tmp/cortana-test/missing-discord-token.json\"\noauth_client = \"/tmp/cortana-test/missing-oauth-client.json\"\n",
                toml_string(&temp.path().join("cortana/data").display().to_string()),
            ),
        )
        .expect("test config");

        with_cortana_config_override(&config_path, || {
            if !bundled_sidecar_available() {
                eprintln!(
                    "SKIP: bundled `cortana` sidecar is missing next to the test executable; \
                     run `bun run desktop:test:native` to prepare it"
                );
                return;
            }
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            // Discovery must fail before any network request when no user
            // token has been stored, and the error must point at browser
            // authorization without ever containing a credential value.
            let error = invoke_json_with(
                &window,
                "desktop_discord_servers",
                json!({ "source": "community" }),
            )
            .expect_err("missing user token must fail closed without network access");
            let message = error.as_str().unwrap_or_default();
            assert!(
                message.contains("Discord Desktop RPC") || message.contains("Discord OAuth"),
                "unexpected server discovery error: {message}"
            );
        });
    }

    #[test]
    fn native_discord_authorization_fails_closed_without_oauth_paths() {
        let temp = tempfile::tempdir().expect("temporary config directory");
        let config_path = temp.path().join("cortana/config.toml");
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::write(
            &config_path,
            format!(
                "data_dir = {}\n\n[[sources]]\nname = \"community\"\nkind = \"discord\"\nenabled = true\nproject = \"work\"\n",
                toml_string(&temp.path().join("cortana/data").display().to_string()),
            ),
        )
        .expect("test config");

        with_cortana_config_override(&config_path, || {
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let error = invoke_json_with(
                &window,
                "desktop_source_authorization_start",
                json!({ "source": "community" }),
            )
            .expect_err("Discord RPC must reject incomplete setup before starting");
            assert!(
                error.as_str().unwrap_or_default().contains("Discord"),
                "unexpected Discord authorization error: {}",
                error.as_str().unwrap_or_default()
            );
        });
    }

    #[test]
    fn native_slack_workspace_discovery_fails_closed_for_other_kinds() {
        let fixture = filesystem_fixture("work-notes", true);
        with_cortana_config_override(&fixture.config, || {
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let error = invoke_json_with(
                &window,
                "desktop_slack_workspaces",
                json!({ "source": "work-notes" }),
            )
            .expect_err("non-Slack sources must fail closed before spawning the sidecar");
            assert!(
                error
                    .as_str()
                    .unwrap_or_default()
                    .contains("only for Slack sources")
            );
        });
    }

    #[test]
    fn native_buzz_community_discovery_fails_closed_for_other_kinds() {
        let fixture = filesystem_fixture("work-notes", true);
        with_cortana_config_override(&fixture.config, || {
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let error = invoke_json_with(
                &window,
                "desktop_buzz_communities",
                json!({ "source": "work-notes" }),
            )
            .expect_err("non-Buzz sources must fail closed before spawning the sidecar");
            assert!(
                error
                    .as_str()
                    .unwrap_or_default()
                    .contains("only for Buzz sources")
            );
        });
    }

    #[test]
    fn native_buzz_community_discovery_reads_the_real_identity_file() {
        let temp = tempfile::tempdir().expect("temporary config directory");
        let config_path = temp.path().join("cortana/config.toml");
        let root = temp.path().join("buzz-root");
        fs::create_dir_all(root.join("agents")).expect("agents directory");
        fs::write(
            root.join("agents/teams.json"),
            r#"[{"id": "builtin-team:welcome", "name": "Welcome Team"}]"#,
        )
        .expect("write identity file");
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::write(
            &config_path,
            format!(
                "data_dir = {}\n\n[[sources]]\nname = \"agent-buzz\"\nkind = \"buzz\"\nenabled = true\nproject = \"agents\"\nroot = {}\n",
                toml_string(&temp.path().join("cortana/data").display().to_string()),
                toml_string(&root.display().to_string()),
            ),
        )
        .expect("test config");

        with_cortana_config_override(&config_path, || {
            if !bundled_sidecar_available() {
                eprintln!(
                    "SKIP: bundled `cortana` sidecar is missing next to the test executable; \
                     run `bun run desktop:test:native` to prepare it"
                );
                return;
            }
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let response = invoke_json_with(
                &window,
                "desktop_buzz_communities",
                json!({ "source": "agent-buzz" }),
            )
            .expect("discover communities");
            assert_eq!(
                response["communities"][0]["id"],
                serde_json::json!("builtin-team:welcome")
            );
            assert_eq!(response["truncated"], serde_json::json!(false));
        });
    }

    #[test]
    fn native_slack_workspace_discovery_fails_closed_without_authorization() {
        let temp = tempfile::tempdir().expect("temporary config directory");
        let config_path = temp.path().join("cortana/config.toml");
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::write(
            &config_path,
            format!(
                "data_dir = {}\n\n[[sources]]\nname = \"team-slack\"\nkind = \"slack\"\nenabled = true\nproject = \"work\"\nchannels = [\"C0123456789\"]\ntoken_env = \"CORTANA_TEST_SLACK_BOT_TOKEN\"\ntoken = \"/tmp/cortana-test/missing-slack-token.json\"\noauth_client = \"/tmp/cortana-test/missing-oauth-client.json\"\n",
                toml_string(&temp.path().join("cortana/data").display().to_string()),
            ),
        )
        .expect("test config");

        with_cortana_config_override(&config_path, || {
            if !bundled_sidecar_available() {
                eprintln!(
                    "SKIP: bundled `cortana` sidecar is missing next to the test executable; \
                     run `bun run desktop:test:native` to prepare it"
                );
                return;
            }
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            // Discovery must fail before any network request when no user
            // token has been stored, and the error must point at browser
            // authorization without ever containing a credential value or
            // treating the bot token environment variable as a path.
            let error = invoke_json_with(
                &window,
                "desktop_slack_workspaces",
                json!({ "source": "team-slack" }),
            )
            .expect_err("missing user token must fail closed without network access");
            let message = error.as_str().unwrap_or_default();
            assert!(
                message.contains("check browser authorization")
                    || message.contains("requires browser authorization"),
                "unexpected workspace discovery error: {message}"
            );
            assert!(!message.contains("CORTANA_TEST_SLACK_BOT_TOKEN"));
        });
    }

    #[test]
    fn native_slack_authorization_fails_closed_without_oauth_paths() {
        let temp = tempfile::tempdir().expect("temporary config directory");
        let config_path = temp.path().join("cortana/config.toml");
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::write(
            &config_path,
            format!(
                "data_dir = {}\n\n[[sources]]\nname = \"team-slack\"\nkind = \"slack\"\nenabled = true\nproject = \"work\"\nchannels = [\"C0123456789\"]\ntoken_env = \"CORTANA_TEST_SLACK_BOT_TOKEN\"\n",
                toml_string(&temp.path().join("cortana/data").display().to_string()),
            ),
        )
        .expect("test config");

        with_cortana_config_override(&config_path, || {
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let error = invoke_json_with(
                &window,
                "desktop_source_authorization_start",
                json!({ "source": "team-slack" }),
            )
            .expect_err("Slack OAuth must reject incomplete setup without a browser");
            assert!(
                error
                    .as_str()
                    .unwrap_or_default()
                    .contains("save a Slack user token destination file")
            );
        });
    }

    #[test]
    fn native_ipc_dispatches_redacted_settings_snapshot() {
        let temp = tempfile::tempdir().expect("temporary config directory");
        let config_path = temp.path().join("cortana/config.toml");
        let secret_file_path = temp.path().join("cortana/secrets.env");
        fs::create_dir_all(config_path.parent().expect("config parent")).expect("config directory");
        fs::write(
            &config_path,
            r##"
            [query]
            api_key_env = "CORTANA_TEST_QUERY_API_KEY"

            [[sources]]
            name = "notes"
            kind = "filesystem"
            enabled = true
            project = "work"
            root = "/tmp/cortana-test-notes"
            token_env = "CORTANA_TEST_SOURCE_TOKEN"
            "##,
        )
        .expect("test config");
        let raw_query_secret = "raw-query-api-key-7f3a9c1e";
        let raw_source_secret = "raw-source-token-4b8d2f6a";
        fs::write(
            &secret_file_path,
            format!(
                "CORTANA_TEST_QUERY_API_KEY={raw_query_secret}\n\
                 CORTANA_TEST_SOURCE_TOKEN={raw_source_secret}\n"
            ),
        )
        .expect("test secrets");

        with_cortana_config_override(&config_path, || {
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let response = invoke_json(&window, "desktop_settings_get").expect("IPC response");
            let object = response.as_object().expect("settings snapshot object");
            for key in [
                "config_path",
                "secret_file_path",
                "secret_file_managed",
                "needs_setup",
                "restart_required",
                "workspaces",
                "sources",
                "auth_principals",
                "embedding",
                "query",
                "hindsight",
                "honcho",
                "ingestion",
                "runtime",
                "secrets",
            ] {
                assert!(object.contains_key(key), "snapshot must contain `{key}`");
            }

            assert_eq!(
                response["config_path"].as_str(),
                Some(config_path.display().to_string().as_str())
            );
            assert_eq!(
                response["secret_file_path"].as_str(),
                Some(secret_file_path.display().to_string().as_str())
            );
            assert_eq!(response["secret_file_managed"], Value::Bool(true));
            assert_eq!(response["needs_setup"], Value::Bool(false));

            let secrets = response["secrets"].as_array().expect("secrets array");
            let names = secrets
                .iter()
                .map(|secret| secret["name"].as_str().expect("secret name"))
                .collect::<Vec<_>>();
            assert_eq!(
                names,
                vec!["CORTANA_TEST_QUERY_API_KEY", "CORTANA_TEST_SOURCE_TOKEN"]
            );
            for secret in secrets {
                let metadata = secret.as_object().expect("secret metadata object");
                let mut keys = metadata.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                assert_eq!(keys, vec!["configured", "name", "source"]);
                assert_eq!(secret["configured"], Value::Bool(true));
                assert_eq!(secret["source"], Value::String("secret-file".into()));
            }

            let serialized = serde_json::to_string(&response).expect("serialize snapshot");
            for raw in [raw_query_secret, raw_source_secret] {
                assert!(
                    !serialized.contains(raw),
                    "raw secret values must never cross the IPC boundary"
                );
            }
        });
    }

    fn principal(scopes: &[&str]) -> settings::AuthPrincipalSettings {
        settings::AuthPrincipalSettings {
            principal: "owner".into(),
            token_env: "OWNER_TOKEN".into(),
            scopes: scopes.iter().map(|scope| (*scope).into()).collect(),
            acl: Vec::new(),
        }
    }

    #[test]
    fn owner_preference_requires_the_requested_scope_too() {
        assert!(!principal_supports_owner_scope(
            &principal(&["admin"]),
            "query"
        ));
        assert!(principal_supports_owner_scope(
            &principal(&["admin", "query"]),
            "query"
        ));
        assert!(!principal_supports_owner_scope(
            &principal(&["admin", "status"]),
            "query"
        ));
    }

    #[test]
    fn retrieval_request_enforces_bounded_non_empty_inputs() {
        let valid = RetrievalRequest {
            query: "bounded ingestion".into(),
            project: Some("work".into()),
            source: None,
        };
        assert!(validate_retrieval_request(&valid).is_ok());

        let empty = RetrievalRequest {
            query: "   ".into(),
            project: None,
            source: None,
        };
        assert_eq!(
            validate_retrieval_request(&empty).unwrap_err(),
            "query must not be empty"
        );

        let oversized = RetrievalRequest {
            query: "q".repeat(MAX_QUERY_LENGTH + 1),
            project: None,
            source: None,
        };
        assert!(
            validate_retrieval_request(&oversized)
                .unwrap_err()
                .contains("desktop safety limit")
        );
    }

    #[test]
    fn document_requests_enforce_fixed_bounded_identifiers() {
        let valid = DocumentListRequest {
            project: Some("work".into()),
            source: None,
            query: Some("release".into()),
            cursor: Some("opaque-cursor".into()),
            limit: 50,
        };
        assert!(validate_document_list_request(&valid).is_ok());
        assert!(
            validate_document_list_request(&DocumentListRequest { limit: 0, ..valid }).is_err()
        );
        let invalid_query = DocumentListRequest {
            project: None,
            source: None,
            query: Some("x".repeat(MAX_SCOPE_LENGTH + 1)),
            cursor: None,
            limit: 50,
        };
        assert!(validate_document_list_request(&invalid_query).is_err());
        let padded_query = DocumentListRequest {
            project: None,
            source: None,
            query: Some(format!(" {} ", "x".repeat(MAX_SCOPE_LENGTH))),
            cursor: None,
            limit: 50,
        };
        assert!(validate_document_list_request(&padded_query).is_err());
        let control_query = DocumentListRequest {
            project: None,
            source: None,
            query: Some("\u{0000}".into()),
            cursor: None,
            limit: 50,
        };
        assert!(validate_document_list_request(&control_query).is_err());
        let whitespace_query = DocumentListRequest {
            project: None,
            source: None,
            query: Some("   ".into()),
            cursor: None,
            limit: 50,
        };
        assert!(validate_document_list_request(&whitespace_query).is_err());
        let invalid_scope = DocumentListRequest {
            project: Some("work\u{0000}personal".into()),
            source: None,
            query: None,
            cursor: None,
            limit: 50,
        };
        assert!(validate_document_list_request(&invalid_scope).is_err());
        assert!(validate_document_id(&"a".repeat(64)).is_ok());
        assert!(validate_document_id("../store.sqlite3").is_err());
    }

    #[test]
    fn validates_external_url_schemes_for_open_bridge() {
        assert!(validate_external_url("https://example.com").is_ok());
        assert!(validate_external_url("http://127.0.0.1").is_ok());
        assert!(validate_external_url("https://user:password@example.com").is_err());
        assert!(validate_external_url("mailto:help@example.com").is_ok());
        assert!(validate_external_url("mailto://user:password@example.com").is_err());
        assert!(
            validate_external_url("slack://channel?team=&id=C123ABC&message=1712345678.000100")
                .is_ok()
        );
        assert!(validate_external_url("slack://channel?id=C123ABC").is_err());
        assert!(validate_external_url("slack://channel?id=C123ABC&message=1&message=2").is_err());
        assert!(
            validate_external_url(
                "slack://channel?id=C123ABC&message=1&redirect=https://evil.example"
            )
            .is_err()
        );
        assert!(validate_external_url("slack://channel?id=C123ABC&message=.").is_err());
        assert!(
            validate_external_url("notes://showNote?identifier=x-coredata%3A%2F%2Fnote-1").is_ok()
        );
        assert!(validate_external_url("notes://showNote?identifier=").is_err());
        assert!(validate_external_url("notes://showNote?identifier=x&extra=1").is_err());
        assert!(validate_external_url("buzz://persona/npub123/session%3A1").is_ok());
        assert!(validate_external_url("buzz://persona/npub123/session/extra").is_err());
        assert!(validate_external_url("buzz://persona/npub123/").is_err());
        assert!(validate_external_url("buzz://persona/npub%2F123/session").is_err());
        assert!(validate_external_url("buzz://persona/npub%00/session").is_err());
        assert!(validate_external_url("file:///tmp/cv.pdf").is_ok());
        assert!(validate_external_url("file://user@localhost/tmp/cv.pdf").is_err());
        assert!(validate_external_url("file://remote.example/cv.pdf").is_err());
        assert!(validate_external_url("file:///tmp/cv.pdf?download=1").is_err());
        assert!(validate_external_url("ftp://example.com").is_err());
        assert!(validate_external_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn local_file_targets_are_contained_by_configured_roots() {
        let temp = tempfile::tempdir().expect("temp directory");
        let root = temp.path().join("source");
        let outside = temp.path().join("outside.txt");
        std::fs::create_dir_all(&root).expect("source root");
        let inside = root.join("note.md");
        std::fs::write(&inside, "note").expect("inside file");
        std::fs::write(&outside, "private").expect("outside file");
        let roots = vec![fs::canonicalize(&root).expect("canonical root")];
        assert!(is_within_filesystem_root(
            &fs::canonicalize(&inside).expect("canonical inside"),
            &roots
        ));
        assert!(!is_within_filesystem_root(
            &fs::canonicalize(&outside).expect("canonical outside"),
            &roots
        ));
    }

    #[test]
    fn tray_ingestion_label_reports_bounded_operational_state() {
        let running = serde_json::json!({
            "sync_runs": [{"status": "running"}],
            "ingestion": {"scheduled": false}
        });
        assert_eq!(ingestion_label(&running), "Ingestion: 1 running");

        let attention = serde_json::json!({
            "sync_runs": [{"status": "budget_exceeded"}, {"status": "failed"}],
            "ingestion": {"scheduled": true}
        });
        assert_eq!(ingestion_label(&attention), "Ingestion: 2 need attention");

        assert_eq!(
            ingestion_label(
                &serde_json::json!({"sync_runs": [], "ingestion": {"scheduled": true}})
            ),
            "Ingestion: scheduled"
        );
        assert_eq!(ingestion_label(&serde_json::json!({})), "Ingestion: manual");
    }

    #[test]
    fn native_settings_save_redacts_secrets_and_persists_workspace_sources() {
        let fixture = filesystem_fixture("work-notes", true);
        let raw_query_secret = "raw-query-api-key-acceptance-91d2";

        with_cortana_config_override(&fixture.config, || {
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let response = invoke_json(&window, "desktop_settings_get").expect("settings get IPC");
            let mut update = response.as_object().expect("snapshot object").clone();
            for key in [
                "config_path",
                "secret_file_path",
                "secret_file_managed",
                "embedding_service_program",
                "needs_setup",
                "restart_required",
                "secrets",
            ] {
                update.remove(key);
            }
            update.insert(
                "workspaces".into(),
                json!([{"id": "work", "name": "Engineering", "account_label": null, "color": null}]),
            );
            update.insert(
                "sources".into(),
                json!([{
                    "name": "work-notes",
                    "kind": "filesystem",
                    "enabled": true,
                    "project": "work",
                    "root": fixture.root.display().to_string(),
                    "source": null,
                    "channels": [],
                    "token_env": "CORTANA_TEST_SOURCE_TOKEN",
                    "token_path": null,
                    "oauth_client_path": null,
                    "query": null,
                    "labels": [],
                    "max_content_chars": null,
                    "max_documents": null,
                    "max_bytes": null,
                    "max_duration_seconds": null,
                    "exclude": [],
                    "acl": ["work"],
                    "editable": true
                }]),
            );
            update.insert(
                "secrets".into(),
                json!([{"name": "CORTANA_TEST_QUERY_API_KEY", "value": raw_query_secret, "clear": false}]),
            );

            let saved = invoke_json_with(
                &window,
                "desktop_settings_save",
                json!({ "update": update }),
            )
            .expect("settings save IPC");
            assert_eq!(saved["restart_required"], Value::Bool(true));
            assert_eq!(saved["needs_setup"], Value::Bool(false));
            assert_eq!(saved["workspaces"][0]["name"], "Engineering");
            assert!(
                saved["sources"]
                    .as_array()
                    .expect("sources array")
                    .iter()
                    .any(|source| source["name"] == "work-notes")
            );
            let secrets = saved["secrets"].as_array().expect("secrets array");
            assert!(secrets.iter().any(|secret| {
                secret["name"] == "CORTANA_TEST_QUERY_API_KEY"
                    && secret["configured"] == Value::Bool(true)
            }));
            let serialized = serde_json::to_string(&saved).expect("serialize save response");
            assert!(
                !serialized.contains(raw_query_secret),
                "raw secret values must never cross the IPC boundary"
            );

            // The same typed command surface returns the persisted values, and the
            // write-only secret landed in the managed secret file next to the config.
            let reloaded = invoke_json(&window, "desktop_settings_get").expect("settings get IPC");
            assert_eq!(reloaded["workspaces"][0]["name"], "Engineering");
            assert!(
                reloaded["sources"]
                    .as_array()
                    .expect("sources array")
                    .iter()
                    .any(|source| source["name"] == "work-notes"
                        && source["enabled"] == Value::Bool(true))
            );
            let reloaded_serialized = serde_json::to_string(&reloaded).expect("serialize reload");
            assert!(!reloaded_serialized.contains(raw_query_secret));
            let secrets_file = fixture
                .config
                .parent()
                .expect("config parent")
                .join("secrets.env");
            let persisted = fs::read_to_string(&secrets_file).expect("read managed secrets");
            assert!(persisted.contains(raw_query_secret));
            let config_body = fs::read_to_string(&fixture.config).expect("read saved config");
            assert!(config_body.contains("Engineering"));
            assert!(config_body.contains("work-notes"));
        });
    }

    #[test]
    fn native_schedule_roundtrip_persists_and_rejects_bad_intervals() {
        let fixture = filesystem_fixture("work-notes", true);
        with_cortana_config_override(&fixture.config, || {
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let saved = invoke_json_with(
                &window,
                "desktop_schedule_save",
                json!({ "schedule": { "sync_interval_seconds": 1800, "backup_interval_seconds": 172800 } }),
            )
            .expect("schedule save IPC");
            assert_eq!(saved["sync_interval_seconds"], 1800);
            assert_eq!(saved["backup_interval_seconds"], 172800);
            assert!(
                fixture
                    .config
                    .parent()
                    .expect("config parent")
                    .join("service-schedule.toml")
                    .is_file()
            );

            let loaded = invoke_json(&window, "desktop_schedule_get").expect("schedule get IPC");
            assert_eq!(loaded["sync_interval_seconds"], 1800);
            assert_eq!(loaded["backup_interval_seconds"], 172800);

            let error = invoke_json_with(
                &window,
                "desktop_schedule_save",
                json!({ "schedule": { "sync_interval_seconds": 30, "backup_interval_seconds": 172800 } }),
            )
            .expect_err("too-aggressive interval must be rejected");
            assert!(
                error
                    .as_str()
                    .unwrap_or_default()
                    .contains("between 60 and")
            );

            let unchanged = invoke_json(&window, "desktop_schedule_get").expect("schedule get IPC");
            assert_eq!(unchanged["sync_interval_seconds"], 1800);
        });
    }

    #[test]
    fn native_updater_check_fails_closed_without_network_endpoints() {
        let app = ipc_test_app();
        let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
            .build()
            .expect("build mock desktop window");

        let error = invoke_json(&window, "desktop_update_check")
            .expect_err("update check must fail closed");
        assert!(
            error
                .as_str()
                .unwrap_or_default()
                .contains("does not have any endpoints"),
            "unexpected update check error: {error:?}"
        );
        let status = invoke_json(&window, "desktop_update_status").expect("update status IPC");
        assert_eq!(status["phase"], "failed");
        assert!(
            status["error"]
                .as_str()
                .unwrap_or_default()
                .contains("does not have any endpoints"),
            "unexpected update status error: {status}"
        );
    }

    #[test]
    fn native_services_status_reports_real_platform_state() {
        let fixture = filesystem_fixture("work-notes", true);
        with_cortana_config_override(&fixture.config, || {
            if !bundled_sidecar_available() {
                eprintln!(
                    "SKIP: bundled `cortana` sidecar is missing next to the test executable; \
                     run `bun run desktop:test:native` to prepare it"
                );
                return;
            }
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let report =
                invoke_json(&window, "desktop_services_status").expect("service status IPC");
            assert_eq!(report["platform"], std::env::consts::OS);
            assert!(report["supported"].is_boolean());
            let services = report["services"].as_array().expect("services array");
            let names = services
                .iter()
                .map(|service| service["name"].as_str().expect("service name"))
                .collect::<Vec<_>>();
            assert_eq!(names, vec!["embedding", "server", "sync", "backup"]);
            for service in services {
                for key in [
                    "label",
                    "installed",
                    "loaded",
                    "state",
                    "pid",
                    "last_exit_status",
                ] {
                    assert!(
                        service.get(key).is_some(),
                        "service report must carry `{key}`"
                    );
                }
            }
        });
    }

    #[test]
    fn native_source_validation_lifecycle_runs_the_real_sidecar() {
        let fixture = filesystem_fixture("work-notes", true);
        fs::write(
            fixture.root.join("note-1.md"),
            "bounded acceptance note one",
        )
        .expect("fixture note");
        fs::write(
            fixture.root.join("note-2.md"),
            "bounded acceptance note two",
        )
        .expect("fixture note");

        with_cortana_config_override(&fixture.config, || {
            if !bundled_sidecar_available() {
                eprintln!(
                    "SKIP: bundled `cortana` sidecar is missing next to the test executable; \
                     run `bun run desktop:test:native` to prepare it"
                );
                return;
            }
            let app = ipc_test_app();
            let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
                .build()
                .expect("build mock desktop window");

            let started = invoke_json_with(
                &window,
                "desktop_source_validation_start",
                json!({ "source": "work-notes", "budget": "small" }),
            )
            .expect("start validation IPC");
            assert_eq!(started["operation"], "validation");
            assert_eq!(started["status"], "running");
            assert_eq!(started["writes_indexed_data"], Value::Bool(false));
            let id = started["id"].as_str().expect("job id").to_string();

            let terminal = wait_for_terminal_job(&window, &id, std::time::Duration::from_secs(120));
            assert_eq!(
                terminal["status"], "succeeded",
                "job log: {}",
                terminal["log"]
            );
            assert_eq!(terminal["exit_code"], 0);
            assert!(!terminal["completed_at_unix_seconds"].is_null());

            let state_path = fixture.data_dir.join("source-validations.json");
            let state: Value = serde_json::from_str(
                &fs::read_to_string(&state_path).expect("read validation state"),
            )
            .expect("parse validation state");
            let record = &state["sources"]["work-notes"];
            assert_eq!(record["status"], "succeeded");
            assert_eq!(record["complete"], Value::Bool(true));
            assert!(record["max_documents"].as_u64().expect("max documents") >= 100);

            let cancelled = invoke_json_with(
                &window,
                "desktop_source_validation_cancel",
                json!({ "id": id }),
            )
            .expect("cancel finished job IPC");
            assert_eq!(cancelled["status"], "succeeded");

            let missing = invoke_json_with(
                &window,
                "desktop_source_validation_cancel",
                json!({ "id": "source-0-0" }),
            )
            .expect_err("cancel of an unknown job must fail");
            assert!(missing.as_str().unwrap_or_default().contains("not found"));

            let jobs = invoke_json(&window, "desktop_source_jobs_status").expect("jobs IPC");
            assert_eq!(jobs[0]["id"], id.as_str());

            let plan = invoke_json_with(
                &window,
                "desktop_source_initial_sync",
                json!({ "source": "work-notes", "budget": "small", "operation": "plan", "planId": "", "approved": false }),
            )
            .expect("initial sync plan IPC");
            assert_eq!(plan["outcome"], "plan");
            assert_eq!(plan["budget"], "small");
            assert_eq!(plan["requires_validation"], Value::Bool(true));
            assert_eq!(plan["validation_covers_budget"], Value::Bool(true));
            assert_eq!(plan["validation_complete"], Value::Bool(true));
            let plan_id = plan["plan_id"].as_str().expect("plan id").to_string();

            let unapproved = invoke_json_with(
                &window,
                "desktop_source_initial_sync",
                json!({ "source": "work-notes", "budget": "small", "operation": "execute", "planId": plan_id, "approved": false }),
            )
            .expect_err("unapproved execution must fail");
            assert!(
                unapproved
                    .as_str()
                    .unwrap_or_default()
                    .contains("explicit plan confirmation")
            );

            let uncovered = invoke_json_with(
                &window,
                "desktop_source_initial_sync",
                json!({ "source": "work-notes", "budget": "medium", "operation": "execute", "planId": plan_id, "approved": true }),
            )
            .expect_err("execution beyond validated limits must fail");
            assert!(
                uncovered
                    .as_str()
                    .unwrap_or_default()
                    .contains("equal or larger limits")
            );
        });
    }

    #[test]
    fn native_close_policy_hides_the_main_window_unless_quitting() {
        QUITTING.store(false, Ordering::SeqCst);
        assert!(should_hide_main_window_on_close(MAIN_WINDOW, false));
        assert!(!should_hide_main_window_on_close(MAIN_WINDOW, true));
        assert!(!should_hide_main_window_on_close("settings", false));
        QUITTING.store(true, Ordering::SeqCst);
        assert!(!should_hide_main_window_on_close(
            MAIN_WINDOW,
            QUITTING.load(Ordering::SeqCst)
        ));
        QUITTING.store(false, Ordering::SeqCst);
    }

    #[test]
    fn native_tray_show_wiring_uses_the_real_window_dispatcher() {
        let app = ipc_test_app();
        let window = tauri::WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
            .build()
            .expect("build mock desktop window");

        on_tray_menu_event(app.handle(), "show");
        assert!(app.get_webview_window(MAIN_WINDOW).is_some());
        assert!(window.is_visible().unwrap_or(false));

        on_tray_menu_event(app.handle(), "unknown-menu-item");
        assert!(app.get_webview_window(MAIN_WINDOW).is_some());
    }

    #[test]
    fn tray_source_job_label_uses_latest_terminal_result_per_source() {
        let snapshot =
            |source: &str, project: &str, status: &'static str| source_jobs::SourceJobSnapshot {
                id: format!("source-1-{}", source.len()),
                operation: "validation",
                source: source.into(),
                kind: "filesystem".into(),
                project: project.into(),
                acl: Vec::new(),
                status,
                summary: String::new(),
                log: String::new(),
                started_at_unix_seconds: 1,
                completed_at_unix_seconds: Some(2),
                exit_code: Some(0),
                retryable: false,
                writes_indexed_data: false,
                budget: None,
            };

        let history = vec![
            snapshot("work-code", "work", "succeeded"),
            snapshot("personal-mail", "work", "succeeded"),
            snapshot("personal-mail", "work", "failed"),
        ];
        assert_eq!(source_jobs_label(&history), "Source jobs: idle");

        let mut failed_latest = history;
        failed_latest[1].status = "failed";
        assert_eq!(
            source_jobs_label(&failed_latest),
            "Source jobs: 1 need attention"
        );

        failed_latest[0].status = "running";
        assert_eq!(source_jobs_label(&failed_latest), "Source jobs: 1 active");

        let duplicate_names = vec![
            snapshot("notes", "work", "failed"),
            snapshot("notes", "personal", "failed"),
            snapshot("notes", "work", "succeeded"),
        ];
        assert_eq!(
            source_jobs_label(&duplicate_names),
            "Source jobs: 2 need attention"
        );
    }
}
