use std::path::Path;
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::answer;
use crate::config::Config;
use crate::embed::Embedder;
use crate::service;
use crate::source_validation;
use crate::store::Store;

const READINESS_EMBEDDING_TIMEOUT_MIN_SECONDS: u64 = 15;
const READINESS_EMBEDDING_TIMEOUT_MAX_SECONDS: u64 = 300;
const READINESS_STORAGE_TIMEOUT_MIN_SECONDS: u64 = 1;
const READINESS_STORAGE_TIMEOUT_MAX_SECONDS: u64 = 300;
pub const READINESS_STORAGE_TIMEOUT_DEFAULT_SECONDS: u64 = 240;

#[derive(Clone, Debug, Serialize)]
pub struct ReadinessReport {
    pub passed: bool,
    pub query_mode: String,
    pub embedding_generation: EmbeddingGeneration,
    pub checks: Vec<ReadinessCheck>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EmbeddingGeneration {
    pub stored: Option<String>,
    pub configured: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReadinessCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

pub async fn run(
    config: &Config,
    store: &Store,
    embedder: &dyn Embedder,
    api_url: &str,
    max_backup_age_hours: u64,
    storage_timeout_seconds: u64,
    allow_sync_service: bool,
) -> ReadinessReport {
    let embedding_generation = embedding_generation_status(store, embedder);
    let embedding_index = embedding_index_check(store, embedder);
    let acl = public_acl_check(config, store);
    let sync = sync_check(allow_sync_service);
    let validation = source_validation_check(config, allow_sync_service);
    let embedding_timeout = embedding_probe_timeout(config);
    let storage_timeout = storage_probe_timeout(storage_timeout_seconds);
    let backup_directory = config.data_dir.join("backups");
    // These probes do not share mutable state. Run them together so readiness
    // is bounded by the slowest external dependency rather than their sum.
    let (database, backup, embedding, api, query) = tokio::join!(
        database_check(store, storage_timeout),
        backup_check(&backup_directory, max_backup_age_hours, storage_timeout),
        embedding_check(embedder, embedding_timeout),
        api_check(api_url),
        query_check(config),
    );
    let checks = vec![
        database,
        embedding_index,
        acl,
        embedding,
        api,
        backup,
        sync,
        validation,
        query,
    ];
    ReadinessReport {
        passed: checks.iter().all(|check| check.passed),
        query_mode: if config.query.synthesis_enabled {
            "synthesis"
        } else {
            "extractive"
        }
        .into(),
        embedding_generation,
        checks,
    }
}

async fn query_check(config: &Config) -> ReadinessCheck {
    if !config.query.synthesis_enabled {
        return ReadinessCheck {
            name: "query-mode".into(),
            passed: true,
            detail: "deterministic extractive fallback".into(),
        };
    }
    let api_key = match config.query.api_key_env.as_deref() {
        Some(name) => match config.environment_value(name) {
            Some(value) => Some(value),
            None => {
                return ReadinessCheck {
                    name: "query-model".into(),
                    passed: false,
                    detail: format!("{name} is not set"),
                };
            }
        },
        None => None,
    };
    if config.query.model.trim().is_empty() {
        return ReadinessCheck {
            name: "query-model".into(),
            passed: false,
            detail: "query model must not be empty when synthesis is enabled".into(),
        };
    }
    match answer::probe_configured_model(&config.query, api_key).await {
        Ok(()) => ReadinessCheck {
            name: "query-model".into(),
            passed: true,
            detail: format!("{} passed the grounded synthesis probe", config.query.model),
        },
        Err(error) => ReadinessCheck {
            name: "query-model".into(),
            passed: false,
            detail: error.to_string(),
        },
    }
}

fn public_acl_check(config: &Config, store: &Store) -> ReadinessCheck {
    match store.public_acl_summary() {
        Ok(summary) => {
            let public = summary
                .iter()
                .map(|project| project.documents)
                .sum::<usize>();
            let shared = !config.auth.tokens.is_empty();
            ReadinessCheck {
                name: "shared-access-acl".into(),
                passed: !shared || public == 0,
                detail: if shared && public > 0 {
                    format!(
                        "{public} public legacy documents remain; run `cortana acl plan` before shared access"
                    )
                } else if shared {
                    "shared principals configured and no public legacy documents remain".into()
                } else {
                    format!(
                        "{public} public documents are reachable only through the local-owner deployment"
                    )
                },
            }
        }
        Err(error) => ReadinessCheck {
            name: "shared-access-acl".into(),
            passed: false,
            detail: error.to_string(),
        },
    }
}

fn database_check_blocking(store: &Store) -> ReadinessCheck {
    match store.integrity_check() {
        Ok(()) => ReadinessCheck {
            name: "database-integrity".into(),
            passed: true,
            detail: "SQLite integrity_check returned ok".into(),
        },
        Err(error) => ReadinessCheck {
            name: "database-integrity".into(),
            passed: false,
            detail: error.to_string(),
        },
    }
}

async fn database_check(store: &Store, timeout: Duration) -> ReadinessCheck {
    let store = store.clone();
    run_blocking_probe("database-integrity", timeout, move || {
        database_check_blocking(&store)
    })
    .await
}

fn embedding_index_check(store: &Store, embedder: &dyn Embedder) -> ReadinessCheck {
    let configured_fingerprint = embedder.fingerprint();
    match store.stats() {
        Ok(stats) => match stats.embedding_fingerprint {
            None => ReadinessCheck {
                name: "embedding-index".into(),
                passed: true,
                detail: "index has no embedding generation yet; the first write will initialize it"
                    .into(),
            },
            Some(index_fingerprint) if index_fingerprint == configured_fingerprint => {
                ReadinessCheck {
                    name: "embedding-index".into(),
                    passed: true,
                    detail: format!("index generation matches {index_fingerprint}"),
                }
            }
            Some(index_fingerprint) => ReadinessCheck {
                name: "embedding-index".into(),
                passed: false,
                detail: format!(
                    "index uses {index_fingerprint}, but the configured provider uses {configured_fingerprint}; rebuild into a new generation before semantic retrieval or ingestion, or explicitly adopt this exact generation with `cortana migrate-embedding --from '{index_fingerprint}' --force` only after verifying the vectors are interchangeable",
                ),
            },
        },
        Err(error) => ReadinessCheck {
            name: "embedding-index".into(),
            passed: false,
            detail: error.to_string(),
        },
    }
}

fn embedding_generation_status(store: &Store, embedder: &dyn Embedder) -> EmbeddingGeneration {
    EmbeddingGeneration {
        stored: store
            .stats()
            .ok()
            .and_then(|stats| stats.embedding_fingerprint),
        configured: embedder.fingerprint(),
    }
}

fn embedding_probe_timeout(config: &Config) -> Duration {
    let timeout = config.embedding.service.startup_timeout_seconds.clamp(
        READINESS_EMBEDDING_TIMEOUT_MIN_SECONDS,
        READINESS_EMBEDDING_TIMEOUT_MAX_SECONDS,
    );
    Duration::from_secs(timeout)
}

fn storage_probe_timeout(seconds: u64) -> Duration {
    Duration::from_secs(seconds.clamp(
        READINESS_STORAGE_TIMEOUT_MIN_SECONDS,
        READINESS_STORAGE_TIMEOUT_MAX_SECONDS,
    ))
}

async fn run_blocking_probe<F>(name: &'static str, timeout: Duration, check: F) -> ReadinessCheck
where
    F: FnOnce() -> ReadinessCheck + Send + 'static,
{
    let (sender, receiver) = tokio::sync::oneshot::channel();
    if let Err(error) = std::thread::Builder::new()
        .name(format!("cortana-readiness-{name}"))
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(check));
            let _ = sender.send(result.map_err(|_| ()));
        })
    {
        return ReadinessCheck {
            name: name.into(),
            passed: false,
            detail: format!("{name} probe thread failed to start: {error}"),
        };
    }
    match tokio::time::timeout(timeout, receiver).await {
        Ok(Ok(Ok(check))) => check,
        Ok(Ok(Err(()))) => ReadinessCheck {
            name: name.into(),
            passed: false,
            detail: format!("{name} probe worker panicked"),
        },
        Ok(Err(_)) => ReadinessCheck {
            name: name.into(),
            passed: false,
            detail: format!("{name} probe worker ended before returning a result"),
        },
        Err(_) => ReadinessCheck {
            name: name.into(),
            passed: false,
            detail: format!("{name} probe timed out after {timeout:?}"),
        },
    }
}

