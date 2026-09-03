use cortana::knowledge_graph::{
    EdgeKind, EdgeOrigin, GraphContract, GraphEdge, GraphNodeId, NodeKind, RelationshipSupport,
};

#[test]
fn inferred_relationships_require_confidence_and_support() {
    let edge = GraphEdge::new(
        GraphNodeId::new(NodeKind::Document, "source-document").expect("source id"),
        GraphNodeId::new(NodeKind::Entity, "release-process").expect("target id"),
        EdgeKind::Mentions,
        EdgeOrigin::Inferred,
        "2026-08-30T12:00:00Z",
        "work",
        vec!["work".into()],
        RelationshipSupport {
            record_ids: vec!["source-document".into()],
            invalidation_keys: vec!["document:source-document@sha256:abc".into()],
        },
    )
    .expect("edge");

    assert!(edge.validate().is_err());
}

#[test]
fn explicit_relationships_explain_their_origin_and_support() {
    let edge = GraphEdge::new(
        GraphNodeId::new(NodeKind::Source, "notes").expect("source id"),
        GraphNodeId::new(NodeKind::Document, "source-document").expect("target id"),
        EdgeKind::Contains,
        EdgeOrigin::Explicit,
        "2026-08-30T12:00:00Z",
        "work",
        vec!["work".into()],
        RelationshipSupport {
            record_ids: vec!["source-document".into()],
            invalidation_keys: vec!["document:source-document@sha256:abc".into()],
        },
    )
    .expect("edge");

    edge.validate().expect("valid explicit edge");
    assert_eq!(edge.contract_version, GraphContract::VERSION);
    assert!(!edge.explanation().is_empty());
}

#[test]
fn node_ids_are_typed_stable_and_bounded() {
    let id = GraphNodeId::new(NodeKind::Memory, "memory:release-decision").expect("node id");
    assert_eq!(id.as_str(), "memory:memory%3Arelease-decision");
    assert!(GraphNodeId::new(NodeKind::Document, "").is_err());
    assert!(GraphNodeId::new(NodeKind::Document, "x".repeat(513)).is_err());
}
