use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
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
const TRIAL_SYNC_MAX_SECONDS: &str = "300";
const MAX_PENDING_PLANS: usize = 8;
const PLAN_TTL_SECONDS: u64 = 600;
const MAX_VALIDATION_STATE_BYTES: u64 = 1024 * 1024;
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);

/// Fixed Desktop initial-sync budgets. The webview can select only one of
/// these tiers; every value below is resolved natively and the matching CLI
/// limits are constructed here, never from renderer input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InitialSyncBudget {
    Small,
    Medium,
    Large,
}

impl InitialSyncBudget {
    pub fn limits(self) -> (usize, u64, u64) {
        match self {
            InitialSyncBudget::Small => (100, 25 * 1024 * 1024, 15 * 60),
            InitialSyncBudget::Medium => (500, 64 * 1024 * 1024, 30 * 60),
            InitialSyncBudget::Large => (2_000, 128 * 1024 * 1024, 60 * 60),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            InitialSyncBudget::Small => "small",
            InitialSyncBudget::Medium => "medium",
            InitialSyncBudget::Large => "large",
        }
    }
}

/// Plan-only requests are read-only and need no approval; execution requires
/// an explicit approval and a plan id issued by a prior plan request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InitialSyncOperation {
    Plan,
    Execute,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "lowercase")]
