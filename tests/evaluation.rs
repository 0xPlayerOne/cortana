use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use assert_cmd::Command;
use axum::{Router, http::StatusCode, response::IntoResponse, routing::post};
use serde_json::json;
use tempfile::tempdir;

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
    assert_eq!(report["activation"]["activated"], false);
    assert_eq!(report["activation"]["provider"], serde_json::Value::Null);
    assert_eq!(
        report["activation"]["retrieval_contract_version"],
        "cortana.retrieval.v2"
    );
    assert!(
        report["activation"]["corpus_revision"]
            .as_u64()
            .unwrap_or(0)
            > 0
    );
    assert_eq!(
        report["activation"]["evaluation_report_digest"]
            .as_str()
            .map(str::len),
        Some(64)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn model_evaluation_synthesizes_even_when_production_config_disables_synthesis() {
    // Local loopback provider: three answer passes run planner+synthesis for
    // the first and the post-update pass; the second pass is a cache hit.
    let calls = Arc::new(AtomicU32::new(0));
    let app = Router::new().route(
        "/v1/chat/completions",
        post({
            let calls = Arc::clone(&calls);
            move || {
                let calls = Arc::clone(&calls);
                async move {
                    let index = calls.fetch_add(1, Ordering::SeqCst) as usize;
                    let content = if index % 2 == 0 {
                        r#"{"queries":["release"]}"#.to_string()
                    } else {
                        "The release process is bounded by safe checks. [1]".to_string()
                    };
                    (
                        StatusCode::OK,
                        serde_json::to_string(
                            &json!({ "choices": [{ "message": { "content": content } }] }),
                        )
                        .expect("mock provider response JSON"),
                    )
                        .into_response()
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock provider listener");
    let address = listener.local_addr().expect("mock provider address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve mock provider");
    });

    let directory = tempdir().expect("temporary directory");
    let config = directory.path().join("config.toml");
    fs::write(
        &config,
        format!(
            "data_dir = {:?}\n[query]\nsynthesis_enabled = false\nbase_url = \"http://{address}/v1\"\nmodel = \"mock-model\"\n",
            directory.path().join("data")
        ),
    )
    .expect("write config");
    let fixture = directory.path().join("fixtures.json");
    fs::write(&fixture, include_str!("../eval/fixtures.json")).expect("write fixture");

    let output = Command::cargo_bin("cortana")
        .expect("cortana binary")
        .args(["--config"])
        .arg(&config)
        .args(["eval", "--model", "--fixture"])
        .arg(&fixture)
        .output()
        .expect("model-backed evaluation command");
    server.abort();

    assert!(
        output.status.success(),
        "model-backed evaluation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("evaluation JSON");
    assert_eq!(report["answer"]["attempted"], true);
    assert_eq!(report["answer"]["planner_model_used"], true);
    assert_eq!(report["answer"]["synthesis_model_used"], true);
    assert_eq!(report["answer"]["fallback_mode"], false);
    assert_eq!(report["answer"]["cache_hit"], true);
    assert_eq!(report["passed"], true);
    assert_eq!(report["activation"]["activated"], true);
    assert_eq!(
        report["activation"]["provider"],
        format!("http://{address}")
    );
    assert_eq!(report["activation"]["model"], "mock-model");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "the provider must be exercised by the in-memory synthesis enable"
    );
}
