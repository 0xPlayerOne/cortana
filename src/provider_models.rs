//! Bounded, read-only model catalog discovery for OpenAI-compatible providers.
//!
//! `cortana provider-models --kind embedding|query` queries the configured
//! provider's `/models` endpoint so the Desktop settings can offer the models
//! the provider actually advertises. This module:
//!
//! - reuses the shared provider base-URL contract (HTTPS, or HTTP only on
//!   loopback) before any request is attempted, so credentials and document
//!   text never travel to an unvalidated host;
//! - never follows redirects, never retries, and enforces a strict fixed
//!   timeout on the discovery call;
//! - bounds the response body, the model count, and every echoed field;
//! - returns sanitized model ids plus capability metadata only when the
//!   provider explicitly advertises it — capabilities are never inferred from
//!   model ids or names with fuzzy matching;
//! - never prints or stores the provider API key; errors name only the
//!   configured environment-variable name.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::{Client, redirect::Policy};
use serde::{Deserialize, Serialize};

use crate::config::{Config, validate_provider_base_url};

/// Strict wall-clock bound for the lightweight `/models` discovery call.
const PROVIDER_MODELS_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_MODELS: usize = 512;
const MAX_MODEL_ID_CHARS: usize = 128;
const MAX_TEXT_CHARS: usize = 128;
const MAX_CAPABILITIES_BYTES: usize = 4096;
const MAX_CAPABILITIES_DEPTH: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    Embedding,
    Query,
}

impl ModelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelKind::Embedding => "embedding",
            ModelKind::Query => "query",
        }
    }
}

/// Safe model catalog returned to the CLI and Desktop. Every field is
/// sanitized and bounded; the list never contains credentials.
#[derive(Debug, Serialize)]
pub struct ProviderModelList {
    pub kind: String,
    /// Normalized provider base URL the catalog was fetched from. It passed
    /// the shared base-URL contract, so it cannot carry credentials, query
    /// parameters, or a fragment.
    pub provider: String,
    pub models: Vec<ModelEntry>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct ModelEntry {
    pub id: String,
    pub object: Option<String>,
    pub owned_by: Option<String>,
    pub created: Option<u64>,
    /// Explicit capability metadata advertised by the provider, echoed
    /// verbatim (sanitized and bounded). Omitted when the provider does not
    /// advertise capabilities; never inferred from the model id or name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ApiModelList {
    data: Vec<ApiModel>,
}

#[derive(Debug, Deserialize)]
struct ApiModel {
    id: String,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default)]
    created: Option<u64>,
    #[serde(default)]
    capabilities: Option<serde_json::Value>,
}

/// List the models advertised by the configured provider for the requested
/// kind. Read-only: never writes configuration, secrets, or data.
pub async fn list_provider_models(config: &Config, kind: ModelKind) -> Result<ProviderModelList> {
    let (base_url, model, api_key_env) = match kind {
        ModelKind::Embedding => (
            &config.embedding.base_url,
            &config.embedding.model,
            config.embedding.api_key_env.as_deref(),
        ),
        ModelKind::Query => (
            &config.query.base_url,
            &config.query.model,
            config.query.api_key_env.as_deref(),
        ),
    };
    validate_provider_base_url(kind.as_str(), base_url)?;
    let provider_url =
        reqwest::Url::parse(base_url).context("parse provider model discovery URL")?;
    anyhow::ensure!(
        provider_url.host_str().is_some(),
        "{} provider URL must include a host",
        kind.as_str()
    );
    anyhow::ensure!(
        !model.trim().is_empty(),
        "{} model must not be empty",
        kind.as_str()
    );
    let api_key = match api_key_env {
        Some(name) => Some(
            config
                .environment_value(name)
                .with_context(|| format!("{name} is not set"))?,
        ),
        None => None,
    };

    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(PROVIDER_MODELS_TIMEOUT)
        .redirect(Policy::none())
        .user_agent(format!("cortana/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build provider model discovery client")?;
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let mut request = client.get(&url);
    if let Some(key) = &api_key {
        request = request.bearer_auth(key);
    }
    let response = request.send().await.context("request provider /models")?;
    let status = response.status();
    anyhow::ensure!(
        status.is_success(),
        "provider /models request failed with status {}",
        status.as_u16()
    );
    let body = bounded_body(response).await?;
    let parsed: ApiModelList =
        serde_json::from_slice(&body).context("provider /models returned invalid JSON")?;
    let truncated = parsed.data.len() >= MAX_MODELS;
    let mut models = Vec::new();
    for item in parsed.data.into_iter().take(MAX_MODELS) {
        models.push(ModelEntry {
            id: sanitize_model_id(&item.id)?,
            object: sanitize_optional_text(item.object, "object")?,
            owned_by: sanitize_optional_text(item.owned_by, "owned_by")?,
            created: item.created,
            capabilities: match item.capabilities {
                Some(value) => Some(sanitize_capabilities(value)?),
                None => None,
            },
        });
    }
    Ok(ProviderModelList {
        kind: kind.as_str().into(),
        provider: provider_url.as_str().trim_end_matches('/').to_string(),
        models,
        truncated,
    })
}

async fn bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        bail!("provider /models response exceeded the safety limit");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("read provider /models response")?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            bail!("provider /models response exceeded the safety limit");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Model ids become selectable option values in the Desktop renderer, so they
/// are restricted to non-empty, bounded, printable ASCII without whitespace or
/// control characters.
fn sanitize_model_id(value: &str) -> Result<String> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= MAX_MODEL_ID_CHARS
            && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte)),
        "provider /models returned an invalid model id"
    );
    Ok(value.to_string())
}

