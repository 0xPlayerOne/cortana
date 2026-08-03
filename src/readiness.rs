use std::path::Path;
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::answer;
use crate::config::Config;
use crate::embed::Embedder;
use crate::service;
use crate::store::Store;

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
    allow_sync_service: bool,
) -> ReadinessReport {
    let database = database_check(store);
    let embedding_generation = embedding_generation_status(store, embedder);
    let embedding_index = embedding_index_check(store, embedder);
    let acl = public_acl_check(config, store);
    let backup = backup_check(&config.data_dir.join("backups"), max_backup_age_hours);
    let sync = sync_check(allow_sync_service);
    // These probes do not share mutable state. Run them together so readiness
    // is bounded by the slowest external dependency rather than their sum.
    let (embedding, api, query) = tokio::join!(
        embedding_check(embedder),
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

fn database_check(store: &Store) -> ReadinessCheck {
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

async fn embedding_check(embedder: &dyn Embedder) -> ReadinessCheck {
    match tokio::time::timeout(Duration::from_secs(15), embedder.probe()).await {
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
            detail: "embedding probe exceeded 15 seconds".into(),
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

fn backup_check(directory: &Path, max_age_hours: u64) -> ReadinessCheck {
    let latest = std::fs::read_dir(directory).ok().and_then(|entries| {
        entries
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "sqlite3")
            })
            .filter_map(|entry| {
                let metadata = std::fs::symlink_metadata(entry.path()).ok()?;
                if !metadata.file_type().is_file() {
                    return None;
                }
                metadata
                    .modified()
                    .ok()
                    .map(|modified| (entry.path(), modified))
            })
            .max_by_key(|(_, modified)| *modified)
    });
    let Some((path, modified)) = latest else {
        return ReadinessCheck {
            name: "backup-freshness".into(),
            passed: false,
            detail: format!("no SQLite backup found in {}", directory.display()),
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
            "{} is {} hours old (maximum {})",
            path.display(),
            age.as_secs() / 3600,
            max_age_hours
        ),
    }
}

fn sync_check(allow_sync_service: bool) -> ReadinessCheck {
    let installed = service::sync_job_installed();
    ReadinessCheck {
        name: "sync-service".into(),
        passed: !installed || allow_sync_service,
        detail: if installed {
            if allow_sync_service {
                "installed and explicitly allowed for this readiness check"
            } else {
                "installed; rerun with --allow-sync-service only after source validation"
            }
        } else {
            "not installed (safe query-only operation)"
        }
        .into(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use tempfile::tempdir;

    use super::*;
    use crate::config::AuthTokenConfig;
    use crate::model::Document;

    #[test]
    fn backup_freshness_requires_a_sqlite_snapshot() {
        let directory = tempdir().expect("temporary directory");
        assert!(!backup_check(directory.path(), 48).passed);
        File::create(directory.path().join("backup.sqlite3")).expect("backup fixture");
        assert!(backup_check(directory.path(), 48).passed);
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

        assert!(!backup_check(directory.path(), 48).passed);
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
}
