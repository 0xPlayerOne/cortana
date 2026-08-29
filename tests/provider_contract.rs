use cortana::integration::{
    ExternalWorkspaceMapping, IntegrationPrincipal, MappingStatus, PrincipalRole,
};

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
