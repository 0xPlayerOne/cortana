use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

const STATE_FILE: &str = "source-validations.json";
const LOCK_FILE: &str = "source-validations.lock";
const MAX_ERROR_CHARS: usize = 500;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SourceValidationStatus {
    pub source: String,
    pub project: String,
    pub kind: String,
    pub status: String,
    pub validated_at: DateTime<Utc>,
    pub documents: Option<usize>,
    pub bytes: Option<u64>,
    pub max_documents: usize,
    pub max_bytes: u64,
    pub max_seconds: u64,
    pub error: Option<String>,
}

#[derive(Default, Deserialize, Serialize)]
struct ValidationState {
    sources: BTreeMap<String, SourceValidationStatus>,
}

pub fn load(data_dir: &Path) -> Result<BTreeMap<String, SourceValidationStatus>> {
    let path = data_dir.join(STATE_FILE);
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("failed to read source validation state {}", path.display()))?;
    let state: ValidationState = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid source validation state {}", path.display()))?;
    Ok(state.sources)
}

pub fn record(data_dir: &Path, mut status: SourceValidationStatus) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    status.error = status.error.map(sanitize_error);
    let lock_path = data_dir.join(LOCK_FILE);
    let lock = owner_only_file(&lock_path)?;
    lock.lock_exclusive()?;

    let mut state = ValidationState {
        sources: load(data_dir)?,
    };
    state.sources.insert(status.source.clone(), status);
    let path = data_dir.join(STATE_FILE);
    let temporary = temporary_path(&path);
    let result = (|| -> Result<()> {
        let mut output = owner_only_file(&temporary)?;
        output.set_len(0)?;
        output.write_all(&serde_json::to_vec_pretty(&state)?)?;
        output.sync_all()?;
        std::fs::rename(&temporary, &path)?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&temporary);
    let _ = FileExt::unlock(&lock);
    result
}

fn owner_only_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

fn temporary_path(path: &Path) -> PathBuf {
    path.with_extension(format!("tmp-{}", std::process::id()))
}

fn sanitize_error(error: String) -> String {
    error
        .replace(['\r', '\n'], " ")
        .chars()
        .take(MAX_ERROR_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_state_is_bounded_by_source_and_sanitizes_errors() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let status = |state: &str| SourceValidationStatus {
            source: "drive".into(),
            project: "work".into(),
            kind: "google-drive".into(),
            status: state.into(),
            validated_at: Utc::now(),
            documents: None,
            bytes: None,
            max_documents: 25,
            max_bytes: 1024,
            max_seconds: 30,
            error: Some("line one\nline two".into()),
        };
        record(directory.path(), status("failed")).expect("first record");
        record(directory.path(), status("succeeded")).expect("replacement record");
        let state = load(directory.path()).expect("state");
        assert_eq!(state.len(), 1);
        assert_eq!(state["drive"].status, "succeeded");
        assert_eq!(state["drive"].error.as_deref(), Some("line one line two"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(directory.path().join(STATE_FILE))
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
