//! Provider-free large-corpus and relationship-quality release gate.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use chrono::{Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use crate::api::{AppState, router};
use crate::auth::AuthPolicy;
use crate::config::{AuthTokenConfig, Config, KnowledgeGraphConfig, WorkspaceConfig};
use crate::contracts::stable_json_digest;
use crate::embed::{DeterministicEmbedder, Embedder};
use crate::knowledge_graph::{EdgeKind, GraphContract};
use crate::model::Document;
use crate::store::Store;

const TOKEN: &str = "synthetic-workspace-token";
const MAX_RESPONSE_CAPTURE_BYTES: usize = 2 * 1024 * 1024 + 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KnowledgeEvaluationFixture {
    pub version: u32,
    pub contract_version: String,
    pub corpus: CorpusConfig,
    pub iterations: usize,
    pub thresholds: KnowledgeThresholds,
    pub release: ReleasePolicy,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CorpusConfig {
    pub workspaces: usize,
    pub sources_per_workspace: usize,
    pub documents_per_workspace: usize,
    pub content_bytes_per_document: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KnowledgeThresholds {
    pub max_startup_p95_ms: u64,
    pub max_status_p95_ms: u64,
    pub max_document_list_p95_ms: u64,
    pub max_search_p95_ms: u64,
    pub max_document_view_p95_ms: u64,
    pub max_graph_open_p95_ms: u64,
    pub max_graph_focus_p95_ms: u64,
    pub max_graph_filter_p95_ms: u64,
    pub max_concurrent_search_graph_p95_ms: u64,
    pub max_response_bytes: usize,
    pub max_index_bytes: u64,
    pub max_peak_rss_bytes: u64,
    pub max_working_set_estimate_bytes: u64,
    pub max_total_wall_ms: u64,
    pub max_total_cpu_ms: u64,
    pub min_edge_precision: f64,
    pub min_edge_coverage: f64,
    pub min_provenance_completeness: f64,
    pub max_acl_leaks: usize,
    pub max_false_inferences: usize,
    pub max_invalidation_failures: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReleasePolicy {
    pub enabled_by_default: Vec<EdgeKind>,
    pub disabled_by_default: Vec<EdgeKind>,
}

#[derive(Clone, Debug, Serialize)]
pub struct KnowledgeEvaluationReport {
    pub fixture_version: u32,
    pub contract_version: String,
    pub graph_contract_version: &'static str,
    pub fixture_class: &'static str,
    pub provider_free: bool,
    pub passed: bool,
    pub thresholds: KnowledgeThresholds,
    pub corpus: CorpusMetrics,
    pub performance: BTreeMap<String, OperationMetrics>,
    pub resources: ResourceMetrics,
    pub relationship_correctness: RelationshipMetrics,
    pub controls: ControlMetrics,
    pub safety: SafetyMetrics,
    pub release: ReleaseEvaluation,
    pub visual_usability: VisualUsabilityStatus,
    pub failures: Vec<String>,
    pub evaluation_digest: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CorpusMetrics {
    pub workspaces: usize,
    pub sources: usize,
    pub documents: usize,
    pub chunks: usize,
    pub corpus_revision: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct OperationMetrics {
    pub samples: usize,
    pub latency_p50_ms: u64,
    pub latency_p95_ms: u64,
    pub latency_max_ms: u64,
    pub response_bytes_max: usize,
    pub requests_per_action: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResourceMetrics {
    pub index_bytes: u64,
    pub peak_rss_bytes: Option<u64>,
    pub working_set_estimate_bytes: u64,
    pub total_wall_ms: u64,
    pub total_cpu_ms: Option<u64>,
    pub cold_reopen_samples: usize,
    pub response_node_limit: usize,
    pub response_edge_limit: usize,
    pub response_byte_limit: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct RelationshipMetrics {
    pub evaluated_edges: usize,
    pub valid_edges: usize,
    pub edge_precision: f64,
    pub expected_edge_kinds: Vec<EdgeKind>,
    pub observed_edge_kinds: Vec<EdgeKind>,
    pub edge_coverage: f64,
    pub provenance_completeness: f64,
    pub false_inferences: usize,
    pub invalid_edge_kinds: Vec<EdgeKind>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ControlMetrics {
    pub graph_required_for_search: bool,
    pub graph_required_for_exact_document: bool,
    pub search_control_passed: bool,
    pub exact_document_control_passed: bool,
    pub graph_navigation_steps: usize,
    pub search_document_navigation_steps: usize,
    pub graph_step_reduction: isize,
}

#[derive(Clone, Debug, Serialize)]
pub struct SafetyMetrics {
    pub acl_leaks: usize,
    pub invalidation_failures: usize,
    pub stale_cursor_rejected: bool,
    pub disabled_edge_absent: bool,
    pub canonical_document_preserved_when_edge_disabled: bool,
    pub response_limits_respected: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReleaseEvaluation {
    pub enabled_by_default: Vec<EdgeKind>,
    pub disabled_by_default: Vec<EdgeKind>,
    pub independent_edge_disable_passed: bool,
    pub semantic_neighbors_enabled: bool,
    pub inferred_edges_enabled: bool,
    pub synthetic_gate_passed: bool,
    pub approved_corpus_gate: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct VisualUsabilityStatus {
    pub status: &'static str,
    pub reason: &'static str,
}

#[derive(Clone)]
struct SyntheticRecord {
    project: String,
    source: String,
    source_id: String,
    thread: String,
    authors: BTreeSet<String>,
    entities: BTreeSet<String>,
    references: BTreeSet<String>,
}

#[derive(Clone)]
struct RequestSpec {
    method: Method,
    uri: String,
    body: Option<Value>,
}

struct ResponseSample {
    status: StatusCode,
    bytes: usize,
    value: Value,
    latency_ms: u64,
}

pub async fn run_default() -> Result<KnowledgeEvaluationReport> {
    let fixture: KnowledgeEvaluationFixture =
        serde_json::from_str(include_str!("../eval/knowledge-graph-v1.json"))
            .context("invalid built-in knowledge evaluation fixture")?;
    run_fixture(fixture).await
}

async fn run_fixture(fixture: KnowledgeEvaluationFixture) -> Result<KnowledgeEvaluationReport> {
    validate_fixture(&fixture)?;
    let wall_started = Instant::now();
    let cpu_started = process_cpu_ms();
    let temporary = tempfile::tempdir().context("create knowledge evaluation directory")?;
    let database_path = temporary.path().join("knowledge-evaluation.sqlite3");
    let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(256));
    let store = Store::open(&database_path)?;
    store.ensure_fingerprint(&embedder.fingerprint())?;
    let records = populate_corpus(&store, &fixture)?;
    let corpus_revision = store.corpus_revision()?;
    drop(store);

    let mut startup_latencies = Vec::new();
    for _ in 0..fixture.iterations {
        let started = Instant::now();
        let reopened = Store::open(&database_path)?;
        startup_latencies.push(elapsed_ms(started));
        drop(reopened);
    }

    let store = Store::open(&database_path)?;
    let config = evaluation_config(&fixture);
    let policy = AuthPolicy::from_config(&config)?;
    let app = router(
        AppState::new(store.clone(), embedder)
            .with_config(&config, false)
            .with_auth_policy(policy),
    );

    let target_source_id = "note-0092";
    let target_id = stable_id("source-w00-0", target_source_id);
    let status_spec = get("/v1/status");
    let list_spec = get("/v1/documents?project=workspace-00&limit=50");
    let search_spec = post(
        "/v1/search",
        json!({
            "query": "unique navigation target 0092",
            "project": "workspace-00",
            "source": "source-w00-0",
            "limit": 10
        }),
    );
    let detail_spec = get(&format!("/v1/documents/{target_id}"));
    let graph_open_spec = get("/v1/graph?project=workspace-00&limit=50");
    let graph_focus_spec = get(&format!(
        "/v1/graph?project=workspace-00&focus_document_id={target_id}&limit=50"
    ));
    let graph_filter_spec = get(&format!(
        "/v1/graph?project=workspace-00&focus_document_id={target_id}&edge_kind=references&limit=50"
    ));

    // Warm SQLite pages and deterministic retrieval caches outside measured samples.
    request(&app, &status_spec).await?;
    request(&app, &graph_focus_spec).await?;

    let mut performance = BTreeMap::new();
    performance.insert(
        "startup-cold-reopen".into(),
        operation_metrics(&startup_latencies, 0, 1),
    );
    for (name, spec) in [
        ("status-workspace-source-tree", &status_spec),
        ("document-list", &list_spec),
        ("search-results", &search_spec),
        ("document-view", &detail_spec),
        ("graph-open", &graph_open_spec),
        ("graph-focus-expansion", &graph_focus_spec),
        ("graph-filter", &graph_filter_spec),
    ] {
        performance.insert(name.into(), measure(&app, spec, fixture.iterations).await?);
    }
    performance.insert(
        "concurrent-search-graph".into(),
        measure_concurrent(&app, &search_spec, &graph_focus_spec, fixture.iterations).await?,
    );

    let status = request(&app, &status_spec).await?;
    let list = request(&app, &list_spec).await?;
    let search = request(&app, &search_spec).await?;
    let detail = request(&app, &detail_spec).await?;
    let graph_open = request(&app, &graph_open_spec).await?;
    let graph_focus = request(&app, &graph_focus_spec).await?;
    let graph_filter = request(&app, &graph_filter_spec).await?;

    let acl_leaks = count_acl_leaks(&status.value, &list.value, &search.value, &graph_open.value);
    let relationship_correctness = relationship_metrics(&graph_focus.value, &records)?;
    let search_control_passed = search.value.as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["source_id"] == target_source_id)
    });
    let exact_document_control_passed = detail.value["source_id"] == target_source_id
        && detail.value["content"]
            .as_str()
            .is_some_and(|content| content.contains("unique navigation target 0092"));
    let reference_found = graph_filter
        .value
        .get("edges")
        .and_then(Value::as_array)
        .is_some_and(|edges| !edges.is_empty());

    let root_cursor = graph_open.value["next_cursor"].as_str().map(str::to_string);
    let mut updated = synthetic_document(0, 92, &fixture.corpus);
    updated.content.push_str(" Updated canonical revision.");
    updated.updated_at += Duration::seconds(1);
    updated.metadata = json!({
        "thread_id": "thread-w00-05",
        "authors": ["author-0"],
        "entities": ["entity-0"]
    });
    store.upsert(&updated, &[(updated.content.clone(), vec![1.0; 256])])?;

    let changed_focus = request(&app, &graph_focus_spec).await?;
    let stale_cursor_rejected = if let Some(cursor) = root_cursor {
        request_allow_status(
            &app,
            &get(&format!(
                "/v1/graph?project=workspace-00&limit=50&cursor={cursor}"
            )),
        )
        .await?
        .status
            == StatusCode::CONFLICT
    } else {
        false
    };
    let references_removed = !changed_focus
        .value
        .get("edges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|edge| {
            edge["kind"] == "references"
                && edge["support"]["record_ids"]
                    .as_array()
                    .is_some_and(|ids| ids.first().is_some_and(|id| id == &target_id))
        });
    let changed_revision = changed_focus
        .value
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|node| node["canonical_record_id"] == target_id)
        .and_then(|node| node["content_revision"].as_str())
        != graph_focus
            .value
            .get("nodes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|node| node["canonical_record_id"] == target_id)
            .and_then(|node| node["content_revision"].as_str());
    let invalidation_failures = usize::from(!stale_cursor_rejected)
        + usize::from(!references_removed)
        + usize::from(!changed_revision);

    let mut mentions_disabled = config.clone();
    mentions_disabled
        .knowledge_graph
        .enabled_edge_kinds
        .retain(|kind| *kind != EdgeKind::Mentions);
    let disabled_app = router(
        AppState::new(store.clone(), Arc::new(DeterministicEmbedder::new(256)))
            .with_config(&mentions_disabled, false)
            .with_auth_policy(AuthPolicy::from_config(&mentions_disabled)?),
    );
    let disabled_graph = request(&disabled_app, &graph_focus_spec).await?;
    let disabled_detail = request(&disabled_app, &detail_spec).await?;
    let disabled_edge_absent = disabled_graph
        .value
        .get("edges")
        .and_then(Value::as_array)
        .is_some_and(|edges| edges.iter().all(|edge| edge["kind"] != "mentions"));
    let canonical_document_preserved_when_edge_disabled =
        disabled_detail.value["source_id"] == target_source_id;

    let max_response_bytes = performance
        .values()
        .map(|operation| operation.response_bytes_max)
        .max()
        .unwrap_or_default();
    let response_limits_respected = max_response_bytes <= fixture.thresholds.max_response_bytes
        && graph_focus.value["nodes"]
            .as_array()
            .is_some_and(|nodes| nodes.len() <= GraphContract::MAX_NODES_PER_EXPANSION)
        && graph_focus.value["edges"]
            .as_array()
            .is_some_and(|edges| edges.len() <= GraphContract::MAX_EDGES_PER_EXPANSION)
        && graph_focus.value["response_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes <= fixture.thresholds.max_response_bytes as u64);

    let index_bytes = directory_bytes(temporary.path())?;
    let working_set_estimate_bytes = index_bytes
        .saturating_add((max_response_bytes as u64).saturating_mul(3))
        .saturating_add(
            (fixture.corpus.workspaces
                * fixture.corpus.documents_per_workspace
                * (fixture.corpus.content_bytes_per_document + 1024)) as u64,
        );
    let resources = ResourceMetrics {
        index_bytes,
        peak_rss_bytes: peak_rss_bytes(),
        working_set_estimate_bytes,
        total_wall_ms: elapsed_ms(wall_started),
        total_cpu_ms: process_cpu_ms()
            .zip(cpu_started)
            .map(|(end, start)| end.saturating_sub(start)),
        cold_reopen_samples: startup_latencies.len(),
        response_node_limit: GraphContract::MAX_NODES_PER_EXPANSION,
        response_edge_limit: GraphContract::MAX_EDGES_PER_EXPANSION,
        response_byte_limit: fixture.thresholds.max_response_bytes,
    };
    let controls = ControlMetrics {
        graph_required_for_search: false,
        graph_required_for_exact_document: false,
        search_control_passed,
        exact_document_control_passed,
        graph_navigation_steps: usize::from(reference_found),
        search_document_navigation_steps: 2,
        graph_step_reduction: if reference_found { 1 } else { 0 },
    };
    let safety = SafetyMetrics {
        acl_leaks,
        invalidation_failures,
        stale_cursor_rejected,
        disabled_edge_absent,
        canonical_document_preserved_when_edge_disabled,
        response_limits_respected,
    };
    let release = ReleaseEvaluation {
        enabled_by_default: fixture.release.enabled_by_default.clone(),
        disabled_by_default: fixture.release.disabled_by_default.clone(),
        independent_edge_disable_passed: disabled_edge_absent
            && canonical_document_preserved_when_edge_disabled,
        semantic_neighbors_enabled: false,
        inferred_edges_enabled: false,
        synthetic_gate_passed: false,
        approved_corpus_gate: "not-run",
    };
    let corpus = CorpusMetrics {
        workspaces: fixture.corpus.workspaces,
        sources: fixture.corpus.workspaces * fixture.corpus.sources_per_workspace,
        documents: fixture.corpus.workspaces * fixture.corpus.documents_per_workspace,
        chunks: fixture.corpus.workspaces * fixture.corpus.documents_per_workspace,
        corpus_revision,
    };

    let mut failures = evaluate_failures(
        &fixture.thresholds,
        &performance,
        &resources,
        &relationship_correctness,
        &controls,
        &safety,
        &release,
    );
    if status.value["workspaces"].as_array().map(Vec::len) != Some(1) {
        failures.push("scoped status did not expose exactly one authorized workspace".into());
    }
    let passed = failures.is_empty();
    let evaluation_digest = stable_json_digest(&json!({
        "fixture_version": fixture.version,
        "contract_version": fixture.contract_version,
        "graph_contract_version": GraphContract::VERSION,
        "corpus": &corpus,
        "relationship_correctness": &relationship_correctness,
        "controls": &controls,
        "safety": &safety,
        "release_enabled": &fixture.release.enabled_by_default,
        "release_disabled": &fixture.release.disabled_by_default,
        "passed": passed,
    }));
    let mut report = KnowledgeEvaluationReport {
        fixture_version: fixture.version,
        contract_version: fixture.contract_version,
        graph_contract_version: GraphContract::VERSION,
        fixture_class: "synthetic",
        provider_free: true,
        passed,
        thresholds: fixture.thresholds,
        corpus,
        performance,
        resources,
        relationship_correctness,
        controls,
        safety,
        release,
        visual_usability: VisualUsabilityStatus {
            status: "separate-packaged-gate",
            reason: "relationship correctness does not substitute for packaged visual and accessibility acceptance",
        },
        failures,
        evaluation_digest,
    };
    report.release.synthetic_gate_passed = report.passed;
    Ok(report)
}

fn validate_fixture(fixture: &KnowledgeEvaluationFixture) -> Result<()> {
    if fixture.version != 1 || fixture.contract_version != "cortana.knowledge-evaluation.v1" {
        bail!("unsupported knowledge evaluation fixture");
    }
    let corpus = &fixture.corpus;
    if !(25..=128).contains(&corpus.workspaces)
        || !(1..=16).contains(&corpus.sources_per_workspace)
        || !(20..=400).contains(&corpus.documents_per_workspace)
        || !(128..=4096).contains(&corpus.content_bytes_per_document)
        || corpus
            .workspaces
            .saturating_mul(corpus.documents_per_workspace)
            > 20_000
        || !(5..=50).contains(&fixture.iterations)
    {
        bail!("knowledge evaluation corpus or iteration budget is invalid");
    }
    let enabled = fixture
        .release
        .enabled_by_default
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let disabled = fixture
        .release
        .disabled_by_default
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if enabled.len() != fixture.release.enabled_by_default.len()
        || disabled.len() != fixture.release.disabled_by_default.len()
        || !enabled.is_disjoint(&disabled)
        || enabled.contains(&EdgeKind::SemanticallyRelated)
        || enabled.union(&disabled).copied().collect::<BTreeSet<_>>()
            != EdgeKind::ALL.into_iter().collect()
    {
        bail!("knowledge evaluation release edge policy is incomplete or unsafe");
    }
    if fixture.thresholds.min_edge_precision != 1.0
        || fixture.thresholds.min_edge_coverage != 1.0
        || fixture.thresholds.min_provenance_completeness != 1.0
        || fixture.thresholds.max_acl_leaks != 0
        || fixture.thresholds.max_false_inferences != 0
        || fixture.thresholds.max_invalidation_failures != 0
        || fixture.thresholds.max_response_bytes > 2 * 1024 * 1024
        || fixture.thresholds.max_index_bytes > 512 * 1024 * 1024
        || fixture.thresholds.max_peak_rss_bytes > 2 * 1024 * 1024 * 1024
        || fixture.thresholds.max_total_wall_ms > 60_000
        || fixture.thresholds.max_total_cpu_ms > 60_000
    {
        bail!("knowledge evaluation thresholds weaken the release safety ceiling");
    }
    Ok(())
}

fn evaluation_config(fixture: &KnowledgeEvaluationFixture) -> Config {
    let mut config = Config {
        workspaces: (0..fixture.corpus.workspaces)
            .map(|index| WorkspaceConfig {
                id: format!("workspace-{index:02}"),
                name: format!("Workspace {index:02}"),
                account_label: None,
                color: None,
            })
            .collect(),
        knowledge_graph: KnowledgeGraphConfig {
            enabled_edge_kinds: fixture.release.enabled_by_default.clone(),
        },
        ..Config::default()
    };
    config
        .environment
        .insert("KNOWLEDGE_EVAL_TOKEN".into(), TOKEN.into());
    config.auth.tokens = vec![AuthTokenConfig {
        principal: "knowledge-evaluator".into(),
        token_env: "KNOWLEDGE_EVAL_TOKEN".into(),
        scopes: vec!["query".into(), "status".into()],
        acl: vec!["workspace-00".into()],
    }];
    config
}

fn populate_corpus(
    store: &Store,
    fixture: &KnowledgeEvaluationFixture,
) -> Result<HashMap<String, SyntheticRecord>> {
    let mut records = HashMap::new();
    for workspace in 0..fixture.corpus.workspaces {
        for index in 0..fixture.corpus.documents_per_workspace {
            let document = synthetic_document(workspace, index, &fixture.corpus);
            let record = synthetic_record(workspace, index, &fixture.corpus);
            let id = stable_id(&document.source, &document.source_id);
            store.upsert(&document, &[(document.content.clone(), vec![1.0; 256])])?;
            records.insert(id, record);
        }
    }
    Ok(records)
}

fn synthetic_document(workspace: usize, index: usize, corpus: &CorpusConfig) -> Document {
    let record = synthetic_record(workspace, index, corpus);
    let unique = if workspace == 0 && index == 92 {
        " unique navigation target 0092"
    } else {
        ""
    };
    let prefix =
        format!("Synthetic canonical document {index:04} in workspace {workspace:02}.{unique} ");
    let content = if prefix.len() >= corpus.content_bytes_per_document {
        prefix
    } else {
        format!(
            "{prefix}{}",
            "x".repeat(corpus.content_bytes_per_document - prefix.len())
        )
    };
    Document {
        source: record.source.clone(),
        source_id: record.source_id.clone(),
        title: format!("Workspace {workspace:02} document {index:04}"),
        content,
        uri: Some(format!(
            "https://synthetic.invalid/{workspace:02}/{index:04}"
        )),
        updated_at: Utc
            .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .single()
            .expect("valid fixture epoch")
            + Duration::seconds((workspace * corpus.documents_per_workspace + index) as i64),
        project: record.project,
        acl: vec![format!("workspace-{workspace:02}")],
        metadata: json!({
            "thread_id": record.thread,
            "authors": record.authors,
            "entities": record.entities,
            "references": record.references,
            "credential": "must-never-become-a-graph-edge"
        }),
    }
}

fn synthetic_record(workspace: usize, index: usize, corpus: &CorpusConfig) -> SyntheticRecord {
    let source_index = index % corpus.sources_per_workspace;
    let references = (index >= corpus.sources_per_workspace)
        .then(|| format!("note-{:04}", index - corpus.sources_per_workspace))
        .into_iter()
        .collect();
    SyntheticRecord {
        project: format!("workspace-{workspace:02}"),
        source: format!("source-w{workspace:02}-{source_index}"),
        source_id: format!("note-{index:04}"),
        thread: format!("thread-w{workspace:02}-{:02}", index / 16),
        authors: BTreeSet::from([format!("author-{}", index % 3)]),
        entities: BTreeSet::from([format!("entity-{}", index % 5)]),
        references,
    }
}

async fn measure(app: &Router, spec: &RequestSpec, iterations: usize) -> Result<OperationMetrics> {
    let mut latencies = Vec::with_capacity(iterations);
    let mut response_bytes = 0;
    for _ in 0..iterations {
        let sample = request(app, spec).await?;
        latencies.push(sample.latency_ms);
        response_bytes = response_bytes.max(sample.bytes);
    }
    Ok(operation_metrics(&latencies, response_bytes, 1))
}

async fn measure_concurrent(
    app: &Router,
    left: &RequestSpec,
    right: &RequestSpec,
    iterations: usize,
) -> Result<OperationMetrics> {
    let mut latencies = Vec::with_capacity(iterations);
    let mut response_bytes = 0;
    for _ in 0..iterations {
        let started = Instant::now();
        let (left, right) = tokio::join!(request(app, left), request(app, right));
        let left = left?;
        let right = right?;
        latencies.push(elapsed_ms(started));
        response_bytes = response_bytes.max(left.bytes.max(right.bytes));
    }
    Ok(operation_metrics(&latencies, response_bytes, 2))
}

fn operation_metrics(
    latencies: &[u64],
    response_bytes: usize,
    requests: usize,
) -> OperationMetrics {
    OperationMetrics {
        samples: latencies.len(),
        latency_p50_ms: percentile(latencies, 50),
        latency_p95_ms: percentile(latencies, 95),
        latency_max_ms: latencies.iter().copied().max().unwrap_or_default(),
        response_bytes_max: response_bytes,
        requests_per_action: requests,
    }
}

async fn request(app: &Router, spec: &RequestSpec) -> Result<ResponseSample> {
    let sample = request_allow_status(app, spec).await?;
    if !sample.status.is_success() {
        bail!(
            "knowledge evaluation request {} returned {}",
            spec.uri,
            sample.status
        );
    }
    Ok(sample)
}

async fn request_allow_status(app: &Router, spec: &RequestSpec) -> Result<ResponseSample> {
    let started = Instant::now();
    let body = spec
        .body
        .as_ref()
        .map(serde_json::to_vec)
        .transpose()?
        .map(Body::from)
        .unwrap_or_else(Body::empty);
    let mut builder = Request::builder()
        .method(spec.method.clone())
        .uri(&spec.uri)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"));
    if spec.body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let response = app
        .clone()
        .oneshot(builder.body(body).context("build evaluation request")?)
        .await
        .context("execute evaluation request")?;
    let status = response.status();
    let body = to_bytes(response.into_body(), MAX_RESPONSE_CAPTURE_BYTES)
        .await
        .context("read evaluation response")?;
    let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    Ok(ResponseSample {
        status,
        bytes: body.len(),
        value,
        latency_ms: elapsed_ms(started),
    })
}

fn get(uri: &str) -> RequestSpec {
    RequestSpec {
        method: Method::GET,
        uri: uri.into(),
        body: None,
    }
}

fn post(uri: &str, body: Value) -> RequestSpec {
    RequestSpec {
        method: Method::POST,
        uri: uri.into(),
        body: Some(body),
    }
}

fn relationship_metrics(
    graph: &Value,
    records: &HashMap<String, SyntheticRecord>,
) -> Result<RelationshipMetrics> {
    let nodes = graph["nodes"]
        .as_array()
        .context("graph evaluation nodes are missing")?;
    let edges = graph["edges"]
        .as_array()
        .context("graph evaluation edges are missing")?;
    let node_labels = nodes
        .iter()
        .filter_map(|node| {
            Some((
                node["id"].as_str()?.to_string(),
                node["label"].as_str()?.to_string(),
            ))
        })
        .collect::<HashMap<_, _>>();
    let node_records = nodes
        .iter()
        .filter_map(|node| {
            Some((
                node["id"].as_str()?.to_string(),
                node["canonical_record_id"].as_str()?.to_string(),
            ))
        })
        .collect::<HashMap<_, _>>();
    let mut valid = 0;
    let mut provenance = 0;
    let mut false_inferences = 0;
    let mut observed = BTreeSet::new();
    let mut invalid_edge_kinds = Vec::new();
    for edge in edges {
        let kind: EdgeKind = serde_json::from_value(edge["kind"].clone())?;
        observed.insert(kind);
        false_inferences += usize::from(edge["origin"] == "inferred");
        let ids = edge["support"]["record_ids"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let invalidation = edge["support"]["invalidation_keys"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !ids.is_empty()
            && !invalidation.is_empty()
            && ids.iter().all(|id| {
                id.as_str().is_some_and(|id| {
                    invalidation
                        .iter()
                        .any(|key| key.as_str().is_some_and(|key| key.contains(id)))
                })
            })
        {
            provenance += 1;
        }
        let record_ids = ids.iter().filter_map(Value::as_str).collect::<Vec<_>>();
        let source_record = edge["source"]
            .as_str()
            .and_then(|node| node_records.get(node))
            .and_then(|id| records.get(id));
        let target_record = edge["target"]
            .as_str()
            .and_then(|node| node_records.get(node))
            .and_then(|id| records.get(id));
        let edge_valid =
            match kind {
                EdgeKind::Contains => record_ids.iter().all(|id| records.contains_key(*id)),
                EdgeKind::References => {
                    source_record
                        .zip(target_record)
                        .is_some_and(|(source, target)| {
                            source.project == target.project
                                && source.references.contains(&target.source_id)
                        })
                }
                EdgeKind::Backlink => {
                    source_record
                        .zip(target_record)
                        .is_some_and(|(target, source)| {
                            source.project == target.project
                                && source.references.contains(&target.source_id)
                        })
                }
                EdgeKind::SameThread => {
                    source_record
                        .zip(target_record)
                        .is_some_and(|(left, right)| {
                            left.project == right.project && left.thread == right.thread
                        })
                }
                EdgeKind::AuthoredBy => source_record.is_some_and(|record| {
                    node_labels
                        .get(edge["target"].as_str().unwrap_or_default())
                        .is_some_and(|label| record.authors.contains(label))
                }),
                EdgeKind::Mentions => source_record.is_some_and(|record| {
                    node_labels
                        .get(edge["target"].as_str().unwrap_or_default())
                        .is_some_and(|label| record.entities.contains(label))
                }),
                EdgeKind::Temporal | EdgeKind::Nearby => source_record
                    .zip(target_record)
                    .is_some_and(|(left, right)| {
                        left.project == right.project && left.source == right.source
                    }),
                _ => false,
            };
        valid += usize::from(edge_valid);
        if !edge_valid {
            invalid_edge_kinds.push(kind);
        }
    }
    let expected = BTreeSet::from([
        EdgeKind::Contains,
        EdgeKind::References,
        EdgeKind::Backlink,
        EdgeKind::Nearby,
        EdgeKind::SameThread,
        EdgeKind::AuthoredBy,
        EdgeKind::Mentions,
        EdgeKind::Temporal,
    ]);
    Ok(RelationshipMetrics {
        evaluated_edges: edges.len(),
        valid_edges: valid,
        edge_precision: ratio(valid, edges.len()),
        expected_edge_kinds: expected.iter().copied().collect(),
        observed_edge_kinds: observed.iter().copied().collect(),
        edge_coverage: ratio(expected.intersection(&observed).count(), expected.len()),
        provenance_completeness: ratio(provenance, edges.len()),
        false_inferences,
        invalid_edge_kinds,
    })
}

fn count_acl_leaks(status: &Value, list: &Value, search: &Value, graph: &Value) -> usize {
    let mut leaks = 0;
    leaks += status["workspaces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|workspace| workspace["id"] != "workspace-00")
        .count();
    leaks += list["documents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|document| document["project"] != "workspace-00")
        .count();
    leaks += search
        .as_array()
        .into_iter()
        .flatten()
        .filter(|evidence| {
            !evidence["source"]
                .as_str()
                .is_some_and(|source| source.starts_with("source-w00-"))
        })
        .count();
    leaks += graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|node| node["project"] != "workspace-00")
        .count();
    leaks
}

#[allow(clippy::too_many_arguments)]
fn evaluate_failures(
    thresholds: &KnowledgeThresholds,
    performance: &BTreeMap<String, OperationMetrics>,
    resources: &ResourceMetrics,
    relationships: &RelationshipMetrics,
    controls: &ControlMetrics,
    safety: &SafetyMetrics,
    release: &ReleaseEvaluation,
) -> Vec<String> {
    let mut failures = Vec::new();
    for (name, maximum) in [
        ("startup-cold-reopen", thresholds.max_startup_p95_ms),
        ("status-workspace-source-tree", thresholds.max_status_p95_ms),
        ("document-list", thresholds.max_document_list_p95_ms),
        ("search-results", thresholds.max_search_p95_ms),
        ("document-view", thresholds.max_document_view_p95_ms),
        ("graph-open", thresholds.max_graph_open_p95_ms),
        ("graph-focus-expansion", thresholds.max_graph_focus_p95_ms),
        ("graph-filter", thresholds.max_graph_filter_p95_ms),
        (
            "concurrent-search-graph",
            thresholds.max_concurrent_search_graph_p95_ms,
        ),
    ] {
        if performance
            .get(name)
            .is_none_or(|value| value.latency_p95_ms > maximum)
        {
            failures.push(format!("{name} exceeded its p95 latency budget"));
        }
    }
    if performance
        .values()
        .any(|value| value.response_bytes_max > thresholds.max_response_bytes)
    {
        failures.push("response byte budget exceeded".into());
    }
    if resources.index_bytes > thresholds.max_index_bytes {
        failures.push("index byte budget exceeded".into());
    }
    if resources
        .peak_rss_bytes
        .is_some_and(|value| value > thresholds.max_peak_rss_bytes)
    {
        failures.push("peak RSS budget exceeded".into());
    }
    if resources.working_set_estimate_bytes > thresholds.max_working_set_estimate_bytes {
        failures.push("working-set estimate budget exceeded".into());
    }
    if resources.total_wall_ms > thresholds.max_total_wall_ms {
        failures.push("total wall-clock budget exceeded".into());
    }
    if resources
        .total_cpu_ms
        .is_some_and(|value| value > thresholds.max_total_cpu_ms)
    {
        failures.push("total CPU budget exceeded".into());
    }
    if relationships.edge_precision < thresholds.min_edge_precision {
        failures.push("edge precision threshold failed".into());
    }
    if relationships.edge_coverage < thresholds.min_edge_coverage {
        failures.push("edge coverage threshold failed".into());
    }
    if relationships.provenance_completeness < thresholds.min_provenance_completeness {
        failures.push("relationship provenance threshold failed".into());
    }
    if relationships.false_inferences > thresholds.max_false_inferences {
        failures.push("false-inference threshold failed".into());
    }
    if safety.acl_leaks > thresholds.max_acl_leaks {
        failures.push("ACL isolation threshold failed".into());
    }
    if safety.invalidation_failures > thresholds.max_invalidation_failures {
        failures.push("relationship invalidation threshold failed".into());
    }
    if !safety.response_limits_respected {
        failures.push("graph response limits were not respected".into());
    }
    if !controls.search_control_passed || !controls.exact_document_control_passed {
        failures.push("search or exact-document control failed without graph".into());
    }
    if !release.independent_edge_disable_passed {
        failures.push("independent edge disable changed canonical access".into());
    }
    failures
}

fn directory_bytes(path: &std::path::Path) -> Result<u64> {
    let mut total = 0_u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

fn stable_id(source: &str, source_id: &str) -> String {
    Sha256::digest(format!("{source}\0{source_id}").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn percentile(values: &[u64], percentile: usize) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut values = values.to_vec();
    values.sort_unstable();
    let rank = percentile.saturating_mul(values.len()).div_ceil(100).max(1);
    values[rank.saturating_sub(1).min(values.len() - 1)]
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

#[cfg(unix)]
fn process_cpu_ms() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    let micros = |value: libc::timeval| {
        (value.tv_sec as u64)
            .saturating_mul(1_000_000)
            .saturating_add(value.tv_usec as u64)
    };
    Some(micros(usage.ru_utime).saturating_add(micros(usage.ru_stime)) / 1000)
}

#[cfg(not(unix))]
fn process_cpu_ms() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
fn peak_rss_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    (unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0)
        .then(|| unsafe { usage.assume_init().ru_maxrss as u64 })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn peak_rss_bytes() -> Option<u64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    (unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0)
        .then(|| (unsafe { usage.assume_init().ru_maxrss as u64 }).saturating_mul(1024))
}

#[cfg(not(unix))]
fn peak_rss_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_fixture_is_bounded_and_complete() {
        let fixture: KnowledgeEvaluationFixture =
            serde_json::from_str(include_str!("../eval/knowledge-graph-v1.json")).unwrap();
        validate_fixture(&fixture).unwrap();
        assert_eq!(fixture.corpus.workspaces, 25);
        assert_eq!(
            fixture.release.enabled_by_default.len() + fixture.release.disabled_by_default.len(),
            EdgeKind::ALL.len()
        );
        assert!(
            !fixture
                .release
                .enabled_by_default
                .contains(&EdgeKind::SemanticallyRelated)
        );
    }

    #[test]
    fn percentile_uses_nearest_rank() {
        assert_eq!(percentile(&[1, 2, 3, 4, 5], 50), 3);
        assert_eq!(percentile(&[1, 2, 3, 4, 5], 95), 5);
    }
}
