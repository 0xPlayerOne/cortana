use std::{
    collections::BTreeSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_shell::{ShellExt, process::CommandEvent};
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
    parse_report(&output.stdout, &output.stderr, output.success)
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
    let use_local_embedding = settings::load()?.embedding.provider == "local";
    let mut args = vec!["service", "install", "--no-web"];
    if !use_local_embedding {
        args.push("--no-embedding-service");
    }
    let output = match sidecar_output(app, &args).await {
        Ok(output) => output,
        Err(error) => {
            audit_action("service.install", "install", &[], "failed", None);
            return Err(error);
        }
    };
    if !output.success {
        audit_action("service.install", "install", &[], "failed", None);
        return Err(bounded_error(&output.stderr));
    }
    audit_action("service.install", "install", &[], "completed", None);
    status(app).await
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
    let output = match sidecar_output(app, &["service", action, service]).await {
        Ok(output) => output,
        Err(error) => {
            audit_action(
                "service.action",
                action,
                &[service],
                "failed",
                Some(service),
            );
            return Err(error);
        }
    };
    if !output.success {
        audit_action(
            "service.action",
            action,
            &[service],
            "failed",
            Some(service),
        );
        return Err(bounded_error(&output.stderr));
    }
    audit_action("service.action", action, &[service], "completed", None);
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
        let output = match sidecar_output(app, &["service", action, service]).await {
            Ok(output) => output,
            Err(error) => {
                audit_action(
                    "service.action_all",
                    action,
                    &CORE_SERVICE_NAMES,
                    "failed",
                    Some(service),
                );
                return Err(error);
            }
        };
        if !output.success {
            audit_action(
                "service.action_all",
                action,
                &CORE_SERVICE_NAMES,
                "failed",
                Some(service),
            );
            return Err(format!("{service}: {}", bounded_error(&output.stderr)));
        }
    }
    audit_action(
        "service.action_all",
        action,
        &CORE_SERVICE_NAMES,
        "completed",
        None,
    );
    status(app).await
}

fn audit_action(
    event_name: &str,
    action: &str,
    services: &[&str],
    outcome: &str,
    failed_service: Option<&str>,
) {
    let event = serde_json::json!({
        "at_unix_seconds": now(),
        "event": event_name,
        "services": services,
        "failed_service": failed_service,
        "action": action,
        "outcome": outcome,
        "approved": true,
        "secret_values_recorded": false,
    });
    let _ = settings::append_audit_event(&settings::default_config_path(), &event);
}

async fn sidecar_output(
    app: &AppHandle,
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

    #[test]
    fn sidecar_output_bounds_each_stream() {
        let mut output = Vec::new();
        append_bounded(&mut output, &vec![b'x'; MAX_OUTPUT_BYTES + 1]);
        assert_eq!(output.len(), MAX_OUTPUT_BYTES);
        append_bounded(&mut output, b"more");
        assert_eq!(output.len(), MAX_OUTPUT_BYTES);
    }
}
