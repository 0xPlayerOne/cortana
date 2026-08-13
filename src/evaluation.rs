use std::collections::HashSet;
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use anyhow::{Context, Result, anyhow};
use chrono::Duration;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::answer::{AnswerEngine, AnswerRequest};
use crate::config::QueryConfig;
use crate::embed::{DeterministicEmbedder, Embedder};
use crate::model::{Document, Evidence};
use crate::retrieval;
use crate::store::Store;

use crate::answer::configured_model;

/// Keep the opt-in model quality gate below one minute while matching the
/// runtime's hard answer deadline. The gate performs more than one bounded
/// provider answer to prove cache reuse and corpus-revision invalidation, so a
/// 30-second whole-run ceiling rejected healthy providers solely because of
/// those required checks. A provider outage still fails closed at this bound.
pub const MODEL_EVALUATION_MAX_SECONDS: u64 = 55;

/// Keep user-supplied fixtures from turning a quality check into an unbounded
/// memory or temporary-index workload before validation can run.
const MAX_FIXTURE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FIXTURE_DOCUMENTS: usize = 2_000;
const MAX_FIXTURE_CASES: usize = 500;
const MAX_FIXTURE_DOCUMENT_CONTENT_BYTES: usize = 2 * 1024 * 1024;
const MAX_FIXTURE_QUERY_BYTES: usize = 16 * 1024;

// The fixture is deliberately tiny. Keep its prompts bounded independently
// from a user's production context/output settings so a large personal
// context budget cannot turn a synthetic release check into a load test.
const MODEL_EVALUATION_MAX_CONTEXT_TOKENS: usize = 2_048;
// Reasoning-capable gateways may spend part of the provider token budget on
// hidden reasoning before emitting the cited answer. Keep the synthetic gate
// bounded, but leave enough room for a short grounded response instead of
// treating an exhausted content budget as a citation failure.
const MODEL_EVALUATION_MAX_OUTPUT_TOKENS: usize = 512;