async fn embedding_check(embedder: &dyn Embedder, startup_timeout: Duration) -> ReadinessCheck {
    match tokio::time::timeout(startup_timeout, embedder.probe()).await {
        Ok(Ok(())) => ReadinessCheck {
            name: "embedding-provider".into(),
            passed: true,
            detail: "embedding probe returned a vector".into(),
        },
        Ok(Err(error)) => ReadinessCheck {
            name: "embedding-provider".into(),
            passed: false,
            detail: error.to_string(),
        },
        Err(_) => ReadinessCheck {
            name: "embedding-provider".into(),
            passed: false,
            detail: format!(
                "embedding probe exceeded {} seconds",
                startup_timeout.as_secs()
            ),
        },
    }
}

async fn api_check(api_url: &str) -> ReadinessCheck {
    let endpoint = format!("{}/healthz", api_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return ReadinessCheck {
                name: "query-api".into(),
                passed: false,
                detail: error.to_string(),
            };
        }
    };
    match client.get(&endpoint).send().await {
        Ok(response) if response.status().is_success() => ReadinessCheck {
            name: "query-api".into(),
            passed: true,
            detail: format!("{endpoint} returned {}", response.status()),
        },
        Ok(response) => ReadinessCheck {
            name: "query-api".into(),
            passed: false,
            detail: format!("{endpoint} returned {}", response.status()),
        },
        Err(error) => ReadinessCheck {
            name: "query-api".into(),
            passed: false,
            detail: error.to_string(),
        },
    }
}

