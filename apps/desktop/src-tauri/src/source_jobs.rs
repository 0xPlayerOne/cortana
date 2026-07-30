use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_shell::{
    ShellExt,
    process::{CommandChild, CommandEvent},
};

use crate::settings;

const MAX_LOG_BYTES: usize = 64 * 1024;
const MAX_JOBS: usize = 20;
const VALIDATION_MAX_DOCUMENTS: &str = "25";
const VALIDATION_MAX_BYTES: &str = "5242880";
const VALIDATION_MAX_SECONDS: &str = "60";
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize)]
pub struct SourceJobSnapshot {
    pub id: String,
    pub source: String,
    pub kind: String,
    pub project: String,
    pub status: &'static str,
    pub summary: String,
    pub log: String,
    pub started_at_unix_seconds: u64,
    pub completed_at_unix_seconds: Option<u64>,
    pub exit_code: Option<i32>,
    pub retryable: bool,
    pub writes_indexed_data: bool,
}

struct SourceJob {
    snapshot: SourceJobSnapshot,
    child: Option<CommandChild>,
}

#[derive(Clone, Default)]
pub struct SourceJobState {
    jobs: Arc<Mutex<BTreeMap<String, SourceJob>>>,
}

impl SourceJobState {
    pub fn start(&self, app: &AppHandle, source_name: &str) -> Result<SourceJobSnapshot, String> {
        let source = settings::configured_source(source_name)?;
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| "source validation state is unavailable".to_string())?;
        if jobs
            .values()
            .any(|job| matches!(job.snapshot.status, "running" | "cancelling"))
        {
            return Err("another source validation is already running".into());
        }
        prune_jobs(&mut jobs);

        let command = app
            .shell()
            .sidecar("cortana")
            .map_err(|error| format!("locate bundled Cortana runtime: {error}"))?
            .args(validation_args(&source.name));
        let (mut receiver, child) = command
            .spawn()
            .map_err(|error| format!("start source validation: {error}"))?;
        let started_at = now();
        let id = format!(
            "source-{started_at}-{}",
            NEXT_JOB.fetch_add(1, Ordering::Relaxed)
        );
        let snapshot = SourceJobSnapshot {
            id: id.clone(),
            source: source.name,
            kind: source.kind,
            project: source.project,
            status: "running",
            summary: "Read-only connector validation is running with a 25 document, 5 MiB, 60 second limit.".into(),
            log: String::new(),
            started_at_unix_seconds: started_at,
            completed_at_unix_seconds: None,
            exit_code: None,
            retryable: false,
            writes_indexed_data: false,
        };
        jobs.insert(
            id.clone(),
            SourceJob {
                snapshot: snapshot.clone(),
                child: Some(child),
            },
        );
        drop(jobs);
        audit(&snapshot, "started");

        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = receiver.recv().await {
                let terminal = matches!(event, CommandEvent::Terminated(_));
                state.handle_event(&id, event);
                if terminal {
                    break;
                }
            }
            state.finish_disconnected(&id);
        });
        Ok(snapshot)
    }

    pub fn status(&self, id: &str) -> Result<SourceJobSnapshot, String> {
        validate_job_id(id)?;
        self.jobs
            .lock()
            .map_err(|_| "source validation state is unavailable".to_string())?
            .get(id)
            .map(|job| job.snapshot.clone())
            .ok_or_else(|| "source validation job was not found".into())
    }

    pub fn cancel(&self, id: &str) -> Result<SourceJobSnapshot, String> {
        validate_job_id(id)?;
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| "source validation state is unavailable".to_string())?;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| "source validation job was not found".to_string())?;
        if job.snapshot.status != "running" {
            return Ok(job.snapshot.clone());
        }
        job.snapshot.status = "cancelling";
        job.snapshot.summary = "Cancelling read-only source validation…".into();
        if let Some(child) = job.child.take() {
            if let Err(error) = child.kill() {
                job.snapshot.status = "failed";
                job.snapshot.summary = "Source validation could not be cancelled.".into();
                job.snapshot.log = sanitize_log(&error.to_string());
                job.snapshot.completed_at_unix_seconds = Some(now());
                job.snapshot.retryable = true;
            }
        }
        Ok(job.snapshot.clone())
    }

    fn handle_event(&self, id: &str, event: CommandEvent) {
        let Ok(mut jobs) = self.jobs.lock() else {
            return;
        };
        let Some(job) = jobs.get_mut(id) else {
            return;
        };
        match event {
            CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                append_bounded_log(&mut job.snapshot.log, &bytes);
            }
            CommandEvent::Error(error) => {
                append_bounded_log(&mut job.snapshot.log, error.as_bytes());
            }
            CommandEvent::Terminated(payload) => {
                job.child = None;
                job.snapshot.exit_code = payload.code;
                job.snapshot.completed_at_unix_seconds = Some(now());
                job.snapshot.status = if job.snapshot.status == "cancelling" {
                    "cancelled"
                } else if payload.code == Some(0) {
                    "succeeded"
                } else {
                    "failed"
                };
                job.snapshot.summary = match job.snapshot.status {
                    "succeeded" => "Source validation passed. No documents were indexed.".into(),
                    "cancelled" => {
                        "Source validation was cancelled. No documents were indexed.".into()
                    }
                    _ => "Source validation failed. No documents were indexed.".into(),
                };
                job.snapshot.retryable = matches!(job.snapshot.status, "failed" | "cancelled");
                audit(&job.snapshot, "completed");
            }
            _ => {}
        }
    }

    fn finish_disconnected(&self, id: &str) {
        let Ok(mut jobs) = self.jobs.lock() else {
            return;
        };
        let Some(job) = jobs.get_mut(id) else {
            return;
        };
        if !matches!(job.snapshot.status, "running" | "cancelling") {
            return;
        }
        job.child = None;
        job.snapshot.completed_at_unix_seconds = Some(now());
        job.snapshot.status = if job.snapshot.status == "cancelling" {
            "cancelled"
        } else {
            "failed"
        };
        job.snapshot.summary = if job.snapshot.status == "cancelled" {
            "Source validation was cancelled. No documents were indexed.".into()
        } else {
            "Source validation ended without a process result. No documents were indexed.".into()
        };
        job.snapshot.retryable = true;
        audit(&job.snapshot, "completed");
    }
}

