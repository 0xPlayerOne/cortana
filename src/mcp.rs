use std::sync::Arc;

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::Deserialize;

use crate::{context, embed::Embedder, retrieval, store::Store};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    query: String,
    project: Option<String>,
    source: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ContextParams {
    query: String,
    project: Option<String>,
    source: Option<String>,
    limit: Option<usize>,
    max_tokens: Option<usize>,
}

#[derive(Clone)]
pub struct BrainServer {
    store: Store,
    embedder: Arc<dyn Embedder>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl BrainServer {
    pub fn new(store: Store, embedder: Arc<dyn Embedder>) -> Self {
        Self {
            store,
            embedder,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Hybrid semantic and exact-term search across configured knowledge sources"
    )]
    async fn search(&self, Parameters(params): Parameters<SearchParams>) -> String {
        match retrieval::retrieve(
            &self.store,
            &self.embedder,
            &params.query,
            params.project.as_deref(),
            params.source.as_deref(),
            params.limit.unwrap_or(10),
        )
        .await
        {
            Ok(rows) => serde_json::to_string(&rows).unwrap_or_else(|error| error.to_string()),
            Err(error) => format!("retrieval error: {error}"),
        }
    }

    #[tool(
        description = "Build a token-bounded, citation-ready context bundle. Agents should prefer this tool before answering questions about the user or their work."
    )]
    async fn context(&self, Parameters(params): Parameters<ContextParams>) -> String {
        match retrieval::retrieve(
            &self.store,
            &self.embedder,
            &params.query,
            params.project.as_deref(),
            params.source.as_deref(),
            params.limit.unwrap_or(20),
        )
        .await
        {
            Ok(rows) => serde_json::to_string(&context::build(
                &params.query,
                &rows,
                params.max_tokens.unwrap_or(8_000),
            ))
            .unwrap_or_else(|error| error.to_string()),
            Err(error) => format!("retrieval error: {error}"),
        }
    }

    #[tool(
        description = "Report index health, source coverage, embedding identity, and persistent embedding-cache telemetry"
    )]
    async fn brain_status(&self) -> String {
        match self.store.stats() {
            Ok(stats) => serde_json::to_string(&stats).unwrap_or_else(|error| error.to_string()),
            Err(error) => format!("status error: {error}"),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for BrainServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Call context before answering questions about the user or their work. Reuse its citation-ready output instead of repeating broad discovery calls.",
        )
    }
}

pub async fn serve(server: BrainServer) -> anyhow::Result<()> {
    server.serve(stdio()).await?.waiting().await?;
    Ok(())
}
