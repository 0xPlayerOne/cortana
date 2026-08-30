//! Versioned, deployment-neutral ContextProvider contracts.
//!
//! The provider surface is deliberately independent of SQLite, filesystem
//! paths, bearer values, and consumer-owned execution state. Local direct,
//! scoped HTTP, and broker transports carry the same semantic requests and
//! outcomes.

use std::collections::{HashMap, VecDeque};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{
    context::{ContextBundle, MAX_CONTEXT_TOKENS, MIN_CONTEXT_TOKENS},
    contracts::{CONTEXT_CONTRACT_VERSION, privacy_scope_digest},
    integration::{ExternalWorkspaceMapping, IntegrationPrincipal, MappingStatus, PrincipalStatus},
};

pub const PROVIDER_CONTRACT_VERSION: &str = "cortana.provider.v1";
pub const PROVIDER_FIXTURE_VERSION: &str = "cortana.provider-fixtures.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderOperation {
    Context,
    EvidenceSearch,
    Status,
    MemoryRecall,
    MemoryWrite,
    MemoryWriteStatus,
}

impl ProviderOperation {
    fn is_write(self) -> bool {
        matches!(self, Self::MemoryWrite)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportProfile {
    DirectLocal,
    ScopedHttp,
    RemoteBroker,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderLimits {
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_context_tokens: usize,
    pub max_timeout_ms: u64,
    pub max_concurrency: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDescriptor {
    pub provider_type: String,
    pub contract_version: String,
    pub fixture_version: String,
    pub transports: Vec<TransportProfile>,
    pub operations: Vec<ProviderOperation>,
    pub limits: ProviderLimits,
    pub connection_identity: String,
}

impl CapabilityDescriptor {
    pub fn current() -> Self {
        Self {
            provider_type: "cortana".into(),
            contract_version: PROVIDER_CONTRACT_VERSION.into(),
            fixture_version: PROVIDER_FIXTURE_VERSION.into(),
            transports: vec![
                TransportProfile::DirectLocal,
                TransportProfile::ScopedHttp,
                TransportProfile::RemoteBroker,
            ],
            operations: vec![
                ProviderOperation::Context,
                ProviderOperation::EvidenceSearch,
                ProviderOperation::Status,
                ProviderOperation::MemoryRecall,
                ProviderOperation::MemoryWrite,
                ProviderOperation::MemoryWriteStatus,
            ],
            limits: ProviderLimits {
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 8 * 1024 * 1024,
                max_context_tokens: MAX_CONTEXT_TOKENS,
                max_timeout_ms: 60_000,
                max_concurrency: 32,
            },
            connection_identity: "cortana-provider".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRequestLimits {
    pub max_tokens: usize,
    pub max_response_bytes: usize,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRequest {
    pub contract_version: String,
    pub request_id: String,
    pub mapping_ref: String,
    pub principal_ref: String,
    pub project_ref: String,
    pub privacy_scope_digest: String,
    pub operation: ProviderOperation,
    pub limits: ProviderRequestLimits,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

impl ProviderRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request_id: &str,
        mapping_ref: &str,
        principal_ref: &str,
        project_ref: &str,
        privacy_scope_digest: String,
        operation: ProviderOperation,
        limits: ProviderRequestLimits,
        idempotency_key: Option<&str>,
    ) -> Result<Self> {
        for (name, value) in [
            ("request_id", request_id),
            ("mapping_ref", mapping_ref),
            ("principal_ref", principal_ref),
            ("project_ref", project_ref),
        ] {
            validate_opaque(name, value)?;
        }
        ensure!(
            privacy_scope_digest.len() == 64
                && privacy_scope_digest
                    .chars()
                    .all(|character| character.is_ascii_hexdigit()),
            "privacy_scope_digest must be a SHA-256 hex digest"
        );
        if let Some(key) = idempotency_key {
            validate_opaque("idempotency_key", key)?;
        }
        let request = Self {
            contract_version: PROVIDER_CONTRACT_VERSION.into(),
            request_id: request_id.into(),
            mapping_ref: mapping_ref.into(),
            principal_ref: principal_ref.into(),
            project_ref: project_ref.into(),
            privacy_scope_digest,
            operation,
            limits,
            idempotency_key: idempotency_key.map(str::to_owned),
        };
        request.validate(&CapabilityDescriptor::current())?;
        Ok(request)
    }

    pub fn validate(&self, capabilities: &CapabilityDescriptor) -> Result<()> {
        ensure!(
            self.contract_version == capabilities.contract_version,
            "incompatible provider contract version"
        );
        ensure!(
            capabilities.operations.contains(&self.operation),
            "provider operation is unsupported"
        );
        ensure!(
            (MIN_CONTEXT_TOKENS..=capabilities.limits.max_context_tokens)
                .contains(&self.limits.max_tokens),
            "provider token budget is out of range"
        );
        ensure!(
            self.limits.max_response_bytes > 0
                && self.limits.max_response_bytes <= capabilities.limits.max_response_bytes,
            "provider response byte budget is out of range"
        );
        ensure!(
            self.limits.timeout_ms > 0
                && self.limits.timeout_ms <= capabilities.limits.max_timeout_ms,
            "provider timeout is out of range"
        );
        ensure!(
            !self.operation.is_write() || self.idempotency_key.is_some(),
            "provider writes require an idempotency key"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerEnvelope {
    pub fixture_version: String,
    pub connection_ref: String,
    pub request: ProviderRequest,
    pub payload_digest: String,
    pub attempt: u16,
}

impl BrokerEnvelope {
    pub fn new(
        connection_ref: &str,
        request: ProviderRequest,
        payload_digest: String,
    ) -> Result<Self> {
        validate_opaque("connection_ref", connection_ref)?;
        validate_digest("payload_digest", &payload_digest)?;
        request.validate(&CapabilityDescriptor::current())?;
        Ok(Self {
            fixture_version: PROVIDER_FIXTURE_VERSION.into(),
            connection_ref: connection_ref.into(),
            request,
            payload_digest,
            attempt: 1,
        })
    }
}

pub fn authorize_provider_request(
    request: &ProviderRequest,
    mapping: &ExternalWorkspaceMapping,
    principal: &IntegrationPrincipal,
    at: &str,
) -> std::result::Result<(), ProviderOutcomeCode> {
    if request.validate(&CapabilityDescriptor::current()).is_err()
        || mapping.status != MappingStatus::Active
        || request.mapping_ref != mapping.mapping_id
        || request.principal_ref != principal.principal_id
        || principal.mapping_ref != mapping.mapping_id
        || request.project_ref != mapping.cortana_project_id
        || principal.status != PrincipalStatus::Active
        || !principal.is_active_at(at).unwrap_or(false)
        || !principal
            .acl
            .iter()
            .all(|label| mapping.permitted_acl.contains(label))
        || request.privacy_scope_digest
            != privacy_scope_digest(Some(&mapping.cortana_project_id), None, &principal.acl)
    {
        return Err(ProviderOutcomeCode::Unauthorized);
    }
    let required_scope = match request.operation {
        ProviderOperation::Context | ProviderOperation::EvidenceSearch => "query",
        ProviderOperation::Status => "status",
        ProviderOperation::MemoryRecall
        | ProviderOperation::MemoryWrite
        | ProviderOperation::MemoryWriteStatus => "memory",
    };
    if !principal.scopes.iter().any(|scope| scope == required_scope) {
        return Err(ProviderOutcomeCode::Unauthorized);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderOutcomeCode {
    Ok,
    Unavailable,
    Unauthorized,
    Stale,
    Degraded,
    Insufficient,
    OverBudget,
    Incompatible,
    Cancelled,
    RateLimited,
    TimedOut,
    HostOffline,
    AmbiguousWrite,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderOutcome<T> {
    pub contract_version: String,
    pub code: ProviderOutcomeCode,
    pub transport: TransportProfile,
    pub retryable: bool,
    pub user_action_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<T>,
}

impl<T> ProviderOutcome<T> {
    pub fn success(transport: TransportProfile, result: T) -> Self {
        Self {
            contract_version: PROVIDER_CONTRACT_VERSION.into(),
            code: ProviderOutcomeCode::Ok,
            transport,
            retryable: false,
            user_action_required: false,
            message: None,
            retry_after_ms: None,
            result: Some(result),
        }
    }

    pub fn failure(transport: TransportProfile, code: ProviderOutcomeCode, message: &str) -> Self {
        let retryable = matches!(
            code,
            ProviderOutcomeCode::Unavailable
                | ProviderOutcomeCode::RateLimited
                | ProviderOutcomeCode::TimedOut
                | ProviderOutcomeCode::HostOffline
        );
        let user_action_required = matches!(
            code,
            ProviderOutcomeCode::Unauthorized
                | ProviderOutcomeCode::Stale
                | ProviderOutcomeCode::Incompatible
                | ProviderOutcomeCode::AmbiguousWrite
        );
        Self {
            contract_version: PROVIDER_CONTRACT_VERSION.into(),
            code,
            transport,
            retryable,
            user_action_required,
            message: Some(safe_message(message)),
            retry_after_ms: None,
            result: None,
        }
    }
}

impl<T: Clone> ProviderOutcome<T> {
    pub fn clone_for_transport(&self, transport: TransportProfile) -> Self {
        let mut clone = self.clone();
        clone.transport = transport;
        clone
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextValidation {
    pub expected_scope_digest: String,
    pub minimum_corpus_revision: u64,
    pub maximum_token_budget: usize,
    pub allow_degraded: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextPin {
    pub provider_contract_version: String,
    pub context_contract_version: String,
    pub context_bundle_id: String,
    pub canonical_digest: String,
    pub privacy_scope_digest: String,
    pub corpus_revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_fingerprint: Option<String>,
    pub retrieval_contract_version: String,
    pub token_budget: usize,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degradation_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationCode {
    IncompatibleContract,
    DigestMismatch,
    ScopeMismatch,
    StaleRevision,
    OverBudget,
    Degraded,
    Malformed,
}

pub fn validate_context_bundle(
    bundle: &ContextBundle,
    approved: &ContextValidation,
) -> std::result::Result<ContextPin, ValidationCode> {
    if bundle.contract_version != CONTEXT_CONTRACT_VERSION {
        return Err(ValidationCode::IncompatibleContract);
    }
    let digest = bundle.digest();
    if digest != bundle.canonical_digest
        || bundle.context_bundle_id != format!("ctx_{digest}")
        || bundle.canonical_digest.len() != 64
    {
        return Err(ValidationCode::DigestMismatch);
    }
    if bundle.privacy_scope_digest != approved.expected_scope_digest {
        return Err(ValidationCode::ScopeMismatch);
    }
    if bundle.corpus_revision < approved.minimum_corpus_revision {
        return Err(ValidationCode::StaleRevision);
    }
    if bundle.token_budget > approved.maximum_token_budget
        || bundle.metrics.estimated_tokens > bundle.token_budget
    {
        return Err(ValidationCode::OverBudget);
    }
    if bundle.degradation.is_some() && !approved.allow_degraded {
        return Err(ValidationCode::Degraded);
    }
    if bundle.created_at.trim().is_empty()
        || bundle.retrieval_contract_version.trim().is_empty()
        || bundle.privacy_scope_digest.len() != 64
    {
        return Err(ValidationCode::Malformed);
    }
    Ok(ContextPin {
        provider_contract_version: PROVIDER_CONTRACT_VERSION.into(),
        context_contract_version: bundle.contract_version.clone(),
        context_bundle_id: bundle.context_bundle_id.clone(),
        canonical_digest: bundle.canonical_digest.clone(),
        privacy_scope_digest: bundle.privacy_scope_digest.clone(),
        corpus_revision: bundle.corpus_revision,
        memory_revision: bundle.memory_revision,
        embedding_fingerprint: bundle.embedding_fingerprint.clone(),
        retrieval_contract_version: bundle.retrieval_contract_version.clone(),
        token_budget: bundle.token_budget,
        created_at: bundle.created_at.clone(),
        degradation_code: bundle
            .degradation
            .as_ref()
            .map(|degradation| degradation.code.clone()),
    })
}

/// Bounded transport replay state. Idempotent reads return `false` on a
/// duplicate so callers can reuse the cached authoritative result. A duplicate
/// write is rejected until its write-status operation resolves the first
/// attempt; automatic re-execution could duplicate a memory effect.
#[derive(Debug)]
pub struct ReplayGuard {
    capacity: usize,
    order: VecDeque<String>,
    operations: HashMap<String, ProviderOperation>,
}

impl ReplayGuard {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::new(),
            operations: HashMap::new(),
        }
    }

    pub fn accept(&mut self, key: &str, operation: ProviderOperation) -> Result<bool> {
        validate_opaque("replay key", key)?;
        if let Some(previous) = self.operations.get(key) {
            ensure!(
                !previous.is_write() && !operation.is_write(),
                "ambiguous write status requires authoritative reconciliation"
            );
            ensure!(*previous == operation, "replay operation does not match");
            return Ok(false);
        }
        if self.order.len() == self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.operations.remove(&oldest);
            }
        }
        self.order.push_back(key.into());
        self.operations.insert(key.into(), operation);
        Ok(true)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectStatus {
    Pending,
    Applied,
    Rejected,
    Ambiguous,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryWriteEffect {
    pub contract_version: String,
    pub effect_id: String,
    pub idempotency_key: String,
    pub status: EffectStatus,
    pub target_revision: Option<u64>,
    pub approving_principal_ref: String,
    pub provenance_digest: String,
}

fn validate_opaque(name: &str, value: &str) -> Result<()> {
    let value = value.trim();
    ensure!(!value.is_empty(), "{name} must not be empty");
    ensure!(value.len() <= 256, "{name} exceeds 256 bytes");
    ensure!(
        !value.contains('/')
            && !value.contains('\\')
            && !value.contains("://")
            && !value.to_ascii_lowercase().starts_with("bearer "),
        "{name} must be an opaque public reference"
    );
    Ok(())
}

fn validate_digest(name: &str, value: &str) -> Result<()> {
    ensure!(
        value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()),
        "{name} must be a SHA-256 hex digest"
    );
    Ok(())
}

fn safe_message(message: &str) -> String {
    let message = message.trim();
    if message.is_empty()
        || message.len() > 512
        || message.contains('/')
        || message.contains('\\')
        || message.to_ascii_lowercase().contains("bearer ")
        || message.to_ascii_lowercase().contains("sqlite")
    {
        "provider request failed".into()
    } else {
        message.into()
    }
}
