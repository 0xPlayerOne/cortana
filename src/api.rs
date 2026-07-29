use std::sync::Arc;

use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::{embed::Embedder, model::Evidence, retrieval, store::Store};

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

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { Json(Health { status: "ok" }) }))
        .route("/v1/search", post(search))
        .with_state(state)
}

async fn search(
    State(state): State<AppState>,
    Json(request): Json<SearchRequest>,
) -> Result<Json<Vec<Evidence>>, (StatusCode, String)> {
    let vectors = state
        .embedder
        .embed(std::slice::from_ref(&request.query))
        .await
        .map_err(internal_error)?;
    retrieval::search(
        &state.store,
        &request.query,
        &vectors[0],
        request.project.as_deref(),
        request.source.as_deref(),
        request.limit.min(50),
    )
    .map(Json)
    .map_err(internal_error)
}

fn internal_error(error: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

fn default_limit() -> usize {
    10
}

pub async fn serve(state: AppState, address: &str) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, router(state)).await?;
    Ok(())
}
