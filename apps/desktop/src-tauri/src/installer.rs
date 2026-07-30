use std::{
    collections::BTreeMap,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::{io::AsyncReadExt, process::Command};

const MAX_LOG_BYTES: u64 = 64 * 1024;
const MAX_JOBS: usize = 10;
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize)]
pub struct InstallJobSnapshot {
    pub id: String,
    pub tool: String,
    pub status: &'static str,
    pub summary: String,
    pub log: String,
    pub started_at_unix_seconds: u64,
    pub completed_at_unix_seconds: Option<u64>,
    pub exit_code: Option<i32>,
    pub retryable: bool,
}

#[derive(Clone)]
struct InstallJob {
    snapshot: InstallJobSnapshot,
    cancelled: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
pub struct InstallerState {
    jobs: Arc<Mutex<BTreeMap<String, InstallJob>>>,
}

struct CommandPlan {
    program: &'static str,
    args: Vec<&'static str>,
    summary: &'static str,
}

impl InstallerState {
    pub fn start(&self, tool: &str, approved: bool) -> Result<InstallJobSnapshot, String> {
        if !approved {
            return Err("installation requires explicit approval".into());
        }
        let plan = install_plan(tool)?;
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| "installer state is unavailable".to_string())?;
        if jobs.values().any(|job| job.snapshot.status == "running") {
            return Err("another installation is already running".into());
        }
        while jobs.len() >= MAX_JOBS {
            let completed = jobs
                .iter()
                .find(|(_, job)| job.snapshot.status != "running")
                .map(|(id, _)| id.clone());
            if let Some(id) = completed {
                jobs.remove(&id);
            } else {
                break;
            }
        }
        let started_at = now();
        let id = format!(
            "install-{started_at}-{}",
            NEXT_JOB.fetch_add(1, Ordering::Relaxed)
        );
        let cancelled = Arc::new(AtomicBool::new(false));
        let snapshot = InstallJobSnapshot {
            id: id.clone(),
            tool: tool.into(),
            status: "running",
            summary: plan.summary.into(),
            log: String::new(),
            started_at_unix_seconds: started_at,
            completed_at_unix_seconds: None,
            exit_code: None,
            retryable: false,
        };
        jobs.insert(
            id.clone(),
            InstallJob {
                snapshot: snapshot.clone(),
                cancelled: cancelled.clone(),
            },
        );
        drop(jobs);

        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = run_plan(plan, cancelled.clone()).await;
            state.complete(&id, result, cancelled.load(Ordering::SeqCst));
        });
        audit(&snapshot, "started");
        Ok(snapshot)
    }

    pub fn status(&self, id: &str) -> Result<InstallJobSnapshot, String> {
        validate_job_id(id)?;
        self.jobs
            .lock()
            .map_err(|_| "installer state is unavailable".to_string())?
            .get(id)
            .map(|job| job.snapshot.clone())
            .ok_or_else(|| "installation job was not found".into())
    }

    pub fn cancel(&self, id: &str) -> Result<InstallJobSnapshot, String> {
        validate_job_id(id)?;
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| "installer state is unavailable".to_string())?;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| "installation job was not found".to_string())?;
        if job.snapshot.status != "running" {
            return Ok(job.snapshot.clone());
        }
        job.cancelled.store(true, Ordering::SeqCst);
        job.snapshot.status = "cancelling";
        Ok(job.snapshot.clone())
    }

    fn complete(&self, id: &str, result: Result<(Option<i32>, String), String>, cancelled: bool) {
        let Ok(mut jobs) = self.jobs.lock() else {
            return;
        };
        let Some(job) = jobs.get_mut(id) else {
            return;
        };
        job.snapshot.completed_at_unix_seconds = Some(now());
        match result {
            Ok((exit_code, log)) => {
                job.snapshot.exit_code = exit_code;
                job.snapshot.log = log;
                job.snapshot.status = if cancelled {
                    "cancelled"
                } else if exit_code == Some(0) {
                    "succeeded"
                } else {
                    "failed"
                };
            }
            Err(error) => {
                job.snapshot.log = sanitize_log(&error);
                job.snapshot.status = if cancelled { "cancelled" } else { "failed" };
            }
        }
        job.snapshot.retryable = matches!(job.snapshot.status, "failed" | "cancelled");
        audit(&job.snapshot, "completed");
    }
}

