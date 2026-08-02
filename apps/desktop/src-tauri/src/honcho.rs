use std::time::Duration;

use reqwest::{Client, Url};
use serde::Serialize;

use crate::settings;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(4);

/// Bounded, metadata-only status for the optional Honcho sidecar.
///
/// Honcho does not expose a stable provider health endpoint across hosted and
/// self-hosted deployments, so the probe requests the configured base URL.
/// A response proves network reachability; only a successful response is
/// reported as healthy. Response bodies are never read or returned.
#[derive(Debug, Clone, Serialize)]
pub struct HonchoStatus {
    pub enabled: bool,
    pub configured: bool,
    pub reachable: bool,
    pub state: &'static str,
    pub endpoint: String,
    pub workspace_id: String,
    pub peer_id: String,
    pub token_configured: bool,
    pub detail: Option<String>,
}

pub async fn status() -> Result<HonchoStatus, String> {
    let snapshot = settings::load()?;
    let config = snapshot.honcho;
    let endpoint = endpoint_label(&config.base_url);
    let workspace_id = config.workspace_id.clone();
    let peer_id = config.peer_id.clone();
    let token = config
        .token_env
        .as_deref()
        .map(settings::secret_value_for_env)
        .transpose()?
        .flatten()
        .filter(|value| {
            !value.is_empty() && value.trim() == value && !value.contains(['\r', '\n'])
        });
    let token_configured = token.is_some();

    if !config.enabled {
        return Ok(HonchoStatus {
            enabled: false,
            configured: false,
            reachable: false,
            state: "disabled",
            endpoint,
            workspace_id,
            peer_id,
            token_configured,
            detail: Some("Optional sidecar is disabled; normal ingestion is unchanged.".into()),
        });
    }
    if !token_configured {
        return Ok(HonchoStatus {
            enabled: true,
            configured: false,
            reachable: false,
            state: "configuration_required",
            endpoint,
            workspace_id,
            peer_id,
            token_configured: false,
            detail: Some("Configure the token environment name and write-only token.".into()),
        });
    }

    let url = parse_endpoint(&config.base_url)?;
    let client = Client::builder()
        .connect_timeout(HEALTH_TIMEOUT)
        .timeout(HEALTH_TIMEOUT)
        .build()
        .map_err(|error| format!("build Honcho health client: {error}"))?;
    let mut request = client.get(url);
    if let Some(token) = token.as_deref() {
        request = request.bearer_auth(token);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return Ok(HonchoStatus {
                enabled: true,
                configured: true,
                reachable: false,
                state: "unreachable",
                endpoint,
                workspace_id,
                peer_id,
                token_configured: true,
                detail: Some(sanitize_error(&error.to_string())),
            });
        }
    };
    let status_code = response.status().as_u16();
    let (state, detail) = if response.status().is_success() {
        (
            "healthy",
            format!("Honcho endpoint responded successfully with HTTP {status_code}."),
        )
    } else if response.status().is_client_error() || response.status().is_redirection() {
        (
            "reachable",
            format!("Honcho endpoint is reachable and returned HTTP {status_code}."),
        )
    } else {
        (
            "unhealthy",
            format!("Honcho endpoint returned HTTP {status_code}."),
        )
    };
    Ok(HonchoStatus {
        enabled: true,
        configured: true,
        reachable: true,
        state,
        endpoint,
        workspace_id,
        peer_id,
        token_configured: true,
        detail: Some(detail),
    })
}

fn parse_endpoint(base_url: &str) -> Result<Url, String> {
    let url = Url::parse(base_url).map_err(|_| "Honcho endpoint URL is invalid".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Honcho endpoint must be a credential-free HTTP(S) URL".into());
    }
    let is_loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    if url.scheme() == "http" && !is_loopback {
        return Err("Honcho remote endpoint must use HTTPS".into());
    }
    Ok(url)
}

fn endpoint_label(base_url: &str) -> String {
    parse_endpoint(base_url)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| "<invalid endpoint>".into())
}

fn sanitize_error(error: &str) -> String {
    let value = error
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.chars().count() > 512 {
        value.chars().take(512).collect()
    } else if value.is_empty() {
        "Honcho health request failed".into()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_validation_rejects_credentials_and_remote_http() {
        assert_eq!(
            endpoint_label("https://api.honcho.dev"),
            "https://api.honcho.dev/"
        );
        assert!(parse_endpoint("https://user:pass@example.test").is_err());
        assert!(parse_endpoint("https://example.test/?token=secret").is_err());
        assert!(parse_endpoint("http://example.test").is_err());
        assert!(parse_endpoint("file:///tmp/honcho").is_err());
    }

    #[test]
    fn errors_are_bounded_and_free_of_control_characters() {
        assert_eq!(
            sanitize_error("failed\nwith\rsecret\u{0000}"),
            "failed with secret"
        );
        assert_eq!(sanitize_error(&"x".repeat(600)).chars().count(), 512);
    }
}