fn backup_check_blocking(directory: &Path, max_age_hours: u64) -> ReadinessCheck {
    let mut candidates = std::fs::read_dir(directory)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "sqlite3")
        })
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            if !metadata.file_type().is_file() {
                return None;
            }
            metadata.modified().ok().map(|modified| (path, modified))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.1.cmp(&left.1));
    let mut invalid = Vec::new();
    let Some((path, modified)) = candidates.into_iter().find(|(path, _)| {
        if Store::verify(path).is_ok() {
            true
        } else {
            invalid.push(path.display().to_string());
            false
        }
    }) else {
        let detail = if invalid.is_empty() {
            format!("no verified SQLite backup found in {}", directory.display())
        } else {
            format!(
                "no verified SQLite backup found in {}; invalid candidates: {}",
                directory.display(),
                invalid.join(", ")
            )
        };
        return ReadinessCheck {
            name: "backup-freshness".into(),
            passed: false,
            detail,
        };
    };
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    let maximum = Duration::from_secs(max_age_hours.saturating_mul(3600));
    ReadinessCheck {
        name: "backup-freshness".into(),
        passed: age <= maximum,
        detail: format!(
            "{} is a verified backup {} hours old (maximum {}){}",
            path.display(),
            age.as_secs() / 3600,
            max_age_hours,
            if invalid.is_empty() {
                String::new()
            } else {
                format!("; ignored {} invalid newer candidate(s)", invalid.len())
            }
        ),
    }
}

