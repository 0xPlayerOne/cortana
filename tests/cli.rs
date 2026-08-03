use std::fs;

use assert_cmd::Command;
use cortana::retrieval;
use predicates::prelude::*;
use rusqlite::Connection;
use tempfile::tempdir;

#[test]
fn model_evaluation_is_opt_in_without_changing_safe_runtime_default() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let missing_key = "CORTANA_TEST_MISSING_QUERY_KEY_7E5B";
    fs::write(
        &config,
        format!(
            "data_dir = {:?}\n[query]\nsynthesis_enabled = false\napi_key_env = {missing_key:?}\n",
            directory.path().join("data")
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args(["eval", "--model"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(format!(
            "query API key environment variable {missing_key} is not set"
        )))
        .stderr(predicate::str::contains("query synthesis is not enabled").not());
}

#[test]
fn google_authorization_fails_closed_before_opening_browser() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let token = directory.path().join("token.json");
    fs::write(
        &config,
        format!(
            "data_dir = {:?}\n\
             [[sources]]\n\
             name = \"personal-drive\"\n\
             kind = \"google-drive\"\n\
             project = \"personal\"\n\
             token = {token:?}\n",
            directory.path().join("data")
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args(["authorize-google", "personal-drive"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Google source personal-drive requires OAuth client path",
        ))
        .stderr(predicate::str::contains("CORTANA_AUTHORIZATION_URL").not());
}

#[test]
fn guarded_sync_fails_before_opening_the_index_without_validation() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n\
             [embedding]\n\
             dimension = 256\n\
             [[sources]]\n\
             name = \"safe-source\"\n\
             kind = \"external\"\n\
             project = \"personal\"\n\
             command = [\"/usr/bin/false\"]\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args([
            "sync",
            "--source",
            "safe-source",
            "--require-validation",
            "--max-documents",
            "25",
            "--max-bytes",
            "5242880",
            "--max-seconds",
            "300",
            "--no-reconcile",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "source safe-source has not been validated",
        ));
    assert!(!data.join("cortana.sqlite3").exists());
}

#[test]
fn guarded_all_sources_sync_fails_before_opening_the_index_without_validation() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n\
             [embedding]\n\
             dimension = 256\n\
             [[sources]]\n\
             name = \"safe-source\"\n\
             kind = \"external\"\n\
             project = \"personal\"\n\
             command = [\"/usr/bin/false\"]\n\
             [[sources]]\n\
             name = \"second-source\"\n\
             kind = \"external\"\n\
             project = \"personal\"\n\
             command = [\"/usr/bin/false\"]\n"
        ),
    )
    .expect("write config");

    // The recurring sync job invokes this exact all-sources form; it must
    // re-check every enabled source before the index is opened.
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["sync", "--require-validation", "--no-reconcile"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "recurring sync requires a current successful validation for source safe-source",
        ))
        .stderr(predicate::str::contains(
            "source safe-source has not been validated",
        ));
    assert!(!data.join("cortana.sqlite3").exists());
}

#[test]
fn service_install_sync_option_fails_before_service_manager_execution_without_validation() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n\
             [embedding]\n\
             dimension = 256\n\
             [[sources]]\n\
             name = \"safe-source\"\n\
             kind = \"external\"\n\
             project = \"personal\"\n\
             command = [\"/usr/bin/false\"]\n"
        ),
    )
    .expect("write config");

    // Explicit recurring sync install must fail at source-validation gating before any
    // service-manager command is invoked.
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["service", "install", "--no-web", "--enable-sync-service"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "recurring sync requires a current successful validation for source safe-source",
        ))
        .stderr(predicate::str::contains(
            "source safe-source has not been validated",
        ));

    assert!(!data.join("source-validations.lock").exists());
    assert!(!data.join("cortana.sqlite3").exists());
}

