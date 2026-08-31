use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_shell::{
    ShellExt,
    process::{CommandChild, CommandEvent},
};

use crate::{paths, settings};

const MAX_JOBS: usize = 12;
const MAX_OUTPUT_BYTES: usize = 512 * 1024;
const MAX_WORKSPACES: usize = 128;
const MAX_WORKSPACE_CHARS: usize = 256;
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VaultExportReport {
    pub output: PathBuf,
    pub workspaces: Vec<String>,
    pub documents: usize,
    pub content_rewrites: usize,
    pub unchanged_documents: usize,
    pub deleted_documents: usize,
    pub dry_run: bool,
    pub files: Vec<PathBuf>,
    pub files_truncated: bool,
    pub previous_vault: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
struct VaultExportProgress {
    phase: String,
    documents_completed: usize,
    files_written: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct VaultExportSnapshot {
    pub id: String,
    pub status: String,
    pub phase: String,
    pub workspaces: Vec<String>,
    pub output: String,
    pub dry_run: bool,
    pub documents_completed: usize,
    pub files_written: usize,
    pub started_at_unix_seconds: u64,
    pub completed_at_unix_seconds: Option<u64>,
    pub report: Option<VaultExportReport>,
    pub error: Option<String>,
}

struct VaultExportJob {
    snapshot: VaultExportSnapshot,
    child: Option<CommandChild>,
    stdout: Vec<u8>,
    stderr_pending: Vec<u8>,
}

#[derive(Clone, Default)]
pub struct VaultExportState {
    jobs: Arc<Mutex<BTreeMap<String, VaultExportJob>>>,
}

impl VaultExportState {
    pub async fn start(
        &self,
        app: &AppHandle,
        workspaces: Vec<String>,
        dry_run: bool,
        approved: bool,
    ) -> Result<Option<VaultExportSnapshot>, String> {
        if !dry_run && !approved {
            return Err("vault export requires explicit approval".into());
        }
        let workspaces = validate_workspaces(workspaces)?;
        let Some(output) = paths::pick(app.clone(), "vault-export").await? else {
            return Ok(None);
        };
        let config_path = PathBuf::from(settings::load()?.config_path);
        let mut args = vec![
            "--config".into(),
            config_path.display().to_string(),
            "export-vault".into(),
            output.clone(),
        ];
        for workspace in &workspaces {
            args.push("--workspace".into());
            args.push(workspace.clone());
        }
        if dry_run {
            args.push("--dry-run".into());
        }

        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| "vault export state is unavailable".to_string())?;
        if jobs
            .values()
            .any(|job| matches!(job.snapshot.status.as_str(), "running" | "cancelling"))
        {
            return Err("another vault export is already running".into());
        }
        prune_jobs(&mut jobs);
        let command = app
            .shell()
            .sidecar("cortana")
            .map_err(|error| format!("locate bundled Cortana runtime: {error}"))?
            .args(args)
            .env("CORTANA_DESKTOP_PROCESS_GROUP", "1")
            .set_raw_out(true);
        let (mut receiver, child) = command
            .spawn()
            .map_err(|error| format!("start vault export: {error}"))?;
        let started = now();
        let id = format!(
            "vault-{started}-{}",
            NEXT_JOB.fetch_add(1, Ordering::Relaxed)
        );
        let snapshot = VaultExportSnapshot {
            id: id.clone(),
            status: "running".into(),
            phase: "starting".into(),
            workspaces,
            output,
            dry_run,
            documents_completed: 0,
            files_written: 0,
            started_at_unix_seconds: started,
            completed_at_unix_seconds: None,
            report: None,
            error: None,
        };
        jobs.insert(
            id.clone(),
            VaultExportJob {
                snapshot: snapshot.clone(),
                child: Some(child),
                stdout: Vec::new(),
                stderr_pending: Vec::new(),
            },
        );
        drop(jobs);

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
        Ok(Some(snapshot))
    }

    pub fn status(&self, id: &str) -> Result<VaultExportSnapshot, String> {
        validate_job_id(id)?;
        self.jobs
            .lock()
            .map_err(|_| "vault export state is unavailable".to_string())?
            .get(id)
            .map(|job| job.snapshot.clone())
            .ok_or_else(|| "vault export job was not found".into())
    }

    pub fn cancel(&self, id: &str) -> Result<VaultExportSnapshot, String> {
        validate_job_id(id)?;
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| "vault export state is unavailable".to_string())?;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| "vault export job was not found".to_string())?;
        if job.snapshot.status != "running" {
            return Ok(job.snapshot.clone());
        }
        job.snapshot.status = "cancelling".into();
        job.snapshot.phase = "cancelling".into();
        request_cancellation(job)?;
        Ok(job.snapshot.clone())
    }

    fn handle_event(&self, id: &str, event: CommandEvent) {
        let Ok(mut jobs) = self.jobs.lock() else {
            return;
        };
        let Some(job) = jobs.get_mut(id) else { return };
        match event {
            CommandEvent::Stdout(bytes) => append_bounded(&mut job.stdout, &bytes),
            CommandEvent::Stderr(bytes) => {
                append_bounded(&mut job.stderr_pending, &bytes);
                update_progress(job);
            }
            CommandEvent::Error(error) => {
                job.snapshot.error = Some(bounded_text(error.as_bytes()));
            }
            CommandEvent::Terminated(payload) => {
                job.child = None;
                job.snapshot.completed_at_unix_seconds = Some(now());
                if job.snapshot.status == "cancelling" {
                    job.snapshot.status = "cancelled".into();
                    job.snapshot.phase = "cancelled".into();
                } else if payload.code == Some(0) {
                    match serde_json::from_slice::<VaultExportReport>(&job.stdout) {
                        Ok(report) if valid_report(&job.snapshot, &report) => {
                            job.snapshot.documents_completed = report.documents;
                            job.snapshot.files_written = report.documents;
                            job.snapshot.phase = "complete".into();
                            job.snapshot.status = "succeeded".into();
                            job.snapshot.report = Some(report);
                        }
                        Ok(_) | Err(_) => {
                            job.snapshot.status = "failed".into();
                            job.snapshot.phase = "failed".into();
                            job.snapshot.error =
                                Some("vault export returned invalid output".into());
                        }
                    }
                } else {
                    job.snapshot.status = "failed".into();
                    job.snapshot.phase = "failed".into();
                    job.snapshot.error = Some(
                        job.snapshot
                            .error
                            .clone()
                            .unwrap_or_else(|| last_stderr_line(&job.stderr_pending)),
                    );
                }
            }
            _ => {}
        }
    }

    fn finish_disconnected(&self, id: &str) {
        let Ok(mut jobs) = self.jobs.lock() else {
            return;
        };
        let Some(job) = jobs.get_mut(id) else { return };
        if !matches!(job.snapshot.status.as_str(), "running" | "cancelling") {
            return;
        }
        job.child = None;
        job.snapshot.status = "failed".into();
        job.snapshot.phase = "failed".into();
        job.snapshot.completed_at_unix_seconds = Some(now());
        job.snapshot.error = Some("vault export process ended without a result".into());
    }
}

