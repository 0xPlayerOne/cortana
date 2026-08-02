use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Extension, Path as AxumPath, Query as AxumQuery, State},
    http::{Request, StatusCode, header},
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
    auth::{ADMIN_SCOPE, AuthPolicy, Principal, QUERY_SCOPE, STATUS_SCOPE},
    config::{Config, WorkspaceConfig},
    context::{self as context_bundle, ContextBundle},
    embed::Embedder,
    model::Evidence,
    retrieval,
    source_validation::{self, SourceValidationStatus},
    store::{AuditEvent, DocumentCursor, DocumentSummary, Store, StoreStats},
};

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
}

impl RuntimeMetrics {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            searches: AtomicU64::new(0),
            contexts: AtomicU64::new(0),
            answers: AtomicU64::new(0),
            errors: AtomicU64::new(0),
        }
    }

    fn uptime_seconds(&self) -> u64 {
        self.started.elapsed().as_secs()
    }
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    query: String,
    project: Option<String>,
    source: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
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
    validation_state_error: Option<String>,
    configured_sources: Vec<ConfiguredSourceStatus>,
}

impl Default for IngestionStatus {
    fn default() -> Self {
        Self::from_config(&Config::default(), false)
    }
}

#[derive(Clone, Debug, Serialize)]
struct ConfiguredSourceStatus {
    name: String,
    source: String,
    kind: String,
    project: String,
    enabled: bool,
    max_documents: usize,
    max_bytes: u64,
    max_duration_seconds: u64,
    validation: Option<SourceValidationStatus>,
}

impl IngestionStatus {
    fn from_config(config: &Config, scheduled: bool) -> Self {
        let configured_sources = config
            .sources
            .iter()
            .map(|source| ConfiguredSourceStatus {
                name: source.name.clone(),
                source: source.source.clone().unwrap_or_else(|| source.name.clone()),
                kind: source.kind.clone(),
                project: source.project.clone(),
                enabled: source.enabled,
                max_documents: source
                    .max_documents
                    .unwrap_or(config.ingestion.max_documents_per_source),
                max_bytes: source
                    .max_bytes
                    .unwrap_or(config.ingestion.max_bytes_per_source),
                max_duration_seconds: source
                    .max_duration_seconds
                    .unwrap_or(config.ingestion.max_duration_seconds),
                validation: None,
            })
            .collect();
        Self {
            data_dir: config.data_dir.clone(),
            mode: if scheduled { "scheduled" } else { "manual" },
            scheduled,
            max_documents_per_source: config.ingestion.max_documents_per_source,
            max_bytes_per_source: config.ingestion.max_bytes_per_source,
            max_duration_seconds: config.ingestion.max_duration_seconds,
            request_concurrency: config.ingestion.request_concurrency,
            validation_state_error: None,
            configured_sources,
        }
        .refreshed()
    }

    fn refreshed(&self) -> Self {
        let mut status = self.clone();
        match source_validation::load(&self.data_dir) {
            Ok(validations) => {
                status.validation_state_error = None;
                for source in &mut status.configured_sources {
                    source.validation = validations.get(&source.name).cloned();
                }
            }
            Err(error) => {
                status.validation_state_error = Some(error.to_string());
                for source in &mut status.configured_sources {
                    source.validation = None;
                }
            }
        }
        status
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
            state.metrics.errors.fetch_add(1, Ordering::Relaxed);
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
            state.metrics.errors.fetch_add(1, Ordering::Relaxed);
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
            state.metrics.errors.fetch_add(1, Ordering::Relaxed);
            Err(internal_error(error))
        }
    }
}

fn validate_document_scope(name: &str, value: Option<&str>) -> Result<(), (StatusCode, String)> {
    if value.is_some_and(|value| value.is_empty() || value.len() > MAX_DOCUMENT_SCOPE_LENGTH) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{name} must contain 1 to {MAX_DOCUMENT_SCOPE_LENGTH} bytes"),
        ));
    }
    Ok(())
}