#[test]
fn offline_ingest_and_search_round_trip() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\nbase_url = \"http://127.0.0.1:6999/v1\"\nmodel = \"Qwen/Qwen3-Embedding-0.6B\"\n"
        ),
    )
    .expect("write config");
    let document = r#"{"source":"test","source_id":"one","title":"Runbook","content":"The deployment uses a blue green release process.","project":"demo"}"#;

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["ingest", "-"])
        .write_stdin(document)
        .assert()
        .success()
        .stdout(predicate::str::contains("changed=1"));

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["search", "blue green", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"title\": \"Runbook\""));
}

#[test]
fn embedding_generation_migration_requires_confirmation_and_preserves_documents() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 256\nbase_url = \"http://127.0.0.1:6999/v1\"\nmodel = \"Qwen/Qwen3-Embedding-0.6B\"\n"
        ),
    )
    .expect("write config");
    let document = r#"{"source":"test","source_id":"one","title":"Runbook","content":"Keep the indexed document during generation adoption.","project":"demo"}"#;

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["ingest", "-"])
        .write_stdin(document)
        .assert()
        .success();

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args(["migrate-embedding", "--from", "deterministic:256"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("rerun with --force"));

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args([
            "migrate-embedding",
            "--from",
            "deterministic:256",
            "--force",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("embedding generation migrated"))
        .stdout(predicate::str::contains(
            "indexed documents were not rebuilt",
        ));

    let connection = Connection::open(data.join("cortana.sqlite3")).expect("open index");
    let (fingerprint, documents, cache_entries): (String, i64, i64) = connection
        .query_row(
            "SELECT
               (SELECT value FROM meta WHERE key='embedding_fingerprint'),
               (SELECT COUNT(*) FROM documents),
               (SELECT COUNT(*) FROM embedding_cache)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("migration state");
    assert_eq!(
        fingerprint,
        "openai:http://127.0.0.1:6999/v1:Qwen/Qwen3-Embedding-0.6B:256"
    );
    assert_eq!(documents, 1);
    assert_eq!(cache_entries, 0);
    assert!(
        fs::read_dir(data.join("backups"))
            .expect("backup directory")
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("cortana-embedding-migration-"))
    );
}

#[test]
fn offline_context_emits_cited_bundle_and_enforces_bounds() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\n\
             [query]\ncontext_tokens = 4096\n"
        ),
    )
    .expect("write config");
    let document = r#"{"source":"test","source_id":"one","title":"Runbook","content":"The deployment uses a blue green release process.","project":"demo"}"#;

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["ingest", "-"])
        .write_stdin(document)
        .assert()
        .success()
        .stdout(predicate::str::contains("changed=1"));

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["context", "blue green", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"context\""))
        .stdout(predicate::str::contains("### [1] Runbook"))
        .stdout(predicate::str::contains("\"max_tokens\": 4096"))
        .stdout(predicate::str::contains("\"metrics\""));

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args([
            "context",
            "blue green",
            "--project",
            "demo",
            "--limit",
            "3",
            "--max-tokens",
            "512",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"max_tokens\": 512"))
        .stdout(predicate::str::contains("\"retrieved\": 1"))
        .stdout(predicate::str::contains("\"included\": 1"));

    // Every CLI context call records a metadata-only audit row under the
    // owner-local principal; query text and evidence content are never written.
    let connection =
        Connection::open(data.join("cortana.sqlite3")).expect("open initialized index");
    let columns = connection
        .prepare("PRAGMA table_info(audit_events)")
        .expect("prepare audit schema")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("audit columns")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("audit column names");
    for forbidden in ["query", "content"] {
        assert!(
            !columns.iter().any(|name| name.contains(forbidden)),
            "audit_events must never store {forbidden} text"
        );
    }
    let (principal, action, outcome, project, result_count, latency_ms): (
        String,
        String,
        String,
        Option<String>,
        Option<i64>,
        i64,
    ) = connection
        .query_row(
            "SELECT principal, action, outcome, project, result_count, latency_ms
             FROM audit_events WHERE action = 'local-cli/context' ORDER BY id DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .expect("context audit row");
    assert_eq!(principal, "local-cli");
    assert_eq!(action, "local-cli/context");
    assert_eq!(outcome, "succeeded");
    assert_eq!(project.as_deref(), Some("demo"));
    assert_eq!(result_count, Some(1));
    assert!(latency_ms >= 0);

    // A runtime failure still records a failed audit row before returning.
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["context", &"x".repeat(retrieval::MAX_QUERY_BYTES + 1)])
        .assert()
        .failure()
        .stderr(predicate::str::contains("query exceeds"));
    let (outcome, result_count): (String, Option<i64>) = connection
        .query_row(
            "SELECT outcome, result_count FROM audit_events
             WHERE action = 'local-cli/context' ORDER BY id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("failed context audit row");
    assert_eq!(outcome, "failed");
    assert_eq!(result_count, None);

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args(["context", "blue green", "--limit", "51"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not in 1..=50"));
}

#[test]
fn audit_export_preserves_retained_metadata_without_query_content() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let export = directory.path().join("audit.jsonl");
    fs::write(
        &config,
        format!("data_dir = {data:?}\n[embedding]\ndimension = 256\n"),
    )
    .expect("write config");
    let document = r#"{"source":"test","source_id":"one","title":"Runbook","content":"The deployment uses a blue green release process.","project":"demo"}"#;

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["ingest", "-"])
        .write_stdin(document)
        .assert()
        .success();
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["context", "blue green", "--project", "demo"])
        .assert()
        .success();

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args(["audit", "export"])
        .arg(&export)
        .assert()
        .success()
        .stdout(predicate::str::contains("audit export wrote 1 events"));

    let exported_jsonl = fs::read_to_string(&export).expect("read audit export");
    let line = exported_jsonl.lines().next().expect("audit event line");
    let event: serde_json::Value = serde_json::from_str(line).expect("audit JSONL");
    assert_eq!(event["principal"], "local-cli");
    assert_eq!(event["action"], "local-cli/context");
    assert!(event.get("query").is_none());
    assert!(event.get("content").is_none());

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args(["audit", "export"])
        .arg(&export)
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));

    let json_export = directory.path().join("audit.json");
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args(["audit", "export", "--format", "json", "--limit", "1"])
        .arg(&json_export)
        .assert()
        .success();
    let events: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(json_export).expect("read JSON export"))
            .expect("audit JSON array");
    assert_eq!(events.as_array().map(Vec::len), Some(1));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o644);
        fs::set_permissions(&export, permissions).expect("loosen export permissions");
        Command::cargo_bin("cortana")
            .expect("binary exists")
            .args(["--config"])
            .arg(&config)
            .args(["audit", "export", "--force"])
            .arg(&export)
            .assert()
            .success();
        assert_eq!(
            fs::metadata(&export)
                .expect("export metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn preembedded_import_validates_and_searches_without_provider_calls() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    fs::write(
        &config,
        format!("data_dir = {data:?}\n[embedding]\ndimension = 256\n"),
    )
    .expect("write config");
    let record = serde_json::json!({
        "type": "document",
        "embedding_fingerprint": "deterministic:256",
        "document": {
            "source": "legacy-code",
            "source_id": "repo::runbook#0",
            "title": "repo/runbook.md",
            "content": "Imported deployment recovery procedure.",
            "project": "repo"
        },
        "chunks": [{
            "content": "Imported deployment recovery procedure.",
            "embedding": vec![0.0_f32; 256]
        }]
    })
    .to_string();
    let stream = format!("{record}\n{{\"type\":\"complete\",\"records\":1}}\n");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["import-embeddings", "-"])
        .write_stdin(stream)
        .assert()
        .success()
        .stdout(predicate::str::contains("changed=1"));

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["search", "deployment recovery", "--project", "repo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"title\": \"repo/runbook.md\""));

    let wrong_fingerprint = format!(
        "{}\n{{\"type\":\"complete\",\"records\":1}}\n",
        record.replace("deterministic:256", "another-model:256")
    );
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["import-embeddings", "-"])
        .write_stdin(wrong_fingerprint)
        .assert()
        .failure()
        .stderr(predicate::str::contains("embedding fingerprint mismatch"));
}

