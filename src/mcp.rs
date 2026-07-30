use std::sync::Arc;
use std::time::Instant;

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::Deserialize;

use crate::{
    auth::{Principal, QUERY_SCOPE, STATUS_SCOPE},
    context,
    embed::Embedder,
    retrieval,
    store::Store,
};

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
    audit_max_events: usize,
    principal: Principal,
}

#[tool_router]
impl BrainServer {
    pub fn new(store: Store, embedder: Arc<dyn Embedder>) -> Self {
        Self {
            store,
            embedder,
            tool_router: Self::tool_router(),
            audit_max_events: 10_000,
            principal: Principal::local("local-mcp"),
        }
    }

    pub fn with_principal(mut self, principal: Principal) -> Self {
        self.principal = principal;
        self
    }

    pub fn with_audit_limit(mut self, max_events: usize) -> Self {
        self.audit_max_events = max_events;
        self
    }

    #[tool(
        description = "Hybrid semantic and exact-term search across configured knowledge sources"
    )]
    async fn search(&self, Parameters(params): Parameters<SearchParams>) -> String {
        let started = Instant::now();
        if !self.principal.has_scope(QUERY_SCOPE) {
            self.audit(
                "mcp.search",
                params.project.as_deref(),
                params.source.as_deref(),
                "forbidden",
                None,
                started,
            );
            return "authorization error: query scope required".into();
        }
        match retrieval::retrieve_scoped(
            &self.store,
            &self.embedder,
            &params.query,
            params.project.as_deref(),
            params.source.as_deref(),
            params.limit.unwrap_or(10),
            &self.principal.acl_labels(),
        )
        .await
        {
            Ok(rows) => {
                self.audit(
                    "mcp.search",
                    params.project.as_deref(),
                    params.source.as_deref(),
                    "succeeded",
                    Some(rows.len()),
                    started,
                );
                serde_json::to_string(&rows).unwrap_or_else(|error| error.to_string())
            }
            Err(error) => {
                self.audit(
                    "mcp.search",
                    params.project.as_deref(),
                    params.source.as_deref(),
                    "failed",
                    None,
                    started,
                );
                format!("retrieval error: {error}")
            }
        }
    }

    #[tool(
        description = "Build a token-bounded, citation-ready context bundle. Agents should prefer this tool before answering questions about the user or their work."
    )]
    async fn context(&self, Parameters(params): Parameters<ContextParams>) -> String {
        let started = Instant::now();
        if !self.principal.has_scope(QUERY_SCOPE) {
            self.audit(
                "mcp.context",
                params.project.as_deref(),
                params.source.as_deref(),
                "forbidden",
                None,
                started,
            );
            return "authorization error: query scope required".into();
        }
        match retrieval::retrieve_scoped(
            &self.store,
            &self.embedder,
            &params.query,
            params.project.as_deref(),
            params.source.as_deref(),
            params.limit.unwrap_or(20),
            &self.principal.acl_labels(),
        )
        .await
        {
            Ok(rows) => {
                self.audit(
                    "mcp.context",
                    params.project.as_deref(),
                    params.source.as_deref(),
                    "succeeded",
                    Some(rows.len()),
                    started,
                );
                serde_json::to_string(&context::build(
                    &params.query,
                    &rows,
                    params.max_tokens.unwrap_or(8_000),
                ))
                .unwrap_or_else(|error| error.to_string())
            }
            Err(error) => {
                self.audit(
                    "mcp.context",
                    params.project.as_deref(),
                    params.source.as_deref(),
                    "failed",
                    None,
                    started,
                );
                format!("retrieval error: {error}")
            }
        }
    }

    #[tool(
        description = "Report index health, source coverage, embedding identity, and persistent embedding-cache telemetry"
    )]
    async fn brain_status(&self) -> String {
        if !self.principal.has_scope(STATUS_SCOPE) {
            return "authorization error: status scope required".into();
        }
        match self.store.stats() {
            Ok(stats) => serde_json::to_string(&stats).unwrap_or_else(|error| error.to_string()),
            Err(error) => format!("status error: {error}"),
        }
    }
}

impl BrainServer {
    #[allow(clippy::too_many_arguments)]
    fn audit(
        &self,
        action: &str,
        project: Option<&str>,
        source: Option<&str>,
        outcome: &str,
        count: Option<usize>,
        started: Instant,
    ) {
        if let Err(error) = self.store.record_audit(
            &self.principal.name,
            action,
            project,
            source,
            outcome,
            count,
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            self.audit_max_events,
        ) {
            tracing::warn!(%error, "MCP audit write failed");
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

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use tempfile::tempdir;

    use super::*;
    use crate::auth::AuthPolicy;
    use crate::config::{AuthTokenConfig, Config};
    use crate::embed::DeterministicEmbedder;
    use crate::model::{Document, Evidence};

    #[tokio::test]
    async fn configured_mcp_principal_enforces_scope_acl_and_audit_identity() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));
        for (source_id, acl) in [("work-note", "work"), ("personal-note", "personal")] {
            let content = format!("shared launch phrase for {source_id}");
            let vector = embedder
                .embed(std::slice::from_ref(&content))
                .await
                .expect("embedding")
                .remove(0);
            store
                .upsert(
                    &Document {
                        source: "notes".into(),
                        source_id: source_id.into(),
                        title: source_id.into(),
                        content: content.clone(),
                        uri: None,
                        updated_at: Utc::now(),
                        project: "demo".into(),
                        acl: vec![acl.into()],
                        metadata: serde_json::json!({}),
                    },
                    &[(content, vector)],
                )
                .expect("document");
        }
        let mut config = Config::default();
        config
            .environment
            .insert("WORK_TOKEN".into(), "work-secret".into());
        config.auth.tokens = vec![AuthTokenConfig {
            principal: "work-agent".into(),
            token_env: "WORK_TOKEN".into(),
            scopes: vec![QUERY_SCOPE.into()],
            acl: vec!["work".into()],
        }];
        let principal = AuthPolicy::from_config(&config, None)
            .expect("policy")
            .authenticate("work-secret")
            .expect("principal");
        let server = BrainServer::new(store.clone(), embedder)
            .with_principal(principal)
            .with_audit_limit(10);

        let payload = server
            .search(Parameters(SearchParams {
                query: "shared launch phrase".into(),
                project: Some("demo".into()),
                source: None,
                limit: Some(10),
            }))
            .await;
        let rows: Vec<Evidence> = serde_json::from_str(&payload).expect("search rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source_id, "work-note");
        assert_eq!(
            server.brain_status().await,
            "authorization error: status scope required"
        );
        let audit = store.audit_events(10).expect("audit");
        assert_eq!(audit[0].principal, "work-agent");
        assert_eq!(audit[0].action, "mcp.search");
    }
}
