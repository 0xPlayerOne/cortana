//! Bounded, non-mutating reflection over authorized memory and evidence.
//!
//! Reflection is deliberately separate from `remember` and candidate
//! promotion.  It may describe patterns or propose an insight, but it never
//! writes the canonical memory table and never advances `memory_revision`.
//! Transport layers (HTTP, MCP, CLI, and Desktop) should authorize their
//! principal first, then pass only the bounded, scoped inputs to this module.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::auth::acl_allows;
use crate::context::estimate_tokens;
use crate::contracts::{privacy_scope_digest, stable_json_digest};
use crate::memory::{
    MemoryContentType, MemoryKind, MemoryRecord, MemoryRetentionTier, MemoryScope,
};
use crate::model::Evidence;

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
pub const MAX_REFLECTION_DEADLINE_MS: u64 = 30_000;
pub const MAX_REFLECTION_TEXT_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
pub trait ReflectionProvider {
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

/// Main reflection entry point. This function is intentionally pure with
/// respect to the store: it never receives a `Store`, so it cannot mutate
/// canonical memory or advance the supplied revision.
pub fn reflect_with_provider(
    request: &ReflectRequest,
    inputs: &ReflectionInputs<'_>,
    provider: Option<&dyn ReflectionProvider>,
) -> Result<ReflectResponse> {
    validate_request(request)?;
    let started = Instant::now();
    let selected = authorize_and_select_memories(request, inputs)?;
    let evidence = select_evidence(request, inputs)?;
    let request_digest = stable_json_digest(request);
    let scope_digest = privacy_scope_digest(
        request.project.as_deref(),
        request.source.as_deref(),
        inputs.principal_acl,
    );
    let provider_name = provider.map(|item| item.name().to_string());

    let mut provider_outcome = ProviderOutcome {
        policy: request.provider_policy,
        selected: provider_name.unwrap_or_else(|| "deterministic".into()),
        status: "not_requested".into(),
        detail: None,
    };

    if request.provider_policy != ProviderPolicy::DeterministicOnly {
        if let Some(provider) = provider {
            match provider.reflect(request, &selected, &evidence) {
                Ok(output) => {
                    if let Err(error) = validate_provider_output(&output, &selected, &evidence) {
                        provider_outcome.status = "failed".into();
                        provider_outcome.detail = Some(error.to_string());
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
                Err(error) => {
                    provider_outcome.status = "failed".into();
                    provider_outcome.detail = Some(sanitize_detail(&error));
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
        if let Some(project) = request.project.as_deref() {
            if memory.project != project {
                continue;
            }
        }
        if !memory_matches_filter(memory, &request.memory)? {
            continue;
        }
        anyhow::ensure!(
            inputs.owner || acl_allows(&memory.acl, inputs.principal_acl),
            "reflection input contains memory outside principal ACL"
        );
        anyhow::ensure!(
            inputs.owner || memory.scope != MemoryScope::OwnerGlobal.as_str(),
            "owner-global memory requires owner authorization"
        );
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
    if let (Some(project), Some(evidence_project)) =
        (request.project.as_deref(), inputs.evidence_project)
    {
        anyhow::ensure!(
            project == evidence_project,
            "reflection evidence crosses the requested project boundary"
        );
    } else if !inputs.evidence.is_empty() && request.project.is_none() {
        bail!("reflection requires a project filter when evidence is included");
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
            text: format!(
                "{}: {}",
                memory.title,
                safe_excerpt(&memory.content, MAX_REFLECTION_TEXT_BYTES)
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
            let project = request.project.clone().unwrap_or_default();
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
    response.metrics.estimated_tokens = estimate_tokens(&serde_json::to_string(&response)?);
    anyhow::ensure!(
        response.metrics.estimated_tokens <= request.token_budget,
        "reflection response exceeds token budget"
    );
    Ok(response)
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
    ReflectResponse {
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
    }
}

fn validate_provider_output(
    output: &ProviderReflection,
    memories: &[MemoryRecord],
    evidence: &[Evidence],
) -> Result<()> {
    let memory_ids: BTreeSet<&str> = memories.iter().map(|item| item.id.as_str()).collect();
    let evidence_ids: BTreeSet<&str> = evidence.iter().map(|item| item.chunk_id.as_str()).collect();
    for claim in &output.claims {
        ensure_ids(&claim.supporting_memory_ids, &memory_ids, "claim memory")?;
        ensure_ids(
            &claim.supporting_evidence_ids,
            &evidence_ids,
            "claim evidence",
        )?;
    }
    for item in &output.patterns {
        ensure_ids(&item.supporting_memory_ids, &memory_ids, "pattern memory")?;
    }
    for item in &output.tensions {
        ensure_ids(&item.supporting_memory_ids, &memory_ids, "tension memory")?;
    }
    for item in &output.recommendations {
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
        ensure_ids(&item.supporting_memory_ids, &memory_ids, "candidate memory")?;
    }
    for item in &output.chronology {
        anyhow::ensure!(
            memory_ids.contains(item.memory_id.as_str()),
            "chronology memory is ungrounded"
        );
    }
    Ok(())
}

fn ensure_ids(ids: &[String], allowed: &BTreeSet<&str>, label: &str) -> Result<()> {
    anyhow::ensure!(!ids.is_empty(), "{label} support cannot be empty");
    anyhow::ensure!(
        ids.iter().all(|id| allowed.contains(id.as_str())),
        "{label} contains an unknown reference"
    );
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
    value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>()
        .chars()
        .take(max_bytes)
        .collect()
}

fn sanitize_detail(value: &str) -> String {
    safe_excerpt(value, 256)
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
        assert_eq!(response.memory_revision, 42);
        assert!(!response.metrics.canonical_memory_mutated);
        assert!(
            response
                .claims
                .iter()
                .all(|claim| !claim.supporting_memory_ids.is_empty())
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
                Err("provider timed out".into())
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
        let response = reflect_with_provider(&request, &inputs, Some(&Failing)).expect("fallback");
        assert_eq!(response.status, ReflectStatus::Fallback);
        assert_eq!(response.provider.status, "fallback");
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
    }
}
