//! Bounded, non-mutating reflection over authorized memory and evidence.
//!
//! Reflection is deliberately separate from `remember` and candidate
//! promotion.  It may describe patterns or propose an insight, but it never
//! writes the canonical memory table and never advances `memory_revision`.
//! Transport layers (HTTP, MCP, CLI, and Desktop) should authorize their
//! principal first, then pass only the bounded, scoped inputs to this module.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{self, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::auth::acl_allows;
use crate::context::estimate_tokens;
use crate::contracts::{privacy_scope_digest, stable_json_digest};
use crate::embed::Embedder;
use crate::memory::{
    MemoryContentType, MemoryKind, MemoryRecord, MemoryRetentionTier, MemoryScope,
};
use crate::model::Evidence;
use crate::store::Store;

pub const REFLECTION_CONTRACT_VERSION: &str = "cortana.reflection.v1";
pub const MIN_REFLECTION_TOKEN_BUDGET: usize = 256;
pub const MAX_REFLECTION_TOKEN_BUDGET: usize = 8_192;
pub const MAX_REFLECTION_OBJECTIVE_BYTES: usize = 512;
pub const MAX_REFLECTION_PROJECT_BYTES: usize = 256;
pub const MAX_REFLECTION_SOURCE_BYTES: usize = 128;
pub const MAX_REFLECTION_MEMORY_LIMIT: usize = 100;
pub const MAX_REFLECTION_EVIDENCE_LIMIT: usize = 50;
pub const MAX_REFLECTION_CLAIMS: usize = 32;
pub const MAX_REFLECTION_PATTERNS: usize = 16;
pub const MAX_REFLECTION_TENSIONS: usize = 16;
pub const MAX_REFLECTION_RECOMMENDATIONS: usize = 16;
pub const MAX_REFLECTION_CANDIDATES: usize = 8;
pub const MAX_PROVIDER_OUTPUT_BYTES: usize = 128 * 1024;
const MIN_PRIVATE_EVIDENCE_FINGERPRINT_CHARS: usize = 12;

static ACTIVE_PROVIDERS: OnceLock<Mutex<BTreeSet<usize>>> = OnceLock::new();

fn acquire_provider(provider: &Arc<dyn ReflectionProvider>) -> Option<usize> {
    let key = Arc::as_ptr(provider) as *const () as usize;
    let active = ACTIVE_PROVIDERS.get_or_init(|| Mutex::new(BTreeSet::new()));
    active.lock().ok()?.insert(key).then_some(key)
}

fn release_provider(key: usize) {
    if let Some(active) = ACTIVE_PROVIDERS.get()
        && let Ok(mut active) = active.lock()
    {
        active.remove(&key);
    }
}
pub const MAX_REFLECTION_DEADLINE_MS: u64 = 30_000;
pub const MAX_REFLECTION_TEXT_BYTES: usize = 512;
const MAX_REFLECTION_INPUT_TEXT_BYTES: usize = 2_048;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderPolicy {
    /// Use only the deterministic local implementation.
    #[default]
    DeterministicOnly,
    /// Try a configured provider, then return the deterministic result if it
    /// is unavailable or returns an invalid/ungrounded response.
    PreferProvider,
    /// A provider is required; unavailable or failed providers return no
    /// synthesized result and an explicit outcome.
    RequireProvider,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryReflectFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default = "default_memory_limit")]
    pub limit: usize,
}

impl Default for MemoryReflectFilter {
    fn default() -> Self {
        Self {
            kind: None,
            content_type: None,
            retention_tier: None,
            scope: None,
            limit: default_memory_limit(),
        }
    }
}

fn default_memory_limit() -> usize {
    32
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReflectRequest {
    pub objective: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default)]
    pub memory: MemoryReflectFilter,
    #[serde(default)]
    pub include_evidence: bool,
    #[serde(default = "default_token_budget")]
    pub token_budget: usize,
    #[serde(default)]
    pub provider_policy: ProviderPolicy,
    #[serde(default = "default_deadline_ms")]
    pub deadline_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

fn default_token_budget() -> usize {
    2_048
}

fn default_deadline_ms() -> u64 {
    5_000
}

