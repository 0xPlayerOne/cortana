use std::{
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
        let response = request
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
        response
            .json()
            .await
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
    installer: State<'_, installer::InstallerState>,
    tool: String,
    approved: bool,
) -> Result<installer::InstallJobSnapshot, String> {
    installer.start(&tool, approved)
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
) -> Result<source_jobs::SourceJobSnapshot, String> {
    jobs.start_validation(&app, &source)
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
fn desktop_source_validation_status(
    jobs: State<'_, source_jobs::SourceJobState>,
    id: String,
) -> Result<source_jobs::SourceJobSnapshot, String> {
    jobs.status(&id)
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
        if value.is_some_and(|value| value.len() > MAX_SCOPE_LENGTH) {
            return Err(format!(
                "{name} exceeds the {MAX_SCOPE_LENGTH} byte desktop safety limit"
            ));
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
        if value.is_some_and(|value| value.is_empty() || value.len() > MAX_SCOPE_LENGTH) {
            return Err(format!("{name} must contain 1 to {MAX_SCOPE_LENGTH} bytes"));
        }
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
    let show = MenuItem::with_id(app, "show", "Show Cortana", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Cortana Desktop", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&health, &corpus, &show, &quit])?;

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

    Ok(TrayStatus { health, corpus })
}

async fn refresh_tray(backend: &BackendClient, tray: &TrayStatus) {
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
        }
        Err(_) => {
            let _ = tray.health.set_text("Runtime: offline");
            let _ = tray.corpus.set_text("Corpus: unavailable");
        }
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
            brain_audit,
            desktop_audit,
            desktop_project_open,
            desktop_info,
            desktop_autostart_set,
            desktop_services_status,
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
            desktop_source_validation_status,
            desktop_source_validation_cancel
        ])
        .setup(move |app| {
            let tray = install_tray(app)?;
            let backend = backend.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    refresh_tray(&backend, &tray).await;
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
            cursor: Some("opaque-cursor".into()),
            limit: 50,
        };
        assert!(validate_document_list_request(&valid).is_ok());
        assert!(
            validate_document_list_request(&DocumentListRequest { limit: 0, ..valid }).is_err()
        );
        assert!(validate_document_id(&"a".repeat(64)).is_ok());
        assert!(validate_document_id("../store.sqlite3").is_err());
    }
}
