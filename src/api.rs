use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
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
    auth::{ADMIN_SCOPE, AuthPolicy, Principal, QUERY_SCOPE, STATUS_SCOPE, acl_allows},
    config::{Config, WorkspaceConfig},
    context::{self as context_bundle, ContextBundle},
    embed::Embedder,
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

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub embedder: Arc<dyn Embedder>,
    metrics: Arc<RuntimeMetrics>,
    auth: AuthPolicy,
    ingestion: Arc<IngestionStatus>,
    workspaces: Arc<Vec<WorkspaceConfig>>,
    answer: AnswerEngine,
    audit_max_events: usize,
}

impl AppState {
    pub fn new(store: Store, embedder: Arc<dyn Embedder>, api_token: Option<String>) -> Self {
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
            auth: AuthPolicy::legacy(api_token),
            ingestion: Arc::new(IngestionStatus::default()),
            workspaces: Arc::new(Vec::new()),
            answer,
            audit_max_events: crate::config::AuthConfig::default().audit_max_events,
        }
    }

    pub fn with_config(mut self, config: &Config, scheduled: bool) -> Self {
        self.ingestion = Arc::new(IngestionStatus::from_config(config, scheduled));
        self.workspaces = Arc::new(config.workspaces.clone());
        self.audit_max_events = config.auth.audit_max_events;
        self
    }

    pub fn with_answer_engine(mut self, answer: AnswerEngine) -> Self {
        self.answer = answer;
        self
    }

    pub fn with_auth_policy(mut self, auth: AuthPolicy) -> Self {
        self.auth = auth;
        self
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

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct Status {
    status: &'static str,
    uptime_seconds: u64,
    searches_total: u64,
    contexts_total: u64,
    answers_total: u64,
    errors_total: u64,
    retrieval_fallbacks_total: u64,
    query: QueryRuntimeStatus,
    ingestion: IngestionStatus,
    workspaces: Vec<WorkspaceConfig>,
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
        .route("/v1/answer", post(answer))
        .route("/v1/audit", get(audit_events))
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
    let path = request.uri().path();
    if matches!(path, "/healthz" | "/readyz") {
        return next.run(request).await;
    }
    let principal = if state.auth.requires_token() {
        request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .and_then(|token| state.auth.authenticate(token))
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
        "/metrics" | "/v1/audit" => ADMIN_SCOPE,
        "/v1/status" => STATUS_SCOPE,
        _ => QUERY_SCOPE,
    };
    if !principal.has_scope(required_scope) {
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
    let result = state.store.list_documents_scoped(
        params.project.as_deref(),
        params.source.as_deref(),
        params.query.as_deref(),
        cursor.as_ref(),
        params.limit.clamp(1, 100),
        &principal.acl_labels(),
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
    match state
        .store
        .document_scoped(&id, &principal.acl_labels(), MAX_DOCUMENT_CONTENT_BYTES)
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
    validate_document_scope("project", params.project.as_deref())?;
    validate_document_scope("source", params.source.as_deref())?;
    validate_document_query(params.query.as_deref())?;
    let cursor = params
        .cursor
        .as_deref()
        .map(decode_document_cursor)
        .transpose()?;
    let started = Instant::now();
    let result = state.store.list_documents_scoped(
        params.project.as_deref(),
        params.source.as_deref(),
        params.query.as_deref(),
        cursor.as_ref(),
        params.limit.clamp(1, 100),
        &principal.acl_labels(),
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
                });
                edges.push(GraphEdge {
                    source: source_id,
                    target: format!("document:{}", document.id),
                    kind: "contains",
                });
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
    let result = match state.store.stats() {
        Ok(_) => state.embedder.probe().await,
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

async fn status(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Status>, (StatusCode, String)> {
    let acl = principal.acl_labels();
    let owner = principal.is_owner();
    let ingestion = state.ingestion.refreshed().visible_to(&principal);
    let stats = if owner {
        state.store.stats()
    } else {
        state
            .store
            .stats_scoped(&acl, &ingestion.visible_sync_source_keys())
    }
    .map_err(internal_error)?;
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
    Ok(Json(Status {
        status: "ok",
        uptime_seconds: state.metrics.uptime_seconds(),
        searches_total: counters.searches,
        contexts_total: counters.contexts,
        answers_total: counters.answers,
        errors_total: counters.errors,
        retrieval_fallbacks_total: counters.retrieval_fallbacks,
        query: state.answer.status(),
        ingestion,
        workspaces,
        stats,
    }))
}

fn fallback_workspaces(
    configured: &[WorkspaceConfig],
    source_projects: impl IntoIterator<Item = String>,
) -> Vec<WorkspaceConfig> {
    if !configured.is_empty() {
        return configured.iter().take(3).cloned().collect();
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
    let result = state
        .answer
        .answer_scoped(request, &principal.acl_labels())
        .await;
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
                Some(response.evidence.len()),
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
    match retrieval::retrieve_scoped_with_status(
        &state.store,
        &state.embedder,
        &request.query,
        request.project.as_deref(),
        request.source.as_deref(),
        request.limit.min(50),
        &principal.acl_labels(),
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
    let retrieval = match retrieval::retrieve_scoped_with_status(
        &state.store,
        &state.embedder,
        &request.query,
        request.project.as_deref(),
        request.source.as_deref(),
        request.limit.min(50),
        &principal.acl_labels(),
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
        Some(retrieval.evidence.len()),
        started,
    );
    Ok(Json(context_bundle::build_with_retrieval(
        &request.query,
        &retrieval.evidence,
        request.max_tokens,
        retrieval.mode.as_str(),
        retrieval.warning.as_deref(),
    )))
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
    let stats = state.store.stats().map_err(internal_error)?;
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

fn default_limit() -> usize {
    10
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
    let authenticated = state.auth.requires_token();
    anyhow::ensure!(
        socket.ip().is_loopback() || (allow_remote && authenticated),
        "refusing non-loopback bind without --allow-remote and a bearer token"
    );
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
        MAX_GOOGLE_TOKEN_BYTES, SourceAuthorizationMethod, source_authorization_summary,
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

    fn test_state(token: Option<String>) -> (tempfile::TempDir, AppState) {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .ensure_fingerprint("deterministic:16")
            .expect("fingerprint");
        let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));
        (directory, AppState::new(store, embedder, token))
    }

    struct UnavailableEmbedder;

    #[async_trait]
    impl Embedder for UnavailableEmbedder {
        async fn embed(&self, _input: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            anyhow::bail!("embedding provider unavailable")
        }

        fn fingerprint(&self) -> String {
            "unavailable:test".into()
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

    fn google_source(token: Option<std::path::PathBuf>) -> SourceConfig {
        SourceConfig {
            name: "personal-gmail".into(),
            kind: "gmail".into(),
            enabled: true,
            project: "personal".into(),
            root: None,
            source: None,
            channels: Vec::new(),
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
        std::fs::write(&token, vec![b'{'; MAX_GOOGLE_TOKEN_BYTES + 1]).expect("oversized fixture");
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
        assert_eq!(fallback_workspaces(&oversized, std::iter::empty()).len(), 3);
    }

    #[tokio::test]
    async fn health_is_public_but_api_and_metrics_require_configured_token() {
        let (_directory, state) = test_state(Some("secret".into()));
        let app = router(state);
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
    }

    #[tokio::test]
    async fn context_rejects_oversized_bodies() {
        let (_directory, state) = test_state(None);
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
        let (_directory, state) = test_state(None);
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
        let (_directory, state) = test_state(None);
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
        let (_directory, state) = test_state(None);
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
        let (directory, state) = test_state(None);
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
        let principal = AuthPolicy::from_config(&auth_config, None)
            .expect("policy")
            .authenticate("work-secret")
            .expect("principal");

        let status = IngestionStatus::from_config(&config, false).visible_to(&principal);
        let names = status
            .configured_sources
            .iter()
            .map(|source| source.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["work-drive", "public-reference"]);

        let mut admin_config = auth_config;
        admin_config.auth.tokens[0].scopes = vec![ADMIN_SCOPE.into()];
        let admin = AuthPolicy::from_config(&admin_config, None)
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
        let (_directory, state) = test_state(None);
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
        let policy = AuthPolicy::from_config(&config, None).expect("auth policy");
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
        let (directory, state) = test_state(None);
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
        let (directory, state) = test_state(None);
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
        let (_directory, state) = test_state(None);
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
        let state = AppState::new(store, Arc::new(UnavailableEmbedder), None);
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
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("search body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("search JSON");
        assert!(value.is_array());
        assert_eq!(value.as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn document_routes_page_content_and_reject_invalid_cursors() {
        let (_directory, state) = test_state(None);
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
        let (_directory, state) = test_state(None);
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
        let policy = AuthPolicy::from_config(&config, None).expect("auth policy");
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
}