fn prune_jobs(jobs: &mut BTreeMap<String, SourceJob>) {
    while jobs.len() >= MAX_JOBS {
        let completed = jobs
            .iter()
            .find(|(_, job)| !matches!(job.snapshot.status, "running" | "cancelling"))
            .map(|(id, _)| id.clone());
        if let Some(id) = completed {
            jobs.remove(&id);
        } else {
            break;
        }
    }
}

fn validation_args(source: &str) -> Vec<String> {
    [
        "validate-source",
        source,
        "--max-documents",
        VALIDATION_MAX_DOCUMENTS,
        "--max-bytes",
        VALIDATION_MAX_BYTES,
        "--max-seconds",
        VALIDATION_MAX_SECONDS,
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn validate_job_id(id: &str) -> Result<(), String> {
    if id.len() > 96
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("source validation job id is invalid".into());
    }
    Ok(())
}

fn append_bounded_log(log: &mut String, bytes: &[u8]) {
    let line = sanitize_log(&String::from_utf8_lossy(bytes));
    if line.is_empty() || log.len() >= MAX_LOG_BYTES {
        return;
    }
    if !log.is_empty() {
        log.push('\n');
    }
    let remaining = MAX_LOG_BYTES.saturating_sub(log.len());
    let mut end = line.len().min(remaining);
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    log.push_str(&line[..end]);
}

fn sanitize_log(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            *character == '\n'
                || *character == '\t'
                || (!character.is_control() && *character != '\u{1b}')
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn audit(snapshot: &SourceJobSnapshot, phase: &str) {
    let event = serde_json::json!({
        "at_unix_seconds": now(),
        "event": format!("source.validation.{phase}"),
        "job_id": snapshot.id,
        "source": snapshot.source,
        "kind": snapshot.kind,
        "project": snapshot.project,
        "status": snapshot.status,
        "exit_code": snapshot.exit_code,
        "writes_indexed_data": false,
        "source_content_recorded": false,
        "secret_values_recorded": false,
    });
    let _ = settings::append_audit_event(&settings::default_config_path(), &event);
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_ids_are_narrowly_validated() {
        assert!(validate_job_id("source-123-4").is_ok());
        assert!(validate_job_id("../source").is_err());
        assert!(validate_job_id(&"x".repeat(97)).is_err());
    }

    #[test]
    fn logs_are_sanitized_and_bounded() {
        let mut log = String::new();
        append_bounded_log(&mut log, b"hello\x1b[31m\0world");
        assert_eq!(log, "hello[31mworld");
        append_bounded_log(&mut log, &vec![b'x'; MAX_LOG_BYTES * 2]);
        assert!(log.len() <= MAX_LOG_BYTES);
    }

    #[test]
    fn validation_command_has_fixed_read_only_limits() {
        assert_eq!(
            validation_args("personal-drive"),
            [
                "validate-source",
                "personal-drive",
                "--max-documents",
                "25",
                "--max-bytes",
                "5242880",
                "--max-seconds",
                "60",
            ]
        );
    }
}