async fn backup_check(directory: &Path, max_age_hours: u64, timeout: Duration) -> ReadinessCheck {
    let directory = directory.to_path_buf();
    run_blocking_probe("backup-freshness", timeout, move || {
        backup_check_blocking(&directory, max_age_hours)
    })
    .await
}

fn sync_check(allow_sync_service: bool) -> ReadinessCheck {
    let installed = service::sync_job_installed();
    let service_status = installed.then(service::status);
    let active = service_status.as_ref().is_some_and(|result| {
        result
            .as_ref()
            .ok()
            .and_then(|report| {
                report
                    .services
                    .iter()
                    .find(|service| service.name == "sync")
            })
            .is_some_and(sync_service_is_active)
    });
    let status_error = service_status
        .as_ref()
        .and_then(|result| result.as_ref().err());
    ReadinessCheck {
        name: "sync-service".into(),
        passed: !installed || (allow_sync_service && active),
        detail: if installed {
            if let Some(error) = status_error {
                format!("installed but service status could not be read: {error:#}")
            } else if !allow_sync_service {
                "installed; rerun with --allow-sync-service only after source validation".into()
            } else if active {
                "installed, loaded, and explicitly allowed for this readiness check".into()
            } else {
                "installed but not loaded or active; start the sync service before readiness".into()
            }
        } else {
            "not installed (safe query-only operation)".into()
        },
    }
}

fn sync_service_is_active(service: &service::ServiceStatus) -> bool {
    if !service.installed || !service.loaded {
        return false;
    }
    matches!(
        service.state.as_deref(),
        Some("active" | "queued" | "ready" | "running" | "waiting")
    )
}