fn validate_document_query(value: Option<&str>) -> Result<(), (StatusCode, String)> {
    if value.is_some_and(|value| value.is_empty() || value.len() > MAX_DOCUMENT_QUERY_LENGTH) {
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
    Extension(_principal): Extension<Principal>,
) -> Result<Json<Status>, (StatusCode, String)> {
    state
        .store
        .stats()
        .map(|stats| {
            let workspaces = fallback_workspaces(
                state.workspaces.as_ref(),
                stats
                    .sources
                    .iter()
                    .map(|source| source.project.clone())
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            Json(Status {
                status: "ok",
                uptime_seconds: state.metrics.uptime_seconds(),
                searches_total: state.metrics.searches.load(Ordering::Relaxed),
                contexts_total: state.metrics.contexts.load(Ordering::Relaxed),
                answers_total: state.metrics.answers.load(Ordering::Relaxed),
                errors_total: state.metrics.errors.load(Ordering::Relaxed),
                query: state.answer.status(),
                ingestion: state.ingestion.refreshed(),
                workspaces,
                stats,
            })
        })
        .map_err(internal_error)
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
    validate_query(&request.query)?;
    let started = Instant::now();
    let project = request.project.clone();
    let source = request.source.clone();
    state.metrics.answers.fetch_add(1, Ordering::Relaxed);
    let result = state
        .answer
        .answer_scoped(request, &principal.acl_labels())
        .await;
    let (outcome, count) = match &result {
        Ok(response) => ("succeeded", Some(response.evidence.len())),
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
        state.metrics.errors.fetch_add(1, Ordering::Relaxed);
        internal_error(error)
    })
}

async fn search(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<SearchRequest>,
) -> Result<Json<Vec<Evidence>>, (StatusCode, String)> {
    validate_query(&request.query)?;
    let started = Instant::now();
    state.metrics.searches.fetch_add(1, Ordering::Relaxed);
    match retrieval::retrieve_scoped(
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
        Ok(evidence) => {
            record_audit(
                &state,
                &principal,
                "search",
                request.project.as_deref(),
                request.source.as_deref(),
                "succeeded",
                Some(evidence.len()),
                started,
            );
            Ok(Json(evidence))
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
            state.metrics.errors.fetch_add(1, Ordering::Relaxed);
            Err(internal_error(error))
        }
    }
}

async fn context(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(request): Json<ContextRequest>,
) -> Result<Json<ContextBundle>, (StatusCode, String)> {
    validate_query(&request.query)?;
    let started = Instant::now();
    state.metrics.contexts.fetch_add(1, Ordering::Relaxed);
    let evidence = match retrieval::retrieve_scoped(
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
        Ok(evidence) => evidence,
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
            state.metrics.errors.fetch_add(1, Ordering::Relaxed);
            return Err(internal_error(error));
        }
    };
    record_audit(
        &state,
        &principal,
        "context",
        request.project.as_deref(),
        request.source.as_deref(),
        "succeeded",
        Some(evidence.len()),
        started,
    );
    Ok(Json(context_bundle::build(
        &request.query,
        &evidence,
        request.max_tokens,
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
         cortana_query_errors_total {}\n",
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

fn internal_error(error: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
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
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tempfile::tempdir;
    use tower::ServiceExt;

    use crate::config::AuthTokenConfig;
    use crate::embed::DeterministicEmbedder;
    use crate::model::Document;

    use super::*;

    fn test_state(token: Option<String>) -> (tempfile::TempDir, AppState) {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("open store");
        store
            .ensure_fingerprint("deterministic:16")
            .expect("fingerprint");
        let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));
        (directory, AppState::new(store, embedder, token))
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
            root = "/tmp/code"
            "##,
        )
        .expect("configuration");
        config.data_dir = directory.path().to_path_buf();
        let state = state.with_config(&config, false);
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
                configuration_fingerprint: None,
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
        assert_eq!(value["ingestion"]["max_documents_per_source"], 25);
        assert_eq!(value["query"]["mode"], "extractive");
        assert_eq!(value["query"]["max_planned_queries"], 4);
        assert_eq!(value["query_cache_entries"], 0);
        assert_eq!(value["sync_runs"], serde_json::json!([]));
        assert_eq!(value["workspaces"][0]["id"], "work");
        assert_eq!(value["workspaces"][0]["account_label"], "team@example.com");
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
        assert_eq!(value["cached"], false);
        assert_eq!(value["plan"]["model_generated"], false);
        assert_eq!(
            value["plan"]["queries"],
            serde_json::json!(["unknown topic"])
        );
        assert_eq!(value["evidence"], serde_json::json!([]));
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
        ];
        config
            .environment
            .insert("WORK_TOKEN".into(), "work-secret".into());
        config
            .environment
            .insert("ADMIN_TOKEN".into(), "admin-secret".into());
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
        assert_eq!(value[0]["principal"], "work-agent");
        assert_eq!(value[0]["action"], "search");
        assert!(value[0].get("query").is_none());
    }
}
