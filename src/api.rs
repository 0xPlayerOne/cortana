use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Extension, Path as AxumPath, Query as AxumQuery, State},
    http::{HeaderValue, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::{
    answer::{AnswerEngine, AnswerRequest, AnswerResponse, QueryRuntimeStatus},
    auth::{
        ADMIN_SCOPE, AuthPolicy, MEMORY_SCOPE, Principal, QUERY_SCOPE, STATUS_SCOPE, acl_allows,
    },
    config::{Config, WorkspaceConfig},
    context::{self as context_bundle, ContextBundle},
    derived::{DerivedMemoryResponse, derive_authorized_memory},
    embed::Embedder,
    memory::MemoryInput,
    observation::ObservationCandidateInput,
    retrieval,
    source_status::{self, ConfiguredSourceStatus},
    store::{AuditEvent, DocumentCursor, DocumentSummary, Store, StoreStats},
};

#[cfg(test)]
use crate::model::Evidence;

const MAX_DOCUMENT_CONTENT_BYTES: usize = 2 * 1024 * 1024;
const MAX_DOCUMENT_SCOPE_LENGTH: usize = 256;
const MAX_DOCUMENT_QUERY_LENGTH: usize = 256;
const MAX_DOCUMENT_ID_LENGTH: usize = 128;
const READY_EMBEDDING_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const READY_STORE_STATS_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub embedder: Arc<dyn Embedder>,
    metrics: Arc<RuntimeMetrics>,
    auth: Arc<RwLock<AuthRuntime>>,
    ingestion: Arc<IngestionStatus>,
    workspaces: Arc<Vec<WorkspaceConfig>>,
    answer: AnswerEngine,
    retrieval_tuning: retrieval::RetrievalTuning,
    memory_defaults: crate::memory::MemoryDefaults,
    audit_max_events: usize,
    auth_config_path: Option<std::path::PathBuf>,
    status_stats_cache: Arc<Mutex<HashMap<String, CachedStatusStats>>>,
}

#[derive(Clone)]
struct CachedStatusStats {
    captured_at: Instant,
    stats: StoreStats,
}

#[derive(Clone)]
struct AuthRuntime {
    policy: AuthPolicy,
    /// A remote listener must never be left without a bearer policy after a
    /// reload. Loopback-only services may use local-owner mode.
    remote_listener: bool,
}

impl AuthRuntime {
    fn new(policy: AuthPolicy, remote_listener: bool) -> Self {
        Self {
            policy,
            remote_listener,
        }
    }
}

impl AppState {
    pub fn new(store: Store, embedder: Arc<dyn Embedder>) -> Self {
        let answer = AnswerEngine::new(
            store.clone(),
            embedder.clone(),
            None,
            crate::config::QueryConfig::default(),
        );
        Self {
            store,
            embedder,
            metrics: Arc::new(RuntimeMetrics::new()),
            auth: Arc::new(RwLock::new(AuthRuntime::new(AuthPolicy::default(), false))),
            ingestion: Arc::new(IngestionStatus::default()),
            workspaces: Arc::new(Vec::new()),
            answer,
            retrieval_tuning: retrieval::RetrievalTuning::default(),
            memory_defaults: crate::memory::MemoryDefaults::default(),
            audit_max_events: crate::config::AuthConfig::default().audit_max_events,
            auth_config_path: None,
            status_stats_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_config(mut self, config: &Config, scheduled: bool) -> Self {
        self.ingestion = Arc::new(IngestionStatus::from_config(config, scheduled));
        self.workspaces = Arc::new(config.workspaces.clone());
        self.memory_defaults = crate::memory::MemoryDefaults {
            confidence: config.memory.default_confidence,
            importance: config.memory.default_importance,
        };
        self.audit_max_events = config.auth.audit_max_events;
        self.retrieval_tuning = config.query.retrieval_tuning();
        self
    }

    pub fn with_answer_engine(mut self, answer: AnswerEngine) -> Self {
        self.retrieval_tuning = answer.retrieval_tuning();
        self.answer = answer;
        self
    }

    pub fn with_memory_defaults(mut self, confidence: f32, importance: f32) -> Self {
        self.memory_defaults = crate::memory::MemoryDefaults {
            confidence,
            importance,
        };
        self
    }

    pub fn with_auth_config_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.auth_config_path = Some(path.into());
        self
    }

    pub fn auth_policy(&self) -> AuthPolicy {
        self.auth_snapshot().policy
    }

    pub fn with_auth_policy(mut self, auth: AuthPolicy) -> Self {
        self.auth = Arc::new(RwLock::new(AuthRuntime::new(auth, false)));
        self
    }

    pub fn with_auth_policy_for_listener(
        mut self,
        auth: AuthPolicy,
        remote_listener: bool,
    ) -> Self {
        self.auth = Arc::new(RwLock::new(AuthRuntime::new(auth, remote_listener)));
        self
    }

    fn auth_snapshot(&self) -> AuthRuntime {
        self.auth
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn cache_status_stats(&self, key: String, stats: StoreStats) {
        let mut cache = self
            .status_stats_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // A bounded cache keeps a rotating set of scoped principals from
        // becoming an unbounded memory consumer. The oldest entry is not
        // required for correctness; any entry is safe to evict because a
        // cache miss still fails closed when a fresh probe is unavailable.
        if cache.len() >= 32 && !cache.contains_key(&key) {
            if let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, value)| value.captured_at)
                .map(|(key, _)| key.clone())
            {
                cache.remove(&oldest_key);
            }
        }
        cache.insert(
            key,
            CachedStatusStats {
                captured_at: Instant::now(),
                stats,
            },
        );
    }

    fn cached_status_stats(&self, key: &str) -> Option<CachedStatusStats> {
        self.status_stats_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(key)
            .cloned()
    }

    fn replace_auth_policy(&self, policy: AuthPolicy) -> Result<()> {
        let mut runtime = self
            .auth
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        anyhow::ensure!(
            !runtime.remote_listener || policy.requires_token(),
            "remote listeners must retain at least one configured bearer principal"
        );
        runtime.policy = policy;
        Ok(())
    }

    fn set_remote_listener(&self, remote_listener: bool) -> Result<()> {
        let mut runtime = self
            .auth
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        anyhow::ensure!(
            !remote_listener || runtime.policy.requires_token(),
            "remote listeners must retain at least one configured bearer principal"
        );
        runtime.remote_listener = remote_listener;
        Ok(())
    }
}

struct RuntimeMetrics {
    started: Instant,
    searches: AtomicU64,
    contexts: AtomicU64,
    answers: AtomicU64,
    errors: AtomicU64,
    retrieval_fallbacks: AtomicU64,
    principals: Mutex<HashMap<String, PrincipalMetrics>>,
}

#[derive(Clone, Copy, Default)]
struct PrincipalMetrics {
    searches: u64,
    contexts: u64,
    answers: u64,
    errors: u64,
    retrieval_fallbacks: u64,
}

impl RuntimeMetrics {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            searches: AtomicU64::new(0),
            contexts: AtomicU64::new(0),
            answers: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            retrieval_fallbacks: AtomicU64::new(0),
            principals: Mutex::new(HashMap::new()),
        }
    }

    fn uptime_seconds(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    fn record(&self, principal: &Principal, metric: PrincipalMetric) {
        match metric {
            PrincipalMetric::Search => self.searches.fetch_add(1, Ordering::Relaxed),
            PrincipalMetric::Context => self.contexts.fetch_add(1, Ordering::Relaxed),
            PrincipalMetric::Answer => self.answers.fetch_add(1, Ordering::Relaxed),
            PrincipalMetric::Error => self.errors.fetch_add(1, Ordering::Relaxed),
            PrincipalMetric::RetrievalFallback => {
                self.retrieval_fallbacks.fetch_add(1, Ordering::Relaxed)
            }
        };
        let mut principals = self
            .principals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let counters = principals.entry(principal.name.clone()).or_default();
        match metric {
            PrincipalMetric::Search => counters.searches = counters.searches.saturating_add(1),
            PrincipalMetric::Context => counters.contexts = counters.contexts.saturating_add(1),
            PrincipalMetric::Answer => counters.answers = counters.answers.saturating_add(1),
            PrincipalMetric::Error => counters.errors = counters.errors.saturating_add(1),
            PrincipalMetric::RetrievalFallback => {
                counters.retrieval_fallbacks = counters.retrieval_fallbacks.saturating_add(1)
            }
        }
    }

    fn counters_for(&self, principal: &Principal, owner: bool) -> PrincipalMetrics {
        if owner {
            return PrincipalMetrics {
                searches: self.searches.load(Ordering::Relaxed),
                contexts: self.contexts.load(Ordering::Relaxed),
                answers: self.answers.load(Ordering::Relaxed),
                errors: self.errors.load(Ordering::Relaxed),
                retrieval_fallbacks: self.retrieval_fallbacks.load(Ordering::Relaxed),
            };
        }
        self.principals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&principal.name)
            .copied()
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy)]
enum PrincipalMetric {
    Search,
    Context,
    Answer,
    Error,
    RetrievalFallback,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchRequest {
    query: String,
    project: Option<String>,
    source: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextRequest {
    query: String,
    project: Option<String>,
    source: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default = "default_context_tokens")]
    max_tokens: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryRememberRequest {
    kind: String,
    content_type: Option<String>,
    retention_tier: Option<String>,
    scope: Option<String>,
    project: String,
    title: String,
    content: String,
    source: Option<String>,
    source_id: Option<String>,
    dedupe_key: Option<String>,
    confidence: Option<f32>,
    importance: Option<f32>,
    acl: Option<Vec<String>>,
    provenance: Option<serde_json::Value>,
    supersedes_id: Option<String>,
    valid_until: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryRecallRequest {
    query: String,
    project: Option<String>,
    kind: Option<String>,
    content_type: Option<String>,
    retention_tier: Option<String>,
    scope: Option<String>,
    #[serde(default = "default_memory_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryExportParams {
    project: Option<String>,
    kind: Option<String>,
    content_type: Option<String>,
    retention_tier: Option<String>,
    scope: Option<String>,
    #[serde(default = "default_memory_export_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DerivedMemoryParams {
    project: Option<String>,
    #[serde(default = "default_derived_memory_limit")]
    limit: usize,
}

fn default_derived_memory_limit() -> usize {
    64
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryForgetRequest {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryCandidateRequest {
    observation_kind: String,
    content_type: String,
    retention_tier: String,
    scope: String,
    project: String,
    title: String,
    content: String,
    source: String,
    source_id: String,
    dedupe_key: Option<String>,
    confidence: f32,
    importance: f32,
    sensitivity: String,
    acl: Option<Vec<String>>,
    provenance: serde_json::Value,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct MemoryCandidateListParams {
    project: Option<String>,
    observation_kind: Option<String>,
    scope: Option<String>,
    #[serde(default = "default_memory_candidate_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryConsolidationRequest {
    policy: crate::consolidation::ConsolidationPolicy,
    #[serde(default)]
    explicit_approval: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryCandidateEditRequest {
    title: String,
    content: String,
}

fn default_memory_candidate_limit() -> usize {
    100
}

#[derive(Debug, Serialize)]
struct MemoryForgetResponse {
    id: String,
    forgotten: bool,
}

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Serialize)]
struct Status {
    status: &'static str,
    #[serde(skip_serializing_if = "is_false")]
    stats_stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stats_age_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stats_warning: Option<&'static str>,
    uptime_seconds: u64,
    searches_total: u64,
    contexts_total: u64,
    answers_total: u64,
    errors_total: u64,
    retrieval_fallbacks_total: u64,
    query: QueryRuntimeStatus,
    ingestion: IngestionStatus,
    workspaces: Vec<WorkspaceConfig>,
    memory: crate::memory::MemoryStats,
    #[serde(flatten)]
    stats: StoreStats,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DocumentListParams {
    project: Option<String>,
    source: Option<String>,
    query: Option<String>,
    cursor: Option<String>,
    #[serde(default = "default_document_limit")]
    limit: usize,
    #[serde(default)]
    include_derived: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EncodedDocumentCursor {
    updated_at: String,
    id: String,
}

#[derive(Debug, Serialize)]
struct DocumentListResponse {
    documents: Vec<DocumentSummary>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct GraphNode {
    id: String,
    kind: &'static str,
    label: String,
    project: String,
    source: Option<String>,
    document_id: Option<String>,
    #[serde(flatten)]
    derived: Option<GraphDerivedMetadata>,
}

#[derive(Debug, Serialize)]
struct GraphDerivedMetadata {
    contract_version: String,
    derivation_version: String,
    memory_revision: u64,
    supporting_memory_ids: Vec<String>,
    contradicting_memory_ids: Vec<String>,
    citation_authority: bool,
}

#[derive(Debug, Serialize)]
struct GraphEdge {
    source: String,
    target: String,
    kind: &'static str,
}

#[derive(Debug, Serialize)]
struct GraphResponse {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct IngestionStatus {
    #[serde(skip)]
    data_dir: std::path::PathBuf,
    mode: &'static str,
    scheduled: bool,
    max_documents_per_source: usize,
    max_bytes_per_source: u64,
    max_duration_seconds: u64,
    request_concurrency: usize,
    validation_max_age_hours: u64,
    sync_freshness_hours: u64,
    validation_state_error: Option<String>,
    configured_sources: Vec<ConfiguredSourceStatus>,
    #[serde(skip)]
    validation_fingerprints: BTreeMap<String, String>,
}

impl Default for IngestionStatus {
    fn default() -> Self {
        Self::from_config(&Config::default(), false)
    }
}

impl IngestionStatus {
    fn from_config(config: &Config, scheduled: bool) -> Self {
        let configured_sources = config
            .sources
            .iter()
            .map(|source| source_status::configured_source_status(config, source))
            .collect();
        let validation_fingerprints = source_status::validation_fingerprints(config);
        Self {
            data_dir: config.data_dir.clone(),
            mode: if scheduled { "scheduled" } else { "manual" },
            scheduled,
            max_documents_per_source: config.ingestion.max_documents_per_source,
            max_bytes_per_source: config.ingestion.max_bytes_per_source,
            max_duration_seconds: config.ingestion.max_duration_seconds,
            request_concurrency: config.ingestion.request_concurrency,
            validation_max_age_hours: config.ingestion.validation_max_age_hours,
            sync_freshness_hours: config.ingestion.sync_freshness_hours,
            validation_state_error: None,
            configured_sources,
            validation_fingerprints,
        }
        .refreshed()
    }

    fn refreshed(&self) -> Self {
        let mut status = self.clone();
        match source_status::refresh_source_validations(
            &mut status.configured_sources,
            &self.data_dir,
            self.validation_max_age_hours,
            &self.validation_fingerprints,
        ) {
            Ok(()) => status.validation_state_error = None,
            Err(message) => {
                tracing::warn!(%message, "failed to load source validation state");
                status.validation_state_error = Some(message);
                for source in &mut status.configured_sources {
                    source.validation = None;
                }
            }
        }
        status
    }

    fn visible_to(&self, principal: &Principal) -> Self {
        let acl = principal.acl_labels();
        let mut status = self.clone();
        if principal.is_owner() {
            return status;
        }
        status
            .configured_sources
            .retain(|source| acl_allows(&source.acl, &acl));
        status
    }

    /// Canonical (source, project) keys of the ACL-visible configured sources.
    /// Sync runs are recorded under these keys, so the scoped stats query can
    /// keep surfacing failed/running/budget-exceeded health for an authorized
    /// source that has not indexed any documents yet.
    fn visible_sync_source_keys(&self) -> HashSet<(String, String)> {
        self.configured_sources
            .iter()
            .map(|source| (source.source.clone(), source.project.clone()))
            .collect()
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { Json(Health { status: "ok" }) }))
        .route("/readyz", get(ready))
        .route("/metrics", get(metrics))
        .route("/v1/status", get(status))
        .route("/v1/documents", get(list_documents))
        .route("/v1/documents/{id}", get(document))
        .route("/v1/graph", get(graph))
        .route("/v1/search", post(search))
        .route("/v1/context", post(context))
        .route("/v1/memory", post(remember_memory))
        .route("/v1/memory/recall", post(recall_memories))
        .route("/v1/memory/forget", post(forget_memory))
        .route("/v1/memory/export", get(export_memories))
        .route("/v1/memory/derived", get(derived_memories))
        .route("/v1/memory/reflect", post(reflect_memory))
        .route(
            "/v1/memory/candidates",
            post(propose_memory_candidate).get(list_memory_candidates),
        )
        .route(
            "/v1/memory/candidates/export",
            get(export_memory_candidates),
        )
        .route(
            "/v1/memory/candidates/{id}/cancel",
            post(cancel_memory_candidate),
        )
        .route(
            "/v1/memory/candidates/{id}/redact",
            post(redact_memory_candidate),
        )
        .route(
            "/v1/memory/candidates/{id}/classify",
            post(classify_memory_candidate),
        )
        .route(
            "/v1/memory/candidates/{id}/consolidate",
            post(consolidate_memory_candidate),
        )
        .route(
            "/v1/memory/candidates/{id}/edit",
            post(edit_memory_candidate),
        )
        .route(
            "/v1/memory/consolidation/pause",
            post(pause_memory_consolidation),
        )
        .route(
            "/v1/memory/consolidation/resume",
            post(resume_memory_consolidation),
        )
        .route("/v1/answer", post(answer))
        .route("/v1/audit", get(audit_events))
        .route("/v1/auth/reload", post(reload_auth))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(60),
        ))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(middleware::from_fn_with_state(state.clone(), authorize))
        .with_state(state)
}

async fn authorize(
    State(state): State<AppState>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let started = Instant::now();
    let path = request.uri().path();
    let auth = state.auth_snapshot();
    // Liveness is intentionally public so local service managers can probe it.
    // Readiness performs bounded store/provider work and must not be an
    // unauthenticated remote probe; `remote_listener` is carried with the
    // policy snapshot so loopback Desktop checks remain frictionless.
    if path == "/healthz" || (path == "/readyz" && !auth.remote_listener) {
        return next.run(request).await;
    }
    let principal = if auth.policy.requires_token() {
        request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .and_then(|token| auth.policy.authenticate(token))
    } else {
        Some(Principal::local("local-http"))
    };
    let Some(principal) = principal else {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer realm=\"cortana\"")],
            "authorization required",
        )
            .into_response();
    };
    let required_scope = match path {
        "/metrics" | "/v1/audit" | "/v1/auth/reload" => ADMIN_SCOPE,
        "/v1/memory"
        | "/v1/memory/recall"
        | "/v1/memory/forget"
        | "/v1/memory/export"
        | "/v1/memory/derived"
        | "/v1/memory/reflect"
        | "/v1/memory/candidates"
        | "/v1/memory/consolidation/pause"
        | "/v1/memory/consolidation/resume" => MEMORY_SCOPE,
        path if path.starts_with("/v1/memory/candidates/") => MEMORY_SCOPE,
        "/v1/status" | "/readyz" => STATUS_SCOPE,
        _ => QUERY_SCOPE,
    };
    if !principal.has_scope(required_scope) {
        if path.ends_with("/consolidate") && path.starts_with("/v1/memory/candidates/") {
            record_audit(
                &state,
                &principal,
                "memory.candidate.consolidate",
                None,
                None,
                "forbidden",
                None,
                started,
            );
        }
        return StatusCode::FORBIDDEN.into_response();
    }
    request.extensions_mut().insert(principal);
    next.run(request).await
}

async fn list_documents(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    AxumQuery(params): AxumQuery<DocumentListParams>,
) -> Result<Json<DocumentListResponse>, (StatusCode, String)> {
    validate_document_scope("project", params.project.as_deref())?;
    validate_document_scope("source", params.source.as_deref())?;
    validate_document_query(params.query.as_deref())?;
    let cursor = params
        .cursor
        .as_deref()
        .map(decode_document_cursor)
        .transpose()?;
    let started = Instant::now();
    let acl = principal.visible_acl();
    let result = state.store.list_documents_scoped(
        params.project.as_deref(),
        params.source.as_deref(),
        params.query.as_deref(),
        cursor.as_ref(),
        params.limit.clamp(1, 100),
        &acl,
    );
    match result {
        Ok(page) => {
            let next_cursor = if page.has_more {
                page.documents
                    .last()
                    .map(encode_document_cursor)
                    .transpose()?
            } else {
                None
            };
            record_audit(
                &state,
                &principal,
                "documents.list",
                params.project.as_deref(),
                params.source.as_deref(),
                "succeeded",
                Some(page.documents.len()),
                started,
            );
            Ok(Json(DocumentListResponse {
                documents: page.documents,
                next_cursor,
            }))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "documents.list",
                params.project.as_deref(),
                params.source.as_deref(),
                "failed",
                None,
                started,
            );
            state.metrics.record(&principal, PrincipalMetric::Error);
            Err(internal_error(error))
        }
    }
}

#[derive(Debug, Serialize)]
struct AuthReloadResponse {
    reloaded: bool,
    requires_token: bool,
}

async fn reload_auth(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<AuthReloadResponse>, (StatusCode, String)> {
    let started = Instant::now();
    let Some(path) = state.auth_config_path.as_ref() else {
        record_audit(
            &state,
            &principal,
            "auth.reload",
            None,
            None,
            "failed",
            None,
            started,
        );
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "auth reload requires a file-backed configuration".into(),
        ));
    };
    let result = (|| -> Result<AuthPolicy> {
        let mut config = Config::load(Some(path))?;
        config.load_environment()?;
        AuthPolicy::from_config_file_preferred(&config)
    })();
    let policy = match result {
        Ok(policy) => policy,
        Err(_error) => {
            record_audit(
                &state,
                &principal,
                "auth.reload",
                None,
                None,
                "failed",
                None,
                started,
            );
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                "auth policy reload failed validation".into(),
            ));
        }
    };
    let requires_token = policy.requires_token();
    if let Err(error) = state.replace_auth_policy(policy) {
        record_audit(
            &state,
            &principal,
            "auth.reload",
            None,
            None,
            "failed",
            None,
            started,
        );
        return Err((StatusCode::CONFLICT, error.to_string()));
    }
    record_audit(
        &state,
        &principal,
        "auth.reload",
        None,
        None,
        "succeeded",
        None,
        started,
    );
    Ok(Json(AuthReloadResponse {
        reloaded: true,
        requires_token,
    }))
}

