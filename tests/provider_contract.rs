use std::sync::Arc;

use axum::{body::Body, http::Request};
use chrono::Utc;
use cortana::integration::{
    ExternalWorkspaceMapping, IntegrationPrincipal, MappingStatus, PrincipalRole,
};
use cortana::provider::{
    CapabilityDescriptor, ContextPin, ContextValidation, ProviderOperation, ProviderOutcome,
    ProviderOutcomeCode, ProviderRequest, ProviderRequestLimits, ReplayGuard, TransportProfile,
    ValidationCode, validate_context_bundle,
};
use cortana::{
    api, context, contracts::privacy_scope_digest, embed::DeterministicEmbedder, model::Evidence,
    store::Store,
};
use tempfile::tempdir;
use tower::ServiceExt;

#[test]
fn workspace_mapping_is_explicit_scoped_and_revocable() {
    let mut mapping = ExternalWorkspaceMapping::new(
        "consumer_acme",
        "workspace_external_42",
        "project_cortana_7",
        "cap_local_opaque",
        vec!["project_cortana_7".into(), "shared".into()],
    )
    .expect("valid mapping");

    assert_ne!(mapping.external_workspace_id, mapping.cortana_project_id);
    assert_eq!(mapping.status, MappingStatus::PendingApproval);
    mapping.approve("owner-local").expect("approval");
    assert_eq!(mapping.status, MappingStatus::Active);
    mapping.revoke("owner-local").expect("revocation");
    assert_eq!(mapping.status, MappingStatus::Revoked);
    assert!(mapping.approve("owner-local").is_err());

    let serialized = serde_json::to_string(&mapping).expect("serialize mapping");
    assert!(!serialized.contains("/Users/"));
    assert!(!serialized.contains("Bearer "));
}

#[test]
fn workspace_mapping_rejects_scope_broadening_and_private_connection_details() {
    assert!(
        ExternalWorkspaceMapping::new(
            "consumer_acme",
            "workspace_external_42",
            "project_cortana_7",
            "cap_local_opaque",
            vec!["*".into()],
        )
        .is_err()
    );
    assert!(
        ExternalWorkspaceMapping::new(
            "consumer_acme",
            "workspace_external_42",
            "project_cortana_7",
            "/var/lib/cortana/index.sqlite3",
            vec!["project_cortana_7".into()],
        )
        .is_err()
    );
}

#[test]
fn principal_templates_are_least_privilege_and_secret_free() {
    let query = IntegrationPrincipal::new(
        "principal_query",
        PrincipalRole::QueryOnly,
        "mapping_opaque",
        vec!["work".into()],
        None,
    )
    .expect("query principal");
    assert_eq!(query.scopes, vec!["query", "status"]);
    assert!(!query.can_read_memory());
    assert!(!query.can_write_memory());

    let memory = IntegrationPrincipal::new(
        "principal_memory",
        PrincipalRole::QueryAndMemory,
        "mapping_opaque",
        vec!["work".into()],
        Some("2099-01-01T00:00:00Z"),
    )
    .expect("memory principal");
    assert!(memory.can_read_memory());
    assert!(memory.can_write_memory());
    assert!(!memory.is_expired_at("2098-01-01T00:00:00Z").unwrap());
    assert!(memory.is_expired_at("2100-01-01T00:00:00Z").unwrap());

    let serialized = serde_json::to_string(&memory).expect("serialize principal");
    assert!(!serialized.contains("token"));
    assert!(!serialized.contains("secret"));
}

#[test]
fn revoked_principal_is_immediately_inactive() {
    let mut principal = IntegrationPrincipal::new(
        "principal_query",
        PrincipalRole::QueryOnly,
        "mapping_opaque",
        vec!["work".into()],
        None,
    )
    .expect("principal");
    assert!(principal.is_active_at("2030-01-01T00:00:00Z").unwrap());
    principal.revoke("owner-local").expect("revocation");
    assert!(!principal.is_active_at("2030-01-01T00:00:00Z").unwrap());
}

#[test]
fn direct_and_remote_transports_advertise_the_same_provider_semantics() {
    let capabilities = CapabilityDescriptor::current();
    assert!(
        capabilities
            .transports
            .contains(&TransportProfile::DirectLocal)
    );
    assert!(
        capabilities
            .transports
            .contains(&TransportProfile::ScopedHttp)
    );
    assert!(
        capabilities
            .transports
            .contains(&TransportProfile::RemoteBroker)
    );
    assert!(
        capabilities
            .operations
            .contains(&ProviderOperation::Context)
    );
    assert!(
        capabilities
            .operations
            .contains(&ProviderOperation::MemoryWrite)
    );
    assert!(capabilities.limits.max_request_bytes > 0);

    let direct = ProviderOutcome::<serde_json::Value>::success(
        TransportProfile::DirectLocal,
        serde_json::json!({"context_bundle_id": "ctx_opaque"}),
    );
    let remote = direct.clone_for_transport(TransportProfile::RemoteBroker);
    assert_eq!(direct.code, remote.code);
    assert_eq!(direct.result, remote.result);
}

