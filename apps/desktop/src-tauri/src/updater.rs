use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex as AsyncMutex;

use crate::settings;

const GITHUB_URL: &str = "https://github.com/0xPlayerOne/cortana";
const MAX_RELEASE_NOTES_CHARS: usize = 32_000;
const UPDATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const CHANGELOG: &str = include_str!("../../../../CHANGELOG.md");

#[derive(Clone, Debug, Serialize)]
pub struct UpdateSnapshot {
    pub current_version: String,
    pub available_version: Option<String>,
    pub release_date: Option<String>,
    pub release_notes: Option<String>,
    pub changelog: String,
    pub github_url: &'static str,
    pub phase: &'static str,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<String>,
    pub restart_required: bool,
}

impl Default for UpdateSnapshot {
    fn default() -> Self {
        Self {
            current_version: env!("CARGO_PKG_VERSION").into(),
            available_version: None,
            release_date: None,
            release_notes: None,
            changelog: bounded(CHANGELOG, MAX_RELEASE_NOTES_CHARS),
            github_url: GITHUB_URL,
            phase: "idle",
            downloaded_bytes: 0,
            total_bytes: None,
            error: None,
            restart_required: false,
        }
    }
}

#[derive(Clone, Default)]
pub struct UpdaterState {
    operation: Arc<AsyncMutex<()>>,
    pending: Arc<AsyncMutex<Option<Update>>>,
    snapshot: Arc<Mutex<UpdateSnapshot>>,
}

impl UpdaterState {
    pub fn status(&self) -> UpdateSnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub async fn check(&self, app: &AppHandle) -> Result<UpdateSnapshot, String> {
        let _operation = self.operation.lock().await;
        self.update_snapshot(|snapshot| {
            snapshot.phase = "checking";
            snapshot.error = None;
            snapshot.available_version = None;
            snapshot.release_date = None;
            snapshot.release_notes = None;
            snapshot.downloaded_bytes = 0;
            snapshot.total_bytes = None;
            snapshot.restart_required = false;
        });
        let result = app
            .updater()
            .map_err(|error| format!("initialize signed updater: {error}"))?
            .check()
            .await
            .map_err(|error| format!("check for signed Cortana update: {error}"));

        match result {
            Ok(Some(update)) => {
                let available_version = update.version.clone();
                let release_date = update.date.map(|value| value.to_string());
                let release_notes = update
                    .body
                    .as_deref()
                    .map(|value| bounded(value, MAX_RELEASE_NOTES_CHARS));
                *self.pending.lock().await = Some(update);
                self.update_snapshot(|snapshot| {
                    snapshot.phase = "available";
                    snapshot.available_version = Some(available_version);
                    snapshot.release_date = release_date;
                    snapshot.release_notes = release_notes;
                    snapshot.error = None;
                    snapshot.restart_required = false;
                });
                Ok(self.status())
            }
            Ok(None) => {
                *self.pending.lock().await = None;
                self.update_snapshot(|snapshot| {
                    snapshot.phase = "current";
                    snapshot.available_version = None;
                    snapshot.release_date = None;
                    snapshot.release_notes = None;
                    snapshot.error = None;
                    snapshot.restart_required = false;
                });
                Ok(self.status())
            }
            Err(error) => {
                *self.pending.lock().await = None;
                self.update_snapshot(|snapshot| {
                    snapshot.phase = "failed";
                    snapshot.available_version = None;
                    snapshot.error = Some(error.clone());
                });
                Err(error)
            }
        }
    }

    pub async fn install(
        &self,
        app: &AppHandle,
        expected_version: &str,
        approved: bool,
        restart: bool,
    ) -> Result<UpdateSnapshot, String> {
        let _operation = self.operation.lock().await;
        if !approved {
            return Err("update installation requires explicit approval".into());
        }
        if expected_version.is_empty() || expected_version.len() > 64 {
            return Err("invalid expected update version".into());
        }
        let update = self
            .pending
            .lock()
            .await
            .take()
            .ok_or_else(|| "check for an update before installing".to_string())?;
        if update.version != expected_version {
            *self.pending.lock().await = Some(update);
            return Err("available update changed; check again before installing".into());
        }

        self.update_snapshot(|snapshot| {
            snapshot.phase = "downloading";
            snapshot.downloaded_bytes = 0;
            snapshot.total_bytes = None;
            snapshot.error = None;
        });
        audit("update.install.started", Some(expected_version), restart);

        let progress = self.snapshot.clone();
        let progress_finished = self.snapshot.clone();
        let retry = update.clone();
        let result = match tokio::time::timeout(
            UPDATE_TIMEOUT,
            update.download_and_install(
                move |chunk, total| {
                    let mut snapshot = progress
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    snapshot.phase = "downloading";
                    snapshot.downloaded_bytes =
                        snapshot.downloaded_bytes.saturating_add(chunk as u64);
                    snapshot.total_bytes = total;
                },
                move || {
                    let mut snapshot = progress_finished
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    snapshot.phase = "installing";
                },
            ),
        )
        .await
        {
            Ok(result) => result
                .map_err(|error| format!("verify and install signed Cortana update: {error}")),
            Err(_) => Err(format!(
                "signed Cortana update timed out after {} seconds",
                UPDATE_TIMEOUT.as_secs()
            )),
        };

        if let Err(error) = result {
            *self.pending.lock().await = Some(retry);
            self.update_snapshot(|snapshot| {
                snapshot.phase = "failed";
                snapshot.error = Some(error.clone());
            });
            audit("update.install.failed", Some(expected_version), restart);
            return Err(error);
        }

        self.update_snapshot(|snapshot| {
            snapshot.phase = "installed";
            snapshot.error = None;
            snapshot.restart_required = true;
        });
        audit("update.install.completed", Some(expected_version), restart);
        let snapshot = self.status();
        if restart {
            app.request_restart();
        }
        Ok(snapshot)
    }

    fn update_snapshot(&self, update: impl FnOnce(&mut UpdateSnapshot)) {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut snapshot);
    }
}

fn audit(event: &str, version: Option<&str>, restart: bool) {
    let value = serde_json::json!({
        "at_unix_seconds": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "event": event,
        "version": version,
        "restart_requested": restart,
        "secret_values_recorded": false,
    });
    let _ = settings::append_audit_event(&settings::default_config_path(), &value);
}

fn bounded(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_is_bounded_and_contains_release_metadata() {
        let snapshot = UpdateSnapshot::default();
        assert_eq!(snapshot.current_version, env!("CARGO_PKG_VERSION"));
        assert_eq!(snapshot.github_url, GITHUB_URL);
        assert!(snapshot.changelog.len() <= MAX_RELEASE_NOTES_CHARS);
        assert_eq!(bounded("cortana", 4), "cort");
    }
}
