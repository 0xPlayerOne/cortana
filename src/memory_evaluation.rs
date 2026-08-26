//! Deterministic native-memory intelligence quality and safety gates.
//!
//! The checked-in lane uses only synthetic content in disposable stores. An
//! approved private fixture may tighten thresholds and add operator evidence,
//! but automatic retention remains disabled until that separate gate passes.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::classification::{
    CandidateClassification, ClassificationProvider, classify_with_optional_provider,
};
use crate::consolidation::ConsolidationPolicy;
use crate::contracts::stable_json_digest;
use crate::derived::derive_authorized_memory;
use crate::memory::{MemoryInput, MemoryRecord};
use crate::observation::{ObservationCandidate, ObservationCandidateInput};
use crate::reflection::{
    MemoryReflectFilter, ProviderPolicy, ProviderReflection, ReflectRequest, ReflectionInputs,
    ReflectionProvider, reflect_with_provider,
};
use crate::store::Store;

pub const MEMORY_EVALUATION_CONTRACT_VERSION: &str = "cortana.memory-evaluation.v1";
const MAX_FIXTURE_BYTES: u64 = 1024 * 1024;
const MAX_CASES: usize = 64;
const MAX_PRIVATE_CPU_SECONDS: f64 = 60.0;
const MAX_PRIVATE_PEAK_RSS_BYTES: u64 = 2 * 1024 * 1024 * 1024;

