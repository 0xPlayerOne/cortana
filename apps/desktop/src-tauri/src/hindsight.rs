use std::time::Duration;

use reqwest::{Client, Url};
use serde::Serialize;

use crate::settings;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize)]
pub struct HindsightStatus {
    pub enabled: bool,
    pub configured: bool,
    pub reachable: bool,
    pub state: &'static str,
    pub endpoint: String,
    pub bank: String,
    pub token_configured: bool,
    pub detail: Option<String>,
}

pub async fn status() -> Result<HindsightStatus, String> {
    let snapshot = settings::load()?;
    let config = snapshot.hindsight;
    let endpoint = endpoint_label(&config.base_url);
    let bank = config.bank.clone();
    let token = config
        .token_env
        .as_deref()
        .map(settings::secret_value_for_env)
        .transpose()?
        .flatten()
        .filter(|value| !value.is_empty() && value.trim() == value && !value.contains(['\r', '\n']));
    let token_configured = token.is_some();

    if !config.enabled {
        return Ok(HindsightStatus {
            enabled: false,
            configured: false,
            reachable: false,
            state: "disabled",
            endpoint,
            bank,
            token_configured,
            detail: Some("Optional sidecar is disabled; normal ingestion is unchanged.".into()),
        });
    }
    if !token_configured {
        return Ok(HindsightStatus {
            enabled: true,
            configured: false,
            reachable: false,
            state: "configuration_required",
            endpoint,
            bank,
            token_configured: false,
            detail: Some("Configure the token environment name and write-only token.".into()),
        });
    }

    let health_url = health_url(&config.base_url)?;
    let client = Client::builder()
        .connect_timeout(HEALTH_TIMEOUT)
        .timeout(HEALTH_TIMEOUT)
        .build()
        .map_err(|error| format!("build Hindsight health client: {error}"))?;
    let mut request = client.get(health_url);
    if let Some(token) = token.as_deref() {
        request = request.bearer_auth(token);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return Ok(HindsightStatus {
                enabled: true,
                configured: true,
                reachable: false,
                state: "unreachable",
                endpoint,
                bank,
                token_configured: true,
                detail: Some(sanitize_error(&error.to_string())),
            });
        }
    };
    if response.status().is_success() {
        Ok(HindsightStatus {
            enabled: true,
            configured: true,
            reachable: true,
            state: "healthy",
            endpoint,
            bank,
            token_configured: true,
            detail: Some("Hindsight health endpoint responded successfully.".into()),
        })
    } else {
        Ok(HindsightStatus {
            enabled: true,
            configured: true,
            reachable: false,
            state: "unhealthy",
            endpoint,
            bank,
            token_configured: true,
            detail: Some(format!("Hindsight health endpoint returned HTTP {}.", response.status().as_u16())),
        })
    }
}

fn health_url(base_url: &str) -> Result<Url, String> {
    let mut url = parse_endpoint(base_url)?;
    let path = format!("{}/health", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}

fn parse_endpoint(base_url: &str) -> Result<Url, String> {
    let url = Url::parse(base_url).map_err(|_| "Hindsight endpoint URL is invalid".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Hindsight endpoint must be a credential-free HTTP(S) URL".into());
    }
    let is_loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    if url.scheme() == "http" && !is_loopback {
        return Err("Hindsight remote endpoint must use HTTPS".into());
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
        .map(|character| if character.is_control() { ' ' } else { character })
        .collect::<String>();
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.len() > 512 {
        value[..512].to_string()
    } else if value.is_empty() {
        "Hindsight health request failed".into()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_url_preserves_reverse_proxy_path_and_rejects_unsafe_urls() {
        assert_eq!(
            health_url("https://example.test/hindsight").expect("health URL").as_str(),
            "https://example.test/hindsight/health"
        );
        assert!(health_url("https://user:pass@example.test").is_err());
        assert!(health_url("https://example.test/?token=secret").is_err());
        assert!(health_url("http://example.test").is_err());
        assert!(health_url("file:///tmp/hindsight").is_err());
        assert_eq!(endpoint_label("https://user:pass@example.test"), "<invalid endpoint>");
    }

    #[test]
    fn errors_are_bounded_and_free_of_control_characters() {
        let error = sanitize_error("failed\nwith\rsecret\u{0000}");
        assert_eq!(error, "failed with secret");
        assert!(sanitize_error(&"x".repeat(600)).len() <= 512);
    }
}
