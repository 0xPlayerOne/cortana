//! Bounded, read-only provider model catalog discovery for the Desktop.
//!
//! The renderer supplies only a fixed kind (`embedding` or `query`); it cannot
//! inject a command, URL, or credential. The bundled `cortana` CLI resolves
//! the configured provider from the saved config, validates the endpoint,
//! queries `/models` with a strict timeout, and prints sanitized model ids
//! plus explicit capability metadata. This module re-validates that payload
//! before it crosses the IPC boundary so a defective or compromised runtime
//! cannot push unsafe metadata into the renderer.

use std::time::Duration;

use serde_json::Value;
use tauri_plugin_shell::{ShellExt, process::CommandEvent};
use tokio::time::timeout;

use crate::source_jobs::{append_bounded_bytes, terminate_source_process};

const PROVIDER_MODELS_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PROVIDER_MODELS_BYTES: usize = 2 * 1024 * 1024;
const MAX_MODELS: usize = 512;
const MAX_MODEL_ID_CHARS: usize = 128;
const MAX_METADATA_CHARS: usize = 128;
const MAX_PROVIDER_URL_CHARS: usize = 2048;

/// Discover the models advertised by the configured provider through the
/// bundled CLI. Read-only: never writes configuration, secrets, or data, and
/// never prints the provider API key.
pub async fn list_provider_models<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    kind: &str,
) -> Result<Value, String> {
    if !matches!(kind, "embedding" | "query") {
        return Err("provider model discovery supports only embedding and query providers".into());
    }
    let command = app
        .shell()
        .sidecar("cortana")
        .map_err(|error| format!("locate bundled Cortana runtime: {error}"))?
        .args(["provider-models", "--kind", kind])
        .env("CORTANA_DESKTOP_PROCESS_GROUP", "1")
        .set_raw_out(true);
    let (mut receiver, child) = command
        .spawn()
        .map_err(|error| format!("start provider model discovery: {error}"))?;
    let result = timeout(PROVIDER_MODELS_TIMEOUT, async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut success = false;
        while let Some(event) = receiver.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    append_bounded_bytes(&mut stdout, &bytes, MAX_PROVIDER_MODELS_BYTES)
                }
                CommandEvent::Stderr(bytes) => {
                    append_bounded_bytes(&mut stderr, &bytes, MAX_PROVIDER_MODELS_BYTES)
                }
                CommandEvent::Error(error) => {
                    return Err(format!("provider model discovery failed: {error}"));
                }
                CommandEvent::Terminated(payload) => {
                    success = payload.code == Some(0);
                    break;
                }
                _ => {}
            }
        }
        Ok((success, stdout, stderr))
    })
    .await;
    let (success, stdout, _stderr) = match result {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            let _ = terminate_source_process(child);
            return Err(error);
        }
        Err(_) => {
            let _ = terminate_source_process(child);
            return Err("provider model discovery timed out".into());
        }
    };
    if !success {
        return Err("provider model discovery failed; check the provider endpoint".into());
    }
    if stdout.len() >= MAX_PROVIDER_MODELS_BYTES {
        return Err("provider model discovery response exceeded 2 MiB".into());
    }
    let value: Value = serde_json::from_slice(&stdout)
        .map_err(|_| "provider model discovery returned invalid JSON".to_string())?;
    validate_provider_models_payload(&value, kind)?;
    Ok(value)
}

