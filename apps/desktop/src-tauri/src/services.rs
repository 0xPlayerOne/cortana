use std::{
    collections::BTreeSet,
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_shell::{ShellExt, process::CommandEvent};
use tokio::time::timeout;

use crate::settings;

// A service restart can reopen the local index and cold-start the embedding
// model before launchd reports the command complete. The configured embedding
// startup ceiling is five minutes; use the same bounded budget here so a
// valid cold start is not reported as a Desktop failure at 60 seconds.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const SERVICE_NAMES: [&str; 4] = ["embedding", "server", "sync", "backup"];
const CORE_SERVICE_NAMES: [&str; 2] = ["embedding", "server"];
const ACTIONS: [&str; 3] = ["start", "stop", "restart"];
const ACTIVITY_ACTIONS: [&str; 4] = ["install", "start", "stop", "restart"];
const ACTIVITY_STATUSES: [&str; 4] = ["running", "succeeded", "failed", "cancelled"];

static SERVICE_ACTION_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceReport {
    pub platform: String,
    pub supported: bool,
    pub services: Vec<ServiceStatus>,
    #[serde(default)]
    pub activity: Option<ServiceActivity>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceActivity {
    pub target: String,
    pub action: String,
    pub status: String,
    pub started_at_unix_seconds: u64,
    pub elapsed_ms: Option<u64>,
    pub detail: Option<String>,
    pub last_output: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceStatus {
    pub name: String,
    pub label: String,
    pub installed: bool,
    pub loaded: bool,
    pub state: Option<String>,
    pub pid: Option<u32>,
    pub last_exit_status: Option<i32>,
}

pub async fn status<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<ServiceReport, String> {
    let output = sidecar_output(app, &["service", "status", "--json"]).await?;
    parse_report(&output.stdout, &output.stderr, output.success).map(with_latest_activity)
}

/// Install the safe, query-only service set from the bundled runtime.
///
/// This deliberately does not install recurring ingestion. The operator can
/// still opt into that separately with the CLI after validating source
/// budgets, preserving the same safe default as `cortana service install`.
pub async fn install(app: &AppHandle, approved: bool) -> Result<ServiceReport, String> {
    if !approved {
        return Err("service installation requires explicit approval".into());
    }
    let _action_guard = acquire_action_lock().await?;
    let started_at = now();
    let started = Instant::now();
    record_activity(
        "core services",
        "install",
        "running",
        started_at,
        None,
        None,
        None,
    );
    let use_local_embedding = match settings::load() {
        Ok(settings) => settings.embedding.provider == "local",
        Err(error) => {
            record_activity(
                "core services",
                "install",
                "failed",
                started_at,
                Some(elapsed_ms(started)),
                Some(&error),
                Some(&error),
            );
            return Err(error);
        }
    };
    let mut args = vec!["service", "install", "--no-web"];
    if !use_local_embedding {
        args.push("--no-embedding-service");
    }
    let output = match sidecar_output(app, &args).await {
        Ok(output) => output,
        Err(error) => {
            record_activity(
                "core services",
                "install",
                "failed",
                started_at,
                Some(elapsed_ms(started)),
                Some(&error),
                Some(&error),
            );
            return Err(error);
        }
    };
    if !output.success {
        let detail = bounded_error(&output.stderr);
        record_activity(
            "core services",
            "install",
            "failed",
            started_at,
            Some(elapsed_ms(started)),
            Some(&detail),
            Some(&detail),
        );
        return Err(detail);
    }
    let report = match status(app).await {
        Ok(report) => report,
        Err(error) => {
            record_activity(
                "core services",
                "install",
                "failed",
                started_at,
                Some(elapsed_ms(started)),
                Some(&error),
                Some(&error),
            );
            return Err(error);
        }
    };
    let last_output = bounded_output(&output.stdout);
    record_activity(
        "core services",
        "install",
        "succeeded",
        started_at,
        Some(elapsed_ms(started)),
        None,
        last_output.as_deref(),
    );
    Ok(with_latest_activity(report))
}

/// Install the explicitly approved recurring sync job after the bundled CLI
/// re-checks validation coverage for every enabled source.
pub async fn install_sync(app: &AppHandle, approved: bool) -> Result<ServiceReport, String> {
    if !approved {
        return Err("recurring sync installation requires explicit approval".into());
    }
    let _action_guard = acquire_action_lock().await?;
    let started_at = now();
    let started = Instant::now();
    record_activity(
        "recurring sync",
        "install",
        "running",
        started_at,
        None,
        None,
        None,
    );
    let use_local_embedding = match settings::load() {
        Ok(settings) => settings.embedding.provider == "local",
        Err(error) => {
            record_activity(
                "recurring sync",
                "install",
                "failed",
                started_at,
                Some(elapsed_ms(started)),
                Some(&error),
                Some(&error),
            );
            return Err(error);
        }
    };
    let mut args = vec!["service", "install", "--no-web", "--enable-sync-service"];
    if !use_local_embedding {
        args.push("--no-embedding-service");
    }
    let output = match sidecar_output(app, &args).await {
        Ok(output) => output,
        Err(error) => {
            record_activity(
                "recurring sync",
                "install",
                "failed",
                started_at,
                Some(elapsed_ms(started)),
                Some(&error),
                Some(&error),
            );
            return Err(error);
        }
    };
    if !output.success {
        let detail = bounded_error(&output.stderr);
        record_activity(
            "recurring sync",
            "install",
            "failed",
            started_at,
            Some(elapsed_ms(started)),
            Some(&detail),
            Some(&detail),
        );
        return Err(detail);
    }
    let report = match status(app).await {
        Ok(report) => report,
        Err(error) => {
            record_activity(
                "recurring sync",
                "install",
                "failed",
                started_at,
                Some(elapsed_ms(started)),
                Some(&error),
                Some(&error),
            );
            return Err(error);
        }
    };
    let last_output = bounded_output(&output.stdout);
    record_activity(
        "recurring sync",
        "install",
        "succeeded",
        started_at,
        Some(elapsed_ms(started)),
        None,
        last_output.as_deref(),
    );
    Ok(with_latest_activity(report))
}

pub async fn action(
    app: &AppHandle,
    service: &str,
    action: &str,
    approved: bool,
) -> Result<ServiceReport, String> {
    if !approved {
        return Err("service action requires explicit approval".into());
    }
    if !SERVICE_NAMES.contains(&service) {
        return Err("unsupported Cortana service".into());
    }
    if !ACTIONS.contains(&action) {
        return Err("unsupported Cortana service action".into());
    }
    let _action_guard = acquire_action_lock().await?;
    let started_at = now();
    let started = Instant::now();
    record_activity(service, action, "running", started_at, None, None, None);
    let output = match sidecar_output(app, &["service", action, service]).await {
        Ok(output) => output,
        Err(error) => {
            record_activity(
                service,
                action,
                "failed",
                started_at,
                Some(elapsed_ms(started)),
                Some(&error),
                Some(&error),
            );
            return Err(error);
        }
    };
    if !output.success {
        let detail = bounded_error(&output.stderr);
        record_activity(
            service,
            action,
            "failed",
            started_at,
            Some(elapsed_ms(started)),
            Some(&detail),
            Some(&detail),
        );
        return Err(detail);
    }
    let report = match status(app).await {
        Ok(report) => report,
        Err(error) => {
            record_activity(
                service,
                action,
                "failed",
                started_at,
                Some(elapsed_ms(started)),
                Some(&error),
                Some(&error),
            );
            return Err(error);
        }
    };
    let elapsed_ms = elapsed_ms(started);
    let last_output = bounded_output(&output.stdout);
    record_activity(
        service,
        action,
        "succeeded",
        started_at,
        Some(elapsed_ms),
        None,
        last_output.as_deref(),
    );
    Ok(with_latest_activity(report))
}

pub async fn action_all(
    app: &AppHandle,
    action: &str,
    approved: bool,
) -> Result<ServiceReport, String> {
    if !approved {
        return Err("whole-app service action requires explicit approval".into());
    }
    if !ACTIONS.contains(&action) {
        return Err("unsupported whole-app service action".into());
    }
    let _action_guard = acquire_action_lock().await?;
    let started_at = now();
    let started = Instant::now();
    record_activity(
        "core services",
        action,
        "running",
        started_at,
        None,
        None,
        None,
    );
    // A cloud embedding provider deliberately omits the local embedding
    // service. Do not send a whole-app action to that absent task: doing so
    // would make an otherwise healthy server report a failed aggregate action.
    let core_services = core_service_names(settings::load()?.embedding.provider == "local");
    let services = if action == "stop" {
        core_services.iter().rev().copied().collect::<Vec<_>>()
    } else {
        core_services.clone()
    };
    for service in services {
        let output = match sidecar_output(app, &["service", action, service]).await {
            Ok(output) => output,
            Err(error) => {
                record_activity(
                    service,
                    action,
                    "failed",
                    started_at,
                    Some(elapsed_ms(started)),
                    Some(&error),
                    Some(&error),
                );
                return Err(error);
            }
        };
        if !output.success {
            let detail = format!("{service}: {}", bounded_error(&output.stderr));
            record_activity(
                service,
                action,
                "failed",
                started_at,
                Some(elapsed_ms(started)),
                Some(&detail),
                Some(&detail),
            );
            return Err(detail);
        }
    }
    let report = match status(app).await {
        Ok(report) => report,
        Err(error) => {
            record_activity(
                "core services",
                action,
                "failed",
                started_at,
                Some(elapsed_ms(started)),
                Some(&error),
                Some(&error),
            );
            return Err(error);
        }
    };
    let elapsed = elapsed_ms(started);
    record_activity(
        "core services",
        action,
        "succeeded",
        started_at,
        Some(elapsed),
        None,
        None,
    );
    Ok(with_latest_activity(report))
}

fn core_service_names(use_local_embedding: bool) -> Vec<&'static str> {
    if use_local_embedding {
        CORE_SERVICE_NAMES.to_vec()
    } else {
        vec!["server"]
    }
}

async fn sidecar_output<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    args: &[&str],
) -> Result<SidecarOutput, String> {
    let command = app
        .shell()
        .sidecar("cortana")
        .map_err(|error| format!("locate bundled Cortana runtime: {error}"))?
        .args(args)
        .env("CORTANA_DESKTOP_PROCESS_GROUP", "1")
        .set_raw_out(true);
    let (mut receiver, child) = command
        .spawn()
        .map_err(|error| format!("run bundled Cortana runtime: {error}"))?;
    let result = timeout(COMMAND_TIMEOUT, async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut success = false;
        while let Some(event) = receiver.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => append_bounded(&mut stdout, &bytes),
                CommandEvent::Stderr(bytes) => append_bounded(&mut stderr, &bytes),
                CommandEvent::Error(error) => {
                    return Err(format!("run bundled Cortana runtime: {error}"));
                }
                CommandEvent::Terminated(payload) => {
                    success = payload.code == Some(0);
                    break;
                }
                _ => {}
            }
        }
        Ok(SidecarOutput {
            success,
            stdout,
            stderr,
        })
    })
    .await;
    match result {
        Ok(result) => result,
        Err(_) => {
            terminate_process_group(child);
            Err("Cortana service command timed out".into())
        }
    }
}