async fn document(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<crate::store::DocumentDetail>, (StatusCode, String)> {
    if id.is_empty()
        || id.len() > MAX_DOCUMENT_ID_LENGTH
        || !id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err((StatusCode::BAD_REQUEST, "invalid document id".into()));
    }
    let started = Instant::now();
    let acl = principal.visible_acl();
    match state
        .store
        .document_scoped(&id, &acl, MAX_DOCUMENT_CONTENT_BYTES)
    {
        Ok(Some(document)) => {
            record_audit(
                &state,
                &principal,
                "documents.read",
                Some(&document.summary.project),
                Some(&document.summary.source),
                "succeeded",
                Some(1),
                started,
            );
            Ok(Json(document))
        }
        Ok(None) => {
            record_audit(
                &state,
                &principal,
                "documents.read",
                None,
                None,
                "not_found",
                Some(0),
                started,
            );
            Err((StatusCode::NOT_FOUND, "document not found".into()))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "documents.read",
                None,
                None,
                "failed",
                None,
                started,
            );
            state.metrics.record(&principal, PrincipalMetric::Error);
            Err(internal_error(error))
        }
    }
}

async fn graph(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    AxumQuery(params): AxumQuery<DocumentListParams>,
) -> Result<Json<GraphResponse>, (StatusCode, String)> {
    if params.include_derived && !principal.has_scope(MEMORY_SCOPE) {
        return Err((
            StatusCode::FORBIDDEN,
            "memory scope required for derived graph projections".into(),
        ));
    }
    validate_document_scope("project", params.project.as_deref())?;
    validate_document_scope("source", params.source.as_deref())?;
    validate_document_query(params.query.as_deref())?;
    let cursor = params
        .cursor
        .as_deref()
        .map(decode_document_cursor)
        .transpose()?;
    let started = Instant::now();
    let acl = principal.visible_acl();
    let result = state.store.list_documents_scoped(
        params.project.as_deref(),
        params.source.as_deref(),
        params.query.as_deref(),
        cursor.as_ref(),
        params.limit.clamp(1, 100),
        &acl,
    );
    match result {
        Ok(page) => {
            let next_cursor = if page.has_more {
                page.documents
                    .last()
                    .map(encode_document_cursor)
                    .transpose()?
            } else {
                None
            };
            let mut nodes = Vec::new();
            let mut edges = Vec::new();
            let mut workspaces = BTreeSet::new();
            let mut sources = BTreeSet::new();
            for document in &page.documents {
                let workspace_id = format!("workspace:{}", serde_json::json!([document.project]));
                let source_id = format!(
                    "source:{}",
                    serde_json::json!([document.project, document.source])
                );
                if workspaces.insert(workspace_id.clone()) {
                    nodes.push(GraphNode {
                        id: workspace_id.clone(),
                        kind: "workspace",
                        label: document.project.clone(),
                        project: document.project.clone(),
                        source: None,
                        document_id: None,
                        derived: None,
                    });
                }
                if sources.insert(source_id.clone()) {
                    nodes.push(GraphNode {
                        id: source_id.clone(),
                        kind: "source",
                        label: document.source.clone(),
                        project: document.project.clone(),
                        source: Some(document.source.clone()),
                        document_id: None,
                        derived: None,
                    });
                    edges.push(GraphEdge {
                        source: workspace_id,
                        target: source_id.clone(),
                        kind: "contains",
                    });
                }
                nodes.push(GraphNode {
                    id: format!("document:{}", document.id),
                    kind: "document",
                    label: document.title.clone(),
                    project: document.project.clone(),
                    source: Some(document.source.clone()),
                    document_id: Some(document.id.clone()),
                    derived: None,
                });
                edges.push(GraphEdge {
                    source: source_id,
                    target: format!("document:{}", document.id),
                    kind: "contains",
                });
            }
            if params.include_derived {
                let derived = derive_authorized_memories(
                    &state,
                    &principal,
                    params.project.as_deref(),
                    params.limit,
                )
                .map_err(internal_error)?;
                let mut memory_nodes = BTreeSet::new();
                for representation in derived.representations {
                    let derived_id = representation.id.clone();
                    let supporting_memory_ids = representation.supporting_memory_ids.clone();
                    let contradicting_memory_ids = representation.contradicting_memory_ids.clone();
                    nodes.push(GraphNode {
                        id: derived_id.clone(),
                        kind: match representation.kind {
                            crate::derived::DerivedKind::Experience => "memory-experience",
                            crate::derived::DerivedKind::Observation => "memory-observation",
                            crate::derived::DerivedKind::MentalModel => "memory-mental-model",
                            crate::derived::DerivedKind::Belief => "memory-belief",
                        },
                        label: representation.statement,
                        project: representation.project.clone(),
                        source: None,
                        document_id: None,
                        derived: Some(GraphDerivedMetadata {
                            contract_version: representation.contract_version,
                            derivation_version: representation.provenance.engine_version,
                            memory_revision: representation.memory_revision,
                            supporting_memory_ids,
                            contradicting_memory_ids,
                            citation_authority: representation.citation_authority,
                        }),
                    });
                    for memory_id in representation.supporting_memory_ids {
                        let node_id = format!("memory:{memory_id}");
                        if memory_nodes.insert(node_id.clone()) {
                            nodes.push(GraphNode {
                                id: node_id.clone(),
                                kind: "canonical-memory",
                                label: memory_id,
                                project: representation.project.clone(),
                                source: None,
                                document_id: None,
                                derived: None,
                            });
                        }
                        edges.push(GraphEdge {
                            source: node_id,
                            target: derived_id.clone(),
                            kind: "supports",
                        });
                    }
                    for memory_id in representation.contradicting_memory_ids {
                        let node_id = format!("memory:{memory_id}");
                        if memory_nodes.insert(node_id.clone()) {
                            nodes.push(GraphNode {
                                id: node_id.clone(),
                                kind: "canonical-memory",
                                label: memory_id,
                                project: representation.project.clone(),
                                source: None,
                                document_id: None,
                                derived: None,
                            });
                        }
                        edges.push(GraphEdge {
                            source: node_id,
                            target: derived_id.clone(),
                            kind: "contradicts",
                        });
                    }
                }
                for relation in derived.relations {
                    let relation_id = relation.id.clone();
                    let supporting_memory_ids = relation.supporting_memory_ids.clone();
                    nodes.push(GraphNode {
                        id: relation_id.clone(),
                        kind: "memory-relation",
                        label: format!("{}: {}", relation.predicate, relation.object),
                        project: relation.project.clone(),
                        source: None,
                        document_id: None,
                        derived: Some(GraphDerivedMetadata {
                            contract_version: relation.contract_version,
                            derivation_version: relation.provenance.engine_version,
                            memory_revision: relation.memory_revision,
                            supporting_memory_ids,
                            contradicting_memory_ids: Vec::new(),
                            citation_authority: relation.citation_authority,
                        }),
                    });
                    for memory_id in relation.supporting_memory_ids {
                        let node_id = format!("memory:{memory_id}");
                        if memory_nodes.insert(node_id.clone()) {
                            nodes.push(GraphNode {
                                id: node_id.clone(),
                                kind: "canonical-memory",
                                label: memory_id,
                                project: relation.project.clone(),
                                source: None,
                                document_id: None,
                                derived: None,
                            });
                        }
                        edges.push(GraphEdge {
                            source: node_id,
                            target: relation_id.clone(),
                            kind: "derives",
                        });
                    }
                }
            }
            record_audit(
                &state,
                &principal,
                "graph.read",
                params.project.as_deref(),
                params.source.as_deref(),
                "succeeded",
                Some(page.documents.len()),
                started,
            );
            Ok(Json(GraphResponse {
                nodes,
                edges,
                next_cursor,
            }))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "graph.read",
                params.project.as_deref(),
                params.source.as_deref(),
                "failed",
                None,
                started,
            );
            state.metrics.record(&principal, PrincipalMetric::Error);
            Err(internal_error(error))
        }
    }
}

fn validate_document_scope(name: &str, value: Option<&str>) -> Result<(), (StatusCode, String)> {
    if value.is_some_and(|value| {
        value.is_empty()
            || value.len() > MAX_DOCUMENT_SCOPE_LENGTH
            || value.chars().any(|character| character.is_control())
    }) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{name} must contain 1 to {MAX_DOCUMENT_SCOPE_LENGTH} bytes"),
        ));
    }
    Ok(())
}

fn validate_document_query(value: Option<&str>) -> Result<(), (StatusCode, String)> {
    if value.is_some_and(|value| {
        value.len() > MAX_DOCUMENT_QUERY_LENGTH
            || value.trim().is_empty()
            || value.chars().any(|character| character.is_control())
    }) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("query must contain 1 to {MAX_DOCUMENT_QUERY_LENGTH} bytes"),
        ));
    }
    Ok(())
}

fn decode_document_cursor(value: &str) -> Result<DocumentCursor, (StatusCode, String)> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid document cursor".into()))?;
    if bytes.len() > 512 {
        return Err((StatusCode::BAD_REQUEST, "invalid document cursor".into()));
    }
    let cursor: EncodedDocumentCursor = serde_json::from_slice(&bytes)
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid document cursor".into()))?;
    if cursor.id.is_empty()
        || cursor.id.len() > MAX_DOCUMENT_ID_LENGTH
        || !cursor.id.bytes().all(|byte| byte.is_ascii_hexdigit())
        || cursor.updated_at.len() > 64
        || chrono::DateTime::parse_from_rfc3339(&cursor.updated_at).is_err()
    {
        return Err((StatusCode::BAD_REQUEST, "invalid document cursor".into()));
    }
    Ok(DocumentCursor {
        updated_at: cursor.updated_at,
        id: cursor.id,
    })
}

fn encode_document_cursor(document: &DocumentSummary) -> Result<String, (StatusCode, String)> {
    serde_json::to_vec(&EncodedDocumentCursor {
        updated_at: document.updated_at.clone(),
        id: document.id.clone(),
    })
    .map(|value| URL_SAFE_NO_PAD.encode(value))
    .map_err(|error| internal_error(error.into()))
}

fn default_document_limit() -> usize {
    50
}

async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    ready_with_probe_timeout(state, READY_EMBEDDING_PROBE_TIMEOUT).await
}

async fn ready_with_probe_timeout(state: AppState, probe_timeout: Duration) -> impl IntoResponse {
    ready_with_probe_timeouts(state, READY_STORE_STATS_TIMEOUT, probe_timeout).await
}

async fn ready_with_probe_timeouts(
    state: AppState,
    store_timeout: Duration,
    probe_timeout: Duration,
) -> impl IntoResponse {
    let result = match store_probe_with_timeout(state.store.clone(), store_timeout).await {
        Ok(_) => probe_with_timeout(state.embedder.as_ref(), probe_timeout).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => (StatusCode::OK, Json(Health { status: "ready" })),
        Err(error) => {
            tracing::error!(%error, "readiness check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(Health {
                    status: "unavailable",
                }),
            )
        }
    }
}

async fn store_probe_with_timeout(store: Store, timeout: Duration) -> Result<()> {
    blocking_stats_with_timeout(move || store.probe(), timeout).await
}

async fn store_stats_with_timeout(store: Store, timeout: Duration) -> Result<StoreStats> {
    blocking_stats_with_timeout(move || store.stats(), timeout).await
}

async fn store_stats_scoped_with_timeout(
    store: Store,
    acl: Vec<String>,
    source_keys: HashSet<(String, String)>,
    timeout: Duration,
) -> Result<StoreStats> {
    blocking_stats_with_timeout(move || store.stats_scoped(&acl, &source_keys), timeout).await
}

async fn memory_stats_with_timeout(
    store: Store,
    principal_acl: Option<Vec<String>>,
    timeout: Duration,
) -> Result<crate::memory::MemoryStats> {
    blocking_stats_with_timeout(
        move || match principal_acl {
            Some(acl) => store.memory_stats_scoped(&acl),
            None => store.memory_stats(),
        },
        timeout,
    )
    .await
}

async fn blocking_stats_with_timeout<F, T>(stats: F, timeout: Duration) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let task = tokio::task::spawn_blocking(stats);
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(anyhow!("readiness stats probe worker failed: {error}")),
        Err(_) => Err(anyhow!("readiness stats probe timed out after {timeout:?}")),
    }
}

async fn probe_with_timeout(embedder: &dyn Embedder, timeout: Duration) -> Result<()> {
    match tokio::time::timeout(timeout, embedder.probe()).await {
        Ok(result) => result,
        Err(_) => Err(anyhow!(
            "embedding provider probe timed out after {timeout:?}"
        )),
    }
}