#[derive(Clone, Debug, Deserialize)]
pub struct EvaluationFixture {
    pub version: u32,
    pub thresholds: EvaluationThresholds,
    pub documents: Vec<Document>,
    pub retrieval_cases: Vec<RetrievalCase>,
    pub answer_case: AnswerCase,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EvaluationThresholds {
    pub min_recall_at_k: f64,
    pub min_mrr: f64,
    pub min_case_pass_rate: f64,
    pub min_citation_validity: f64,
    pub max_latency_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RetrievalCase {
    pub name: String,
    pub query: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    pub top_k: usize,
    #[serde(default)]
    pub acl: Vec<String>,
    #[serde(default)]
    pub expected_source_ids: Vec<String>,
    #[serde(default)]
    pub forbidden_source_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AnswerCase {
    pub query: String,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub acl: Vec<String>,
    pub expected_source_id: String,
    #[serde(default)]
    pub forbidden_source_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EvaluationReport {
    pub fixture_version: u32,
    pub passed: bool,
    pub thresholds: EvaluationThresholds,
    pub metrics: EvaluationMetrics,
    pub cases: Vec<CaseReport>,
    pub answer: AnswerEvaluation,
}

#[derive(Clone, Debug, Serialize)]
pub struct EvaluationMetrics {
    pub recall_at_k: f64,
    pub mrr: f64,
    pub case_pass_rate: f64,
    pub citation_validity: f64,
    pub max_latency_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CaseReport {
    pub name: String,
    pub passed: bool,
    pub latency_ms: u64,
    pub returned_source_ids: Vec<String>,
    pub missing_source_ids: Vec<String>,
    pub leaked_source_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AnswerEvaluation {
    pub attempted: bool,
    pub passed: bool,
    pub planner_model_used: bool,
    pub synthesis_model_used: bool,
    pub planner_bounded: bool,
    pub citations_valid: bool,
    pub expected_evidence_present: bool,
    pub forbidden_citations_absent: bool,
    pub fallback_mode: bool,
    pub fallback_provider_unavailable: bool,
    /// Stable, non-secret reason for an extractive fallback. This keeps a
    /// provider outage distinguishable from an invalid model response or a
    /// deadline without exposing raw provider errors in evaluation artifacts.
    pub fallback_reason: Option<String>,
    pub cache_hit: bool,
    pub cache_invalidated_after_update: bool,
    pub latency_ms: u64,
    pub deadline_ms: u64,
}

pub async fn run(path: &Path) -> Result<EvaluationReport> {
    let fixture = parse_fixture_file(path)?;
    run_fixture(fixture, None).await
}

pub async fn run_default() -> Result<EvaluationReport> {
    let fixture: EvaluationFixture = serde_json::from_str(include_str!("../eval/fixtures.json"))
        .context("invalid built-in evaluation fixture")?;
    run_fixture(fixture, None).await
}

pub async fn run_with_config(
    path: &Path,
    query: &QueryConfig,
    api_key: Option<String>,
) -> Result<EvaluationReport> {
    let fixture = parse_fixture_file(path)?;
    run_model_fixture(fixture, Some((bounded_model_config(query), api_key))).await
}

fn parse_fixture_file(path: &Path) -> Result<EvaluationFixture> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to inspect evaluation fixture {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_file(),
        "evaluation fixture is not a regular file: {}",
        path.display()
    );
    anyhow::ensure!(
        metadata.len() <= MAX_FIXTURE_BYTES,
        "evaluation fixture exceeds the {MAX_FIXTURE_BYTES} byte safety limit"
    );
    let file = std::fs::File::open(path)
        .with_context(|| format!("failed to read evaluation fixture {}", path.display()))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_FIXTURE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read evaluation fixture {}", path.display()))?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_FIXTURE_BYTES,
        "evaluation fixture exceeds the {MAX_FIXTURE_BYTES} byte safety limit"
    );
    let fixture: EvaluationFixture = serde_json::from_slice(&bytes)
        .with_context(|| format!("invalid evaluation fixture {}", path.display()))?;
    validate_fixture(&fixture)?;
    Ok(fixture)
}

pub async fn run_with_model_default(
    query: &QueryConfig,
    api_key: Option<String>,
) -> Result<EvaluationReport> {
    let fixture: EvaluationFixture = serde_json::from_str(include_str!("../eval/fixtures.json"))
        .context("invalid built-in evaluation fixture")?;
    run_model_fixture(fixture, Some((bounded_model_config(query), api_key))).await
}

async fn run_model_fixture(
    fixture: EvaluationFixture,
    model: Option<(QueryConfig, Option<String>)>,
) -> Result<EvaluationReport> {
    bounded_model_evaluation(
        run_fixture(fixture, model),
        StdDuration::from_secs(MODEL_EVALUATION_MAX_SECONDS),
    )
    .await
}

async fn bounded_model_evaluation<T, F>(future: F, timeout: StdDuration) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| anyhow!("model-backed evaluation timed out after {timeout:?}"))?
}

fn bounded_model_config(query: &QueryConfig) -> QueryConfig {
    let mut bounded = query.clone();
    bounded.context_tokens = bounded
        .context_tokens
        .min(MODEL_EVALUATION_MAX_CONTEXT_TOKENS);
    bounded.output_tokens = bounded
        .output_tokens
        .min(MODEL_EVALUATION_MAX_OUTPUT_TOKENS);
    bounded.answer_timeout_seconds = bounded
        .answer_timeout_seconds
        .min(MODEL_EVALUATION_MAX_SECONDS);
    bounded.request_timeout_seconds = bounded
        .request_timeout_seconds
        .min(MODEL_EVALUATION_MAX_SECONDS);
    bounded
}

async fn run_fixture(
    fixture: EvaluationFixture,
    model: Option<(QueryConfig, Option<String>)>,
) -> Result<EvaluationReport> {
    validate_fixture(&fixture)?;

    let temporary = TemporaryIndex::new()?;
    let store = Store::open(&temporary.path.join("evaluation.sqlite3"))?;
    let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(256));
    store.ensure_fingerprint(&embedder.fingerprint())?;
    for document in &fixture.documents {
        let embedding = embedder
            .embed(std::slice::from_ref(&document.content))
            .await?
            .into_iter()
            .next()
            .context("deterministic embedder returned no vector")?;
        store.upsert(document, &[(document.content.clone(), embedding)])?;
    }

    let mut case_reports = Vec::with_capacity(fixture.retrieval_cases.len());
    let mut relevant_total = 0_usize;
    let mut relevant_found = 0_usize;
    let mut reciprocal_ranks = Vec::new();
    for case in &fixture.retrieval_cases {
        let started = Instant::now();
        let evidence = retrieval::retrieve_scoped(
            &store,
            &embedder,
            &case.query,
            case.project.as_deref(),
            case.source.as_deref(),
            case.top_k,
            &effective_acl(&case.acl),
        )
        .await?;
        let latency_ms = elapsed_ms(started);
        let returned = evidence
            .iter()
            .map(|item| item.source_id.clone())
            .collect::<Vec<_>>();
        let returned_set = returned.iter().collect::<HashSet<_>>();
        let missing = case
            .expected_source_ids
            .iter()
            .filter(|source_id| !returned_set.contains(source_id))
            .cloned()
            .collect::<Vec<_>>();
        let leaked = case
            .forbidden_source_ids
            .iter()
            .filter(|source_id| returned_set.contains(source_id))
            .cloned()
            .collect::<Vec<_>>();
        relevant_total += case.expected_source_ids.len();
        relevant_found += case.expected_source_ids.len() - missing.len();
        if !case.expected_source_ids.is_empty() {
            reciprocal_ranks.push(reciprocal_rank(&evidence, &case.expected_source_ids));
        }
        let source_scope_valid = case
            .source
            .as_ref()
            .is_none_or(|source| evidence.iter().all(|item| &item.source == source));
        case_reports.push(CaseReport {
            name: case.name.clone(),
            passed: missing.is_empty() && leaked.is_empty() && source_scope_valid,
            latency_ms,
            returned_source_ids: returned,
            missing_source_ids: missing,
            leaked_source_ids: leaked,
        });
    }

    let answer = evaluate_answer(&store, &embedder, &fixture, model).await?;
    let recall_at_k = ratio(relevant_found, relevant_total);
    let mrr = if reciprocal_ranks.is_empty() {
        1.0
    } else {
        reciprocal_ranks.iter().sum::<f64>() / reciprocal_ranks.len() as f64
    };
    let passed_cases = case_reports.iter().filter(|case| case.passed).count();
    let case_pass_rate = ratio(passed_cases, case_reports.len());
    let citation_validity = if answer.citations_valid { 1.0 } else { 0.0 };
    let max_latency_ms = case_reports
        .iter()
        .map(|case| case.latency_ms)
        .max()
        .unwrap_or_default();
    let metrics = EvaluationMetrics {
        recall_at_k,
        mrr,
        case_pass_rate,
        citation_validity,
        max_latency_ms,
    };
    let thresholds = &fixture.thresholds;
    let passed = answer.passed
        && metrics.recall_at_k >= thresholds.min_recall_at_k
        && metrics.mrr >= thresholds.min_mrr
        && metrics.case_pass_rate >= thresholds.min_case_pass_rate
        && metrics.citation_validity >= thresholds.min_citation_validity
        && metrics.max_latency_ms <= thresholds.max_latency_ms;
    Ok(EvaluationReport {
        fixture_version: fixture.version,
        passed,
        thresholds: fixture.thresholds,
        metrics,
        cases: case_reports,
        answer,
    })
}

async fn evaluate_answer(
    store: &Store,
    embedder: &Arc<dyn Embedder>,
    fixture: &EvaluationFixture,
    model: Option<(QueryConfig, Option<String>)>,
) -> Result<AnswerEvaluation> {
    let (config, model) = if let Some((config, api_key)) = model {
        let model = configured_model(&config, api_key)?;
        (config, model)
    } else {
        (
            QueryConfig {
                synthesis_enabled: false,
                max_planned_queries: 2,
                cache_max_entries: 32,
                cache_ttl_seconds: 300,
                ..QueryConfig::default()
            },
            None,
        )
    };
    let model_available = config.synthesis_enabled && model.is_some();
    let engine = AnswerEngine::new(store.clone(), embedder.clone(), model, config.clone());
    let request = AnswerRequest {
        query: fixture.answer_case.query.clone(),
        project: fixture.answer_case.project.clone(),
        source: fixture.answer_case.source.clone(),
    };
    let acl = effective_acl(&fixture.answer_case.acl);
    let first = engine.answer_scoped(request.clone(), &acl).await?;
    let expected_evidence_present = first
        .evidence
        .iter()
        .any(|item| item.source_id == fixture.answer_case.expected_source_id);
    let forbidden_citations_absent = first
        .evidence
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            fixture
                .answer_case
                .forbidden_source_ids
                .contains(&item.source_id)
        })
        .all(|(index, _)| !first.answer.contains(&format!("[{}]", index + 1)));
    let planner_model_used = first.plan.model_generated;
    let synthesis_model_used = first.mode == "synthesized";
    let fallback_mode = first.mode == "extractive";
    let planner_bounded =
        !first.plan.queries.is_empty() && first.plan.queries.len() <= config.max_planned_queries;
    let citations_valid = citations_are_valid(&first.answer, first.evidence.len());
    let deadline_ms = config
        .answer_timeout_seconds
        .clamp(1, 55)
        .saturating_mul(1000);
    let fallback_provider_unavailable = model_available
        && first.warnings.iter().any(|warning| {
            warning.contains("planner unavailable") || warning.contains("synthesis unavailable")
        });
    let fallback_reason = classify_fallback_reason(&first.warnings, fallback_mode);

    // A provider outage must produce a bounded, actionable report. Do not
    // repeat the same network failure for the cache and post-update checks:
    // fallback responses are deliberately not cached, so retries would only
    // multiply the provider timeout without adding evidence.
    if fallback_provider_unavailable {
        return Ok(AnswerEvaluation {
            attempted: true,
            passed: false,
            planner_model_used,
            synthesis_model_used,
            planner_bounded,
            citations_valid,
            expected_evidence_present,
            forbidden_citations_absent,
            fallback_mode,
            fallback_provider_unavailable: true,
            fallback_reason,
            cache_hit: false,
            cache_invalidated_after_update: false,
            latency_ms: first.latency_ms,
            deadline_ms,
        });
    }

    let second = engine.answer_scoped(request.clone(), &acl).await?;

    let mut updated = fixture
        .documents
        .iter()
        .find(|document| document.source_id == fixture.answer_case.expected_source_id)
        .cloned()
        .context("answer case expected_source_id is absent from documents")?;
    updated.content.push_str(" Updated evaluation revision.");
    updated.updated_at += Duration::seconds(1);
    let embedding = embedder
        .embed(std::slice::from_ref(&updated.content))
        .await?
        .into_iter()
        .next()
        .context("deterministic embedder returned no vector")?;
    store.upsert(&updated, &[(updated.content.clone(), embedding)])?;
    let third = engine.answer_scoped(request, &acl).await?;

    let cache_hit = second.cached;
    let cache_invalidated_after_update = !third.cached;
    let passed = if model_available {
        synthesis_model_used
            && planner_model_used
            && citations_valid
            && expected_evidence_present
            && forbidden_citations_absent
            && planner_bounded
            && first.latency_ms <= deadline_ms
            && cache_invalidated_after_update
    } else {
        fallback_mode
            && planner_bounded
            && citations_valid
            && expected_evidence_present
            && forbidden_citations_absent
            && cache_hit
            && cache_invalidated_after_update
    };

    Ok(AnswerEvaluation {
        attempted: model_available,
        passed,
        planner_model_used,
        synthesis_model_used,
        planner_bounded,
        citations_valid,
        expected_evidence_present,
        forbidden_citations_absent,
        fallback_mode,
        fallback_provider_unavailable,
        fallback_reason,
        cache_hit,
        cache_invalidated_after_update,
        latency_ms: first.latency_ms,
        deadline_ms,
    })
}

