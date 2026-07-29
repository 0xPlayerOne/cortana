use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

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