async fn status(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Status>, (StatusCode, String)> {
    let acl = principal.acl_labels();
    let owner = principal.is_owner();
    let ingestion = state.ingestion.refreshed().visible_to(&principal);
    let visible_sync_sources = ingestion.visible_sync_source_keys();
    let cache_key = if owner {
        "owner".to_string()
    } else {
        let mut source_keys = visible_sync_sources.iter().cloned().collect::<Vec<_>>();
        source_keys.sort();
        let fallback_key = format!("scoped:{acl:?}:{source_keys:?}");
        serde_json::to_string(&(acl.clone(), source_keys)).unwrap_or(fallback_key)
    };
    let fresh_stats = if owner {
        store_stats_with_timeout(state.store.clone(), READY_STORE_STATS_TIMEOUT).await
    } else {
        store_stats_scoped_with_timeout(
            state.store.clone(),
            acl,
            visible_sync_sources,
            READY_STORE_STATS_TIMEOUT,
        )
        .await
    };
    let (stats, stats_stale, stats_age_seconds, stats_warning) = match fresh_stats {
        Ok(stats) => {
            state.cache_status_stats(cache_key.clone(), stats.clone());
            (stats, false, None, None)
        }
        Err(error) => match state.cached_status_stats(&cache_key) {
            Some(cached) => (
                cached.stats,
                true,
                Some(cached.captured_at.elapsed().as_secs()),
                Some(
                    "database statistics are temporarily stale; showing the last successful snapshot",
                ),
            ),
            None => return Err(status_stats_error(error)),
        },
    };
    let source_projects = if owner {
        stats
            .sources
            .iter()
            .map(|source| source.project.clone())
            .collect::<Vec<_>>()
    } else {
        let mut projects = ingestion
            .configured_sources
            .iter()
            .map(|source| source.project.clone())
            .collect::<Vec<_>>();
        projects.extend(stats.sources.iter().map(|source| source.project.clone()));
        projects
    };
    let visible_workspaces = if owner {
        state.workspaces.as_ref().clone()
    } else {
        let projects = source_projects
            .iter()
            .map(|project| project.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        state
            .workspaces
            .iter()
            .filter(|workspace| projects.contains(&workspace.id.to_ascii_lowercase()))
            .cloned()
            .collect()
    };
    let workspaces = fallback_workspaces(&visible_workspaces, source_projects);
    let counters = state.metrics.counters_for(&principal, owner);
    // Memory statistics use the same SQLite read mutex as corpus statistics.
    // Keep this synchronous work off the async request worker so a contended
    // database cannot stall /healthz or other control-plane requests.
    let memory = memory_stats_with_timeout(
        state.store.clone(),
        (!owner).then(|| principal.visible_acl()),
        READY_STORE_STATS_TIMEOUT,
    )
    .await
    .unwrap_or_default();
    Ok(Json(Status {
        status: "ok",
        stats_stale,
        stats_age_seconds,
        stats_warning,
        uptime_seconds: state.metrics.uptime_seconds(),
        searches_total: counters.searches,
        contexts_total: counters.contexts,
        answers_total: counters.answers,
        errors_total: counters.errors,
        retrieval_fallbacks_total: counters.retrieval_fallbacks,
        query: state.answer.status(),
        ingestion,
        workspaces,
        memory,
        stats,
    }))
}

fn fallback_workspaces(
    configured: &[WorkspaceConfig],
    source_projects: impl IntoIterator<Item = String>,
) -> Vec<WorkspaceConfig> {
    if !configured.is_empty() {
        // Explicit configuration is authoritative. Never silently hide a
        // workspace (and its source assignment) from the operator UI; only
        // synthesized fallback workspaces are bounded below.
        return configured.to_vec();
    }
    let mut project_ids: BTreeSet<String> = source_projects
        .into_iter()
        .map(|project| project.to_ascii_lowercase())
        .collect();
    let mut workspaces = Vec::new();

    for project in ["work", "personal", "special"] {
        if project_ids.remove(project) {
            workspaces.push(WorkspaceConfig {
                id: project.to_string(),
                name: title_case(project),
                account_label: None,
                color: None,
            });
        }
    }
    for project in project_ids {
        if workspaces.len() >= 3 {
            break;
        }
        workspaces.push(WorkspaceConfig {
            id: project.clone(),
            name: title_case(&project),
            account_label: None,
            color: None,
        });
    }

    if !workspaces.is_empty() {
        workspaces.truncate(3);
        return workspaces;
    }

    ["work", "personal", "special"]
        .into_iter()
        .map(|id| WorkspaceConfig {
            id: id.to_string(),
            name: title_case(id),
            account_label: None,
            color: None,
        })
        .collect()
}

fn title_case(value: &str) -> String {
    let mut characters = value.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + characters.as_str())
        .unwrap_or_default()
}

async fn answer(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<AnswerRequest>,
) -> Result<Json<AnswerResponse>, (StatusCode, String)> {
    validate_retrieval_scope(request.project.as_deref(), request.source.as_deref())?;
    validate_query(&request.query)?;
    let started = Instant::now();
    let project = request.project.clone();
    let source = request.source.clone();
    state.metrics.record(&principal, PrincipalMetric::Answer);
    let visible_acl = principal.visible_acl();
    let memory_acl = principal
        .has_scope(MEMORY_SCOPE)
        .then_some(visible_acl.as_slice());
    let result = if principal.is_owner() && memory_acl.is_some() {
        state
            .answer
            .answer_scoped_with_memory_as_owner(request, &visible_acl)
            .await
    } else {
        state
            .answer
            .answer_scoped_with_memory(request, &visible_acl, memory_acl)
            .await
    };
    let (outcome, count) = match &result {
        Ok(response) => {
            if response.retrieval_degraded {
                state
                    .metrics
                    .record(&principal, PrincipalMetric::RetrievalFallback);
            }
            (
                if response.retrieval_degraded {
                    "degraded"
                } else {
                    "succeeded"
                },
                Some(
                    response
                        .evidence
                        .len()
                        .saturating_add(response.memories.len()),
                ),
            )
        }
        Err(_) => ("failed", None),
    };
    record_audit(
        &state,
        &principal,
        "answer",
        project.as_deref(),
        source.as_deref(),
        outcome,
        count,
        started,
    );
    result.map(Json).map_err(|error| {
        state.metrics.record(&principal, PrincipalMetric::Error);
        internal_error(error)
    })
}

async fn search(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<SearchRequest>,
) -> Result<Response, (StatusCode, String)> {
    validate_retrieval_scope(request.project.as_deref(), request.source.as_deref())?;
    validate_query(&request.query)?;
    let started = Instant::now();
    state.metrics.record(&principal, PrincipalMetric::Search);
    let acl = principal.visible_acl();
    match retrieval::retrieve_scoped_with_status_tuned(
        &state.store,
        &state.embedder,
        &request.query,
        request.project.as_deref(),
        request.source.as_deref(),
        request.limit.min(50),
        &acl,
        state.retrieval_tuning,
    )
    .await
    {
        Ok(retrieval) => {
            if retrieval.degraded() {
                state
                    .metrics
                    .record(&principal, PrincipalMetric::RetrievalFallback);
            }
            let mut response = Json(&retrieval.evidence).into_response();
            response.headers_mut().insert(
                "x-cortana-retrieval-mode",
                HeaderValue::from_static(retrieval.mode.as_str()),
            );
            response.headers_mut().insert(
                "x-cortana-retrieval-degraded",
                HeaderValue::from_static(if retrieval.degraded() {
                    "true"
                } else {
                    "false"
                }),
            );
            response.headers_mut().insert(
                "x-cortana-retrieval-ranking",
                HeaderValue::from_static(retrieval.diagnostics.contract_version),
            );
            for (name, value) in [
                (
                    "x-cortana-retrieval-candidates",
                    retrieval.diagnostics.fused_candidates,
                ),
                (
                    "x-cortana-retrieval-deduplicated",
                    retrieval.diagnostics.deduplicated_candidates,
                ),
                (
                    "x-cortana-retrieval-returned",
                    retrieval.diagnostics.returned,
                ),
            ] {
                if let Ok(value) = HeaderValue::try_from(value.to_string()) {
                    response.headers_mut().insert(name, value);
                }
            }
            record_audit(
                &state,
                &principal,
                "search",
                request.project.as_deref(),
                request.source.as_deref(),
                if retrieval.degraded() {
                    "degraded"
                } else {
                    "succeeded"
                },
                Some(retrieval.evidence.len()),
                started,
            );
            Ok(response)
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "search",
                request.project.as_deref(),
                request.source.as_deref(),
                "failed",
                None,
                started,
            );
            state.metrics.record(&principal, PrincipalMetric::Error);
            Err(internal_error(error))
        }
    }
}

async fn context(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<ContextRequest>,
) -> Result<Json<ContextBundle>, (StatusCode, String)> {
    validate_retrieval_scope(request.project.as_deref(), request.source.as_deref())?;
    validate_query(&request.query)?;
    let started = Instant::now();
    state.metrics.record(&principal, PrincipalMetric::Context);
    let acl = principal.visible_acl();
    let retrieval = match retrieval::retrieve_scoped_with_status_tuned(
        &state.store,
        &state.embedder,
        &request.query,
        request.project.as_deref(),
        request.source.as_deref(),
        request.limit.min(50),
        &acl,
        state.retrieval_tuning,
    )
    .await
    {
        Ok(retrieval) => retrieval,
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "context",
                request.project.as_deref(),
                request.source.as_deref(),
                "failed",
                None,
                started,
            );
            state.metrics.record(&principal, PrincipalMetric::Error);
            return Err(internal_error(error));
        }
    };
    if retrieval.degraded() {
        state
            .metrics
            .record(&principal, PrincipalMetric::RetrievalFallback);
    }
    let memories = if principal.has_scope(MEMORY_SCOPE) {
        let recalled = if principal.is_owner() {
            state.store.recall_memories_as_owner(
                &request.query,
                request.project.as_deref(),
                None,
                request.limit.min(crate::memory::MAX_MEMORY_RECALL_LIMIT),
            )
        } else {
            state.store.recall_memories(
                &request.query,
                request.project.as_deref(),
                None,
                request.limit.min(crate::memory::MAX_MEMORY_RECALL_LIMIT),
                &acl,
            )
        };
        recalled.unwrap_or_else(|error| {
            tracing::warn!(%error, "native memory recall unavailable while building context");
            Vec::new()
        })
    } else {
        Vec::new()
    };
    record_audit(
        &state,
        &principal,
        "context",
        request.project.as_deref(),
        request.source.as_deref(),
        if retrieval.degraded() {
            "degraded"
        } else {
            "succeeded"
        },
        Some(retrieval.evidence.len().saturating_add(memories.len())),
        started,
    );
    let memory_revision = principal
        .has_scope(MEMORY_SCOPE)
        .then(|| state.store.memory_revision())
        .transpose()
        .map_err(internal_error)?;
    let bundle = context_bundle::build_with_retrieval_and_memory(
        &request.query,
        &retrieval.evidence,
        &memories,
        request.max_tokens,
        retrieval.mode.as_str(),
        retrieval.warning.as_deref(),
    )
    .with_metadata(context_bundle::metadata(
        context_bundle::ContextMetadataInput {
            token_budget: request.max_tokens,
            corpus_revision: state.store.corpus_revision().map_err(internal_error)?,
            memory_revision,
            embedding_fingerprint: Some(state.embedder.fingerprint()),
            project: request.project.as_deref(),
            source: request.source.as_deref(),
            acl: &acl,
            retrieval_warning: retrieval.warning.as_deref(),
        },
    ));
    Ok(Json(bundle))
}

async fn remember_memory(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<MemoryRememberRequest>,
) -> Result<Json<crate::memory::MemoryRecord>, (StatusCode, String)> {
    let started = Instant::now();
    if request.project.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "project must not be empty".into()));
    }
    let requested_acl = request.acl.unwrap_or_default();
    let visible_acl = principal.visible_acl();
    let acl = if principal.is_owner() {
        requested_acl
    } else if requested_acl.is_empty() {
        visible_acl.clone()
    } else if requested_acl
        .iter()
        .all(|label| visible_acl.iter().any(|visible| visible == label))
    {
        requested_acl
    } else {
        record_audit(
            &state,
            &principal,
            "memory.remember",
            Some(&request.project),
            None,
            "forbidden",
            None,
            started,
        );
        return Err((
            StatusCode::FORBIDDEN,
            "memory ACL exceeds principal visibility".into(),
        ));
    };
    let axes = crate::memory::MemoryAxes::with_overrides(
        &request.kind,
        request.content_type.as_deref(),
        request.retention_tier.as_deref(),
        request.scope.as_deref(),
    )
    .map_err(|error| (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))?;
    let input = MemoryInput {
        kind: request.kind,
        project: request.project.clone(),
        title: request.title,
        content: request.content,
        source: request.source.unwrap_or_else(|| "agent".into()),
        source_id: request.source_id.unwrap_or_default(),
        dedupe_key: request.dedupe_key,
        confidence: request
            .confidence
            .unwrap_or(state.memory_defaults.confidence),
        importance: request
            .importance
            .unwrap_or(state.memory_defaults.importance),
        acl,
        provenance: request.provenance.unwrap_or_else(|| {
            serde_json::json!({
                "principal": principal.name,
                "interface": "http"
            })
        }),
        supersedes_id: request.supersedes_id,
        valid_until: request.valid_until,
    };
    match state
        .store
        .remember_scoped_with_axes(&input, &visible_acl, principal.is_owner(), axes)
    {
        Ok(memory) => {
            record_audit(
                &state,
                &principal,
                "memory.remember",
                Some(&memory.project),
                Some(&memory.source),
                "succeeded",
                Some(1),
                started,
            );
            Ok(Json(memory))
        }
        Err(error) if crate::memory::is_authorization_error(&error) => {
            record_audit(
                &state,
                &principal,
                "memory.remember",
                Some(&input.project),
                Some(&input.source),
                "forbidden",
                None,
                started,
            );
            Err((StatusCode::FORBIDDEN, "memory ACL denied".into()))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "memory.remember",
                Some(&input.project),
                Some(&input.source),
                "invalid",
                None,
                started,
            );
            Err((StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))
        }
    }
}

async fn recall_memories(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<MemoryRecallRequest>,
) -> Result<Json<Vec<crate::memory::MemorySearchResult>>, (StatusCode, String)> {
    if !principal.has_scope(MEMORY_SCOPE) {
        record_audit(
            &state,
            &principal,
            "memory.recall",
            request.project.as_deref(),
            None,
            "forbidden",
            None,
            Instant::now(),
        );
        return Err((StatusCode::FORBIDDEN, "memory scope required".into()));
    }
    validate_query(&request.query)?;
    validate_retrieval_scope(request.project.as_deref(), None)?;
    let started = Instant::now();
    let recalled = if principal.is_owner() {
        state.store.recall_memories_with_axes_as_owner(
            &request.query,
            request.project.as_deref(),
            request.kind.as_deref(),
            request.content_type.as_deref(),
            request.retention_tier.as_deref(),
            request.scope.as_deref(),
            request
                .limit
                .clamp(1, crate::memory::MAX_MEMORY_RECALL_LIMIT),
        )
    } else {
        state.store.recall_memories_with_axes(
            &request.query,
            request.project.as_deref(),
            request.kind.as_deref(),
            request.content_type.as_deref(),
            request.retention_tier.as_deref(),
            request.scope.as_deref(),
            request
                .limit
                .clamp(1, crate::memory::MAX_MEMORY_RECALL_LIMIT),
            &principal.visible_acl(),
        )
    };
    match recalled {
        Ok(memories) => {
            record_audit(
                &state,
                &principal,
                "memory.recall",
                request.project.as_deref(),
                None,
                "succeeded",
                Some(memories.len()),
                started,
            );
            Ok(Json(memories))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "memory.recall",
                request.project.as_deref(),
                None,
                "failed",
                None,
                started,
            );
            Err(internal_error(error))
        }
    }
}

async fn propose_memory_candidate(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<MemoryCandidateRequest>,
) -> Result<Json<crate::observation::ObservationCandidate>, (StatusCode, String)> {
    let started = Instant::now();
    let requested_acl = request.acl.unwrap_or_default();
    let visible_acl = principal.visible_acl();
    let acl = if principal.is_owner() {
        requested_acl
    } else if requested_acl.is_empty() {
        visible_acl.clone()
    } else if requested_acl
        .iter()
        .all(|label| visible_acl.iter().any(|visible| visible == label))
    {
        requested_acl
    } else {
        record_audit(
            &state,
            &principal,
            "memory.candidate.create",
            Some(&request.project),
            Some(&request.source),
            "forbidden",
            None,
            started,
        );
        return Err((
            StatusCode::FORBIDDEN,
            "candidate ACL exceeds principal visibility".into(),
        ));
    };
    let input = ObservationCandidateInput {
        observation_kind: request.observation_kind,
        content_type: request.content_type,
        retention_tier: request.retention_tier,
        scope: request.scope,
        project: request.project.clone(),
        title: request.title,
        content: request.content,
        source: request.source,
        source_id: request.source_id,
        dedupe_key: request.dedupe_key,
        confidence: request.confidence,
        importance: request.importance,
        sensitivity: request.sensitivity,
        acl,
        provenance: request.provenance,
        expires_at: request.expires_at,
    };
    let result = state.store.propose_memory_candidate(
        &input,
        &principal.name,
        &visible_acl,
        principal.is_owner(),
    );
    match result {
        Ok(candidate) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.create",
                Some(&candidate.project),
                Some(&candidate.source),
                "succeeded",
                Some(1),
                started,
            );
            Ok(Json(candidate))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.create",
                Some(&input.project),
                Some(&input.source),
                "rejected",
                None,
                started,
            );
            Err((StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))
        }
    }
}

async fn list_memory_candidates(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    AxumQuery(params): AxumQuery<MemoryCandidateListParams>,
) -> Result<Json<Vec<crate::store::MemoryCandidateReview>>, (StatusCode, String)> {
    let started = Instant::now();
    match state.store.list_memory_candidate_reviews(
        params.project.as_deref(),
        params.observation_kind.as_deref(),
        params.scope.as_deref(),
        params.limit,
        &principal.name,
        &principal.visible_acl(),
        principal.is_owner(),
    ) {
        Ok(candidates) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.list",
                params.project.as_deref(),
                None,
                "succeeded",
                Some(candidates.len()),
                started,
            );
            Ok(Json(candidates))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.list",
                params.project.as_deref(),
                None,
                "failed",
                None,
                started,
            );
            Err((StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))
        }
    }
}

async fn export_memory_candidates(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    AxumQuery(params): AxumQuery<MemoryCandidateListParams>,
) -> Result<Json<Vec<crate::observation::ObservationCandidate>>, (StatusCode, String)> {
    let started = Instant::now();
    match state.store.export_memory_candidates(
        params.project.as_deref(),
        params.observation_kind.as_deref(),
        params.scope.as_deref(),
        params.limit,
        &principal.name,
        &principal.visible_acl(),
        principal.is_owner(),
    ) {
        Ok(candidates) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.export",
                params.project.as_deref(),
                None,
                "succeeded",
                Some(candidates.len()),
                started,
            );
            Ok(Json(candidates))
        }
        Err(error) => Err((StatusCode::UNPROCESSABLE_ENTITY, error.to_string())),
    }
}

async fn reflect_memory(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<crate::reflection::ReflectRequest>,
) -> Result<Json<crate::reflection::ReflectResponse>, (StatusCode, String)> {
    let started = Instant::now();
    if !principal.has_scope(MEMORY_SCOPE) {
        record_audit(
            &state,
            &principal,
            "memory.reflect",
            request.project.as_deref(),
            request.source.as_deref(),
            "forbidden",
            None,
            started,
        );
        return Err((StatusCode::FORBIDDEN, "memory scope required".into()));
    }
    match crate::reflection::reflect_authorized(
        &state.store,
        &state.embedder,
        &request,
        &principal.visible_acl(),
        principal.is_owner(),
    )
    .await
    {
        Ok(response) => {
            record_audit(
                &state,
                &principal,
                "memory.reflect",
                request.project.as_deref(),
                request.source.as_deref(),
                "succeeded",
                Some(response.metrics.memories_included),
                started,
            );
            Ok(Json(response))
        }
        Err(error) if crate::memory::is_authorization_error(&error) => {
            record_audit(
                &state,
                &principal,
                "memory.reflect",
                request.project.as_deref(),
                request.source.as_deref(),
                "forbidden",
                None,
                started,
            );
            Err((StatusCode::FORBIDDEN, "reflection scope denied".into()))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "memory.reflect",
                request.project.as_deref(),
                request.source.as_deref(),
                "failed",
                None,
                started,
            );
            Err((StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))
        }
    }
}

async fn cancel_memory_candidate(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    update_memory_candidate(&state, &principal, &id, false).await
}

async fn redact_memory_candidate(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    update_memory_candidate(&state, &principal, &id, true).await
}