fn validate_workspaces(workspaces: Vec<String>) -> Result<Vec<String>, String> {
    if workspaces.is_empty() || workspaces.len() > MAX_WORKSPACES {
        return Err(format!("select between 1 and {MAX_WORKSPACES} workspaces"));
    }
    let mut unique = BTreeSet::new();
    for workspace in &workspaces {
        if workspace.is_empty()
            || workspace != workspace.trim()
            || workspace.chars().count() > MAX_WORKSPACE_CHARS
            || workspace.chars().any(char::is_control)
            || !unique.insert(workspace.clone())
        {
            return Err("workspace selections must be unique, bounded, trimmed names".into());
        }
    }
    Ok(unique.into_iter().collect())
}

fn validate_job_id(id: &str) -> Result<(), String> {
    if id.len() > 96
        || !id.starts_with("vault-")
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("invalid vault export job id".into());
    }
    Ok(())
}

fn update_progress(job: &mut VaultExportJob) {
    while let Some(end) = job.stderr_pending.iter().position(|byte| *byte == b'\n') {
        let line = job.stderr_pending.drain(..=end).collect::<Vec<_>>();
        if let Ok(progress) = serde_json::from_slice::<VaultExportProgress>(&line) {
            job.snapshot.phase = progress.phase;
            job.snapshot.documents_completed = progress.documents_completed;
            job.snapshot.files_written = progress.files_written;
        } else if !line.iter().all(u8::is_ascii_whitespace) {
            job.snapshot.error = Some(bounded_text(&line));
        }
    }
}