#[test]
fn configured_external_source_sync_is_incremental() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let input = directory.path().join("external.jsonl");
    let document = r#"{"source":"upstream","source_id":"one","title":"External","content":"Reusable context from an external connector.","project":"demo"}"#;
    fs::write(&input, document).expect("write external source");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\n\
             [[sources]]\nname = \"external-demo\"\nkind = \"external\"\nproject = \"demo\"\n\
             command = [\"/bin/cat\", {input:?}]\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["sync", "--source", "external-demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("changed=1 unchanged=0"));

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["sync", "--source", "external-demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("changed=0 unchanged=1"));
    assert_eq!(
        fs::read_dir(data.join("staging"))
            .expect("staging directory")
            .count(),
        0,
        "connector spools must be removed after a completed sync"
    );

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args([
            "search",
            "reusable context",
            "--project",
            "demo",
            "--source",
            "external-demo",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"title\": \"External\""));
}

#[test]
fn external_source_sync_reconciliation_deletes_and_preserves_records_with_no_reconcile() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let input = directory.path().join("external.jsonl");
    let two_records = r#"{"source":"upstream","source_id":"one","title":"Run one","content":"first","project":"demo"}
{"source":"upstream","source_id":"two","title":"Run two","content":"second","project":"demo"}
"#;

    fs::write(&input, two_records).expect("write initial external fixture");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\n\
             [[sources]]\nname = \"external-demo\"\nkind = \"external\"\nproject = \"demo\"\n\
             command = [\"/bin/cat\", {input:?}]\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["sync", "--source", "external-demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "synced source=external-demo deleted=0",
        ));

    let connection = Connection::open(data.join("cortana.sqlite3")).expect("open index");
    let documents: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM documents WHERE source='external-demo' AND project='demo'",
            [],
            |row| row.get(0),
        )
        .expect("initial document count");
    assert_eq!(documents, 2);

    fs::write(
        &input,
        r#"{"source":"upstream","source_id":"one","title":"Run one","content":"first","project":"demo"}
"#,
    )
    .expect("write single-record fixture");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["sync", "--source", "external-demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "synced source=external-demo deleted=1",
        ));

    let post_reconcile_documents: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM documents WHERE source='external-demo' AND project='demo'",
            [],
            |row| row.get(0),
        )
        .expect("post-reconcile document count");
    assert_eq!(post_reconcile_documents, 1);
    let deleted_record_exists: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM documents WHERE source='external-demo' AND source_id='two'",
            [],
            |row| row.get(0),
        )
        .expect("deleted record check");
    assert_eq!(deleted_record_exists, 0);

    fs::write(&input, two_records).expect("restore missing record in fixture");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["sync", "--source", "external-demo", "--no-reconcile"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "synced source=external-demo deleted=0",
        ));
    let final_documents: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM documents WHERE source='external-demo' AND project='demo'",
            [],
            |row| row.get(0),
        )
        .expect("post-no-reconcile document count");
    assert_eq!(final_documents, 2);
    drop(connection);

    fs::write(
        &input,
        r#"{"source":"upstream","source_id":"one","title":"Run one","content":"first","project":"demo"}
"#,
    )
    .expect("write partial non-reconciling fixture");
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["sync", "--source", "external-demo", "--no-reconcile"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "synced source=external-demo deleted=0",
        ));

    let connection = Connection::open(data.join("cortana.sqlite3")).expect("reopen index");
    let final_documents: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM documents WHERE source='external-demo' AND project='demo'",
            [],
            |row| row.get(0),
        )
        .expect("final document count");
    assert_eq!(
        final_documents, 2,
        "no-reconcile must preserve missing records"
    );
}

