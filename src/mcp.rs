use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use std::time::Instant;

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::{Deserialize, Serialize};

use crate::{
    auth::{Principal, QUERY_SCOPE, STATUS_SCOPE, acl_allows},
    context,
    embed::Embedder,
    retrieval,
    store::Store,
};

const MAX_SCOPE_BYTES: usize = 256;
const STATS_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchParams {
    query: String,
    project: Option<String>,
    source: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextParams {
    query: String,
    project: Option<String>,
    source: Option<String>,
    limit: Option<usize>,
    max_tokens: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DomainSearchParams {
    query: String,
    project: Option<String>,
    limit: Option<usize>,
}

/// Safe, non-secret source configuration exposed to agents through
/// `brain_status`. This deliberately omits credential paths, environment names,
/// and connector arguments.
pub use crate::source_status::ConfiguredSourceStatus;

#[derive(Debug, Serialize)]
struct BrainStatus {
    #[serde(flatten)]
    stats: crate::store::StoreStats,
    configured_sources: Vec<ConfiguredSourceStatus>,
    retrieval_fallbacks_total: u64,
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
    configured_sources: Vec<ConfiguredSourceStatus>,
    retrieval_fallbacks: Arc<AtomicU64>,
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
            configured_sources: Vec::new(),
            retrieval_fallbacks: Arc::new(AtomicU64::new(0)),
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

    pub fn with_configured_sources(mut self, sources: Vec<ConfiguredSourceStatus>) -> Self {
        self.configured_sources = sources;
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
        if let Err(error) = validate_request(
            &params.query,
            params.project.as_deref(),
            params.source.as_deref(),
        ) {
            self.audit(
                "mcp.search",
                params.project.as_deref(),
                params.source.as_deref(),
                "invalid",
                None,
                started,
            );
            return format!("invalid request: {error}");
        }
        let acl = self.principal.visible_acl();
        match retrieval::retrieve_scoped_with_status(
            &self.store,
            &self.embedder,
            &params.query,
            params.project.as_deref(),
            params.source.as_deref(),
            params
                .limit
                .unwrap_or(10)
                .clamp(1, retrieval::MAX_RESULT_LIMIT),
            &acl,
        )
        .await
        {
            Ok(retrieval) => {
                if retrieval.degraded() {
                    self.retrieval_fallbacks.fetch_add(1, Ordering::Relaxed);
                }
                self.audit(
                    "mcp.search",
                    params.project.as_deref(),
                    params.source.as_deref(),
                    if retrieval.degraded() {
                        "degraded"
                    } else {
                        "succeeded"
                    },
                    Some(retrieval.evidence.len()),
                    started,
                );
                serde_json::to_string(&retrieval.evidence).unwrap_or_else(|error| error.to_string())
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
        if let Err(error) = validate_request(
            &params.query,
            params.project.as_deref(),
            params.source.as_deref(),
        ) {
            self.audit(
                "mcp.context",
                params.project.as_deref(),
                params.source.as_deref(),
                "invalid",
                None,
                started,
            );
            return format!("invalid request: {error}");
        }
        let acl = self.principal.visible_acl();
        match retrieval::retrieve_scoped_with_status(
            &self.store,
            &self.embedder,
            &params.query,
            params.project.as_deref(),
            params.source.as_deref(),
            params
                .limit
                .unwrap_or(20)
                .clamp(1, retrieval::MAX_RESULT_LIMIT),
            &acl,
        )
        .await
        {
            Ok(retrieval) => {
                if retrieval.degraded() {
                    self.retrieval_fallbacks.fetch_add(1, Ordering::Relaxed);
                }
                self.audit(
                    "mcp.context",
                    params.project.as_deref(),
                    params.source.as_deref(),
                    if retrieval.degraded() {
                        "degraded"
                    } else {
                        "succeeded"
                    },
                    Some(retrieval.evidence.len()),
                    started,
                );
                serde_json::to_string(&context::build_with_retrieval(
                    &params.query,
                    &retrieval.evidence,
                    params.max_tokens.unwrap_or(8_000),
                    retrieval.mode.as_str(),
                    retrieval.warning.as_deref(),
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
        description = "Search configured Buzz, Gmail, Slack, Discord, and other communication evidence without invoking a language model"
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
        description = "Report index health, configured source coverage, embedding identity, and persistent cache telemetry without exposing credentials"
    )]
    async fn brain_status(&self) -> String {
        let started = Instant::now();
        if !self.principal.has_scope(STATUS_SCOPE) {
            self.audit("mcp.brain_status", None, None, "forbidden", None, started);
            return "authorization error: status scope required".into();
        }
        let acl = self.principal.visible_acl();
        match if self.principal.is_owner() {
            let store = self.store.clone();
            stats_with_timeout(STATS_TIMEOUT, move || store.stats()).await
        } else {
            let store = self.store.clone();
            let stats_acl = acl.clone();
            let allowed_sync_sources = self
                .configured_sources
                .iter()
                .filter(|source| acl_allows(&source.acl, &acl))
                .map(|source| (source.source.clone(), source.project.clone()))
                .collect::<HashSet<_>>();
            stats_with_timeout(STATS_TIMEOUT, move || {
                store.stats_scoped(&stats_acl, &allowed_sync_sources)
            })
            .await
        } {
            Ok(stats) => {
                let count = usize::try_from(stats.documents).ok();
                let result = serde_json::to_string(&BrainStatus {
                    stats,
                    configured_sources: self
                        .configured_sources
                        .iter()
                        .filter(|source| acl_allows(&source.acl, &acl))
                        .cloned()
                        .collect(),
                    retrieval_fallbacks_total: self.retrieval_fallbacks.load(Ordering::Relaxed),
                });
                match result {
                    Ok(payload) => {
                        self.audit("mcp.brain_status", None, None, "succeeded", count, started);
                        payload
                    }
                    Err(error) => {
                        self.audit("mcp.brain_status", None, None, "failed", None, started);
                        error.to_string()
                    }
                }
            }
            Err(error) => {
                self.audit("mcp.brain_status", None, None, "failed", None, started);
                format!("status error: {error}")
            }
        }
    }
}

async fn stats_with_timeout<F>(
    timeout: Duration,
    stats: F,
) -> anyhow::Result<crate::store::StoreStats>
where
    F: FnOnce() -> anyhow::Result<crate::store::StoreStats> + Send + 'static,
{
    let task = tokio::task::spawn_blocking(stats);
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(anyhow::anyhow!("MCP stats probe worker failed: {error}")),
        Err(_) => Err(anyhow::anyhow!(
            "MCP stats probe timed out after {timeout:?}"
        )),
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
        if let Err(error) = validate_request(&params.query, params.project.as_deref(), None) {
            self.audit(
                action,
                params.project.as_deref(),
                None,
                "invalid",
                None,
                started,
            );
            return format!("invalid request: {error}");
        }
        let acl = self.principal.visible_acl();
        match retrieval::retrieve_sources_scoped_with_status(
            &self.store,
            &self.embedder,
            &params.query,
            params.project.as_deref(),
            sources,
            params.limit.unwrap_or(10).clamp(1, 50),
            &acl,
        )
        .await
        {
            Ok(retrieval) => {
                if retrieval.degraded() {
                    self.retrieval_fallbacks.fetch_add(1, Ordering::Relaxed);
                }
                self.audit(
                    action,
                    params.project.as_deref(),
                    None,
                    if retrieval.degraded() {
                        "degraded"
                    } else {
                        "succeeded"
                    },
                    Some(retrieval.evidence.len()),
                    started,
                );
                serde_json::to_string(&retrieval.evidence).unwrap_or_else(|error| error.to_string())
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

fn validate_scopes(project: Option<&str>, source: Option<&str>) -> Result<(), String> {
    for (name, value) in [("project", project), ("source", source)] {
        if value.is_some_and(|value| {
            value.is_empty()
                || value.len() > MAX_SCOPE_BYTES
                || value.chars().any(|character| character.is_control())
        }) {
            return Err(format!("{name} must contain 1 to {MAX_SCOPE_BYTES} bytes"));
        }
    }
    Ok(())
}

fn validate_request(
    query: &str,
    project: Option<&str>,
    source: Option<&str>,
) -> Result<(), String> {
    if query.trim().is_empty() {
        return Err("query must not be empty".into());
    }
    if query.len() > retrieval::MAX_QUERY_BYTES {
        return Err(format!(
            "query exceeds {} bytes",
            retrieval::MAX_QUERY_BYTES
        ));
    }
    validate_scopes(project, source)
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
    use crate::auth::{ADMIN_SCOPE, AuthPolicy};
    use crate::config::{AuthTokenConfig, Config};
    use crate::embed::DeterministicEmbedder;
    use crate::model::{Document, Evidence};
    use crate::source_validation::{self, SourceValidationStatus};

    /// Mirror of the `mcp` command wiring in `main.rs`: build the safe status
    /// views for every configured source and attach persisted validation.
    fn configured_sources(config: &Config) -> Vec<ConfiguredSourceStatus> {
        let mut sources = config
            .sources
            .iter()
            .map(|source| crate::source_status::configured_source_status(config, source))
            .collect::<Vec<_>>();
        let fingerprints = crate::source_status::validation_fingerprints(config);
        crate::source_status::refresh_source_validations(
            &mut sources,
            &config.data_dir,
            config.ingestion.validation_max_age_hours,
            &fingerprints,
        )
        .expect("validation refresh");
        sources
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
        let principal = AuthPolicy::from_config(&config)
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
        assert_eq!(audit[0].action, "mcp.brain_status");
        assert_eq!(audit[1].action, "mcp.search");
    }

    #[tokio::test]
    async fn admin_mcp_principal_with_named_acl_can_access_records_outside_acl_scope() {
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
        config
            .environment
            .insert("ADMIN_TOKEN".into(), "admin-secret".into());
        config.auth.tokens = vec![
            AuthTokenConfig {
                principal: "work-agent".into(),
                token_env: "WORK_TOKEN".into(),
                scopes: vec![QUERY_SCOPE.into()],
                acl: vec!["work".into()],
            },
            AuthTokenConfig {
                principal: "admin-agent".into(),
                token_env: "ADMIN_TOKEN".into(),
                scopes: vec![QUERY_SCOPE.into(), ADMIN_SCOPE.into()],
                acl: vec!["work".into()],
            },
        ];
        let work_principal = AuthPolicy::from_config(&config)
            .expect("policy")
            .authenticate("work-secret")
            .expect("work principal");
        let admin_principal = AuthPolicy::from_config(&config)
            .expect("policy")
            .authenticate("admin-secret")
            .expect("admin principal");

        let work_server =
            BrainServer::new(store.clone(), embedder.clone()).with_principal(work_principal);
        let admin_server = BrainServer::new(store, embedder).with_principal(admin_principal);

        let work_rows: Vec<Evidence> = serde_json::from_str(
            &work_server
                .search(Parameters(SearchParams {
                    query: "shared launch phrase".into(),
                    project: Some("demo".into()),
                    source: None,
                    limit: Some(10),
                }))
                .await,
        )
        .expect("work rows");
        assert_eq!(work_rows.len(), 1);
        assert_eq!(work_rows[0].source_id, "work-note");

        let admin_rows: Vec<Evidence> = serde_json::from_str(
            &admin_server
                .search(Parameters(SearchParams {
                    query: "shared launch phrase".into(),
                    project: Some("demo".into()),
                    source: None,
                    limit: Some(10),
                }))
                .await,
        )
        .expect("admin rows");
        assert_eq!(admin_rows.len(), 2);
        let admin_ids = admin_rows
            .iter()
            .map(|evidence| evidence.source_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert!(admin_ids.contains("work-note"));
        assert!(admin_ids.contains("personal-note"));

        let work_context: serde_json::Value = serde_json::from_str(
            &work_server
                .context(Parameters(ContextParams {
                    query: "shared launch phrase".into(),
                    project: Some("demo".into()),
                    source: None,
                    limit: Some(10),
                    max_tokens: Some(200),
                }))
                .await,
        )
        .expect("work context");
        assert_eq!(work_context["evidence"].as_array().map(Vec::len), Some(1));

        let admin_context: serde_json::Value = serde_json::from_str(
            &admin_server
                .context(Parameters(ContextParams {
                    query: "shared launch phrase".into(),
                    project: Some("demo".into()),
                    source: None,
                    limit: Some(10),
                    max_tokens: Some(200),
                }))
                .await,
        )
        .expect("admin context");
        assert_eq!(admin_context["evidence"].as_array().map(Vec::len), Some(2));
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

    #[tokio::test]
    async fn brain_status_reports_configured_sources_without_credentials() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));
        let mut config: Config = toml::from_str(
            r#"
            [[sources]]
            name = "personal-gmail"
            kind = "gmail"
            enabled = false
            project = "personal"
            acl = ["personal"]
            "#,
        )
        .expect("configuration");
        config.data_dir = directory.path().to_path_buf();
        let server =
            BrainServer::new(store, embedder).with_configured_sources(configured_sources(&config));

        let status: serde_json::Value =
            serde_json::from_str(&server.brain_status().await).expect("status JSON");
        assert_eq!(status["configured_sources"][0]["name"], "personal-gmail");
        assert_eq!(status["configured_sources"][0]["enabled"], false);
        assert_eq!(
            status["configured_sources"][0]["authorization"]["method"],
            "google_oauth"
        );
        assert_eq!(
            status["configured_sources"][0]["authorization"]["authorized"],
            false
        );
        assert_eq!(
            status["configured_sources"][0]["authorization"]["setup_required"],
            true
        );
        assert!(status["configured_sources"][0].get("token").is_none());
        assert!(status["configured_sources"][0]["validation"].is_null());
        assert!(status.get("sources").is_some());
    }

    #[tokio::test]
    async fn brain_status_filters_configured_sources_by_principal_acl() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));
        let mut config: Config = toml::from_str(
            r#"
            [[sources]]
            name = "work-drive"
            kind = "google-drive"
            project = "work"
            enabled = true
            acl = ["work"]

            [[sources]]
            name = "personal-notes"
            kind = "apple-notes"
            project = "personal"
            enabled = true
            acl = ["personal"]

            [[sources]]
            name = "public-reference"
            kind = "filesystem"
            project = "reference"
            enabled = true
            "#,
        )
        .expect("configuration");
        config.data_dir = directory.path().to_path_buf();
        config
            .environment
            .insert("WORK_TOKEN".into(), "work-secret".into());
        config.auth.tokens = vec![AuthTokenConfig {
            principal: "work-agent".into(),
            token_env: "WORK_TOKEN".into(),
            scopes: vec![STATUS_SCOPE.into()],
            acl: vec!["work".into()],
        }];
        let principal = AuthPolicy::from_config(&config)
            .expect("policy")
            .authenticate("work-secret")
            .expect("principal");
        let server = BrainServer::new(store, embedder)
            .with_principal(principal)
            .with_configured_sources(configured_sources(&config));

        let status: serde_json::Value =
            serde_json::from_str(&server.brain_status().await).expect("status JSON");
        let names = status["configured_sources"]
            .as_array()
            .expect("configured sources")
            .iter()
            .filter_map(|source| source["name"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["work-drive"]);
    }

    #[tokio::test]
    async fn admin_brain_status_includes_configured_sources_outside_named_acl() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));
        let mut config: Config = toml::from_str(
            r#"
            [[sources]]
            name = "work-drive"
            kind = "google-drive"
            project = "work"
            enabled = true
            acl = ["work"]

            [[sources]]
            name = "personal-notes"
            kind = "apple-notes"
            project = "personal"
            enabled = true
            acl = ["personal"]
            "#,
        )
        .expect("configuration");
        config.data_dir = directory.path().to_path_buf();
        config
            .environment
            .insert("ADMIN_TOKEN".into(), "admin-secret".into());
        config.auth.tokens = vec![AuthTokenConfig {
            principal: "admin-agent".into(),
            token_env: "ADMIN_TOKEN".into(),
            scopes: vec![ADMIN_SCOPE.into(), STATUS_SCOPE.into()],
            acl: vec!["work".into()],
        }];
        let principal = AuthPolicy::from_config(&config)
            .expect("policy")
            .authenticate("admin-secret")
            .expect("principal");
        let server = BrainServer::new(store, embedder)
            .with_principal(principal)
            .with_configured_sources(configured_sources(&config));

        let status: serde_json::Value =
            serde_json::from_str(&server.brain_status().await).expect("status JSON");
        let names = status["configured_sources"]
            .as_array()
            .expect("configured sources")
            .iter()
            .filter_map(|source| source["name"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["work-drive", "personal-notes"]);
    }

    #[tokio::test]
    async fn brain_status_reports_authorization_and_validation_status_for_configured_sources() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));
        let token_path = directory.path().join("google-token.json");
        write_private_fixture(
            &token_path,
            "{\"refresh_token\":\"refresh\",\"client_id\":\"client\"}",
        );
        let mut config: Config = toml::from_str(&format!(
            r#"
            [ingestion]
            validation_max_age_hours = 24

            [[sources]]
            name = "gmail"
            kind = "google-drive"
            enabled = true
            project = "work"
            token = "{token_path_display}"
            acl = ["work"]

            [[sources]]
            name = "calendar"
            kind = "google-calendar"
            enabled = true
            project = "work"
            acl = ["work"]

            [[sources]]
            name = "slack"
            kind = "slack"
            enabled = true
            project = "work"
            token_env = "SLACK_TOKEN"
            acl = ["work"]

            [[sources]]
            name = "discord"
            kind = "discord"
            enabled = true
            project = "work"
            token = "{discord_token}"
            oauth_client = "{discord_client}"
            acl = ["work"]
            "#,
            token_path_display = token_path.display(),
            discord_token = directory.path().join("discord-rpc-token.json").display(),
            discord_client = directory.path().join("discord-rpc-client.json").display(),
        ))
        .expect("configuration");
        config.data_dir = directory.path().to_path_buf();
        config
            .environment
            .insert("SLACK_TOKEN".into(), "present".into());

        let gmail = config
            .sources
            .iter()
            .find(|source| source.name == "gmail")
            .expect("gmail source");
        let slack = config
            .sources
            .iter()
            .find(|source| source.name == "slack")
            .expect("slack source");
        source_validation::record(
            &config.data_dir,
            SourceValidationStatus {
                source: "gmail".into(),
                project: "work".into(),
                kind: "google-drive".into(),
                status: "succeeded".into(),
                validated_at: chrono::Utc::now() - chrono::Duration::hours(1),
                documents: Some(12),
                bytes: Some(2048),
                max_documents: 25,
                max_bytes: 4096,
                max_seconds: 60,
                configuration_fingerprint: Some(
                    source_validation::configuration_fingerprint(gmail).expect("fingerprint"),
                ),
                complete: None,
                error: None,
            },
        )
        .expect("gmail validation");
        source_validation::record(
            &config.data_dir,
            SourceValidationStatus {
                source: "slack".into(),
                project: "work".into(),
                kind: "slack".into(),
                status: "failed".into(),
                validated_at: chrono::Utc::now(),
                documents: None,
                bytes: None,
                max_documents: 25,
                max_bytes: 4096,
                max_seconds: 60,
                configuration_fingerprint: Some(
                    source_validation::configuration_fingerprint(slack).expect("fingerprint"),
                ),
                complete: None,
                error: Some("connector returned 403 Forbidden with Bearer super-secret".into()),
            },
        )
        .expect("slack validation");

        let server =
            BrainServer::new(store, embedder).with_configured_sources(configured_sources(&config));
        let text = server.brain_status().await;
        let status: serde_json::Value = serde_json::from_str(&text).expect("status JSON");
        let configured = status["configured_sources"].as_array().expect("sources");

        let gmail_status = configured
            .iter()
            .find(|source| source["name"] == "gmail")
            .expect("gmail status");
        assert_eq!(gmail_status["authorization"]["method"], "google_oauth");
        assert_eq!(gmail_status["authorization"]["authorized"], true);
        assert_eq!(gmail_status["authorization"]["setup_required"], false);
        assert_eq!(gmail_status["validation"]["status"], "succeeded");
        assert_eq!(gmail_status["validation"]["documents"], 12);
        assert_eq!(gmail_status["validation"]["fresh"], true);
        assert!(gmail_status["validation"]["error"].is_null());

        let calendar_status = configured
            .iter()
            .find(|source| source["name"] == "calendar")
            .expect("calendar status");
        assert_eq!(calendar_status["authorization"]["method"], "google_oauth");
        assert_eq!(calendar_status["authorization"]["authorized"], false);
        assert_eq!(calendar_status["authorization"]["setup_required"], true);
        assert!(calendar_status["validation"].is_null());

        let slack_status = configured
            .iter()
            .find(|source| source["name"] == "slack")
            .expect("slack status");
        assert_eq!(slack_status["authorization"]["method"], "token");
        assert_eq!(slack_status["authorization"]["authorized"], true);
        assert_eq!(slack_status["authorization"]["setup_required"], false);
        assert_eq!(slack_status["validation"]["status"], "failed");
        assert_eq!(
            slack_status["validation"]["error"],
            "source validation failed"
        );
        assert_eq!(
            slack_status["validation"]["error_category"],
            "authorization"
        );

        let discord_status = configured
            .iter()
            .find(|source| source["name"] == "discord")
            .expect("discord status");
        assert_eq!(discord_status["authorization"]["method"], "discord_rpc");
        assert_eq!(discord_status["authorization"]["authorized"], false);
        assert_eq!(discord_status["authorization"]["setup_required"], true);
        assert!(discord_status["validation"].is_null());

        // Secret redaction: no environment variable names, token values,
        // credential paths, or raw connector diagnostics reach the agent.
        assert!(!text.contains("SLACK_TOKEN"));
        assert!(!text.contains("DISCORD_TOKEN"));
        assert!(!text.contains("super-secret"));
        assert!(!text.contains("403 Forbidden"));
        assert!(!text.contains("Bearer"));
        assert!(!text.contains(token_path.to_string_lossy().as_ref()));
    }

    #[tokio::test]
    async fn brain_status_validation_status_respects_freshness_and_configuration_fingerprint() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));
        let mut config: Config = toml::from_str(
            r#"
            [ingestion]
            validation_max_age_hours = 24

            [[sources]]
            name = "drive"
            kind = "google-drive"
            enabled = true
            project = "work"
            acl = ["work"]
            "#,
        )
        .expect("configuration");
        config.data_dir = directory.path().to_path_buf();
        let source = config.sources.first().expect("configured source");
        let fingerprint =
            source_validation::configuration_fingerprint(source).expect("validation fingerprint");
        source_validation::record(
            &config.data_dir,
            SourceValidationStatus {
                source: "drive".into(),
                project: "work".into(),
                kind: "google-drive".into(),
                status: "succeeded".into(),
                validated_at: chrono::Utc::now() - chrono::Duration::hours(200),
                documents: Some(1),
                bytes: Some(1),
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 60,
                configuration_fingerprint: Some(fingerprint.clone()),
                complete: None,
                error: None,
            },
        )
        .expect("stale validation");

        let status: serde_json::Value = serde_json::from_str(
            &BrainServer::new(store.clone(), embedder.clone())
                .with_configured_sources(configured_sources(&config))
                .brain_status()
                .await,
        )
        .expect("status JSON");
        let validation = &status["configured_sources"][0]["validation"];
        assert_eq!(validation["status"], "succeeded");
        assert_eq!(validation["fresh"], false);
        assert!(
            validation["age_seconds"]
                .as_u64()
                .is_some_and(|age| age >= 200 * 3_600)
        );

        source_validation::record(
            &config.data_dir,
            SourceValidationStatus {
                source: "drive".into(),
                project: "work".into(),
                kind: "google-drive".into(),
                status: "succeeded".into(),
                validated_at: chrono::Utc::now() - chrono::Duration::hours(1),
                documents: Some(1),
                bytes: Some(1),
                max_documents: 25,
                max_bytes: 1024,
                max_seconds: 60,
                configuration_fingerprint: Some(fingerprint.clone()),
                complete: None,
                error: None,
            },
        )
        .expect("fresh validation");
        let status: serde_json::Value = serde_json::from_str(
            &BrainServer::new(store.clone(), embedder.clone())
                .with_configured_sources(configured_sources(&config))
                .brain_status()
                .await,
        )
        .expect("status JSON");
        assert_eq!(status["configured_sources"][0]["validation"]["fresh"], true);

        // A configuration change invalidates the persisted fingerprint, so the
        // validation disappears even though the record is fresh.
        config.sources[0].query = Some("from:changed".into());
        let status: serde_json::Value = serde_json::from_str(
            &BrainServer::new(store, embedder)
                .with_configured_sources(configured_sources(&config))
                .brain_status()
                .await,
        )
        .expect("status JSON");
        assert!(status["configured_sources"][0]["validation"].is_null());
    }

    #[test]
    fn mcp_scope_filters_are_explicitly_bounded() {
        assert!(validate_scopes(Some("work"), None).is_ok());
        assert!(validate_scopes(Some(""), None).is_err());
        assert!(validate_scopes(None, Some(&"x".repeat(MAX_SCOPE_BYTES + 1))).is_err());
        assert!(validate_scopes(Some("work\u{0000}personal"), None).is_err());
        assert!(
            serde_json::from_str::<SearchParams>(r#"{"query":"work","unexpected":"field"}"#)
                .is_err()
        );
    }

    #[test]
    fn mcp_requests_reject_empty_and_oversized_queries() {
        assert_eq!(
            validate_request(" \n\t", None, None).expect_err("blank query"),
            "query must not be empty"
        );
        assert!(
            validate_request(&"x".repeat(retrieval::MAX_QUERY_BYTES + 1), None, None)
                .expect_err("oversized query")
                .contains("query exceeds")
        );
        assert!(validate_request("work", Some("engineering"), Some("notes")).is_ok());
    }
}
