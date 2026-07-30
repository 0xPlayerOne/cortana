use std::{
    collections::BTreeSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tokio::time::timeout;

use crate::settings;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const SERVICE_NAMES: [&str; 4] = ["embedding", "server", "sync", "backup"];
const CORE_SERVICE_NAMES: [&str; 2] = ["embedding", "server"];
const ACTIONS: [&str; 3] = ["start", "stop", "restart"];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceReport {
    pub platform: String,
    pub supported: bool,
    pub services: Vec<ServiceStatus>,
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

pub async fn status(app: &AppHandle) -> Result<ServiceReport, String> {
    let output = sidecar_output(app, &["service", "status", "--json"]).await?;
    parse_report(&output.stdout, &output.stderr, output.status.success())
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
    let output = sidecar_output(app, &["service", action, service]).await?;
    if !output.status.success() {
        return Err(bounded_error(&output.stderr));
    }
    let event = serde_json::json!({
        "at_unix_seconds": now(),
        "event": "service.action",
        "service": service,
        "action": action,
        "approved": true,
        "secret_values_recorded": false,
    });
    let _ = settings::append_audit_event(&settings::default_config_path(), &event);
    status(app).await
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
    let services = if action == "stop" {
        CORE_SERVICE_NAMES.into_iter().rev().collect::<Vec<_>>()
    } else {
        CORE_SERVICE_NAMES.to_vec()
    };
    for service in services {
        let output = sidecar_output(app, &["service", action, service]).await?;
        if !output.status.success() {
            return Err(format!("{service}: {}", bounded_error(&output.stderr)));
        }
    }
    let event = serde_json::json!({
        "at_unix_seconds": now(),
        "event": "service.action_all",
        "services": CORE_SERVICE_NAMES,
        "excluded_services": ["sync", "backup"],
        "action": action,
        "approved": true,
        "secret_values_recorded": false,
    });
    let _ = settings::append_audit_event(&settings::default_config_path(), &event);
    status(app).await
}

async fn sidecar_output(
    app: &AppHandle,
    args: &[&str],
) -> Result<tauri_plugin_shell::process::Output, String> {
    let command = app
        .shell()
        .sidecar("cortana")
        .map_err(|error| format!("locate bundled Cortana runtime: {error}"))?
        .args(args);
    timeout(COMMAND_TIMEOUT, command.output())
        .await
        .map_err(|_| "Cortana service command timed out".to_string())?
        .map_err(|error| format!("run bundled Cortana runtime: {error}"))
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
    let end = bytes.len().min(4096);
    let value = String::from_utf8_lossy(&bytes[..end])
        .chars()
        .filter(|character| *character == '\n' || *character == '\t' || !character.is_control())
        .collect::<String>();
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.is_empty() {
        "Cortana service command failed".into()
    } else {
        value
    }
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
}