#[test]
fn source_failure_does_not_block_later_sources() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let input = directory.path().join("healthy.jsonl");
    fs::write(
        &input,
        r#"{"source":"upstream","source_id":"one","title":"Healthy","content":"Indexed after a failed source.","project":"demo"}"#,
    )
    .expect("write healthy source");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\n\
             [[sources]]\nname = \"broken\"\nkind = \"external\"\nproject = \"demo\"\n\
             command = [\"/usr/bin/false\"]\n\
             [[sources]]\nname = \"healthy\"\nkind = \"external\"\nproject = \"demo\"\n\
             command = [\"/bin/cat\", {input:?}]\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .arg("sync")
        .assert()
        .failure()
        .stdout(predicate::str::contains("synced source=healthy"))
        .stderr(predicate::str::contains(
            "source sync failed: source=broken",
        ));

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["search", "indexed after", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"title\": \"Healthy\""));
}

#[test]
fn connector_wall_clock_timeout_is_enforced() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\n\
             [connectors]\ntimeout_seconds = 1\n\
             [[sources]]\nname = \"wedged\"\nkind = \"external\"\nproject = \"demo\"\n\
             command = [\"/bin/sleep\", \"30\"]\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .arg("sync")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "connector wedged timed out after 1 seconds",
        ));
}

