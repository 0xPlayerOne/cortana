use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use rusqlite::Connection;
use tempfile::tempdir;

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