fn classify_fallback_reason(warnings: &[String], fallback_mode: bool) -> Option<String> {
    if !fallback_mode {
        return None;
    }
    if warnings.iter().any(|warning| {
        warning.contains("planner unavailable") || warning.contains("synthesis unavailable")
    }) {
        return Some("provider_unavailable".into());
    }
    if warnings
        .iter()
        .any(|warning| warning.contains("invalid or missing citations"))
    {
        return Some("invalid_citations".into());
    }
    if warnings
        .iter()
        .any(|warning| warning.contains("answer deadline reached"))
    {
        return Some("deadline".into());
    }
    Some("extractive_fallback".into())
}

fn validate_fixture(fixture: &EvaluationFixture) -> Result<()> {
    anyhow::ensure!(
        fixture.version == 1,
        "unsupported evaluation fixture version"
    );
    anyhow::ensure!(!fixture.documents.is_empty(), "fixture has no documents");
    anyhow::ensure!(
        fixture.documents.len() <= MAX_FIXTURE_DOCUMENTS,
        "fixture exceeds the {MAX_FIXTURE_DOCUMENTS} document safety limit"
    );
    anyhow::ensure!(
        !fixture.retrieval_cases.is_empty(),
        "fixture has no retrieval cases"
    );
    anyhow::ensure!(
        fixture.retrieval_cases.len() <= MAX_FIXTURE_CASES,
        "fixture exceeds the {MAX_FIXTURE_CASES} retrieval-case safety limit"
    );
    anyhow::ensure!(
        fixture
            .documents
            .iter()
            .all(|document| document.content.len() <= MAX_FIXTURE_DOCUMENT_CONTENT_BYTES),
        "fixture document content exceeds the {MAX_FIXTURE_DOCUMENT_CONTENT_BYTES} byte safety limit"
    );
    anyhow::ensure!(
        fixture
            .retrieval_cases
            .iter()
            .all(|case| !case.name.trim().is_empty()
                && !case.query.trim().is_empty()
                && case.query.len() <= MAX_FIXTURE_QUERY_BYTES
                && (1..=50).contains(&case.top_k)),
        "retrieval cases require names, queries, and top_k between 1 and 50"
    );
    anyhow::ensure!(
        fixture.answer_case.query.len() <= MAX_FIXTURE_QUERY_BYTES,
        "answer case query exceeds the {MAX_FIXTURE_QUERY_BYTES} byte safety limit"
    );
    let source_ids = fixture
        .documents
        .iter()
        .map(|document| document.source_id.as_str())
        .collect::<HashSet<_>>();
    anyhow::ensure!(
        source_ids.len() == fixture.documents.len(),
        "fixture document source_id values must be unique"
    );
    anyhow::ensure!(
        source_ids.contains(fixture.answer_case.expected_source_id.as_str()),
        "answer case expected_source_id is absent from documents"
    );
    Ok(())
}

