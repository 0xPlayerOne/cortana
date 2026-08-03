use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::settings;

const MAX_FILE_BYTES: u64 = 16 * 1024;
const MIN_SYNC_INTERVAL_SECONDS: u64 = 60;
const MAX_SYNC_INTERVAL_SECONDS: u64 = 604_800;
const MIN_BACKUP_INTERVAL_SECONDS: u64 = 300;
const MAX_BACKUP_INTERVAL_SECONDS: u64 = 2_592_000;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScheduleSettings {
    pub sync_interval_seconds: u64,
    pub backup_interval_seconds: u64,
}

impl Default for ScheduleSettings {
    fn default() -> Self {
        Self {
            sync_interval_seconds: 900,
            backup_interval_seconds: 86_400,
        }
    }
}

pub fn load() -> Result<ScheduleSettings, String> {
    let path = schedule_path()?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ScheduleSettings::default());
        }
        Err(error) => return Err(format!("inspect service schedule: {error}")),
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing to use symlinked service schedule {}",
            path.display()
        ));
    }
    if !metadata.is_file() {
        return Err("service schedule must be a regular file".into());
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err("service schedule exceeds the 16 KiB limit".into());
    }
    let body =
        fs::read_to_string(&path).map_err(|error| format!("read service schedule: {error}"))?;
    let settings: ScheduleSettings =
        toml::from_str(&body).map_err(|error| format!("parse service schedule: {error}"))?;
    validate(&settings)?;
    Ok(settings)
}

pub fn save(settings: ScheduleSettings) -> Result<ScheduleSettings, String> {
    validate(&settings)?;
    let path = schedule_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| "service schedule path has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("create service schedule directory: {error}"))?;
    reject_symlink(&path)?;
    let rendered = toml::to_string_pretty(&settings)
        .map_err(|error| format!("serialize service schedule: {error}"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("read service schedule clock: {error}"))?
        .as_nanos();
    let temporary = parent.join(format!(".cortana-service-{nonce}.tmp"));
    reject_symlink(&temporary)?;
    if path.exists() {
        let backup = path.with_extension("toml.backup");
        reject_symlink(&backup)?;
        fs::copy(&path, &backup).map_err(|error| format!("back up service schedule: {error}"))?;
        set_owner_only_path(&backup)?;
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| format!("create service schedule: {error}"))?;
    set_owner_only(&file)?;
    std::io::Write::write_all(&mut file, rendered.as_bytes())
        .map_err(|error| format!("write service schedule: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("flush service schedule: {error}"))?;
    drop(file);
    fs::rename(&temporary, &path).map_err(|error| format!("replace service schedule: {error}"))?;
    let event = serde_json::json!({
        "at_unix_seconds": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        "event": "service.schedule.updated",
        "sync_interval_seconds": settings.sync_interval_seconds,
        "backup_interval_seconds": settings.backup_interval_seconds,
        "secret_values_recorded": false,
    });
    settings::append_audit_event(&settings::default_config_path(), &event)?;
    Ok(settings)
}

fn validate(settings: &ScheduleSettings) -> Result<(), String> {
    if !(MIN_SYNC_INTERVAL_SECONDS..=MAX_SYNC_INTERVAL_SECONDS)
        .contains(&settings.sync_interval_seconds)
    {
        return Err(format!(
            "recurring sync interval must be between {MIN_SYNC_INTERVAL_SECONDS} and {MAX_SYNC_INTERVAL_SECONDS} seconds"
        ));
    }
    if !(MIN_BACKUP_INTERVAL_SECONDS..=MAX_BACKUP_INTERVAL_SECONDS)
        .contains(&settings.backup_interval_seconds)
    {
        return Err(format!(
            "backup interval must be between {MIN_BACKUP_INTERVAL_SECONDS} and {MAX_BACKUP_INTERVAL_SECONDS} seconds"
        ));
    }
    Ok(())
}

fn schedule_path() -> Result<PathBuf, String> {
    let config = settings::default_config_path();
    let parent = config
        .parent()
        .ok_or_else(|| "configuration path has no parent".to_string())?;
    Ok(parent.join("service-schedule.toml"))
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing to use symlinked service schedule {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "inspect service schedule {}: {error}",
            path.display()
        )),
    }
}

fn set_owner_only(file: &std::fs::File) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("secure service schedule: {error}"))?;
    }
    Ok(())
}

fn set_owner_only_path(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(path)
            .map_err(|error| format!("inspect service schedule backup: {error}"))?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)
            .map_err(|error| format!("secure service schedule backup: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe_and_bounded() {
        let settings = ScheduleSettings::default();
        validate(&settings).expect("default schedule is valid");
        assert_eq!(settings.sync_interval_seconds, 900);
        assert_eq!(settings.backup_interval_seconds, 86_400);
    }

    #[test]
    fn rejects_intervals_that_are_too_aggressive_or_slow() {
        let invalid = [
            ScheduleSettings {
                sync_interval_seconds: MIN_SYNC_INTERVAL_SECONDS - 1,
                ..ScheduleSettings::default()
            },
            ScheduleSettings {
                sync_interval_seconds: MAX_SYNC_INTERVAL_SECONDS + 1,
                ..ScheduleSettings::default()
            },
            ScheduleSettings {
                backup_interval_seconds: MIN_BACKUP_INTERVAL_SECONDS - 1,
                ..ScheduleSettings::default()
            },
            ScheduleSettings {
                backup_interval_seconds: MAX_BACKUP_INTERVAL_SECONDS + 1,
                ..ScheduleSettings::default()
            },
        ];
        assert!(invalid.iter().all(|settings| validate(settings).is_err()));
    }
}
