use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use cortana::{
    api::{self, AppState},
    auth::AuthPolicy,
    config::{AuthTokenConfig, Config},
    embed::DeterministicEmbedder,
    store::Store,
};
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

async fn request(
    app: &Router,
    path: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().uri(path);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = if let Some(value) = body {
        builder = builder.method(Method::POST);
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(serde_json::to_vec(&value).unwrap())
    } else {
        Body::empty()
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"text": String::from_utf8_lossy(&bytes)}));
    (status, value)
}

fn local_app(path: &std::path::Path) -> Router {
    api::router(AppState::new(
        Store::open(path).unwrap(),
        Arc::new(DeterministicEmbedder::new(16)),
    ))
}

fn self_hosted_app(path: &std::path::Path, token: &str) -> Router {
    let mut config = Config::default();
    config
        .environment
        .insert("PROVIDER_TOKEN".into(), token.into());
    config.auth.tokens = vec![AuthTokenConfig {
        principal: "self-hosted-owner".into(),
        token_env: "PROVIDER_TOKEN".into(),
        scopes: vec![
            "query".into(),
            "status".into(),
            "memory".into(),
            "admin".into(),
        ],
        acl: Vec::new(),
    }];
    let auth = AuthPolicy::from_config(&config).unwrap();
    api::router(
        AppState::new(
            Store::open(path).unwrap(),
            Arc::new(DeterministicEmbedder::new(16)),
        )
        .with_auth_policy_for_listener(auth, true),
    )
}

#[tokio::test]
async fn local_and_self_hosted_profiles_return_equivalent_context_semantics() {
    let local_directory = tempdir().unwrap();
    let hosted_directory = tempdir().unwrap();
    let local = local_app(&local_directory.path().join("local.sqlite3"));
    let hosted = self_hosted_app(
        &hosted_directory.path().join("hosted.sqlite3"),
        "hosted-secret",
    );
    let payload = json!({
        "query": "empty provider fixture",
        "project": "work",
        "limit": 5,
        "max_tokens": 512
    });

    let (local_status, local_bundle) =
        request(&local, "/v1/context", Some(payload.clone()), None).await;
    let (hosted_status, hosted_bundle) =
        request(&hosted, "/v1/context", Some(payload), Some("hosted-secret")).await;
    assert_eq!(local_status, StatusCode::OK);
    assert_eq!(hosted_status, StatusCode::OK);
    for field in [
        "contract_version",
        "context_bundle_id",
        "canonical_digest",
        "token_budget",
        "evidence",
        "memories",
        "metrics",
        "retrieval_mode",
        "corpus_revision",
        "memory_revision",
        "embedding_fingerprint",
        "retrieval_contract_version",
        "privacy_scope_digest",
    ] {
        assert_eq!(local_bundle[field], hosted_bundle[field], "field {field}");
    }

    let (missing_status, _) = request(&hosted, "/v1/context", None, None).await;
    let (invalid_status, _) = request(&hosted, "/v1/context", None, Some("wrong")).await;
    assert_eq!(missing_status, StatusCode::UNAUTHORIZED);
    assert_eq!(invalid_status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn self_hosted_memory_state_survives_service_recreation() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("hosted.sqlite3");
    let payload = json!({
        "kind": "episodic",
        "project": "work",
        "title": "Restart fixture",
        "content": "Provider state survives recreation",
        "source": "provider-conformance",
        "source_id": "restart-1",
        "dedupe_key": "provider-restart-1",
        "acl": ["work"],
        "provenance": {"fixture": "self_hosted_single_node"}
    });
    let first = self_hosted_app(&database, "hosted-secret");
    let (status, written) =
        request(&first, "/v1/memory", Some(payload), Some("hosted-secret")).await;
    assert_eq!(status, StatusCode::OK);
    let memory_id = written["id"].clone();
    drop(first);

    let restarted = self_hosted_app(&database, "hosted-secret");
    let (status, recalled) = request(
        &restarted,
        "/v1/memory/recall",
        Some(json!({"query": "provider state recreation", "project": "work", "limit": 5})),
        Some("hosted-secret"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(recalled[0]["id"], memory_id);
    assert_eq!(
        recalled[0]["provenance"]["fixture"],
        "self_hosted_single_node"
    );
}
