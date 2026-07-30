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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DomainSearchParams {
    query: String,
    project: Option<String>,
    limit: Option<usize>,
}

#[derive(Clone)]
pub struct BrainServer {
    store: Store,
    embedder: Arc<dyn Embedder>,
    tool_router: ToolRouter<Self>,
    audit_max_events: usize,
    principal: Principal,
    code_sources: Vec<String>,
    message_sources: Vec<String>,
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
            code_sources: Vec::new(),
            message_sources: Vec::new(),
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

    pub fn with_source_groups(
        mut self,
        code_sources: Vec<String>,
        message_sources: Vec<String>,
    ) -> Self {
        self.code_sources = normalized_sources(code_sources);
        self.message_sources = normalized_sources(message_sources);
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
        description = "Search configured code and filesystem indexes with hybrid retrieval and bounded neighboring context"
    )]
    async fn search_code(&self, Parameters(params): Parameters<DomainSearchParams>) -> String {
        self.domain_search("mcp.search_code", &self.code_sources, params)
            .await
    }

    #[tool(
        description = "Search configured Gmail, Slack, and Discord evidence without invoking a language model"
    )]
    async fn search_messages(&self, Parameters(params): Parameters<DomainSearchParams>) -> String {
        self.domain_search("mcp.search_messages", &self.message_sources, params)
            .await
    }

    #[tool(
        description = "Find communication evidence about people who may know a subject; returns cited source records rather than inferred profiles"
    )]
    async fn who_knows(&self, Parameters(params): Parameters<DomainSearchParams>) -> String {
        self.domain_search("mcp.who_knows", &self.message_sources, params)
            .await
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
    async fn domain_search(
        &self,
        action: &str,
        sources: &[String],
        params: DomainSearchParams,
    ) -> String {
        let started = Instant::now();
        if !self.principal.has_scope(QUERY_SCOPE) {
            self.audit(
                action,
                params.project.as_deref(),
                None,
                "forbidden",
                None,
                started,
            );
            return "authorization error: query scope required".into();
        }
        match retrieval::retrieve_sources_scoped(
            &self.store,
            &self.embedder,
            &params.query,
            params.project.as_deref(),
            sources,
            params.limit.unwrap_or(10).clamp(1, 50),
            &self.principal.acl_labels(),
        )
        .await
        {
            Ok(rows) => {
                self.audit(
                    action,
                    params.project.as_deref(),
                    None,
                    "succeeded",
                    Some(rows.len()),
                    started,
                );
                serde_json::to_string(&rows).unwrap_or_else(|error| error.to_string())
            }
            Err(error) => {
                self.audit(
                    action,
                    params.project.as_deref(),
                    None,
                    "failed",
                    None,
                    started,
                );
                format!("retrieval error: {error}")
            }
        }
    }

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
            "Call context before answering questions about the user or their work. Prefer search_code, search_messages, or who_knows for narrow discovery. Reuse citation-ready output instead of repeating broad calls.",
        )
    }
}

fn normalized_sources(sources: Vec<String>) -> Vec<String> {
    let mut sources = sources
        .into_iter()
        .map(|source| source.trim().to_string())
        .filter(|source| !source.is_empty())
        .collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    sources.truncate(32);
    sources
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

    #[tokio::test]
    async fn task_specific_tools_search_only_configured_source_groups() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));
        for (source, source_id, content) in [
            ("code-index", "repository", "shared symbol implementation"),
            ("slack-work", "thread", "Avery knows the cache architecture"),
            ("notes", "private-note", "shared symbol personal reminder"),
        ] {
            let content = content.to_string();
            let vector = embedder
                .embed(std::slice::from_ref(&content))
                .await
                .expect("embedding")
                .remove(0);
            store
                .upsert(
                    &Document {
                        source: source.into(),
                        source_id: source_id.into(),
                        title: source_id.into(),
                        content: content.clone(),
                        uri: None,
                        updated_at: Utc::now(),
                        project: "demo".into(),
                        acl: Vec::new(),
                        metadata: serde_json::json!({}),
                    },
                    &[(content, vector)],
                )
                .expect("document");
        }
        let server = BrainServer::new(store.clone(), embedder).with_source_groups(
            vec!["code-index".into(), "code-index".into(), String::new()],
            vec!["slack-work".into()],
        );

        let code: Vec<Evidence> = serde_json::from_str(
            &server
                .search_code(Parameters(DomainSearchParams {
                    query: "shared symbol".into(),
                    project: Some("demo".into()),
                    limit: Some(10),
                }))
                .await,
        )
        .expect("code evidence");
        assert_eq!(code.len(), 1);
        assert_eq!(code[0].source, "code-index");

        let people: Vec<Evidence> = serde_json::from_str(
            &server
                .who_knows(Parameters(DomainSearchParams {
                    query: "cache architecture".into(),
                    project: Some("demo".into()),
                    limit: Some(10),
                }))
                .await,
        )
        .expect("expertise evidence");
        assert_eq!(people.len(), 1);
        assert_eq!(people[0].source, "slack-work");
        assert!(people[0].content.contains("Avery"));

        let audit = store.audit_events(10).expect("audit");
        assert_eq!(audit[0].action, "mcp.who_knows");
        assert_eq!(audit[1].action, "mcp.search_code");
    }
}
