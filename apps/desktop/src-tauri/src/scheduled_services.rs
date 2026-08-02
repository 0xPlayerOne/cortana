use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::AppHandle;
use tauri_plugin_shell::{ShellExt, process::CommandEvent};
use tokio::time::timeout;

use crate::{schedule, services, settings};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Install the core service set while applying the Desktop-owned backup
/// interval. The default path delegates to the existing implementation so
/// its command behavior remains identical for untouched installations.
pub async fn install_core(
    app: &AppHandle,
    approved: bool,
) -> Result<services::ServiceReport, String> {
    if !approved {
        return Err("service installation requires explicit approval".into());
    }
    let schedule = schedule::load()?;
    if schedule.backup_interval_seconds
        == schedule::ScheduleSettings::default().backup_interval_seconds
    {
        return services::install(app, approved).await;
    }
    let desktop_settings = settings::load()?;
    let mut args = vec![
        "service".to_string(),
        "install".to_string(),
        "--no-web".to_string(),
        "--backup-seconds".to_string(),
        schedule.backup_interval_seconds.to_string(),
    ];
    if desktop_settings.embedding.provider != "local" {
        args.push("--no-embedding-service".into());
    }
    let output = match sidecar_output(app, &args).await {
        Ok(output) => output,
        Err(error) => {
            audit_install("failed", &schedule);
            return Err(error);
        }
    };
    if !output.success {
        audit_install("failed", &schedule);
        return Err(bounded_error(&output.stderr));
    }
    audit_install("completed", &schedule);
    services::status(app).await
}

/// Install recurring sync using the explicit Desktop-owned schedule.
///
/// The existing service module intentionally retains its CLI-compatible
/// defaults. Desktop settings live in a separate owner-only file so this path
/// can evolve without rewriting an active user-owned settings diff.
pub async fn install_sync(
    app: &AppHandle,
    approved: bool,
) -> Result<services::ServiceReport, String> {
    if !approved {
        return Err("recurring sync installation requires explicit approval".into());
    }
    let schedule = schedule::load()?;
    if schedule == schedule::ScheduleSettings::default() {
        return services::install_sync(app, approved).await;
    }
    let desktop_settings = settings::load()?;
    let mut args = vec![
        "service".to_string(),
        "install".to_string(),
        "--no-web".to_string(),
        "--enable-sync-service".to_string(),
        "--sync-seconds".to_string(),
        schedule.sync_interval_seconds.to_string(),
        "--backup-seconds".to_string(),
        schedule.backup_interval_seconds.to_string(),
    ];
    if desktop_settings.embedding.provider != "local" {
        args.push("--no-embedding-service".into());
    }
    let output = match sidecar_output(app, &args).await {
        Ok(output) => output,
        Err(error) => {
            audit("failed", &schedule);
            return Err(error);
        }
    };
    if !output.success {
        audit("failed", &schedule);
        return Err(bounded_error(&output.stderr));
    }
    audit("completed", &schedule);
    services::status(app).await
}

async fn sidecar_output(app: &AppHandle, args: &[String]) -> Result<SidecarOutput, String> {
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
    match timeout(COMMAND_TIMEOUT, async {
        let mut stderr = Vec::new();
        let mut success = false;
        while let Some(event) = receiver.recv().await {
            match event {
                CommandEvent::Stdout(_) => {}
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
        Ok(SidecarOutput { success, stderr })
    })
    .await
    {
        Ok(result) => result,
        Err(_) => {
            terminate_process_group(child);
            Err("Cortana service command timed out".into())
        }
    }
}

struct SidecarOutput {
    success: bool,
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
            let _ = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        }
    }
    let _ = child.kill();
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

fn audit(outcome: &str, schedule: &schedule::ScheduleSettings) {
    let event = serde_json::json!({
        "at_unix_seconds": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "event": "service.sync_install",
        "services": ["sync", "backup"],
        "outcome": outcome,
        "sync_interval_seconds": schedule.sync_interval_seconds,
        "backup_interval_seconds": schedule.backup_interval_seconds,
        "approved": true,
        "secret_values_recorded": false,
    });
    let _ = settings::append_audit_event(&settings::default_config_path(), &event);
}

fn audit_install(outcome: &str, schedule: &schedule::ScheduleSettings) {
    let event = serde_json::json!({
        "at_unix_seconds": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "event": "service.install",
        "services": ["embedding", "server", "backup"],
        "outcome": outcome,
        "backup_interval_seconds": schedule.backup_interval_seconds,
        "approved": true,
        "secret_values_recorded": false,
    });
    let _ = settings::append_audit_event(&settings::default_config_path(), &event);
}