fn request_cancellation(job: &mut VaultExportJob) -> Result<(), String> {
    #[cfg(unix)]
    {
        let child = job
            .child
            .as_ref()
            .ok_or_else(|| "vault export process is no longer running".to_string())?;
        let pid = child.pid();
        if pid > 0 && pid <= i32::MAX as u32 {
            let result = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGTERM) };
            if result == 0 {
                return Ok(());
            }
        }
    }
    let child = job
        .child
        .take()
        .ok_or_else(|| "vault export process is no longer running".to_string())?;
    child
        .kill()
        .map_err(|error| format!("cancel vault export: {error}"))
}

fn valid_report(snapshot: &VaultExportSnapshot, report: &VaultExportReport) -> bool {
    report.output == PathBuf::from(&snapshot.output)
        && report.workspaces == snapshot.workspaces
        && report.dry_run == snapshot.dry_run
        && report.files.len() <= 100
        && report.documents >= report.files.len()
}

fn append_bounded(buffer: &mut Vec<u8>, bytes: &[u8]) {
    let remaining = MAX_OUTPUT_BYTES.saturating_sub(buffer.len());
    buffer.extend_from_slice(&bytes[..bytes.len().min(remaining)]);
}

fn bounded_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(2048)])
        .chars()
        .filter(|character| *character == '\n' || *character == '\t' || !character.is_control())
        .collect::<String>()
}

fn last_stderr_line(bytes: &[u8]) -> String {
    let value = bounded_text(bytes);
    value
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("vault export failed")
        .trim()
        .to_string()
}

fn prune_jobs(jobs: &mut BTreeMap<String, VaultExportJob>) {
    while jobs.len() >= MAX_JOBS {
        let Some(id) = jobs
            .iter()
            .filter(|(_, job)| !matches!(job.snapshot.status.as_str(), "running" | "cancelling"))
            .min_by_key(|(_, job)| job.snapshot.started_at_unix_seconds)
            .map(|(id, _)| id.clone())
        else {
            break;
        };
        jobs.remove(&id);
    }
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
    fn workspace_and_job_identifiers_are_bounded() {
        assert_eq!(
            validate_workspaces(vec!["work".into(), "personal".into()]).unwrap(),
            ["personal", "work"]
        );
        assert!(validate_workspaces(Vec::new()).is_err());
        assert!(validate_workspaces(vec!["work".into(), "work".into()]).is_err());
        assert!(validate_workspaces(vec![" bad".into()]).is_err());
        assert!(validate_job_id("vault-123-1").is_ok());
        assert!(validate_job_id("../vault-1").is_err());
    }

    #[test]
    fn progress_parser_accepts_only_bounded_json_lines() {
        let mut job = VaultExportJob {
            snapshot: VaultExportSnapshot {
                id: "vault-1-1".into(),
                status: "running".into(),
                phase: "starting".into(),
                workspaces: vec!["work".into()],
                output: "/tmp/vault".into(),
                dry_run: false,
                documents_completed: 0,
                files_written: 0,
                started_at_unix_seconds: 1,
                completed_at_unix_seconds: None,
                report: None,
                error: None,
            },
            child: None,
            stdout: Vec::new(),
            stderr_pending: br#"not-json
{"phase":"writing","documents_completed":25,"files_written":4}
"#
            .to_vec(),
        };
        update_progress(&mut job);
        assert_eq!(job.snapshot.phase, "writing");
        assert_eq!(job.snapshot.documents_completed, 25);
        assert_eq!(job.snapshot.files_written, 4);
    }
}