fn citations_are_valid(answer: &str, evidence_count: usize) -> bool {
    let bytes = answer.as_bytes();
    let mut citations = 0_usize;
    let mut index = 0_usize;
    while index < bytes.len() {
        if bytes[index] != b'[' {
            index += 1;
            continue;
        }
        let start = index + 1;
        let Some(relative_end) = bytes[start..].iter().position(|byte| *byte == b']') else {
            return false;
        };
        let end = start + relative_end;
        if end > start && bytes[start..end].iter().all(u8::is_ascii_digit) {
            let citation = answer[start..end].parse::<usize>().unwrap_or_default();
            if citation == 0 || citation > evidence_count {
                return false;
            }
            citations += 1;
        }
        index = end + 1;
    }
    citations > 0
}

fn reciprocal_rank(evidence: &[Evidence], expected: &[String]) -> f64 {
    evidence
        .iter()
        .position(|item| {
            expected
                .iter()
                .any(|source_id| source_id == &item.source_id)
        })
        .map_or(0.0, |index| 1.0 / (index + 1) as f64)
}

fn effective_acl(labels: &[String]) -> Vec<String> {
    if labels.is_empty() {
        vec!["*".into()]
    } else {
        labels.to_vec()
    }
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        1.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

struct TemporaryIndex {
    path: PathBuf,
}

impl TemporaryIndex {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("cortana-eval-{}", Uuid::new_v4()));
        std::fs::create_dir(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TemporaryIndex {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            tracing::warn!(%error, path = %self.path.display(), "failed to remove evaluation index");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use axum::{
        Router,
        http::StatusCode,
        response::{IntoResponse, Response},
        routing::post,
    };
    use serde_json::json;
    use tempfile::tempdir;

    #[derive(Clone)]
    struct MockModelServer {
        responses: Arc<Vec<ResponseTemplate>>,
        calls: Arc<AtomicU32>,
        latency_ms: Option<u64>,
    }

    #[derive(Clone)]
    enum ResponseTemplate {
        Planner(String),
        Synthesis(String),
        Failure(String),
    }

    impl ResponseTemplate {
        fn into_response(self) -> Response {
            match self {
                Self::Failure(error) => (StatusCode::BAD_GATEWAY, error).into_response(),
                Self::Planner(plan) | Self::Synthesis(plan) => (
                    StatusCode::OK,
                    serde_json::to_string(
                        &json!({ "choices": [{ "message": { "content": plan } }] }),
                    )
                    .expect("planner response JSON"),
                )
                    .into_response(),
            }
        }
    }

    async fn start_mock_model_server(
        responses: Vec<ResponseTemplate>,
        latency_ms: Option<u64>,
    ) -> (SocketAddr, Arc<AtomicU32>, tokio::task::JoinHandle<()>) {
        let state = MockModelServer {
            responses: Arc::new(responses),
            calls: Arc::new(AtomicU32::new(0)),
            latency_ms,
        };
        let app = Router::new().route(
            "/v1/chat/completions",
            post({
                let state = state.clone();
                move || {
                    let state = state.clone();
                    async move {
                        let index = state.calls.fetch_add(1, Ordering::SeqCst) as usize;
                        if let Some(delay) = state.latency_ms {
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                        }
                        state
                            .responses
                            .get(index)
                            .cloned()
                            .unwrap_or_else(|| ResponseTemplate::Failure("{}".into()))
                            .into_response()
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock model listener");
        let address = listener.local_addr().expect("mock model address");
        let handle =
            tokio::spawn(
                async move { axum::serve(listener, app).await.expect("serve mock model") },
            );
        (address, state.calls, handle)
    }

    fn fixture_path() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempdir().expect("fixture temp dir");
        let path = directory.path().join("fixtures.json");
        std::fs::write(&path, include_str!("../eval/fixtures.json")).expect("write fixture");
        (directory, path)
    }

    fn model_query_config(base_url: &str) -> QueryConfig {
        QueryConfig {
            synthesis_enabled: true,
            max_planned_queries: 2,
            answer_timeout_seconds: 3,
            base_url: base_url.to_string(),
            ..QueryConfig::default()
        }
    }

    #[tokio::test]
    async fn model_eval_successful_grounded_synthesis_meets_contract() {
        let responses = vec![
            ResponseTemplate::Planner(r#"{"queries":["release"]}"#.into()),
            ResponseTemplate::Synthesis(
                "The release process is bounded by safe checks. [1]".into(),
            ),
            ResponseTemplate::Planner(r#"{"queries":["release"]}"#.into()),
            ResponseTemplate::Synthesis(
                "The release process is bounded by safe checks. [1]".into(),
            ),
            ResponseTemplate::Planner(r#"{"queries":["release"]}"#.into()),
            ResponseTemplate::Synthesis(
                "The release process is bounded by safe checks. [1]".into(),
            ),
        ];
        let (address, _calls, server) = start_mock_model_server(responses, None).await;
        let (directory, fixture_path) = fixture_path();
        let report = run_with_config(
            &fixture_path,
            &model_query_config(&format!("http://{address}/v1")),
            None,
        )
        .await
        .expect("model-backed evaluation");
        drop(directory);
        server.abort();

        assert!(report.answer.attempted);
        assert!(report.answer.passed);
        assert!(report.answer.planner_model_used);
        assert!(report.answer.synthesis_model_used);
        assert!(report.answer.citations_valid);
        assert!(report.answer.expected_evidence_present);
        assert!(report.answer.forbidden_citations_absent);
        assert!(!report.answer.fallback_mode);
        assert!(!report.answer.fallback_provider_unavailable);
        assert!(report.answer.fallback_reason.is_none());
        assert!(report.answer.latency_ms <= report.answer.deadline_ms);
    }

    #[tokio::test]
    async fn model_eval_invalid_citations_fall_back_without_cache_pollution() {
        let responses = vec![
            ResponseTemplate::Planner(r#"{"queries":["release"]}"#.into()),
            ResponseTemplate::Synthesis("The release process cannot be verified.".into()),
            ResponseTemplate::Planner(r#"{"queries":["release"]}"#.into()),
            ResponseTemplate::Synthesis("The release process cannot be verified.".into()),
            ResponseTemplate::Planner(r#"{"queries":["release"]}"#.into()),
            ResponseTemplate::Synthesis("The release process cannot be verified.".into()),
        ];
        let (address, _calls, server) = start_mock_model_server(responses, None).await;
        let (directory, fixture_path) = fixture_path();
        let report = run_with_config(
            &fixture_path,
            &model_query_config(&format!("http://{address}/v1")),
            None,
        )
        .await
        .expect("model-backed evaluation fallback");
        drop(directory);
        server.abort();

        assert!(report.answer.attempted);
        assert!(!report.answer.passed);
        assert!(!report.answer.synthesis_model_used);
        assert!(report.answer.fallback_mode);
        assert!(!report.answer.cache_hit);
        assert!(!report.answer.fallback_provider_unavailable);
        assert_eq!(
            report.answer.fallback_reason.as_deref(),
            Some("invalid_citations")
        );
    }

    #[tokio::test]
    async fn model_eval_provider_failure_falls_back_and_surfaces_failure_marker() {
        let responses = vec![
            ResponseTemplate::Planner(r#"{"queries":["release"]}"#.into()),
            ResponseTemplate::Failure("provider failed".into()),
            ResponseTemplate::Planner(r#"{"queries":["release"]}"#.into()),
            ResponseTemplate::Failure("provider failed".into()),
            ResponseTemplate::Planner(r#"{"queries":["release"]}"#.into()),
            ResponseTemplate::Failure("provider failed".into()),
        ];
        let (address, calls, server) = start_mock_model_server(responses, None).await;
        let (directory, fixture_path) = fixture_path();
        let report = run_with_config(
            &fixture_path,
            &model_query_config(&format!("http://{address}/v1")),
            None,
        )
        .await
        .expect("model-backed evaluation");
        drop(directory);
        server.abort();

        assert!(report.answer.attempted);
        assert!(!report.answer.synthesis_model_used);
        assert!(report.answer.fallback_mode);
        assert!(report.answer.fallback_provider_unavailable);
        assert_eq!(
            report.answer.fallback_reason.as_deref(),
            Some("provider_unavailable")
        );
        assert!(!report.answer.passed);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "a provider outage must not repeat network failures for cache checks"
        );
    }

    #[tokio::test]
    async fn model_eval_respects_answer_deadline_bound() {
        let responses = vec![
            ResponseTemplate::Planner(r#"{"queries":["release"]}"#.into()),
            ResponseTemplate::Failure("provider failed".into()),
            ResponseTemplate::Planner(r#"{"queries":["release"]}"#.into()),
            ResponseTemplate::Failure("provider failed".into()),
            ResponseTemplate::Planner(r#"{"queries":["release"]}"#.into()),
            ResponseTemplate::Failure("provider failed".into()),
        ];
        let (address, _calls, server) = start_mock_model_server(responses, Some(2_000)).await;
        let (directory, fixture_path) = fixture_path();
        let mut config = model_query_config(&format!("http://{address}/v1"));
        config.answer_timeout_seconds = 1;
        let report = run_with_config(&fixture_path, &config, None)
            .await
            .expect("deadline bounded evaluation");
        drop(directory);
        server.abort();

        assert!(report.answer.attempted);
        assert!(report.answer.latency_ms <= 2_000);
        assert_eq!(report.answer.deadline_ms, 1_000);
        assert!(report.answer.latency_ms < 1_500);
    }

    #[test]
    fn citation_validation_requires_real_evidence_indices() {
        assert!(citations_are_valid("Supported [1].", 1));
        assert!(citations_are_valid("Supported [1] and [2].", 2));
        assert!(!citations_are_valid("Unsupported.", 1));
        assert!(!citations_are_valid("Unknown [2].", 1));
        assert!(!citations_are_valid("Invalid [0].", 1));
    }

    #[test]
    fn model_evaluation_caps_provider_and_answer_deadlines() {
        let config = QueryConfig {
            answer_timeout_seconds: 55,
            request_timeout_seconds: 60,
            ..QueryConfig::default()
        };
        let bounded = bounded_model_config(&config);
        assert_eq!(bounded.answer_timeout_seconds, MODEL_EVALUATION_MAX_SECONDS);
        assert_eq!(
            bounded.request_timeout_seconds,
            MODEL_EVALUATION_MAX_SECONDS
        );
        assert_eq!(bounded.context_tokens, MODEL_EVALUATION_MAX_CONTEXT_TOKENS);
        assert_eq!(bounded.output_tokens, MODEL_EVALUATION_MAX_OUTPUT_TOKENS);

        let already_bounded = QueryConfig {
            answer_timeout_seconds: 5,
            request_timeout_seconds: 7,
            ..QueryConfig::default()
        };
        let unchanged = bounded_model_config(&already_bounded);
        assert_eq!(unchanged.answer_timeout_seconds, 5);
        assert_eq!(unchanged.request_timeout_seconds, 7);
        assert_eq!(
            unchanged.context_tokens,
            QueryConfig::default()
                .context_tokens
                .min(MODEL_EVALUATION_MAX_CONTEXT_TOKENS)
        );
        assert_eq!(
            unchanged.output_tokens,
            QueryConfig::default()
                .output_tokens
                .min(MODEL_EVALUATION_MAX_OUTPUT_TOKENS)
        );
    }

    #[test]
    fn model_evaluation_budget_stays_below_one_minute() {
        let budget = std::hint::black_box(MODEL_EVALUATION_MAX_SECONDS);
        assert!(budget < 60);
    }

    #[tokio::test]
    async fn model_evaluation_timeout_fails_closed() {
        let error = bounded_model_evaluation(
            std::future::pending::<Result<()>>(),
            StdDuration::from_millis(1),
        )
        .await
        .expect_err("a stalled model evaluation must fail closed");
        assert!(
            error
                .to_string()
                .contains("model-backed evaluation timed out")
        );
    }
}