/// Re-validate the sidecar's model catalog payload before it crosses the IPC
/// boundary. Ids, metadata, and the provider URL are bounded exactly like the
/// CLI enforces them.
fn validate_provider_models_payload(value: &Value, kind: &str) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "provider model discovery returned an invalid payload".to_string())?;
    if object.get("kind").and_then(Value::as_str) != Some(kind) {
        return Err("provider model discovery returned an unexpected kind".into());
    }
    if let Some(truncated) = object.get("truncated") {
        if !truncated.is_boolean() {
            return Err("provider model discovery returned an invalid truncation flag".into());
        }
    }
    let provider = object
        .get("provider")
        .and_then(Value::as_str)
        .ok_or_else(|| "provider model discovery returned an invalid provider".to_string())?;
    validate_provider_url(provider)?;
    let models = object
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| "provider model discovery returned an invalid model list".to_string())?;
    if models.len() > MAX_MODELS {
        return Err("provider model discovery returned too many models".into());
    }
    for model in models {
        let model = model
            .as_object()
            .ok_or_else(|| "provider model discovery returned an invalid model".to_string())?;
        let id = model
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "provider model discovery returned an invalid model id".to_string())?;
        if id.is_empty()
            || id.len() > MAX_MODEL_ID_CHARS
            || !id.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err("provider model discovery returned an unsafe model id".into());
        }
        for key in ["object", "owned_by"] {
            if let Some(value) = model.get(key) {
                if !value.is_null() {
                    let text = value.as_str().ok_or_else(|| {
                        "provider model discovery returned invalid model metadata".to_string()
                    })?;
                    if text.is_empty()
                        || text.chars().count() > MAX_METADATA_CHARS
                        || text.chars().any(char::is_control)
                    {
                        return Err(
                            "provider model discovery returned unsafe model metadata".into()
                        );
                    }
                }
            }
        }
        if let Some(created) = model.get("created") {
            if !created.is_null() && !created.is_u64() {
                return Err("provider model discovery returned invalid model metadata".into());
            }
        }
        if let Some(capabilities) = model.get("capabilities") {
            validate_capabilities(capabilities, 0)?;
        }
    }
    Ok(())
}

