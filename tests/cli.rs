use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn doctor_reports_healthy_bootstrap() {
    Command::cargo_bin("cortana")
        .expect("binary exists")
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("bootstrap healthy"));
}
