use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_shell::{ShellExt, process::CommandEvent};
use tokio::time::timeout;

use crate::{paths, services, settings};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_DETAIL_BYTES: usize = 4 * 1024;
/// The Desktop control plane refuses snapshots larger than this bound before
/// restore and after backup. This keeps picker-selected paths and sidecar
/// output bounded even when a machine contains an unexpectedly large index.
pub const MAX_BACKUP_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct DatabaseActionResult {
    pub action: String,
    pub path: String,
    pub bytes: u64,
    pub detail: String,
}

pub async fn backup<R: tauri::Runtime>(
    app: &AppHandle<R>,
    approved: bool,
) -> Result<Option<DatabaseActionResult>, String> {
    if !approved {
        return Err("database backup requires explicit approval".into());
    }
    let Some(path) = paths::pick(app.clone(), "backup-export").await? else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    let snapshot = settings::load()?;
    let config_path = PathBuf::from(snapshot.config_path);
    if let Err(error) = validate_export_path(&path) {
        audit(&config_path, "backup", &path, "failed", None, Some(&error));
        return Err(error);
    }

    let args = vec![
        "--config".to_string(),
        config_path.display().to_string(),
        "backup".to_string(),
        path.display().to_string(),
    ];
    let output = match sidecar_output(app, &args).await {
        Ok(output) => output,
        Err(error) => {
            audit(&config_path, "backup", &path, "failed", None, Some(&error));
            return Err(error);
        }
    };
    if !output.success {
        let error = bounded_error(&output.stderr);
        audit(&config_path, "backup", &path, "failed", None, Some(&error));
        return Err(error);
    }
    let bytes = match validate_snapshot_file(&path, false) {
        Ok(bytes) => bytes,
        Err(error) => {
            audit(&config_path, "backup", &path, "failed", None, Some(&error));
            return Err(error);
        }
    };
    let detail = bounded_output(&output.stdout);
    if !detail.contains("backup verified") {
        let error = "bundled Cortana backup returned an unexpected result".to_string();
        audit(
            &config_path,
            "backup",
            &path,
            "failed",
            Some(bytes),
            Some(&error),
        );
        return Err(error);
    }
    audit(
        &config_path,
        "backup",
        &path,
        "succeeded",
        Some(bytes),
        None,
    );
    Ok(Some(DatabaseActionResult {
        action: "backup".into(),
        path: path.display().to_string(),
        bytes,
        detail,
    }))
}

pub async fn restore<R: tauri::Runtime>(
    app: &AppHandle<R>,
    approved: bool,
) -> Result<Option<DatabaseActionResult>, String> {
    if !approved {
        return Err("database restore requires explicit approval".into());
    }
    let Some(path) = paths::pick(app.clone(), "backup-import").await? else {
        return Ok(None);
    };
    let path = PathBuf::from(path);
    let snapshot = settings::load()?;
    let config_path = PathBuf::from(snapshot.config_path);
    let bytes = match validate_snapshot_file(&path, true) {
        Ok(bytes) => bytes,
        Err(error) => {
            audit(&config_path, "restore", &path, "failed", None, Some(&error));
            return Err(error);
        }
    };

    let report = match services::status(app).await {
        Ok(report) => report,
        Err(error) => {
            audit(
                &config_path,
                "restore",
                &path,
                "failed",
                Some(bytes),
                Some(&error),
            );
            return Err(error);
        }
    };
    if let Err(error) = ensure_services_stopped(&report) {
        audit(
            &config_path,
            "restore",
            &path,
            "failed",
            Some(bytes),
            Some(&error),
        );
        return Err(error);
    }

    let args = vec![
        "--config".to_string(),
        config_path.display().to_string(),
        "restore".to_string(),
        path.display().to_string(),
        "--force".to_string(),
    ];
    let output = match sidecar_output(app, &args).await {
        Ok(output) => output,
        Err(error) => {
            audit(
                &config_path,
                "restore",
                &path,
                "failed",
                Some(bytes),
                Some(&error),
            );
            return Err(error);
        }
    };
    if !output.success {
        let error = bounded_error(&output.stderr);
        audit(
            &config_path,
            "restore",
            &path,
            "failed",
            Some(bytes),
            Some(&error),
        );
        return Err(error);
    }
    let detail = bounded_output(&output.stdout);
    if !detail.contains("database restored") {
        let error = "bundled Cortana restore returned an unexpected result".to_string();
        audit(
            &config_path,
            "restore",
            &path,
            "failed",
            Some(bytes),
            Some(&error),
        );
        return Err(error);
    }
    audit(
        &config_path,
        "restore",
        &path,
        "succeeded",
        Some(bytes),
        None,
    );
    Ok(Some(DatabaseActionResult {
        action: "restore".into(),
        path: path.display().to_string(),
        bytes,
        detail,
    }))
}