#[test]
fn sync_plan_is_read_only_and_can_inspect_a_disabled_source() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let source = directory.path().join("source");
    fs::create_dir(&source).expect("source directory");
    fs::write(source.join("one.rs"), "fn one() {}").expect("source file");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n\
             [[sources]]\nname = \"code\"\nkind = \"filesystem\"\nenabled = false\n\
             project = \"demo\"\nroot = {source:?}\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args(["sync", "--source", "code", "--plan"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"documents\":1"))
        .stdout(predicate::str::contains("\"enabled\":false"));

    assert!(
        !data.exists(),
        "plan mode must not initialize or modify the index"
    );
}

#[test]
fn source_validation_fetches_one_disabled_source_without_initializing_the_index() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let input = directory.path().join("external.jsonl");
    fs::write(
        &input,
        r#"{"source":"upstream","source_id":"one","title":"External","content":"Synthetic validation document.","project":"demo"}"#,
    )
    .expect("write external source");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n\
             [[sources]]\nname = \"external-demo\"\nkind = \"external\"\nenabled = false\n\
             project = \"demo\"\ncommand = [\"/bin/cat\", {input:?}]\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args([
            "validate-source",
            "external-demo",
            "--max-documents",
            "2",
            "--max-bytes",
            "4096",
            "--max-seconds",
            "5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"validated\":true"))
        .stdout(predicate::str::contains("\"documents\":0"))
        .stdout(predicate::str::contains("\"embeddings\":0"))
        .stdout(predicate::str::contains("\"reconciliations\":0"));

    assert!(
        !data.join("cortana.sqlite3").exists(),
        "validation must not initialize the index"
    );
    let validation: serde_json::Value = serde_json::from_slice(
        &fs::read(data.join("source-validations.json")).expect("validation state"),
    )
    .expect("validation JSON");
    assert_eq!(
        validation["sources"]["external-demo"]["status"],
        "succeeded"
    );
    assert_eq!(
        validation["sources"]["external-demo"]["documents"],
        serde_json::json!(1)
    );
    assert_eq!(
        fs::read_dir(data.join("staging"))
            .expect("staging directory")
            .count(),
        0,
        "validation spools must be removed"
    );
}

#[test]
fn connector_live_output_bound_stops_oversized_validation() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let input = directory.path().join("oversized.jsonl");
    fs::write(&input, "x".repeat(100_000)).expect("write oversized source");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n\
             [[sources]]\nname = \"oversized\"\nkind = \"external\"\nproject = \"demo\"\n\
             command = [\"/bin/cat\", {input:?}]\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args([
            "validate-source",
            "oversized",
            "--max-documents",
            "1",
            "--max-bytes",
            "1",
            "--max-seconds",
            "5",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "connector oversized exceeded its live output safety bound",
        ));
    assert!(
        !data.join("cortana.sqlite3").exists(),
        "failed validation must not initialize the index"
    );
    let validation: serde_json::Value = serde_json::from_slice(
        &fs::read(data.join("source-validations.json")).expect("failed validation state"),
    )
    .expect("validation JSON");
    assert_eq!(validation["sources"]["oversized"]["status"], "failed");
    assert!(
        validation["sources"]["oversized"]["error"]
            .as_str()
            .is_some_and(|error| error.contains("live output safety bound"))
    );
}