pub enum InitialSyncOutcome {
    Plan(InitialSyncPlan),
    Job(SourceJobSnapshot),
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceJobSnapshot {
    pub id: String,
    pub operation: &'static str,
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
    pub budget: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SetupOpenOutcome {
    pub source: String,
    pub kind: String,
    pub url: &'static str,
    pub opened: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitialSyncPlan {
    pub source: String,
    pub kind: String,
    pub project: String,
    pub enabled: bool,
    pub budget: InitialSyncBudget,
    pub budget_documents: usize,
    pub budget_bytes: u64,
    pub budget_seconds: u64,
    pub writes_indexed_data: bool,
    pub requires_validation: bool,
    pub validation_covers_budget: Option<bool>,
    pub plan_id: String,
}

struct SourceJob {
    snapshot: SourceJobSnapshot,
    child: Option<CommandChild>,
}

struct PendingPlan {
    source: String,
    budget: InitialSyncBudget,
    created_at: u64,
}

#[derive(Clone, Default)]
pub struct SourceJobState {
    jobs: Arc<Mutex<BTreeMap<String, SourceJob>>>,
    plans: Arc<Mutex<BTreeMap<String, PendingPlan>>>,
}

impl SourceJobState {
    pub fn start_validation(
        &self,
        app: &AppHandle,
        source_name: &str,
        budget: Option<InitialSyncBudget>,
    ) -> Result<SourceJobSnapshot, String> {
        let source = settings::configured_source(source_name)?;
        let (args, summary) = match budget {
            Some(budget) => {
                let (documents, bytes, seconds) = budget.limits();
                (
                    validation_args_with(source_name, documents, bytes, seconds),
                    format!(
                        "Read-only connector validation is running with a {documents} document, {} MiB, {} minute limit.",
                        bytes / (1024 * 1024),
                        seconds / 60
                    ),
                )
            }
            None => (
                validation_args(source_name),
                "Read-only connector validation is running with a 25 document, 5 MiB, 60 second limit.".into(),
            ),
        };
        self.start(app, source, "validation", args, summary, None, false)
    }

    pub fn plan_initial_sync(
        &self,
        source_name: &str,
        budget: InitialSyncBudget,
    ) -> Result<InitialSyncPlan, String> {
        let source = settings::configured_source(source_name)?;
        let data_dir = settings::load()?.runtime.data_dir;
        self.build_initial_sync_plan(&source, budget, Path::new(&data_dir))
    }

    pub fn execute_initial_sync(
        &self,
        app: &AppHandle,
        source_name: &str,
        budget: InitialSyncBudget,
        plan_id: &str,
        approved: bool,
    ) -> Result<SourceJobSnapshot, String> {
        let source = settings::configured_source(source_name)?;
        let data_dir = settings::load()?.runtime.data_dir;
        self.confirm_initial_sync_execution(
            &source,
            budget,
            plan_id,
            approved,
            Path::new(&data_dir),
        )?;
        let snapshot = self.start(
            app,
            source,
            "initial-sync",
            initial_sync_args(source_name, budget),
            initial_sync_summary(budget),
            Some(budget),
            true,
        )?;
        // The plan is one-shot only for executions that actually start; a
        // transient spawn failure must not burn it.
        self.consume_initial_sync_plan(plan_id);
        Ok(snapshot)
    }

    fn build_initial_sync_plan(
        &self,
        source: &settings::SourceSettings,
        budget: InitialSyncBudget,
        data_dir: &Path,
    ) -> Result<InitialSyncPlan, String> {
        let (budget_documents, budget_bytes, budget_seconds) = budget.limits();
        let validation_covers_budget = validation_covers_budget_at(data_dir, &source.name, budget)?;
        let plan_id = format!(
            "plan-{}-{}",
            now(),
            NEXT_JOB.fetch_add(1, Ordering::Relaxed)
        );
        let mut plans = self
            .plans
            .lock()
            .map_err(|_| "source plan state is unavailable".to_string())?;
        prune_plans(&mut plans);
        plans.insert(
            plan_id.clone(),
            PendingPlan {
                source: source.name.clone(),
                budget,
                created_at: now(),
            },
        );
        drop(plans);
        let event = serde_json::json!({
            "at_unix_seconds": now(),
            "event": "source.initial-sync-plan.requested",
            "source": &source.name,
            "kind": &source.kind,
            "project": &source.project,
            "budget": budget.as_str(),
            "budget_documents": budget_documents,
            "budget_bytes": budget_bytes,
            "budget_seconds": budget_seconds,
            "writes_indexed_data": true,
            "source_content_recorded": false,
            "secret_values_recorded": false,
        });
        let _ = settings::append_audit_event(&settings::default_config_path(), &event);
        Ok(InitialSyncPlan {
            source: source.name.clone(),
            kind: source.kind.clone(),
            project: source.project.clone(),
            enabled: source.enabled,
            budget,
            budget_documents,
            budget_bytes,
            budget_seconds,
            writes_indexed_data: true,
            requires_validation: true,
            validation_covers_budget,
            plan_id,
        })
    }

    fn confirm_initial_sync_execution(
        &self,
        source: &settings::SourceSettings,
        budget: InitialSyncBudget,
        plan_id: &str,
        approved: bool,
        data_dir: &Path,
    ) -> Result<(), String> {
        if !approved {
            return Err("initial sync requires explicit plan confirmation".into());
        }
        if !source.enabled {
            return Err("save and enable this source before an initial sync".into());
        }
        validate_plan_id(plan_id)?;
        match validation_covers_budget_at(data_dir, &source.name, budget)? {
            Some(true) => {}
            _ => {
                return Err(
                    "initial sync requires a successful validation at equal or larger limits; validate with this budget first"
                        .into(),
                );
            }
        }
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| "source job state is unavailable".to_string())?;
        if jobs
            .values()
            .any(|job| matches!(job.snapshot.status, "running" | "cancelling"))
        {
            return Err("another source operation is already running".into());
        }
        drop(jobs);
        let mut plans = self
            .plans
            .lock()
            .map_err(|_| "source plan state is unavailable".to_string())?;
        prune_plans(&mut plans);
        let pending = plans.get(plan_id).ok_or_else(|| {
            "initial sync plan was not found or has expired; request a new plan".to_string()
        })?;
        if pending.source != source.name || pending.budget != budget {
            return Err("initial sync plan does not match this source and budget".into());
        }
        Ok(())
    }

    fn consume_initial_sync_plan(&self, plan_id: &str) {
        if let Ok(mut plans) = self.plans.lock() {
            plans.remove(plan_id);
        }
    }

    pub fn start_authorization(
        &self,
        app: &AppHandle,
        source_name: &str,
    ) -> Result<SourceJobSnapshot, String> {
        let source = settings::configured_source(source_name)?;
        if !matches!(
            source.kind.as_str(),
            "google-drive" | "gmail" | "google-calendar"
        ) {
            return Err("browser authorization is available only for Google sources".into());
        }
        if source.token_path.is_none() || source.oauth_client_path.is_none() {
            return Err(
                "save both the Google token destination and Desktop OAuth client paths first"
                    .into(),
            );
        }
        self.start(
            app,
            source,
            "authorization",
            authorization_args(source_name),
            "Waiting for Google authorization in the system browser. No source data is being read."
                .into(),
            None,
            false,
        )
    }

    pub fn start_trial_sync(
        &self,
        app: &AppHandle,
        source_name: &str,
        approved: bool,
    ) -> Result<SourceJobSnapshot, String> {
        if !approved {
            return Err("trial sync requires explicit approval".into());
        }
        let source = settings::configured_source(source_name)?;
        if !source.enabled {
            return Err("save and enable this source before a trial sync".into());
        }
        self.start(
            app,
            source,
            "trial-sync",
            trial_sync_args(source_name),
            "Guarded trial sync may index up to 25 documents or 5 MiB for at most 5 minutes. Reconciliation is disabled.".into(),
            None,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn start(
        &self,
        app: &AppHandle,
        source: settings::SourceSettings,
        operation: &'static str,
        args: Vec<String>,
        summary: String,
        budget: Option<InitialSyncBudget>,
        writes_indexed_data: bool,
    ) -> Result<SourceJobSnapshot, String> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| "source job state is unavailable".to_string())?;
        if jobs
            .values()
            .any(|job| matches!(job.snapshot.status, "running" | "cancelling"))
        {
            return Err("another source operation is already running".into());
        }
        prune_jobs(&mut jobs);

        let command = app
            .shell()
            .sidecar("cortana")
            .map_err(|error| format!("locate bundled Cortana runtime: {error}"))?
            .args(args);
        let (mut receiver, child) = command
            .spawn()
            .map_err(|error| format!("start source {operation}: {error}"))?;
        let started_at = now();
        let id = format!(
            "source-{started_at}-{}",
            NEXT_JOB.fetch_add(1, Ordering::Relaxed)
        );
        let snapshot = SourceJobSnapshot {
            id: id.clone(),
            operation,
            source: source.name,
            kind: source.kind,
            project: source.project,
            status: "running",
            summary,
            log: String::new(),
            started_at_unix_seconds: started_at,
            completed_at_unix_seconds: None,
            exit_code: None,
            retryable: false,
            writes_indexed_data,
            budget: budget.map(InitialSyncBudget::as_str).map(str::to_string),
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
            .map_err(|_| "source job state is unavailable".to_string())?
            .get(id)
            .map(|job| job.snapshot.clone())
            .ok_or_else(|| "source job was not found".into())
    }

    pub fn cancel(&self, id: &str) -> Result<SourceJobSnapshot, String> {
        validate_job_id(id)?;
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| "source job state is unavailable".to_string())?;
        let job = jobs
            .get_mut(id)
            .ok_or_else(|| "source job was not found".to_string())?;
        if job.snapshot.status != "running" {
            return Ok(job.snapshot.clone());
        }
        job.snapshot.status = "cancelling";
        job.snapshot.summary = format!("Cancelling source {}…", job.snapshot.operation);
        if let Some(child) = job.child.take() {
            if let Err(error) = child.kill() {
                job.snapshot.status = "failed";
                job.snapshot.summary =
                    format!("Source {} could not be cancelled.", job.snapshot.operation);
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
                job.snapshot.summary =
                    terminal_summary(job.snapshot.operation, job.snapshot.status, false);
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
        job.snapshot.summary = terminal_summary(job.snapshot.operation, job.snapshot.status, true);
        job.snapshot.retryable = true;
        audit(&job.snapshot, "completed");
    }
}

pub fn open_setup(source_name: &str) -> Result<SetupOpenOutcome, String> {
    let source = settings::configured_source(source_name)?;
    let url = match source.kind.as_str() {
        "google-drive" | "gmail" | "google-calendar" => {
            "https://console.cloud.google.com/apis/credentials"
        }
        "slack" => "https://api.slack.com/apps",
        "discord" => "https://discord.com/developers/applications",
        _ => return Err("this source does not have a browser-based account setup page".into()),
    };
    open::that_detached(url).map_err(|error| format!("open source setup page: {error}"))?;
    let event = serde_json::json!({
        "at_unix_seconds": now(),
        "event": "source.setup.opened",
        "source": &source.name,
        "kind": &source.kind,
        "project": &source.project,
        "url_origin": reqwest::Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_string)),
        "secret_values_recorded": false,
    });
    let _ = settings::append_audit_event(&settings::default_config_path(), &event);
    Ok(SetupOpenOutcome {
        source: source.name,
        kind: source.kind,
        url,
        opened: true,
    })
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
    validation_args_with(source, 25, 5_242_880, 60)
}

fn validation_args_with(source: &str, documents: usize, bytes: u64, seconds: u64) -> Vec<String> {
    [
        "validate-source",
        source,
        "--max-documents",
        &documents.to_string(),
        "--max-bytes",
        &bytes.to_string(),
        "--max-seconds",
        &seconds.to_string(),
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn authorization_args(source: &str) -> Vec<String> {
    ["authorize-google", source]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn trial_sync_args(source: &str) -> Vec<String> {
    [
        "sync",
        "--source",
        source,
        "--require-validation",
        "--no-reconcile",
        "--max-documents",
        VALIDATION_MAX_DOCUMENTS,
        "--max-bytes",
        VALIDATION_MAX_BYTES,
        "--max-seconds",
        TRIAL_SYNC_MAX_SECONDS,
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn initial_sync_args(source: &str, budget: InitialSyncBudget) -> Vec<String> {
    let (documents, bytes, seconds) = budget.limits();
    [
        "sync",
        "--source",
        source,
        "--require-validation",
        "--no-reconcile",
        "--max-documents",
        &documents.to_string(),
        "--max-bytes",
        &bytes.to_string(),
        "--max-seconds",
        &seconds.to_string(),
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn initial_sync_summary(budget: InitialSyncBudget) -> String {
    let (documents, bytes, seconds) = budget.limits();
    format!(
        "Guarded initial sync may index up to {documents} documents or {} MiB for at most {} minutes. Reconciliation is disabled.",
        bytes / (1024 * 1024),
        seconds / 60
    )
}

fn prune_plans(plans: &mut BTreeMap<String, PendingPlan>) {
    let cutoff = now().saturating_sub(PLAN_TTL_SECONDS);
    plans.retain(|_, plan| plan.created_at >= cutoff);
    while plans.len() > MAX_PENDING_PLANS {
        let oldest = plans
            .iter()
            .min_by_key(|(_, plan)| plan.created_at)
            .map(|(id, _)| id.clone());
        if let Some(id) = oldest {
            plans.remove(&id);
        } else {
            break;
        }
    }
}

fn validate_plan_id(id: &str) -> Result<(), String> {
    if id.len() > 96
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("initial sync plan id is invalid".into());
    }
    Ok(())
}

/// Read-only hint about whether the latest validation record for a source
/// covers the selected budget. The sidecar remains the authoritative gate via
/// `--require-validation`; this only drives Desktop plan messaging.
fn validation_covers_budget_at(
    data_dir: &Path,
    source_name: &str,
    budget: InitialSyncBudget,
) -> Result<Option<bool>, String> {
    let path = data_dir.join("source-validations.json");
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "refusing to read symlinked file {}",
                    path.display()
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("inspect source validation state: {error}")),
    }
    let file =
        fs::File::open(&path).map_err(|error| format!("open source validation state: {error}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_VALIDATION_STATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read source validation state: {error}"))?;
    if bytes.len() as u64 > MAX_VALIDATION_STATE_BYTES {
        return Err("source validation state exceeds the 1 MiB Desktop read limit".into());
    }
    let root: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid source validation state: {error}"))?;
    let Some(record) = root
        .get("sources")
        .and_then(Value::as_object)
        .and_then(|sources| sources.get(source_name))
    else {
        return Ok(None);
    };
    let (Some(status), Some(max_documents), Some(max_bytes), Some(max_seconds)) = (
        record.get("status").and_then(Value::as_str),
        record.get("max_documents").and_then(Value::as_u64),
        record.get("max_bytes").and_then(Value::as_u64),
        record.get("max_seconds").and_then(Value::as_u64),
    ) else {
        // A record that omits any coverage field cannot be proven to cover the
        // budget; treat it as unknown so the UI asks for a fresh validation.
        return Ok(None);
    };
    let (documents, bytes, seconds) = budget.limits();
    Ok(Some(
        status == "succeeded"
            && max_documents >= documents as u64
            && max_bytes >= bytes
            && max_seconds >= seconds,
    ))
}

fn terminal_summary(operation: &str, status: &str, disconnected: bool) -> String {
    match (operation, status, disconnected) {
        ("trial-sync", "succeeded", _) => {
            "Guarded trial sync completed without deletion reconciliation.".into()
        }
        ("trial-sync", "cancelled", _) => {
            "Guarded trial sync was cancelled. Committed batches remain indexed; reconciliation did not run.".into()
        }
        ("trial-sync", _, true) => {
            "Guarded trial sync ended without a process result; reconciliation did not run.".into()
        }
        ("trial-sync", _, false) => {
            "Guarded trial sync failed; deletion reconciliation did not run.".into()
        }
        ("initial-sync", "succeeded", _) => {
            "Guarded initial sync completed within its selected budget without deletion reconciliation.".into()
        }
        ("initial-sync", "cancelled", _) => {
            "Guarded initial sync was cancelled. Committed batches remain indexed; reconciliation did not run.".into()
        }
        ("initial-sync", _, true) => {
            "Guarded initial sync ended without a process result; reconciliation did not run.".into()
        }
        ("initial-sync", _, false) => {
            "Guarded initial sync failed; deletion reconciliation did not run.".into()
        }
        ("authorization", "succeeded", _) => {
            "Google authorization completed and the token was stored privately.".into()
        }
        ("authorization", "cancelled", _) => "Google authorization was cancelled.".into(),
        ("authorization", _, true) => "Google authorization ended without a process result.".into(),
        ("authorization", _, false) => "Google authorization failed.".into(),
        (_, "succeeded", _) => "Source validation passed. No documents were indexed.".into(),
        (_, "cancelled", _) => "Source validation was cancelled. No documents were indexed.".into(),
        (_, _, true) => {
            "Source validation ended without a process result. No documents were indexed.".into()
        }
        _ => "Source validation failed. No documents were indexed.".into(),
    }
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
    let event = audit_event_json(snapshot, phase);
    let _ = settings::append_audit_event(&settings::default_config_path(), &event);
}

fn audit_event_json(snapshot: &SourceJobSnapshot, phase: &str) -> serde_json::Value {
    let mut event = serde_json::json!({
        "at_unix_seconds": now(),
        "event": format!("source.{}.{phase}", snapshot.operation),
        "job_id": snapshot.id,
        "source": snapshot.source,
        "kind": snapshot.kind,
        "project": snapshot.project,
        "status": snapshot.status,
        "exit_code": snapshot.exit_code,
        "writes_indexed_data": snapshot.writes_indexed_data,
        "source_content_recorded": false,
        "secret_values_recorded": false,
    });
    if let Some(budget) = &snapshot.budget {
        event["budget"] = serde_json::Value::String(budget.clone());
    }
    event
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

    #[test]
    fn authorization_command_accepts_only_the_configured_source_name() {
        assert_eq!(
            authorization_args("personal-drive"),
            ["authorize-google", "personal-drive"]
        );
    }

    #[test]
    fn trial_sync_is_validation_gated_bounded_and_never_reconciles() {
        assert_eq!(
            trial_sync_args("personal-drive"),
            [
                "sync",
                "--source",
                "personal-drive",
                "--require-validation",
                "--no-reconcile",
                "--max-documents",
                "25",
                "--max-bytes",
                "5242880",
                "--max-seconds",
                "300",
            ]
        );
    }

    #[test]
    fn initial_sync_budgets_are_fixed_and_never_escalate() {
        assert_eq!(InitialSyncBudget::Small.limits(), (100, 26_214_400, 900));
        assert_eq!(InitialSyncBudget::Medium.limits(), (500, 67_108_864, 1_800));
        assert_eq!(
            InitialSyncBudget::Large.limits(),
            (2_000, 134_217_728, 3_600)
        );
        assert_eq!(InitialSyncBudget::Small.as_str(), "small");
        assert_eq!(InitialSyncBudget::Medium.as_str(), "medium");
        assert_eq!(InitialSyncBudget::Large.as_str(), "large");
    }

    #[test]
    fn initial_sync_budget_enum_rejects_unbounded_or_unknown_values() {
        for tier in ["small", "medium", "large"] {
            assert!(serde_json::from_str::<InitialSyncBudget>(&format!("\"{tier}\"")).is_ok());
        }
        for unknown in [
            "huge",
            "1000000000",
            "--max-documents",
            "\"small\"; drop",
            "",
        ] {
            assert!(
                serde_json::from_str::<InitialSyncBudget>(&format!("\"{unknown}\"")).is_err(),
                "budget {unknown:?} must be rejected"
            );
        }
        assert!(serde_json::from_str::<InitialSyncBudget>("\"small\",\"medium\"").is_err());
        assert!(serde_json::from_str::<InitialSyncOperation>("\"plan\"").is_ok());
        assert!(serde_json::from_str::<InitialSyncOperation>("\"execute\"").is_ok());
        assert!(serde_json::from_str::<InitialSyncOperation>("\"plan-and-run\"").is_err());
    }

    #[test]
    fn initial_sync_command_is_validation_gated_bounded_and_never_reconciles() {
        assert_eq!(
            initial_sync_args("personal-drive", InitialSyncBudget::Small),
            [
                "sync",
                "--source",
                "personal-drive",
                "--require-validation",
                "--no-reconcile",
                "--max-documents",
                "100",
                "--max-bytes",
                "26214400",
                "--max-seconds",
                "900",
            ]
        );
        assert_eq!(
            initial_sync_args("personal-drive", InitialSyncBudget::Medium),
            [
                "sync",
                "--source",
                "personal-drive",
                "--require-validation",
                "--no-reconcile",
                "--max-documents",
                "500",
                "--max-bytes",
                "67108864",
                "--max-seconds",
                "1800",
            ]
        );
        assert_eq!(
            initial_sync_args("personal-drive", InitialSyncBudget::Large),
            [
                "sync",
                "--source",
                "personal-drive",
                "--require-validation",
                "--no-reconcile",
                "--max-documents",
                "2000",
                "--max-bytes",
                "134217728",
                "--max-seconds",
                "3600",
            ]
        );
    }

    #[test]
    fn budget_scoped_validation_uses_the_selected_limits_and_legacy_stays_fixed() {
        assert_eq!(
            validation_args_with("mail", 100, 26_214_400, 900),
            [
                "validate-source",
                "mail",
                "--max-documents",
                "100",
                "--max-bytes",
                "26214400",
                "--max-seconds",
                "900",
            ]
        );
        assert_eq!(
            validation_args("mail"),
            [
                "validate-source",
                "mail",
                "--max-documents",
                "25",
                "--max-bytes",
                "5242880",
                "--max-seconds",
                "60",
            ]
        );
        assert!(
            initial_sync_summary(InitialSyncBudget::Small)
                .contains("100 documents or 25 MiB for at most 15 minutes")
        );
    }

    fn test_source(name: &str, enabled: bool) -> settings::SourceSettings {
        settings::SourceSettings {
            name: name.into(),
            kind: "filesystem".into(),
            enabled,
            project: "work".into(),
            root: Some("/Users/example/docs".into()),
            source: None,
            channels: Vec::new(),
            token_env: None,
            token_path: None,
            oauth_client_path: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            acl: Vec::new(),
            editable: true,
        }
    }

    fn covered_validation_state(data_dir: &std::path::Path, source: &str) {
        std::fs::create_dir_all(data_dir).expect("data directory");
        let path = data_dir.join("source-validations.json");
        let mut root = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .unwrap_or_else(|| serde_json::json!({ "sources": {} }));
        root["sources"][source] = serde_json::json!({
            "status": "succeeded",
            "max_documents": 500,
            "max_bytes": 67108864,
            "max_seconds": 1800,
        });
        std::fs::write(&path, serde_json::to_vec(&root).expect("validation state"))
            .expect("write validation state");
    }

    #[test]
    fn initial_sync_plan_reports_validation_coverage_without_reading_credentials() {
        let temp = tempfile::tempdir().expect("temp directory");
        let state = SourceJobState::default();
        let plan = state
            .build_initial_sync_plan(
                &test_source("work-code", true),
                InitialSyncBudget::Medium,
                temp.path(),
            )
            .expect("plan");
        assert_eq!(plan.source, "work-code");
        assert!(plan.enabled);
        assert_eq!(plan.budget_documents, 500);
        assert_eq!(plan.budget_bytes, 67_108_864);
        assert_eq!(plan.budget_seconds, 1_800);
        assert!(plan.writes_indexed_data);
        assert!(plan.requires_validation);
        assert_eq!(plan.validation_covers_budget, None);
        assert!(plan.plan_id.starts_with("plan-"));

        covered_validation_state(temp.path(), "work-code");
        let plan = state
            .build_initial_sync_plan(
                &test_source("work-code", true),
                InitialSyncBudget::Small,
                temp.path(),
            )
            .expect("covered plan");
        assert_eq!(plan.validation_covers_budget, Some(true));
    }

    #[test]
    fn initial_sync_execution_requires_plan_confirmation_and_matching_budget() {
        let temp = tempfile::tempdir().expect("temp directory");
        let state = SourceJobState::default();
        let source = test_source("work-code", true);

        let error = state
            .confirm_initial_sync_execution(
                &source,
                InitialSyncBudget::Small,
                "plan-1-1",
                false,
                temp.path(),
            )
            .expect_err("unconfirmed execution must fail");
        assert!(error.contains("explicit plan confirmation"));

        covered_validation_state(temp.path(), "work-code");
        covered_validation_state(temp.path(), "other-code");
        let error = state
            .confirm_initial_sync_execution(
                &source,
                InitialSyncBudget::Small,
                "plan-1-1",
                true,
                temp.path(),
            )
            .expect_err("execution without a pending plan must fail");
        assert!(error.contains("plan was not found"));

        let plan = state
            .build_initial_sync_plan(&source, InitialSyncBudget::Small, temp.path())
            .expect("plan");
        let error = state
            .confirm_initial_sync_execution(
                &source,
                InitialSyncBudget::Medium,
                &plan.plan_id,
                true,
                temp.path(),
            )
            .expect_err("budget mismatch must fail");
        assert!(error.contains("does not match this source and budget"));

        let error = state
            .confirm_initial_sync_execution(
                &test_source("other-code", true),
                InitialSyncBudget::Small,
                &plan.plan_id,
                true,
                temp.path(),
            )
            .expect_err("source mismatch must fail");
        assert!(error.contains("does not match this source and budget"));

        state
            .confirm_initial_sync_execution(
                &source,
                InitialSyncBudget::Small,
                &plan.plan_id,
                true,
                temp.path(),
            )
            .expect("matching plan and approval must pass");
        // Confirmation alone must not consume the plan: a transient start
        // failure must not burn the token.
        state
            .confirm_initial_sync_execution(
                &source,
                InitialSyncBudget::Small,
                &plan.plan_id,
                true,
                temp.path(),
            )
            .expect("a passing confirmation keeps the plan pending");
        // Consumption happens only once the job actually started.
        state.consume_initial_sync_plan(&plan.plan_id);
        let error = state
            .confirm_initial_sync_execution(
                &source,
                InitialSyncBudget::Small,
                &plan.plan_id,
                true,
                temp.path(),
            )
            .expect_err("a consumed plan must not be reusable");
        assert!(error.contains("plan was not found"));

        let disabled = settings::SourceSettings {
            enabled: false,
            ..source
        };
        let plan = state
            .build_initial_sync_plan(&disabled, InitialSyncBudget::Small, temp.path())
            .expect("disabled plan");
        let error = state
            .confirm_initial_sync_execution(
                &disabled,
                InitialSyncBudget::Small,
                &plan.plan_id,
                true,
                temp.path(),
            )
            .expect_err("disabled sources must fail execution");
        assert!(error.contains("save and enable this source"));
    }

    #[test]
    fn initial_sync_coverage_requires_seconds_at_least_the_budget() {
        let temp = tempfile::tempdir().expect("temp directory");
        let state_path = temp.path().join("source-validations.json");
        let write_record = |record: serde_json::Value| {
            std::fs::write(
                &state_path,
                serde_json::to_vec(&serde_json::json!({ "sources": { "work-code": record } }))
                    .expect("serialize state"),
            )
            .expect("write state")
        };

        // A record whose time budget is smaller than the selected tier never
        // covers it, even when document and byte limits would.
        write_record(serde_json::json!({
            "status": "succeeded",
            "max_documents": 500,
            "max_bytes": 67108864,
            "max_seconds": 60,
        }));
        assert_eq!(
            validation_covers_budget_at(temp.path(), "work-code", InitialSyncBudget::Small),
            Ok(Some(false))
        );

        // A record that omits max_seconds cannot prove time coverage.
        write_record(serde_json::json!({
            "status": "succeeded",
            "max_documents": 500,
            "max_bytes": 67108864,
        }));
        assert_eq!(
            validation_covers_budget_at(temp.path(), "work-code", InitialSyncBudget::Small),
            Ok(None)
        );

        // Sufficient time coverage still requires every limit to fit.
        write_record(serde_json::json!({
            "status": "succeeded",
            "max_documents": 500,
            "max_bytes": 67108864,
            "max_seconds": 1800,
        }));
        assert_eq!(
            validation_covers_budget_at(temp.path(), "work-code", InitialSyncBudget::Medium),
            Ok(Some(true))
        );
        assert_eq!(
            validation_covers_budget_at(temp.path(), "work-code", InitialSyncBudget::Large),
            Ok(Some(false))
        );
    }

    #[test]
    fn initial_sync_plan_guard_rejects_unsafe_plan_ids_and_missing_coverage() {
        assert!(validate_plan_id("plan-123-4").is_ok());
        assert!(validate_plan_id("../plan").is_err());
        assert!(validate_plan_id(&"p".repeat(97)).is_err());

        let temp = tempfile::tempdir().expect("temp directory");
        let state = SourceJobState::default();
        let source = test_source("work-code", true);
        let plan = state
            .build_initial_sync_plan(&source, InitialSyncBudget::Large, temp.path())
            .expect("plan");
        let error = state
            .confirm_initial_sync_execution(
                &source,
                InitialSyncBudget::Large,
                &plan.plan_id,
                true,
                temp.path(),
            )
            .expect_err("missing validation coverage must fail");
        assert!(error.contains("validate with this budget first"));

        covered_validation_state(temp.path(), "work-code");
        let mut record = serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(temp.path().join("source-validations.json")).expect("state"),
        )
        .expect("state json");
        record["sources"]["work-code"]["status"] = serde_json::json!("failed");
        std::fs::write(
            temp.path().join("source-validations.json"),
            serde_json::to_vec(&record).expect("state"),
        )
        .expect("write state");
        assert_eq!(
            validation_covers_budget_at(temp.path(), "work-code", InitialSyncBudget::Large),
            Ok(Some(false))
        );
    }

    #[test]
    fn validation_coverage_refuses_symlinked_or_oversized_state() {
        let temp = tempfile::tempdir().expect("temp directory");
        assert_eq!(
            validation_covers_budget_at(temp.path(), "work-code", InitialSyncBudget::Small),
            Ok(None)
        );
        std::fs::write(
            temp.path().join("source-validations.json"),
            vec![b'x'; MAX_VALIDATION_STATE_BYTES as usize + 1],
        )
        .expect("oversized state");
        assert!(
            validation_covers_budget_at(temp.path(), "work-code", InitialSyncBudget::Small)
                .expect_err("oversized state must fail")
                .contains("1 MiB Desktop read limit")
        );
        std::fs::remove_file(temp.path().join("source-validations.json")).expect("remove state");
        #[cfg(unix)]
        {
            let link = temp.path().join("source-validations.json");
            std::os::unix::fs::symlink(temp.path().join("secrets.env"), &link)
                .expect("symlink state");
            assert!(
                validation_covers_budget_at(temp.path(), "work-code", InitialSyncBudget::Small)
                    .expect_err("symlinked state must fail")
                    .contains("symlinked file")
            );
        }
    }

    #[test]
    fn initial_sync_terminal_summaries_cover_cancellation_and_disconnects() {
        assert!(
            terminal_summary("initial-sync", "succeeded", false)
                .contains("without deletion reconciliation")
        );
        assert!(
            terminal_summary("initial-sync", "cancelled", false)
                .contains("Committed batches remain indexed")
        );
        assert!(
            terminal_summary("initial-sync", "failed", true)
                .contains("ended without a process result")
        );
        assert!(
            terminal_summary("initial-sync", "failed", false)
                .contains("reconciliation did not run")
        );
        assert!(
            terminal_summary("trial-sync", "cancelled", false)
                .contains("Committed batches remain indexed")
        );
    }

    #[test]
    fn initial_sync_audit_events_are_metadata_only_and_include_the_budget() {
        let snapshot = SourceJobSnapshot {
            id: "source-1-1".into(),
            operation: "initial-sync",
            source: "work-code".into(),
            kind: "filesystem".into(),
            project: "work".into(),
            status: "succeeded",
            summary: "done".into(),
            log: "secret output must never appear".into(),
            started_at_unix_seconds: 1,
            completed_at_unix_seconds: Some(2),
            exit_code: Some(0),
            retryable: false,
            writes_indexed_data: true,
            budget: Some("medium".into()),
        };
        let event = audit_event_json(&snapshot, "completed");
        assert_eq!(event["event"], "source.initial-sync.completed");
        assert_eq!(event["budget"], "medium");
        assert_eq!(event["writes_indexed_data"], true);
        assert_eq!(event["secret_values_recorded"], false);
        assert_eq!(event["source_content_recorded"], false);
        assert!(event.get("log").is_none());
        assert!(event.get("args").is_none());
        assert!(!format!("{event}").contains("secret output"));
    }
}
