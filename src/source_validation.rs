use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
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
const MAX_STATE_BYTES: u64 = 1024 * 1024;

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
    /// Whether the validation covered the entire source within its limits.
    /// `Some(false)` marks a bounded sample that may authorize only equally
    /// bounded non-reconciling runs; `None` (records persisted before
    /// sampling existed) is treated as complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complete: Option<bool>,
    pub error: Option<String>,
}

#[derive(Default, Deserialize, Serialize)]
struct ValidationState {
    sources: BTreeMap<String, SourceValidationStatus>,
}

pub fn load(data_dir: &Path) -> Result<BTreeMap<String, SourceValidationStatus>> {
    let path = data_dir.join(STATE_FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeMap::new());
        }
        Err(error) => return Err(error.into()),
    }
    validate_data_directory(data_dir, false)?;
    let input = open_existing_file(&path)
        .with_context(|| format!("failed to open source validation state {}", path.display()))?;
    let mut bytes = Vec::new();
    input
        .take(MAX_STATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read source validation state {}", path.display()))?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_STATE_BYTES,
        "source validation state exceeds {MAX_STATE_BYTES} bytes"
    );
    let state: ValidationState = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid source validation state {}", path.display()))?;
    Ok(state.sources)
}

pub fn record(data_dir: &Path, mut status: SourceValidationStatus) -> Result<()> {
    validate_data_directory(data_dir, true)?;
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
        let mut output = create_owner_only_file(&temporary)?;
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
    max_seconds: u64,
    max_age: chrono::Duration,
    reconcile: bool,
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
        !reconcile || validation.complete != Some(false),
        "source {} latest validation was a bounded sample and cannot authorize a full-corpus sync; re-run validate-source without --sample for the complete source",
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
    anyhow::ensure!(
        validation.max_seconds >= max_seconds,
        "source {} validation duration limit was smaller than this sync",
        source.name
    );
    if max_age > chrono::Duration::zero() {
        let age = Utc::now().signed_duration_since(validation.validated_at);
        anyhow::ensure!(
            age >= chrono::Duration::zero(),
            "source {} validation timestamp is {} in the future; check the system clock and re-run validate-source",
            source.name,
            describe_age(-age)
        );
        anyhow::ensure!(
            age <= max_age,
            "source {} validation is {} old (maximum {}); re-run validate-source",
            source.name,
            describe_age(age),
            describe_age(max_age)
        );
    }
    Ok(())
}

fn describe_age(age: chrono::Duration) -> String {
    let days = age.num_days();
    if days >= 1 {
        format!("{days} day{}", if days == 1 { "" } else { "s" })
    } else {
        let hours = age.num_hours();
        if hours >= 1 {
            format!("{hours} hour{}", if hours == 1 { "" } else { "s" })
        } else {
            format!("{} minutes", age.num_minutes().max(0))
        }
    }
}

fn owner_only_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    configure_no_follow(&mut options);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    reject_symlink(path)?;
    let file = options.open(path)?;
    validate_open_file(&file, path)?;
    set_owner_only(&file)?;
    Ok(file)
}

fn create_owner_only_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    configure_no_follow(&mut options);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    validate_open_file(&file, path)?;
    set_owner_only(&file)?;
    Ok(file)
}

fn open_existing_file(path: &Path) -> Result<File> {
    reject_symlink(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);
    let file = options.open(path)?;
    validate_open_file(&file, path)?;
    Ok(file)
}

fn reject_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "refusing to use symlinked validation path {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn validate_data_directory(path: &Path, create: bool) -> Result<()> {
    reject_symlink(path)?;
    if create {
        std::fs::create_dir_all(path)?;
        reject_symlink(path)?;
    }
    let metadata = std::fs::metadata(path)?;
    anyhow::ensure!(
        metadata.is_dir(),
        "validation data path is not a directory: {}",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        anyhow::ensure!(
            metadata.uid() == unsafe { libc::geteuid() },
            "validation data directory is not owned by the current user: {}",
            path.display()
        );
        if create {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

fn validate_open_file(file: &File, path: &Path) -> Result<()> {
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file(),
        "validation path is not a regular file: {}",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        anyhow::ensure!(
            metadata.uid() == unsafe { libc::geteuid() },
            "validation file is not owned by the current user: {}",
            path.display()
        );
        anyhow::ensure!(
            metadata.nlink() == 1,
            "validation file has multiple hard links: {}",
            path.display()
        );
    }
    Ok(())
}

fn set_owner_only(file: &File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(unix)]
fn configure_no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
}