struct SidecarOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn append_bounded(buffer: &mut Vec<u8>, bytes: &[u8]) {
    let remaining = MAX_OUTPUT_BYTES.saturating_sub(buffer.len());
    buffer.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
}

fn terminate_process_group(child: tauri_plugin_shell::process::CommandChild) {
    #[cfg(unix)]
    {
        let pid = child.pid();
        if pid > 0 && pid <= i32::MAX as u32 {
            // The bundled CLI opts into this process group so a timeout also
            // terminates connector/service helpers it may have started.
            let _ = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        }
    }
    let _ = child.kill();
}

fn parse_report(stdout: &[u8], stderr: &[u8], succeeded: bool) -> Result<ServiceReport, String> {
    if !succeeded {
        return Err(bounded_error(stderr));
    }
    if stdout.len() > MAX_OUTPUT_BYTES {
        return Err("Cortana service report exceeded 64 KiB".into());
    }
    let report: ServiceReport =
        serde_json::from_slice(stdout).map_err(|_| "Cortana service report was invalid")?;
    let names = report
        .services
        .iter()
        .map(|item| item.name.as_str())
        .collect::<BTreeSet<_>>();
    if report.services.len() != SERVICE_NAMES.len()
        || names.len() != SERVICE_NAMES.len()
        || names.iter().any(|name| !SERVICE_NAMES.contains(name))
    {
        return Err("Cortana service report contained unsupported services".into());
    }
    Ok(report)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn bounded_error(bytes: &[u8]) -> String {
    sanitize_activity_text(
        &String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]),
        4096,
    )
}