fn validate_export_path(path: &Path) -> Result<(), String> {
    validate_absolute_path(path)?;
    validate_extension(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| "backup destination has no parent directory".to_string())?;
    if !parent.is_dir() {
        return Err("backup destination parent directory does not exist".into());
    }
    reject_symlink_components(path)?;
    if path.exists() {
        return Err("backup destination already exists; choose a new snapshot path".into());
    }
    Ok(())
}

fn validate_snapshot_file(path: &Path, must_exist: bool) -> Result<u64, String> {
    validate_absolute_path(path)?;
    validate_extension(path)?;
    reject_symlink_components(path)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !must_exist => {
            return Err("backup command did not create a snapshot".into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("selected backup snapshot does not exist".into());
        }
        Err(error) => return Err(format!("inspect backup snapshot: {error}")),
    };
    if metadata.file_type().is_symlink() {
        return Err("backup snapshot must not be a symlink".into());
    }
    if !metadata.is_file() {
        return Err("backup snapshot must be a regular file".into());
    }
    if metadata.len() == 0 {
        return Err("backup snapshot is empty".into());
    }
    if metadata.len() > MAX_BACKUP_BYTES {
        return Err(format!(
            "backup snapshot exceeds the {MAX_BACKUP_BYTES} byte Desktop limit"
        ));
    }
    Ok(metadata.len())
}

fn validate_absolute_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path.parent().is_none()
        || path.parent().is_none_or(|parent| parent.parent().is_none())
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err("backup paths require an absolute non-root path".into());
    }
    Ok(())
}

fn validate_extension(path: &Path) -> Result<(), String> {
    if path.extension().and_then(|value| value.to_str()) != Some("sqlite3") {
        return Err("backup paths must use a .sqlite3 file".into());
    }
    Ok(())
}

fn reject_symlink_components(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "refusing symlinked backup path component {}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(format!("inspect backup path: {error}")),
        }
    }
    Ok(())
}

fn ensure_services_stopped(report: &services::ServiceReport) -> Result<(), String> {
    if !report.supported {
        return Err("restore requires supported local service control".into());
    }
    let running = report
        .services
        .iter()
        .filter(|service| {
            matches!(
                service.name.as_str(),
                "embedding" | "server" | "sync" | "backup" | "vault"
            ) && service_is_running(service)
        })
        .map(|service| service.name.as_str())
        .collect::<Vec<_>>();
    if running.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "stop Cortana services before restore: {}",
            running.join(", ")
        ))
    }
}

fn service_is_running(service: &services::ServiceStatus) -> bool {
    service.state.as_deref() == Some("running") || (service.loaded && service.state.is_none())
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
    .await
    {
        Ok(result) => result,
        Err(_) => {
            terminate_process_group(child);
            Err("Cortana database command timed out".into())
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
            let _ = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        }
    }
    let _ = child.kill();
}

fn bounded_output(bytes: &[u8]) -> String {
    bounded_text(bytes, MAX_DETAIL_BYTES)
}

fn bounded_error(bytes: &[u8]) -> String {
    let value = bounded_text(bytes, MAX_DETAIL_BYTES);
    if value.is_empty() {
        "Cortana database command failed".into()
    } else {
        value
    }
}