/// Verify the recurring-sync validation gate without touching any connector.
///
/// Query-only readiness never requires source validation, mirroring the
/// `sync-service` check: an operator blesses recurring sync explicitly with
/// `--allow-sync-service`, and only then does every enabled source need a
/// current successful validation at equal or larger budgets than its
/// configured limits. This closes the loop with `ensure_recurring_sync_validated`
/// (the install gate) and `require_success` (the per-source gate): a source
/// whose configuration changed, whose validation failed or lapsed, or whose
/// budget grew after the last `validate-source` run fails production readiness
/// even though the scheduled job would still attempt the sync.
fn source_validation_check(config: &Config, allow_sync_service: bool) -> ReadinessCheck {
    if !allow_sync_service {
        return ReadinessCheck {
            name: "source-validation".into(),
            passed: true,
            detail: "not required for query-only readiness; rerun with --allow-sync-service to verify recurring-sync source validation"
                .into(),
        };
    }
    if config.ingestion.validation_max_age_hours == 0 {
        return ReadinessCheck {
            name: "source-validation".into(),
            passed: false,
            detail: "reconciling sync requires a positive ingestion.validation_max_age_hours; set it to at least 1 hour"
                .into(),
        };
    }
    let mut problems = Vec::new();
    for source in config.sources.iter().filter(|source| source.enabled) {
        // Mirror SourceLimits::resolve in main.rs so readiness blesses the same
        // budgets the recurring sync job and install gate resolve.
        let max_documents = source
            .max_documents
            .unwrap_or(config.ingestion.max_documents_per_source);
        let max_bytes = source
            .max_bytes
            .unwrap_or(config.ingestion.max_bytes_per_source);
        let max_seconds = source
            .max_duration_seconds
            .unwrap_or(config.ingestion.max_duration_seconds);
        if max_documents == 0 {
            problems.push(format!(
                "source {} requires a positive document budget",
                source.name
            ));
        } else if max_bytes == 0 {
            problems.push(format!(
                "source {} requires a positive byte budget",
                source.name
            ));
        } else if max_seconds == 0 {
            problems.push(format!(
                "source {} requires a positive duration budget",
                source.name
            ));
        } else if let Err(error) = source_validation::require_success(
            &config.data_dir,
            source,
            max_documents,
            max_bytes,
            max_seconds,
            chrono::Duration::hours(config.ingestion.validation_max_age_hours as i64),
            // Recurring sync reconciles the full corpus, so a bounded sample
            // validation must never satisfy this gate.
            true,
        ) {
            problems.push(format!("{error}"));
        }
    }
    ReadinessCheck {
        name: "source-validation".into(),
        passed: problems.is_empty(),
        detail: if problems.is_empty() {
            if config.sources.iter().any(|source| source.enabled) {
                "every enabled source has a current successful validation at its configured budgets"
                    .to_string()
            } else {
                "no enabled sources require validation".to_string()
            }
        } else {
            problems.join("; ")
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use tempfile::tempdir;

    use super::*;
    use crate::config::{AuthTokenConfig, SourceConfig};
    use crate::model::Document;
    use crate::source_validation::SourceValidationStatus;

    struct DelayedProbeEmbedder {
        delay: Duration,
    }

    #[async_trait::async_trait]
    impl Embedder for DelayedProbeEmbedder {
        async fn embed(&self, _input: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            tokio::time::sleep(self.delay).await;
            Ok(vec![vec![0.25]])
        }

        fn fingerprint(&self) -> String {
            "delayed-probe-embedder".into()
        }
    }

    #[tokio::test]
    async fn embedding_check_passes_when_probe_finishes_after_15s_within_budget() {
        let embedder = DelayedProbeEmbedder {
            delay: Duration::from_secs(16),
        };

        let check = embedding_check(&embedder, Duration::from_secs(20)).await;

        assert!(check.passed);
        assert_eq!(check.name, "embedding-provider");
    }

    #[tokio::test]
    async fn embedding_check_fails_when_probe_exceeds_embedded_timeout() {
        let embedder = DelayedProbeEmbedder {
            delay: Duration::from_secs(16),
        };

        let check = embedding_check(&embedder, Duration::from_secs(10)).await;
        assert!(!check.passed);
        assert_eq!(check.detail, "embedding probe exceeded 10 seconds");
    }

    #[test]
    fn embedding_probe_timeout_is_bounded_by_readiness_defaults() {
        let mut config = Config::default();
        config.embedding.service.startup_timeout_seconds = 120;
        assert_eq!(embedding_probe_timeout(&config), Duration::from_secs(120));

        config.embedding.service.startup_timeout_seconds = 10;
        assert_eq!(
            embedding_probe_timeout(&config),
            Duration::from_secs(READINESS_EMBEDDING_TIMEOUT_MIN_SECONDS)
        );

        config.embedding.service.startup_timeout_seconds = 3600;
        assert_eq!(
            embedding_probe_timeout(&config),
            Duration::from_secs(READINESS_EMBEDDING_TIMEOUT_MAX_SECONDS)
        );
    }

    #[test]
    fn storage_probe_timeout_is_bounded_by_readiness_defaults() {
        assert_eq!(
            storage_probe_timeout(0),
            Duration::from_secs(READINESS_STORAGE_TIMEOUT_MIN_SECONDS)
        );
        assert_eq!(storage_probe_timeout(30), Duration::from_secs(30));
        assert_eq!(
            storage_probe_timeout(u64::MAX),
            Duration::from_secs(READINESS_STORAGE_TIMEOUT_MAX_SECONDS)
        );
    }

    #[tokio::test]
    async fn blocking_probe_timeout_fails_closed_with_degraded_detail() {
        let check = run_blocking_probe("database-integrity", Duration::from_millis(1), || {
            std::thread::sleep(Duration::from_millis(25));
            ReadinessCheck {
                name: "database-integrity".into(),
                passed: true,
                detail: "unexpected success".into(),
            }
        })
        .await;

        assert!(!check.passed);
        assert_eq!(check.name, "database-integrity");
        assert!(check.detail.contains("probe timed out"));
    }

    #[test]
    fn blocking_probe_timeout_does_not_hold_runtime_shutdown() {
        let started = std::time::Instant::now();
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let check = runtime.block_on(run_blocking_probe(
            "database-integrity",
            Duration::from_millis(1),
            || {
                std::thread::sleep(Duration::from_millis(500));
                ReadinessCheck {
                    name: "database-integrity".into(),
                    passed: true,
                    detail: "unexpected success".into(),
                }
            },
        ));
        drop(runtime);

        assert!(!check.passed);
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "timed-out readiness worker held runtime shutdown"
        );
    }

    #[test]
    fn backup_freshness_requires_a_sqlite_snapshot() {
        let directory = tempdir().expect("temporary directory");
        assert!(!backup_check_blocking(directory.path(), 48).passed);
        File::create(directory.path().join("backup.sqlite3")).expect("invalid backup fixture");
        assert!(!backup_check_blocking(directory.path(), 48).passed);
        let store = Store::open(&directory.path().join("source.sqlite3")).expect("source store");
        store
            .backup(&directory.path().join("verified.sqlite3"))
            .expect("verified backup fixture");
        assert!(backup_check_blocking(directory.path(), 48).passed);
    }

    #[test]
    fn sync_service_requires_a_loaded_nonterminal_state() {
        let active = service::ServiceStatus {
            name: "sync",
            label: "ai.cortana.sync",
            installed: true,
            loaded: true,
            state: Some("waiting".into()),
            pid: None,
            last_exit_status: None,
        };
        assert!(sync_service_is_active(&active));

        for state in ["dead", "failed", "inactive", "unknown", "not loaded"] {
            let status = service::ServiceStatus {
                name: "sync",
                label: "ai.cortana.sync",
                installed: true,
                loaded: true,
                state: Some(state.into()),
                pid: None,
                last_exit_status: None,
            };
            assert!(!sync_service_is_active(&status), "state={state}");
        }

        for state in ["loaded", "starting", "stopped"] {
            let status = service::ServiceStatus {
                name: "sync",
                label: "ai.cortana.sync",
                installed: true,
                loaded: true,
                state: Some(state.into()),
                pid: None,
                last_exit_status: None,
            };
            assert!(!sync_service_is_active(&status), "state={state}");
        }

        let unloaded = service::ServiceStatus {
            name: "sync",
            label: "ai.cortana.sync",
            installed: true,
            loaded: false,
            state: Some("waiting".into()),
            pid: None,
            last_exit_status: None,
        };
        assert!(!sync_service_is_active(&unloaded));
    }

    #[test]
    fn backup_freshness_uses_an_older_verified_backup_when_newest_is_corrupt() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("source.sqlite3")).expect("source store");
        store
            .backup(&directory.path().join("verified.sqlite3"))
            .expect("verified backup fixture");
        std::thread::sleep(Duration::from_millis(5));
        File::create(directory.path().join("newest.sqlite3")).expect("invalid backup fixture");

        let check = backup_check_blocking(directory.path(), 48);
        assert!(check.passed);
        assert!(check.detail.contains("verified backup"));
        assert!(check.detail.contains("ignored 1 invalid newer candidate"));
    }

    #[cfg(unix)]
    #[test]
    fn backup_freshness_ignores_symlinked_sqlite_files() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().expect("temporary directory");
        let outside = tempdir().expect("external temporary directory");
        let target = outside.path().join("external.sqlite3");
        File::create(&target).expect("external backup fixture");
        symlink(&target, directory.path().join("backup.sqlite3")).expect("backup symlink");

        assert!(!backup_check_blocking(directory.path(), 48).passed);
    }

    #[test]
    fn shared_mode_fails_readiness_while_legacy_public_rows_remain() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        store
            .upsert(
                &Document {
                    source: "test".into(),
                    source_id: "public".into(),
                    title: "Public".into(),
                    content: "legacy".into(),
                    uri: None,
                    updated_at: chrono::Utc::now(),
                    project: "work".into(),
                    acl: Vec::new(),
                    metadata: serde_json::json!({}),
                },
                &[("legacy".into(), vec![1.0])],
            )
            .expect("public document");
        let mut config = Config::default();
        assert!(public_acl_check(&config, &store).passed);
        config.auth.tokens.push(AuthTokenConfig {
            principal: "shared".into(),
            token_env: "SHARED_TOKEN".into(),
            scopes: vec!["query".into()],
            acl: vec!["work".into()],
        });
        assert!(!public_acl_check(&config, &store).passed);
    }

    #[test]
    fn embedding_index_reports_a_generation_mismatch_without_rebuilding() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        store
            .ensure_fingerprint("deterministic:16")
            .expect("fingerprint");
        let embedder = crate::embed::DeterministicEmbedder::new(32);

        let check = embedding_index_check(&store, &embedder);
        assert!(!check.passed);
        assert_eq!(check.name, "embedding-index");
        assert!(check.detail.contains("deterministic:16"));
        assert!(check.detail.contains("deterministic:32"));
        assert!(check.detail.contains("rebuild into a new generation"));
    }

    fn configured_source(enabled: bool) -> SourceConfig {
        SourceConfig {
            name: "drive".into(),
            kind: "google-drive".into(),
            enabled,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            teams: Vec::new(),
            team_names: Vec::new(),
            communities: Vec::new(),
            community_names: Vec::new(),
            repositories: Vec::new(),
            token_env: None,
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

    fn succeeded_validation(
        source: &SourceConfig,
        max_documents: usize,
        max_bytes: u64,
        max_seconds: u64,
    ) -> SourceValidationStatus {
        SourceValidationStatus {
            source: source.name.clone(),
            project: source.project.clone(),
            kind: source.kind.clone(),
            status: "succeeded".into(),
            validated_at: chrono::Utc::now(),
            documents: Some(5),
            bytes: Some(256),
            max_documents,
            max_bytes,
            max_seconds,
            configuration_fingerprint: source_validation::configuration_fingerprint(source).ok(),
            complete: Some(true),
            error: None,
        }
    }

    fn config_with_source(source: SourceConfig) -> (tempfile::TempDir, Config) {
        let directory = tempdir().expect("temporary directory");
        let config = Config {
            data_dir: directory.path().to_path_buf(),
            sources: vec![source],
            ..Config::default()
        };
        (directory, config)
    }

    #[test]
    fn source_validation_is_not_required_for_query_only_readiness() {
        let (directory, config) = config_with_source(configured_source(true));
        let check = source_validation_check(&config, false);
        assert!(check.passed);
        assert_eq!(check.name, "source-validation");
        assert!(check.detail.contains("--allow-sync-service"));
        assert!(
            !source_validation::load(directory.path())
                .expect("empty state")
                .contains_key("drive")
        );
    }

    #[test]
    fn source_validation_requires_a_current_success_before_blessing_sync() {
        let (directory, config) = config_with_source(configured_source(true));

        let missing = source_validation_check(&config, true);
        assert!(!missing.passed);
        assert!(missing.detail.contains("drive has not been validated"));

        let source = &config.sources[0];
        source_validation::record(directory.path(), succeeded_validation(source, 25, 1024, 60))
            .expect("validation");
        let passing = source_validation_check(&config, true);
        assert!(passing.passed);
        assert!(passing.detail.contains("every enabled source"));
    }

    #[test]
    fn source_validation_detects_configuration_drift() {
        let (directory, mut config) = config_with_source(configured_source(true));
        let source = &config.sources[0];
        source_validation::record(directory.path(), succeeded_validation(source, 25, 1024, 60))
            .expect("validation");

        config.sources[0].query = Some("from:someone@example.com".into());
        let changed = source_validation_check(&config, true);
        assert!(!changed.passed);
        assert!(
            changed
                .detail
                .contains("configuration changed since validation")
        );
    }

    #[test]
    fn source_validation_tracks_budget_growth_without_configuration_changes() {
        // The validation fingerprint covers only the source entry. Raising the
        // [ingestion] defaults behind an override-less source grows the resolved
        // budget without changing the fingerprint, so require_success must catch
        // the growth through its limit comparison instead.
        let base = configured_source(true);
        let override_less = SourceConfig {
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            ..base
        };
        let (directory, mut config) = config_with_source(override_less);
        config.ingestion.max_documents_per_source = 25;
        config.ingestion.max_bytes_per_source = 1024;
        config.ingestion.max_duration_seconds = 60;
        source_validation::record(
            directory.path(),
            succeeded_validation(&config.sources[0], 25, 1024, 60),
        )
        .expect("validation");
        assert!(source_validation_check(&config, true).passed);

        config.ingestion.max_documents_per_source = 26;
        let grown = source_validation_check(&config, true);
        assert!(!grown.passed);
        assert!(
            grown
                .detail
                .contains("validation limits were smaller than this sync")
        );

        config.ingestion.max_documents_per_source = 25;
        config.ingestion.max_duration_seconds = 61;
        let longer = source_validation_check(&config, true);
        assert!(!longer.passed);
        assert!(
            longer
                .detail
                .contains("validation duration limit was smaller than this sync")
        );
    }

    #[test]
    fn source_validation_rejects_a_lapsed_validation_until_revalidated() {
        let (directory, config) = config_with_source(configured_source(true));
        let source = &config.sources[0];
        let mut lapsed = succeeded_validation(source, 25, 1024, 60);
        lapsed.validated_at = chrono::Utc::now() - chrono::Duration::days(30);
        source_validation::record(directory.path(), lapsed).expect("lapsed validation");

        let check = source_validation_check(&config, true);
        assert!(!check.passed);
        assert!(check.detail.contains("30 days old"));
        assert!(check.detail.contains("re-run validate-source"));

        let mut fresh = succeeded_validation(source, 25, 1024, 60);
        fresh.validated_at = chrono::Utc::now();
        source_validation::record(directory.path(), fresh).expect("fresh validation");
        assert!(source_validation_check(&config, true).passed);
    }

    #[test]
    fn source_validation_rejects_unbounded_freshness_for_recurring_sync() {
        let (directory, mut config) = config_with_source(configured_source(true));
        config.ingestion.validation_max_age_hours = 0;
        let source = &config.sources[0];
        let mut lapsed = succeeded_validation(source, 25, 1024, 60);
        lapsed.validated_at = chrono::Utc::now() - chrono::Duration::days(30);
        source_validation::record(directory.path(), lapsed).expect("lapsed validation");

        let check = source_validation_check(&config, true);
        assert!(!check.passed);
        assert!(
            check
                .detail
                .contains("requires a positive ingestion.validation_max_age_hours")
        );
    }

    #[test]
    fn source_validation_rejects_failed_state_positive_budgets_and_disabled_sources() {
        let (directory, config) = config_with_source(configured_source(true));
        let source = &config.sources[0];
        let mut failed = succeeded_validation(source, 25, 1024, 60);
        failed.status = "failed".into();
        failed.error = Some("connector error".into());
        source_validation::record(directory.path(), failed).expect("failed validation");
        let rejected = source_validation_check(&config, true);
        assert!(!rejected.passed);
        assert!(
            rejected
                .detail
                .contains("latest validation did not succeed")
        );

        let (directory, config) = config_with_source(configured_source(false));
        let check = source_validation_check(&config, true);
        assert!(check.passed);
        assert!(
            check
                .detail
                .contains("no enabled sources require validation")
        );
        assert!(
            !source_validation::load(directory.path())
                .expect("empty state")
                .contains_key("drive")
        );

        let (_, mut config) = config_with_source(configured_source(true));
        config.sources[0].max_documents = Some(0);
        let zero = source_validation_check(&config, true);
        assert!(!zero.passed);
        assert!(zero.detail.contains("requires a positive document budget"));
    }
}
