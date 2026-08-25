use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri_plugin_updater::{Error as UpdaterError, Update, UpdaterExt};
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

    pub async fn check<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
    ) -> Result<UpdateSnapshot, String> {
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
        // Keep the observable state fail-closed even when the updater plugin
        // cannot be initialized (for example in a headless test runtime). The
        // previous `?` returned while leaving the snapshot stuck at `checking`.
        let result = match app.updater() {
            Ok(updater) => updater.check().await,
            Err(error) => {
                let error = format!("initialize signed updater: {error}");
                self.update_snapshot(|snapshot| {
                    snapshot.phase = "failed";
                    snapshot.error = Some(error.clone());
                });
                return Err(error);
            }
        };

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
            Err(error) if is_target_unavailable(&error) => {
                *self.pending.lock().await = None;
                self.update_snapshot(|snapshot| {
                    // A release can be valid while deliberately omitting an
                    // unsigned or unavailable platform. Treat that as a
                    // supported no-update state instead of surfacing a
                    // misleading network/JSON failure to the user.
                    snapshot.phase = "unavailable";
                    snapshot.available_version = None;
                    snapshot.release_date = None;
                    snapshot.release_notes = None;
                    snapshot.error = None;
                    snapshot.restart_required = false;
                });
                Ok(self.status())
            }
            Err(error) => {
                let error = format!("check for signed Cortana update: {error}");
                *self.pending.lock().await = None;
                self.update_snapshot(|snapshot| {
                    snapshot.phase = "failed";
                    snapshot.error = Some(error.clone());
                });
                Err(error)
            }
        }
    }

    pub async fn install<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        expected_version: &str,
        approved: bool,
        restart: bool,
    ) -> Result<UpdateSnapshot, String> {
        let _operation = self.operation.lock().await;
        let pending = self.pending.lock().await.take();
        let update = match validate_install_request(
            approved,
            expected_version,
            pending.as_ref().map(|update| update.version.as_str()),
        ) {
            Ok(()) => match pending {
                Some(update) => update,
                None => {
                    // Keep this path explicit even though validation currently
                    // requires a pending version. The pending slot is shared
                    // state; a future refactor must not turn an invariant
                    // violation into a desktop-process panic.
                    return Err(InstallGuardError::NoPendingUpdate.message());
                }
            },
            Err(guard) => {
                // Preserve an available update on every rejection so a
                // failed install attempt never silently drops it.
                if let Some(update) = pending {
                    *self.pending.lock().await = Some(update);
                }
                return Err(guard.message());
            }
        };

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
            Ok(result) => {
                result.map_err(|error| format!("verify and install signed Cortana update: {error}"))
            }
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

/// A rejected install request, kept distinct so the caller can tell a
/// missing pending update (nothing to restore) from a changed one (must be
/// preserved for a retry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallGuardError {
    ApprovalRequired,
    InvalidVersion,
    NoPendingUpdate,
    VersionMismatch,
}

impl InstallGuardError {
    fn message(&self) -> String {
        match self {
            Self::ApprovalRequired => "update installation requires explicit approval".into(),
            Self::InvalidVersion => "invalid expected update version".into(),
            Self::NoPendingUpdate => "check for an update before installing".into(),
            Self::VersionMismatch => {
                "available update changed; check again before installing".into()
            }
        }
    }
}