fn bounded_output(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        None
    } else {
        Some(sanitize_activity_text(
            &String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_OUTPUT_BYTES)]),
            MAX_OUTPUT_BYTES,
        ))
    }
}

fn sanitize_activity_text(value: &str, max_bytes: usize) -> String {
    let value = value
        .chars()
        .filter(|character| *character == '\n' || *character == '\t' || !character.is_control())
        .collect::<String>()
        .split_whitespace()
        .map(|part| {
            let lower = part.to_ascii_lowercase();
            if ["token=", "password=", "secret=", "api_key="]
                .iter()
                .any(|prefix| lower.starts_with(prefix))
            {
                part.split('=').next().unwrap_or("value").to_string() + "=<redacted>"
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut bounded = String::new();
    for character in value.chars() {
        if bounded.len() + character.len_utf8() > max_bytes {
            break;
        }
        bounded.push(character);
    }
    if bounded.is_empty() {
        "Cortana service command failed".into()
    } else {
        bounded
    }
}

async fn acquire_action_lock() -> Result<tokio::sync::MutexGuard<'static, ()>, String> {
    SERVICE_ACTION_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .try_lock()
        .map_err(|_| "a Cortana service action is already running; wait for it to finish".into())
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

fn with_latest_activity(mut report: ServiceReport) -> ServiceReport {
    report.activity = latest_activity();
    report
}

fn latest_activity() -> Option<ServiceActivity> {
    settings::desktop_audit_events(100)
        .ok()?
        .into_iter()
        .find_map(|event| parse_activity(&event))
}

fn record_activity(
    target: &str,
    action: &str,
    status: &str,
    started_at: u64,
    elapsed_ms: Option<u64>,
    detail: Option<&str>,
    last_output: Option<&str>,
) {
    let event = serde_json::json!({
        "at_unix_seconds": now(),
        "event": "service.activity",
        "service_target": target,
        "service_action": action,
        "service_status": status,
        "service_started_at_unix_seconds": started_at,
        "service_elapsed_ms": elapsed_ms,
        "service_detail": detail.map(|value| sanitize_activity_text(value, 4096)),
        "service_last_output": last_output.map(|value| sanitize_activity_text(value, MAX_OUTPUT_BYTES)),
        "secret_values_recorded": false,
    });
    let _ = settings::append_audit_event(&settings::default_config_path(), &event);
}

fn parse_activity(event: &serde_json::Value) -> Option<ServiceActivity> {
    if event.get("event")?.as_str()? != "service.activity" {
        return None;
    }
    let action = event.get("service_action")?.as_str()?.to_string();
    let target = event.get("service_target")?.as_str()?.to_string();
    let status = event.get("service_status")?.as_str()?.to_string();
    if !ACTIVITY_ACTIONS.contains(&action.as_str())
        || (!SERVICE_NAMES.contains(&target.as_str())
            && target != "core services"
            && target != "recurring sync")
        || !ACTIVITY_STATUSES.contains(&status.as_str())
    {
        return None;
    }
    Some(ServiceActivity {
        target,
        action,
        status,
        started_at_unix_seconds: event.get("service_started_at_unix_seconds")?.as_u64()?,
        elapsed_ms: event
            .get("service_elapsed_ms")
            .and_then(serde_json::Value::as_u64),
        detail: event
            .get("service_detail")
            .and_then(serde_json::Value::as_str)
            .map(|value| sanitize_activity_text(value, 4096)),
        last_output: event
            .get("service_last_output")
            .and_then(serde_json::Value::as_str)
            .map(|value| sanitize_activity_text(value, MAX_OUTPUT_BYTES)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_reject_unknown_services_and_bound_errors() {
        let report = serde_json::json!({
            "platform": "macos",
            "supported": true,
            "services": [
                {"name":"embedding","label":"ai.cortana.embedding","installed":true,"loaded":true},
                {"name":"server","label":"ai.cortana.server","installed":true,"loaded":true},
                {"name":"sync","label":"ai.cortana.sync","installed":false,"loaded":false},
                {"name":"other","label":"ai.cortana.other","installed":true,"loaded":false}
            ]
        });
        assert!(parse_report(report.to_string().as_bytes(), b"", true).is_err());
        assert_eq!(bounded_error(b"bad\x1b[31m\0 result"), "bad[31m result");
        assert!(!CORE_SERVICE_NAMES.contains(&"sync"));
        assert!(!CORE_SERVICE_NAMES.contains(&"backup"));
    }

    #[test]
    fn sidecar_output_bounds_each_stream() {
        let mut output = Vec::new();
        append_bounded(&mut output, &vec![b'x'; MAX_OUTPUT_BYTES + 1]);
        assert_eq!(output.len(), MAX_OUTPUT_BYTES);
        append_bounded(&mut output, b"more");
        assert_eq!(output.len(), MAX_OUTPUT_BYTES);
    }

    #[test]
    fn cloud_embedding_aggregate_actions_skip_the_absent_local_service() {
        assert_eq!(core_service_names(false), vec!["server"]);
        assert_eq!(core_service_names(true), CORE_SERVICE_NAMES.to_vec());
    }

    #[test]
    fn service_command_budget_covers_a_cold_server_restart() {
        assert!(COMMAND_TIMEOUT >= Duration::from_secs(5 * 60));
    }

    #[test]
    fn service_activity_is_bounded_and_redacts_control_data() {
        let event = serde_json::json!({
            "event": "service.activity",
            "service_action": "restart",
            "service_target": "embedding",
            "service_status": "failed",
            "service_started_at_unix_seconds": 10,
            "service_elapsed_ms": 42,
            "service_detail": "embedding failed with token=private-value",
            "service_last_output": "embedding failed with token=private-value"
        });

        let activity = parse_activity(&event).expect("valid service activity");
        assert_eq!(activity.action, "restart");
        assert_eq!(activity.target, "embedding");
        assert_eq!(activity.status, "failed");
        assert_eq!(activity.elapsed_ms, Some(42));
        assert!(!activity
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("private-value"));
        assert!(activity.last_output.as_deref().unwrap_or_default().len() <= MAX_OUTPUT_BYTES);
    }

    #[test]
    fn invalid_service_activity_is_rejected() {
        let event = serde_json::json!({
            "event": "service.activity",
            "service_action": "shell",
            "service_target": "../../config",
            "service_status": "unknown",
            "service_started_at_unix_seconds": 10
        });

        assert!(parse_activity(&event).is_none());
    }
}