fn validate_capabilities(value: &Value, depth: usize) -> Result<(), String> {
    const MAX_CAPABILITIES_DEPTH: usize = 8;
    const MAX_CAPABILITIES_BYTES: usize = 4096;

    if depth > MAX_CAPABILITIES_DEPTH {
        return Err("provider model discovery returned deeply nested capabilities".into());
    }
    if serde_json::to_vec(value)
        .map_err(|_| "provider model discovery returned invalid capabilities".to_string())?
        .len()
        > MAX_CAPABILITIES_BYTES
    {
        return Err("provider model discovery returned oversized capabilities".into());
    }
    match value {
        Value::String(text) => {
            if text.is_empty()
                || text.chars().count() > MAX_METADATA_CHARS
                || text.chars().any(char::is_control)
            {
                return Err("provider model discovery returned unsafe capabilities".into());
            }
        }
        Value::Array(items) => {
            for item in items {
                validate_capabilities(item, depth + 1)?;
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                if key.is_empty()
                    || key.chars().count() > MAX_METADATA_CHARS
                    || key.chars().any(char::is_control)
                {
                    return Err("provider model discovery returned unsafe capabilities".into());
                }
                validate_capabilities(item, depth + 1)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

/// The provider URL is already validated by the shared CLI contract (HTTPS,
/// or HTTP only on loopback, no credentials, no query, no fragment). Re-check
/// it here so the renderer never receives an unsafe endpoint.
fn validate_provider_url(provider: &str) -> Result<(), String> {
    if provider.is_empty()
        || provider.len() > MAX_PROVIDER_URL_CHARS
        || provider.chars().any(char::is_control)
    {
        return Err("provider model discovery returned an unsafe provider URL".into());
    }
    let url = reqwest::Url::parse(provider)
        .map_err(|_| "provider model discovery returned an invalid provider URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err("provider model discovery returned an unsafe provider URL".into());
    }
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    if url.scheme() == "http" && !loopback {
        return Err("provider model discovery returned an unsafe provider URL".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn payload_validation_accepts_a_sanitized_catalog() {
        let payload = json!({
            "kind": "embedding",
            "provider": "http://127.0.0.1:6999/v1",
            "truncated": false,
            "models": [
                {
                    "id": "Qwen/Qwen3-Embedding-0.6B",
                    "object": "model",
                    "owned_by": "local",
                    "created": 1700000000,
                    "capabilities": ["embedding"]
                },
                { "id": "text-embedding-3-small", "created": null, "capabilities": null }
            ]
        });
        validate_provider_models_payload(&payload, "embedding").expect("sanitized catalog");
    }

    #[test]
    fn payload_validation_rejects_kind_mismatches_and_unsafe_ids() {
        let payload = json!({
            "kind": "query",
            "provider": "https://api.example.test/v1",
            "truncated": false,
            "models": []
        });
        let error = validate_provider_models_payload(&payload, "embedding")
            .expect_err("kind mismatch must fail closed");
        assert!(error.contains("unexpected kind"));

        for id in ["", "with space", "tab\tinside", "unicode-\u{00e9}"] {
            let payload = json!({
                "kind": "query",
                "provider": "https://api.example.test/v1",
                "truncated": false,
                "models": [{ "id": id }]
            });
            let error = validate_provider_models_payload(&payload, "query")
                .expect_err("unsafe model id must fail closed");
            assert!(
                error.contains("unsafe model id"),
                "unexpected error: {error}"
            );
        }
        let payload = json!({
            "kind": "query",
            "provider": "https://api.example.test/v1",
            "truncated": false,
            "models": [{ "id": "x".repeat(129) }]
        });
        let error = validate_provider_models_payload(&payload, "query")
            .expect_err("oversized model id must fail closed");
        assert!(error.contains("unsafe model id"));
    }

    #[test]
    fn payload_validation_rejects_unsafe_provider_urls() {
        for provider in [
            "ftp://127.0.0.1/v1",
            "http://api.example.test/v1",
            "https://user:pass@api.example.test/v1",
            "https://api.example.test/v1?token=secret",
            "https://api.example.test/v1#fragment",
            "https://api.example.test/v1\u{0}",
        ] {
            let payload = json!({
                "kind": "query",
                "provider": provider,
                "truncated": false,
                "models": []
            });
            let error = validate_provider_models_payload(&payload, "query")
                .expect_err("unsafe provider URL must fail closed");
            assert!(
                error.contains("unsafe provider URL"),
                "unexpected error: {error}"
            );
        }
        let payload = json!({
            "kind": "query",
            "provider": "x".repeat(2049),
            "truncated": false,
            "models": []
        });
        let error = validate_provider_models_payload(&payload, "query")
            .expect_err("oversized provider URL must fail closed");
        assert!(
            error.contains("unsafe provider URL"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn payload_validation_accepts_the_maximum_catalog_size() {
        let models = (0..MAX_MODELS)
            .map(|index| json!({ "id": format!("model-{index}") }))
            .collect::<Vec<_>>();
        let payload = json!({
            "kind": "query",
            "provider": "https://api.example.test/v1",
            "truncated": true,
            "models": models
        });
        validate_provider_models_payload(&payload, "query").expect("maximum catalog is valid");
    }

    #[test]
    fn payload_validation_rejects_oversized_catalogs_and_bad_metadata() {
        let models = (0..MAX_MODELS + 1)
            .map(|index| json!({ "id": format!("model-{index}") }))
            .collect::<Vec<_>>();
        let payload = json!({
            "kind": "query",
            "provider": "https://api.example.test/v1",
            "truncated": false,
            "models": models
        });
        let error = validate_provider_models_payload(&payload, "query")
            .expect_err("oversized catalogs must fail closed");
        assert!(error.contains("too many models"));

        for (key, value) in [
            ("object", "x".repeat(129)),
            ("owned_by", "bad\nvalue".to_string()),
            ("created", "not-a-number".to_string()),
        ] {
            let payload = json!({
                "kind": "query",
                "provider": "https://api.example.test/v1",
                "truncated": false,
                "models": [{ "id": "model-a", key: value }]
            });
            let error = validate_provider_models_payload(&payload, "query")
                .expect_err("unsafe metadata must fail closed");
            assert!(error.contains("metadata"), "unexpected error: {error}");
        }

        let payload = json!({
            "kind": "query",
            "provider": "https://api.example.test/v1",
            "truncated": "yes",
            "models": []
        });
        let error = validate_provider_models_payload(&payload, "query")
            .expect_err("invalid truncation flag must fail closed");
        assert!(error.contains("truncation flag"));
    }

    #[test]
    fn payload_validation_rejects_unsafe_capabilities_and_missing_hosts() {
        let payload = json!({
            "kind": "query",
            "provider": "https://:443/v1",
            "truncated": false,
            "models": []
        });
        let error = validate_provider_models_payload(&payload, "query")
            .expect_err("provider URLs without hosts must fail closed");
        assert!(error.contains("provider URL"));

        let payload = json!({
            "kind": "query",
            "provider": "https://api.example.test/v1",
            "truncated": false,
            "models": [{ "id": "model-a", "capabilities": { "chat\n": true } }]
        });
        let error = validate_provider_models_payload(&payload, "query")
            .expect_err("unsafe capabilities must fail closed");
        assert!(error.contains("capabilities"));
    }
}
