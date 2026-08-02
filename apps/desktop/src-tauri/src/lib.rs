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

mod installer;
mod hindsight;
mod honcho;
mod paths;
mod readiness;
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
        if let Some(token) = settings::bearer_for_scope(scope)? {
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

fn validate_external_url(url: &str) -> Result<(), String> {
    let parsed = Url::parse(url).map_err(|error| format!("invalid URL: {error}"))?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("external links must not contain embedded credentials".into());
    }
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        "mailto" => Ok(()),
        "slack" => validate_slack_url(&parsed),
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
    let message = message
        .ok_or_else(|| "Slack links must include a message timestamp".to_string())?;
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
    let target = fs::canonicalize(&target)
        .map_err(|error| format!("resolve local source path: {error}"))?;
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
fn desktop_info(app: AppHandle) -> DesktopInfo {
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
async fn desktop_services_status(app: AppHandle) -> Result<services::ServiceReport, String> {
    services::status(&app).await
}

#[tauri::command]
async fn desktop_services_install(
    app: AppHandle,
    approved: bool,
) -> Result<services::ServiceReport, String> {
    services::install(&app, approved).await
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
fn desktop_update_status(updater: State<'_, updater::UpdaterState>) -> updater::UpdateSnapshot {
    updater.status()
}

#[tauri::command]
async fn desktop_update_check(
    app: AppHandle,
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
fn desktop_source_validation_start(
    app: AppHandle,
    jobs: State<'_, source_jobs::SourceJobState>,
    source: String,
    budget: Option<source_jobs::InitialSyncBudget>,
) -> Result<source_jobs::SourceJobSnapshot, String> {
    jobs.start_validation(&app, &source, budget)
}

#[tauri::command]
fn desktop_source_authorization_start(
    app: AppHandle,
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
fn desktop_source_initial_sync(
    app: AppHandle,
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
async fn desktop_readiness_scan(app: AppHandle) -> readiness::ReadinessSnapshot {
    readiness::scan(&app).await
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
    if request
        .query
        .as_ref()
        .is_some_and(|query| query.is_empty() || query.len() > MAX_SCOPE_LENGTH)
    {
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

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn install_tray(app: &mut tauri::App) -> tauri::Result<TrayStatus> {
    let health = MenuItem::with_id(app, "health", "Runtime: checking", false, None::<&str>)?;
    let corpus = MenuItem::with_id(app, "corpus", "Corpus: checking", false, None::<&str>)?;
    let ingestion =
        MenuItem::with_id(app, "ingestion", "Ingestion: checking", false, None::<&str>)?;
    let source_jobs =
        MenuItem::with_id(app, "source-jobs", "Source jobs: checking", false, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "Show Cortana", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Cortana Desktop", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&health, &corpus, &ingestion, &source_jobs, &show, &quit])?;

    let mut builder = TrayIconBuilder::with_id("cortana")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Cortana second brain")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => {
                QUITTING.store(true, Ordering::SeqCst);
                app.exit(0);
            }
            _ => {}
        })
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
        .filter(|job| seen_sources.insert(job.source.as_str()))
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
            desktop_url_open,
            desktop_info,
            desktop_autostart_set,
            desktop_services_status,
            desktop_services_install,
            desktop_hindsight_status,
            desktop_honcho_status,
            desktop_service_action,
            desktop_services_action_all,
            desktop_update_status,
            desktop_update_check,
            desktop_update_install,
            desktop_settings_get,
            desktop_settings_save,
            desktop_settings_export,
            desktop_settings_import,
            desktop_path_pick,
            desktop_readiness_scan,
            desktop_installer_start,
            desktop_installer_status,
            desktop_installer_cancel,
            desktop_source_validation_start,
            desktop_source_authorization_start,
            desktop_source_trial_sync_start,
            desktop_source_setup_open,
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
            if window.label() == MAIN_WINDOW
                && !QUITTING.load(Ordering::SeqCst)
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
        assert!(validate_external_url(
            "slack://channel?team=&id=C123ABC&message=1712345678.000100"
        )
        .is_ok());
        assert!(validate_external_url("slack://channel?id=C123ABC").is_err());
        assert!(validate_external_url("slack://channel?id=C123ABC&message=1&message=2").is_err());
        assert!(validate_external_url(
            "slack://channel?id=C123ABC&message=1&redirect=https://evil.example"
        )
        .is_err());
        assert!(validate_external_url("slack://channel?id=C123ABC&message=.").is_err());
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
            ingestion_label(&serde_json::json!({"sync_runs": [], "ingestion": {"scheduled": true}})),
            "Ingestion: scheduled"
        );
        assert_eq!(
            ingestion_label(&serde_json::json!({})),
            "Ingestion: manual"
        );
    }

    #[test]
    fn tray_source_job_label_uses_latest_terminal_result_per_source() {
        let snapshot = |source: &str, status: &'static str| source_jobs::SourceJobSnapshot {
            id: format!("source-1-{}", source.len()),
            operation: "validation",
            source: source.into(),
            kind: "filesystem".into(),
            project: "work".into(),
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
            snapshot("work-code", "succeeded"),
            snapshot("personal-mail", "succeeded"),
            snapshot("personal-mail", "failed"),
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
    }
}