fn bounded_text(bytes: &[u8], max_bytes: usize) -> String {
    let end = bytes.len().min(max_bytes);
    String::from_utf8_lossy(&bytes[..end])
        .chars()
        .filter(|character| *character == '\n' || *character == '\t' || !character.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn audit(
    config_path: &Path,
    action: &str,
    path: &Path,
    outcome: &str,
    bytes: Option<u64>,
    detail: Option<&str>,
) {
    let event = serde_json::json!({
        "at_unix_seconds": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "event": format!("desktop.database.{action}"),
        "action": action,
        "path": path.display().to_string(),
        "bytes": bytes,
        "outcome": outcome,
        "detail": detail.map(|value| bounded_text(value.as_bytes(), MAX_DETAIL_BYTES)),
        "secret_values_recorded": false,
    });
    let _ = settings::append_audit_event(config_path, &event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(supported: bool, running: &[&str]) -> services::ServiceReport {
        services::ServiceReport {
            platform: "test".into(),
            supported,
            activity: None,
            services: ["embedding", "server", "sync", "backup", "vault"]
                .into_iter()
                .map(|name| services::ServiceStatus {
                    name: name.into(),
                    label: name.into(),
                    installed: true,
                    loaded: running.contains(&name),
                    state: running.contains(&name).then(|| "running".into()),
                    pid: None,
                    last_exit_status: None,
                })
                .collect(),
        }
    }

    #[test]
    fn backup_paths_require_regular_sqlite3_paths_and_reject_symlinks() {
        let temp = tempfile::tempdir().expect("temp directory");
        // macOS exposes `/var` and `/tmp` as symlinks; validate the physical
        // temp root so this fixture exercises the selected path itself rather
        // than rejecting an OS-owned parent alias.
        let root = temp
            .path()
            .canonicalize()
            .expect("canonical temp directory");
        let destination = root.join("backup.sqlite3");
        assert!(validate_export_path(&destination).is_ok());
        assert!(validate_export_path(&root.join("backup.db")).is_err());
        assert!(validate_export_path(Path::new("relative.sqlite3")).is_err());

        let target = root.join("existing.sqlite3");
        fs::write(&target, b"snapshot").expect("snapshot");
        assert!(validate_export_path(&target).is_err());
        assert_eq!(
            validate_snapshot_file(&target, true).expect("valid snapshot"),
            8
        );

        #[cfg(unix)]
        {
            let link = root.join("link.sqlite3");
            std::os::unix::fs::symlink(&target, &link).expect("symlink");
            assert!(validate_snapshot_file(&link, true).is_err());
        }
    }

    #[test]
    fn restore_requires_supported_and_stopped_services() {
        assert!(ensure_services_stopped(&report(false, &[])).is_err());
        assert!(ensure_services_stopped(&report(true, &[])).is_ok());
        let error = ensure_services_stopped(&report(true, &["server"]))
            .expect_err("running core service must block restore");
        assert!(error.contains("server"));
        let error = ensure_services_stopped(&report(true, &["sync"]))
            .expect_err("running sync must block restore");
        assert!(error.contains("sync"));

        let mut loaded_not_running = report(true, &[]);
        let backup = loaded_not_running
            .services
            .iter_mut()
            .find(|service| service.name == "backup")
            .expect("backup service");
        backup.loaded = true;
        backup.state = Some("not running".into());
        assert!(ensure_services_stopped(&loaded_not_running).is_ok());
    }

    #[test]
    fn sidecar_output_and_audit_detail_are_bounded() {
        let mut output = Vec::new();
        append_bounded(&mut output, &vec![b'x'; MAX_OUTPUT_BYTES + 1]);
        assert_eq!(output.len(), MAX_OUTPUT_BYTES);
        assert!(bounded_output(&vec![b'x'; MAX_DETAIL_BYTES + 100]).len() <= MAX_DETAIL_BYTES);
        let event =
            json!({"detail": bounded_text(&vec![b'x'; MAX_DETAIL_BYTES + 100], MAX_DETAIL_BYTES)});
        assert!(event["detail"].as_str().unwrap().len() <= MAX_DETAIL_BYTES);
    }
}