fn sanitize_optional_text(value: Option<String>, label: &str) -> Result<Option<String>> {
    value.map(|text| sanitize_text(&text, label)).transpose()
}

fn sanitize_text(value: &str, label: &str) -> Result<String> {
    let sanitized = value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>()
        .trim()
        .to_string();
    anyhow::ensure!(
        !sanitized.is_empty(),
        "provider /models returned an empty {label}"
    );
    anyhow::ensure!(
        sanitized.chars().count() <= MAX_TEXT_CHARS,
        "provider /models returned an oversized {label}"
    );
    Ok(sanitized)
}

/// Capability metadata is echoed only when the provider advertises it, and
/// only within strict bounds: limited nesting, sanitized strings, and a
/// serialized size cap per model entry. Anything outside those bounds fails
/// closed rather than crossing the process boundary.
fn sanitize_capabilities(value: serde_json::Value) -> Result<serde_json::Value> {
    let sanitized = sanitize_capabilities_at(value, 0)?;
    anyhow::ensure!(
        serde_json::to_vec(&sanitized)
            .context("serialize provider capability metadata")?
            .len()
            <= MAX_CAPABILITIES_BYTES,
        "provider capability metadata is too large"
    );
    Ok(sanitized)
}

fn sanitize_capabilities_at(value: serde_json::Value, depth: usize) -> Result<serde_json::Value> {
    anyhow::ensure!(
        depth <= MAX_CAPABILITIES_DEPTH,
        "provider capability metadata is too deeply nested"
    );
    match value {
        serde_json::Value::String(text) => Ok(serde_json::Value::String(sanitize_text(
            &text,
            "capability",
        )?)),
        serde_json::Value::Array(items) => {
            let mut sanitized = Vec::with_capacity(items.len());
            for item in items {
                sanitized.push(sanitize_capabilities_at(item, depth + 1)?);
            }
            Ok(serde_json::Value::Array(sanitized))
        }
        serde_json::Value::Object(map) => {
            let mut sanitized = serde_json::Map::new();
            for (key, item) in map {
                let key = sanitize_text(&key, "capability key")?;
                sanitized.insert(key, sanitize_capabilities_at(item, depth + 1)?);
            }
            Ok(serde_json::Value::Object(sanitized))
        }
        // Numbers, booleans, and null pass through unchanged.
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::{Router, body::Body, routing::get};
    use serde_json::json;

    use super::*;

    fn config_with(base_url: &str) -> Config {
        let mut config = Config::default();
        config.embedding.base_url = base_url.into();
        config.query.base_url = base_url.into();
        config.embedding.api_key_env = Some("CORTANA_TEST_PROVIDER_API_KEY".into());
        config.environment.insert(
            "CORTANA_TEST_PROVIDER_API_KEY".into(),
            "super-secret-provider-key".into(),
        );
        config
    }

    fn json_body(value: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&value).expect("serialize test payload")
    }

    async fn serve(body: Vec<u8>) -> String {
        let app = Router::new().route("/v1/models", get(move || async { Body::from(body) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind provider server");
        let address = listener.local_addr().expect("provider address");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve provider /models");
        });
        format!("http://{address}/v1")
    }

    #[tokio::test]
    async fn discovery_returns_sanitized_ids_and_advertised_capabilities() {
        let base_url = serve(json_body(json!({
            "object": "list",
            "data": [
                {
                    "id": "text-embedding-3-small",
                    "object": "model",
                    "created": 1677610602,
                    "owned_by": "openai",
                    "capabilities": ["embedding"]
                },
                {
                    "id": "Qwen/Qwen3-Embedding-0.6B",
                    "object": "model",
                    "owned_by": "local",
                    "capabilities": { "embedding": true, "max_input_tokens": 32768 }
                }
            ]
        })))
        .await;
        let config = config_with(&base_url);

        let list = list_provider_models(&config, ModelKind::Embedding)
            .await
            .expect("embedding catalog");
        assert_eq!(list.kind, "embedding");
        assert_eq!(list.provider, base_url.trim_end_matches('/'));
        assert!(!list.truncated);
        assert_eq!(list.models.len(), 2);
        assert_eq!(list.models[0].id, "text-embedding-3-small");
        assert_eq!(list.models[0].owned_by.as_deref(), Some("openai"));
        assert_eq!(list.models[0].created, Some(1_677_610_602));
        assert_eq!(list.models[0].capabilities, Some(json!(["embedding"])));
        assert_eq!(list.models[1].id, "Qwen/Qwen3-Embedding-0.6B");
        assert_eq!(list.models[1].owned_by.as_deref(), Some("local"));
        assert_eq!(
            list.models[1].capabilities,
            Some(json!({ "embedding": true, "max_input_tokens": 32768 }))
        );
    }

    #[tokio::test]
    async fn discovery_selects_the_kind_specific_provider() {
        let embedding_url = serve(json_body(json!({ "data": [{ "id": "embed-a" }] }))).await;
        let query_url = serve(json_body(json!({ "data": [{ "id": "chat-b" }] }))).await;
        let mut config = config_with(&embedding_url);
        config.query.base_url = query_url;

        let embedding = list_provider_models(&config, ModelKind::Embedding)
            .await
            .expect("embedding catalog");
        assert_eq!(embedding.models[0].id, "embed-a");
        assert_eq!(embedding.kind, "embedding");

        let query = list_provider_models(&config, ModelKind::Query)
            .await
            .expect("query catalog");
        assert_eq!(query.models[0].id, "chat-b");
        assert_eq!(query.kind, "query");
    }

    #[tokio::test]
    async fn capabilities_are_never_inferred_from_model_names() {
        let base_url = serve(json_body(json!({
            "data": [
                { "id": "text-embedding-3-small" },
                { "id": "provider-custom-embedding", "owned_by": "openai" }
            ]
        })))
        .await;
        let config = config_with(&base_url);

        let list = list_provider_models(&config, ModelKind::Query)
            .await
            .expect("query catalog");
        // The provider advertises no capabilities, so none may appear even
        // though the ids look like embedding/chat models.
        assert!(list.models.iter().all(|model| model.capabilities.is_none()));
        let serialized = serde_json::to_string(&list).expect("serialize catalog");
        // Capabilities are omitted when the provider does not advertise them;
        // they are never populated from the model id or name.
        assert!(!serialized.contains("\"capabilities\""));
        assert!(!serialized.contains("\"capabilities\":[\"embedding\""));
    }

    #[tokio::test]
    async fn oversized_provider_responses_are_rejected_before_deserialization() {
        let base_url = serve(vec![b'x'; MAX_RESPONSE_BYTES + 1]).await;
        let config = config_with(&base_url);

        let error = list_provider_models(&config, ModelKind::Embedding)
            .await
            .expect_err("oversized response");
        assert!(error.to_string().contains("safety limit"));
    }

    #[tokio::test]
    async fn invalid_model_ids_fail_closed() {
        for invalid in [
            "",
            "with space",
            "tab\tinside",
            "line\nbreak",
            "trailing ",
            "unicode-\u{00e9}",
            &"x".repeat(MAX_MODEL_ID_CHARS + 1),
        ] {
            let base_url = serve(json_body(json!({ "data": [{ "id": invalid }] }))).await;
            let config = config_with(&base_url);
            let error = list_provider_models(&config, ModelKind::Embedding)
                .await
                .expect_err("invalid model id must fail closed");
            assert!(
                error.to_string().contains("invalid model id"),
                "unexpected error for id {invalid:?}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn model_lists_are_truncated_with_a_flag() {
        let data = (0..MAX_MODELS + 20)
            .map(|index| json!({ "id": format!("model-{index}") }))
            .collect::<Vec<_>>();
        let base_url = serve(json_body(json!({ "data": data }))).await;
        let config = config_with(&base_url);

        let list = list_provider_models(&config, ModelKind::Embedding)
            .await
            .expect("truncated catalog");
        assert!(list.truncated);
        assert_eq!(list.models.len(), MAX_MODELS);
        assert_eq!(list.models[0].id, "model-0");
        assert_eq!(
            list.models[MAX_MODELS - 1].id,
            format!("model-{}", MAX_MODELS - 1)
        );
        assert!(
            list.models
                .iter()
                .all(|model| !model.id.contains("model-512"))
        );
    }

    #[tokio::test]
    async fn capability_metadata_is_bounded_and_sanitized() {
        // Control characters are stripped and whitespace is trimmed.
        let base_url = serve(json_body(json!({
            "data": [{ "id": "a", "capabilities": ["completion\u{0}", " embedding "] }]
        })))
        .await;
        let config = config_with(&base_url);
        let list = list_provider_models(&config, ModelKind::Embedding)
            .await
            .expect("sanitized capabilities");
        assert_eq!(
            list.models[0].capabilities,
            Some(json!(["completion", "embedding"]))
        );

        // Oversized metadata fails closed: many bounded strings whose total
        // serialized size exceeds the per-entry capability cap.
        let many = vec!["x".repeat(MAX_TEXT_CHARS); MAX_CAPABILITIES_BYTES / MAX_TEXT_CHARS + 4];
        let base_url = serve(json_body(json!({
            "data": [{ "id": "a", "capabilities": many }]
        })))
        .await;
        let config = config_with(&base_url);
        let error = list_provider_models(&config, ModelKind::Embedding)
            .await
            .expect_err("oversized capabilities must fail closed");
        assert!(error.to_string().contains("too large"));

        // Excessive nesting fails closed.
        let mut nested = json!("leaf");
        for _ in 0..MAX_CAPABILITIES_DEPTH + 2 {
            nested = json!([nested]);
        }
        let base_url = serve(json_body(json!({
            "data": [{ "id": "a", "capabilities": nested }]
        })))
        .await;
        let config = config_with(&base_url);
        let error = list_provider_models(&config, ModelKind::Embedding)
            .await
            .expect_err("deeply nested capabilities must fail closed");
        assert!(error.to_string().contains("deeply nested"));
    }

    #[tokio::test]
    async fn remote_http_provider_urls_fail_before_any_network_request() {
        let config = config_with("http://example.com/v1");
        let error = list_provider_models(&config, ModelKind::Embedding)
            .await
            .expect_err("remote HTTP must fail before the request");
        assert!(error.to_string().contains("must use HTTPS"));
    }

    #[tokio::test]
    async fn serialized_discovery_never_contains_credentials() {
        let base_url = serve(json_body(json!({
            "data": [{ "id": "model-a", "owned_by": "provider" }]
        })))
        .await;
        let config = config_with(&base_url);

        let list = list_provider_models(&config, ModelKind::Embedding)
            .await
            .expect("catalog");
        let serialized = serde_json::to_string(&list).expect("serialize catalog");
        assert!(!serialized.contains("super-secret-provider-key"));
        assert!(!serialized.contains("CORTANA_TEST_PROVIDER_API_KEY"));
    }

    #[tokio::test]
    async fn missing_api_key_environment_fails_closed_without_network() {
        let base_url = serve(json_body(json!({ "data": [] }))).await;
        let mut config = config_with(&base_url);
        config.environment.remove("CORTANA_TEST_PROVIDER_API_KEY");

        let error = list_provider_models(&config, ModelKind::Embedding)
            .await
            .expect_err("missing API key must fail closed");
        let message = error.to_string();
        assert!(message.contains("CORTANA_TEST_PROVIDER_API_KEY"));
        assert!(!message.contains("super-secret"));
    }

    #[test]
    fn provider_models_client_policy_is_strict() {
        // Bounds are fixed so the Desktop timeout and payload caps can be
        // reasoned about without reading the provider implementation.
        assert_eq!(PROVIDER_MODELS_TIMEOUT, Duration::from_secs(10));
        assert_eq!(MAX_MODELS, 512);
        assert_eq!(MAX_RESPONSE_BYTES, 2 * 1024 * 1024);
    }
}