#[test]
fn provider_requests_are_bounded_versioned_and_transport_independent() {
    let request = ProviderRequest::new(
        "request_42",
        "mapping_opaque",
        "principal_opaque",
        "project_opaque",
        privacy_scope_digest(Some("work"), None, &["work".into()]),
        ProviderOperation::Context,
        ProviderRequestLimits {
            max_tokens: 2_000,
            max_response_bytes: 256_000,
            timeout_ms: 20_000,
        },
        Some("read_idempotency_42"),
    )
    .expect("valid provider request");
    request.validate(&CapabilityDescriptor::current()).unwrap();

    let serialized = serde_json::to_value(&request).expect("serialize request");
    assert!(serialized.get("endpoint").is_none());
    assert!(serialized.get("database_path").is_none());
    assert!(serialized.get("credential").is_none());
}

#[test]
fn replay_guard_deduplicates_reads_and_stops_ambiguous_write_retries() {
    let mut guard = ReplayGuard::new(4);
    assert!(guard.accept("read_1", ProviderOperation::Context).unwrap());
    assert!(!guard.accept("read_1", ProviderOperation::Context).unwrap());
    assert!(
        guard
            .accept("write_1", ProviderOperation::MemoryWrite)
            .unwrap()
    );
    assert!(
        guard
            .accept("write_1", ProviderOperation::MemoryWrite)
            .is_err()
    );
}

#[test]
fn provider_failures_are_explicit_machine_readable_and_safe() {
    let outcome = ProviderOutcome::<serde_json::Value>::failure(
        TransportProfile::RemoteBroker,
        ProviderOutcomeCode::HostOffline,
        "provider host is offline",
    );
    assert!(outcome.retryable);
    assert!(outcome.result.is_none());
    assert_eq!(outcome.code, ProviderOutcomeCode::HostOffline);
    let serialized = serde_json::to_string(&outcome).unwrap();
    assert!(!serialized.contains("/Users/"));
    assert!(!serialized.contains("sqlite"));
}

#[test]
fn context_pinning_rejects_scope_revision_budget_and_digest_mismatch() {
    let evidence = Evidence {
        chunk_id: "evidence_1".into(),
        source: "fixture".into(),
        source_id: "fixture-source-1".into(),
        title: "Fixture".into(),
        content: "bounded evidence".into(),
        uri: Some("fixture://evidence/1".into()),
        updated_at: Utc::now(),
        score: 1.0,
        semantic_rank: Some(1),
        lexical_rank: Some(1),
    };
    let scope = privacy_scope_digest(Some("work"), None, &["work".into()]);
    let bundle = context::build("fixture query", &[evidence], 512).with_metadata(
        context::metadata(context::ContextMetadataInput {
            token_budget: 512,
            corpus_revision: 7,
            memory_revision: Some(3),
            embedding_fingerprint: Some("fixture-embedding-v1".into()),
            project: Some("work"),
            source: None,
            acl: &["work".into()],
            retrieval_warning: None,
        }),
    );
    let approved = ContextValidation {
        expected_scope_digest: scope.clone(),
        minimum_corpus_revision: 7,
        maximum_token_budget: 512,
        allow_degraded: false,
    };
    let pin = validate_context_bundle(&bundle, &approved).expect("valid bundle");
    assert_eq!(pin.context_bundle_id, bundle.context_bundle_id);
    assert!(
        !serde_json::to_string(&pin)
            .unwrap()
            .contains("fixture query")
    );

    let mut wrong_scope = approved.clone();
    wrong_scope.expected_scope_digest = privacy_scope_digest(Some("other"), None, &[]);
    assert_eq!(
        validate_context_bundle(&bundle, &wrong_scope).unwrap_err(),
        ValidationCode::ScopeMismatch
    );
    let mut stale = approved.clone();
    stale.minimum_corpus_revision = 8;
    assert_eq!(
        validate_context_bundle(&bundle, &stale).unwrap_err(),
        ValidationCode::StaleRevision
    );

    let mut tampered = bundle.clone();
    tampered.context.push_str(" tampered");
    assert_eq!(
        validate_context_bundle(&tampered, &approved).unwrap_err(),
        ValidationCode::DigestMismatch
    );
}

#[test]
fn context_pin_contains_only_non_secret_replay_metadata() {
    let pin = ContextPin {
        provider_contract_version: "cortana.provider.v1".into(),
        context_contract_version: "cortana.context.v1".into(),
        context_bundle_id: "ctx_opaque".into(),
        canonical_digest: "a".repeat(64),
        privacy_scope_digest: "b".repeat(64),
        corpus_revision: 9,
        memory_revision: Some(4),
        embedding_fingerprint: Some("embedding-v1".into()),
        retrieval_contract_version: "cortana.retrieval.v2".into(),
        token_budget: 512,
        created_at: "2030-01-01T00:00:00Z".into(),
        degradation_code: None,
    };
    let value = serde_json::to_value(pin).unwrap();
    assert!(value.get("query").is_none());
    assert!(value.get("context").is_none());
    assert!(value.get("credential").is_none());
}

#[tokio::test]
async fn http_exposes_the_versioned_provider_capability_descriptor() {
    let directory = tempdir().unwrap();
    let store = Store::open(&directory.path().join("store.sqlite3")).unwrap();
    let app = api::router(api::AppState::new(
        store,
        Arc::new(DeterministicEmbedder::new(16)),
    ));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/provider/capabilities")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let capabilities: CapabilityDescriptor = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(capabilities.contract_version, "cortana.provider.v1");
}
