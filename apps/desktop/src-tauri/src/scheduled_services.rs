use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::AppHandle;
use tauri_plugin_shell::{ShellExt, process::CommandEvent};
use tokio::time::timeout;

use crate::{schedule, services, settings};

// Core service installation can immediately launch a cold local embedding or
// server process. Keep the operation bounded by the configured five-minute
// startup ceiling while allowing the observed index-open/model-warmup window
// to complete.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Install the core service set while applying the Desktop-owned backup
/// interval. The default path delegates to the existing implementation so
/// its command behavior remains identical for untouched installations.
pub async fn install_core<R: tauri::Runtime>(
    app: &AppHandle<R>,
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
    let args = install_args(&schedule, &desktop_settings.embedding.provider, false);
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
pub async fn install_sync<R: tauri::Runtime>(
    app: &AppHandle<R>,
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
    let args = install_args(&schedule, &desktop_settings.embedding.provider, true);
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

/// Build the argument vector for the bundled runtime's service install
/// command from the Desktop-owned schedule and embedding provider.
///
/// Pure so the interval and embedding flag wiring can be unit tested without
/// a sidecar or the settings store. The async installers above only build
/// these args when the schedule differs from the CLI-compatible default and
/// otherwise delegate to the existing service module.
fn install_args(
    schedule: &schedule::ScheduleSettings,
    embedding_provider: &str,
    enable_sync_service: bool,
) -> Vec<String> {
    let mut args = vec![
        "service".to_string(),
        "install".to_string(),
        "--no-web".to_string(),
    ];
    if enable_sync_service {
        args.push("--enable-sync-service".into());
        args.push("--sync-seconds".into());
        args.push(schedule.sync_interval_seconds.to_string());
    }
    args.push("--backup-seconds".into());
    args.push(schedule.backup_interval_seconds.to_string());
    if embedding_provider != "local" {
        args.push("--no-embedding-service".into());
    }
    args
}

async fn sidecar_output<R: tauri::Runtime>(
    app: &AppHandle<R>,
    args: &[String],
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

#[cfg(test)]
mod tests {
    use super::*;

    fn schedule(
        sync_interval_seconds: u64,
        backup_interval_seconds: u64,
    ) -> schedule::ScheduleSettings {
        schedule::ScheduleSettings {
            sync_interval_seconds,
            backup_interval_seconds,
        }
    }

    /// Value carried by the flag at `index` in the built argument vector.
    fn flag_value<'a>(args: &'a [String], flag: &str) -> &'a str {
        let index = args
            .iter()
            .position(|arg| arg == flag)
            .unwrap_or_else(|| panic!("expected {flag} in {args:?}"));
        &args[index + 1]
    }

    #[test]
    fn install_args_use_default_intervals_without_sync() {
        let args = install_args(&schedule::ScheduleSettings::default(), "local", false);
        assert_eq!(
            args,
            vec![
                "service".to_string(),
                "install".to_string(),
                "--no-web".to_string(),
                "--backup-seconds".to_string(),
                "86400".to_string(),
            ]
        );
        assert!(!args.iter().any(|arg| arg == "--enable-sync-service"));
        assert!(!args.iter().any(|arg| arg == "--sync-seconds"));
        assert!(!args.iter().any(|arg| arg == "--no-embedding-service"));
    }

    #[test]
    fn install_args_carry_custom_backup_interval() {
        let args = install_args(&schedule(900, 3_600), "local", false);
        assert_eq!(flag_value(&args, "--backup-seconds"), "3600");
        assert!(!args.iter().any(|arg| arg == "--no-embedding-service"));
    }

    #[test]
    fn install_args_carry_custom_sync_and_backup_intervals() {
        let args = install_args(&schedule(1_800, 7_200), "local", true);
        assert!(args.iter().any(|arg| arg == "--enable-sync-service"));
        assert_eq!(flag_value(&args, "--sync-seconds"), "1800");
        assert_eq!(flag_value(&args, "--backup-seconds"), "7200");
    }

    #[test]
    fn install_args_keep_sync_flags_out_when_sync_disabled() {
        let args = install_args(&schedule(1_800, 7_200), "local", false);
        assert!(!args.iter().any(|arg| arg == "--enable-sync-service"));
        assert!(!args.iter().any(|arg| arg == "--sync-seconds"));
        assert_eq!(flag_value(&args, "--backup-seconds"), "7200");
    }

    #[test]
    fn install_args_only_disable_embedding_for_non_local_providers() {
        let args = install_args(&schedule::ScheduleSettings::default(), "local", false);
        assert!(!args.iter().any(|arg| arg == "--no-embedding-service"));

        let args = install_args(&schedule::ScheduleSettings::default(), "cloud", false);
        assert!(args.iter().any(|arg| arg == "--no-embedding-service"));
        assert_eq!(
            args.last().map(String::as_str),
            Some("--no-embedding-service")
        );
    }

    #[test]
    fn service_install_budget_covers_cold_core_startup() {
        assert!(COMMAND_TIMEOUT >= Duration::from_secs(5 * 60));
    }
}