async fn edit_memory_candidate(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<MemoryCandidateEditRequest>,
) -> Result<Json<crate::observation::ObservationCandidate>, (StatusCode, String)> {
    let started = Instant::now();
    match state.store.edit_memory_candidate_scoped(
        &id,
        &request.title,
        &request.content,
        &principal.name,
        &principal.visible_acl(),
        principal.is_owner(),
    ) {
        Ok(Some(candidate)) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.edit",
                Some(&candidate.project),
                Some(&candidate.source),
                "succeeded",
                Some(1),
                started,
            );
            Ok(Json(candidate))
        }
        Ok(None) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.edit",
                None,
                None,
                "not_found",
                Some(0),
                started,
            );
            Err((StatusCode::NOT_FOUND, "pending candidate not found".into()))
        }
        Err(error)
            if crate::memory::is_authorization_error(&error)
                || error.to_string() == "candidate ACL denied" =>
        {
            record_audit(
                &state,
                &principal,
                "memory.candidate.edit",
                None,
                None,
                "forbidden",
                None,
                started,
            );
            Err((StatusCode::FORBIDDEN, "candidate ACL denied".into()))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.edit",
                None,
                None,
                "failed",
                None,
                started,
            );
            Err((StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))
        }
    }
}

async fn pause_memory_consolidation(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    update_memory_consolidation_control(&state, &principal, true)
}

async fn resume_memory_consolidation(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    update_memory_consolidation_control(&state, &principal, false)
}

fn update_memory_consolidation_control(
    state: &AppState,
    principal: &Principal,
    pause: bool,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let started = Instant::now();
    if !principal.is_owner() {
        record_audit(
            state,
            principal,
            if pause {
                "memory.consolidation.pause"
            } else {
                "memory.consolidation.resume"
            },
            None,
            None,
            "forbidden",
            None,
            started,
        );
        return Err((
            StatusCode::FORBIDDEN,
            "consolidation control requires owner authorization".into(),
        ));
    }
    let changed = if pause {
        state.store.pause_memory_consolidation()
    } else {
        state.store.resume_memory_consolidation()
    }
    .map_err(internal_error)?;
    record_audit(
        state,
        principal,
        if pause {
            "memory.consolidation.pause"
        } else {
            "memory.consolidation.resume"
        },
        None,
        None,
        "succeeded",
        Some(changed),
        started,
    );
    Ok(Json(serde_json::json!({
        "status": if pause { "paused" } else { "resumed" },
        "changed": changed
    })))
}

async fn classify_memory_candidate(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<crate::classification::CandidateClassification>, (StatusCode, String)> {
    let started = Instant::now();
    if !principal.has_scope(MEMORY_SCOPE) {
        record_audit(
            &state,
            &principal,
            "memory.candidate.classify",
            None,
            None,
            "forbidden",
            None,
            started,
        );
        return Err((StatusCode::FORBIDDEN, "memory scope required".into()));
    }
    match state.store.classify_memory_candidate(
        &id,
        &principal.name,
        &principal.visible_acl(),
        principal.is_owner(),
    ) {
        Ok(result) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.classify",
                None,
                None,
                "succeeded",
                Some(1),
                started,
            );
            Ok(Json(result))
        }
        Err(error)
            if crate::memory::is_authorization_error(&error)
                || error.to_string() == "candidate ACL denied" =>
        {
            record_audit(
                &state,
                &principal,
                "memory.candidate.classify",
                None,
                None,
                "forbidden",
                None,
                started,
            );
            Err((StatusCode::FORBIDDEN, "candidate ACL denied".into()))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.classify",
                None,
                None,
                "failed",
                None,
                started,
            );
            Err((StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))
        }
    }
}

async fn consolidate_memory_candidate(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<MemoryConsolidationRequest>,
) -> Result<Json<crate::consolidation::ConsolidationOutcome>, (StatusCode, String)> {
    let started = Instant::now();
    if !principal.has_scope(MEMORY_SCOPE) {
        return Err((StatusCode::FORBIDDEN, "memory scope required".into()));
    }
    match state.store.consolidate_memory_candidate(
        &id,
        &request.policy,
        &principal.name,
        &principal.visible_acl(),
        principal.is_owner(),
        request.explicit_approval,
    ) {
        Ok(outcome) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.consolidate",
                None,
                None,
                &outcome.status,
                Some(1),
                started,
            );
            if principal.is_owner() && request.policy.enabled {
                let store = state.store.clone();
                let policy = request.policy.clone();
                let principal_name = principal.name.clone();
                let principal_acl = principal.visible_acl();
                tokio::task::spawn_blocking(move || {
                    if let Err(error) = store.process_pending_memory_consolidation(
                        &policy,
                        &principal_name,
                        &principal_acl,
                        true,
                        policy.max_queue,
                    ) {
                        tracing::warn!(%error, "memory consolidation recovery worker failed");
                        let _ = store.record_audit(
                            &principal_name,
                            "memory.consolidation.recovery",
                            None,
                            Some("candidate"),
                            "failed",
                            None,
                            0,
                            10_000,
                        );
                    }
                });
            }
            Ok(Json(outcome))
        }
        Err(error) if crate::memory::is_authorization_error(&error) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.consolidate",
                None,
                None,
                "denied",
                None,
                started,
            );
            Err((
                StatusCode::FORBIDDEN,
                "candidate consolidation denied".into(),
            ))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "memory.candidate.consolidate",
                None,
                None,
                "failed",
                None,
                started,
            );
            Err((StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))
        }
    }
}
async fn update_memory_candidate(
    state: &AppState,
    principal: &Principal,
    id: &str,
    redact: bool,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let started = Instant::now();
    let action = if redact {
        "memory.candidate.redact"
    } else {
        "memory.candidate.cancel"
    };
    let result = if redact {
        state.store.redact_memory_candidate_scoped(
            id,
            &principal.name,
            &principal.visible_acl(),
            principal.is_owner(),
        )
    } else {
        state.store.cancel_memory_candidate_scoped(
            id,
            &principal.name,
            &principal.visible_acl(),
            principal.is_owner(),
        )
    };
    match result {
        Ok(true) => {
            record_audit(
                state,
                principal,
                action,
                None,
                None,
                "succeeded",
                Some(1),
                started,
            );
            Ok(Json(
                serde_json::json!({"id": id, "updated": true, "status": if redact { "redacted" } else { "cancelled" }}),
            ))
        }
        Ok(false) => {
            record_audit(
                state,
                principal,
                action,
                None,
                None,
                "not_found",
                Some(0),
                started,
            );
            Err((StatusCode::NOT_FOUND, "pending candidate not found".into()))
        }
        Err(error)
            if crate::memory::is_authorization_error(&error)
                || error.to_string() == "candidate ACL denied" =>
        {
            record_audit(
                state,
                principal,
                action,
                None,
                None,
                "forbidden",
                None,
                started,
            );
            Err((StatusCode::FORBIDDEN, "candidate ACL denied".into()))
        }
        Err(error) => {
            record_audit(
                state, principal, action, None, None, "failed", None, started,
            );
            Err((StatusCode::UNPROCESSABLE_ENTITY, error.to_string()))
        }
    }
}

async fn forget_memory(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<MemoryForgetRequest>,
) -> Result<Json<MemoryForgetResponse>, (StatusCode, String)> {
    let started = Instant::now();
    let memory = state
        .store
        .memory(&request.id)
        .map_err(internal_error)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "memory not found".into()))?;
    if !principal.is_owner() && !acl_allows(&memory.acl, &principal.visible_acl()) {
        record_audit(
            &state,
            &principal,
            "memory.forget",
            Some(&memory.project),
            Some(&memory.source),
            "forbidden",
            None,
            started,
        );
        return Err((StatusCode::FORBIDDEN, "memory ACL denied".into()));
    }
    let forgotten = match state.store.forget_memory_scoped(
        &request.id,
        &principal.visible_acl(),
        principal.is_owner(),
    ) {
        Ok(forgotten) => forgotten,
        Err(error) if crate::memory::is_authorization_error(&error) => {
            record_audit(
                &state,
                &principal,
                "memory.forget",
                Some(&memory.project),
                Some(&memory.source),
                "forbidden",
                None,
                started,
            );
            return Err((StatusCode::FORBIDDEN, "memory ACL denied".into()));
        }
        Err(error) => return Err(internal_error(error)),
    };
    record_audit(
        &state,
        &principal,
        "memory.forget",
        Some(&memory.project),
        Some(&memory.source),
        "succeeded",
        Some(usize::from(forgotten)),
        started,
    );
    Ok(Json(MemoryForgetResponse {
        id: request.id,
        forgotten,
    }))
}

async fn export_memories(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    AxumQuery(params): AxumQuery<MemoryExportParams>,
) -> Result<Json<Vec<crate::memory::MemoryRecord>>, (StatusCode, String)> {
    validate_retrieval_scope(params.project.as_deref(), None)?;
    let started = Instant::now();
    let exported = if principal.is_owner() {
        state.store.export_memories_with_axes_as_owner(
            params.project.as_deref(),
            params.kind.as_deref(),
            params.content_type.as_deref(),
            params.retention_tier.as_deref(),
            params.scope.as_deref(),
            params.limit,
        )
    } else {
        state.store.export_memories_with_axes(
            params.project.as_deref(),
            params.kind.as_deref(),
            params.content_type.as_deref(),
            params.retention_tier.as_deref(),
            params.scope.as_deref(),
            params.limit,
            &principal.visible_acl(),
        )
    };
    match exported {
        Ok(memories) => {
            record_audit(
                &state,
                &principal,
                "memory.export",
                params.project.as_deref(),
                None,
                "succeeded",
                Some(memories.len()),
                started,
            );
            Ok(Json(memories))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "memory.export",
                params.project.as_deref(),
                None,
                "failed",
                None,
                started,
            );
            Err(internal_error(error))
        }
    }
}

fn derive_authorized_memories(
    state: &AppState,
    principal: &Principal,
    project: Option<&str>,
    limit: usize,
) -> Result<DerivedMemoryResponse> {
    derive_authorized_memory(
        &state.store,
        project,
        limit,
        &principal.visible_acl(),
        principal.is_owner(),
    )
}

async fn derived_memories(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    AxumQuery(params): AxumQuery<DerivedMemoryParams>,
) -> Result<Json<DerivedMemoryResponse>, (StatusCode, String)> {
    validate_retrieval_scope(params.project.as_deref(), None)?;
    let started = Instant::now();
    match derive_authorized_memories(&state, &principal, params.project.as_deref(), params.limit) {
        Ok(response) => {
            record_audit(
                &state,
                &principal,
                "memory.derived.read",
                params.project.as_deref(),
                None,
                "succeeded",
                Some(response.representations.len() + response.relations.len()),
                started,
            );
            Ok(Json(response))
        }
        Err(error) => {
            record_audit(
                &state,
                &principal,
                "memory.derived.read",
                params.project.as_deref(),
                None,
                "failed",
                None,
                started,
            );
            Err(internal_error(error))
        }
    }
}

#[derive(Deserialize)]
struct AuditParams {
    #[serde(default = "default_audit_limit")]
    limit: usize,
}

async fn audit_events(
    State(state): State<AppState>,
    Extension(_principal): Extension<Principal>,
    AxumQuery(params): AxumQuery<AuditParams>,
) -> Result<Json<Vec<AuditEvent>>, (StatusCode, String)> {
    state
        .store
        .audit_events(params.limit.clamp(1, 500))
        .map(Json)
        .map_err(internal_error)
}

#[allow(clippy::too_many_arguments)]
fn record_audit(
    state: &AppState,
    principal: &Principal,
    action: &str,
    project: Option<&str>,
    source: Option<&str>,
    outcome: &str,
    count: Option<usize>,
    started: Instant,
) {
    if let Err(error) = state.store.record_audit(
        &principal.name,
        action,
        project,
        source,
        outcome,
        count,
        elapsed_ms(started),
        state.audit_max_events,
    ) {
        tracing::warn!(%error, "query audit write failed");
    }
}

async fn metrics(
    State(state): State<AppState>,
    Extension(_principal): Extension<Principal>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let stats = store_stats_with_timeout(state.store.clone(), READY_STORE_STATS_TIMEOUT)
        .await
        .map_err(internal_error)?;
    let body = format!(
        "# HELP cortana_uptime_seconds Process uptime in seconds.\n\
         # TYPE cortana_uptime_seconds gauge\n\
         cortana_uptime_seconds {}\n\
         # HELP cortana_documents Indexed canonical documents.\n\
         # TYPE cortana_documents gauge\n\
         cortana_documents {}\n\
         # HELP cortana_chunks Indexed evidence chunks.\n\
         # TYPE cortana_chunks gauge\n\
         cortana_chunks {}\n\
         # HELP cortana_embedding_cache_entries Persisted embedding cache entries.\n\
         # TYPE cortana_embedding_cache_entries gauge\n\
         cortana_embedding_cache_entries {}\n\
         # HELP cortana_embedding_cache_hits_total Persisted embedding cache hits.\n\
         # TYPE cortana_embedding_cache_hits_total counter\n\
         cortana_embedding_cache_hits_total {}\n\
         # HELP cortana_query_cache_entries Persisted planned-answer cache entries.\n\
         # TYPE cortana_query_cache_entries gauge\n\
         cortana_query_cache_entries {}\n\
         # HELP cortana_query_cache_hits_total Persisted planned-answer cache hits.\n\
         # TYPE cortana_query_cache_hits_total counter\n\
         cortana_query_cache_hits_total {}\n\
         # HELP cortana_search_requests_total Raw evidence search requests.\n\
         # TYPE cortana_search_requests_total counter\n\
         cortana_search_requests_total {}\n\
         # HELP cortana_context_requests_total Context bundle requests.\n\
         # TYPE cortana_context_requests_total counter\n\
         cortana_context_requests_total {}\n\
         # HELP cortana_answer_requests_total Planned answer requests.\n\
         # TYPE cortana_answer_requests_total counter\n\
         cortana_answer_requests_total {}\n\
         # HELP cortana_query_errors_total Query pipeline errors.\n\
         # TYPE cortana_query_errors_total counter\n\
         cortana_query_errors_total {}\n\
         # HELP cortana_retrieval_fallbacks_total Queries that used lexical retrieval because the embedding provider was unavailable or timed out.\n\
         # TYPE cortana_retrieval_fallbacks_total counter\n\
         cortana_retrieval_fallbacks_total {}\n",
        state.metrics.uptime_seconds(),
        stats.documents,
        stats.chunks,
        stats.embedding_cache_entries,
        stats.embedding_cache_hits,
        stats.query_cache_entries,
        stats.query_cache_hits,
        state.metrics.searches.load(Ordering::Relaxed),
        state.metrics.contexts.load(Ordering::Relaxed),
        state.metrics.answers.load(Ordering::Relaxed),
        state.metrics.errors.load(Ordering::Relaxed),
        state.metrics.retrieval_fallbacks.load(Ordering::Relaxed),
    );
    Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body))
}

fn validate_query(query: &str) -> Result<(), (StatusCode, String)> {
    if query.trim().is_empty() {
        Err((StatusCode::BAD_REQUEST, "query must not be empty".into()))
    } else if query.len() > retrieval::MAX_QUERY_BYTES {
        Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("query exceeds {} bytes", retrieval::MAX_QUERY_BYTES),
        ))
    } else {
        Ok(())
    }
}

fn validate_retrieval_scope(
    project: Option<&str>,
    source: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    validate_document_scope("project", project)?;
    validate_document_scope("source", source)
}

fn internal_error(error: anyhow::Error) -> (StatusCode, String) {
    tracing::error!(%error, "Cortana API request failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Cortana could not complete the request".into(),
    )
}

fn status_stats_error(error: anyhow::Error) -> (StatusCode, String) {
    let detail = error.to_string();
    if detail.contains("stats probe timed out") {
        tracing::warn!(%error, "Cortana status snapshot is temporarily unavailable");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Cortana is warming up; live status will be available shortly".into(),
        );
    }
    internal_error(error)
}

fn default_limit() -> usize {
    10
}

fn default_memory_limit() -> usize {
    10
}

fn default_memory_export_limit() -> usize {
    10_000
}

fn default_context_tokens() -> usize {
    8_000
}