#[cfg(not(unix))]
fn configure_no_follow(_options: &mut OpenOptions) {}

fn temporary_path(path: &Path) -> PathBuf {
    path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()))
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
            complete: None,
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
                complete: None,
                error: None,
            },
        )
        .unwrap();
        require_success(
            directory.path(),
            &source,
            25,
            1024,
            60,
            chrono::Duration::hours(168),
            false,
        )
        .unwrap();
        assert!(
            require_success(
                directory.path(),
                &source,
                26,
                1024,
                60,
                chrono::Duration::hours(168),
                false
            )
            .is_err()
        );
        assert!(
            require_success(
                directory.path(),
                &source,
                25,
                1024,
                61,
                chrono::Duration::hours(168),
                false
            )
            .is_err()
        );
        let mut changed = source;
        changed.query = Some("from:someone@example.com".into());
        assert!(
            require_success(
                directory.path(),
                &changed,
                25,
                1024,
                60,
                chrono::Duration::hours(168),
                false
            )
            .is_err()
        );
    }

    #[test]
    fn sampled_validation_never_authorizes_a_full_corpus_sync() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let source = source();
        let status_for = |complete: Option<bool>| SourceValidationStatus {
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
            complete,
            error: None,
        };

        // A sampled (partial) record satisfies only equally bounded
        // non-reconciling runs; it must never bless a reconciling sync.
        record(directory.path(), status_for(Some(false))).unwrap();
        let sampled = require_success(
            directory.path(),
            &source,
            25,
            1024,
            60,
            chrono::Duration::hours(168),
            true,
        )
        .expect_err("a bounded sample must not authorize a full-corpus sync");
        assert!(format!("{sampled:#}").contains("bounded sample"));
        assert!(format!("{sampled:#}").contains("--sample"));
        require_success(
            directory.path(),
            &source,
            25,
            1024,
            60,
            chrono::Duration::hours(168),
            false,
        )
        .expect("an equally bounded non-reconciling run accepts a sample");

        // A sample that covered the whole corpus is complete and blesses
        // reconciling syncs like any other successful validation.
        record(directory.path(), status_for(Some(true))).unwrap();
        require_success(
            directory.path(),
            &source,
            25,
            1024,
            60,
            chrono::Duration::hours(168),
            true,
        )
        .expect("a complete validation authorizes a full-corpus sync");

        // Records persisted before sampling existed carry no completeness
        // marker and keep their legacy full-corpus authority.
        record(directory.path(), status_for(None)).unwrap();
        require_success(
            directory.path(),
            &source,
            25,
            1024,
            60,
            chrono::Duration::hours(168),
            true,
        )
        .expect("a legacy record without a completeness marker stays complete");
    }

    #[test]
    fn required_validation_lapses_after_the_freshness_bound() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let source = source();
        let mut status = SourceValidationStatus {
            source: source.name.clone(),
            project: source.project.clone(),
            kind: source.kind.clone(),
            status: "succeeded".into(),
            validated_at: Utc::now() - chrono::Duration::hours(48),
            documents: Some(12),
            bytes: Some(512),
            max_documents: 25,
            max_bytes: 1024,
            max_seconds: 60,
            configuration_fingerprint: Some(configuration_fingerprint(&source).unwrap()),
            complete: None,
            error: None,
        };
        record(directory.path(), status.clone()).unwrap();

        let lapsed = require_success(
            directory.path(),
            &source,
            25,
            1024,
            60,
            chrono::Duration::hours(24),
            false,
        )
        .expect_err("a 48-hour-old validation must not satisfy a 24-hour bound");
        assert!(format!("{lapsed:#}").contains("2 days old"));
        assert!(format!("{lapsed:#}").contains("re-run validate-source"));

        assert!(
            require_success(
                directory.path(),
                &source,
                25,
                1024,
                60,
                chrono::Duration::hours(72),
                false,
            )
            .is_ok()
        );

        status.validated_at = Utc::now() - chrono::Duration::minutes(30);
        record(directory.path(), status).unwrap();
        require_success(
            directory.path(),
            &source,
            25,
            1024,
            60,
            chrono::Duration::hours(24),
            false,
        )
        .expect("a fresh validation must pass");
        let hour_old = require_success(
            directory.path(),
            &source,
            25,
            1024,
            60,
            chrono::Duration::minutes(25),
            false,
        )
        .expect_err("a 30-minute-old validation must fail a 25-minute bound");
        assert!(format!("{hour_old:#}").contains("30 minutes old"));
        assert!(format!("{hour_old:#}").contains("maximum 25 minutes"));
    }

    #[test]
    fn zero_freshness_bound_accepts_any_validation_age() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let source = source();
        record(
            directory.path(),
            SourceValidationStatus {
                source: source.name.clone(),
                project: source.project.clone(),
                kind: source.kind.clone(),
                status: "succeeded".into(),
                validated_at: Utc::now() - chrono::Duration::days(365),
                documents: Some(12),
                bytes: Some(512),
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 60,
                configuration_fingerprint: Some(configuration_fingerprint(&source).unwrap()),
                complete: None,
                error: None,
            },
        )
        .unwrap();
        assert!(
            require_success(
                directory.path(),
                &source,
                25,
                1024,
                60,
                chrono::Duration::zero(),
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn future_validation_timestamp_fails_a_bounded_check_but_passes_unlimited() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let source = source();
        record(
            directory.path(),
            SourceValidationStatus {
                source: source.name.clone(),
                project: source.project.clone(),
                kind: source.kind.clone(),
                status: "succeeded".into(),
                validated_at: Utc::now() + chrono::Duration::hours(2),
                documents: Some(12),
                bytes: Some(512),
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 60,
                configuration_fingerprint: Some(configuration_fingerprint(&source).unwrap()),
                complete: None,
                error: None,
            },
        )
        .unwrap();

        let future = require_success(
            directory.path(),
            &source,
            25,
            1024,
            60,
            chrono::Duration::hours(24),
            false,
        )
        .expect_err("a future validation timestamp must fail a bounded freshness check");
        let message = format!("{future:#}");
        assert!(message.contains("in the future"), "{message}");
        assert!(message.contains("system clock"), "{message}");
        assert!(message.contains("re-run validate-source"), "{message}");

        require_success(
            directory.path(),
            &source,
            25,
            1024,
            60,
            chrono::Duration::zero(),
            false,
        )
        .expect("an unlimited freshness bound accepts future timestamps");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_validation_state_without_reading_target() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory");
        let target = directory.path().join("target.json");
        std::fs::write(&target, r#"{"sources":{}}"#).expect("write target");
        symlink(&target, directory.path().join(STATE_FILE)).expect("create symlink");

        let error = load(directory.path()).expect_err("symlink must be rejected");
        assert!(format!("{error:#}").contains("symlinked validation path"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_lock_without_modifying_target() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory");
        let target = directory.path().join("target.txt");
        std::fs::write(&target, "unchanged").expect("write target");
        symlink(&target, directory.path().join(LOCK_FILE)).expect("create symlink");

        let error = record(
            directory.path(),
            SourceValidationStatus {
                source: "drive".into(),
                project: "work".into(),
                kind: "google-drive".into(),
                status: "succeeded".into(),
                validated_at: Utc::now(),
                documents: Some(1),
                bytes: Some(8),
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 30,
                configuration_fingerprint: None,
                complete: None,
                error: None,
            },
        )
        .expect_err("symlink must be rejected");

        assert!(error.to_string().contains("symlinked validation path"));
        assert_eq!(
            std::fs::read_to_string(target).expect("read target"),
            "unchanged"
        );
    }
}
