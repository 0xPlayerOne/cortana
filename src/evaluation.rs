use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::Duration;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::answer::{AnswerEngine, AnswerRequest};
use crate::config::QueryConfig;
use crate::embed::{DeterministicEmbedder, Embedder};
use crate::model::{Document, Evidence};
use crate::retrieval;
use crate::store::Store;

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
    pub passed: bool,
    pub fallback_mode: bool,
    pub planner_bounded: bool,
    pub citations_valid: bool,
    pub expected_evidence_present: bool,
    pub forbidden_citations_absent: bool,
    pub cache_hit: bool,
    pub cache_invalidated_after_update: bool,
}

pub async fn run(path: &Path) -> Result<EvaluationReport> {
    let fixture: EvaluationFixture = serde_json::from_slice(
        &std::fs::read(path)
            .with_context(|| format!("failed to read evaluation fixture {}", path.display()))?,
    )
    .with_context(|| format!("invalid evaluation fixture {}", path.display()))?;
    run_fixture(fixture).await
}

pub async fn run_default() -> Result<EvaluationReport> {
    let fixture: EvaluationFixture = serde_json::from_str(include_str!("../eval/fixtures.json"))
        .context("invalid built-in evaluation fixture")?;
    run_fixture(fixture).await
}

async fn run_fixture(fixture: EvaluationFixture) -> Result<EvaluationReport> {
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

    let answer = evaluate_answer(&store, &embedder, &fixture).await?;
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
) -> Result<AnswerEvaluation> {
    let config = QueryConfig {
        synthesis_enabled: false,
        max_planned_queries: 2,
        cache_max_entries: 32,
        cache_ttl_seconds: 300,
        ..QueryConfig::default()
    };
    let engine = AnswerEngine::new(store.clone(), embedder.clone(), None, config.clone());
    let request = AnswerRequest {
        query: fixture.answer_case.query.clone(),
        project: fixture.answer_case.project.clone(),
        source: fixture.answer_case.source.clone(),
    };
    let acl = effective_acl(&fixture.answer_case.acl);
    let first = engine.answer_scoped(request.clone(), &acl).await?;
    let second = engine.answer_scoped(request.clone(), &acl).await?;
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

    let fallback_mode = first.mode == "extractive";
    let planner_bounded = !first.plan.model_generated
        && !first.plan.queries.is_empty()
        && first.plan.queries.len() <= config.max_planned_queries;
    let citations_valid = citations_are_valid(&first.answer, first.evidence.len());
    let cache_hit = second.cached;
    let cache_invalidated_after_update = !third.cached;
    Ok(AnswerEvaluation {
        passed: fallback_mode
            && planner_bounded
            && citations_valid
            && expected_evidence_present
            && forbidden_citations_absent
            && cache_hit
            && cache_invalidated_after_update,
        fallback_mode,
        planner_bounded,
        citations_valid,
        expected_evidence_present,
        forbidden_citations_absent,
        cache_hit,
        cache_invalidated_after_update,
    })
}

fn validate_fixture(fixture: &EvaluationFixture) -> Result<()> {
    anyhow::ensure!(
        fixture.version == 1,
        "unsupported evaluation fixture version"
    );
    anyhow::ensure!(!fixture.documents.is_empty(), "fixture has no documents");
    anyhow::ensure!(
        !fixture.retrieval_cases.is_empty(),
        "fixture has no retrieval cases"
    );
    anyhow::ensure!(
        fixture
            .retrieval_cases
            .iter()
            .all(|case| !case.name.trim().is_empty()
                && !case.query.trim().is_empty()
                && (1..=50).contains(&case.top_k)),
        "retrieval cases require names, queries, and top_k between 1 and 50"
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

    #[test]
    fn citation_validation_requires_real_evidence_indices() {
        assert!(citations_are_valid("Supported [1].", 1));
        assert!(citations_are_valid("Supported [1] and [2].", 2));
        assert!(!citations_are_valid("Unsupported.", 1));
        assert!(!citations_are_valid("Unknown [2].", 1));
        assert!(!citations_are_valid("Invalid [0].", 1));
    }
}
