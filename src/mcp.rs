use std::sync::Arc;

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::Deserialize;

use crate::{embed::Embedder, retrieval, store::Store};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    query: String,
    project: Option<String>,
    source: Option<String>,
    limit: Option<usize>,
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
        match self
            .embedder
            .embed(std::slice::from_ref(&params.query))
            .await
        {
            Ok(vectors) => match retrieval::search(
                &self.store,
                &params.query,
                &vectors[0],
                params.project.as_deref(),
                params.source.as_deref(),
                params.limit.unwrap_or(10).min(50),
            ) {
                Ok(rows) => serde_json::to_string(&rows).unwrap_or_else(|error| error.to_string()),
                Err(error) => format!("retrieval error: {error}"),
            },
            Err(error) => format!("embedding error: {error}"),
        }
    }
}

#[tool_handler]
impl ServerHandler for BrainServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Retrieve cited evidence from the user's scoped second brain before broad context discovery."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

pub async fn serve(server: BrainServer) -> anyhow::Result<()> {
    server.serve(stdio()).await?.waiting().await?;
    Ok(())
}
