use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
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
    config::Config,
    context::{self as context_bundle, ContextBundle},
    embed::Embedder,
    model::Evidence,
    retrieval,
    store::{Store, StoreStats},
};

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub embedder: Arc<dyn Embedder>,
    metrics: Arc<RuntimeMetrics>,
    api_token: Option<Arc<str>>,
    ingestion: Arc<IngestionStatus>,
}

impl AppState {
    pub fn new(store: Store, embedder: Arc<dyn Embedder>, api_token: Option<String>) -> Self {
        Self {
            store,
            embedder,
            metrics: Arc::new(RuntimeMetrics::new()),
            api_token: api_token.map(Arc::from),
            ingestion: Arc::new(IngestionStatus::default()),
        }
    }

    pub fn with_config(mut self, config: &Config, scheduled: bool) -> Self {
        self.ingestion = Arc::new(IngestionStatus::from_config(config, scheduled));
        self
    }
}

struct RuntimeMetrics {
    started: Instant,
    searches: AtomicU64,
    contexts: AtomicU64,
    errors: AtomicU64,
}

impl RuntimeMetrics {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            searches: AtomicU64::new(0),
            contexts: AtomicU64::new(0),
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
    errors_total: u64,
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
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if state.api_token.is_none() || matches!(path, "/healthz" | "/readyz") {
        return next.run(request).await;
    }
    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .zip(state.api_token.as_deref())
        .is_some_and(|(provided, expected)| provided == expected);
    if authorized {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer realm=\"cortana\"")],
            "authorization required",
        )
            .into_response()
    }
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

async fn status(State(state): State<AppState>) -> Result<Json<Status>, (StatusCode, String)> {
    state
        .store
        .stats()
        .map(|stats| {
            Json(Status {
                status: "ok",
                uptime_seconds: state.metrics.uptime_seconds(),
                searches_total: state.metrics.searches.load(Ordering::Relaxed),
                contexts_total: state.metrics.contexts.load(Ordering::Relaxed),
                errors_total: state.metrics.errors.load(Ordering::Relaxed),
                ingestion: (*state.ingestion).clone(),
                stats,
            })
        })
        .map_err(internal_error)
}

async fn search(
    State(state): State<AppState>,
    Json(request): Json<SearchRequest>,
) -> Result<Json<Vec<Evidence>>, (StatusCode, String)> {
    validate_query(&request.query)?;
    state.metrics.searches.fetch_add(1, Ordering::Relaxed);
    match retrieval::retrieve(
        &state.store,
        &state.embedder,
        &request.query,
        request.project.as_deref(),
        request.source.as_deref(),
        request.limit.min(50),
    )
    .await
    {
        Ok(evidence) => Ok(Json(evidence)),
        Err(error) => {
            state.metrics.errors.fetch_add(1, Ordering::Relaxed);
            Err(internal_error(error))
        }
    }
}

async fn context(
    State(state): State<AppState>,
    Json(request): Json<ContextRequest>,
) -> Result<Json<ContextBundle>, (StatusCode, String)> {
    validate_query(&request.query)?;
    state.metrics.contexts.fetch_add(1, Ordering::Relaxed);
    let evidence = retrieval::retrieve(
        &state.store,
        &state.embedder,
        &request.query,
        request.project.as_deref(),
        request.source.as_deref(),
        request.limit.min(50),
    )
    .await
    .map_err(|error| {
        state.metrics.errors.fetch_add(1, Ordering::Relaxed);
        internal_error(error)
    })?;
    Ok(Json(context_bundle::build(
        &request.query,
        &evidence,
        request.max_tokens,
    )))
}

async fn metrics(State(state): State<AppState>) -> Result<impl IntoResponse, (StatusCode, String)> {
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
         # HELP cortana_search_requests_total Raw evidence search requests.\n\
         # TYPE cortana_search_requests_total counter\n\
         cortana_search_requests_total {}\n\
         # HELP cortana_context_requests_total Context bundle requests.\n\
         # TYPE cortana_context_requests_total counter\n\
         cortana_context_requests_total {}\n\
         # HELP cortana_query_errors_total Query pipeline errors.\n\
         # TYPE cortana_query_errors_total counter\n\
         cortana_query_errors_total {}\n",
        state.metrics.uptime_seconds(),
        stats.documents,
        stats.chunks,
        stats.embedding_cache_entries,
        stats.embedding_cache_hits,
        state.metrics.searches.load(Ordering::Relaxed),
        state.metrics.contexts.load(Ordering::Relaxed),
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

pub async fn serve(
    state: AppState,
    address: &str,
    web_dir: Option<&Path>,
    allow_remote: bool,
) -> Result<()> {
    let socket: std::net::SocketAddr = address.parse()?;
    let authenticated = state.api_token.is_some();
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

    use crate::embed::DeterministicEmbedder;

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
        assert_eq!(value["sync_runs"], serde_json::json!([]));
    }
}
