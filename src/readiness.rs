use std::path::Path;
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::config::Config;
use crate::embed::Embedder;
use crate::service;
use crate::store::Store;

#[derive(Clone, Debug, Serialize)]
pub struct ReadinessReport {
    pub passed: bool,
    pub query_mode: String,
    pub checks: Vec<ReadinessCheck>,
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
    let mut checks = Vec::new();
    checks.push(database_check(store));
    checks.push(embedding_check(embedder).await);
    checks.push(api_check(api_url).await);
    checks.push(backup_check(
        &config.data_dir.join("backups"),
        max_backup_age_hours,
    ));
    checks.push(sync_check(allow_sync_service));
    checks.push(ReadinessCheck {
        name: "query-mode".into(),
        passed: true,
        detail: if config.query.synthesis_enabled {
            format!("synthesis enabled with model {}", config.query.model)
        } else {
            "deterministic extractive fallback".into()
        },
    });
    ReadinessReport {
        passed: checks.iter().all(|check| check.passed),
        query_mode: if config.query.synthesis_enabled {
            "synthesis"
        } else {
            "extractive"
        }
        .into(),
        checks,
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
                entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
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

    #[test]
    fn backup_freshness_requires_a_sqlite_snapshot() {
        let directory = tempdir().expect("temporary directory");
        assert!(!backup_check(directory.path(), 48).passed);
        File::create(directory.path().join("backup.sqlite3")).expect("backup fixture");
        assert!(backup_check(directory.path(), 48).passed);
    }
}
