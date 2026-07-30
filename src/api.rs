use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Extension, Query as AxumQuery, State},
    http::{Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::{
    answer::{AnswerEngine, AnswerRequest, AnswerResponse, QueryRuntimeStatus},
    auth::{ADMIN_SCOPE, AuthPolicy, Principal, QUERY_SCOPE, STATUS_SCOPE},
    config::Config,
    context::{self as context_bundle, ContextBundle},
    embed::Embedder,
    model::Evidence,
    retrieval,
    store::{AuditEvent, Store, StoreStats},
};

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub embedder: Arc<dyn Embedder>,
    metrics: Arc<RuntimeMetrics>,
    auth: AuthPolicy,
    ingestion: Arc<IngestionStatus>,
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
            answer,
            audit_max_events: crate::config::AuthConfig::default().audit_max_events,
        }
    }

    pub fn with_config(mut self, config: &Config, scheduled: bool) -> Self {
        self.ingestion = Arc::new(IngestionStatus::from_config(config, scheduled));
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
    #[serde(flatten)]
    stats: StoreStats,
}

#[derive(Clone, Debug, Serialize)]
struct IngestionStatus {
    mode: &'static str,
    scheduled: bool,
    max_documents_per_source: usize,
    max_bytes_per_source: u64,
    max_duration_seconds: u64,
    request_concurrency: usize,
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
            })
            .collect();
        Self {
            mode: if scheduled { "scheduled" } else { "manual" },
            scheduled,
            max_documents_per_source: config.ingestion.max_documents_per_source,
            max_bytes_per_source: config.ingestion.max_bytes_per_source,
            max_duration_seconds: config.ingestion.max_duration_seconds,
            request_concurrency: config.ingestion.request_concurrency,
            configured_sources,
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { Json(Health { status: "ok" }) }))
        .route("/readyz", get(ready))
        .route("/metrics", get(metrics))
        .route("/v1/status", get(status))
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
            Json(Status {
                status: "ok",
                uptime_seconds: state.metrics.uptime_seconds(),
                searches_total: state.metrics.searches.load(Ordering::Relaxed),
                contexts_total: state.metrics.contexts.load(Ordering::Relaxed),
                answers_total: state.metrics.answers.load(Ordering::Relaxed),
                errors_total: state.metrics.errors.load(Ordering::Relaxed),
                query: state.answer.status(),
                ingestion: (*state.ingestion).clone(),
                stats,
            })
        })
        .map_err(internal_error)
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
    async fn status_reports_safe_ingestion_mode_and_configured_sources() {
        let (_directory, state) = test_state(None);
        let config: Config = toml::from_str(
            r#"
            [ingestion]
            max_documents_per_source = 25
            max_bytes_per_source = 4096
            max_duration_seconds = 45
            request_concurrency = 1

            [[sources]]
            name = "code"
            kind = "filesystem"
            enabled = false
            project = "work"
            source = "work-code"
            root = "/tmp/code"
            "#,
        )
        .expect("configuration");
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
        assert_eq!(value["ingestion"]["max_documents_per_source"], 25);
        assert_eq!(value["query"]["mode"], "extractive");
        assert_eq!(value["query"]["max_planned_queries"], 4);
        assert_eq!(value["query_cache_entries"], 0);
        assert_eq!(value["sync_runs"], serde_json::json!([]));
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