fn default_audit_limit() -> usize {
    100
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub async fn serve(
    state: AppState,
    address: &str,
    web_dir: Option<&Path>,
    allow_remote: bool,
) -> Result<()> {
    let socket: std::net::SocketAddr = address.parse()?;
    let authenticated = state.auth_snapshot().policy.requires_token();
    anyhow::ensure!(
        socket.ip().is_loopback() || (allow_remote && authenticated),
        "refusing non-loopback bind without --allow-remote and a bearer token"
    );
    state.set_remote_listener(!socket.ip().is_loopback())?;
    if !socket.ip().is_loopback() {
        tracing::warn!(%socket, "serving a bearer-authenticated remote endpoint; terminate TLS upstream");
    }
    let listener = tokio::net::TcpListener::bind(address).await?;
    let mut app = router(state);
    if let Some(directory) = web_dir {
        let index = directory.join("index.html");
        anyhow::ensure!(
            index.is_file(),
            "workspace build is missing: run `bun run build` or use --no-web"
        );
        app =
            app.fallback_service(ServeDir::new(directory).not_found_service(ServeFile::new(index)));
    }
    tracing::info!(%socket, web = web_dir.is_some(), "Cortana API listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install terminate handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use async_trait::async_trait;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::config::{AuthTokenConfig, SourceConfig};
    use crate::embed::{DeterministicEmbedder, Embedder};
    use crate::model::Document;
    use crate::source_status::{
        MAX_TOKEN_FILE_BYTES, SourceAuthorizationMethod, source_authorization_summary,
        validation_error_category,
    };
    use crate::source_validation::{self, SourceValidationStatus};
    use crate::store::SyncRunStatus;

    use super::*;

    #[test]
    fn internal_errors_do_not_expose_server_details() {
        let (status, message) = internal_error(anyhow!(
            "open /Users/private/.config/cortana/store.sqlite3: permission denied"
        ));
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(message, "Cortana could not complete the request");
        assert!(!message.contains("/Users/private"));
    }

    #[test]
    fn status_stats_timeout_is_reported_as_retryable_warmup() {
        let (status, message) =
            status_stats_error(anyhow!("readiness stats probe timed out after 2s"));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            message,
            "Cortana is warming up; live status will be available shortly"
        );
    }

    #[test]
    fn status_stats_failures_remain_generic_and_do_not_expose_details() {
        let (status, message) = status_stats_error(anyhow!(
            "open /Users/private/.config/cortana/store.sqlite3: permission denied"
        ));
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(message, "Cortana could not complete the request");
        assert!(!message.contains("/Users/private"));
    }

    #[test]
    fn validation_error_categories_are_safe_and_bounded() {
        assert_eq!(
            validation_error_category("Client error: 403 Forbidden"),
            Some("authorization")
        );
        assert_eq!(
            validation_error_category("connector timed out after 30 seconds"),
            Some("timeout")
        );
        assert_eq!(
            validation_error_category("No such file or directory"),
            Some("missing-credential-or-path")
        );
        assert_eq!(
            validation_error_category("filesystem source exceeds the 25 document budget"),
            Some("budget")
        );
        assert_eq!(
            validation_error_category("unclassified failure"),
            Some("connector")
        );
    }

    fn test_state() -> (tempfile::TempDir, AppState) {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .ensure_fingerprint("deterministic:16")
            .expect("fingerprint");
        let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));
        (directory, AppState::new(store, embedder))
    }

    #[test]
    fn status_snapshot_cache_is_bounded_and_returns_cloned_stats() {
        let (_directory, state) = test_state();
        let stats = state.store.stats().expect("empty stats");
        for index in 0..40 {
            state.cache_status_stats(format!("principal-{index}"), stats.clone());
        }

        let cache = state.status_stats_cache.lock().expect("status cache lock");
        assert_eq!(cache.len(), 32);
        drop(cache);
        let cached = state
            .cached_status_stats("principal-39")
            .expect("latest snapshot");
        assert_eq!(cached.stats.documents, stats.documents);
    }

    struct UnavailableEmbedder;

    struct DelayedEmbedder {
        delay: Duration,
    }

    #[async_trait]
    impl Embedder for UnavailableEmbedder {
        async fn embed(&self, _input: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            anyhow::bail!("embedding provider unavailable")
        }

        fn fingerprint(&self) -> String {
            "unavailable:test".into()
        }
    }

    #[async_trait]
    impl Embedder for DelayedEmbedder {
        async fn embed(&self, _input: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            tokio::time::sleep(self.delay).await;
            Ok(vec![vec![1.0]])
        }

        fn fingerprint(&self) -> String {
            "delayed:test".into()
        }
    }

    fn write_private_fixture(path: &std::path::Path, body: &str) {
        std::fs::write(path, body).expect("write fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .expect("secure fixture");
        }
    }

    fn write_auth_reload_fixture(
        directory: &std::path::Path,
        token_env: &str,
        token: &str,
        tokens: &str,
    ) -> std::path::PathBuf {
        let env_path = directory.join("secrets.env");
        write_private_fixture(&env_path, &format!("{token_env}={token}\n"));
        let config_path = directory.join("config.toml");
        write_private_fixture(
            &config_path,
            &format!("[runtime]\nenv_file = \"secrets.env\"\n\n[auth]\n{tokens}\n"),
        );
        config_path
    }

    fn google_source(token: Option<std::path::PathBuf>) -> SourceConfig {
        SourceConfig {
            name: "personal-gmail".into(),
            kind: "gmail".into(),
            enabled: true,
            project: "personal".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            folders: Vec::new(),
            exclude_folders: Vec::new(),
            servers: Vec::new(),
            teams: Vec::new(),
            team_names: Vec::new(),
            communities: Vec::new(),
            community_names: Vec::new(),
            repositories: Vec::new(),
            token_env: None,
            token,
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        }
    }

    fn github_source(token: Option<std::path::PathBuf>) -> SourceConfig {
        SourceConfig {
            name: "work-github".into(),
            kind: "github".into(),
            enabled: true,
            project: "work".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            folders: Vec::new(),
            exclude_folders: Vec::new(),
            servers: Vec::new(),
            teams: Vec::new(),
            team_names: Vec::new(),
            communities: Vec::new(),
            community_names: Vec::new(),
            repositories: vec!["owner/repository".into()],
            token_env: None,
            token,
            oauth_client: None,
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        }
    }

    #[test]
    fn google_token_file_with_access_token_authorizes_google_source() {
        let directory = tempdir().expect("temporary directory");
        let token = directory.path().join("google-token.json");
        write_private_fixture(&token, "{\"token\":\"abc\",\"client_id\":\"unused\"}\n");
        let summary = source_authorization_summary(&Config::default(), &google_source(Some(token)));

        assert!(summary.authorized);
        assert!(!summary.setup_required);
        assert!(matches!(
            summary.method,
            SourceAuthorizationMethod::GoogleOauth
        ));
    }

    #[test]
    fn github_token_file_authorizes_github_source() {
        let directory = tempdir().expect("temporary directory");
        let token = directory.path().join("github-token.json");
        write_private_fixture(
            &token,
            "{\"access_token\":\"gho_test\",\"token_type\":\"bearer\"}\n",
        );
        let summary = source_authorization_summary(&Config::default(), &github_source(Some(token)));

        assert!(summary.authorized);
        assert!(!summary.setup_required);
        assert!(matches!(
            summary.method,
            SourceAuthorizationMethod::GithubOauth
        ));
    }

    #[test]
    fn github_token_environment_requires_a_bearer_shaped_value() {
        let mut source = github_source(None);
        source.token_env = Some("GITHUB_TOKEN".into());
        let mut config = Config::default();
        config
            .environment
            .insert("GITHUB_TOKEN".into(), "gho_test\n".into());
        assert!(!source_authorization_summary(&config, &source).authorized);

        config
            .environment
            .insert("GITHUB_TOKEN".into(), "gho_test".into());
        let summary = source_authorization_summary(&config, &source);
        assert!(summary.authorized);
        assert!(!summary.setup_required);
    }

    #[test]
    fn google_token_file_with_refresh_token_and_client_id_authorizes_google_source() {
        let directory = tempdir().expect("temporary directory");
        let token = directory.path().join("google-token.json");
        write_private_fixture(
            &token,
            "{\"refresh_token\":\"refresh\",\"client_id\":\"client-id\"}\n",
        );
        let summary = source_authorization_summary(&Config::default(), &google_source(Some(token)));

        assert!(summary.authorized);
        assert!(!summary.setup_required);
    }

    #[test]
    fn google_token_file_without_credentials_is_not_authorized() {
        let directory = tempdir().expect("temporary directory");
        let token = directory.path().join("google-token.json");
        write_private_fixture(&token, "{\"foo\": \"bar\"}\n");

        let summary = source_authorization_summary(&Config::default(), &google_source(Some(token)));
        assert!(!summary.authorized);
        assert!(summary.setup_required);
    }

    #[test]
    fn google_token_file_rejects_malformed_json() {
        let directory = tempdir().expect("temporary directory");
        let token = directory.path().join("google-token.json");
        write_private_fixture(&token, "{token: [\"unclosed\"\n");
        let summary = source_authorization_summary(&Config::default(), &google_source(Some(token)));

        assert!(!summary.authorized);
        assert!(summary.setup_required);
    }

    #[test]
    fn google_token_file_rejects_non_object_payloads() {
        let directory = tempdir().expect("temporary directory");
        let token = directory.path().join("google-token.json");
        write_private_fixture(&token, "[]\n");

        let summary = source_authorization_summary(&Config::default(), &google_source(Some(token)));
        assert!(!summary.authorized);
        assert!(summary.setup_required);
    }

    #[test]
    fn google_token_file_rejects_oversized_payload() {
        let directory = tempdir().expect("temporary directory");
        let token = directory.path().join("google-token.json");
        std::fs::write(&token, vec![b'{'; MAX_TOKEN_FILE_BYTES + 1]).expect("oversized fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&token, std::fs::Permissions::from_mode(0o600))
                .expect("secure fixture");
        }

        let summary = source_authorization_summary(&Config::default(), &google_source(Some(token)));
        assert!(!summary.authorized);
        assert!(summary.setup_required);
    }

    #[cfg(unix)]
    #[test]
    fn google_token_environment_value_rejects_symlinked_parent() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().expect("temporary directory");
        let real = directory.path().join("real");
        std::fs::create_dir(&real).expect("real token directory");
        let token = real.join("google-token.json");
        write_private_fixture(&token, "{\"token\":\"abc\"}\n");
        let linked = directory.path().join("linked");
        symlink(&real, &linked).expect("symlink token directory");

        let mut source = google_source(None);
        source.token_env = Some("GOOGLE_TOKEN_PATH".into());
        let mut config = Config::default();
        config.environment.insert(
            "GOOGLE_TOKEN_PATH".into(),
            linked.join("google-token.json").display().to_string(),
        );

        let summary = source_authorization_summary(&config, &source);
        assert!(!summary.authorized);
    }

    #[test]
    fn google_token_environment_value_requires_an_absolute_valid_authorization_payload() {
        let directory = tempdir().expect("temporary directory");
        let token = directory.path().join("google-token.json");
        let mut source = google_source(None);
        source.token_env = Some("GOOGLE_TOKEN_PATH".into());
        let mut config = Config::default();

        config
            .environment
            .insert("GOOGLE_TOKEN_PATH".into(), token.display().to_string());
        let missing = source_authorization_summary(&config, &source);
        assert!(!missing.authorized);
        assert!(missing.setup_required);

        config.environment.insert(
            "GOOGLE_TOKEN_PATH".into(),
            "relative/google-token.json".into(),
        );
        assert!(!source_authorization_summary(&config, &source).authorized);

        write_private_fixture(&token, "{\"access_token\":\"abc\"}\n");
        config
            .environment
            .insert("GOOGLE_TOKEN_PATH".into(), token.display().to_string());
        let ready = source_authorization_summary(&config, &source);
        assert!(ready.authorized);
        assert!(!ready.setup_required);
    }

    #[test]
    fn google_oauth_requires_a_token_destination_before_authorize() {
        let directory = tempdir().expect("temporary directory");
        let client = directory.path().join("oauth-client.json");
        write_private_fixture(&client, "{}\n");
        let mut source = google_source(None);
        source.oauth_client = Some(client);
        let summary = source_authorization_summary(&Config::default(), &source);

        assert!(!summary.authorized);
        assert!(summary.setup_required);
    }

    #[test]
    fn google_oauth_client_can_authorize_to_a_new_token_destination() {
        let directory = tempdir().expect("temporary directory");
        let client = directory.path().join("oauth-client.json");
        let token = directory.path().join("google-token.json");
        write_private_fixture(&client, "{}\n");
        let mut source = google_source(Some(token));
        source.oauth_client = Some(client);
        let summary = source_authorization_summary(&Config::default(), &source);

        assert!(!summary.authorized);
        assert!(!summary.setup_required);
    }

    #[test]
    fn fallback_workspaces_prefer_core_scopes_and_bound_other_projects() {
        let workspaces = fallback_workspaces(
            &[],
            ["community", "special", "agents", "work", "personal"]
                .into_iter()
                .map(String::from),
        );
        let ids: Vec<_> = workspaces
            .iter()
            .map(|workspace| workspace.id.as_str())
            .collect();
        assert_eq!(ids, vec!["work", "personal", "special"]);
        assert_eq!(workspaces[0].name, "Work");

        let explicit = vec![WorkspaceConfig {
            id: "custom".into(),
            name: "Custom".into(),
            account_label: Some("owner@example.com".into()),
            color: Some("#123456".into()),
        }];
        let preserved = fallback_workspaces(&explicit, ["work".into()]);
        assert_eq!(preserved.len(), 1);
        assert_eq!(preserved[0].id, "custom");
        assert_eq!(
            preserved[0].account_label.as_deref(),
            Some("owner@example.com")
        );
        assert_eq!(preserved[0].color.as_deref(), Some("#123456"));

        let oversized = (0..4)
            .map(|index| WorkspaceConfig {
                id: format!("workspace-{index}"),
                name: format!("Workspace {index}"),
                account_label: None,
                color: None,
            })
            .collect::<Vec<_>>();
        assert_eq!(fallback_workspaces(&oversized, std::iter::empty()).len(), 4);
    }

    #[tokio::test]
    async fn health_is_public_but_api_and_metrics_require_configured_token() {
        let (_directory, state) = test_state();
        let mut config = Config::default();
        config
            .environment
            .insert("SHARED_TOKEN".into(), "secret".into());
        config.auth.tokens = vec![AuthTokenConfig {
            principal: "shared-agent".into(),
            token_env: "SHARED_TOKEN".into(),
            scopes: vec![QUERY_SCOPE.into(), STATUS_SCOPE.into(), ADMIN_SCOPE.into()],
            acl: Vec::new(),
        }];
        let policy = AuthPolicy::from_config(&config).expect("auth policy");
        let app = router(state.clone().with_auth_policy(policy));
        let health = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("health response");
        assert_eq!(health.status(), StatusCode::OK);

        let denied = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("denied response");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .header(header::AUTHORIZATION, "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("metrics response");
        assert_eq!(authorized.status(), StatusCode::OK);

        let memory_denied = router(
            state
                .clone()
                .with_auth_policy(AuthPolicy::from_config(&config).expect("policy")),
        )
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/memory/recall")
                .header(header::AUTHORIZATION, "Bearer secret")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"query":"release notes"}"#))
                .expect("memory request"),
        )
        .await
        .expect("memory denial response");
        assert_eq!(memory_denied.status(), StatusCode::FORBIDDEN);
        let consolidation_denied = router(
            state
                .clone()
                .with_auth_policy(AuthPolicy::from_config(&config).expect("policy")),
        )
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/memory/candidates/hidden/consolidate")
                .header(header::AUTHORIZATION, "Bearer secret")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"policy":{"enabled":false}}"#))
                .expect("consolidation request"),
        )
        .await
        .expect("consolidation denial response");
        assert_eq!(consolidation_denied.status(), StatusCode::FORBIDDEN);
        let events = state.store.audit_events(10).expect("audit events");
        assert!(events.iter().any(|event| {
            event.action == "memory.candidate.consolidate" && event.outcome == "forbidden"
        }));
    }

    #[tokio::test]
    async fn reflection_route_accepts_a_memory_scoped_principal_without_query_scope() {
        let (_directory, state) = test_state();
        let mut config = Config::default();
        config
            .environment
            .insert("MEMORY_TOKEN".into(), "memory-secret".into());
        config.auth.tokens = vec![AuthTokenConfig {
            principal: "memory-agent".into(),
            token_env: "MEMORY_TOKEN".into(),
            scopes: vec![MEMORY_SCOPE.into()],
            acl: vec!["work".into()],
        }];
        let app =
            router(state.with_auth_policy(AuthPolicy::from_config(&config).expect("auth policy")));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/memory/reflect")
                    .header(header::AUTHORIZATION, "Bearer memory-secret")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"objective":"Review launch risk","project":"work"}"#,
                    ))
                    .expect("reflection request"),
            )
            .await
            .expect("reflection response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("reflection body");
        let parsed: serde_json::Value =
            serde_json::from_slice(&body).expect("reflection JSON response");
        assert_eq!(parsed["objective"], "Review launch risk");
        assert_eq!(parsed["metrics"]["canonical_memory_mutated"], false);
    }

    #[tokio::test]
    async fn derived_memory_is_acl_scoped_non_mutating_and_inspectable_from_graph_and_reflection() {
        let (_directory, state) = test_state();
        let work = state
            .store
            .remember(&MemoryInput {
                kind: "semantic".into(),
                project: "work".into(),
                title: "Release policy".into(),
                content: "Release checks are required because quality matters".into(),
                source: "agent".into(),
                source_id: "work-derived".into(),
                dedupe_key: None,
                confidence: 0.9,
                importance: 0.8,
                acl: vec!["work".into()],
                provenance: serde_json::json!({"test": true}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect("work memory");
        let personal = state
            .store
            .remember(&MemoryInput {
                kind: "semantic".into(),
                project: "personal".into(),
                title: "Private policy".into(),
                content: "Private personal phrase".into(),
                source: "agent".into(),
                source_id: "personal-derived".into(),
                dedupe_key: None,
                confidence: 0.9,
                importance: 0.8,
                acl: vec!["personal".into()],
                provenance: serde_json::json!({"test": true}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect("personal memory");
        let revision = state.store.memory_revision().expect("revision");
        let mut config = Config::default();
        config
            .environment
            .insert("MEMORY_TOKEN".into(), "memory-secret".into());
        config.auth.tokens = vec![AuthTokenConfig {
            principal: "memory-agent".into(),
            token_env: "MEMORY_TOKEN".into(),
            scopes: vec![MEMORY_SCOPE.into(), QUERY_SCOPE.into()],
            acl: vec!["work".into()],
        }];
        let app = router(
            state
                .clone()
                .with_auth_policy(AuthPolicy::from_config(&config).expect("auth policy")),
        );

        let derived = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/memory/derived?project=work&limit=20")
                    .header(header::AUTHORIZATION, "Bearer memory-secret")
                    .body(Body::empty())
                    .expect("derived request"),
            )
            .await
            .expect("derived response");
        assert_eq!(derived.status(), StatusCode::OK);
        let derived_body = to_bytes(derived.into_body(), 128 * 1024)
            .await
            .expect("derived body");
        let derived_json: serde_json::Value =
            serde_json::from_slice(&derived_body).expect("derived JSON");
        assert_eq!(derived_json["canonical_memory_mutated"], false);
        assert_eq!(derived_json["memory_revision"], revision);
        let serialized = String::from_utf8(derived_body.to_vec()).expect("UTF-8");
        assert!(serialized.contains(&work.id));
        assert!(!serialized.contains(&personal.id));
        assert_eq!(
            state.store.memory_revision().expect("stable revision"),
            revision
        );

        let graph = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/graph?project=work&limit=20&include_derived=true")
                    .header(header::AUTHORIZATION, "Bearer memory-secret")
                    .body(Body::empty())
                    .expect("graph request"),
            )
            .await
            .expect("graph response");
        assert_eq!(graph.status(), StatusCode::OK);
        let graph_body = to_bytes(graph.into_body(), 128 * 1024)
            .await
            .expect("graph body");
        let graph_json: serde_json::Value =
            serde_json::from_slice(&graph_body).expect("graph JSON");
        let observation = graph_json["nodes"]
            .as_array()
            .and_then(|nodes| {
                nodes
                    .iter()
                    .find(|node| node["kind"] == "memory-observation")
            })
            .expect("derived observation node");
        assert_eq!(
            observation["contract_version"],
            crate::derived::DERIVED_MEMORY_CONTRACT_VERSION
        );
        assert_eq!(
            observation["derivation_version"],
            crate::derived::DERIVATION_ENGINE_VERSION
        );
        assert_eq!(observation["memory_revision"], revision);
        assert_eq!(observation["citation_authority"], false);
        assert!(
            observation["supporting_memory_ids"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == &work.id))
        );

        let mut query_config = Config::default();
        query_config
            .environment
            .insert("QUERY_TOKEN".into(), "query-secret".into());
        query_config.auth.tokens = vec![AuthTokenConfig {
            principal: "query-agent".into(),
            token_env: "QUERY_TOKEN".into(),
            scopes: vec![QUERY_SCOPE.into()],
            acl: vec!["work".into()],
        }];
        let query_only_app = router(
            state
                .clone()
                .with_auth_policy(AuthPolicy::from_config(&query_config).expect("query policy")),
        );
        let denied_graph = query_only_app
            .oneshot(
                Request::builder()
                    .uri("/v1/graph?project=work&include_derived=true")
                    .header(header::AUTHORIZATION, "Bearer query-secret")
                    .body(Body::empty())
                    .expect("query-only graph request"),
            )
            .await
            .expect("query-only graph response");
        assert_eq!(denied_graph.status(), StatusCode::FORBIDDEN);

        let reflection = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/memory/reflect")
                    .header(header::AUTHORIZATION, "Bearer memory-secret")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"objective":"Inspect release policy","project":"work","include_derived":true}"#,
                    ))
                    .expect("reflection request"),
            )
            .await
            .expect("reflection response");
        assert_eq!(reflection.status(), StatusCode::OK);
        let reflection_body = to_bytes(reflection.into_body(), 128 * 1024)
            .await
            .expect("reflection body");
        let reflection_json: serde_json::Value =
            serde_json::from_slice(&reflection_body).expect("reflection JSON");
        assert!(
            reflection_json["derived_representations"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
        assert_eq!(
            state.store.memory_revision().expect("stable revision"),
            revision
        );
    }

    #[tokio::test]
    async fn remote_readiness_requires_bearer_but_liveness_stays_public() {
        let (_directory, state) = test_state();
        let mut config = Config::default();
        config
            .environment
            .insert("REMOTE_TOKEN".into(), "remote-secret".into());
        config.auth.tokens = vec![AuthTokenConfig {
            principal: "remote-agent".into(),
            token_env: "REMOTE_TOKEN".into(),
            scopes: vec![QUERY_SCOPE.into(), STATUS_SCOPE.into()],
            acl: Vec::new(),
        }];
        let policy = AuthPolicy::from_config(&config).expect("auth policy");
        let app = router(state.with_auth_policy_for_listener(policy, true));

        let health = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("health request"),
            )
            .await
            .expect("health response");
        assert_eq!(health.status(), StatusCode::OK);

        let denied = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .expect("readiness request"),
            )
            .await
            .expect("readiness response");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

        let authorized = app
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .header(header::AUTHORIZATION, "Bearer remote-secret")
                    .body(Body::empty())
                    .expect("authorized readiness request"),
            )
            .await
            .expect("authorized readiness response");
        assert_ne!(authorized.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn auth_reload_rotates_http_tokens_and_revokes_the_old_value() {
        let (directory, state) = test_state();
        let config_path = write_auth_reload_fixture(
            directory.path(),
            "CORTANA_TOKEN",
            "old-secret",
            "[[auth.tokens]]\nprincipal = \"admin-agent\"\ntoken_env = \"CORTANA_TOKEN\"\nscopes = [\"query\", \"status\", \"admin\"]\n",
        );
        let mut config = Config::load(Some(&config_path)).expect("config");
        config.load_environment().expect("environment");
        let policy = AuthPolicy::from_config(&config).expect("policy");
        let state = state
            .with_auth_policy(policy)
            .with_auth_config_path(&config_path);
        let app = router(state.clone());

        let before = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(header::AUTHORIZATION, "Bearer old-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(before.status(), StatusCode::OK);

        write_private_fixture(
            &directory.path().join("secrets.env"),
            "CORTANA_TOKEN=new-secret\n",
        );
        let reloaded = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/auth/reload")
                    .header(header::AUTHORIZATION, "Bearer old-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("reload response");
        assert_eq!(reloaded.status(), StatusCode::OK);

        let revoked = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(header::AUTHORIZATION, "Bearer old-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("revoked response");
        assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
        let current = app
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(header::AUTHORIZATION, "Bearer new-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("current response");
        assert_eq!(current.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn auth_reload_preserves_last_good_policy_on_invalid_config() {
        let (directory, state) = test_state();
        let config_path = write_auth_reload_fixture(
            directory.path(),
            "CORTANA_TOKEN",
            "stable-secret",
            "[[auth.tokens]]\nprincipal = \"admin-agent\"\ntoken_env = \"CORTANA_TOKEN\"\nscopes = [\"query\", \"status\", \"admin\"]\n",
        );
        let mut config = Config::load(Some(&config_path)).expect("config");
        config.load_environment().expect("environment");
        let policy = AuthPolicy::from_config(&config).expect("policy");
        let state = state
            .with_auth_policy(policy)
            .with_auth_config_path(&config_path);
        let app = router(state.clone());

        write_private_fixture(&directory.path().join("secrets.env"), "CORTANA_TOKEN=\n");
        let failed = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/auth/reload")
                    .header(header::AUTHORIZATION, "Bearer stable-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("reload response");
        assert_eq!(failed.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let still_valid = app
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(header::AUTHORIZATION, "Bearer stable-secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("status response");
        assert_eq!(still_valid.status(), StatusCode::OK);
        let audit = state.store.audit_events(10).expect("audit events");
        assert!(audit.iter().any(|event| {
            event.action == "auth.reload"
                && event.outcome == "failed"
                && event.principal == "admin-agent"
                && event.project.is_none()
                && event.source.is_none()
        }));
    }

    #[test]
    fn remote_listener_cannot_be_deauthenticated_by_reload() {
        let (_directory, state) = test_state();
        let mut config = Config::default();
        config
            .environment
            .insert("REMOTE_TOKEN".into(), "secret".into());
        config.auth.tokens.push(AuthTokenConfig {
            principal: "remote-admin".into(),
            token_env: "REMOTE_TOKEN".into(),
            scopes: vec![QUERY_SCOPE.into(), STATUS_SCOPE.into(), ADMIN_SCOPE.into()],
            acl: Vec::new(),
        });
        let policy = AuthPolicy::from_config(&config).expect("policy");
        let state = state.with_auth_policy_for_listener(policy, true);
        assert!(state.replace_auth_policy(AuthPolicy::default()).is_err());
        assert!(state.auth_policy().requires_token());
    }

    #[tokio::test]
    async fn readiness_probe_fails_closed_when_embedding_provider_stalls() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let state = AppState::new(
            store,
            Arc::new(DelayedEmbedder {
                delay: Duration::from_secs(60),
            }),
        );
        let started = Instant::now();
        let response = ready_with_probe_timeout(state, Duration::from_millis(10))
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn readiness_stats_probe_fails_closed_when_store_stats_stall() {
        let started = Instant::now();
        let result = blocking_stats_with_timeout(
            || {
                std::thread::sleep(Duration::from_millis(25));
                Err::<StoreStats, _>(anyhow!("unexpected stats completion"))
            },
            Duration::from_millis(1),
        )
        .await;

        let error = result.expect_err("stalled stats probe must fail closed");
        assert!(
            error
                .to_string()
                .contains("readiness stats probe timed out")
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn remote_bind_requires_a_configured_bearer_token() {
        let (_directory, state) = test_state();
        let error = serve(state, "0.0.0.0:0", None, true)
            .await
            .expect_err("remote bind without configured auth must fail closed");
        assert!(
            error
                .to_string()
                .contains("--allow-remote and a bearer token")
        );
    }

    #[tokio::test]
    async fn context_rejects_oversized_bodies() {
        let (_directory, state) = test_state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/context")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("x".repeat(1024 * 1024 + 1)))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn search_rejects_oversized_queries_before_embedding() {
        let (_directory, state) = test_state();
        let body = serde_json::to_vec(&serde_json::json!({
            "query": "x".repeat(retrieval::MAX_QUERY_BYTES + 1)
        }))
        .expect("request JSON");
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/search")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn retrieval_rejects_oversized_scope_filters() {
        let (_directory, state) = test_state();
        let body = serde_json::to_vec(&serde_json::json!({
            "query": "release",
            "project": "x".repeat(MAX_DOCUMENT_SCOPE_LENGTH + 1)
        }))
        .expect("request JSON");
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/search")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn retrieval_rejects_control_characters_in_scope_filters() {
        let (_directory, state) = test_state();
        let body = serde_json::to_vec(&serde_json::json!({
            "query": "release",
            "source": "slack\u{0000}work"
        }))
        .expect("request JSON");
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/search")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn status_reports_safe_ingestion_mode_and_configured_sources() {
        let (directory, state) = test_state();
        let mut config: Config = toml::from_str(
            r##"
            [ingestion]
            max_documents_per_source = 25
            max_bytes_per_source = 4096
            max_duration_seconds = 45
            request_concurrency = 1

            [[workspaces]]
            id = "work"
            name = "Work"
            account_label = "team@example.com"
            color = "#5A9BD5"

            [[sources]]
            name = "code"
            kind = "filesystem"
            enabled = false
            project = "work"
            source = "work-code"
            acl = ["work", "admin"]
            root = "/tmp/code"
            "##,
        )
        .expect("configuration");
        config.data_dir = directory.path().to_path_buf();
        let state = state.with_config(&config, false);
        let validation_fingerprint = source_validation::configuration_fingerprint(
            config.sources.first().expect("configured source"),
        )
        .expect("validation fingerprint");
        source_validation::record(
            &config.data_dir,
            SourceValidationStatus {
                source: "code".into(),
                project: "work".into(),
                kind: "filesystem".into(),
                status: "succeeded".into(),
                validated_at: chrono::Utc::now(),
                documents: Some(7),
                bytes: Some(512),
                max_documents: 25,
                max_bytes: 4096,
                max_seconds: 45,
                configuration_fingerprint: Some(validation_fingerprint),
                complete: None,
                error: None,
            },
        )
        .expect("validation state");
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("status response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("status body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("status JSON");
        assert_eq!(value["ingestion"]["mode"], "manual");
        assert_eq!(value["ingestion"]["scheduled"], false);
        assert_eq!(
            value["ingestion"]["configured_sources"][0]["source"],
            "work-code"
        );
        assert_eq!(
            value["ingestion"]["configured_sources"][0]["enabled"],
            false
        );
        assert_eq!(
            value["ingestion"]["configured_sources"][0]["validation"]["status"],
            "succeeded"
        );
        assert!(
            value["ingestion"]["configured_sources"][0]["validation"]
                .get("configuration_fingerprint")
                .is_none()
        );
        assert_eq!(
            value["ingestion"]["configured_sources"][0]["acl"],
            serde_json::json!(["work", "admin"])
        );
        assert_eq!(
            value["ingestion"]["configured_sources"][0]["authorization"]["method"],
            "none"
        );
        assert_eq!(
            value["ingestion"]["configured_sources"][0]["authorization"]["setup_required"],
            false
        );
        assert_eq!(
            value["ingestion"]["configured_sources"][0]["authorization"]["authorized"],
            true
        );
        assert_eq!(value["ingestion"]["max_documents_per_source"], 25);
        assert_eq!(value["ingestion"]["validation_max_age_hours"], 168);
        assert_eq!(value["ingestion"]["sync_freshness_hours"], 48);
        assert!(
            value["ingestion"]["configured_sources"][0]["validation"]["fresh"]
                .as_bool()
                .is_some_and(|fresh| fresh)
        );
        assert!(
            value["ingestion"]["configured_sources"][0]["validation"]["age_seconds"]
                .as_u64()
                .is_some_and(|age| age <= 3_600)
        );
        assert_eq!(value["query"]["mode"], "extractive");
        assert_eq!(value["query"]["max_planned_queries"], 4);
        assert_eq!(value["query_cache_entries"], 0);
        assert_eq!(value["sync_runs"], serde_json::json!([]));
        assert_eq!(value["workspaces"][0]["id"], "work");
        assert_eq!(value["workspaces"][0]["account_label"], "team@example.com");
    }

    #[test]
    fn scoped_status_source_inventory_follows_principal_acl() {
        let directory = tempdir().expect("temporary directory");
        let mut config: Config = toml::from_str(
            r#"
            [[sources]]
            name = "work-drive"
            kind = "google-drive"
            project = "work"
            acl = ["work"]

            [[sources]]
            name = "personal-notes"
            kind = "apple-notes"
            project = "personal"
            acl = ["personal"]

            [[sources]]
            name = "public-reference"
            kind = "filesystem"
            project = "reference"
            "#,
        )
        .expect("configuration");
        config.data_dir = directory.path().to_path_buf();

        let mut auth_config = Config::default();
        auth_config
            .environment
            .insert("WORK_TOKEN".into(), "work-secret".into());
        auth_config.auth.tokens = vec![AuthTokenConfig {
            principal: "work-agent".into(),
            token_env: "WORK_TOKEN".into(),
            scopes: vec![STATUS_SCOPE.into()],
            acl: vec!["work".into()],
        }];
        let principal = AuthPolicy::from_config(&auth_config)
            .expect("policy")
            .authenticate("work-secret")
            .expect("principal");

        let status = IngestionStatus::from_config(&config, false).visible_to(&principal);
        let names = status
            .configured_sources
            .iter()
            .map(|source| source.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["work-drive"]);

        let mut admin_config = auth_config;
        admin_config.auth.tokens[0].scopes = vec![ADMIN_SCOPE.into()];
        let admin = AuthPolicy::from_config(&admin_config)
            .expect("admin policy")
            .authenticate("work-secret")
            .expect("admin principal");
        let admin_status = IngestionStatus::from_config(&config, false).visible_to(&admin);
        let admin_names = admin_status
            .configured_sources
            .iter()
            .map(|source| source.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            admin_names,
            vec!["work-drive", "personal-notes", "public-reference"]
        );
    }

    #[tokio::test]
    async fn scoped_status_includes_sync_runs_for_acl_visible_sources_without_documents() {
        let (_directory, state) = test_state();
        let failed = state
            .store
            .begin_sync("work-drive", "work", 100, 2_048, 30)
            .expect("begin failed sync");
        state
            .store
            .finish_sync(&failed, SyncRunStatus::Failed, None, None, None)
            .expect("finish failed sync");
        let running = state
            .store
            .begin_sync("personal-notes", "personal", 50, 1_024, 60)
            .expect("begin running sync");
        for (project, acl, source_id) in [
            ("work", "work", "work-status-memory"),
            ("personal", "personal", "personal-status-memory"),
        ] {
            state
                .store
                .remember(&MemoryInput {
                    kind: "semantic".into(),
                    project: project.into(),
                    title: format!("{project} status memory"),
                    content: format!("{project} status context."),
                    source: "agent".into(),
                    source_id: source_id.into(),
                    dedupe_key: None,
                    confidence: 0.8,
                    importance: 0.7,
                    acl: vec![acl.into()],
                    provenance: serde_json::json!({"test":true}),
                    supersedes_id: None,
                    valid_until: None,
                })
                .expect("memory");
        }
        let store = state.store.clone();

        let mut config: Config = toml::from_str(
            r#"
            [[sources]]
            name = "work-drive"
            kind = "google-drive"
            project = "work"
            acl = ["work"]

            [[sources]]
            name = "personal-notes"
            kind = "apple-notes"
            project = "personal"
            acl = ["personal"]
            "#,
        )
        .expect("configuration");
        config.data_dir = _directory.path().to_path_buf();
        config.auth.tokens = vec![
            AuthTokenConfig {
                principal: "work-agent".into(),
                token_env: "WORK_TOKEN".into(),
                scopes: vec![STATUS_SCOPE.into()],
                acl: vec!["work".into()],
            },
            AuthTokenConfig {
                principal: "personal-agent".into(),
                token_env: "PERSONAL_TOKEN".into(),
                scopes: vec![STATUS_SCOPE.into()],
                acl: vec!["personal".into()],
            },
            AuthTokenConfig {
                principal: "auditor".into(),
                token_env: "ADMIN_TOKEN".into(),
                scopes: vec![ADMIN_SCOPE.into(), STATUS_SCOPE.into()],
                acl: Vec::new(),
            },
        ];
        config
            .environment
            .insert("WORK_TOKEN".into(), "work-secret".into());
        config
            .environment
            .insert("PERSONAL_TOKEN".into(), "personal-secret".into());
        config
            .environment
            .insert("ADMIN_TOKEN".into(), "admin-secret".into());
        let policy = AuthPolicy::from_config(&config).expect("auth policy");
        let app = router(state.with_config(&config, false).with_auth_policy(policy));

        let work_status = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(header::AUTHORIZATION, "Bearer work-secret")
                    .body(Body::empty())
                    .expect("status request"),
            )
            .await
            .expect("status response");
        assert_eq!(work_status.status(), StatusCode::OK);
        let work_body = to_bytes(work_status.into_body(), 1024 * 1024)
            .await
            .expect("status body");
        let work_value: serde_json::Value =
            serde_json::from_slice(&work_body).expect("status JSON");
        assert_eq!(work_value["documents"], 0);
        assert_eq!(work_value["sources"], serde_json::json!([]));
        assert_eq!(work_value["memory"]["active"], 1);
        assert_eq!(work_value["memory"]["total"], 1);
        let work_runs = work_value["sync_runs"].as_array().expect("sync runs");
        assert_eq!(work_runs.len(), 1);
        assert_eq!(work_runs[0]["source"], "work-drive");
        assert_eq!(work_runs[0]["project"], "work");
        assert_eq!(work_runs[0]["status"], "failed");
        assert!(work_runs[0]["completed_at"].is_string());
        assert_eq!(work_runs[0]["budget_documents"], 100);

        let personal_status = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(header::AUTHORIZATION, "Bearer personal-secret")
                    .body(Body::empty())
                    .expect("status request"),
            )
            .await
            .expect("status response");
        assert_eq!(personal_status.status(), StatusCode::OK);
        let personal_body = to_bytes(personal_status.into_body(), 1024 * 1024)
            .await
            .expect("status body");
        let personal_value: serde_json::Value =
            serde_json::from_slice(&personal_body).expect("status JSON");
        assert_eq!(personal_value["memory"]["active"], 1);
        assert_eq!(personal_value["memory"]["total"], 1);
        let personal_runs = personal_value["sync_runs"].as_array().expect("sync runs");
        assert_eq!(personal_runs.len(), 1);
        assert_eq!(personal_runs[0]["source"], "personal-notes");
        assert_eq!(personal_runs[0]["status"], "running");
        assert!(personal_runs[0]["completed_at"].is_null());

        let admin_status = app
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .body(Body::empty())
                    .expect("status request"),
            )
            .await
            .expect("status response");
        assert_eq!(admin_status.status(), StatusCode::OK);
        let admin_body = to_bytes(admin_status.into_body(), 1024 * 1024)
            .await
            .expect("status body");
        let admin_value: serde_json::Value =
            serde_json::from_slice(&admin_body).expect("status JSON");
        assert_eq!(admin_value["memory"]["active"], 2);
        assert_eq!(admin_value["memory"]["total"], 2);
        let admin_runs = admin_value["sync_runs"].as_array().expect("sync runs");
        assert_eq!(admin_runs.len(), 2, "owner view keeps every run");

        store
            .finish_sync(&running, SyncRunStatus::Cancelled, None, None, None)
            .expect("finish running sync");
    }

    #[test]
    fn status_marks_a_current_validation_fresh_with_a_bounded_age() {
        let directory = tempdir().expect("temporary directory");
        let mut config = Config {
            data_dir: directory.path().to_path_buf(),
            sources: vec![google_source(None)],
            ..Config::default()
        };
        config.ingestion.validation_max_age_hours = 24;
        let source = config.sources.first().expect("configured source");
        let fingerprint =
            source_validation::configuration_fingerprint(source).expect("validation fingerprint");
        source_validation::record(
            &config.data_dir,
            SourceValidationStatus {
                source: source.name.clone(),
                project: source.project.clone(),
                kind: source.kind.clone(),
                status: "succeeded".into(),
                validated_at: chrono::Utc::now() - chrono::Duration::hours(2),
                documents: Some(1),
                bytes: Some(1),
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 60,
                configuration_fingerprint: Some(fingerprint),
                complete: None,
                error: None,
            },
        )
        .expect("validation state");

        let status = IngestionStatus::from_config(&config, false);
        let validation = status.configured_sources[0]
            .validation
            .as_ref()
            .expect("current validation");
        assert!(validation.fresh);
        // The age only grows between recording and inspection, so a lower bound
        // of the fixture age (minus sub-second recording skew) is stable.
        assert!(validation.age_seconds >= 2 * 3_600 - 5);
        assert_eq!(status.validation_max_age_hours, 24);
    }

    #[test]
    fn status_marks_a_lapsed_validation_expired_and_honors_an_unlimited_bound() {
        let directory = tempdir().expect("temporary directory");
        let mut config = Config {
            data_dir: directory.path().to_path_buf(),
            sources: vec![google_source(None)],
            ..Config::default()
        };
        config.ingestion.validation_max_age_hours = 24;
        let source = config.sources.first().expect("configured source");
        let fingerprint =
            source_validation::configuration_fingerprint(source).expect("validation fingerprint");
        source_validation::record(
            &config.data_dir,
            SourceValidationStatus {
                source: source.name.clone(),
                project: source.project.clone(),
                kind: source.kind.clone(),
                status: "succeeded".into(),
                validated_at: chrono::Utc::now() - chrono::Duration::hours(200),
                documents: Some(1),
                bytes: Some(1),
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 60,
                configuration_fingerprint: Some(fingerprint),
                complete: None,
                error: None,
            },
        )
        .expect("validation state");

        let status = IngestionStatus::from_config(&config, false);
        let validation = status.configured_sources[0]
            .validation
            .as_ref()
            .expect("persisted validation");
        assert!(!validation.fresh);
        assert!(validation.age_seconds >= 200 * 3_600);

        // `0` disables the freshness bound: the same lapsed record stays fresh.
        config.ingestion.validation_max_age_hours = 0;
        let status = IngestionStatus::from_config(&config, false);
        let validation = status.configured_sources[0]
            .validation
            .as_ref()
            .expect("persisted validation");
        assert!(validation.fresh);
    }

    #[test]
    fn status_marks_a_future_validation_stale_but_keeps_zero_age() {
        let directory = tempdir().expect("temporary directory");
        let mut config = Config {
            data_dir: directory.path().to_path_buf(),
            sources: vec![google_source(None)],
            ..Config::default()
        };
        config.ingestion.validation_max_age_hours = 24;
        let source = config.sources.first().expect("configured source");
        let fingerprint =
            source_validation::configuration_fingerprint(source).expect("validation fingerprint");
        source_validation::record(
            &config.data_dir,
            SourceValidationStatus {
                source: source.name.clone(),
                project: source.project.clone(),
                kind: source.kind.clone(),
                status: "succeeded".into(),
                validated_at: chrono::Utc::now() + chrono::Duration::hours(2),
                documents: Some(1),
                bytes: Some(1),
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 60,
                configuration_fingerprint: Some(fingerprint),
                complete: None,
                error: None,
            },
        )
        .expect("validation state");

        let status = IngestionStatus::from_config(&config, false);
        let validation = status.configured_sources[0]
            .validation
            .as_ref()
            .expect("persisted validation");
        assert!(!validation.fresh);
        assert_eq!(validation.age_seconds, 0);

        // `0` disables the freshness bound: the same future record stays fresh.
        config.ingestion.validation_max_age_hours = 0;
        let status = IngestionStatus::from_config(&config, false);
        let validation = status.configured_sources[0]
            .validation
            .as_ref()
            .expect("persisted validation");
        assert!(validation.fresh);
        assert_eq!(validation.age_seconds, 0);
    }

    #[test]
    fn stale_source_validation_is_hidden_from_status() {
        let directory = tempdir().expect("temporary directory");
        let mut config = Config {
            data_dir: directory.path().to_path_buf(),
            sources: vec![google_source(None)],
            ..Config::default()
        };
        let source = config.sources.first().expect("configured source");
        let fingerprint =
            source_validation::configuration_fingerprint(source).expect("validation fingerprint");
        source_validation::record(
            &config.data_dir,
            SourceValidationStatus {
                source: source.name.clone(),
                project: source.project.clone(),
                kind: source.kind.clone(),
                status: "succeeded".into(),
                validated_at: chrono::Utc::now(),
                documents: Some(1),
                bytes: Some(1),
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 60,
                configuration_fingerprint: Some(fingerprint),
                complete: None,
                error: None,
            },
        )
        .expect("validation state");

        config.sources[0].query = Some("from:changed".into());
        let status = IngestionStatus::from_config(&config, false);
        assert!(status.configured_sources[0].validation.is_none());
    }

    #[test]
    fn source_validation_diagnostics_are_generic_in_public_status() {
        let directory = tempdir().expect("temporary directory");
        let config = Config {
            data_dir: directory.path().to_path_buf(),
            sources: vec![google_source(None)],
            ..Config::default()
        };
        let source = config.sources.first().expect("configured source");
        let fingerprint =
            source_validation::configuration_fingerprint(source).expect("validation fingerprint");
        source_validation::record(
            &config.data_dir,
            SourceValidationStatus {
                source: source.name.clone(),
                project: source.project.clone(),
                kind: source.kind.clone(),
                status: "failed".into(),
                validated_at: chrono::Utc::now(),
                documents: None,
                bytes: None,
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 60,
                configuration_fingerprint: Some(fingerprint),
                complete: None,
                error: Some("Bearer top-secret /Users/amf/private".into()),
            },
        )
        .expect("validation state");

        let status = IngestionStatus::from_config(&config, false);
        let validation = status.configured_sources[0]
            .validation
            .as_ref()
            .expect("failed validation");
        assert_eq!(
            validation.error.as_deref(),
            Some("source validation failed")
        );
        assert!(
            !serde_json::to_string(validation)
                .expect("validation JSON")
                .contains("top-secret")
        );
    }

    #[test]
    fn source_validation_exposes_google_connector_reason() {
        let directory = tempdir().expect("temporary directory");
        let config = Config {
            data_dir: directory.path().to_path_buf(),
            sources: vec![google_source(None)],
            ..Config::default()
        };
        let source = config.sources.first().expect("configured source");
        let fingerprint =
            source_validation::configuration_fingerprint(source).expect("validation fingerprint");
        source_validation::record(
            &config.data_dir,
            SourceValidationStatus {
                source: source.name.clone(),
                project: source.project.clone(),
                kind: source.kind.clone(),
                status: "failed".into(),
                validated_at: chrono::Utc::now(),
                documents: None,
                bytes: None,
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 60,
                configuration_fingerprint: Some(fingerprint),
                complete: None,
                error: Some("Drive listing is incomplete; refusing partial snapshot".into()),
            },
        )
        .expect("validation state");

        let status = IngestionStatus::from_config(&config, false);
        let validation = status.configured_sources[0]
            .validation
            .as_ref()
            .expect("failed validation");
        assert_eq!(
            validation.error.as_deref(),
            Some("google source snapshot was incomplete")
        );
        assert_eq!(validation.error_category, Some("connector"));
    }

    #[test]
    fn validation_state_errors_are_generic_to_callers() {
        let directory = tempdir().expect("temporary directory");
        write_private_fixture(
            &directory.path().join("source-validations.json"),
            "{\"sources\": invalid}",
        );
        let config = Config {
            data_dir: directory.path().to_path_buf(),
            ..Config::default()
        };

        let status = IngestionStatus::from_config(&config, false);
        assert_eq!(
            status.validation_state_error.as_deref(),
            Some("source validation state unavailable")
        );
        assert!(
            !status
                .validation_state_error
                .as_deref()
                .unwrap_or_default()
                .contains(directory.path().to_string_lossy().as_ref())
        );
    }

    #[tokio::test]
    async fn status_reports_source_authorization_readiness_for_google_and_token_backed_sources() {
        let (directory, state) = test_state();
        let token_path = directory.path().join("google-token.json");
        let oauth_client_path = directory.path().join("google-oauth-client.json");
        let incomplete_oauth_client_path =
            directory.path().join("missing-google-oauth-client.json");
        write_private_fixture(&token_path, "{\"refresh_token\":\"token\"}");
        write_private_fixture(
            &oauth_client_path,
            "{\"installed\":{\"client_id\":\"id\",\"client_secret\":\"secret\",\"auth_uri\":\"https://example.com/auth\",\"token_uri\":\"https://example.com/token\",\"auth_provider_x509_cert_url\":\"https://example.com/x509\",\"redirect_uris\":[\"http://127.0.0.1\"]}}",
        );

        let mut config: Config = toml::from_str(&format!(
            r##"
            [ingestion]
            request_concurrency = 1

            [[sources]]
            name = "slack"
            kind = "slack"
            enabled = true
            project = "work"
            token_env = "SLACK_TOKEN"
            acl = ["work"]

            [[sources]]
            name = "gmail"
            kind = "google-drive"
            enabled = true
            project = "work"
            oauth_client = "{oauth_client_path_display}"
            token = "{token_path_display}"
            acl = ["work"]

            [[sources]]
            name = "calendar"
            kind = "google-calendar"
            enabled = true
            project = "work"
            oauth_client = "{incomplete_oauth_client_path_display}"
            acl = ["work"]
            "##,
            oauth_client_path_display = oauth_client_path.display(),
            token_path_display = token_path.display(),
            incomplete_oauth_client_path_display = incomplete_oauth_client_path.display(),
        ))
        .expect("configuration");
        config.data_dir = directory.path().to_path_buf();
        config
            .environment
            .insert("SLACK_TOKEN".into(), "present".into());
        let state = state.with_config(&config, false);

        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("status response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("status body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("status JSON");

        let configured = &value["ingestion"]["configured_sources"];
        let slack = configured
            .as_array()
            .and_then(|list| list.iter().find(|item| item["name"] == "slack"))
            .expect("slack status");
        assert_eq!(slack["authorization"]["method"], "token");
        assert_eq!(slack["authorization"]["setup_required"], false);
        assert_eq!(slack["authorization"]["authorized"], true);

        let gmail = configured
            .as_array()
            .and_then(|list| list.iter().find(|item| item["name"] == "gmail"))
            .expect("gmail status");
        assert_eq!(gmail["authorization"]["method"], "google_oauth");
        assert_eq!(gmail["authorization"]["setup_required"], false);
        assert_eq!(gmail["authorization"]["authorized"], false);

        let calendar = configured
            .as_array()
            .and_then(|list| list.iter().find(|item| item["name"] == "calendar"))
            .expect("calendar status");
        assert_eq!(calendar["authorization"]["method"], "google_oauth");
        assert_eq!(calendar["authorization"]["setup_required"], true);
        assert_eq!(calendar["authorization"]["authorized"], false);
    }

    #[tokio::test]
    async fn status_does_not_leak_google_token_values() {
        let (directory, state) = test_state();
        let token_path = directory.path().join("google-token.json");
        let secret = "top-secret-token-value";
        write_private_fixture(
            &token_path,
            &format!("{{\"access_token\":\"{secret}\",\"client_id\":\"id\"}}"),
        );
        let mut config: Config = toml::from_str(&format!(
            r#"
            [[sources]]
            name = "gmail"
            kind = "google-drive"
            enabled = true
            project = "work"
            token = "{token_path_display}"
            acl = ["work"]
            "#,
            token_path_display = token_path.display(),
        ))
        .expect("configuration");
        config.data_dir = directory.path().to_path_buf();

        let response = router(state.with_config(&config, false))
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("status response");
        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("status body");
        let text = String::from_utf8(body.clone().to_vec()).expect("status text");
        assert!(text.contains("gmail"));
        assert!(!text.contains("top-secret-token-value"));

        let value: serde_json::Value = serde_json::from_slice(&body).expect("status JSON");
        let configured_source = &value["ingestion"]["configured_sources"][0];
        assert_eq!(configured_source["name"], "gmail");
        assert_eq!(configured_source["authorization"]["method"], "google_oauth");
        let authorization = configured_source["authorization"]
            .as_object()
            .expect("authorization summary");
        assert_eq!(authorization.len(), 3);
    }

    #[tokio::test]
    async fn answer_returns_a_bounded_extractive_fallback_without_a_model() {
        let (_directory, state) = test_state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/answer")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"query":"unknown topic","project":null,"source":null}"#,
                    ))
                    .expect("request"),
            )
            .await
            .expect("answer response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("answer body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("answer JSON");
        assert_eq!(value["mode"], "extractive");
        assert_eq!(value["retrieval_mode"], "hybrid");
        assert_eq!(value["retrieval_degraded"], false);
        assert_eq!(value["cached"], false);
        assert_eq!(value["plan"]["model_generated"], false);
        assert_eq!(
            value["plan"]["queries"],
            serde_json::json!(["unknown topic"])
        );
        assert_eq!(value["evidence"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn search_surfaces_lexical_fallback_without_changing_the_json_shape() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .upsert(
                &Document {
                    source: "notes".into(),
                    source_id: "fallback-runbook".into(),
                    title: "Fallback runbook".into(),
                    content: "Use lexical retrieval while embeddings are offline.".into(),
                    uri: None,
                    updated_at: chrono::Utc::now(),
                    project: "demo".into(),
                    acl: Vec::new(),
                    metadata: serde_json::json!({}),
                },
                &[(
                    "Use lexical retrieval while embeddings are offline.".into(),
                    vec![1.0, 0.0],
                )],
            )
            .expect("document");
        let state = AppState::new(store, Arc::new(UnavailableEmbedder));
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/search")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"query":"lexical retrieval","project":"demo","source":null,"limit":10}"#,
                    ))
                    .expect("search request"),
            )
            .await
            .expect("search response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("x-cortana-retrieval-mode")
                .and_then(|value| value.to_str().ok()),
            Some("lexical-fallback")
        );
        assert_eq!(
            response
                .headers()
                .get("x-cortana-retrieval-degraded")
                .and_then(|value| value.to_str().ok()),
            Some("true")
        );
        assert_eq!(
            response
                .headers()
                .get("x-cortana-retrieval-ranking")
                .and_then(|value| value.to_str().ok()),
            Some("cortana.retrieval.ranking.v2")
        );
        assert!(
            response
                .headers()
                .get("x-cortana-retrieval-candidates")
                .and_then(|value| value.to_str().ok())
                .is_some()
        );
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("search body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("search JSON");
        assert!(value.is_array());
        assert_eq!(value.as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn document_routes_page_content_and_reject_invalid_cursors() {
        let (_directory, state) = test_state();
        for (index, content) in ["first body", "second body"].into_iter().enumerate() {
            state
                .store
                .upsert(
                    &Document {
                        source: "notes".into(),
                        source_id: format!("note-{index}"),
                        title: format!("Note {index}"),
                        content: content.into(),
                        uri: Some(format!("https://example.test/{index}")),
                        updated_at: chrono::Utc::now()
                            - chrono::Duration::seconds(i64::try_from(index).unwrap_or_default()),
                        project: "demo".into(),
                        acl: Vec::new(),
                        metadata: serde_json::json!({"kind": "note"}),
                    },
                    &[(content.into(), vec![1.0; 16])],
                )
                .expect("document");
        }
        let app = router(state);
        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/documents?project=demo&limit=1")
                    .body(Body::empty())
                    .expect("list request"),
            )
            .await
            .expect("list response");
        assert_eq!(first.status(), StatusCode::OK);
        let first_body = to_bytes(first.into_body(), 1024 * 1024)
            .await
            .expect("list body");
        let first_value: serde_json::Value =
            serde_json::from_slice(&first_body).expect("list JSON");
        assert_eq!(first_value["documents"].as_array().map(Vec::len), Some(1));
        assert_eq!(first_value["documents"][0]["source_id"], "note-0");
        let cursor = first_value["next_cursor"].as_str().expect("next cursor");
        let id = first_value["documents"][0]["id"]
            .as_str()
            .expect("document id");

        let second = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/documents?project=demo&limit=1&cursor={cursor}"
                    ))
                    .body(Body::empty())
                    .expect("second request"),
            )
            .await
            .expect("second response");
        assert_eq!(second.status(), StatusCode::OK);
        let detail = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/documents/{id}"))
                    .body(Body::empty())
                    .expect("detail request"),
            )
            .await
            .expect("detail response");
        assert_eq!(detail.status(), StatusCode::OK);
        let detail_body = to_bytes(detail.into_body(), 1024 * 1024)
            .await
            .expect("detail body");
        let detail_value: serde_json::Value =
            serde_json::from_slice(&detail_body).expect("detail JSON");
        assert_eq!(detail_value["content"], "first body");
        assert_eq!(detail_value["metadata"]["kind"], "note");
        assert_eq!(detail_value["source_id"], "note-0");
        assert_eq!(detail_value["acl"], serde_json::json!([]));
        assert_eq!(
            detail_value["surrounding"][0]["source_id"],
            serde_json::json!("note-1")
        );

        let filtered = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/documents?project=demo&query=Note%201&limit=10")
                    .body(Body::empty())
                    .expect("filtered request"),
            )
            .await
            .expect("filtered response");
        assert_eq!(filtered.status(), StatusCode::OK);
        let filtered_body = to_bytes(filtered.into_body(), 1024 * 1024)
            .await
            .expect("filtered body");
        let filtered_value: serde_json::Value =
            serde_json::from_slice(&filtered_body).expect("filtered JSON");
        assert_eq!(
            filtered_value["documents"][0]["source_id"],
            serde_json::json!("note-1")
        );

        let blank_filter = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/documents?project=demo&query=%20%20&limit=10")
                    .body(Body::empty())
                    .expect("blank filter request"),
            )
            .await
            .expect("blank filter response");
        assert_eq!(blank_filter.status(), StatusCode::BAD_REQUEST);

        let padded_filter_query = format!("%20{}%20", "x".repeat(MAX_DOCUMENT_QUERY_LENGTH));
        let padded_filter = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/documents?project=demo&query={padded_filter_query}&limit=10"
                    ))
                    .body(Body::empty())
                    .expect("padded filter request"),
            )
            .await
            .expect("padded filter response");
        assert_eq!(padded_filter.status(), StatusCode::BAD_REQUEST);

        let control_filter = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/documents?project=demo&query=%00&limit=10")
                    .body(Body::empty())
                    .expect("control filter request"),
            )
            .await
            .expect("control filter response");
        assert_eq!(control_filter.status(), StatusCode::BAD_REQUEST);

        let graph = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/graph?project=demo&limit=10")
                    .body(Body::empty())
                    .expect("graph request"),
            )
            .await
            .expect("graph response");
        assert_eq!(graph.status(), StatusCode::OK);
        let graph_body = to_bytes(graph.into_body(), 1024 * 1024)
            .await
            .expect("graph body");
        let graph_value: serde_json::Value =
            serde_json::from_slice(&graph_body).expect("graph JSON");
        assert_eq!(graph_value["nodes"].as_array().map(Vec::len), Some(4));
        assert_eq!(graph_value["edges"].as_array().map(Vec::len), Some(3));

        let invalid = app
            .oneshot(
                Request::builder()
                    .uri("/v1/documents?cursor=not-valid-base64")
                    .body(Body::empty())
                    .expect("invalid request"),
            )
            .await
            .expect("invalid response");
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn document_cursor_decode_rejects_invalid_payloads() {
        assert!(decode_document_cursor("not-valid-base64").is_err());

        let malformed_json = URL_SAFE_NO_PAD.encode("not-json");
        assert!(decode_document_cursor(&malformed_json).is_err());

        let oversized = URL_SAFE_NO_PAD.encode(vec![b'X'; 513]);
        assert!(decode_document_cursor(&oversized).is_err());

        let now = chrono::Utc::now().to_rfc3339();
        let non_hex_id = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&EncodedDocumentCursor {
                updated_at: now.clone(),
                id: "not-a-hex-id".into(),
            })
            .expect("encode invalid id cursor"),
        );
        let non_hex = decode_document_cursor(&non_hex_id).expect_err("invalid ids should fail");
        assert_eq!(
            non_hex,
            (
                StatusCode::BAD_REQUEST,
                "invalid document cursor".to_string()
            )
        );

        let invalid_updated_at = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&EncodedDocumentCursor {
                updated_at: "2026-13-01T00:00:00Z".into(),
                id: "deadbeef".into(),
            })
            .expect("encode invalid updated_at cursor"),
        );
        let invalid_time = decode_document_cursor(&invalid_updated_at)
            .expect_err("invalid timestamps should fail");
        assert_eq!(
            invalid_time,
            (
                StatusCode::BAD_REQUEST,
                "invalid document cursor".to_string()
            )
        );

        let valid = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&EncodedDocumentCursor {
                updated_at: now.clone(),
                id: "feedface".into(),
            })
            .expect("encode valid cursor"),
        );
        let decoded = decode_document_cursor(&valid).expect("decode valid cursor");
        assert_eq!(decoded.id, "feedface");
        assert_eq!(decoded.updated_at, now);
    }

    #[tokio::test]
    async fn scoped_tokens_filter_evidence_and_admin_only_audit_omits_queries() {
        let (_directory, state) = test_state();
        state
            .store
            .upsert(
                &Document {
                    source: "notes".into(),
                    source_id: "personal".into(),
                    title: "Personal secret".into(),
                    content: "classified launch phrase".into(),
                    uri: None,
                    updated_at: chrono::Utc::now(),
                    project: "demo".into(),
                    acl: vec!["personal".into()],
                    metadata: serde_json::json!({}),
                },
                &[("classified launch phrase".into(), vec![1.0; 16])],
            )
            .expect("private document");
        state
            .store
            .upsert(
                &Document {
                    source: "notes".into(),
                    source_id: "work".into(),
                    title: "Work note".into(),
                    content: "shared launch phrase".into(),
                    uri: None,
                    updated_at: chrono::Utc::now(),
                    project: "demo".into(),
                    acl: vec!["work".into()],
                    metadata: serde_json::json!({}),
                },
                &[("shared launch phrase".into(), vec![1.0; 16])],
            )
            .expect("work document");
        let mut config = Config::default();
        config.auth.tokens = vec![
            AuthTokenConfig {
                principal: "work-agent".into(),
                token_env: "WORK_TOKEN".into(),
                scopes: vec!["query".into(), "status".into()],
                acl: vec!["work".into()],
            },
            AuthTokenConfig {
                principal: "auditor".into(),
                token_env: "ADMIN_TOKEN".into(),
                scopes: vec!["admin".into()],
                acl: Vec::new(),
            },
            AuthTokenConfig {
                principal: "personal-agent".into(),
                token_env: "PERSONAL_TOKEN".into(),
                scopes: vec!["query".into(), "status".into()],
                acl: vec!["personal".into()],
            },
        ];
        config
            .environment
            .insert("WORK_TOKEN".into(), "work-secret".into());
        config
            .environment
            .insert("ADMIN_TOKEN".into(), "admin-secret".into());
        config
            .environment
            .insert("PERSONAL_TOKEN".into(), "personal-secret".into());
        let policy = AuthPolicy::from_config(&config).expect("auth policy");
        let app = router(state.with_config(&config, false).with_auth_policy(policy));
        let graph = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/graph?project=demo&limit=10")
                    .header(header::AUTHORIZATION, "Bearer work-secret")
                    .body(Body::empty())
                    .expect("graph request"),
            )
            .await
            .expect("graph response");
        assert_eq!(graph.status(), StatusCode::OK);
        let graph_body = to_bytes(graph.into_body(), 1024 * 1024)
            .await
            .expect("graph body");
        let graph_value: serde_json::Value =
            serde_json::from_slice(&graph_body).expect("graph JSON");
        let document_nodes = graph_value["nodes"]
            .as_array()
            .expect("graph nodes")
            .iter()
            .filter(|node| node["kind"] == "document")
            .collect::<Vec<_>>();
        assert_eq!(document_nodes.len(), 1);
        assert_eq!(document_nodes[0]["label"], "Work note");

        let search = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/search")
                    .header(header::AUTHORIZATION, "Bearer work-secret")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"query":"launch phrase","project":"demo","limit":10}"#,
                    ))
                    .expect("search request"),
            )
            .await
            .expect("search response");
        assert_eq!(search.status(), StatusCode::OK);
        let search_body = to_bytes(search.into_body(), 1024 * 1024)
            .await
            .expect("search body");
        let rows: Vec<Evidence> = serde_json::from_slice(&search_body).expect("search JSON");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source_id, "work");

        let status = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(header::AUTHORIZATION, "Bearer work-secret")
                    .body(Body::empty())
                    .expect("status request"),
            )
            .await
            .expect("status response");
        assert_eq!(status.status(), StatusCode::OK);
        let status_body = to_bytes(status.into_body(), 1024 * 1024)
            .await
            .expect("status body");
        let status_value: serde_json::Value =
            serde_json::from_slice(&status_body).expect("status JSON");
        assert_eq!(status_value["documents"], 1);
        assert_eq!(status_value["chunks"], 1);
        assert_eq!(status_value["sources"].as_array().map(Vec::len), Some(1));
        assert_eq!(status_value["searches_total"], 1);
        assert_eq!(status_value["contexts_total"], 0);
        assert_eq!(status_value["answers_total"], 0);
        assert_eq!(status_value["errors_total"], 0);

        let personal_search = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/search")
                    .header(header::AUTHORIZATION, "Bearer personal-secret")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"query":"launch phrase","project":"demo","limit":10}"#,
                    ))
                    .expect("personal search request"),
            )
            .await
            .expect("personal search response");
        assert_eq!(personal_search.status(), StatusCode::OK);

        let work_status_after_personal_request = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(header::AUTHORIZATION, "Bearer work-secret")
                    .body(Body::empty())
                    .expect("scoped status request"),
            )
            .await
            .expect("scoped status response");
        let work_status_body =
            to_bytes(work_status_after_personal_request.into_body(), 1024 * 1024)
                .await
                .expect("scoped status body");
        let work_status_value: serde_json::Value =
            serde_json::from_slice(&work_status_body).expect("scoped status JSON");
        assert_eq!(work_status_value["searches_total"], 1);

        let forbidden = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/audit")
                    .header(header::AUTHORIZATION, "Bearer work-secret")
                    .body(Body::empty())
                    .expect("forbidden request"),
            )
            .await
            .expect("forbidden response");
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

        let audit = app
            .oneshot(
                Request::builder()
                    .uri("/v1/audit?limit=10")
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .body(Body::empty())
                    .expect("audit request"),
            )
            .await
            .expect("audit response");
        assert_eq!(audit.status(), StatusCode::OK);
        let audit_body = to_bytes(audit.into_body(), 1024 * 1024)
            .await
            .expect("audit body");
        let value: serde_json::Value = serde_json::from_slice(&audit_body).expect("audit JSON");
        let search_events = value
            .as_array()
            .expect("audit events")
            .iter()
            .filter(|event| event["action"] == "search")
            .collect::<Vec<_>>();
        assert!(
            search_events
                .iter()
                .any(|event| event["principal"] == "work-agent")
        );
        assert!(
            search_events
                .iter()
                .any(|event| event["principal"] == "personal-agent")
        );
        assert!(
            search_events
                .iter()
                .all(|event| event.get("query").is_none())
        );
    }

    #[tokio::test]
    async fn admin_with_acl_scoped_labels_can_read_documents_outside_acl_and_retrieval_scopes() {
        let (_directory, state) = test_state();
        state
            .store
            .upsert(
                &Document {
                    source: "notes".into(),
                    source_id: "personal-note".into(),
                    title: "Personal note".into(),
                    content: "personal launch phrase".into(),
                    uri: None,
                    updated_at: chrono::Utc::now(),
                    project: "demo".into(),
                    acl: vec!["personal".into()],
                    metadata: serde_json::json!({}),
                },
                &[("personal launch phrase".into(), vec![1.0; 16])],
            )
            .expect("personal document");
        state
            .store
            .upsert(
                &Document {
                    source: "notes".into(),
                    source_id: "work-note".into(),
                    title: "Work note".into(),
                    content: "work launch phrase".into(),
                    uri: None,
                    updated_at: chrono::Utc::now(),
                    project: "demo".into(),
                    acl: vec!["work".into()],
                    metadata: serde_json::json!({}),
                },
                &[("work launch phrase".into(), vec![1.0; 16])],
            )
            .expect("work document");
        state
            .store
            .remember(&MemoryInput {
                kind: "semantic".into(),
                project: "demo".into(),
                title: "Launch memory".into(),
                content: "The shared launch phrase is approved for work.".into(),
                source: "agent".into(),
                source_id: String::new(),
                dedupe_key: Some("api:admin-launch-memory".into()),
                confidence: 0.9,
                importance: 0.8,
                acl: vec!["work".into()],
                provenance: serde_json::json!({"test": true}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect("memory");

        let mut config = Config::default();
        config
            .environment
            .insert("WORK_TOKEN".into(), "work-secret".into());
        config
            .environment
            .insert("ADMIN_TOKEN".into(), "admin-secret".into());
        config.auth.tokens = vec![
            AuthTokenConfig {
                principal: "work-agent".into(),
                token_env: "WORK_TOKEN".into(),
                scopes: vec![QUERY_SCOPE.into(), STATUS_SCOPE.into()],
                acl: vec!["work".into()],
            },
            AuthTokenConfig {
                principal: "admin-agent".into(),
                token_env: "ADMIN_TOKEN".into(),
                scopes: vec![QUERY_SCOPE.into(), ADMIN_SCOPE.into(), MEMORY_SCOPE.into()],
                acl: vec!["work".into()],
            },
        ];
        let policy = AuthPolicy::from_config(&config).expect("auth policy");
        let app = router(state.with_config(&config, false).with_auth_policy(policy));

        let work_search = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/search")
                    .header(header::AUTHORIZATION, "Bearer work-secret")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"query":"launch phrase","project":"demo","limit":10}"#,
                    ))
                    .expect("work search request"),
            )
            .await
            .expect("work search response");
        assert_eq!(work_search.status(), StatusCode::OK);
        let work_rows: Vec<Evidence> = serde_json::from_slice(
            &to_bytes(work_search.into_body(), 1024 * 1024)
                .await
                .expect("work search body"),
        )
        .expect("work evidence");
        assert_eq!(work_rows.len(), 1);
        assert_eq!(work_rows[0].source_id, "work-note");

        let admin_search = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/search")
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"query":"launch phrase","project":"demo","limit":10}"#,
                    ))
                    .expect("admin search request"),
            )
            .await
            .expect("admin search response");
        assert_eq!(admin_search.status(), StatusCode::OK);
        let admin_rows: Vec<Evidence> = serde_json::from_slice(
            &to_bytes(admin_search.into_body(), 1024 * 1024)
                .await
                .expect("admin search body"),
        )
        .expect("admin evidence");
        assert_eq!(admin_rows.len(), 2);
        let admin_source_ids = admin_rows
            .iter()
            .map(|row| row.source_id.as_str())
            .collect::<BTreeSet<_>>();
        assert!(admin_source_ids.contains("personal-note"));
        assert!(admin_source_ids.contains("work-note"));

        let work_graph = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/graph?project=demo&limit=20")
                    .header(header::AUTHORIZATION, "Bearer work-secret")
                    .body(Body::empty())
                    .expect("work graph request"),
            )
            .await
            .expect("work graph response");
        let work_graph_value = serde_json::from_slice::<serde_json::Value>(
            &to_bytes(work_graph.into_body(), 1024 * 1024)
                .await
                .expect("work graph body"),
        )
        .expect("work graph JSON");
        let work_graph_nodes = work_graph_value["nodes"]
            .as_array()
            .expect("work graph nodes")
            .iter()
            .filter(|node| node["kind"] == "document")
            .collect::<Vec<_>>();
        assert_eq!(work_graph_nodes.len(), 1);

        let admin_graph = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/graph?project=demo&limit=20")
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .body(Body::empty())
                    .expect("admin graph request"),
            )
            .await
            .expect("admin graph response");
        let admin_graph_value = serde_json::from_slice::<serde_json::Value>(
            &to_bytes(admin_graph.into_body(), 1024 * 1024)
                .await
                .expect("admin graph body"),
        )
        .expect("admin graph JSON");
        let admin_graph_nodes = admin_graph_value["nodes"]
            .as_array()
            .expect("admin graph nodes")
            .iter()
            .filter(|node| node["kind"] == "document")
            .collect::<Vec<_>>();
        assert_eq!(admin_graph_nodes.len(), 2);

        let work_context = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/context")
                    .header(header::AUTHORIZATION, "Bearer work-secret")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"query":"launch phrase","project":"demo","limit":10}"#,
                    ))
                    .expect("work context request"),
            )
            .await
            .expect("work context response");
        assert_eq!(work_context.status(), StatusCode::OK);
        let work_context_value = serde_json::from_slice::<serde_json::Value>(
            &to_bytes(work_context.into_body(), 1024 * 1024)
                .await
                .expect("work context body"),
        )
        .expect("work context JSON");
        assert_eq!(
            work_context_value["evidence"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            work_context_value["contract_version"],
            crate::contracts::CONTEXT_CONTRACT_VERSION
        );
        assert!(
            work_context_value["context_bundle_id"]
                .as_str()
                .is_some_and(|value| value.starts_with("ctx_"))
        );
        assert_eq!(
            work_context_value["retrieval_contract_version"],
            crate::contracts::RETRIEVAL_CONTRACT_VERSION
        );
        assert_eq!(
            work_context_value["canonical_digest"]
                .as_str()
                .map(str::len),
            Some(64)
        );
        assert_eq!(
            work_context_value["privacy_scope_digest"]
                .as_str()
                .map(str::len),
            Some(64)
        );

        let admin_context = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/context")
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"query":"launch phrase","project":"demo","limit":10}"#,
                    ))
                    .expect("admin context request"),
            )
            .await
            .expect("admin context response");
        assert_eq!(admin_context.status(), StatusCode::OK);
        let admin_context_value = serde_json::from_slice::<serde_json::Value>(
            &to_bytes(admin_context.into_body(), 1024 * 1024)
                .await
                .expect("admin context body"),
        )
        .expect("admin context JSON");
        assert_eq!(
            admin_context_value["evidence"].as_array().map(Vec::len),
            Some(2)
        );

        let work_answer = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/answer")
                    .header(header::AUTHORIZATION, "Bearer work-secret")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"query":"launch phrase","project":"demo"}"#))
                    .expect("work answer request"),
            )
            .await
            .expect("work answer response");
        assert_eq!(work_answer.status(), StatusCode::OK);
        let work_answer_value = serde_json::from_slice::<serde_json::Value>(
            &to_bytes(work_answer.into_body(), 1024 * 1024)
                .await
                .expect("work answer body"),
        )
        .expect("work answer JSON");
        assert_eq!(
            work_answer_value["evidence"].as_array().map(Vec::len),
            Some(1)
        );
        assert!(work_answer_value.get("memories").is_none());

        let admin_answer = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/answer")
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"query":"launch phrase","project":"demo"}"#))
                    .expect("admin answer request"),
            )
            .await
            .expect("admin answer response");
        assert_eq!(admin_answer.status(), StatusCode::OK);
        let admin_answer_value = serde_json::from_slice::<serde_json::Value>(
            &to_bytes(admin_answer.into_body(), 1024 * 1024)
                .await
                .expect("admin answer body"),
        )
        .expect("admin answer JSON");
        assert_eq!(
            admin_answer_value["evidence"].as_array().map(Vec::len),
            Some(2)
        );
        assert_eq!(
            admin_answer_value["memories"].as_array().map(Vec::len),
            Some(1)
        );

        let admin_list = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/documents?project=demo&source=notes&limit=10")
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .body(Body::empty())
                    .expect("admin documents request"),
            )
            .await
            .expect("admin documents response");
        assert_eq!(admin_list.status(), StatusCode::OK);
        let admin_documents = serde_json::from_slice::<serde_json::Value>(
            &to_bytes(admin_list.into_body(), 1024 * 1024)
                .await
                .expect("admin documents body"),
        )
        .expect("admin documents JSON");
        let personal_document = admin_documents["documents"]
            .as_array()
            .expect("admin documents list")
            .iter()
            .find(|document| document["source_id"] == "personal-note")
            .expect("personal document from admin list");
        let personal_id = personal_document["id"]
            .as_str()
            .expect("personal document id");

        let work_personal_read = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/documents/{personal_id}"))
                    .header(header::AUTHORIZATION, "Bearer work-secret")
                    .body(Body::empty())
                    .expect("work personal read request"),
            )
            .await
            .expect("work personal read response");
        assert_eq!(work_personal_read.status(), StatusCode::NOT_FOUND);

        let admin_personal_read = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/documents/{personal_id}"))
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .body(Body::empty())
                    .expect("admin personal read request"),
            )
            .await
            .expect("admin personal read response");
        assert_eq!(admin_personal_read.status(), StatusCode::OK);
    }
}