#[test]
fn acl_backfill_requires_matching_source_defaults_and_force() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 256\n\
             [[sources]]\nname = \"demo-source\"\nkind = \"external\"\nenabled = false\n\
             project = \"demo\"\nacl = [\"work\"]\ncommand = [\"/usr/bin/false\"]\n"
        ),
    )
    .expect("write config");
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["ingest", "-"])
        .write_stdin(
            r#"{"source":"demo-source","source_id":"legacy","title":"Legacy","content":"legacy public row","project":"demo"}"#,
        )
        .assert()
        .success();

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args(["acl", "plan", "--project", "demo=work"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"applied\":false"))
        .stdout(predicate::str::contains("\"documents\":1"))
        .stdout(predicate::str::contains("\"source_alignment_errors\":[]"));
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args(["acl", "apply", "--project", "demo=work"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("ACL apply requires --force"));
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .args(["acl", "apply", "--project", "demo=work", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"documents_changed\":1"));

    let connection =
        Connection::open(data.join("cortana.sqlite3")).expect("open initialized index");
    let acl: String = connection
        .query_row("SELECT acl_json FROM documents", [], |row| row.get(0))
        .expect("document ACL");
    assert_eq!(acl, r#"["work"]"#);
}

#[test]
fn source_budget_rejects_snapshot_before_partial_ingestion() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let input = directory.path().join("external.jsonl");
    fs::write(
        &input,
        concat!(
            "{\"source\":\"upstream\",\"source_id\":\"one\",\"title\":\"One\",",
            "\"content\":\"first document\",\"project\":\"demo\"}\n",
            "{\"source\":\"upstream\",\"source_id\":\"two\",\"title\":\"Two\",",
            "\"content\":\"second document\",\"project\":\"demo\"}\n"
        ),
    )
    .expect("write external source");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\n\
             [ingestion]\nmax_documents_per_source = 1\n\
             [[sources]]\nname = \"external-demo\"\nkind = \"external\"\nproject = \"demo\"\n\
             command = [\"/bin/cat\", {input:?}]\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["sync", "--source", "external-demo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "source external-demo exceeds the 1 document budget",
        ));

    let connection =
        Connection::open(data.join("cortana.sqlite3")).expect("open initialized index");
    let documents: i64 = connection
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .expect("document count");
    assert_eq!(
        documents, 0,
        "a failed preflight must not partially ingest the snapshot"
    );
    let sync_status: String = connection
        .query_row(
            "SELECT status FROM sync_runs WHERE source='external-demo'",
            [],
            |row| row.get(0),
        )
        .expect("sync status");
    assert_eq!(sync_status, "budget_exceeded");
}

#[test]
fn bounded_connector_sync_accepts_its_permitted_one_document_scope() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let input = directory.path().join("drive.jsonl");
    fs::write(
        &input,
        concat!(
            "{\"source\":\"google-drive\",\"source_id\":\"one\",\"title\":\"One\",",
            "\"content\":\"first document\",\"project\":\"demo\"}\n",
            "{\"source\":\"google-drive\",\"source_id\":\"two\",\"title\":\"Two\",",
            "\"content\":\"second document\",\"project\":\"demo\"}\n"
        ),
    )
    .expect("write drive source");
    // The fake built-in connector verifies the bounded sync passes the
    // upstream document cap (without --no-cache) and then emits its whole
    // fixture; the cap is what keeps the spool inside the live output safety
    // bound and the sync inside its permitted one-document scope.
    let connector = format!(
        "case \" $* \" in *\" --max-documents 1 \"*) ;; *) exit 1 ;; esac; head -n 1 {input:?}\n"
    );
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\n[connectors]\n\
             command = [\"/bin/sh\", \"-c\", {connector:?}]\n\
             [[sources]]\nname = \"bounded-drive\"\nkind = \"google-drive\"\nproject = \"demo\"\n\
             token = \"/tmp/fake-google-token.json\"\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args([
            "sync",
            "--source",
            "bounded-drive",
            "--no-reconcile",
            "--max-documents",
            "1",
            "--max-bytes",
            "1048576",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("synced source=bounded-drive"))
        .stdout(predicate::str::contains("changed=1"));

    let connection =
        Connection::open(data.join("cortana.sqlite3")).expect("open initialized index");
    let documents: i64 = connection
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .expect("document count");
    assert_eq!(
        documents, 1,
        "only the permitted one-document scope is ingested"
    );
    let source_id: String = connection
        .query_row("SELECT source_id FROM documents", [], |row| row.get(0))
        .expect("ingested source id");
    assert_eq!(source_id, "one");
    let sync_status: String = connection
        .query_row(
            "SELECT status FROM sync_runs WHERE source='bounded-drive'",
            [],
            |row| row.get(0),
        )
        .expect("sync status");
    assert_eq!(sync_status, "succeeded");
    let sync_documents: i64 = connection
        .query_row(
            "SELECT documents FROM sync_runs WHERE source='bounded-drive'",
            [],
            |row| row.get(0),
        )
        .expect("sync document count");
    assert_eq!(sync_documents, 1);
}

