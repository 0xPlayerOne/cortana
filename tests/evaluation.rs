use assert_cmd::Command;

#[test]
fn deterministic_evaluation_cli_passes_built_in_thresholds() {
    let output = Command::cargo_bin("cortana")
        .expect("cortana binary")
        .arg("eval")
        .output()
        .expect("evaluation command");
    assert!(
        output.status.success(),
        "evaluation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("evaluation JSON");
    assert_eq!(report["passed"], true);
    assert_eq!(report["metrics"]["recall_at_k"], 1.0);
    assert_eq!(report["metrics"]["mrr"], 1.0);
    assert_eq!(report["metrics"]["case_pass_rate"], 1.0);
    assert_eq!(report["answer"]["cache_hit"], true);
    assert_eq!(report["answer"]["cache_invalidated_after_update"], true);
}
