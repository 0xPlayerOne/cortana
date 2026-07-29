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