const REQUIRED_CASES: &[(&str, &str, FailureDomain)] = &[
    (
        "explicit-idempotent-recall",
        "recall",
        FailureDomain::Corpus,
    ),
    (
        "expiry-excludes-stale-working",
        "retention",
        FailureDomain::Policy,
    ),
    (
        "redaction-removes-recall",
        "redaction",
        FailureDomain::Policy,
    ),
    (
        "acl-cross-workspace-denial",
        "authorization",
        FailureDomain::Policy,
    ),
    (
        "candidate-safety-boundaries",
        "candidate",
        FailureDomain::Candidate,
    ),
    (
        "classification-exact-duplicate",
        "classification",
        FailureDomain::Candidate,
    ),
    (
        "classification-contradiction",
        "classification",
        FailureDomain::Candidate,
    ),
    (
        "classification-supersession",
        "classification",
        FailureDomain::Candidate,
    ),
    (
        "classification-provider-failure",
        "classification",
        FailureDomain::Model,
    ),
    (
        "automatic-retention-remains-disabled",
        "activation",
        FailureDomain::Policy,
    ),
    (
        "consolidation-idempotency",
        "consolidation",
        FailureDomain::Policy,
    ),
    (
        "policy-change-rechecks-candidate",
        "consolidation",
        FailureDomain::Policy,
    ),
    (
        "backup-restore-preserves-memory",
        "recovery",
        FailureDomain::Corpus,
    ),
    ("reflection-grounding", "reflection", FailureDomain::Model),
    (
        "reflection-provider-failure",
        "reflection",
        FailureDomain::Model,
    ),
    (
        "derived-representation-invalidation",
        "derived",
        FailureDomain::Corpus,
    ),
];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEvaluationFixture {
    pub version: u32,
    pub fixture_class: String,
    pub thresholds: MemoryEvaluationThresholds,
    pub cases: Vec<MemoryEvaluationCase>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEvaluationThresholds {
    pub min_candidate_precision: f64,
    pub min_candidate_recall: f64,
    pub min_classification_accuracy: f64,
    pub min_lifecycle_pass_rate: f64,
    pub min_reflection_grounding: f64,
    pub min_duplicate_suppression: f64,
    pub min_contradiction_detection: f64,
    pub min_supersession_correctness: f64,
    pub min_retention_accuracy: f64,
    pub min_recall_quality: f64,
    pub min_derived_invalidation: f64,
    pub max_unauthorized_exposures: usize,
    pub max_unsupported_reflection_claims: usize,
    pub max_automatic_retention_without_private_gate: usize,
    pub max_approval_load: f64,
    pub max_provider_requests: usize,
    pub max_estimated_provider_cost_usd: f64,
    pub max_disposable_store_cases: usize,
    pub max_case_store_bytes: u64,
    pub max_total_store_bytes: u64,
    pub max_case_latency_ms: u64,
    pub max_total_case_latency_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEvaluationCase {
    pub id: String,
    pub category: String,
    pub failure_domain: FailureDomain,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FailureDomain {
    Candidate,
    Model,
    Policy,
    Corpus,
}

impl MemoryFailureDomainCounts {
    fn increment(&mut self, domain: FailureDomain) {
        match domain {
            FailureDomain::Candidate => self.candidate += 1,
            FailureDomain::Model => self.model += 1,
            FailureDomain::Policy => self.policy += 1,
            FailureDomain::Corpus => self.corpus += 1,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryEvaluationReport {
    pub contract_version: String,
    pub fixture_version: u32,
    pub fixture_class: String,
    pub fixture_digest: String,
    pub passed: bool,
    pub thresholds: MemoryEvaluationThresholds,
    pub metrics: MemoryEvaluationMetrics,
    pub cases: Vec<MemoryCaseReport>,
    pub baseline: MemoryBaselineComparison,
    pub comparisons: Vec<MemoryFeatureComparison>,
    pub activation: MemoryActivationRecord,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryEvaluationMetrics {
    pub candidate_precision: f64,
    pub candidate_recall: f64,
    pub classification_accuracy: f64,
    pub duplicate_suppression: f64,
    pub contradiction_detection: f64,
    pub supersession_correctness: f64,
    pub lifecycle_pass_rate: f64,
    pub retention_accuracy: f64,
    pub recall_quality: f64,
    pub reflection_grounding: f64,
    pub derived_invalidation: f64,
    pub unauthorized_exposures: usize,
    pub unsupported_reflection_claims: usize,
    pub automatic_retentions_without_private_gate: usize,
    pub approval_load: f64,
    pub max_case_latency_ms: u64,
    pub total_case_latency_ms: u64,
    pub provider_requests: usize,
    pub estimated_provider_cost_usd: f64,
    pub disposable_store_cases: usize,
    pub max_case_store_bytes: u64,
    pub total_store_bytes: u64,
    pub failures_by_domain: MemoryFailureDomainCounts,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MemoryFailureDomainCounts {
    pub candidate: usize,
    pub model: usize,
    pub policy: usize,
    pub corpus: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryCaseReport {
    pub id: String,
    pub category: String,
    pub failure_domain: FailureDomain,
    pub passed: bool,
    pub latency_ms: u64,
    pub reason_code: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryBaselineComparison {
    pub explicit_memory_passed: bool,
    pub intelligence_disabled_preserves_explicit_memory: bool,
    pub canonical_revision_stable_during_reflection: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryFeatureComparison {
    pub capability: String,
    pub passed: bool,
    pub explicit_memory_preserved: bool,
    pub canonical_write_requires_approval: bool,
    pub failure_domain: Option<FailureDomain>,
    pub reason_code: String,
}

#[derive(Clone, Debug, Default)]
struct CaseMeasurement {
    expected_candidates: usize,
    proposed_candidates: usize,
    true_positive_candidates: usize,
    approval_eligible: usize,
    approval_required: usize,
    provider_requests: usize,
    estimated_provider_cost_usd: f64,
}

struct FeatureComparisonOutcome {
    comparison: MemoryFeatureComparison,
    measurement: CaseMeasurement,
    latency_ms: u64,
}

#[derive(Debug)]
struct FailureAttribution {
    domain: FailureDomain,
    reason_code: &'static str,
}

impl std::fmt::Display for FailureAttribution {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.reason_code)
    }
}

impl std::error::Error for FailureAttribution {}

fn attributed<T>(domain: FailureDomain, reason_code: &'static str, result: Result<T>) -> Result<T> {
    result.map_err(|error| {
        if error.downcast_ref::<FailureAttribution>().is_some() {
            error
        } else {
            error.context(FailureAttribution {
                domain,
                reason_code,
            })
        }
    })
}

fn failure_check(domain: FailureDomain, reason_code: &'static str, condition: bool) -> Result<()> {
    attributed(
        domain,
        reason_code,
        if condition {
            Ok(())
        } else {
            Err(anyhow::anyhow!(reason_code))
        },
    )
}

fn expected_policy_rejection<T>(result: Result<T>, expected_reason: &str) -> Result<bool> {
    match result {
        Ok(_) => Ok(false),
        Err(error) if error.to_string() == expected_reason => Ok(true),
        Err(error) => Err(error),
    }
}

fn observed_failure(error: &anyhow::Error) -> (FailureDomain, String) {
    error
        .downcast_ref::<FailureAttribution>()
        .map(|attribution| (attribution.domain, attribution.reason_code.to_string()))
        .unwrap_or((
            FailureDomain::Corpus,
            "evaluation-infrastructure-failed".into(),
        ))
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryActivationRecord {
    pub candidate_review: bool,
    pub deterministic_classification: bool,
    pub approval_gated_consolidation: bool,
    pub reflection: bool,
    pub derived_representations: bool,
    pub automatic_retention: bool,
    pub approved_private_gate_required: bool,
    pub private_gate_status: String,
    pub report_digest: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PrivateGateEvidence {
    contract_version: String,
    approved: bool,
    raw_data_location: String,
    automatic_retention_activation_authorized: bool,
    governance: PrivateGateGovernance,
    required_cases: Vec<PrivateGateCase>,
    required_metrics: PrivateGateMetrics,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PrivateGateGovernance {
    reviewer_ids: Vec<String>,
    corpus_revision: String,
    deletion_contact: String,
    secrets_allowed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PrivateGateCase {
    id: String,
    category: String,
    result: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PrivateGateMetrics {
    unauthorized_exposures: usize,
    unsupported_reflection_claims: usize,
    automatic_retentions: usize,
    candidate_precision: Option<f64>,
    candidate_recall: Option<f64>,
    classification_accuracy: Option<f64>,
    duplicate_suppression: Option<f64>,
    contradiction_detection: Option<f64>,
    supersession_correctness: Option<f64>,
    approval_load: Option<f64>,
    retention_accuracy: Option<f64>,
    recall_quality: Option<f64>,
    reflection_grounding: Option<f64>,
    derived_invalidation: Option<f64>,
    latency_p95_ms: Option<u64>,
    cpu_seconds: Option<f64>,
    peak_rss_bytes: Option<u64>,
    provider_requests: Option<usize>,
    estimated_provider_cost_usd: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PrivateGateEvidenceReport {
    pub contract_version: String,
    pub passed: bool,
    pub evidence_digest: String,
    pub case_count: usize,
    pub reviewer_count: usize,
    pub corpus_revision: String,
    pub automatic_retention_activation_authorized: bool,
}

pub fn verify_private_evidence(path: &Path) -> Result<PrivateGateEvidenceReport> {
    let bytes = read_bounded_fixture(path)?;
    let evidence: PrivateGateEvidence =
        serde_json::from_slice(&bytes).context("invalid private memory gate evidence")?;
    anyhow::ensure!(
        evidence.contract_version == "cortana.memory-private-gate.v1",
        "unsupported private memory gate contract"
    );
    anyhow::ensure!(evidence.approved, "private memory gate is not approved");
    anyhow::ensure!(
        evidence.raw_data_location == "external-encrypted-store",
        "private raw data must remain in the external encrypted store"
    );
    anyhow::ensure!(
        !evidence.automatic_retention_activation_authorized,
        "private evidence cannot authorize runtime automatic retention"
    );
    anyhow::ensure!(
        !evidence.governance.secrets_allowed
            && !evidence.governance.reviewer_ids.is_empty()
            && evidence.governance.reviewer_ids.len() <= 16
            && evidence
                .governance
                .reviewer_ids
                .iter()
                .all(|id| bounded_label(id))
            && bounded_label(&evidence.governance.corpus_revision)
            && bounded_label(&evidence.governance.deletion_contact),
        "private memory gate governance is incomplete or unsafe"
    );
    let has_placeholder_reviewer = evidence
        .governance
        .reviewer_ids
        .iter()
        .any(|id| id.contains("opaque-reviewer"));
    let has_placeholder_corpus = evidence
        .governance
        .corpus_revision
        .contains("opaque-approved");
    anyhow::ensure!(
        !(has_placeholder_reviewer || has_placeholder_corpus),
        "private memory gate placeholders must be replaced by approved opaque identifiers"
    );
    const PRIVATE_CASES: &[(&str, &str)] = &[
        ("private-preference", "preference"),
        ("private-decision", "decision"),
        ("private-procedure", "procedure"),
        ("private-repeated-experience", "repeated-experience"),
        ("private-contradiction", "contradiction"),
        ("private-stale-working-state", "stale-working-state"),
        ("private-sensitive-data", "sensitive-data"),
        ("private-cross-workspace-distractor", "cross-workspace"),
    ];
    anyhow::ensure!(
        evidence.required_cases.len() == PRIVATE_CASES.len()
            && evidence
                .required_cases
                .iter()
                .zip(PRIVATE_CASES)
                .all(|(case, expected)| case.id == expected.0
                    && case.category == expected.1
                    && case.result == "passed"),
        "private memory gate requires every canonical private case to pass"
    );
    validate_private_metrics(&evidence.required_metrics)?;
    let evidence_digest = stable_json_digest(&evidence);
    Ok(PrivateGateEvidenceReport {
        contract_version: evidence.contract_version,
        passed: true,
        evidence_digest,
        case_count: evidence.required_cases.len(),
        reviewer_count: evidence.governance.reviewer_ids.len(),
        corpus_revision: evidence.governance.corpus_revision,
        automatic_retention_activation_authorized: false,
    })
}

fn read_bounded_fixture(path: &Path) -> Result<Vec<u8>> {
    let metadata = std::fs::metadata(path).with_context(|| {
        format!(
            "failed to inspect memory evaluation fixture {}",
            path.display()
        )
    })?;
    anyhow::ensure!(
        metadata.is_file(),
        "memory evaluation fixture is not a regular file"
    );
    anyhow::ensure!(
        metadata.len() <= MAX_FIXTURE_BYTES,
        "memory evaluation fixture exceeds its byte safety limit"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::fs::File::open(path)?
        .take(MAX_FIXTURE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_FIXTURE_BYTES,
        "memory evaluation fixture exceeds its byte safety limit"
    );
    Ok(bytes)
}

fn bounded_label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

fn validate_private_metrics(metrics: &PrivateGateMetrics) -> Result<()> {
    let unit = [
        metrics.candidate_precision,
        metrics.candidate_recall,
        metrics.classification_accuracy,
        metrics.duplicate_suppression,
        metrics.contradiction_detection,
        metrics.supersession_correctness,
        metrics.retention_accuracy,
        metrics.recall_quality,
        metrics.reflection_grounding,
        metrics.derived_invalidation,
    ];
    anyhow::ensure!(
        metrics.unauthorized_exposures == 0
            && metrics.unsupported_reflection_claims == 0
            && metrics.automatic_retentions == 0
            && unit.into_iter().all(|value| value == Some(1.0))
            && metrics
                .approval_load
                .is_some_and(|value| value.is_finite() && (0.0..=1.0).contains(&value))
            && metrics.latency_p95_ms.is_some_and(|value| value <= 5_000)
            && metrics.cpu_seconds.is_some_and(
                |value| value.is_finite() && (0.0..=MAX_PRIVATE_CPU_SECONDS).contains(&value)
            )
            && metrics
                .peak_rss_bytes
                .is_some_and(|value| (1..=MAX_PRIVATE_PEAK_RSS_BYTES).contains(&value))
            && metrics.provider_requests.is_some_and(|value| value <= 16)
            && metrics
                .estimated_provider_cost_usd
                .is_some_and(|value| value.is_finite() && (0.0..=1.0).contains(&value)),
        "private memory gate metrics are incomplete or below required thresholds"
    );
    Ok(())
}

pub async fn run_default() -> Result<MemoryEvaluationReport> {
    let fixture: MemoryEvaluationFixture =
        serde_json::from_str(include_str!("../eval/memory-intelligence-fixtures.json"))
            .context("invalid built-in memory evaluation fixture")?;
    run_fixture(fixture).await
}

pub async fn run(path: &Path) -> Result<MemoryEvaluationReport> {
    let bytes = read_bounded_fixture(path)?;
    let fixture = serde_json::from_slice(&bytes).context("invalid memory evaluation fixture")?;
    run_fixture(fixture).await
}

async fn run_fixture(fixture: MemoryEvaluationFixture) -> Result<MemoryEvaluationReport> {
    run_fixture_with_evaluator(fixture, evaluate_case).await
}

async fn run_fixture_with_evaluator<F>(
    fixture: MemoryEvaluationFixture,
    evaluator: F,
) -> Result<MemoryEvaluationReport>
where
    F: Fn(&str) -> Result<CaseMeasurement>,
{
    let _evaluation_guard = EVALUATION_RUN_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("memory evaluation lock poisoned"))?;
    validate_fixture(&fixture)?;
    let fixture_digest = stable_json_digest(&fixture);
    let mut cases = Vec::with_capacity(fixture.cases.len());
    let mut measurements = Vec::with_capacity(fixture.cases.len());
    STORE_CASES.store(0, Ordering::SeqCst);
    MAX_CASE_STORE_BYTES.store(0, Ordering::SeqCst);
    TOTAL_STORE_BYTES.store(0, Ordering::SeqCst);
    for case in &fixture.cases {
        let started = Instant::now();
        let result = evaluator(&case.id);
        let expected_failure_domain = required_case(&case.id)
            .map(|(_, domain)| domain)
            .unwrap_or(case.failure_domain);
        let (passed, failure_domain, reason_code, measurement) = match result {
            Ok(measurement) => (
                true,
                expected_failure_domain,
                "passed".to_string(),
                measurement,
            ),
            Err(error) => {
                let (failure_domain, reason_code) = observed_failure(&error);
                (
                    false,
                    failure_domain,
                    reason_code,
                    CaseMeasurement::default(),
                )
            }
        };
        cases.push(MemoryCaseReport {
            id: case.id.clone(),
            category: case.category.clone(),
            failure_domain,
            passed,
            latency_ms: elapsed_ms(started),
            reason_code,
        });
        measurements.push(measurement);
    }

    let pass = |id: &str| cases.iter().any(|case| case.id == id && case.passed);
    let ratio =
        |ids: &[&str]| ids.iter().filter(|id| pass(id)).count() as f64 / ids.len().max(1) as f64;
    let classification_ids = [
        "classification-exact-duplicate",
        "classification-contradiction",
        "classification-supersession",
        "classification-provider-failure",
    ];
    let lifecycle_ids = [
        "explicit-idempotent-recall",
        "expiry-excludes-stale-working",
        "redaction-removes-recall",
        "backup-restore-preserves-memory",
        "consolidation-idempotency",
        "policy-change-rechecks-candidate",
    ];
    let reflection_ids = ["reflection-grounding", "reflection-provider-failure"];
    let unauthorized_exposures = usize::from(!pass("acl-cross-workspace-denial"));
    let unsupported_reflection_claims = reflection_ids.iter().filter(|id| !pass(id)).count();
    let automatic_retentions_without_private_gate =
        usize::from(!pass("automatic-retention-remains-disabled"));
    let comparison_outcomes = [
        "candidate-review",
        "classification",
        "consolidation",
        "reflection",
        "derived-representations",
    ]
    .into_iter()
    .map(evaluate_feature_comparison)
    .collect::<Vec<_>>();
    measurements.extend(
        comparison_outcomes
            .iter()
            .map(|outcome| outcome.measurement.clone()),
    );
    let max_operation_latency_ms = cases
        .iter()
        .map(|case| case.latency_ms)
        .chain(comparison_outcomes.iter().map(|outcome| outcome.latency_ms))
        .max()
        .unwrap_or(0);
    let total_operation_latency_ms = cases
        .iter()
        .map(|case| case.latency_ms)
        .chain(comparison_outcomes.iter().map(|outcome| outcome.latency_ms))
        .sum();
    let expected_candidates = measurements
        .iter()
        .map(|measurement| measurement.expected_candidates)
        .sum::<usize>();
    let proposed_candidates = measurements
        .iter()
        .map(|measurement| measurement.proposed_candidates)
        .sum::<usize>();
    let true_positive_candidates = measurements
        .iter()
        .map(|measurement| measurement.true_positive_candidates)
        .sum::<usize>();
    let candidate_precision = fraction(true_positive_candidates, proposed_candidates);
    let candidate_recall = fraction(true_positive_candidates, expected_candidates);
    let approval_eligible = measurements
        .iter()
        .map(|measurement| measurement.approval_eligible)
        .sum::<usize>();
    let approval_required = measurements
        .iter()
        .map(|measurement| measurement.approval_required)
        .sum::<usize>();
    let mut failures_by_domain = MemoryFailureDomainCounts::default();
    for case in cases.iter().filter(|case| !case.passed) {
        failures_by_domain.increment(case.failure_domain);
    }
    for outcome in comparison_outcomes
        .iter()
        .filter(|outcome| !outcome.comparison.passed)
    {
        if let Some(domain) = outcome.comparison.failure_domain {
            failures_by_domain.increment(domain);
        }
    }
    let metrics = MemoryEvaluationMetrics {
        candidate_precision,
        candidate_recall,
        classification_accuracy: ratio(&classification_ids),
        duplicate_suppression: ratio(&[
            "classification-exact-duplicate",
            "consolidation-idempotency",
        ]),
        contradiction_detection: ratio(&["classification-contradiction"]),
        supersession_correctness: ratio(&["classification-supersession"]),
        lifecycle_pass_rate: ratio(&lifecycle_ids),
        retention_accuracy: ratio(&["expiry-excludes-stale-working", "redaction-removes-recall"]),
        recall_quality: ratio(&["explicit-idempotent-recall", "acl-cross-workspace-denial"]),
        reflection_grounding: ratio(&reflection_ids),
        derived_invalidation: ratio(&["derived-representation-invalidation"]),
        unauthorized_exposures,
        unsupported_reflection_claims,
        automatic_retentions_without_private_gate,
        approval_load: fraction(approval_required, approval_eligible),
        max_case_latency_ms: max_operation_latency_ms,
        total_case_latency_ms: total_operation_latency_ms,
        provider_requests: measurements
            .iter()
            .map(|measurement| measurement.provider_requests)
            .sum(),
        estimated_provider_cost_usd: measurements
            .iter()
            .map(|measurement| measurement.estimated_provider_cost_usd)
            .sum(),
        disposable_store_cases: STORE_CASES.load(Ordering::SeqCst),
        max_case_store_bytes: MAX_CASE_STORE_BYTES.load(Ordering::SeqCst),
        total_store_bytes: TOTAL_STORE_BYTES.load(Ordering::SeqCst),
        failures_by_domain,
    };
    let baseline = MemoryBaselineComparison {
        explicit_memory_passed: pass("explicit-idempotent-recall"),
        intelligence_disabled_preserves_explicit_memory: pass(
            "automatic-retention-remains-disabled",
        ),
        canonical_revision_stable_during_reflection: pass("reflection-grounding"),
    };
    let candidate_review_passed = pass("candidate-safety-boundaries")
        && metrics.candidate_precision >= fixture.thresholds.min_candidate_precision
        && metrics.candidate_recall >= fixture.thresholds.min_candidate_recall;
    let classification_passed = metrics.classification_accuracy == 1.0;
    let consolidation_passed = pass("consolidation-idempotency")
        && pass("policy-change-rechecks-candidate")
        && pass("automatic-retention-remains-disabled");
    let reflection_passed = metrics.reflection_grounding == 1.0;
    let derived_passed = metrics.derived_invalidation == 1.0;
    let mut comparisons = comparison_outcomes
        .into_iter()
        .map(|outcome| outcome.comparison)
        .collect::<Vec<_>>();
    let passed = cases.iter().all(|case| case.passed)
        && comparisons.iter().all(|comparison| {
            comparison.passed
                && comparison.explicit_memory_preserved
                && comparison.canonical_write_requires_approval
        })
        && metrics.candidate_precision >= fixture.thresholds.min_candidate_precision
        && metrics.candidate_recall >= fixture.thresholds.min_candidate_recall
        && metrics.classification_accuracy >= fixture.thresholds.min_classification_accuracy
        && metrics.lifecycle_pass_rate >= fixture.thresholds.min_lifecycle_pass_rate
        && metrics.reflection_grounding >= fixture.thresholds.min_reflection_grounding
        && metrics.duplicate_suppression >= fixture.thresholds.min_duplicate_suppression
        && metrics.contradiction_detection >= fixture.thresholds.min_contradiction_detection
        && metrics.supersession_correctness >= fixture.thresholds.min_supersession_correctness
        && metrics.retention_accuracy >= fixture.thresholds.min_retention_accuracy
        && metrics.recall_quality >= fixture.thresholds.min_recall_quality
        && metrics.derived_invalidation >= fixture.thresholds.min_derived_invalidation
        && metrics.unauthorized_exposures <= fixture.thresholds.max_unauthorized_exposures
        && metrics.unsupported_reflection_claims
            <= fixture.thresholds.max_unsupported_reflection_claims
        && metrics.automatic_retentions_without_private_gate
            <= fixture
                .thresholds
                .max_automatic_retention_without_private_gate
        && metrics.approval_load <= fixture.thresholds.max_approval_load
        && metrics.provider_requests <= fixture.thresholds.max_provider_requests
        && metrics.estimated_provider_cost_usd
            <= fixture.thresholds.max_estimated_provider_cost_usd
        && metrics.disposable_store_cases <= fixture.thresholds.max_disposable_store_cases
        && metrics.max_case_store_bytes <= fixture.thresholds.max_case_store_bytes
        && metrics.total_store_bytes <= fixture.thresholds.max_total_store_bytes
        && metrics.max_case_latency_ms <= fixture.thresholds.max_case_latency_ms
        && metrics.total_case_latency_ms <= fixture.thresholds.max_total_case_latency_ms;
    // Private-corpus evidence is governed externally and cannot be asserted by
    // relabeling a synthetic executable fixture.
    let private_gate_passed = false;
    comparisons.push(feature_comparison(
        "automatic-retention",
        private_gate_passed,
        baseline.intelligence_disabled_preserves_explicit_memory,
        true,
        None,
        "private-gate-not-run",
    ));
    let capability_activated = |capability: &str| {
        comparisons
            .iter()
            .find(|comparison| comparison.capability == capability)
            .is_some_and(|comparison| {
                comparison.passed
                    && comparison.explicit_memory_preserved
                    && comparison.canonical_write_requires_approval
            })
    };
    let mut activation = MemoryActivationRecord {
        candidate_review: candidate_review_passed && capability_activated("candidate-review"),
        deterministic_classification: classification_passed
            && capability_activated("classification"),
        approval_gated_consolidation: consolidation_passed && capability_activated("consolidation"),
        reflection: reflection_passed && capability_activated("reflection"),
        derived_representations: derived_passed && capability_activated("derived-representations"),
        // A fixture report is evidence, not runtime authorization. Enabling
        // automatic retention remains a separate reviewed release action.
        automatic_retention: false,
        approved_private_gate_required: true,
        private_gate_status: if private_gate_passed {
            "passed-evidence-only".into()
        } else {
            "not-run".into()
        },
        report_digest: String::new(),
    };
    let deterministic_cases = cases
        .iter()
        .map(|case| {
            serde_json::json!({
                "id": case.id,
                "passed": case.passed,
                "failure_domain": case.failure_domain,
                "reason_code": case.reason_code,
            })
        })
        .collect::<Vec<_>>();
    activation.report_digest = stable_json_digest(&serde_json::json!({
        "contract": MEMORY_EVALUATION_CONTRACT_VERSION,
        "fixture": fixture_digest,
        "passed": passed,
        "quality": {
            "candidate_precision": metrics.candidate_precision,
            "candidate_recall": metrics.candidate_recall,
            "classification_accuracy": metrics.classification_accuracy,
            "duplicate_suppression": metrics.duplicate_suppression,
            "contradiction_detection": metrics.contradiction_detection,
            "supersession_correctness": metrics.supersession_correctness,
            "retention_accuracy": metrics.retention_accuracy,
            "recall_quality": metrics.recall_quality,
            "reflection_grounding": metrics.reflection_grounding,
            "derived_invalidation": metrics.derived_invalidation,
            "unauthorized_exposures": metrics.unauthorized_exposures,
            "unsupported_reflection_claims": metrics.unsupported_reflection_claims,
            "automatic_retentions_without_private_gate": metrics.automatic_retentions_without_private_gate,
        },
        "cases": deterministic_cases,
        "baseline": &baseline,
        "comparisons": &comparisons,
    }));
    Ok(MemoryEvaluationReport {
        contract_version: MEMORY_EVALUATION_CONTRACT_VERSION.into(),
        fixture_version: fixture.version,
        fixture_class: fixture.fixture_class,
        fixture_digest,
        passed,
        thresholds: fixture.thresholds,
        metrics,
        cases,
        baseline,
        comparisons,
        activation,
    })
}

fn feature_comparison(
    capability: &str,
    passed: bool,
    explicit_memory_preserved: bool,
    canonical_write_requires_approval: bool,
    failure_domain: Option<FailureDomain>,
    reason_code: &str,
) -> MemoryFeatureComparison {
    MemoryFeatureComparison {
        capability: capability.into(),
        passed,
        explicit_memory_preserved,
        canonical_write_requires_approval,
        failure_domain,
        reason_code: reason_code.into(),
    }
}

fn comparison_failure(capability: &str) -> (FailureDomain, &'static str) {
    match capability {
        "candidate-review" => (FailureDomain::Candidate, "candidate-comparison-failed"),
        "classification" => (FailureDomain::Candidate, "classification-comparison-failed"),
        "consolidation" => (
            FailureDomain::Policy,
            "consolidation-policy-comparison-failed",
        ),
        "reflection" => (FailureDomain::Model, "reflection-comparison-failed"),
        "derived-representations" => (FailureDomain::Corpus, "derived-comparison-failed"),
        _ => (FailureDomain::Corpus, "unknown-comparison-failed"),
    }
}

fn evaluate_feature_comparison(capability: &str) -> FeatureComparisonOutcome {
    let started = Instant::now();
    let result = (|| -> Result<(bool, bool, bool, CaseMeasurement)> {
        let case = CaseStore::new()?;
        let mut measurement = CaseMeasurement::default();
        let explicit = case.store.remember(&memory_input(
            "work",
            &format!("Explicit baseline for {capability}"),
            Some(&format!("eval:baseline:{capability}")),
            None,
        ))?;
        let revision = case.store.memory_revision()?;
        let operation_passed = match capability {
            "candidate-review" => {
                case.store.propose_memory_candidate(
                    &candidate_input("work", "Candidate comparison", "normal", None),
                    "agent-a",
                    &["work".into()],
                    false,
                )?;
                measurement.approval_eligible = 1;
                measurement.approval_required = 1;
                true
            }
            "classification" => {
                let candidate = case.store.propose_memory_candidate(
                    &candidate_input("work", "Classification comparison", "normal", None),
                    "agent-a",
                    &["work".into()],
                    false,
                )?;
                case.store.classify_memory_candidate(
                    &candidate.id,
                    "agent-a",
                    &["work".into()],
                    false,
                )?;
                measurement.approval_eligible = 1;
                measurement.approval_required = 1;
                true
            }
            "consolidation" => {
                let candidate = case.store.propose_memory_candidate(
                    &candidate_input("work", "Consolidation comparison", "normal", None),
                    "agent-a",
                    &["work".into()],
                    false,
                )?;
                let outcome = case.store.consolidate_memory_candidate(
                    &candidate.id,
                    &ConsolidationPolicy::default(),
                    "agent-a",
                    &["work".into()],
                    false,
                    false,
                )?;
                let review_required = outcome.status == "review" && outcome.memory_id.is_none();
                measurement.approval_eligible = 1;
                measurement.approval_required = usize::from(review_required);
                review_required
            }
            "reflection" => {
                let memories =
                    case.store
                        .export_memories(Some("work"), None, 20, &["work".into()])?;
                let response = reflect_with_provider(
                    &reflect_request(ProviderPolicy::DeterministicOnly),
                    &ReflectionInputs {
                        memories: &memories,
                        evidence: &[],
                        evidence_project: None,
                        principal_acl: &["work".into()],
                        owner: false,
                        memory_revision: revision,
                    },
                    None,
                )?;
                let proposed = response.proposed_candidates.len();
                let approval_required = response
                    .proposed_candidates
                    .iter()
                    .filter(|candidate| candidate.approval_required)
                    .count();
                measurement.approval_eligible = proposed;
                measurement.approval_required = approval_required;
                approval_required == proposed
            }
            "derived-representations" => {
                let derived = derive_authorized_memory(
                    &case.store,
                    Some("work"),
                    64,
                    &["work".into()],
                    false,
                )?;
                !derived.canonical_memory_mutated
            }
            _ => false,
        };
        let explicit_preserved = case.store.memory(&explicit.id)?.is_some();
        let unapproved_write_blocked = case.store.memory_revision()? == revision;
        Ok((
            operation_passed,
            explicit_preserved,
            unapproved_write_blocked,
            measurement,
        ))
    })();
    let (comparison_domain, comparison_reason) = comparison_failure(capability);
    let (comparison, measurement) = match result {
        Ok((operation_passed, explicit_preserved, approval_required, measurement)) => {
            let (failure_domain, reason_code) = if !explicit_preserved {
                (Some(FailureDomain::Corpus), "explicit-memory-not-preserved")
            } else if !approval_required {
                (Some(FailureDomain::Policy), "canonical-write-policy-failed")
            } else if !operation_passed {
                (Some(comparison_domain), comparison_reason)
            } else {
                (None, "passed")
            };
            (
                feature_comparison(
                    capability,
                    operation_passed && explicit_preserved && approval_required,
                    explicit_preserved,
                    approval_required,
                    failure_domain,
                    reason_code,
                ),
                measurement,
            )
        }
        Err(error) => {
            let (failure_domain, reason_code) = observed_failure(&error);
            (
                feature_comparison(
                    capability,
                    false,
                    false,
                    false,
                    Some(failure_domain),
                    &reason_code,
                ),
                CaseMeasurement::default(),
            )
        }
    };
    FeatureComparisonOutcome {
        comparison,
        measurement,
        latency_ms: elapsed_ms(started),
    }
}

fn fraction(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn required_case(id: &str) -> Option<(&'static str, FailureDomain)> {
    REQUIRED_CASES
        .iter()
        .find(|(required_id, _, _)| *required_id == id)
        .map(|(_, category, domain)| (*category, *domain))
}

fn validate_fixture(fixture: &MemoryEvaluationFixture) -> Result<()> {
    anyhow::ensure!(
        fixture.version == 1,
        "unsupported memory evaluation fixture version"
    );
    anyhow::ensure!(
        fixture.fixture_class == "synthetic",
        "executable memory evaluation fixtures must be synthetic; approved-private evidence uses the external governance record"
    );
    anyhow::ensure!(
        fixture.cases.len() == REQUIRED_CASES.len() && fixture.cases.len() <= MAX_CASES,
        "gating memory evaluation fixtures must contain the complete canonical case set"
    );
    for (case, (required_id, required_category, required_domain)) in
        fixture.cases.iter().zip(REQUIRED_CASES)
    {
        anyhow::ensure!(
            case.id == *required_id
                && case.category == *required_category
                && case.failure_domain == *required_domain,
            "gating memory evaluation case metadata must match the canonical registry"
        );
    }
    for value in [
        fixture.thresholds.min_candidate_precision,
        fixture.thresholds.min_candidate_recall,
        fixture.thresholds.min_classification_accuracy,
        fixture.thresholds.min_lifecycle_pass_rate,
        fixture.thresholds.min_reflection_grounding,
        fixture.thresholds.min_duplicate_suppression,
        fixture.thresholds.min_contradiction_detection,
        fixture.thresholds.min_supersession_correctness,
        fixture.thresholds.min_retention_accuracy,
        fixture.thresholds.min_recall_quality,
        fixture.thresholds.min_derived_invalidation,
        fixture.thresholds.max_approval_load,
    ] {
        anyhow::ensure!(
            value.is_finite() && (0.0..=1.0).contains(&value),
            "invalid threshold"
        );
    }
    anyhow::ensure!(
        (1..=30_000).contains(&fixture.thresholds.max_case_latency_ms),
        "memory evaluation latency bound is invalid"
    );
    anyhow::ensure!(
        (1..=120_000).contains(&fixture.thresholds.max_total_case_latency_ms),
        "memory evaluation total latency bound is invalid"
    );
    anyhow::ensure!(
        fixture
            .thresholds
            .max_estimated_provider_cost_usd
            .is_finite()
            && fixture.thresholds.max_estimated_provider_cost_usd >= 0.0,
        "memory evaluation cost bound is invalid"
    );
    anyhow::ensure!(
        fixture.thresholds.max_unauthorized_exposures == 0
            && fixture.thresholds.max_unsupported_reflection_claims == 0
            && fixture
                .thresholds
                .max_automatic_retention_without_private_gate
                == 0,
        "memory evaluation safety maxima cannot be weakened"
    );
    anyhow::ensure!(
        fixture.thresholds.min_candidate_precision == 1.0
            && fixture.thresholds.min_candidate_recall == 1.0
            && fixture.thresholds.min_classification_accuracy == 1.0
            && fixture.thresholds.min_lifecycle_pass_rate == 1.0
            && fixture.thresholds.min_reflection_grounding == 1.0
            && fixture.thresholds.min_duplicate_suppression == 1.0
            && fixture.thresholds.min_contradiction_detection == 1.0
            && fixture.thresholds.min_supersession_correctness == 1.0
            && fixture.thresholds.min_retention_accuracy == 1.0
            && fixture.thresholds.min_recall_quality == 1.0
            && fixture.thresholds.min_derived_invalidation == 1.0
            && fixture.thresholds.max_approval_load <= 1.0
            && fixture.thresholds.max_provider_requests <= 2
            && fixture.thresholds.max_estimated_provider_cost_usd == 0.0
            && fixture.thresholds.max_disposable_store_cases <= 20
            && fixture.thresholds.max_case_store_bytes <= 16 * 1024 * 1024
            && fixture.thresholds.max_total_store_bytes <= 256 * 1024 * 1024
            && fixture.thresholds.max_case_latency_ms <= 5_000
            && fixture.thresholds.max_total_case_latency_ms <= 30_000,
        "memory evaluation gating thresholds may tighten but cannot weaken the canonical release gate"
    );
    Ok(())
}

fn evaluate_case(id: &str) -> Result<CaseMeasurement> {
    match id {
        "explicit-idempotent-recall" => measured(explicit_idempotent_recall),
        "expiry-excludes-stale-working" => measured(expiry_excludes_stale_working),
        "redaction-removes-recall" => measured(redaction_removes_recall),
        "acl-cross-workspace-denial" => measured(acl_cross_workspace_denial),
        "candidate-safety-boundaries" => candidate_safety_boundaries(),
        "classification-exact-duplicate" => {
            classification_case("Remembered release incident", "exact-duplicate", "episodic")
        }
        "classification-contradiction" => {
            classification_case("Do not deploy on Friday", "contradiction", "semantic")
        }
        "classification-supersession" => classification_case(
            "Changed: do not deploy on Friday; deploy on Monday instead",
            "supersession",
            "semantic",
        ),
        "classification-provider-failure" => classification_provider_failure(),
        "automatic-retention-remains-disabled" => automatic_retention_remains_disabled(),
        "consolidation-idempotency" => measured(consolidation_idempotency),
        "policy-change-rechecks-candidate" => measured(policy_change_rechecks_candidate),
        "backup-restore-preserves-memory" => measured(backup_restore_preserves_memory),
        "reflection-grounding" => measured(reflection_grounding),
        "reflection-provider-failure" => reflection_provider_failure(),
        "derived-representation-invalidation" => measured(derived_representation_invalidation),
        _ => anyhow::bail!("unknown memory evaluation case"),
    }
}

fn measured(case: impl FnOnce() -> Result<()>) -> Result<CaseMeasurement> {
    case()?;
    Ok(CaseMeasurement::default())
}

fn explicit_idempotent_recall() -> Result<()> {
    let case = CaseStore::new()?;
    let input = memory_input("work", "Deploy on Friday", Some("eval:deploy"), None);
    let first = case.store.remember(&input)?;
    let revision = case.store.memory_revision()?;
    let retry = case.store.remember(&input)?;
    let recalled =
        case.store
            .recall_memories("deploy friday", Some("work"), None, 10, &["work".into()])?;
    failure_check(
        FailureDomain::Corpus,
        "explicit-idempotency-failed",
        first.id == retry.id && revision == case.store.memory_revision()?,
    )?;
    failure_check(
        FailureDomain::Corpus,
        "explicit-recall-failed",
        recalled.iter().any(|item| item.memory.id == first.id),
    )?;
    Ok(())
}

fn expiry_excludes_stale_working() -> Result<()> {
    let case = CaseStore::new()?;
    let mut input = memory_input("work", "Temporary release state", None, None);
    input.kind = "working".into();
    input.valid_until = Some((Utc::now() - Duration::minutes(1)).to_rfc3339());
    case.store.remember(&input)?;
    let recalled = case.store.recall_memories(
        "temporary release",
        Some("work"),
        None,
        10,
        &["work".into()],
    )?;
    failure_check(
        FailureDomain::Policy,
        "expiry-policy-failed",
        recalled.is_empty(),
    )?;
    Ok(())
}

fn redaction_removes_recall() -> Result<()> {
    let case = CaseStore::new()?;
    let memory = case
        .store
        .remember(&memory_input("work", "Redact this memory", None, None))?;
    failure_check(
        FailureDomain::Policy,
        "redaction-policy-failed",
        case.store.forget_memory(&memory.id)?,
    )?;
    let recalled =
        case.store
            .recall_memories("redact memory", Some("work"), None, 10, &["work".into()])?;
    failure_check(
        FailureDomain::Policy,
        "redacted-memory-recalled",
        recalled.is_empty(),
    )?;
    Ok(())
}

fn acl_cross_workspace_denial() -> Result<()> {
    let case = CaseStore::new()?;
    let memory = case
        .store
        .remember(&memory_input("work", "Private work decision", None, None))?;
    let recalled =
        case.store
            .recall_memories("private decision", None, None, 10, &["personal".into()])?;
    failure_check(
        FailureDomain::Policy,
        "acl-recall-policy-failed",
        recalled.is_empty(),
    )?;
    let mut replacement = memory_input("work", "Changed private decision", None, None);
    replacement.supersedes_id = Some(memory.id);
    let mutation_rejected = expected_policy_rejection(
        case.store
            .remember_scoped(&replacement, &["personal".into()], false),
        "memory supersession target is outside principal visibility",
    )?;
    failure_check(
        FailureDomain::Policy,
        "acl-mutation-policy-failed",
        mutation_rejected,
    )?;
    Ok(())
}

fn candidate_safety_boundaries() -> Result<CaseMeasurement> {
    let case = CaseStore::new()?;
    let revision = case.store.memory_revision()?;
    case.store.propose_memory_candidate(
        &candidate_input("work", "A safe approved preference", "normal", None),
        "agent-a",
        &["work".into()],
        false,
    )?;
    let safe_accepted = true;
    let sensitive = candidate_input("work", "Secret token material", "sensitive", None);
    let sensitive_rejected = expected_policy_rejection(
        case.store
            .propose_memory_candidate(&sensitive, "agent-a", &["work".into()], false),
        "candidate rejected: sensitive observations require explicit review and are not accepted by the bounded capture path",
    )?;
    let cross = candidate_input("personal", "Cross workspace distractor", "normal", None);
    let cross_rejected = expected_policy_rejection(
        case.store
            .propose_memory_candidate(&cross, "agent-a", &["work".into()], false),
        "candidate ACL denied",
    )?;
    let sensitive_accepted = !sensitive_rejected;
    let cross_accepted = !cross_rejected;
    let measurement = CaseMeasurement {
        expected_candidates: 1,
        proposed_candidates: usize::from(safe_accepted)
            + usize::from(sensitive_accepted)
            + usize::from(cross_accepted),
        true_positive_candidates: usize::from(safe_accepted),
        approval_eligible: usize::from(safe_accepted),
        approval_required: usize::from(safe_accepted),
        ..Default::default()
    };
    failure_check(
        FailureDomain::Candidate,
        "safe-candidate-rejected",
        safe_accepted,
    )?;
    failure_check(
        FailureDomain::Policy,
        "candidate-policy-boundary-failed",
        !sensitive_accepted && !cross_accepted,
    )?;
    failure_check(
        FailureDomain::Candidate,
        "candidate-canonical-write-failed",
        case.store.memory_revision()? == revision,
    )?;
    Ok(measurement)
}

fn classification_case(
    candidate_text: &str,
    expected: &str,
    content_type: &str,
) -> Result<CaseMeasurement> {
    let case = CaseStore::new()?;
    let baseline = if content_type == "episodic" {
        "Remembered release incident"
    } else {
        "Deploy on Friday"
    };
    case.store.remember(&memory_input_type(
        "work",
        baseline,
        content_type,
        None,
        None,
    ))?;
    let candidate = case.store.propose_memory_candidate(
        &candidate_input_type("work", candidate_text, content_type, "normal", None),
        "agent-a",
        &["work".into()],
        false,
    )?;
    let revision = case.store.memory_revision()?;
    let classification =
        case.store
            .classify_memory_candidate(&candidate.id, "agent-a", &["work".into()], false)?;
    failure_check(
        FailureDomain::Candidate,
        "classification-result-mismatch",
        classification.classification == expected,
    )?;
    failure_check(
        FailureDomain::Candidate,
        "classification-canonical-write-failed",
        case.store.memory_revision()? == revision,
    )?;
    Ok(CaseMeasurement {
        approval_eligible: 1,
        approval_required: 1,
        ..Default::default()
    })
}

struct FailingClassificationProvider {
    calls: Arc<AtomicUsize>,
}

impl ClassificationProvider for FailingClassificationProvider {
    fn classify(
        &self,
        _candidate: &ObservationCandidate,
        _memories: &[MemoryRecord],
    ) -> std::result::Result<CandidateClassification, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err("private provider detail".into())
    }
}

fn classification_provider_failure() -> Result<CaseMeasurement> {
    let candidate = standalone_candidate("New bounded observation");
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = FailingClassificationProvider {
        calls: Arc::clone(&calls),
    };
    let result = classify_with_optional_provider(&candidate, &[], Some(&provider));
    failure_check(
        FailureDomain::Model,
        "classification-provider-fallback-failed",
        result.proposed_action == "retain-for-review",
    )?;
    failure_check(
        FailureDomain::Model,
        "classification-provider-detail-leaked",
        result
            .unresolved_ambiguity
            .as_deref()
            .is_some_and(|value| !value.contains("private provider detail")),
    )?;
    Ok(CaseMeasurement {
        provider_requests: calls.load(Ordering::SeqCst),
        estimated_provider_cost_usd: 0.0,
        ..Default::default()
    })
}

fn automatic_retention_remains_disabled() -> Result<CaseMeasurement> {
    let case = CaseStore::new()?;
    let explicit = case.store.remember(&memory_input(
        "work",
        "Explicit baseline survives",
        None,
        None,
    ))?;
    let candidate = case.store.propose_memory_candidate(
        &candidate_input("work", "High quality automatic proposal", "normal", None),
        "agent-a",
        &["work".into()],
        false,
    )?;
    let outcome = case.store.consolidate_memory_candidate(
        &candidate.id,
        &ConsolidationPolicy::default(),
        "agent-a",
        &["work".into()],
        false,
        false,
    )?;
    failure_check(
        FailureDomain::Policy,
        "automatic-retention-policy-failed",
        outcome.status == "review" && outcome.memory_id.is_none(),
    )?;
    failure_check(
        FailureDomain::Corpus,
        "explicit-memory-not-preserved",
        case.store.memory(&explicit.id)?.is_some(),
    )?;
    Ok(CaseMeasurement {
        approval_eligible: 1,
        approval_required: usize::from(outcome.status == "review"),
        ..Default::default()
    })
}

fn consolidation_idempotency() -> Result<()> {
    let case = CaseStore::new()?;
    let candidate = case.store.propose_memory_candidate(
        &candidate_input_type(
            "work",
            "Approved durable procedure",
            "procedural",
            "normal",
            Some("eval:approved"),
        ),
        "agent-a",
        &["work".into()],
        false,
    )?;
    let mut policy = ConsolidationPolicy {
        enabled: true,
        ..Default::default()
    };
    policy.preferences.allow_auto_retain = false;
    let first = case.store.consolidate_memory_candidate(
        &candidate.id,
        &policy,
        "agent-a",
        &["work".into()],
        false,
        true,
    )?;
    let retry = case.store.consolidate_memory_candidate(
        &candidate.id,
        &policy,
        "agent-a",
        &["work".into()],
        false,
        true,
    )?;
    failure_check(
        FailureDomain::Policy,
        "consolidation-idempotency-failed",
        first.memory_id.is_some() && first.memory_id == retry.memory_id,
    )?;
    Ok(())
}

fn policy_change_rechecks_candidate() -> Result<()> {
    let case = CaseStore::new()?;
    let candidate = case.store.propose_memory_candidate(
        &candidate_input("work", "Policy change candidate", "normal", None),
        "agent-a",
        &["work".into()],
        false,
    )?;
    let disabled = case.store.consolidate_memory_candidate(
        &candidate.id,
        &ConsolidationPolicy::default(),
        "agent-a",
        &["work".into()],
        false,
        false,
    )?;
    let mut enabled = ConsolidationPolicy {
        enabled: true,
        ..Default::default()
    };
    enabled.preferences.allow_auto_retain = false;
    let approved = case.store.consolidate_memory_candidate(
        &candidate.id,
        &enabled,
        "agent-a",
        &["work".into()],
        false,
        true,
    )?;
    failure_check(
        FailureDomain::Policy,
        "policy-recheck-failed",
        disabled.status == "review" && approved.memory_id.is_some(),
    )?;
    Ok(())
}

fn backup_restore_preserves_memory() -> Result<()> {
    let case = CaseStore::new()?;
    let memory = case
        .store
        .remember(&memory_input("work", "Backed up memory", None, None))?;
    let backup = case.root.join("memory-backup.sqlite3");
    case.store.backup(&backup)?;
    Store::verify(&backup)?;
    let restored = Store::open(&backup)?;
    failure_check(
        FailureDomain::Corpus,
        "backup-restore-failed",
        restored.memory(&memory.id)?.is_some(),
    )?;
    Ok(())
}

fn reflection_grounding() -> Result<()> {
    let case = CaseStore::new()?;
    case.store.remember(&memory_input(
        "work",
        "Use bounded release checks",
        None,
        None,
    ))?;
    let revision = case.store.memory_revision()?;
    let memories = case
        .store
        .export_memories(Some("work"), None, 20, &["work".into()])?;
    let ids = memories
        .iter()
        .map(|memory| memory.id.as_str())
        .collect::<HashSet<_>>();
    let request = reflect_request(ProviderPolicy::DeterministicOnly);
    let response = reflect_with_provider(
        &request,
        &ReflectionInputs {
            memories: &memories,
            evidence: &[],
            evidence_project: None,
            principal_acl: &["work".into()],
            owner: false,
            memory_revision: revision,
        },
        None,
    )?;
    failure_check(
        FailureDomain::Model,
        "reflection-canonical-write-failed",
        case.store.memory_revision()? == revision,
    )?;
    failure_check(
        FailureDomain::Model,
        "reflection-grounding-failed",
        response.claims.iter().all(|claim| {
            !claim.supporting_memory_ids.is_empty()
                && claim
                    .supporting_memory_ids
                    .iter()
                    .all(|id| ids.contains(id.as_str()))
        }),
    )?;
    failure_check(
        FailureDomain::Policy,
        "reflection-approval-policy-failed",
        response
            .proposed_candidates
            .iter()
            .all(|candidate| candidate.approval_required),
    )?;
    Ok(())
}

struct FailingReflectionProvider {
    calls: Arc<AtomicUsize>,
}

impl ReflectionProvider for FailingReflectionProvider {
    fn name(&self) -> &str {
        "failing-evaluation-provider"
    }

    fn reflect(
        &self,
        _request: &ReflectRequest,
        _memories: &[MemoryRecord],
        _evidence: &[crate::model::Evidence],
    ) -> std::result::Result<ProviderReflection, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err("private provider detail".into())
    }
}

fn reflection_provider_failure() -> Result<CaseMeasurement> {
    let case = CaseStore::new()?;
    case.store.remember(&memory_input(
        "work",
        "Provider fallback remains grounded",
        None,
        None,
    ))?;
    let revision = case.store.memory_revision()?;
    let memories = case
        .store
        .export_memories(Some("work"), None, 20, &["work".into()])?;
    let calls = Arc::new(AtomicUsize::new(0));
    let response = reflect_with_provider(
        &reflect_request(ProviderPolicy::PreferProvider),
        &ReflectionInputs {
            memories: &memories,
            evidence: &[],
            evidence_project: None,
            principal_acl: &["work".into()],
            owner: false,
            memory_revision: revision,
        },
        Some(Arc::new(FailingReflectionProvider {
            calls: Arc::clone(&calls),
        })),
    )?;
    failure_check(
        FailureDomain::Model,
        "reflection-provider-fallback-failed",
        response.provider.status == "fallback",
    )?;
    failure_check(
        FailureDomain::Model,
        "reflection-provider-detail-leaked",
        response
            .provider
            .detail
            .as_deref()
            .is_some_and(|detail| !detail.contains("private provider detail")),
    )?;
    failure_check(
        FailureDomain::Model,
        "reflection-canonical-write-failed",
        case.store.memory_revision()? == revision,
    )?;
    Ok(CaseMeasurement {
        provider_requests: calls.load(Ordering::SeqCst),
        estimated_provider_cost_usd: 0.0,
        ..Default::default()
    })
}

fn derived_representation_invalidation() -> Result<()> {
    let case = CaseStore::new()?;
    case.store
        .remember(&memory_input("work", "First derived support", None, None))?;
    let first = derive_authorized_memory(&case.store, Some("work"), 64, &["work".into()], false)?;
    case.store
        .remember(&memory_input("work", "Second derived support", None, None))?;
    let second = derive_authorized_memory(&case.store, Some("work"), 64, &["work".into()], false)?;
    failure_check(
        FailureDomain::Corpus,
        "derived-revision-invalidation-failed",
        second.memory_revision > first.memory_revision,
    )?;
    failure_check(
        FailureDomain::Policy,
        "derived-canonical-write-failed",
        !first.canonical_memory_mutated && !second.canonical_memory_mutated,
    )?;
    failure_check(
        FailureDomain::Corpus,
        "derived-recomputation-failed",
        second.recomputed,
    )?;
    Ok(())
}

fn memory_input(
    project: &str,
    content: &str,
    dedupe_key: Option<&str>,
    valid_until: Option<String>,
) -> MemoryInput {
    memory_input_type(project, content, "preference", dedupe_key, valid_until)
}

fn memory_input_type(
    project: &str,
    content: &str,
    content_type: &str,
    dedupe_key: Option<&str>,
    valid_until: Option<String>,
) -> MemoryInput {
    MemoryInput {
        kind: content_type.into(),
        project: project.into(),
        title: content.into(),
        content: content.into(),
        source: "memory-evaluation".into(),
        source_id: stable_json_digest(&serde_json::json!([project, content])),
        dedupe_key: dedupe_key.map(str::to_string),
        confidence: 0.95,
        importance: 0.9,
        acl: vec![project.into()],
        provenance: serde_json::json!({"fixture":"synthetic"}),
        supersedes_id: None,
        valid_until,
    }
}

fn candidate_input(
    project: &str,
    content: &str,
    sensitivity: &str,
    dedupe_key: Option<&str>,
) -> ObservationCandidateInput {
    candidate_input_type(project, content, "preference", sensitivity, dedupe_key)
}

fn candidate_input_type(
    project: &str,
    content: &str,
    content_type: &str,
    sensitivity: &str,
    dedupe_key: Option<&str>,
) -> ObservationCandidateInput {
    ObservationCandidateInput {
        observation_kind: "evidence-backed".into(),
        content_type: content_type.into(),
        retention_tier: "durable".into(),
        scope: "workspace".into(),
        project: project.into(),
        title: content.into(),
        content: content.into(),
        source: "memory-evaluation".into(),
        source_id: stable_json_digest(&serde_json::json!([project, content])),
        dedupe_key: dedupe_key.map(str::to_string),
        confidence: 0.95,
        importance: 0.9,
        sensitivity: sensitivity.into(),
        acl: vec![project.into()],
        provenance: serde_json::json!({"fixture":"synthetic"}),
        expires_at: (Utc::now() + Duration::days(1)).to_rfc3339(),
    }
}

fn standalone_candidate(content: &str) -> ObservationCandidate {
    let now = Utc::now().to_rfc3339();
    ObservationCandidate {
        id: "candidate-evaluation".into(),
        observation_kind: "evidence-backed".into(),
        content_type: "preference".into(),
        retention_tier: "durable".into(),
        scope: "workspace".into(),
        created_by: "agent-a".into(),
        project: "work".into(),
        title: content.into(),
        content: content.into(),
        source: "memory-evaluation".into(),
        source_id: "candidate-evaluation".into(),
        dedupe_key: None,
        confidence: 0.95,
        importance: 0.9,
        sensitivity: "normal".into(),
        status: "pending".into(),
        acl: vec!["work".into()],
        provenance: serde_json::json!({"fixture":"synthetic"}),
        expires_at: (Utc::now() + Duration::days(1)).to_rfc3339(),
        rejection_reason: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn reflect_request(provider_policy: ProviderPolicy) -> ReflectRequest {
    ReflectRequest {
        objective: "Review bounded release-memory patterns".into(),
        project: Some("work".into()),
        memory: MemoryReflectFilter {
            limit: 20,
            ..Default::default()
        },
        include_evidence: false,
        include_derived: true,
        token_budget: 2_048,
        provider_policy,
        deadline_ms: 5_000,
        source: None,
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

static EVALUATION_RUN_LOCK: Mutex<()> = Mutex::new(());
static STORE_CASES: AtomicUsize = AtomicUsize::new(0);
static MAX_CASE_STORE_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TOTAL_STORE_BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct CaseStore {
    store: Store,
    root: PathBuf,
}

impl CaseStore {
    fn new() -> Result<Self> {
        let root =
            std::env::temp_dir().join(format!("cortana-memory-eval-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root)?;
        let store = Store::open(&root.join("memory.sqlite3"))?;
        STORE_CASES.fetch_add(1, Ordering::SeqCst);
        Ok(Self { store, root })
    }
}

impl Drop for CaseStore {
    fn drop(&mut self) {
        let bytes = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.metadata().ok())
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len())
            .sum::<u64>();
        TOTAL_STORE_BYTES.fetch_add(bytes, Ordering::SeqCst);
        MAX_CASE_STORE_BYTES.fetch_max(bytes, Ordering::SeqCst);
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            tracing::warn!(%error, path = %self.root.display(), "failed to remove memory evaluation store");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn executable_fixture_cannot_self_assert_private_approval() {
        let mut fixture: MemoryEvaluationFixture =
            serde_json::from_str(include_str!("../eval/memory-intelligence-fixtures.json"))
                .expect("built-in fixture");
        fixture.fixture_class = "approved-private".into();
        let error = validate_fixture(&fixture).expect_err("private evidence is external");
        assert!(error.to_string().contains("must be synthetic"));
    }

    fn built_in_fixture() -> MemoryEvaluationFixture {
        serde_json::from_str(include_str!("../eval/memory-intelligence-fixtures.json"))
            .expect("built-in fixture")
    }

    #[test]
    fn gating_fixture_requires_the_complete_canonical_registry() {
        let mut fixture = built_in_fixture();
        fixture.cases.pop();
        let error = validate_fixture(&fixture).expect_err("incomplete fixture must fail closed");
        assert!(error.to_string().contains("complete canonical case set"));

        let mut fixture = built_in_fixture();
        fixture.cases[0].failure_domain = FailureDomain::Model;
        let error = validate_fixture(&fixture).expect_err("relabeled domain must fail closed");
        assert!(error.to_string().contains("canonical registry"));
    }

    #[test]
    fn gating_fixture_cannot_weaken_resource_or_quality_thresholds() {
        let mut fixture = built_in_fixture();
        fixture.thresholds.max_provider_requests = 3;
        let error = validate_fixture(&fixture).expect_err("weaker threshold must fail closed");
        assert!(error.to_string().contains("cannot weaken"));

        let mut fixture = built_in_fixture();
        fixture.thresholds.min_candidate_recall = 0.5;
        let error = validate_fixture(&fixture).expect_err("weaker quality gate must fail closed");
        assert!(error.to_string().contains("cannot weaken"));
    }

    #[test]
    fn failure_reporting_uses_observed_sanitized_attribution() {
        let error = attributed::<()>(
            FailureDomain::Policy,
            "candidate-policy-boundary-failed",
            Err(anyhow::anyhow!("private provider detail")),
        )
        .expect_err("attributed failure");
        let (domain, reason_code) = observed_failure(&error);
        assert_eq!(domain, FailureDomain::Policy);
        assert_eq!(reason_code, "candidate-policy-boundary-failed");
        assert!(!reason_code.contains("private provider detail"));

        let nested = attributed(
            FailureDomain::Candidate,
            "candidate-evaluation-failed",
            attributed::<()>(
                FailureDomain::Policy,
                "candidate-policy-boundary-failed",
                Err(anyhow::anyhow!("policy regression")),
            ),
        )
        .expect_err("inner attribution must survive outer case attribution");
        assert_eq!(
            observed_failure(&nested),
            (
                FailureDomain::Policy,
                "candidate-policy-boundary-failed".into()
            )
        );

        let untagged = anyhow::anyhow!("unexpected infrastructure detail");
        assert_eq!(
            observed_failure(&untagged),
            (
                FailureDomain::Corpus,
                "evaluation-infrastructure-failed".into()
            )
        );
    }

    #[tokio::test]
    async fn unexpected_negative_path_store_failure_is_reported_as_corpus_infrastructure() {
        let report = run_fixture_with_evaluator(built_in_fixture(), |id| {
            if id == "candidate-safety-boundaries" {
                expected_policy_rejection::<()>(
                    Err(anyhow::Error::new(rusqlite::Error::InvalidQuery)),
                    "candidate ACL denied",
                )?;
                unreachable!("unexpected store errors must propagate")
            } else {
                evaluate_case(id)
            }
        })
        .await
        .expect("evaluation report");
        let failed = report
            .cases
            .iter()
            .find(|case| case.id == "candidate-safety-boundaries")
            .expect("forced case");
        assert!(!failed.passed);
        assert_eq!(failed.failure_domain, FailureDomain::Corpus);
        assert_eq!(failed.reason_code, "evaluation-infrastructure-failed");
        assert!(!failed.reason_code.contains("private sqlite detail"));
        assert_eq!(report.metrics.failures_by_domain.candidate, 0);
        assert_eq!(report.metrics.failures_by_domain.corpus, 1);
    }

    #[tokio::test]
    async fn report_digest_excludes_observational_timing() {
        let first = run_fixture(built_in_fixture()).await.expect("first report");
        let second = run_fixture(built_in_fixture())
            .await
            .expect("second report");
        assert_eq!(
            first.activation.report_digest,
            second.activation.report_digest
        );
        assert_eq!(first.metrics.candidate_precision, 1.0);
        assert_eq!(first.metrics.candidate_recall, 1.0);
        assert_eq!(first.metrics.failures_by_domain.candidate, 0);
        assert!(first.comparisons.iter().take(5).all(|comparison| {
            comparison.passed
                && comparison.explicit_memory_preserved
                && comparison.canonical_write_requires_approval
        }));
    }

    #[test]
    fn private_evidence_verifier_requires_approved_complete_external_results() {
        let mut template: serde_json::Value = serde_json::from_str(include_str!(
            "../eval/memory-intelligence-private.example.json"
        ))
        .expect("private template");
        template["approved"] = true.into();
        template["governance"]["reviewer_ids"] = serde_json::json!(["reviewer-digest-a"]);
        template["governance"]["corpus_revision"] = "corpus-digest-a".into();
        for case in template["required_cases"]
            .as_array_mut()
            .expect("private cases")
        {
            case["result"] = "passed".into();
        }
        let metrics = template["required_metrics"]
            .as_object_mut()
            .expect("private metrics");
        for name in [
            "candidate_precision",
            "candidate_recall",
            "classification_accuracy",
            "duplicate_suppression",
            "contradiction_detection",
            "supersession_correctness",
            "retention_accuracy",
            "recall_quality",
            "reflection_grounding",
            "derived_invalidation",
        ] {
            metrics.insert(name.into(), 1.0.into());
        }
        metrics.insert("approval_load".into(), 1.0.into());
        metrics.insert("latency_p95_ms".into(), 500.into());
        metrics.insert("cpu_seconds".into(), 1.0.into());
        metrics.insert("peak_rss_bytes".into(), 1024.into());
        metrics.insert("provider_requests".into(), 2.into());
        metrics.insert("estimated_provider_cost_usd".into(), 0.0.into());
        let mut file = tempfile::NamedTempFile::new().expect("private evidence file");
        serde_json::to_writer(&mut file, &template).expect("write private evidence");
        file.flush().expect("flush private evidence");
        let report = verify_private_evidence(file.path()).expect("approved private evidence");
        assert!(report.passed);
        assert!(!report.automatic_retention_activation_authorized);
        assert_eq!(report.case_count, 8);

        template["governance"]["reviewer_ids"] = serde_json::json!(["opaque-reviewer-id"]);
        let mut reviewer_placeholder =
            tempfile::NamedTempFile::new().expect("reviewer placeholder file");
        serde_json::to_writer(&mut reviewer_placeholder, &template)
            .expect("write reviewer placeholder");
        reviewer_placeholder
            .flush()
            .expect("flush reviewer placeholder");
        let error = verify_private_evidence(reviewer_placeholder.path())
            .expect_err("reviewer placeholder must fail alone");
        assert!(error.to_string().contains("placeholders must be replaced"));

        template["governance"]["reviewer_ids"] = serde_json::json!(["reviewer-digest-a"]);
        template["governance"]["corpus_revision"] = "opaque-approved-corpus-revision".into();
        let mut corpus_placeholder =
            tempfile::NamedTempFile::new().expect("corpus placeholder file");
        serde_json::to_writer(&mut corpus_placeholder, &template)
            .expect("write corpus placeholder");
        corpus_placeholder
            .flush()
            .expect("flush corpus placeholder");
        let error = verify_private_evidence(corpus_placeholder.path())
            .expect_err("corpus placeholder must fail alone");
        assert!(error.to_string().contains("placeholders must be replaced"));

        template["governance"]["corpus_revision"] = "corpus-digest-a".into();
        template["required_metrics"]["cpu_seconds"] = 61.0.into();
        let mut excessive_cpu = tempfile::NamedTempFile::new().expect("CPU evidence file");
        serde_json::to_writer(&mut excessive_cpu, &template).expect("write CPU evidence");
        excessive_cpu.flush().expect("flush CPU evidence");
        let error = verify_private_evidence(excessive_cpu.path()).expect_err("CPU cap must fail");
        assert!(error.to_string().contains("below required thresholds"));

        template["required_metrics"]["cpu_seconds"] = 1.0.into();
        template["required_metrics"]["peak_rss_bytes"] = (MAX_PRIVATE_PEAK_RSS_BYTES + 1).into();
        let mut excessive_rss = tempfile::NamedTempFile::new().expect("RSS evidence file");
        serde_json::to_writer(&mut excessive_rss, &template).expect("write RSS evidence");
        excessive_rss.flush().expect("flush RSS evidence");
        let error = verify_private_evidence(excessive_rss.path()).expect_err("RSS cap must fail");
        assert!(error.to_string().contains("below required thresholds"));

        template["required_metrics"]["peak_rss_bytes"] = 1024.into();
        template["required_metrics"]["unauthorized_exposures"] = 1.into();
        let mut rejected = tempfile::NamedTempFile::new().expect("rejected evidence file");
        serde_json::to_writer(&mut rejected, &template).expect("write rejected evidence");
        rejected.flush().expect("flush rejected evidence");
        let error = verify_private_evidence(rejected.path()).expect_err("exposure must fail");
        assert!(error.to_string().contains("below required thresholds"));
    }
}