async fn run_plan(
    plan: CommandPlan,
    cancelled: Arc<AtomicBool>,
) -> Result<(Option<i32>, String), String> {
    let mut child = Command::new(plan.program)
        .args(plan.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("start {}: {error}", plan.program))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "installer stdout is unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "installer stderr is unavailable".to_string())?;
    let stdout_task = tauri::async_runtime::spawn(read_bounded(stdout));
    let stderr_task = tauri::async_runtime::spawn(read_bounded(stderr));
    let status = loop {
        if cancelled.load(Ordering::SeqCst) {
            child
                .kill()
                .await
                .map_err(|error| format!("cancel installer: {error}"))?;
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("wait for installer: {error}"))?
        {
            break status;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let stdout = stdout_task
        .await
        .map_err(|error| format!("collect installer output: {error}"))??;
    let stderr = stderr_task
        .await
        .map_err(|error| format!("collect installer errors: {error}"))??;
    let log = sanitize_log(&format!(
        "{}{}{}",
        String::from_utf8_lossy(&stdout),
        if stdout.is_empty() || stderr.is_empty() {
            ""
        } else {
            "\n"
        },
        String::from_utf8_lossy(&stderr)
    ));
    Ok((status.code(), log))
}

async fn read_bounded<R: tokio::io::AsyncRead + Unpin>(reader: R) -> Result<Vec<u8>, String> {
    let mut reader = reader;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = reader
            .read(&mut chunk)
            .await
            .map_err(|error| format!("read installer log: {error}"))?;
        if read == 0 {
            break;
        }
        let remaining = MAX_LOG_BYTES.saturating_sub(bytes.len() as u64) as usize;
        bytes.extend_from_slice(&chunk[..read.min(remaining)]);
    }
    Ok(bytes)
}

fn install_plan(tool: &str) -> Result<CommandPlan, String> {
    match (tool, std::env::consts::OS) {
        ("uv", "macos") => Ok(CommandPlan {
            program: "brew",
            args: vec!["install", "uv"],
            summary: "Install uv from Homebrew core",
        }),
        ("uv", "windows") => Ok(CommandPlan {
            program: "winget",
            args: vec!["install", "--id=astral-sh.uv", "-e"],
            summary: "Install uv with WinGet",
        }),
        ("uv", "linux") => Ok(CommandPlan {
            program: "sh",
            args: vec![
                "-c",
                "curl --proto '=https' --tlsv1.2 -LsSf https://astral.sh/uv/install.sh | sh",
            ],
            summary: "Install uv with Astral's HTTPS installer",
        }),
        ("python", _) => Ok(CommandPlan {
            program: "uv",
            args: vec!["python", "install", "3.11"],
            summary: "Install an isolated Python 3.11 runtime with uv",
        }),
        ("cortana", _) | ("connectors", _) => Err(
            "Cortana and connector installation require the signed bundled runtime and are not downloaded independently"
                .into(),
        ),
        _ => Err("that tool has no supported installer".into()),
    }
}

fn validate_job_id(id: &str) -> Result<(), String> {
    if id.len() > 96
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("invalid installation job id".into());
    }
    Ok(())
}

fn sanitize_log(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '\0' | '\u{1b}'))
        .take(MAX_LOG_BYTES as usize)
        .collect()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn audit(snapshot: &InstallJobSnapshot, phase: &str) {
    let event = serde_json::json!({
        "at_unix_seconds": now(),
        "event": format!("installer.{phase}"),
        "job_id": snapshot.id,
        "tool": snapshot.tool,
        "status": snapshot.status,
        "exit_code": snapshot.exit_code,
        "log_recorded": false,
    });
    let _ = crate::settings::append_audit_event(&crate::settings::default_config_path(), &event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_requires_approval_and_rejects_unknown_tools() {
        let state = InstallerState::default();
        assert!(state.start("uv", false).is_err());
        assert!(state.start("anything", true).is_err());
    }

    #[test]
    fn logs_remove_escape_and_nul_sequences() {
        assert_eq!(sanitize_log("safe\u{1b}[31m\0text"), "safe[31mtext");
    }

    #[test]
    fn job_ids_are_narrowly_validated() {
        assert!(validate_job_id("install-123-1").is_ok());
        assert!(validate_job_id("../other").is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fixed_job_runner_captures_output_and_honors_cancellation() {
        let completed = run_plan(
            CommandPlan {
                program: "sh",
                args: vec!["-c", "printf ready"],
                summary: "test",
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .expect("completed job");
        assert_eq!(completed.0, Some(0));
        assert_eq!(completed.1, "ready");

        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = cancelled.clone();
        let task = tokio::spawn(async move {
            run_plan(
                CommandPlan {
                    program: "sh",
                    args: vec!["-c", "sleep 5"],
                    summary: "test",
                },
                cancellation,
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancelled.store(true, Ordering::SeqCst);
        let result = task.await.expect("cancel task").expect("cancelled job");
        assert_ne!(result.0, Some(0));
    }
}