#[test]
fn bounded_connector_sync_still_rejects_over_budget_content_bytes() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let input = directory.path().join("drive.jsonl");
    // One valid document whose searchable content alone exceeds the byte
    // budget. It stays below the live output safety bound (budget bytes plus
    // per-document headroom) so the spool validation must reject it.
    let content = "x".repeat(50_000);
    fs::write(
        &input,
        format!(
            "{{\"source\":\"google-drive\",\"source_id\":\"one\",\"title\":\"One\",\
             \"content\":\"{content}\",\"project\":\"demo\"}}\n"
        ),
    )
    .expect("write drive source");
    let connector = format!(
        "case \" $* \" in *\" --max-documents 1 \"*) ;; *) exit 1 ;; esac; cat {input:?}\n"
    );
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\n[connectors]\n\
             command = [\"/bin/sh\", \"-c\", {connector:?}]\n\
             [[sources]]\nname = \"bounded-drive\"\nkind = \"google-drive\"\nproject = \"demo\"\n\
             token = \"/tmp/fake-google-token.json\"\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args([
            "sync",
            "--source",
            "bounded-drive",
            "--no-reconcile",
            "--max-documents",
            "1",
            "--max-bytes",
            "1024",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "source bounded-drive exceeds the 1024 byte budget",
        ));

    let connection =
        Connection::open(data.join("cortana.sqlite3")).expect("open initialized index");
    let documents: i64 = connection
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .expect("document count");
    assert_eq!(documents, 0, "over-budget content must not be ingested");
    let sync_status: String = connection
        .query_row(
            "SELECT status FROM sync_runs WHERE source='bounded-drive'",
            [],
            |row| row.get(0),
        )
        .expect("sync status");
    assert_eq!(sync_status, "budget_exceeded");
}

#[test]
fn reconcile_sync_fails_closed_when_builtin_snapshot_exceeds_document_budget() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let input = directory.path().join("drive.jsonl");
    fs::write(
        &input,
        concat!(
            "{\"source\":\"google-drive\",\"source_id\":\"one\",\"title\":\"One\",",
            "\"content\":\"first document\",\"project\":\"demo\"}\n",
            "{\"source\":\"google-drive\",\"source_id\":\"two\",\"title\":\"Two\",",
            "\"content\":\"second document\",\"project\":\"demo\"}\n"
        ),
    )
    .expect("write drive source");
    // A reconciliation run must never receive the upstream cap: the full
    // snapshot is emitted and the fail-closed preflight rejects it instead of
    // truncating and then deleting the records outside the scope.
    let connector =
        format!("case \" $* \" in *\" --max-documents \"*) exit 1 ;; esac; cat {input:?}\n");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\n[connectors]\n\
             command = [\"/bin/sh\", \"-c\", {connector:?}]\n\
             [[sources]]\nname = \"bounded-drive\"\nkind = \"google-drive\"\nproject = \"demo\"\n\
             token = \"/tmp/fake-google-token.json\"\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args([
            "sync",
            "--source",
            "bounded-drive",
            "--max-documents",
            "1",
            "--max-bytes",
            "1048576",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "source bounded-drive exceeds the 1 document budget",
        ));

    let connection =
        Connection::open(data.join("cortana.sqlite3")).expect("open initialized index");
    let documents: i64 = connection
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .expect("document count");
    assert_eq!(documents, 0, "a truncated snapshot must never be ingested");
    let sync_status: String = connection
        .query_row(
            "SELECT status FROM sync_runs WHERE source='bounded-drive'",
            [],
            |row| row.get(0),
        )
        .expect("sync status");
    assert_eq!(sync_status, "budget_exceeded");
}

