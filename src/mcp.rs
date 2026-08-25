use std::collections::HashSet;
use std::path::PathBuf;
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
    auth::{AuthPolicy, MEMORY_SCOPE, Principal, QUERY_SCOPE, STATUS_SCOPE, acl_allows},
    config::Config,
    context,
    embed::Embedder,
    memory::{MemoryInput, MemoryStats},
    retrieval::{self, RetrievalTuning},
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryRememberParams {
    kind: String,
    content_type: Option<String>,
    retention_tier: Option<String>,
    scope: Option<String>,
    project: String,
    title: String,
    content: String,
    source: Option<String>,
    source_id: Option<String>,
    dedupe_key: Option<String>,
    confidence: Option<f32>,
    importance: Option<f32>,
    acl: Option<Vec<String>>,
    // `serde_json::Value` intentionally preserves arbitrary JSON provenance
    // from existing clients. Advertise it as an object to MCP consumers,
    // because schemars represents an untyped Value as the boolean schema
    // `true`, which causes strict clients to reject the entire tools/list
    // response before they can call any tool.
    #[schemars(with = "Option<std::collections::BTreeMap<String, serde_json::Value>>")]
    provenance: Option<serde_json::Value>,
    supersedes_id: Option<String>,
    valid_until: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryRecallParams {
    query: String,
    project: Option<String>,
    kind: Option<String>,
    content_type: Option<String>,
    retention_tier: Option<String>,
    scope: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryExportParams {
    project: Option<String>,
    kind: Option<String>,
    content_type: Option<String>,
    retention_tier: Option<String>,
    scope: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryForgetParams {
    id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryCandidateParams {
    observation_kind: String,
    content_type: String,
    retention_tier: String,
    scope: String,
    project: String,
    title: String,
    content: String,
    source: String,
    source_id: String,
    dedupe_key: Option<String>,
    confidence: f32,
    importance: f32,
    sensitivity: String,
    acl: Option<Vec<String>>,
    #[schemars(with = "std::collections::BTreeMap<String, serde_json::Value>")]
    provenance: std::collections::BTreeMap<String, serde_json::Value>,
    expires_at: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryCandidateListParams {
    project: Option<String>,
    observation_kind: Option<String>,
    scope: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryCandidateIdParams {
    id: String,
}

/// Safe, non-secret source configuration exposed to agents through
/// `brain_status`. This deliberately omits credential paths, environment names,
/// and connector arguments.
pub use crate::source_status::ConfiguredSourceStatus;

#[derive(Debug, Serialize)]
struct BrainStatus {
    #[serde(flatten)]
    stats: crate::store::StoreStats,
    memory: crate::memory::MemoryStats,
    configured_sources: Vec<ConfiguredSourceStatus>,
    retrieval_fallbacks_total: u64,
}

#[derive(Clone)]
pub struct BrainServer {
    store: Store,
    embedder: Arc<dyn Embedder>,
    tool_router: ToolRouter<Self>,
    audit_max_events: usize,
    principal: PrincipalSource,
    code_sources: Vec<String>,
    message_sources: Vec<String>,
    configured_sources: Vec<ConfiguredSourceStatus>,
    retrieval_fallbacks: Arc<AtomicU64>,
    memory_defaults: crate::memory::MemoryDefaults,
    retrieval_tuning: RetrievalTuning,
}

#[derive(Clone)]
enum PrincipalSource {
    Static(Principal),
    Reloadable {
        config_path: PathBuf,
        token_env: String,
    },
}

impl PrincipalSource {
    fn resolve(&self) -> anyhow::Result<Principal> {
        match self {
            Self::Static(principal) => Ok(principal.clone()),
            Self::Reloadable {
                config_path,
                token_env,
            } => {
                let mut config = Config::load(Some(config_path))?;
                config.load_environment()?;
                let token = config
                    .environment
                    .get(token_env)
                    .cloned()
                    .or_else(|| std::env::var(token_env).ok())
                    .ok_or_else(|| anyhow::anyhow!("MCP token environment variable is not set"))?;
                AuthPolicy::from_config_file_preferred(&config)?
                    .authenticate(&token)
                    .ok_or_else(|| {
                        anyhow::anyhow!("MCP token does not match a configured principal")
                    })
            }
        }
    }
}

#[tool_router]
impl BrainServer {
    pub fn new(store: Store, embedder: Arc<dyn Embedder>) -> Self {
        Self {
            store,
            embedder,
            tool_router: Self::tool_router(),
            audit_max_events: 10_000,
            principal: PrincipalSource::Static(Principal::local("local-mcp")),
            code_sources: Vec::new(),
            message_sources: Vec::new(),
            configured_sources: Vec::new(),
            retrieval_fallbacks: Arc::new(AtomicU64::new(0)),
            memory_defaults: crate::memory::MemoryDefaults::default(),
            retrieval_tuning: RetrievalTuning::default(),
        }
    }

    pub fn with_principal(mut self, principal: Principal) -> Self {
        self.principal = PrincipalSource::Static(principal);
        self
    }

    pub fn with_memory_defaults(mut self, confidence: f32, importance: f32) -> Self {
        self.memory_defaults = crate::memory::MemoryDefaults {
            confidence,
            importance,
        };
        self
    }

    pub fn with_retrieval_tuning(mut self, tuning: RetrievalTuning) -> Self {
        self.retrieval_tuning = tuning.bounded();
        self
    }

    /// Resolve the MCP bearer principal from the file-backed configuration for
    /// every tool call. This makes token rotation and revocation effective
    /// without restarting the stdio process; malformed or unreadable policy
    /// fails closed for that call.
    pub fn with_reloadable_principal(
        mut self,
        config_path: impl Into<PathBuf>,
        token_env: impl Into<String>,
    ) -> Self {
        self.principal = PrincipalSource::Reloadable {
            config_path: config_path.into(),
            token_env: token_env.into(),
        };
        self
    }

    fn resolve_principal(&self) -> anyhow::Result<Principal> {
        self.principal.resolve()
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
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => {
                self.audit_as(
                    "mcp-unauthenticated",
                    "mcp.search",
                    params.project.as_deref(),
                    params.source.as_deref(),
                    "unauthorized",
                    None,
                    started,
                );
                return format!("authorization error: {error}");
            }
        };
        if !principal.has_scope(QUERY_SCOPE) {
            self.audit_principal(
                &principal,
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
            self.audit_principal(
                &principal,
                "mcp.search",
                params.project.as_deref(),
                params.source.as_deref(),
                "invalid",
                None,
                started,
            );
            return format!("invalid request: {error}");
        }
        let acl = principal.visible_acl();
        match retrieval::retrieve_scoped_with_status_tuned(
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
            self.retrieval_tuning,
        )
        .await
        {
            Ok(retrieval) => {
                if retrieval.degraded() {
                    self.retrieval_fallbacks.fetch_add(1, Ordering::Relaxed);
                }
                self.audit_principal(
                    &principal,
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
                self.audit_principal(
                    &principal,
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
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => {
                self.audit_as(
                    "mcp-unauthenticated",
                    "mcp.context",
                    params.project.as_deref(),
                    params.source.as_deref(),
                    "unauthorized",
                    None,
                    started,
                );
                return format!("authorization error: {error}");
            }
        };
        if !principal.has_scope(QUERY_SCOPE) {
            self.audit_principal(
                &principal,
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
            self.audit_principal(
                &principal,
                "mcp.context",
                params.project.as_deref(),
                params.source.as_deref(),
                "invalid",
                None,
                started,
            );
            return format!("invalid request: {error}");
        }
        let acl = principal.visible_acl();
        match retrieval::retrieve_scoped_with_status_tuned(
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
            self.retrieval_tuning,
        )
        .await
        {
            Ok(retrieval) => {
                if retrieval.degraded() {
                    self.retrieval_fallbacks.fetch_add(1, Ordering::Relaxed);
                }
                let memories = if principal.has_scope(MEMORY_SCOPE) {
                    let recalled = if principal.is_owner() {
                        self.store.recall_memories_as_owner(
                            &params.query,
                            params.project.as_deref(),
                            None,
                            params
                                .limit
                                .unwrap_or(20)
                                .clamp(1, crate::memory::MAX_MEMORY_RECALL_LIMIT),
                        )
                    } else {
                        self.store.recall_memories(
                            &params.query,
                            params.project.as_deref(),
                            None,
                            params
                                .limit
                                .unwrap_or(20)
                                .clamp(1, crate::memory::MAX_MEMORY_RECALL_LIMIT),
                            &acl,
                        )
                    };
                    recalled.unwrap_or_else(|error| {
                            tracing::warn!(%error, "native memory recall unavailable while building MCP context");
                            Vec::new()
                        })
                } else {
                    Vec::new()
                };
                self.audit_principal(
                    &principal,
                    "mcp.context",
                    params.project.as_deref(),
                    params.source.as_deref(),
                    if retrieval.degraded() {
                        "degraded"
                    } else {
                        "succeeded"
                    },
                    Some(retrieval.evidence.len().saturating_add(memories.len())),
                    started,
                );
                let max_tokens = params.max_tokens.unwrap_or(8_000);
                let corpus_revision = match self.store.corpus_revision() {
                    Ok(revision) => revision,
                    Err(error) => return format!("context contract error: {error}"),
                };
                let memory_revision = if principal.has_scope(MEMORY_SCOPE) {
                    match self.store.memory_revision() {
                        Ok(revision) => Some(revision),
                        Err(error) => return format!("context contract error: {error}"),
                    }
                } else {
                    None
                };
                let bundle = context::build_with_retrieval_and_memory(
                    &params.query,
                    &retrieval.evidence,
                    &memories,
                    max_tokens,
                    retrieval.mode.as_str(),
                    retrieval.warning.as_deref(),
                )
                .with_metadata(context::metadata(context::ContextMetadataInput {
                    token_budget: max_tokens,
                    corpus_revision,
                    memory_revision,
                    embedding_fingerprint: Some(self.embedder.fingerprint()),
                    project: params.project.as_deref(),
                    source: params.source.as_deref(),
                    acl: &acl,
                    retrieval_warning: retrieval.warning.as_deref(),
                }));
                serde_json::to_string(&bundle).unwrap_or_else(|error| error.to_string())
            }
            Err(error) => {
                self.audit_principal(
                    &principal,
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
        description = "Write one explicit, provenance-bearing memory to Cortana's native memory store. Use this for durable facts, preferences, procedures, episodes, or short-lived working state; do not copy whole source documents."
    )]
    async fn remember(&self, Parameters(params): Parameters<MemoryRememberParams>) -> String {
        let started = Instant::now();
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => {
                self.audit_as(
                    "mcp-unauthenticated",
                    "mcp.remember",
                    None,
                    None,
                    "unauthorized",
                    None,
                    started,
                );
                return format!("authorization error: {error}");
            }
        };
        if !principal.has_scope(MEMORY_SCOPE) {
            self.audit_principal(
                &principal,
                "mcp.remember",
                Some(&params.project),
                None,
                "forbidden",
                None,
                started,
            );
            return "authorization error: memory scope required".into();
        }
        let requested_acl = params.acl.unwrap_or_default();
        let visible_acl = principal.visible_acl();
        let acl = if principal.is_owner() {
            requested_acl
        } else if requested_acl.is_empty() {
            visible_acl.clone()
        } else if requested_acl
            .iter()
            .all(|label| visible_acl.iter().any(|visible| visible == label))
        {
            requested_acl
        } else {
            self.audit_principal(
                &principal,
                "mcp.remember",
                Some(&params.project),
                None,
                "forbidden",
                None,
                started,
            );
            return "authorization error: memory ACL exceeds principal visibility".into();
        };
        let input = MemoryInput {
            kind: params.kind,
            project: params.project.clone(),
            title: params.title,
            content: params.content,
            source: params.source.unwrap_or_else(|| "agent".into()),
            source_id: params.source_id.unwrap_or_default(),
            dedupe_key: params.dedupe_key,
            confidence: params.confidence.unwrap_or(self.memory_defaults.confidence),
            importance: params.importance.unwrap_or(self.memory_defaults.importance),
            acl,
            provenance: params.provenance.unwrap_or_else(|| {
                serde_json::json!({
                    "principal": principal.name,
                    "interface": "mcp"
                })
            }),
            supersedes_id: params.supersedes_id,
            valid_until: params.valid_until,
        };
        let axes = match crate::memory::MemoryAxes::with_overrides(
            &input.kind,
            params.content_type.as_deref(),
            params.retention_tier.as_deref(),
            params.scope.as_deref(),
        ) {
            Ok(axes) => axes,
            Err(error) => return format!("invalid memory axes: {error}"),
        };
        match self
            .store
            .remember_scoped_with_axes(&input, &visible_acl, principal.is_owner(), axes)
        {
            Ok(memory) => {
                self.audit_principal(
                    &principal,
                    "mcp.remember",
                    Some(&memory.project),
                    Some(&memory.source),
                    "succeeded",
                    Some(1),
                    started,
                );
                serde_json::to_string(&memory).unwrap_or_else(|error| error.to_string())
            }
            Err(error) => {
                self.audit_principal(
                    &principal,
                    "mcp.remember",
                    Some(&input.project),
                    Some(&input.source),
                    if crate::memory::is_authorization_error(&error) {
                        "forbidden"
                    } else {
                        "invalid"
                    },
                    None,
                    started,
                );
                if crate::memory::is_authorization_error(&error) {
                    "authorization error: memory ACL denied".into()
                } else {
                    format!("memory error: {error}")
                }
            }
        }
    }

    #[tool(
        description = "Submit one bounded, provenance-bearing observation for review. This never writes canonical memory or changes memory revision; full transcripts and sensitive proposals are rejected."
    )]
    async fn propose_memory_candidate(
        &self,
        Parameters(params): Parameters<MemoryCandidateParams>,
    ) -> String {
        let started = Instant::now();
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => {
                self.audit_as(
                    "mcp-unauthenticated",
                    "mcp.memory_candidate.create",
                    Some(&params.project),
                    Some(&params.source),
                    "unauthorized",
                    None,
                    started,
                );
                return format!("authorization error: {error}");
            }
        };
        if !principal.has_scope(MEMORY_SCOPE) {
            self.audit_principal(
                &principal,
                "mcp.memory_candidate.create",
                Some(&params.project),
                Some(&params.source),
                "forbidden",
                None,
                started,
            );
            return "authorization error: memory scope required".into();
        }
        let requested_acl = params.acl.unwrap_or_default();
        let visible_acl = principal.visible_acl();
        let acl = if principal.is_owner() {
            requested_acl
        } else if requested_acl.is_empty() {
            visible_acl.clone()
        } else if requested_acl
            .iter()
            .all(|label| visible_acl.iter().any(|visible| visible == label))
        {
            requested_acl
        } else {
            self.audit_principal(
                &principal,
                "mcp.memory_candidate.create",
                Some(&params.project),
                Some(&params.source),
                "forbidden",
                None,
                started,
            );
            return "authorization error: candidate ACL exceeds principal visibility".into();
        };
        let input = crate::observation::ObservationCandidateInput {
            observation_kind: params.observation_kind,
            content_type: params.content_type,
            retention_tier: params.retention_tier,
            scope: params.scope,
            project: params.project.clone(),
            title: params.title,
            content: params.content,
            source: params.source.clone(),
            source_id: params.source_id,
            dedupe_key: params.dedupe_key,
            confidence: params.confidence,
            importance: params.importance,
            sensitivity: params.sensitivity,
            acl,
            provenance: serde_json::Value::Object(params.provenance.into_iter().collect()),
            expires_at: params.expires_at,
        };
        match self.store.propose_memory_candidate(
            &input,
            &principal.name,
            &visible_acl,
            principal.is_owner(),
        ) {
            Ok(candidate) => {
                self.audit_principal(
                    &principal,
                    "mcp.memory_candidate.create",
                    Some(&candidate.project),
                    Some(&candidate.source),
                    "succeeded",
                    Some(1),
                    started,
                );
                serde_json::to_string(&candidate).unwrap_or_else(|error| error.to_string())
            }
            Err(error) => {
                self.audit_principal(
                    &principal,
                    "mcp.memory_candidate.create",
                    Some(&input.project),
                    Some(&input.source),
                    "rejected",
                    None,
                    started,
                );
                format!("candidate rejected: {error}")
            }
        }
    }

    #[tool(
        description = "List bounded observation candidates visible to the current principal; candidates are not canonical memory recall results."
    )]
    async fn list_memory_candidates(
        &self,
        Parameters(params): Parameters<MemoryCandidateListParams>,
    ) -> String {
        let started = Instant::now();
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => return format!("authorization error: {error}"),
        };
        if !principal.has_scope(MEMORY_SCOPE) {
            return "authorization error: memory scope required".into();
        }
        match self.store.list_memory_candidates(
            params.project.as_deref(),
            params.observation_kind.as_deref(),
            params.scope.as_deref(),
            params.limit.unwrap_or(100),
            &principal.name,
            &principal.visible_acl(),
            principal.is_owner(),
        ) {
            Ok(candidates) => {
                self.audit_principal(
                    &principal,
                    "mcp.memory_candidate.list",
                    params.project.as_deref(),
                    None,
                    "succeeded",
                    Some(candidates.len()),
                    started,
                );
                serde_json::to_string(&candidates).unwrap_or_else(|error| error.to_string())
            }
            Err(error) => {
                self.audit_principal(
                    &principal,
                    "mcp.memory_candidate.list",
                    params.project.as_deref(),
                    None,
                    "failed",
                    None,
                    started,
                );
                format!("candidate list error: {error}")
            }
        }
    }

    #[tool(
        description = "Export bounded observation candidates visible to the current principal, including lifecycle tombstones, for audit or backup."
    )]
    async fn export_memory_candidates(
        &self,
        Parameters(params): Parameters<MemoryCandidateListParams>,
    ) -> String {
        let started = Instant::now();
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => return format!("authorization error: {error}"),
        };
        if !principal.has_scope(MEMORY_SCOPE) {
            return "authorization error: memory scope required".into();
        }
        match self.store.export_memory_candidates(
            params.project.as_deref(),
            params.observation_kind.as_deref(),
            params.scope.as_deref(),
            params.limit.unwrap_or(100),
            &principal.name,
            &principal.visible_acl(),
            principal.is_owner(),
        ) {
            Ok(candidates) => {
                self.audit_principal(
                    &principal,
                    "mcp.memory_candidate.export",
                    params.project.as_deref(),
                    None,
                    "succeeded",
                    Some(candidates.len()),
                    started,
                );
                serde_json::to_string(&candidates).unwrap_or_else(|error| error.to_string())
            }
            Err(error) => format!("candidate export error: {error}"),
        }
    }

    #[tool(
        description = "Cancel a pending observation candidate without changing canonical memory."
    )]
    async fn cancel_memory_candidate(
        &self,
        Parameters(params): Parameters<MemoryCandidateIdParams>,
    ) -> String {
        self.update_memory_candidate(params.id, false).await
    }

    #[tool(
        description = "Redact a pending observation candidate while retaining an audit tombstone."
    )]
    async fn redact_memory_candidate(
        &self,
        Parameters(params): Parameters<MemoryCandidateIdParams>,
    ) -> String {
        self.update_memory_candidate(params.id, true).await
    }

    #[tool(
        description = "Classify one visible pending memory candidate against same-scope canonical memory. This is deterministic, provider-free, review-only, and never mutates canonical memory."
    )]
    async fn classify_memory_candidate(
        &self,
        Parameters(params): Parameters<MemoryCandidateIdParams>,
    ) -> String {
        let started = Instant::now();
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => return format!("authorization error: {error}"),
        };
        if !principal.has_scope(MEMORY_SCOPE) {
            return "authorization error: memory scope required".into();
        }
        match self.store.classify_memory_candidate(
            &params.id,
            &principal.name,
            &principal.visible_acl(),
            principal.is_owner(),
        ) {
            Ok(result) => {
                self.audit_principal(
                    &principal,
                    "mcp.memory_candidate.classify",
                    None,
                    None,
                    "succeeded",
                    Some(1),
                    started,
                );
                serde_json::to_string(&result).unwrap_or_else(|error| error.to_string())
            }
            Err(error) => {
                self.audit_principal(
                    &principal,
                    "mcp.memory_candidate.classify",
                    None,
                    None,
                    "failed",
                    None,
                    started,
                );
                format!("candidate classification error: {error}")
            }
        }
    }

    #[tool(
        description = "Reflect over authorized active memory and optional scoped evidence without mutating canonical memory."
    )]
    async fn reflect_memory(
        &self,
        Parameters(request): Parameters<crate::reflection::ReflectRequest>,
    ) -> String {
        let started = Instant::now();
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => return format!("authorization error: {error}"),
        };
        if !principal.has_scope(MEMORY_SCOPE) {
            return "authorization error: memory scope required".into();
        }
        match crate::reflection::reflect_authorized(
            &self.store,
            &self.embedder,
            &request,
            &principal.visible_acl(),
            principal.is_owner(),
        )
        .await
        {
            Ok(response) => {
                self.audit_principal(
                    &principal,
                    "mcp.memory.reflect",
                    request.project.as_deref(),
                    request.source.as_deref(),
                    "succeeded",
                    Some(response.metrics.memories_included),
                    started,
                );
                serde_json::to_string(&response).unwrap_or_else(|error| error.to_string())
            }
            Err(error) => {
                self.audit_principal(
                    &principal,
                    "mcp.memory.reflect",
                    request.project.as_deref(),
                    request.source.as_deref(),
                    "failed",
                    None,
                    started,
                );
                format!("memory reflection error: {error}")
            }
        }
    }

    async fn update_memory_candidate(&self, id: String, redact: bool) -> String {
        let started = Instant::now();
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => return format!("authorization error: {error}"),
        };
        if !principal.has_scope(MEMORY_SCOPE) {
            return "authorization error: memory scope required".into();
        }
        let action = if redact {
            "mcp.memory_candidate.redact"
        } else {
            "mcp.memory_candidate.cancel"
        };
        let result = if redact {
            self.store.redact_memory_candidate_scoped(
                &id,
                &principal.name,
                &principal.visible_acl(),
                principal.is_owner(),
            )
        } else {
            self.store.cancel_memory_candidate_scoped(
                &id,
                &principal.name,
                &principal.visible_acl(),
                principal.is_owner(),
            )
        };
        match result {
            Ok(true) => {
                self.audit_principal(
                    &principal,
                    action,
                    None,
                    None,
                    "succeeded",
                    Some(1),
                    started,
                );
                serde_json::json!({"id": id, "updated": true}).to_string()
            }
            Ok(false) => {
                self.audit_principal(
                    &principal,
                    action,
                    None,
                    None,
                    "not_found",
                    Some(0),
                    started,
                );
                "pending candidate not found".into()
            }
            Err(error) => {
                self.audit_principal(&principal, action, None, None, "failed", None, started);
                format!("candidate update error: {error}")
            }
        }
    }

    #[tool(
        description = "Recall relevant active memories from Cortana's native store with project, type, confidence, provenance, and ACL filtering."
    )]
    async fn recall(&self, Parameters(params): Parameters<MemoryRecallParams>) -> String {
        let started = Instant::now();
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => {
                self.audit_as(
                    "mcp-unauthenticated",
                    "mcp.recall",
                    params.project.as_deref(),
                    None,
                    "unauthorized",
                    None,
                    started,
                );
                return format!("authorization error: {error}");
            }
        };
        if !principal.has_scope(MEMORY_SCOPE) {
            self.audit_principal(
                &principal,
                "mcp.recall",
                params.project.as_deref(),
                None,
                "forbidden",
                None,
                started,
            );
            return "authorization error: memory scope required".into();
        }
        if let Err(error) = validate_request(&params.query, params.project.as_deref(), None) {
            self.audit_principal(
                &principal,
                "mcp.recall",
                params.project.as_deref(),
                None,
                "invalid",
                None,
                started,
            );
            return format!("invalid request: {error}");
        }
        let recalled = if principal.is_owner() {
            self.store.recall_memories_with_axes_as_owner(
                &params.query,
                params.project.as_deref(),
                params.kind.as_deref(),
                params.content_type.as_deref(),
                params.retention_tier.as_deref(),
                params.scope.as_deref(),
                params.limit.unwrap_or(10),
            )
        } else {
            self.store.recall_memories_with_axes(
                &params.query,
                params.project.as_deref(),
                params.kind.as_deref(),
                params.content_type.as_deref(),
                params.retention_tier.as_deref(),
                params.scope.as_deref(),
                params.limit.unwrap_or(10),
                &principal.visible_acl(),
            )
        };
        match recalled {
            Ok(memories) => {
                self.audit_principal(
                    &principal,
                    "mcp.recall",
                    params.project.as_deref(),
                    None,
                    "succeeded",
                    Some(memories.len()),
                    started,
                );
                serde_json::to_string(&memories).unwrap_or_else(|error| error.to_string())
            }
            Err(error) => {
                self.audit_principal(
                    &principal,
                    "mcp.recall",
                    params.project.as_deref(),
                    None,
                    "failed",
                    None,
                    started,
                );
                format!("memory recall error: {error}")
            }
        }
    }

    #[tool(
        description = "Redact one native memory while retaining a minimal tombstone for auditability."
    )]
    async fn forget(&self, Parameters(params): Parameters<MemoryForgetParams>) -> String {
        let started = Instant::now();
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => {
                self.audit_as(
                    "mcp-unauthenticated",
                    "mcp.forget",
                    None,
                    None,
                    "unauthorized",
                    None,
                    started,
                );
                return format!("authorization error: {error}");
            }
        };
        if !principal.has_scope(MEMORY_SCOPE) {
            self.audit_principal(
                &principal,
                "mcp.forget",
                None,
                None,
                "forbidden",
                None,
                started,
            );
            return "authorization error: memory scope required".into();
        }
        let memory = match self.store.memory(&params.id) {
            Ok(Some(memory)) => memory,
            Ok(None) => {
                self.audit_principal(
                    &principal,
                    "mcp.forget",
                    None,
                    None,
                    "not_found",
                    None,
                    started,
                );
                return "memory not found".into();
            }
            Err(error) => return format!("memory error: {error}"),
        };
        if !principal.is_owner() && !acl_allows(&memory.acl, &principal.visible_acl()) {
            self.audit_principal(
                &principal,
                "mcp.forget",
                Some(&memory.project),
                Some(&memory.source),
                "forbidden",
                None,
                started,
            );
            return "authorization error: memory ACL denied".into();
        }
        match self.store.forget_memory_scoped(
            &params.id,
            &principal.visible_acl(),
            principal.is_owner(),
        ) {
            Ok(forgotten) => {
                self.audit_principal(
                    &principal,
                    "mcp.forget",
                    Some(&memory.project),
                    Some(&memory.source),
                    "succeeded",
                    Some(usize::from(forgotten)),
                    started,
                );
                serde_json::json!({"id": params.id, "forgotten": forgotten}).to_string()
            }
            Err(error) if crate::memory::is_authorization_error(&error) => {
                self.audit_principal(
                    &principal,
                    "mcp.forget",
                    Some(&memory.project),
                    Some(&memory.source),
                    "forbidden",
                    None,
                    started,
                );
                "authorization error: memory ACL denied".into()
            }
            Err(error) => format!("memory error: {error}"),
        }
    }

    #[tool(
        description = "Export bounded native memory records visible to this principal, including redacted tombstones"
    )]
    async fn export_memory(&self, Parameters(params): Parameters<MemoryExportParams>) -> String {
        let started = Instant::now();
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => {
                self.audit_as(
                    "mcp-unauthenticated",
                    "mcp.memory_export",
                    params.project.as_deref(),
                    None,
                    "unauthorized",
                    None,
                    started,
                );
                return format!("authorization error: {error}");
            }
        };
        if !principal.has_scope(MEMORY_SCOPE) {
            self.audit_principal(
                &principal,
                "mcp.memory_export",
                params.project.as_deref(),
                None,
                "forbidden",
                None,
                started,
            );
            return "authorization error: memory scope required".into();
        }
        let limit = params
            .limit
            .unwrap_or(10_000)
            .min(crate::memory::MAX_MEMORY_EXPORT_LIMIT);
        let exported = if principal.is_owner() {
            self.store.export_memories_with_axes_as_owner(
                params.project.as_deref(),
                params.kind.as_deref(),
                params.content_type.as_deref(),
                params.retention_tier.as_deref(),
                params.scope.as_deref(),
                limit,
            )
        } else {
            self.store.export_memories_with_axes(
                params.project.as_deref(),
                params.kind.as_deref(),
                params.content_type.as_deref(),
                params.retention_tier.as_deref(),
                params.scope.as_deref(),
                limit,
                &principal.visible_acl(),
            )
        };
        match exported {
            Ok(memories) => {
                self.audit_principal(
                    &principal,
                    "mcp.memory_export",
                    params.project.as_deref(),
                    None,
                    "succeeded",
                    Some(memories.len()),
                    started,
                );
                serde_json::to_string(&memories).unwrap_or_else(|error| error.to_string())
            }
            Err(error) => {
                self.audit_principal(
                    &principal,
                    "mcp.memory_export",
                    params.project.as_deref(),
                    None,
                    "failed",
                    None,
                    started,
                );
                format!("memory export error: {error}")
            }
        }
    }

    #[tool(
        description = "Report index health, configured source coverage, embedding identity, and persistent cache telemetry without exposing credentials"
    )]
    async fn brain_status(&self) -> String {
        let started = Instant::now();
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => {
                self.audit_as(
                    "mcp-unauthenticated",
                    "mcp.brain_status",
                    None,
                    None,
                    "unauthorized",
                    None,
                    started,
                );
                return format!("authorization error: {error}");
            }
        };
        if !principal.has_scope(STATUS_SCOPE) {
            self.audit_principal(
                &principal,
                "mcp.brain_status",
                None,
                None,
                "forbidden",
                None,
                started,
            );
            return "authorization error: status scope required".into();
        }
        let acl = principal.visible_acl();
        match if principal.is_owner() {
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
                let memory = memory_stats_with_timeout(
                    self.store.clone(),
                    (!principal.is_owner()).then(|| acl.clone()),
                    STATS_TIMEOUT,
                )
                .await
                .unwrap_or_default();
                let result = serde_json::to_string(&BrainStatus {
                    stats,
                    memory,
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
                        self.audit_principal(
                            &principal,
                            "mcp.brain_status",
                            None,
                            None,
                            "succeeded",
                            count,
                            started,
                        );
                        payload
                    }
                    Err(error) => {
                        self.audit_principal(
                            &principal,
                            "mcp.brain_status",
                            None,
                            None,
                            "failed",
                            None,
                            started,
                        );
                        error.to_string()
                    }
                }
            }
            Err(error) => {
                self.audit_principal(
                    &principal,
                    "mcp.brain_status",
                    None,
                    None,
                    "failed",
                    None,
                    started,
                );
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

async fn memory_stats_with_timeout(
    store: Store,
    principal_acl: Option<Vec<String>>,
    timeout: Duration,
) -> anyhow::Result<MemoryStats> {
    let task = tokio::task::spawn_blocking(move || match principal_acl {
        Some(acl) => store.memory_stats_scoped(&acl),
        None => store.memory_stats(),
    });
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(anyhow::anyhow!(
            "MCP memory stats probe worker failed: {error}"
        )),
        Err(_) => Err(anyhow::anyhow!(
            "MCP memory stats probe timed out after {timeout:?}"
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
        let principal = match self.resolve_principal() {
            Ok(principal) => principal,
            Err(error) => {
                self.audit_as(
                    "mcp-unauthenticated",
                    action,
                    params.project.as_deref(),
                    None,
                    "unauthorized",
                    None,
                    started,
                );
                return format!("authorization error: {error}");
            }
        };
        if !principal.has_scope(QUERY_SCOPE) {
            self.audit_principal(
                &principal,
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
            self.audit_principal(
                &principal,
                action,
                params.project.as_deref(),
                None,
                "invalid",
                None,
                started,
            );
            return format!("invalid request: {error}");
        }
        let acl = principal.visible_acl();
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
                self.audit_principal(
                    &principal,
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
                self.audit_principal(
                    &principal,
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
    fn audit_as(
        &self,
        principal: &str,
        action: &str,
        project: Option<&str>,
        source: Option<&str>,
        outcome: &str,
        count: Option<usize>,
        started: Instant,
    ) {
        if let Err(error) = self.store.record_audit(
            principal,
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

    #[allow(clippy::too_many_arguments)]
    fn audit_principal(
        &self,
        principal: &Principal,
        action: &str,
        project: Option<&str>,
        source: Option<&str>,
        outcome: &str,
        count: Option<usize>,
        started: Instant,
    ) {
        if let Err(error) = self.store.record_audit(
            &principal.name,
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
            "Call context before answering questions about the user or their work. Use recall for durable agent memory, remember only explicit conclusions with provenance, and forget when a memory is withdrawn. Prefer search_code, search_messages, or who_knows for narrow discovery.",
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
    async fn reloadable_mcp_principal_tracks_file_rotation_and_fails_closed() {
        let directory = tempdir().expect("temporary directory");
        let store = Store::open(&directory.path().join("store.sqlite3")).expect("store");
        let embedder: Arc<dyn Embedder> = Arc::new(DeterministicEmbedder::new(16));
        let config_path = directory.path().join("config.toml");
        let secrets_path = directory.path().join("secrets.env");
        let mut config = Config {
            data_dir: directory.path().to_path_buf(),
            ..Config::default()
        };
        config.runtime.env_file = Some(secrets_path.clone());
        config.auth.tokens = vec![AuthTokenConfig {
            principal: "old-agent".into(),
            token_env: "MCP_TOKEN".into(),
            scopes: vec![STATUS_SCOPE.into()],
            acl: vec!["work".into()],
        }];
        std::fs::write(
            &config_path,
            toml::to_string(&config).expect("serialize config"),
        )
        .expect("write config");
        write_private_fixture(&secrets_path, "MCP_TOKEN=old-secret\n");

        let server = BrainServer::new(store.clone(), embedder)
            .with_reloadable_principal(&config_path, "MCP_TOKEN")
            .with_audit_limit(10);
        assert!(server.brain_status().await.contains("configured_sources"));
        assert_eq!(
            store.audit_events(1).expect("audit")[0].principal,
            "old-agent"
        );

        config.auth.tokens[0].principal = "replacement-agent".into();
        std::fs::write(
            &config_path,
            toml::to_string(&config).expect("serialize rotation"),
        )
        .expect("rotate config");
        write_private_fixture(&secrets_path, "MCP_TOKEN=replacement-secret\n");
        assert!(server.brain_status().await.contains("configured_sources"));
        assert_eq!(
            store.audit_events(1).expect("audit")[0].principal,
            "replacement-agent"
        );

        write_private_fixture(&secrets_path, "MCP_TOKEN=\n");
        assert!(
            server
                .brain_status()
                .await
                .starts_with("authorization error:")
        );
        assert_eq!(
            store.audit_events(1).expect("audit")[0].principal,
            "mcp-unauthenticated"
        );
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
        store
            .remember(&MemoryInput {
                kind: "semantic".into(),
                project: "demo".into(),
                title: "Launch memory".into(),
                content: "The shared launch phrase is approved for work.".into(),
                source: "agent".into(),
                source_id: String::new(),
                dedupe_key: Some("test:admin-launch-memory".into()),
                confidence: 0.9,
                importance: 0.8,
                acl: vec!["work".into()],
                provenance: serde_json::json!({"test":true}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect("memory");
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
                scopes: vec![QUERY_SCOPE.into(), ADMIN_SCOPE.into(), MEMORY_SCOPE.into()],
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
        assert!(work_context.get("memories").is_none());

        assert_eq!(
            work_server
                .recall(Parameters(MemoryRecallParams {
                    query: "shared launch phrase".into(),
                    project: Some("demo".into()),
                    kind: None,
                    content_type: None,
                    retention_tier: None,
                    scope: None,
                    limit: Some(10),
                }))
                .await,
            "authorization error: memory scope required"
        );

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
        assert_eq!(admin_context["memories"].as_array().map(Vec::len), Some(1));

        let admin_memories: Vec<crate::memory::MemorySearchResult> = serde_json::from_str(
            &admin_server
                .recall(Parameters(MemoryRecallParams {
                    query: "shared launch phrase".into(),
                    project: Some("demo".into()),
                    kind: None,
                    content_type: None,
                    retention_tier: None,
                    scope: None,
                    limit: Some(10),
                }))
                .await,
        )
        .expect("admin memory recall");
        assert_eq!(admin_memories.len(), 1);
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
        store
            .remember(&MemoryInput {
                kind: "semantic".into(),
                project: "work".into(),
                title: "Work status memory".into(),
                content: "Work status context.".into(),
                source: "agent".into(),
                source_id: "work-status-memory".into(),
                dedupe_key: None,
                confidence: 0.8,
                importance: 0.7,
                acl: vec!["work".into()],
                provenance: serde_json::json!({"test":true}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect("work memory");
        store
            .remember(&MemoryInput {
                kind: "semantic".into(),
                project: "personal".into(),
                title: "Personal status memory".into(),
                content: "Personal status context.".into(),
                source: "agent".into(),
                source_id: "personal-status-memory".into(),
                dedupe_key: None,
                confidence: 0.8,
                importance: 0.7,
                acl: vec!["personal".into()],
                provenance: serde_json::json!({"test":true}),
                supersedes_id: None,
                valid_until: None,
            })
            .expect("personal memory");
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
        assert_eq!(status["memory"]["active"], 1);
        assert_eq!(status["memory"]["total"], 1);
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