#[derive(Clone, Debug)]
pub struct ReflectionInputs<'a> {
    /// Memory records must come from the existing scoped recall/export path.
    pub memories: &'a [MemoryRecord],
    /// Evidence must come from an existing scoped retrieval path. Since the
    /// legacy Evidence contract has no ACL field, callers also supply the
    /// retrieval scope below so this module can reject cross-workspace use.
    pub evidence: &'a [Evidence],
    pub evidence_project: Option<&'a str>,
    pub principal_acl: &'a [String],
    pub owner: bool,
    pub memory_revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReflectStatus {
    Completed,
    Fallback,
    ProviderUnavailable,
    ProviderFailed,
    DeadlineExceeded,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderOutcome {
    pub policy: ProviderPolicy,
    pub selected: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReflectClaim {
    pub text: String,
    pub supporting_memory_ids: Vec<String>,
    pub supporting_evidence_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReflectPattern {
    pub statement: String,
    pub supporting_memory_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReflectTension {
    pub statement: String,
    pub supporting_memory_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReflectChronology {
    pub observed_at: String,
    pub title: String,
    pub memory_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReflectRecommendation {
    pub statement: String,
    pub supporting_memory_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProposedReflectCandidate {
    pub project: String,
    pub title: String,
    pub content: String,
    pub content_type: String,
    pub retention_tier: String,
    pub scope: String,
    pub supporting_memory_ids: Vec<String>,
    /// A transport must call the explicit memory retain/remember operation.
    pub approval_required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReflectMetrics {
    pub memories_considered: usize,
    pub memories_included: usize,
    pub evidence_considered: usize,
    pub evidence_included: usize,
    pub estimated_tokens: usize,
    pub memory_revision: u64,
    pub canonical_memory_mutated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReflectResponse {
    pub contract_version: String,
    pub request_digest: String,
    pub status: ReflectStatus,
    pub objective: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub privacy_scope_digest: String,
    pub memory_revision: u64,
    pub provider: ProviderOutcome,
    pub claims: Vec<ReflectClaim>,
    pub patterns: Vec<ReflectPattern>,
    pub tensions: Vec<ReflectTension>,
    pub chronology: Vec<ReflectChronology>,
    pub recommendations: Vec<ReflectRecommendation>,
    pub proposed_candidates: Vec<ProposedReflectCandidate>,
    pub evidence_ids: Vec<String>,
    pub metrics: ReflectMetrics,
}

/// Optional provider hook. Providers receive only already-authorized,
/// bounded records. A provider response is accepted only when every claim is
/// grounded in one of those records; otherwise the caller receives a bounded
/// failure or deterministic fallback according to `ProviderPolicy`.
pub trait ReflectionProvider: Send + Sync {
    fn name(&self) -> &str;
    fn reflect(
        &self,
        request: &ReflectRequest,
        memories: &[MemoryRecord],
        evidence: &[Evidence],
    ) -> Result<ProviderReflection, String>;
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderReflection {
    pub claims: Vec<ReflectClaim>,
    pub patterns: Vec<ReflectPattern>,
    pub tensions: Vec<ReflectTension>,
    pub chronology: Vec<ReflectChronology>,
    pub recommendations: Vec<ReflectRecommendation>,
    pub proposed_candidates: Vec<ProposedReflectCandidate>,
}

/// Reflect over authorized inputs without contacting a provider.
pub fn reflect(request: &ReflectRequest, inputs: &ReflectionInputs<'_>) -> Result<ReflectResponse> {
    reflect_with_provider(request, inputs, None)
}

/// First-party integration used by HTTP and MCP. Inputs are obtained only
/// through Cortana's existing scoped store and retrieval paths.
pub async fn reflect_authorized(
    store: &Store,
    embedder: &Arc<dyn Embedder>,
    request: &ReflectRequest,
    principal_acl: &[String],
    owner: bool,
) -> Result<ReflectResponse> {
    validate_request(request)?;
    let started = Instant::now();
    let revision = store.memory_revision()?;
    let memories = authorized_memories(store, request, principal_acl, owner)?;
    let Some(remaining) = Duration::from_millis(request.deadline_ms).checked_sub(started.elapsed())
    else {
        return Ok(deadline_exceeded_response(
            request,
            revision,
            principal_acl,
            memories.len(),
            0,
            "reflection deadline exceeded while reading memory",
        ));
    };
    let evidence = if request.include_evidence {
        let project = request.project.as_deref().ok_or_else(|| {
            anyhow::anyhow!("reflection requires a project filter when evidence is included")
        })?;
        match tokio::time::timeout(
            remaining,
            crate::retrieval::retrieve_scoped(
                store,
                embedder,
                &request.objective,
                Some(project),
                request.source.as_deref(),
                MAX_REFLECTION_EVIDENCE_LIMIT,
                principal_acl,
            ),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                return Ok(deadline_exceeded_response(
                    request,
                    revision,
                    principal_acl,
                    memories.len(),
                    0,
                    "reflection deadline exceeded while retrieving evidence",
                ));
            }
        }
    } else {
        Vec::new()
    };
    anyhow::ensure!(
        store.memory_revision()? == revision,
        "memory changed while reflection inputs were being collected"
    );
    let mut bounded_request = request.clone();
    let Some(remaining) = Duration::from_millis(request.deadline_ms).checked_sub(started.elapsed())
    else {
        return Ok(deadline_exceeded_response(
            request,
            revision,
            principal_acl,
            memories.len(),
            evidence.len(),
            "reflection deadline exceeded before synthesis",
        ));
    };
    bounded_request.deadline_ms = u64::try_from(remaining.as_millis()).unwrap_or(1).max(1);
    reflect(
        &bounded_request,
        &ReflectionInputs {
            memories: &memories,
            evidence: &evidence,
            evidence_project: request.project.as_deref(),
            principal_acl,
            owner,
            memory_revision: revision,
        },
    )
}

fn deadline_exceeded_response(
    request: &ReflectRequest,
    revision: u64,
    principal_acl: &[String],
    memories_considered: usize,
    evidence_considered: usize,
    detail: &str,
) -> ReflectResponse {
    empty_response(
        request,
        stable_json_digest(request),
        privacy_scope_digest(
            request.project.as_deref(),
            request.source.as_deref(),
            principal_acl,
        ),
        revision,
        ReflectStatus::DeadlineExceeded,
        ProviderOutcome {
            policy: request.provider_policy,
            selected: "deterministic".into(),
            status: "deadline_exceeded".into(),
            detail: Some(detail.into()),
        },
        memories_considered,
        evidence_considered,
        0,
    )
}

/// Local CLI integration when no retrieval runtime is initialized.
pub fn reflect_authorized_memory_only(
    store: &Store,
    request: &ReflectRequest,
    principal_acl: &[String],
    owner: bool,
) -> Result<ReflectResponse> {
    let started = Instant::now();
    validate_request(request)?;
    anyhow::ensure!(
        !request.include_evidence,
        "CLI reflection requires include_evidence=false; use HTTP or MCP for scoped evidence retrieval"
    );
    let revision = store.memory_revision()?;
    let memories = authorized_memories(store, request, principal_acl, owner)?;
    if started.elapsed() >= Duration::from_millis(request.deadline_ms) {
        return Ok(deadline_exceeded_response(
            request,
            revision,
            principal_acl,
            memories.len(),
            0,
            "reflection deadline exceeded while reading memory",
        ));
    }
    anyhow::ensure!(
        store.memory_revision()? == revision,
        "memory changed while reflection inputs were being collected"
    );
    let mut bounded_request = request.clone();
    let Some(remaining) = Duration::from_millis(request.deadline_ms).checked_sub(started.elapsed())
    else {
        return Ok(deadline_exceeded_response(
            request,
            revision,
            principal_acl,
            memories.len(),
            0,
            "reflection deadline exceeded before synthesis",
        ));
    };
    bounded_request.deadline_ms = u64::try_from(remaining.as_millis()).unwrap_or(1).max(1);
    reflect(
        &bounded_request,
        &ReflectionInputs {
            memories: &memories,
            evidence: &[],
            evidence_project: None,
            principal_acl,
            owner,
            memory_revision: revision,
        },
    )
}

fn authorized_memories(
    store: &Store,
    request: &ReflectRequest,
    principal_acl: &[String],
    owner: bool,
) -> Result<Vec<MemoryRecord>> {
    if owner {
        store.export_memories_with_axes_as_owner(
            request.project.as_deref(),
            request.memory.kind.as_deref(),
            request.memory.content_type.as_deref(),
            request.memory.retention_tier.as_deref(),
            request.memory.scope.as_deref(),
            request.memory.limit,
        )
    } else {
        store.export_memories_with_axes(
            request.project.as_deref(),
            request.memory.kind.as_deref(),
            request.memory.content_type.as_deref(),
            request.memory.retention_tier.as_deref(),
            request.memory.scope.as_deref(),
            request.memory.limit,
            principal_acl,
        )
    }
}

/// Main reflection entry point. This function is intentionally pure with
/// respect to the store: it never receives a `Store`, so it cannot mutate
/// canonical memory or advance the supplied revision.
pub fn reflect_with_provider(
    request: &ReflectRequest,
    inputs: &ReflectionInputs<'_>,
    provider: Option<Arc<dyn ReflectionProvider>>,
) -> Result<ReflectResponse> {
    validate_request(request)?;
    let started = Instant::now();
    let selected = authorize_and_select_memories(request, inputs)?
        .into_iter()
        .map(bound_memory_input)
        .collect::<Vec<_>>();
    let evidence = select_evidence(request, inputs)?
        .into_iter()
        .map(bound_evidence_input)
        .collect::<Vec<_>>();
    let request_digest = stable_json_digest(request);
    let scope_digest = privacy_scope_digest(
        request.project.as_deref(),
        request.source.as_deref(),
        inputs.principal_acl,
    );
    let provider_name = provider.as_ref().map(|item| item.name().to_string());

    let mut provider_outcome = ProviderOutcome {
        policy: request.provider_policy,
        selected: provider_name.unwrap_or_else(|| "deterministic".into()),
        status: "not_requested".into(),
        detail: None,
    };

    if request.provider_policy != ProviderPolicy::DeterministicOnly {
        if let Some(provider) = provider {
            if let Some(provider_key) = acquire_provider(&provider) {
                let (sender, receiver) = mpsc::sync_channel(1);
                let provider_request = request.clone();
                let provider_memories = selected.clone();
                let provider_evidence = evidence.clone();
                std::thread::spawn(move || {
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        provider.reflect(&provider_request, &provider_memories, &provider_evidence)
                    }))
                    .unwrap_or_else(|_| Err("reflection provider panicked".into()));
                    release_provider(provider_key);
                    let _ = sender.send(result);
                });
                let remaining =
                    Duration::from_millis(request.deadline_ms).saturating_sub(started.elapsed());
                match receiver.recv_timeout(remaining) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        provider_outcome.status = "deadline_exceeded".into();
                        provider_outcome.detail =
                            Some("reflection provider exceeded deadline".into());
                        if request.provider_policy == ProviderPolicy::RequireProvider {
                            return Ok(empty_response(
                                request,
                                request_digest,
                                scope_digest,
                                inputs.memory_revision,
                                ReflectStatus::DeadlineExceeded,
                                provider_outcome,
                                selected.len(),
                                inputs.evidence.len(),
                                evidence.len(),
                            ));
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        provider_outcome.status = "failed".into();
                        provider_outcome.detail = Some("reflection provider disconnected".into());
                    }
                    Ok(Ok(output)) => {
                        if let Err(error) =
                            validate_provider_output(request, &output, &selected, &evidence)
                        {
                            provider_outcome.status = "failed".into();
                            let _ = error;
                            provider_outcome.detail = Some("provider output rejected".into());
                            if request.provider_policy == ProviderPolicy::RequireProvider {
                                return Ok(empty_response(
                                    request,
                                    request_digest,
                                    scope_digest,
                                    inputs.memory_revision,
                                    ReflectStatus::ProviderFailed,
                                    provider_outcome,
                                    selected.len(),
                                    inputs.evidence.len(),
                                    evidence.len(),
                                ));
                            }
                        } else {
                            provider_outcome.status = "succeeded".into();
                            let response = response_from_output(
                                request,
                                request_digest,
                                scope_digest,
                                inputs.memory_revision,
                                ReflectStatus::Completed,
                                provider_outcome,
                                output,
                                &selected,
                                &evidence,
                                inputs.evidence.len(),
                            )?;
                            return Ok(response);
                        }
                    }
                    Ok(Err(error)) => {
                        provider_outcome.status = "failed".into();
                        let _ = error;
                        provider_outcome.detail = Some("reflection provider failed".into());
                        if request.provider_policy == ProviderPolicy::RequireProvider {
                            return Ok(empty_response(
                                request,
                                request_digest,
                                scope_digest,
                                inputs.memory_revision,
                                ReflectStatus::ProviderFailed,
                                provider_outcome,
                                selected.len(),
                                inputs.evidence.len(),
                                evidence.len(),
                            ));
                        }
                    }
                }
            } else {
                provider_outcome.status = "busy".into();
                provider_outcome.detail =
                    Some("reflection provider already has an active request".into());
                if request.provider_policy == ProviderPolicy::RequireProvider {
                    return Ok(empty_response(
                        request,
                        request_digest,
                        scope_digest,
                        inputs.memory_revision,
                        ReflectStatus::ProviderFailed,
                        provider_outcome,
                        selected.len(),
                        inputs.evidence.len(),
                        evidence.len(),
                    ));
                }
            }
        } else {
            provider_outcome.status = "unavailable".into();
            provider_outcome.detail = Some("no reflection provider is configured".into());
            if request.provider_policy == ProviderPolicy::RequireProvider {
                return Ok(empty_response(
                    request,
                    request_digest,
                    scope_digest,
                    inputs.memory_revision,
                    ReflectStatus::ProviderUnavailable,
                    provider_outcome,
                    selected.len(),
                    inputs.evidence.len(),
                    evidence.len(),
                ));
            }
        }
    }

    provider_outcome.selected = "deterministic".into();
    let fallback = deterministic_reflection(request, &selected, &evidence, started);
    let status = if request.provider_policy == ProviderPolicy::DeterministicOnly {
        ReflectStatus::Completed
    } else {
        provider_outcome.status = "fallback".into();
        ReflectStatus::Fallback
    };
    let mut response = response_from_output(
        request,
        request_digest,
        scope_digest,
        inputs.memory_revision,
        status,
        provider_outcome,
        fallback.output,
        &selected,
        &evidence,
        inputs.evidence.len(),
    )?;
    if fallback.deadline_exceeded {
        response.status = ReflectStatus::DeadlineExceeded;
        response.claims.clear();
        response.patterns.clear();
        response.tensions.clear();
        response.chronology.clear();
        response.recommendations.clear();
        response.proposed_candidates.clear();
    }
    Ok(response)
}

fn validate_request(request: &ReflectRequest) -> Result<()> {
    validate_text(
        "objective",
        &request.objective,
        MAX_REFLECTION_OBJECTIVE_BYTES,
    )?;
    if let Some(project) = request.project.as_deref() {
        validate_text("project", project, MAX_REFLECTION_PROJECT_BYTES)?;
    }
    if let Some(source) = request.source.as_deref() {
        validate_text("source", source, MAX_REFLECTION_SOURCE_BYTES)?;
    }
    anyhow::ensure!(
        (MIN_REFLECTION_TOKEN_BUDGET..=MAX_REFLECTION_TOKEN_BUDGET).contains(&request.token_budget),
        "token_budget must be between {MIN_REFLECTION_TOKEN_BUDGET} and {MAX_REFLECTION_TOKEN_BUDGET}"
    );
    anyhow::ensure!(
        (1..=MAX_REFLECTION_DEADLINE_MS).contains(&request.deadline_ms),
        "deadline_ms must be between 1 and {MAX_REFLECTION_DEADLINE_MS}"
    );
    anyhow::ensure!(
        (1..=MAX_REFLECTION_MEMORY_LIMIT).contains(&request.memory.limit),
        "memory.limit must be between 1 and {MAX_REFLECTION_MEMORY_LIMIT}"
    );
    if let Some(value) = request.memory.kind.as_deref() {
        MemoryKind::parse(value)
            .map_err(|error| anyhow::anyhow!("invalid memory.kind: {error}"))?;
    }
    if let Some(value) = request.memory.content_type.as_deref() {
        MemoryContentType::parse(value)
            .map_err(|error| anyhow::anyhow!("invalid memory.content_type: {error}"))?;
    }
    if let Some(value) = request.memory.retention_tier.as_deref() {
        MemoryRetentionTier::parse(value)
            .map_err(|error| anyhow::anyhow!("invalid memory.retention_tier: {error}"))?;
    }
    if let Some(value) = request.memory.scope.as_deref() {
        MemoryScope::parse(value)
            .map_err(|error| anyhow::anyhow!("invalid memory.scope: {error}"))?;
    }
    Ok(())
}

fn authorize_and_select_memories(
    request: &ReflectRequest,
    inputs: &ReflectionInputs<'_>,
) -> Result<Vec<MemoryRecord>> {
    let mut projects = BTreeSet::new();
    let mut selected = Vec::new();
    for memory in inputs.memories {
        anyhow::ensure!(
            inputs.owner || acl_allows(&memory.acl, inputs.principal_acl),
            "reflection input contains memory outside principal ACL"
        );
        anyhow::ensure!(
            inputs.owner || memory.scope != MemoryScope::OwnerGlobal.as_str(),
            "owner-global memory requires owner authorization"
        );
        if let Some(project) = request.project.as_deref() {
            anyhow::ensure!(
                memory.project == project,
                "reflection input crosses the requested project boundary"
            );
        }
        if !memory_is_active(memory) {
            continue;
        }
        if !memory_matches_filter(memory, &request.memory)? {
            continue;
        }
        projects.insert(memory.project.clone());
        selected.push(memory.clone());
        if selected.len() >= request.memory.limit {
            break;
        }
    }
    if request.project.is_none() && projects.len() > 1 {
        bail!("reflection requires a project filter for cross-workspace memory");
    }
    if let Some(scope) = request.memory.scope.as_deref() {
        if scope == "owner-global" && !inputs.owner {
            bail!("owner-global reflection requires owner authorization");
        }
    }
    Ok(selected)
}

fn memory_is_active(memory: &MemoryRecord) -> bool {
    if memory.status != "active" {
        return false;
    }
    let now = Utc::now();
    let started = DateTime::parse_from_rfc3339(&memory.valid_from)
        .map(|value| value.with_timezone(&Utc) <= now)
        .unwrap_or(false);
    let unexpired = memory.valid_until.as_deref().is_none_or(|value| {
        DateTime::parse_from_rfc3339(value)
            .map(|expires| expires.with_timezone(&Utc) > now)
            .unwrap_or(false)
    });
    started && unexpired
}

fn memory_matches_filter(memory: &MemoryRecord, filter: &MemoryReflectFilter) -> Result<bool> {
    let kind = filter.kind.as_deref().map(MemoryKind::parse).transpose()?;
    let content_type = filter
        .content_type
        .as_deref()
        .map(MemoryContentType::parse)
        .transpose()?;
    let retention_tier = filter
        .retention_tier
        .as_deref()
        .map(MemoryRetentionTier::parse)
        .transpose()?;
    let scope = filter
        .scope
        .as_deref()
        .map(MemoryScope::parse)
        .transpose()?;
    Ok(kind.is_none_or(|value| memory.kind == value.as_str())
        && content_type.is_none_or(|value| memory.content_type == value.as_str())
        && retention_tier.is_none_or(|value| memory.retention_tier == value.as_str())
        && scope.is_none_or(|value| memory.scope == value.as_str()))
}

fn select_evidence(
    request: &ReflectRequest,
    inputs: &ReflectionInputs<'_>,
) -> Result<Vec<Evidence>> {
    if !request.include_evidence {
        return Ok(Vec::new());
    }
    if !inputs.evidence.is_empty() {
        let project = request.project.as_deref().ok_or_else(|| {
            anyhow::anyhow!("reflection requires a project filter when evidence is included")
        })?;
        let evidence_project = inputs.evidence_project.ok_or_else(|| {
            anyhow::anyhow!("reflection evidence requires an authorized project scope")
        })?;
        anyhow::ensure!(
            project == evidence_project,
            "reflection evidence crosses the requested project boundary"
        );
    }
    let mut ids = BTreeSet::new();
    let mut evidence = Vec::new();
    for item in inputs.evidence.iter().take(MAX_REFLECTION_EVIDENCE_LIMIT) {
        anyhow::ensure!(
            ids.insert(item.chunk_id.clone()),
            "reflection evidence contains duplicate IDs"
        );
        validate_text("evidence.chunk_id", &item.chunk_id, 256)?;
        validate_text("evidence.title", &item.title, MAX_REFLECTION_TEXT_BYTES)?;
        evidence.push(item.clone());
    }
    Ok(evidence)
}

struct DeterministicResult {
    output: ProviderReflection,
    deadline_exceeded: bool,
}

fn deterministic_reflection(
    request: &ReflectRequest,
    memories: &[MemoryRecord],
    evidence: &[Evidence],
    started: Instant,
) -> DeterministicResult {
    let deadline = Duration::from_millis(request.deadline_ms);
    let mut output = ProviderReflection::default();
    for memory in memories {
        if started.elapsed() >= deadline {
            return DeterministicResult {
                output,
                deadline_exceeded: true,
            };
        }
        output.claims.push(ReflectClaim {
            text: safe_excerpt(
                &format!(
                    "{}: {}",
                    memory.title,
                    safe_excerpt(&memory.content, MAX_REFLECTION_TEXT_BYTES)
                ),
                MAX_REFLECTION_TEXT_BYTES,
            ),
            supporting_memory_ids: vec![memory.id.clone()],
            supporting_evidence_ids: Vec::new(),
        });
        output.chronology.push(ReflectChronology {
            observed_at: memory.observed_at.clone(),
            title: safe_excerpt(&memory.title, MAX_REFLECTION_TEXT_BYTES),
            memory_id: memory.id.clone(),
        });
    }
    output.chronology.sort_by(|left, right| {
        left.observed_at
            .cmp(&right.observed_at)
            .then_with(|| left.memory_id.cmp(&right.memory_id))
    });

    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for memory in memories {
        groups
            .entry(memory.content_type.clone())
            .or_default()
            .push(memory.id.clone());
    }
    for (content_type, ids) in groups {
        if started.elapsed() >= deadline {
            return DeterministicResult {
                output,
                deadline_exceeded: true,
            };
        }
        output.patterns.push(ReflectPattern {
            statement: format!(
                "{} visible {} memories share this reflection scope",
                ids.len(),
                content_type
            ),
            supporting_memory_ids: ids.clone(),
        });
        if ids.len() >= 2 && output.proposed_candidates.len() < MAX_REFLECTION_CANDIDATES {
            let project = request
                .project
                .clone()
                .or_else(|| memories.first().map(|memory| memory.project.clone()))
                .unwrap_or_default();
            output.proposed_candidates.push(ProposedReflectCandidate {
                project,
                title: format!("Observed {content_type} pattern"),
                content: format!(
                    "Review the recurring {content_type} pattern supported by {} visible memories.",
                    ids.len()
                ),
                content_type,
                retention_tier: "working".into(),
                scope: "workspace".into(),
                supporting_memory_ids: ids,
                approval_required: true,
            });
        }
    }

    for (index, left) in memories.iter().enumerate() {
        for right in memories.iter().skip(index + 1) {
            if started.elapsed() >= deadline {
                return DeterministicResult {
                    output,
                    deadline_exceeded: true,
                };
            }
            if left.project == right.project
                && left.content_type == right.content_type
                && token_similarity(&normalized_text(left), &normalized_text(right)) >= 0.35
                && has_negation(&normalized_text(left)) != has_negation(&normalized_text(right))
            {
                output.tensions.push(ReflectTension {
                    statement: format!(
                        "Visible memories `{}` and `{}` have conflicting polarity and require review",
                        left.id, right.id
                    ),
                    supporting_memory_ids: vec![left.id.clone(), right.id.clone()],
                });
                output.recommendations.push(ReflectRecommendation {
                    statement:
                        "Review the conflicting memories before retaining a new durable insight"
                            .into(),
                    supporting_memory_ids: vec![left.id.clone(), right.id.clone()],
                });
                if output.tensions.len() >= MAX_REFLECTION_TENSIONS {
                    break;
                }
            }
        }
        if output.tensions.len() >= MAX_REFLECTION_TENSIONS {
            break;
        }
    }
    if request.include_evidence && !evidence.is_empty() && !output.claims.is_empty() {
        output.claims[0].supporting_evidence_ids = evidence
            .iter()
            .take(4)
            .map(|item| item.chunk_id.clone())
            .collect();
    }
    output.claims.truncate(MAX_REFLECTION_CLAIMS);
    output.patterns.truncate(MAX_REFLECTION_PATTERNS);
    output.tensions.truncate(MAX_REFLECTION_TENSIONS);
    output
        .recommendations
        .truncate(MAX_REFLECTION_RECOMMENDATIONS);
    DeterministicResult {
        output,
        deadline_exceeded: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn response_from_output(
    request: &ReflectRequest,
    request_digest: String,
    scope_digest: String,
    memory_revision: u64,
    status: ReflectStatus,
    provider: ProviderOutcome,
    mut output: ProviderReflection,
    memories: &[MemoryRecord],
    evidence: &[Evidence],
    evidence_considered: usize,
) -> Result<ReflectResponse> {
    output.claims.truncate(MAX_REFLECTION_CLAIMS);
    output.patterns.truncate(MAX_REFLECTION_PATTERNS);
    output.tensions.truncate(MAX_REFLECTION_TENSIONS);
    output
        .recommendations
        .truncate(MAX_REFLECTION_RECOMMENDATIONS);
    output
        .proposed_candidates
        .truncate(MAX_REFLECTION_CANDIDATES);
    let evidence_ids = evidence.iter().map(|item| item.chunk_id.clone()).collect();
    let mut response = ReflectResponse {
        contract_version: REFLECTION_CONTRACT_VERSION.into(),
        request_digest,
        status,
        objective: request.objective.clone(),
        project: request.project.clone(),
        privacy_scope_digest: scope_digest,
        memory_revision,
        provider,
        claims: output.claims,
        patterns: output.patterns,
        tensions: output.tensions,
        chronology: output.chronology,
        recommendations: output.recommendations,
        proposed_candidates: output.proposed_candidates,
        evidence_ids,
        metrics: ReflectMetrics {
            memories_considered: memories.len(),
            memories_included: memories.len(),
            evidence_considered,
            evidence_included: evidence.len(),
            estimated_tokens: 0,
            memory_revision,
            canonical_memory_mutated: false,
        },
    };
    loop {
        response.metrics.estimated_tokens = estimate_tokens(&serde_json::to_string(&response)?);
        if response.metrics.estimated_tokens <= request.token_budget {
            break;
        }
        if response.proposed_candidates.pop().is_some()
            || response.recommendations.pop().is_some()
            || response.tensions.pop().is_some()
            || response.patterns.pop().is_some()
            || response.chronology.pop().is_some()
            || response.claims.pop().is_some()
            || response.evidence_ids.pop().is_some()
        {
            continue;
        }
        break;
    }
    anyhow::ensure!(
        response.metrics.estimated_tokens <= request.token_budget,
        "reflection response metadata exceeds token budget"
    );
    Ok(response)
}

fn bound_memory_input(mut memory: MemoryRecord) -> MemoryRecord {
    memory.title = safe_excerpt(&memory.title, MAX_REFLECTION_TEXT_BYTES);
    memory.content = safe_excerpt(&memory.content, MAX_REFLECTION_INPUT_TEXT_BYTES);
    memory.acl.clear();
    memory.provenance = serde_json::Value::Null;
    memory
}

fn bound_evidence_input(mut evidence: Evidence) -> Evidence {
    evidence.title = safe_excerpt(&evidence.title, MAX_REFLECTION_TEXT_BYTES);
    evidence.content = safe_excerpt(&evidence.content, MAX_REFLECTION_INPUT_TEXT_BYTES);
    evidence.source_id.clear();
    evidence.uri = None;
    evidence
}

#[allow(clippy::too_many_arguments)]
fn empty_response(
    request: &ReflectRequest,
    request_digest: String,
    scope_digest: String,
    memory_revision: u64,
    status: ReflectStatus,
    provider: ProviderOutcome,
    memories_considered: usize,
    evidence_considered: usize,
    evidence_included: usize,
) -> ReflectResponse {
    let mut response = ReflectResponse {
        contract_version: REFLECTION_CONTRACT_VERSION.into(),
        request_digest,
        status,
        objective: request.objective.clone(),
        project: request.project.clone(),
        privacy_scope_digest: scope_digest,
        memory_revision,
        provider,
        claims: Vec::new(),
        patterns: Vec::new(),
        tensions: Vec::new(),
        chronology: Vec::new(),
        recommendations: Vec::new(),
        proposed_candidates: Vec::new(),
        evidence_ids: Vec::new(),
        metrics: ReflectMetrics {
            memories_considered,
            memories_included: 0,
            evidence_considered,
            evidence_included,
            estimated_tokens: 0,
            memory_revision,
            canonical_memory_mutated: false,
        },
    };
    loop {
        response.metrics.estimated_tokens = serde_json::to_string(&response)
            .map(|serialized| estimate_tokens(&serialized))
            .unwrap_or(usize::MAX);
        if response.metrics.estimated_tokens <= request.token_budget {
            return response;
        }
        if response.provider.detail.take().is_some() || response.project.take().is_some() {
            continue;
        }
        if !response.objective.is_empty() {
            let next_bytes = response.objective.len() / 2;
            response.objective = safe_excerpt(&response.objective, next_bytes);
            continue;
        }
        return response;
    }
}

fn validate_provider_output(
    request: &ReflectRequest,
    output: &ProviderReflection,
    memories: &[MemoryRecord],
    evidence: &[Evidence],
) -> Result<()> {
    anyhow::ensure!(
        output.claims.len() <= MAX_REFLECTION_CLAIMS,
        "too many claims"
    );
    anyhow::ensure!(
        output.patterns.len() <= MAX_REFLECTION_PATTERNS,
        "too many patterns"
    );
    anyhow::ensure!(
        output.tensions.len() <= MAX_REFLECTION_TENSIONS,
        "too many tensions"
    );
    anyhow::ensure!(
        output.recommendations.len() <= MAX_REFLECTION_RECOMMENDATIONS,
        "too many recommendations"
    );
    anyhow::ensure!(
        output.proposed_candidates.len() <= MAX_REFLECTION_CANDIDATES,
        "too many proposed candidates"
    );
    anyhow::ensure!(
        output.chronology.len() <= MAX_REFLECTION_MEMORY_LIMIT,
        "too many chronology entries"
    );
    let memory_ids: BTreeSet<&str> = memories.iter().map(|item| item.id.as_str()).collect();
    let evidence_ids: BTreeSet<&str> = evidence.iter().map(|item| item.chunk_id.as_str()).collect();
    for claim in &output.claims {
        validate_text("claim.text", &claim.text, MAX_REFLECTION_TEXT_BYTES)?;
        anyhow::ensure!(
            !claim.supporting_memory_ids.is_empty() || !claim.supporting_evidence_ids.is_empty(),
            "claim support cannot be empty"
        );
        ensure_optional_ids(&claim.supporting_memory_ids, &memory_ids, "claim memory")?;
        ensure_optional_ids(
            &claim.supporting_evidence_ids,
            &evidence_ids,
            "claim evidence",
        )?;
    }
    for item in &output.patterns {
        validate_text(
            "pattern.statement",
            &item.statement,
            MAX_REFLECTION_TEXT_BYTES,
        )?;
        ensure_ids(&item.supporting_memory_ids, &memory_ids, "pattern memory")?;
    }
    for item in &output.tensions {
        validate_text(
            "tension.statement",
            &item.statement,
            MAX_REFLECTION_TEXT_BYTES,
        )?;
        ensure_ids(&item.supporting_memory_ids, &memory_ids, "tension memory")?;
    }
    for item in &output.recommendations {
        validate_text(
            "recommendation.statement",
            &item.statement,
            MAX_REFLECTION_TEXT_BYTES,
        )?;
        ensure_ids(
            &item.supporting_memory_ids,
            &memory_ids,
            "recommendation memory",
        )?;
    }
    for item in &output.proposed_candidates {
        anyhow::ensure!(
            item.approval_required,
            "provider candidate must require approval"
        );
        let requested_project = request.project.as_deref().unwrap_or_default();
        anyhow::ensure!(
            !requested_project.is_empty() && item.project == requested_project,
            "provider candidate crosses the requested project boundary"
        );
        validate_text("candidate.title", &item.title, MAX_REFLECTION_TEXT_BYTES)?;
        validate_text(
            "candidate.content",
            &item.content,
            MAX_REFLECTION_TEXT_BYTES,
        )?;
        validate_text(
            "candidate.content_type",
            &item.content_type,
            MAX_REFLECTION_SOURCE_BYTES,
        )?;
        validate_text(
            "candidate.retention_tier",
            &item.retention_tier,
            MAX_REFLECTION_SOURCE_BYTES,
        )?;
        validate_text("candidate.scope", &item.scope, MAX_REFLECTION_SOURCE_BYTES)?;
        MemoryContentType::parse(&item.content_type)?;
        MemoryRetentionTier::parse(&item.retention_tier)?;
        let scope = MemoryScope::parse(&item.scope)?;
        anyhow::ensure!(
            scope == MemoryScope::Workspace,
            "provider candidate must remain workspace-scoped"
        );
        ensure_ids(&item.supporting_memory_ids, &memory_ids, "candidate memory")?;
    }
    for item in &output.chronology {
        validate_text("chronology.title", &item.title, MAX_REFLECTION_TEXT_BYTES)?;
        validate_text("chronology.observed_at", &item.observed_at, 64)?;
        DateTime::parse_from_rfc3339(&item.observed_at)
            .map_err(|_| anyhow::anyhow!("chronology timestamp is invalid"))?;
        anyhow::ensure!(
            memory_ids.contains(item.memory_id.as_str()),
            "chronology memory is ungrounded"
        );
    }
    serde_json::to_writer(BoundedWriter::new(MAX_PROVIDER_OUTPUT_BYTES), output)
        .map_err(|error| anyhow::anyhow!("provider output exceeds its aggregate bound: {error}"))?;
    ensure_no_private_evidence_echo(&provider_output_text(output), evidence)?;
    Ok(())
}

struct BoundedWriter {
    remaining: usize,
}

impl BoundedWriter {
    fn new(limit: usize) -> Self {
        Self { remaining: limit }
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > self.remaining {
            return Err(io::Error::other("serialized value is too large"));
        }
        self.remaining -= buffer.len();
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn ensure_no_private_evidence_echo(serialized_output: &str, evidence: &[Evidence]) -> Result<()> {
    let normalized_output = serialized_output.to_lowercase();
    let output_characters = normalized_output.chars().collect::<Vec<_>>();
    let output_windows = output_characters
        .windows(MIN_PRIVATE_EVIDENCE_FINGERPRINT_CHARS)
        .map(|window| window.iter().collect::<String>())
        .collect::<HashSet<_>>();
    for item in evidence {
        let normalized = item.content.to_lowercase();
        let characters = normalized.chars().collect::<Vec<_>>();
        if characters.len() < MIN_PRIVATE_EVIDENCE_FINGERPRINT_CHARS {
            anyhow::ensure!(
                normalized.is_empty() || !normalized_output.contains(&normalized),
                "provider output echoes private evidence content"
            );
            continue;
        }
        anyhow::ensure!(
            !characters
                .windows(MIN_PRIVATE_EVIDENCE_FINGERPRINT_CHARS)
                .any(|window| { output_windows.contains(&window.iter().collect::<String>()) }),
            "provider output echoes private evidence content"
        );
    }
    Ok(())
}

fn provider_output_text(output: &ProviderReflection) -> String {
    let mut values = Vec::new();
    values.extend(output.claims.iter().map(|item| item.text.as_str()));
    values.extend(output.patterns.iter().map(|item| item.statement.as_str()));
    values.extend(output.tensions.iter().map(|item| item.statement.as_str()));
    values.extend(
        output
            .recommendations
            .iter()
            .map(|item| item.statement.as_str()),
    );
    for item in &output.proposed_candidates {
        values.push(item.title.as_str());
        values.push(item.content.as_str());
    }
    values.extend(output.chronology.iter().map(|item| item.title.as_str()));
    values.join("\n")
}

fn ensure_optional_ids(ids: &[String], allowed: &BTreeSet<&str>, label: &str) -> Result<()> {
    anyhow::ensure!(
        ids.len() <= allowed.len(),
        "{label} contains too many references"
    );
    anyhow::ensure!(
        ids.iter().all(|id| allowed.contains(id.as_str())),
        "{label} contains an unknown reference"
    );
    validate_reference_ids(ids, label)?;
    Ok(())
}

fn ensure_ids(ids: &[String], allowed: &BTreeSet<&str>, label: &str) -> Result<()> {
    anyhow::ensure!(!ids.is_empty(), "{label} support cannot be empty");
    anyhow::ensure!(
        ids.len() <= allowed.len(),
        "{label} contains too many references"
    );
    anyhow::ensure!(
        ids.iter().all(|id| allowed.contains(id.as_str())),
        "{label} contains an unknown reference"
    );
    validate_reference_ids(ids, label)?;
    Ok(())
}

fn validate_reference_ids(ids: &[String], label: &str) -> Result<()> {
    anyhow::ensure!(
        ids.iter().all(|id| id.len() <= MAX_REFLECTION_SOURCE_BYTES),
        "{label} contains an oversized reference"
    );
    let unique = ids.iter().collect::<BTreeSet<_>>();
    anyhow::ensure!(unique.len() == ids.len(), "{label} contains duplicates");
    Ok(())
}

fn validate_text(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    anyhow::ensure!(!value.trim().is_empty(), "{name} must not be empty");
    anyhow::ensure!(value.len() <= max_bytes, "{name} exceeds {max_bytes} bytes");
    anyhow::ensure!(
        !value.chars().any(char::is_control),
        "{name} must not contain control characters"
    );
    Ok(())
}

fn safe_excerpt(value: &str, max_bytes: usize) -> String {
    let sanitized = value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    if sanitized.len() <= max_bytes {
        return sanitized;
    }
    let mut end = max_bytes;
    while !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    sanitized[..end].to_owned()
}

fn normalized_text(memory: &MemoryRecord) -> String {
    format!("{} {}", memory.title, memory.content)
        .to_ascii_lowercase()
        .split_whitespace()
        .map(|word| word.trim_matches(|character: char| !character.is_ascii_alphanumeric()))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn token_similarity(left: &str, right: &str) -> f64 {
    let left: BTreeSet<&str> = left.split_whitespace().collect();
    let right: BTreeSet<&str> = right.split_whitespace().collect();
    let shared = left.intersection(&right).count();
    let total = left.len() + right.len();
    if total == 0 {
        0.0
    } else {
        (2 * shared) as f64 / total as f64
    }
}

fn has_negation(value: &str) -> bool {
    value.split_whitespace().any(|word| {
        matches!(
            word,
            "no" | "not" | "never" | "dont" | "don't" | "cannot" | "can't"
        )
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::*;

    fn memory(id: &str, content: &str, project: &str, acl: &[&str]) -> MemoryRecord {
        MemoryRecord {
            id: id.into(),
            kind: "semantic".into(),
            content_type: "semantic".into(),
            retention_tier: "durable".into(),
            scope: "workspace".into(),
            project: project.into(),
            title: format!("Memory {id}"),
            content: content.into(),
            source: "test".into(),
            source_id: format!("source-{id}"),
            dedupe_key: None,
            confidence: 0.9,
            importance: 0.5,
            status: "active".into(),
            acl: acl.iter().map(|item| (*item).into()).collect(),
            provenance: json!({}),
            observed_at: Utc::now().to_rfc3339(),
            valid_from: Utc::now().to_rfc3339(),
            valid_until: None,
            supersedes_id: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    fn request() -> ReflectRequest {
        ReflectRequest {
            objective: "find durable work patterns".into(),
            project: Some("work".into()),
            memory: MemoryReflectFilter::default(),
            include_evidence: false,
            token_budget: 2_048,
            provider_policy: ProviderPolicy::DeterministicOnly,
            deadline_ms: 5_000,
            source: None,
        }
    }

    #[test]
    fn deterministic_reflection_is_grounded_and_non_mutating() {
        let memories = vec![
            memory("m1", "the release checklist is required", "work", &["work"]),
            memory(
                "m2",
                "the release checklist is not required",
                "work",
                &["work"],
            ),
        ];
        let inputs = ReflectionInputs {
            memories: &memories,
            evidence: &[],
            evidence_project: None,
            principal_acl: &["work".into()],
            owner: false,
            memory_revision: 42,
        };
        let response = reflect(&request(), &inputs).expect("reflection");
        assert_eq!(response.status, ReflectStatus::Completed);
        assert!(!response.tensions.is_empty());
        assert!(!response.proposed_candidates.is_empty());
        assert!(response.proposed_candidates[0].approval_required);
        assert!(!response.proposed_candidates[0].project.is_empty());
        assert_eq!(response.memory_revision, 42);
        assert!(!response.metrics.canonical_memory_mutated);
        assert!(
            response
                .claims
                .iter()
                .all(|claim| !claim.supporting_memory_ids.is_empty()
                    && claim.text.len() <= MAX_REFLECTION_TEXT_BYTES)
        );
    }

    #[test]
    fn rejects_cross_workspace_and_acl_violations() {
        let memories = vec![memory("m1", "private", "personal", &["personal"])];
        let mut request = request();
        request.project = None;
        let inputs = ReflectionInputs {
            memories: &memories,
            evidence: &[],
            evidence_project: None,
            principal_acl: &["work".into()],
            owner: false,
            memory_revision: 1,
        };
        assert!(reflect(&request, &inputs).is_err());
    }

    #[test]
    fn provider_required_reports_unavailable_without_fabricating_results() {
        let memories = vec![memory("m1", "release policy", "work", &["work"])];
        let inputs = ReflectionInputs {
            memories: &memories,
            evidence: &[],
            evidence_project: None,
            principal_acl: &["work".into()],
            owner: false,
            memory_revision: 7,
        };
        let mut request = request();
        request.provider_policy = ProviderPolicy::RequireProvider;
        let response = reflect(&request, &inputs).expect("bounded provider outcome");
        assert_eq!(response.status, ReflectStatus::ProviderUnavailable);
        assert!(response.claims.is_empty());
        assert_eq!(response.memory_revision, 7);

        let mut tight_request = request.clone();
        tight_request.objective = "x".repeat(MAX_REFLECTION_OBJECTIVE_BYTES);
        tight_request.project = Some("p".repeat(MAX_REFLECTION_PROJECT_BYTES));
        tight_request.token_budget = MIN_REFLECTION_TOKEN_BUDGET;
        let response = deadline_exceeded_response(
            &tight_request,
            7,
            &["work".into()],
            1,
            0,
            "reflection deadline exceeded while reading memory",
        );
        assert!(response.metrics.estimated_tokens > 0);
        assert!(response.metrics.estimated_tokens <= tight_request.token_budget);
        assert!(response.claims.is_empty());
    }

    #[test]
    fn preferred_provider_falls_back_when_provider_fails() {
        struct Failing;
        impl ReflectionProvider for Failing {
            fn name(&self) -> &str {
                "test-provider"
            }
            fn reflect(
                &self,
                _: &ReflectRequest,
                _: &[MemoryRecord],
                _: &[Evidence],
            ) -> Result<ProviderReflection, String> {
                Err("provider failed with private source content".into())
            }
        }
        let memories = vec![memory("m1", "release policy", "work", &["work"])];
        let inputs = ReflectionInputs {
            memories: &memories,
            evidence: &[],
            evidence_project: None,
            principal_acl: &["work".into()],
            owner: false,
            memory_revision: 9,
        };
        let mut request = request();
        request.provider_policy = ProviderPolicy::PreferProvider;
        let response =
            reflect_with_provider(&request, &inputs, Some(Arc::new(Failing))).expect("fallback");
        assert_eq!(response.status, ReflectStatus::Fallback);
        assert_eq!(response.provider.status, "fallback");
        assert_eq!(
            response.provider.detail.as_deref(),
            Some("reflection provider failed")
        );
        assert!(!response.claims.is_empty());
    }

    #[test]
    fn evidence_requires_matching_project_and_is_id_based() {
        let memories = vec![memory("m1", "release policy", "work", &["work"])];
        let evidence = vec![Evidence {
            chunk_id: "doc-1:0".into(),
            source: "notes".into(),
            source_id: "note-1".into(),
            title: "Release note".into(),
            uri: None,
            content: "private source content".into(),
            score: 1.0,
            semantic_rank: Some(1),
            lexical_rank: Some(1),
            updated_at: Utc::now(),
        }];
        let mut request = request();
        request.include_evidence = true;
        let inputs = ReflectionInputs {
            memories: &memories,
            evidence: &evidence,
            evidence_project: Some("work"),
            principal_acl: &["work".into()],
            owner: false,
            memory_revision: 2,
        };
        let response = reflect(&request, &inputs).expect("reflection");
        assert_eq!(response.evidence_ids, vec!["doc-1:0"]);
        assert!(
            !serde_json::to_string(&response)
                .unwrap()
                .contains("private source content")
        );

        struct Paraphrase;
        impl ReflectionProvider for Paraphrase {
            fn name(&self) -> &str {
                "paraphrase"
            }
            fn reflect(
                &self,
                _: &ReflectRequest,
                _: &[MemoryRecord],
                _: &[Evidence],
            ) -> Result<ProviderReflection, String> {
                Ok(ProviderReflection {
                    claims: vec![ReflectClaim {
                        text: "The source supports the release policy".into(),
                        supporting_memory_ids: Vec::new(),
                        supporting_evidence_ids: vec!["doc-1:0".into()],
                    }],
                    ..Default::default()
                })
            }
        }
        request.provider_policy = ProviderPolicy::PreferProvider;
        let response = reflect_with_provider(&request, &inputs, Some(Arc::new(Paraphrase)))
            .expect("grounded paraphrase succeeds");
        assert_eq!(response.status, ReflectStatus::Completed);
        assert_eq!(response.provider.status, "succeeded");

        struct Echo;
        impl ReflectionProvider for Echo {
            fn name(&self) -> &str {
                "echo"
            }
            fn reflect(
                &self,
                _: &ReflectRequest,
                _: &[MemoryRecord],
                _: &[Evidence],
            ) -> Result<ProviderReflection, String> {
                Ok(ProviderReflection {
                    claims: vec![ReflectClaim {
                        text: "private source".into(),
                        supporting_memory_ids: Vec::new(),
                        supporting_evidence_ids: vec!["doc-1:0".into()],
                    }],
                    ..Default::default()
                })
            }
        }
        let response = reflect_with_provider(&request, &inputs, Some(Arc::new(Echo)))
            .expect("private echo falls back");
        assert_eq!(response.status, ReflectStatus::Fallback);
        assert_eq!(response.provider.status, "fallback");
    }

    #[test]
    fn rejects_unscoped_evidence_and_explicit_cross_project_inputs() {
        let memories = vec![memory("m1", "release policy", "other", &["work"])];
        let evidence = vec![Evidence {
            chunk_id: "doc-1:0".into(),
            source: "notes".into(),
            source_id: "note-1".into(),
            title: "Release note".into(),
            uri: None,
            content: "scoped content".into(),
            score: 1.0,
            semantic_rank: Some(1),
            lexical_rank: Some(1),
            updated_at: Utc::now(),
        }];
        let mut request = request();
        request.include_evidence = true;
        let inputs = ReflectionInputs {
            memories: &[],
            evidence: &evidence,
            evidence_project: None,
            principal_acl: &["work".into()],
            owner: false,
            memory_revision: 1,
        };
        assert!(reflect(&request, &inputs).is_err());
        let inputs = ReflectionInputs {
            memories: &memories,
            evidence: &[],
            evidence_project: None,
            principal_acl: &["work".into()],
            owner: false,
            memory_revision: 1,
        };
        request.include_evidence = false;
        assert!(reflect(&request, &inputs).is_err());
    }

    #[test]
    fn inactive_memories_are_excluded_and_response_degrades_to_budget() {
        let mut memories = (0..40)
            .map(|index| {
                memory(
                    &format!("m{index}"),
                    &"detail ".repeat(100),
                    "work",
                    &["work"],
                )
            })
            .collect::<Vec<_>>();
        memories[0].status = "superseded".into();
        memories[1].valid_until = Some("2000-01-01T00:00:00Z".into());
        let inputs = ReflectionInputs {
            memories: &memories,
            evidence: &[],
            evidence_project: None,
            principal_acl: &["work".into()],
            owner: false,
            memory_revision: 1,
        };
        let mut request = request();
        request.token_budget = MIN_REFLECTION_TOKEN_BUDGET;
        request.memory.limit = 40;
        let response = reflect(&request, &inputs).expect("bounded reflection");
        assert_eq!(response.metrics.memories_included, 38);
        assert!(response.metrics.estimated_tokens <= request.token_budget);
        assert!(
            response
                .claims
                .iter()
                .all(|claim| claim.supporting_memory_ids != ["m0"]
                    && claim.supporting_memory_ids != ["m1"])
        );
    }

    #[test]
    fn provider_timeout_and_invalid_candidate_fail_closed() {
        struct Slow;
        impl ReflectionProvider for Slow {
            fn name(&self) -> &str {
                "slow"
            }
            fn reflect(
                &self,
                _: &ReflectRequest,
                _: &[MemoryRecord],
                _: &[Evidence],
            ) -> Result<ProviderReflection, String> {
                std::thread::sleep(Duration::from_millis(250));
                Ok(ProviderReflection::default())
            }
        }
        let memories = vec![memory("m1", "release policy", "work", &["work"])];
        let inputs = ReflectionInputs {
            memories: &memories,
            evidence: &[],
            evidence_project: None,
            principal_acl: &["work".into()],
            owner: false,
            memory_revision: 1,
        };
        let mut request = request();
        request.provider_policy = ProviderPolicy::RequireProvider;
        request.deadline_ms = 1;
        let slow: Arc<dyn ReflectionProvider> = Arc::new(Slow);
        let response =
            reflect_with_provider(&request, &inputs, Some(slow.clone())).expect("deadline outcome");
        assert_eq!(response.status, ReflectStatus::DeadlineExceeded);
        request.deadline_ms = 5_000;
        let response =
            reflect_with_provider(&request, &inputs, Some(slow)).expect("busy provider outcome");
        assert_eq!(response.status, ReflectStatus::ProviderFailed);
        assert_eq!(response.provider.status, "busy");

        struct Invalid;
        impl ReflectionProvider for Invalid {
            fn name(&self) -> &str {
                "invalid"
            }
            fn reflect(
                &self,
                _: &ReflectRequest,
                _: &[MemoryRecord],
                _: &[Evidence],
            ) -> Result<ProviderReflection, String> {
                Ok(ProviderReflection {
                    proposed_candidates: vec![ProposedReflectCandidate {
                        project: "other".into(),
                        title: "unsafe".into(),
                        content: "unsafe".into(),
                        content_type: "semantic".into(),
                        retention_tier: "durable".into(),
                        scope: "owner-global".into(),
                        supporting_memory_ids: vec!["m1".into()],
                        approval_required: true,
                    }],
                    ..Default::default()
                })
            }
        }
        request.provider_policy = ProviderPolicy::PreferProvider;
        request.deadline_ms = 5_000;
        let response =
            reflect_with_provider(&request, &inputs, Some(Arc::new(Invalid))).expect("fallback");
        assert_eq!(response.status, ReflectStatus::Fallback);
        assert!(
            response
                .proposed_candidates
                .iter()
                .all(|item| item.project == "work")
        );
    }

    #[test]
    fn provider_panic_releases_concurrency_guard_for_retry() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct PanicOnce(AtomicUsize);
        impl ReflectionProvider for PanicOnce {
            fn name(&self) -> &str {
                "panic-once"
            }
            fn reflect(
                &self,
                _: &ReflectRequest,
                _: &[MemoryRecord],
                _: &[Evidence],
            ) -> Result<ProviderReflection, String> {
                if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("provider panic");
                }
                Ok(ProviderReflection {
                    claims: vec![ReflectClaim {
                        text: "The release policy is active".into(),
                        supporting_memory_ids: vec!["m1".into()],
                        supporting_evidence_ids: Vec::new(),
                    }],
                    ..Default::default()
                })
            }
        }

        let memories = vec![memory("m1", "release policy", "work", &["work"])];
        let inputs = ReflectionInputs {
            memories: &memories,
            evidence: &[],
            evidence_project: None,
            principal_acl: &["work".into()],
            owner: false,
            memory_revision: 1,
        };
        let mut request = request();
        request.provider_policy = ProviderPolicy::PreferProvider;
        let provider: Arc<dyn ReflectionProvider> = Arc::new(PanicOnce(AtomicUsize::new(0)));

        let first = reflect_with_provider(&request, &inputs, Some(provider.clone()))
            .expect("panic falls back");
        assert_eq!(first.status, ReflectStatus::Fallback);
        assert_eq!(first.provider.status, "fallback");

        let retry =
            reflect_with_provider(&request, &inputs, Some(provider)).expect("retry succeeds");
        assert_eq!(retry.status, ReflectStatus::Completed);
        assert_eq!(retry.provider.status, "succeeded");
    }

    #[test]
    fn provider_reference_and_aggregate_bounds_fail_before_response_serialization() {
        let memories = (0..MAX_REFLECTION_MEMORY_LIMIT)
            .map(|index| {
                memory(
                    &format!("m{index:03}-{}", "x".repeat(120)),
                    "release policy",
                    "work",
                    &["work"],
                )
            })
            .collect::<Vec<_>>();
        let all_ids = memories
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();

        let duplicate = ProviderReflection {
            claims: vec![ReflectClaim {
                text: "Duplicate support".into(),
                supporting_memory_ids: vec![all_ids[0].clone(), all_ids[0].clone()],
                supporting_evidence_ids: Vec::new(),
            }],
            ..Default::default()
        };
        assert!(validate_provider_output(&request(), &duplicate, &memories, &[]).is_err());

        let oversized_aggregate = ProviderReflection {
            patterns: (0..MAX_REFLECTION_PATTERNS)
                .map(|index| ReflectPattern {
                    statement: format!("Pattern {index}"),
                    supporting_memory_ids: all_ids.clone(),
                })
                .collect(),
            ..Default::default()
        };
        assert!(
            validate_provider_output(&request(), &oversized_aggregate, &memories, &[]).is_err()
        );
    }
}
