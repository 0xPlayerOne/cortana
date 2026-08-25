//! Versioned public contracts shared by HTTP, MCP, CLI, and Desktop clients.
//!
//! The contract module deliberately contains only transport-safe metadata and
//! deterministic hashing helpers. It must never read credentials or expose
//! private filesystem paths.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ENTITY_CONTRACT_VERSION: &str = "cortana.entity.v1";
pub const CONTEXT_CONTRACT_VERSION: &str = "cortana.context.v1";
pub const RETRIEVAL_CONTRACT_VERSION: &str = "cortana.retrieval.v1";
pub const API_CONTRACT_VERSION: &str = "cortana.api.v1";
pub const CONNECTOR_CONTRACT_VERSION: &str = "cortana.connector.v1";
pub const MEMORY_CONTRACT_VERSION: &str = "cortana.memory.v1";
pub const IDENTITY_CONTRACT_VERSION: &str = "cortana.identity.v1";

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct DegradationState {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContextMetadata {
    pub contract_version: String,
    pub created_at: String,
    pub token_budget: usize,
    pub corpus_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_fingerprint: Option<String>,
    pub retrieval_contract_version: String,
    pub privacy_scope_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degradation: Option<DegradationState>,
}

impl Default for ContextMetadata {
    fn default() -> Self {
        Self {
            contract_version: CONTEXT_CONTRACT_VERSION.into(),
            created_at: "1970-01-01T00:00:00.000Z".into(),
            token_budget: 0,
            corpus_revision: 0,
            memory_revision: None,
            embedding_fingerprint: None,
            retrieval_contract_version: RETRIEVAL_CONTRACT_VERSION.into(),
            privacy_scope_digest: privacy_scope_digest(None, None, &[]),
            degradation: None,
        }
    }
}

/// Hash normalized request scope without including raw scope values in a
/// public pin. Labels are sorted and deduplicated before hashing.
pub fn privacy_scope_digest(project: Option<&str>, source: Option<&str>, acl: &[String]) -> String {
    let mut acl = acl.to_vec();
    acl.sort();
    acl.dedup();
    let payload = serde_json::json!({
        "project": project,
        "source": source,
        "acl": acl,
    });
    stable_json_digest(&payload)
}

pub fn stable_json_digest<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("contract values must be serializable");
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_digest_is_order_independent_and_does_not_expose_labels() {
        let first = privacy_scope_digest(Some("work"), Some("drive"), &["b".into(), "a".into()]);
        let second = privacy_scope_digest(Some("work"), Some("drive"), &["a".into(), "b".into()]);
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(!first.contains("work"));
        assert!(!first.contains("drive"));
    }

    #[test]
    fn contract_versions_are_explicit() {
        assert_eq!(CONTEXT_CONTRACT_VERSION, "cortana.context.v1");
        assert_eq!(API_CONTRACT_VERSION, "cortana.api.v1");
        assert_eq!(ENTITY_CONTRACT_VERSION, "cortana.entity.v1");
    }
}