#[test]
fn sync_live_output_bound_stops_oversized_external_output() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let input = directory.path().join("oversized.jsonl");
    fs::write(&input, "x".repeat(100_000)).expect("write oversized source");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\n\
             [[sources]]\nname = \"oversized\"\nkind = \"external\"\nproject = \"demo\"\n\
             command = [\"/bin/cat\", {input:?}]\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args([
            "sync",
            "--source",
            "oversized",
            "--max-documents",
            "1",
            "--max-bytes",
            "1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "connector oversized exceeded its live output safety bound",
        ));

    let connection =
        Connection::open(data.join("cortana.sqlite3")).expect("open initialized index");
    let documents: i64 = connection
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .expect("document count");
    assert_eq!(
        documents, 0,
        "oversized external output must not be ingested"
    );
    let sync_status: String = connection
        .query_row(
            "SELECT status FROM sync_runs WHERE source='oversized'",
            [],
            |row| row.get(0),
        )
        .expect("sync status");
    assert_eq!(sync_status, "budget_exceeded");
}

#[cfg(unix)]
#[test]
fn sync_terminates_connector_on_shutdown_signal() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\n\
             [[sources]]\nname = \"slow\"\nkind = \"external\"\nproject = \"demo\"\n\
             command = [\"/bin/sleep\", \"30\"]\n"
        ),
    )
    .expect("write config");

    let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin!("cortana"));
    command
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["sync", "--source", "slow"]);
    let started = std::time::Instant::now();
    let mut child = command.spawn().expect("start sync");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let signal = std::process::Command::new("/bin/kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("send terminate signal");
    assert!(signal.success(), "terminate signal must be delivered");
    let status = child.wait().expect("wait for cancelled sync");

    assert!(!status.success(), "cancelled sync must report failure");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "connector cancellation must not wait for its normal timeout"
    );
}

#[test]
fn backup_verify_and_restore_round_trip() {
    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    let data = directory.path().join("data");
    let backup = directory.path().join("snapshot.sqlite3");
    fs::write(
        &config,
        format!(
            "data_dir = {data:?}\n[embedding]\ndimension = 1024\n\
             base_url = \"http://127.0.0.1:6999/v1\"\nmodel = \"Qwen/Qwen3-Embedding-0.6B\"\n"
        ),
    )
    .expect("write config");

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["ingest", "-"])
        .write_stdin(
            r#"{"source":"test","source_id":"one","title":"First","content":"recoverable snapshot","project":"demo"}"#,
        )
        .assert()
        .success();
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .arg("backup")
        .arg(&backup)
        .assert()
        .success()
        .stdout(predicate::str::contains("backup verified"));
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .arg("verify")
        .arg(&backup)
        .assert()
        .success();

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["ingest", "-"])
        .write_stdin(
            r#"{"source":"test","source_id":"two","title":"Second","content":"temporary mutation","project":"demo"}"#,
        )
        .assert()
        .success();
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--config"])
        .arg(&config)
        .arg("restore")
        .arg(&backup)
        .arg("--force")
        .assert()
        .success()
        .stdout(predicate::str::contains("previous index retained"));

    Command::cargo_bin("cortana")
        .expect("binary exists")
        .args(["--offline", "--config"])
        .arg(&config)
        .args(["search", "recoverable", "--project", "demo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"title\": \"First\""))
        .stdout(predicate::str::contains("\"title\": \"Second\"").not());
}