/// Validate the preconditions for installing a pending update.
///
/// Pure so the approval, version and pending guards can be unit tested
/// without the updater plugin or any network access. The caller takes the
/// pending update before invoking this and restores it on every rejection
/// so an available update always survives a failed install attempt.
fn validate_install_request(
    approved: bool,
    expected_version: &str,
    pending_version: Option<&str>,
) -> Result<(), InstallGuardError> {
    if !approved {
        return Err(InstallGuardError::ApprovalRequired);
    }
    if expected_version.is_empty() || expected_version.len() > 64 {
        return Err(InstallGuardError::InvalidVersion);
    }
    match pending_version {
        None => Err(InstallGuardError::NoPendingUpdate),
        Some(pending) if pending != expected_version => Err(InstallGuardError::VersionMismatch),
        Some(_) => Ok(()),
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

fn is_target_unavailable(error: &UpdaterError) -> bool {
    matches!(
        error,
        UpdaterError::TargetNotFound(_) | UpdaterError::TargetsNotFound(_)
    )
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

    #[test]
    fn missing_platform_is_a_nonfatal_update_state() {
        assert!(is_target_unavailable(&UpdaterError::TargetNotFound(
            "darwin-aarch64".into()
        )));
        assert!(is_target_unavailable(&UpdaterError::TargetsNotFound(vec![
            "darwin-aarch64-app".into(),
            "darwin-aarch64".into(),
        ])));
        assert!(!is_target_unavailable(&UpdaterError::ReleaseNotFound));
    }

    #[test]
    fn install_guard_rejects_without_approval() {
        let result = validate_install_request(false, "1.2.3", Some("1.2.3"));
        assert_eq!(result, Err(InstallGuardError::ApprovalRequired));
        assert_eq!(
            result.unwrap_err().message(),
            "update installation requires explicit approval"
        );
    }

    #[test]
    fn install_guard_rejects_empty_and_oversized_versions() {
        assert_eq!(
            validate_install_request(true, "", Some("1.2.3")),
            Err(InstallGuardError::InvalidVersion)
        );
        let oversized = "x".repeat(65);
        assert_eq!(
            validate_install_request(true, &oversized, Some("1.2.3")),
            Err(InstallGuardError::InvalidVersion)
        );
        // The version guard is checked before the pending guard, matching
        // the installer's original check ordering.
        assert_eq!(
            validate_install_request(true, "", None),
            Err(InstallGuardError::InvalidVersion)
        );
    }

    #[test]
    fn install_guard_rejects_without_pending_update() {
        let result = validate_install_request(true, "1.2.3", None);
        assert_eq!(result, Err(InstallGuardError::NoPendingUpdate));
        assert_eq!(
            result.unwrap_err().message(),
            "check for an update before installing"
        );
    }

    #[test]
    fn install_guard_accepts_matching_pending_version() {
        assert_eq!(
            validate_install_request(true, "1.2.3", Some("1.2.3")),
            Ok(())
        );
    }

    #[test]
    fn install_guard_rejects_mismatch_and_preserves_pending_update() {
        // `Update` cannot be constructed without the updater plugin, so the
        // caller contract of `install` is exercised with a version stub:
        // the pending value is taken, the guard runs, and a rejected guard
        // restores it so a retry still sees the available update.
        let mut pending: Option<String> = Some("2.0.0".into());
        let taken = pending.take();
        let result = validate_install_request(true, "1.2.3", taken.as_deref());
        assert_eq!(result, Err(InstallGuardError::VersionMismatch));
        assert_eq!(
            result.unwrap_err().message(),
            "available update changed; check again before installing"
        );
        pending = taken;
        assert_eq!(
            pending.as_deref(),
            Some("2.0.0"),
            "available update survives a rejected install"
        );
    }

    #[test]
    fn install_guards_fail_closed_without_network() {
        let app = tauri::test::mock_app();
        let state = UpdaterState::default();
        let error = tauri::async_runtime::block_on(async {
            state.install(app.handle(), "1.2.3", false, false).await
        })
        .unwrap_err();
        assert_eq!(error, "update installation requires explicit approval");
        let pending_is_empty =
            tauri::async_runtime::block_on(async { state.pending.lock().await.is_none() });
        assert!(pending_is_empty);
    }

    #[test]
    fn install_rejects_invalid_version_without_network() {
        let app = tauri::test::mock_app();
        let state = UpdaterState::default();
        for expected in ["".to_string(), "x".repeat(65)] {
            let error = tauri::async_runtime::block_on(async {
                state.install(app.handle(), &expected, true, false).await
            })
            .unwrap_err();
            assert_eq!(error, "invalid expected update version");
        }
    }

    #[test]
    fn install_rejects_without_pending_update_without_network() {
        let app = tauri::test::mock_app();
        let state = UpdaterState::default();
        let error = tauri::async_runtime::block_on(async {
            state.install(app.handle(), "1.2.3", true, false).await
        })
        .unwrap_err();
        assert_eq!(error, "check for an update before installing");
    }
}
