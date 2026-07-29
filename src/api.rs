use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};

use crate::{
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
    #[serde(flatten)]
    stats: StoreStats,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { Json(Health { status: "ok" }) }))
        .route("/v1/status", get(status))
        .route("/v1/search", post(search))
        .route("/v1/context", post(context))
        .with_state(state)
}

async fn status(State(state): State<AppState>) -> Result<Json<Status>, (StatusCode, String)> {
    state
        .store
        .stats()
        .map(|stats| {
            Json(Status {
                status: "ok",
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
    retrieval::retrieve(
        &state.store,
        &state.embedder,
        &request.query,
        request.project.as_deref(),
        request.source.as_deref(),
        request.limit.min(50),
    )
    .await
    .map(Json)
    .map_err(internal_error)
}

async fn context(
    State(state): State<AppState>,
    Json(request): Json<ContextRequest>,
) -> Result<Json<ContextBundle>, (StatusCode, String)> {
    validate_query(&request.query)?;
    let evidence = retrieval::retrieve(
        &state.store,
        &state.embedder,
        &request.query,
        request.project.as_deref(),
        request.source.as_deref(),
        request.limit.min(50),
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(context_bundle::build(
        &request.query,
        &evidence,
        request.max_tokens,
    )))
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

pub async fn serve(state: AppState, address: &str, web_dir: Option<&Path>) -> Result<()> {
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
    axum::serve(listener, app).await?;
    Ok(())
}
