use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::SourceConfig;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_fingerprint: Option<String>,
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

pub fn configuration_fingerprint(source: &SourceConfig) -> Result<String> {
    let encoded = serde_json::to_vec(source).context("serialize source configuration")?;
    Ok(URL_SAFE_NO_PAD.encode(Sha256::digest(encoded)))
}

pub fn require_success(
    data_dir: &Path,
    source: &SourceConfig,
    max_documents: usize,
    max_bytes: u64,
) -> Result<()> {
    let validations = load(data_dir)?;
    let validation = validations
        .get(&source.name)
        .with_context(|| format!("source {} has not been validated", source.name))?;
    anyhow::ensure!(
        validation.status == "succeeded",
        "source {} latest validation did not succeed",
        source.name
    );
    anyhow::ensure!(
        validation.project == source.project && validation.kind == source.kind,
        "source {} configuration changed since validation",
        source.name
    );
    let fingerprint = configuration_fingerprint(source)?;
    anyhow::ensure!(
        validation.configuration_fingerprint.as_deref() == Some(fingerprint.as_str()),
        "source {} configuration changed since validation",
        source.name
    );
    anyhow::ensure!(
        validation.max_documents >= max_documents && validation.max_bytes >= max_bytes,
        "source {} validation limits were smaller than this sync",
        source.name
    );
    Ok(())
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

    fn source() -> SourceConfig {
        SourceConfig {
            name: "drive".into(),
            kind: "google-drive".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            token_env: Some("GOOGLE_TOKEN".into()),
            token: None,
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: Some(25),
            max_bytes: Some(1024),
            max_duration_seconds: Some(60),
            exclude: Vec::new(),
            command: Vec::new(),
            acl: vec!["work".into()],
        }
    }

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
            configuration_fingerprint: None,
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

    #[test]
    fn required_validation_is_bound_to_configuration_and_limits() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let source = source();
        record(
            directory.path(),
            SourceValidationStatus {
                source: source.name.clone(),
                project: source.project.clone(),
                kind: source.kind.clone(),
                status: "succeeded".into(),
                validated_at: Utc::now(),
                documents: Some(12),
                bytes: Some(512),
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 60,
                configuration_fingerprint: Some(configuration_fingerprint(&source).unwrap()),
                error: None,
            },
        )
        .unwrap();
        require_success(directory.path(), &source, 25, 1024).unwrap();
        assert!(require_success(directory.path(), &source, 26, 1024).is_err());
        let mut changed = source;
        changed.query = Some("from:someone@example.com".into());
        assert!(require_success(directory.path(), &changed, 25, 1024).is_err());
    }
}
