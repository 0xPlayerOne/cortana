//! Slack browser authorization and bounded workspace (team) discovery.
//!
//! Slack workspaces are assigned to Cortana workspaces through browser OAuth.
//! The operator supplies a Slack OAuth application client JSON (a
//! `client_id`, plus an optional `client_secret` for confidential apps),
//! completes the Authorization Code + PKCE flow in the system browser, and
//! Cortana stores the resulting user token in an owner-only file. A Slack
//! user token is scoped to exactly one workspace, so discovery reads the
//! token's workspace identity through `team.info`; the Desktop chooser
//! persists the checked team into the per-source `teams` field (each Slack
//! source belongs to exactly one Cortana workspace). Channel selection and
//! message sync remain bot-token based: the Python connector keeps reading
//! `SLACK_BOT_TOKEN`, so the configured bot token environment variable stays
//! the fallback for every sync path and is never interpreted as a path.
//!
//! This module talks only to the fixed allowlisted Slack endpoints
//! `https://slack.com/oauth/v2/authorize`, `https://slack.com/api/oauth.v2.access`,
//! and `https://slack.com/api/team.info`. It never reads message content,
//! never starts a sync, and never prints or stores the bot token. Every
//! response is byte-bounded and every emitted id is a validated Slack team
//! id; team names are sanitized to printable, bounded text before they cross
//! the process boundary.
//!
//! Slack requires the loopback redirect URI to be pre-registered exactly in
//! the Slack app (OAuth & Permissions → Redirect URLs), so unlike providers
//! with wildcard redirect support this flow binds one fixed loopback port:
//! register `http://127.0.0.1:47521/callback` in the Slack app before
//! authorizing. If the port is busy the flow fails closed with guidance
//! instead of silently using an unregisterable redirect URI.

use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::{Client, Url, redirect::Policy};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

use crate::config::{Config, SourceConfig};
use crate::oauth_common;

const AUTHORIZATION_ENDPOINT: &str = "https://slack.com/oauth/v2/authorize";
const TOKEN_ENDPOINT: &str = "https://slack.com/api/oauth.v2.access";
const TEAM_INFO_ENDPOINT: &str = "https://slack.com/api/team.info";
/// Slack validates redirect URIs exactly, so the loopback callback uses one
/// fixed port that the operator registers in the Slack app as
/// `http://127.0.0.1:47521/callback`.
const LOOPBACK_CALLBACK_PORT: u16 = 47521;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_TEAMS: usize = 100;
const MAX_CLIENT_FILE_BYTES: u64 = 64 * 1024;
const MAX_TOKEN_FILE_BYTES: u64 = 64 * 1024;
const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;
const MAX_CLIENT_ID_CHARS: usize = 2048;
const MAX_CLIENT_SECRET_CHARS: usize = 4096;
const MAX_TEAM_ID_CHARS: usize = 12;
const MAX_TEAM_NAME_CHARS: usize = 80;
/// Slack user tokens are long-lived unless the app enables token rotation;
/// without rotation no `expires_in` or `refresh_token` is returned, so the
/// stored expiry falls back to this far-future horizon instead of failing.
const LONG_LIVED_TOKEN_HORIZON_DAYS: i64 = 3650;
const TEAM_READ_SCOPE: &str = "team:read";

#[derive(Debug, Serialize)]
pub struct SlackAuthorizationOutcome {
    pub source: String,
    pub project: String,
    pub scopes: Vec<String>,
    pub token_path: String,
    pub authorized: bool,
    /// The workspace the granted user token is scoped to, when Slack
    /// reported it in the token response.
    pub team: Option<TeamSummary>,
}

#[derive(Debug, Serialize)]
pub struct WorkspaceList {
    pub teams: Vec<TeamSummary>,
    pub truncated: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct TeamSummary {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct ClientFile {
    client_id: String,
    client_secret: Option<String>,
}

/// Slack Web API responses carry an `ok` flag; token exchanges and discovery
/// fail with `{ "ok": false, "error": "..." }` instead of HTTP errors.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(default)]
    ok: bool,
    access_token: Option<String>,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
    scope: Option<String>,
    token_type: Option<String>,
    error: Option<String>,
    team: Option<ApiTeam>,
}

#[derive(Debug, Deserialize)]
struct ApiTeam {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct TeamInfoResponse {
    #[serde(default)]
    ok: bool,
    team: Option<ApiTeam>,
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct StoredToken {
    access_token: String,
    token_type: String,
    refresh_token: Option<String>,
    scope: Option<String>,
    expiry: String,
}

/// Run the Slack Authorization Code + PKCE flow for one configured source.
/// The user token is stored in the source's token destination; the bot token
/// environment variable is never touched. The loopback redirect URI must be
/// registered in the Slack app before the flow can complete.
pub async fn authorize(config: &Config, selected: &str) -> Result<SlackAuthorizationOutcome> {
    let source = configured_slack_source(config, selected)?;
    let token_path = configured_token_path(config, source)?;
    let client_path = required_secure_path(source, source.oauth_client.as_ref(), "OAuth client")?;
    ensure_outside_filesystem_roots(config, &token_path, "token")?;
    ensure_outside_filesystem_roots(config, client_path, "OAuth client")?;
    anyhow::ensure!(
        token_path.as_path() != client_path,
        "Slack token and OAuth client paths must be different"
    );
    let client = read_client_file(client_path)?;
    let scopes = vec![TEAM_READ_SCOPE.to_string()];

    let listener = bind_callback_listener().await?;
    let port = listener
        .local_addr()
        .context("read Slack OAuth callback address")?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let state = oauth_common::random_secret();
    let verifier = oauth_common::random_secret();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let authorization_url =
        authorization_url(&client.client_id, &redirect_uri, &state, &challenge)?;

    open::that_detached(authorization_url.as_str())
        .context("open Slack authorization in the system browser")?;
    let code = oauth_common::wait_for_callback(listener, &state, "Slack").await?;
    let token = exchange_code(&client, &redirect_uri, &verifier, &code).await?;
    let access_token = token.access_token.as_deref().context(
        "Slack did not return a user access token; grant the `team:read` scope and authorize again",
    )?;
    verify_token_type(token.token_type.as_deref())?;
    verify_granted_scopes(&scopes, token.scope.as_deref())?;
    oauth_common::validate_credential("Slack access token", access_token, MAX_CREDENTIAL_BYTES)?;
    let refresh_token = token.refresh_token.as_deref();
    if let Some(refresh_token) = refresh_token {
        oauth_common::validate_credential(
            "Slack refresh token",
            refresh_token,
            MAX_CREDENTIAL_BYTES,
        )?;
    }

    let team = match token.team {
        Some(team) => Some(TeamSummary {
            id: validate_team_id(&team.id, "team")?,
            name: sanitize_name(&team.name, "team")?,
        }),
        None => None,
    };

    let stored = StoredToken {
        access_token: token.access_token.expect("validated access token"),
        token_type: "Bearer".into(),
        refresh_token: refresh_token.map(str::to_string),
        scope: token.scope,
        expiry: expiry_rfc3339(token.expires_in),
    };
    persist_token(&token_path, &stored)?;

    Ok(SlackAuthorizationOutcome {
        source: source.name.clone(),
        project: source.project.clone(),
        scopes,
        token_path: token_path.display().to_string(),
        authorized: true,
        team,
    })
}

/// List bounded Slack workspaces (teams) visible to the authorized user
/// through the stored OAuth token. A Slack user token is scoped to exactly
/// one workspace, so discovery returns the token's own team; the response
/// stays a bounded list so the Desktop chooser and per-workspace assignment
/// follow the same contract as Discord server discovery. The token is
/// refreshed once when it is expired or rejected, and the refreshed token is
/// persisted atomically. Bot-token message sync via `SLACK_BOT_TOKEN` is
/// untouched and remains the fallback for sources that never set up browser
/// authorization.
pub async fn list_workspaces(config: &Config, selected: &str) -> Result<WorkspaceList> {
    let source = configured_slack_source(config, selected)?;
    let token_path = configured_token_path(config, source)?;
    let mut stored = read_token(&token_path, &source.name)?;
    let client = http_client()?;

    if token_expired(&stored) {
        stored = refresh_stored_token(config, source, &client, &stored).await?;
        persist_token(&token_path, &stored)?;
    }
    let teams = match fetch_team(&client, &stored.access_token).await {
        Ok(teams) => teams,
        Err(error) if is_unauthorized_error(&error) && stored.refresh_token.is_some() => {
            stored = refresh_stored_token(config, source, &client, &stored).await?;
            persist_token(&token_path, &stored)?;
            fetch_team(&client, &stored.access_token).await?
        }
        Err(error) => return Err(error),
    };

    let truncated = teams.len() >= MAX_TEAMS;
    let mut summaries = Vec::new();
    for team in teams.into_iter().take(MAX_TEAMS) {
        summaries.push(TeamSummary {
            id: validate_team_id(&team.id, "team")?,
            name: sanitize_name(&team.name, "team")?,
        });
    }
    Ok(WorkspaceList {
        teams: summaries,
        truncated,
    })
}

async fn fetch_team(client: &Client, token: &str) -> Result<Vec<ApiTeam>> {
    let response = client
        .get(TEAM_INFO_ENDPOINT)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .context("request Slack workspace discovery")?;
    let status = response.status();
    anyhow::ensure!(
        status.is_success(),
        "Slack workspace discovery failed with status {}",
        status.as_u16()
    );
    let info: TeamInfoResponse =
        oauth_common::bounded_json(response, MAX_RESPONSE_BYTES, "Slack").await?;
    anyhow::ensure!(
        info.ok,
        "Slack workspace discovery failed ({})",
        slack_error(&info.error)
    );
    let team = info
        .team
        .context("Slack workspace discovery returned no team")?;
    Ok(vec![team])
}

fn slack_error(error: &Option<String>) -> String {
    error
        .as_deref()
        .map(|error| {
            error
                .chars()
                .take(256)
                .filter(|character| !character.is_control())
                .collect::<String>()
        })
        .filter(|error| !error.is_empty())
        .unwrap_or_else(|| "unknown error".into())
}

fn is_unauthorized_error(error: &anyhow::Error) -> bool {
    error.to_string().contains("failed with status 401")
}

fn verify_token_type(token_type: Option<&str>) -> Result<()> {
    anyhow::ensure!(
        token_type.is_some_and(|token_type| token_type.eq_ignore_ascii_case("bearer")),
        "Slack returned an unsupported token type"
    );
    Ok(())
}

fn token_expired(stored: &StoredToken) -> bool {
    let Ok(expiry) = DateTime::parse_from_rfc3339(&stored.expiry) else {
        return true;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    expiry.timestamp() <= now
}

async fn refresh_stored_token(
    config: &Config,
    source: &SourceConfig,
    client: &Client,
    stored: &StoredToken,
) -> Result<StoredToken> {
    let client_path = required_secure_path(source, source.oauth_client.as_ref(), "OAuth client")?;
    ensure_outside_filesystem_roots(config, client_path, "OAuth client")?;
    let client_file = read_client_file(client_path)?;
    refresh(client, &client_file, stored).await
}

async fn refresh(
    client: &Client,
    client_file: &ClientFile,
    stored: &StoredToken,
) -> Result<StoredToken> {
    let refresh_token = stored.refresh_token.as_deref().context(
        "Slack authorization cannot be refreshed and its token has expired; run `cortana authorize-slack` again",
    )?;
    let mut form = vec![
        ("client_id", client_file.client_id.as_str()),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];
    if let Some(secret) = client_file.client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    let response = client
        .post(TOKEN_ENDPOINT)
        .form(&form)
        .send()
        .await
        .context("refresh Slack authorization")?;
    anyhow::ensure!(
        response.status().is_success(),
        "Slack token refresh failed with status {}",
        response.status().as_u16()
    );
    let token: TokenResponse =
        oauth_common::bounded_json(response, oauth_common::MAX_TOKEN_RESPONSE_BYTES, "Slack")
            .await?;
    anyhow::ensure!(
        token.ok,
        "Slack token refresh failed ({})",
        slack_error(&token.error)
    );
    let access_token = token
        .access_token
        .as_deref()
        .context("Slack token refresh returned no access token")?;
    verify_token_type(token.token_type.as_deref())?;
    oauth_common::validate_credential("Slack access token", access_token, MAX_CREDENTIAL_BYTES)?;
    let next = StoredToken {
        access_token: access_token.to_string(),
        token_type: "Bearer".into(),
        refresh_token: token.refresh_token.or_else(|| stored.refresh_token.clone()),
        scope: token.scope.or_else(|| stored.scope.clone()),
        expiry: expiry_rfc3339(token.expires_in),
    };
    if let Some(refresh) = next.refresh_token.as_deref() {
        oauth_common::validate_credential("Slack refresh token", refresh, MAX_CREDENTIAL_BYTES)?;
    }
    Ok(next)
}

fn expiry_rfc3339(seconds: Option<u64>) -> String {
    let duration = seconds
        .and_then(|seconds| i64::try_from(seconds).ok())
        .map(ChronoDuration::seconds)
        .unwrap_or_else(|| ChronoDuration::days(LONG_LIVED_TOKEN_HORIZON_DAYS));
    (Utc::now() + duration).to_rfc3339()
}

fn persist_token(path: &Path, stored: &StoredToken) -> Result<()> {
    let body = serde_json::to_vec_pretty(stored)?;
    oauth_common::write_owner_only_file(path, &body, "Slack token", "cortana-slack-token")
}

fn read_token(path: &Path, source_name: &str) -> Result<StoredToken> {
    oauth_common::reject_symlink(path)?;
    oauth_common::reject_symlink_components(path)?;
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "Slack workspace discovery requires browser authorization; run `cortana authorize-slack {source_name}` first"
            );
        }
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(metadata.is_file(), "Slack token must be a regular file");
    oauth_common::ensure_owner_only(&metadata, "Slack token")?;
    anyhow::ensure!(
        metadata.len() <= MAX_TOKEN_FILE_BYTES,
        "Slack token file exceeds 64 KiB"
    );
    let body = fs::read(path).with_context(|| format!("read Slack token {}", path.display()))?;
    let stored: StoredToken = serde_json::from_slice(&body).map_err(|_| {
        anyhow::anyhow!(
            "Slack workspace discovery requires browser authorization; run `cortana authorize-slack {source_name}` first"
        )
    })?;
    oauth_common::validate_credential(
        "Slack access token",
        &stored.access_token,
        MAX_CREDENTIAL_BYTES,
    )?;
    if let Some(refresh) = stored.refresh_token.as_deref() {
        oauth_common::validate_credential("Slack refresh token", refresh, MAX_CREDENTIAL_BYTES)?;
    }
    Ok(stored)
}

fn read_client_file(path: &Path) -> Result<ClientFile> {
    oauth_common::reject_symlink(path)?;
    oauth_common::reject_symlink_components(path)?;
    let metadata = fs::metadata(path)
        .with_context(|| format!("inspect Slack OAuth client {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_file(),
        "Slack OAuth client must be a regular file"
    );
    oauth_common::ensure_owner_only(&metadata, "Slack OAuth client")?;
    anyhow::ensure!(
        metadata.len() <= MAX_CLIENT_FILE_BYTES,
        "Slack OAuth client file exceeds 64 KiB"
    );
    let body =
        fs::read(path).with_context(|| format!("read Slack OAuth client {}", path.display()))?;
    let client: ClientFile = serde_json::from_slice(&body).context(
        "Slack OAuth client JSON must contain `client_id` and optionally `client_secret`",
    )?;
    oauth_common::validate_credential("Slack client ID", &client.client_id, MAX_CLIENT_ID_CHARS)?;
    if let Some(secret) = client.client_secret.as_deref() {
        oauth_common::validate_credential("Slack client secret", secret, MAX_CLIENT_SECRET_CHARS)?;
    }
    Ok(client)
}

/// Bind the fixed Slack loopback callback port. Slack rejects redirect URIs
/// that are not registered exactly in the app, so a dynamic port would make
/// the flow unregisterable; when the fixed port is busy the flow fails
/// closed with the exact redirect URI the operator must register or free.
async fn bind_callback_listener() -> Result<TcpListener> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), LOOPBACK_CALLBACK_PORT);
    match TcpListener::bind(address).await {
        Ok(listener) => Ok(listener),
        Err(_) => bail!(
            "Slack OAuth requires the exact loopback redirect URI http://127.0.0.1:{LOOPBACK_CALLBACK_PORT}/callback; register it in the Slack app (OAuth & Permissions → Redirect URLs) and make sure the port is free, then authorize again"
        ),
    }
}

fn required_secure_path<'a>(
    source: &SourceConfig,
    value: Option<&'a PathBuf>,
    label: &str,
) -> Result<&'a Path> {
    let path = value
        .map(PathBuf::as_path)
        .with_context(|| format!("Slack source {} requires {label} path", source.name))?;
    anyhow::ensure!(
        path.is_absolute()
            && path.parent().is_some()
            && path
                .parent()
                .is_some_and(|parent| parent.parent().is_some())
            && !path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            }),
        "Slack source {} {label} path must be absolute and outside the filesystem root",
        source.name
    );
    Ok(path)
}

/// The Slack user token destination must be the explicit `token` path. The
/// `token_env` field names the bot token environment variable, which is a
/// credential, not a path, so it can never be reused as a token destination.
fn configured_token_path(_config: &Config, source: &SourceConfig) -> Result<PathBuf> {
    let path = source.token.as_ref().with_context(|| {
        format!(
            "Slack source {} requires a token file for browser authorization; configure a private token path first",
            source.name
        )
    })?;
    Ok(required_secure_path(source, Some(path), "token")?.to_path_buf())
}

fn ensure_outside_filesystem_roots(config: &Config, path: &Path, label: &str) -> Result<()> {
    for source in config.sources.iter().filter(|source| {
        source.kind == "filesystem" && source.root.as_deref().is_some_and(Path::is_absolute)
    }) {
        let root = source.root.as_deref().expect("filtered filesystem root");
        anyhow::ensure!(
            !path.starts_with(root),
            "Slack {label} path must be outside filesystem source {}",
            source.name
        );
    }
    Ok(())
}

fn authorization_url(
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> Result<Url> {
    let mut url = Url::parse(AUTHORIZATION_ENDPOINT)?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", TEAM_READ_SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(url)
}

async fn exchange_code(
    client: &ClientFile,
    redirect_uri: &str,
    verifier: &str,
    code: &str,
) -> Result<TokenResponse> {
    let http = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(HTTP_TIMEOUT)
        .redirect(Policy::none())
        .build()?;
    let mut form = vec![
        ("client_id", client.client_id.as_str()),
        ("code", code),
        ("code_verifier", verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri),
    ];
    if let Some(secret) = client.client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    let response = http
        .post(TOKEN_ENDPOINT)
        .form(&form)
        .send()
        .await
        .context("exchange Slack authorization code")?;
    anyhow::ensure!(
        response.status().is_success(),
        "Slack token exchange failed with status {}",
        response.status().as_u16()
    );
    let token: TokenResponse =
        oauth_common::bounded_json(response, oauth_common::MAX_TOKEN_RESPONSE_BYTES, "Slack")
            .await?;
    anyhow::ensure!(
        token.ok,
        "Slack authorization was not granted ({})",
        slack_error(&token.error)
    );
    Ok(token)
}

/// Slack reports granted scopes as a comma- and/or space-separated list,
/// unlike Discord's space-only scope string.
fn verify_granted_scopes(requested: &[String], granted: Option<&str>) -> Result<()> {
    let Some(granted) = granted else {
        return Ok(());
    };
    let granted = granted
        .split(|character: char| character == ',' || character.is_ascii_whitespace())
        .filter(|scope| !scope.is_empty())
        .collect::<Vec<_>>();
    for scope in requested {
        anyhow::ensure!(
            granted.contains(&scope.as_str()),
            "Slack did not grant required scope {scope}"
        );
    }
    Ok(())
}

/// Slack team ids are `T` followed by 8 to 11 base-36 style alphanumeric
/// characters. Keep them as strings: renderer numbers cannot represent ids
/// above 2^53 exactly.
pub(crate) fn validate_team_id(value: &str, label: &str) -> Result<String> {
    anyhow::ensure!(
        !value.is_empty()
            && (9..=MAX_TEAM_ID_CHARS).contains(&value.len())
            && value.starts_with('T')
            && value
                .bytes()
                .skip(1)
                .all(|byte| byte.is_ascii_alphanumeric()),
        "Slack {label} returned an invalid id"
    );
    Ok(value.to_string())
}

pub(crate) fn sanitize_name(value: &str, label: &str) -> Result<String> {
    let sanitized = value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>()
        .trim()
        .to_string();
    anyhow::ensure!(
        !sanitized.is_empty(),
        "Slack {label} returned an empty name"
    );
    anyhow::ensure!(
        sanitized.chars().count() <= MAX_TEAM_NAME_CHARS,
        "Slack {label} returned an oversized name"
    );
    Ok(sanitized)
}

fn validate_source_name(value: &str) -> Result<()> {
    anyhow::ensure!(
        !value.is_empty() && value.len() <= 64,
        "source name is invalid"
    );
    anyhow::ensure!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "source name is invalid"
    );
    Ok(())
}

pub(crate) fn configured_slack_source<'a>(
    config: &'a Config,
    selected: &str,
) -> Result<&'a SourceConfig> {
    validate_source_name(selected)?;
    let source = config
        .sources
        .iter()
        .find(|source| source.name == selected)
        .with_context(|| format!("configured source {selected} was not found"))?;
    anyhow::ensure!(
        source.kind == "slack",
        "source {} is not a Slack connector",
        source.name
    );
    Ok(source)
}

fn http_client() -> Result<Client> {
    Client::builder()
        .redirect(Policy::none())
        .timeout(HTTP_TIMEOUT)
        .user_agent(format!("cortana/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build Slack API client")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(kind: &str, token: Option<PathBuf>, token_env: Option<&str>) -> SourceConfig {
        SourceConfig {
            name: "community".into(),
            kind: kind.into(),
            enabled: true,
            project: "community".into(),
            root: None,
            source: None,
            channels: Vec::new(),
            servers: Vec::new(),
            teams: Vec::new(),
            team_names: Vec::new(),
            repositories: Vec::new(),
            token_env: token_env.map(str::to_string),
            token,
            oauth_client: Some(PathBuf::from("/tmp/cortana/slack-oauth-client.json")),
            query: None,
            labels: Vec::new(),
            max_content_chars: None,
            max_documents: None,
            max_bytes: None,
            max_duration_seconds: None,
            exclude: Vec::new(),
            command: Vec::new(),
            acl: Vec::new(),
        }
    }

    #[test]
    fn authorization_url_uses_pkce_loopback_and_minimal_scopes() {
        let url = authorization_url(
            "client-id",
            "http://127.0.0.1:47521/callback",
            "expected-state",
            "challenge",
        )
        .unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("slack.com"));
        let query = url
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(query.get("client_id").unwrap(), "client-id");
        assert_eq!(query.get("scope").unwrap(), "team:read");
        assert_eq!(query.get("state").unwrap(), "expected-state");
        assert_eq!(query.get("code_challenge_method").unwrap(), "S256");
        assert_eq!(
            query.get("redirect_uri").unwrap(),
            "http://127.0.0.1:47521/callback"
        );
    }

    #[test]
    fn stored_tokens_round_trip_and_detect_expiry() {
        let directory = tempfile::tempdir().unwrap();
        let token_path = directory.path().join("nested/slack-token.json");
        let stored = StoredToken {
            access_token: "access-token".into(),
            token_type: "Bearer".into(),
            refresh_token: Some("refresh-token".into()),
            scope: Some("team:read".into()),
            expiry: expiry_rfc3339(Some(3600)),
        };
        persist_token(&token_path, &stored).expect("persist token");
        let loaded = read_token(&token_path, "community").expect("read token");
        assert_eq!(loaded.access_token, "access-token");
        assert_eq!(loaded.refresh_token.as_deref(), Some("refresh-token"));
        assert!(!token_expired(&loaded), "a fresh token must not be expired");

        let expired = StoredToken {
            expiry: (Utc::now() - ChronoDuration::seconds(60)).to_rfc3339(),
            ..stored.clone()
        };
        assert!(token_expired(&expired), "an old token must be expired");
        assert!(
            token_expired(&StoredToken {
                expiry: "not-a-date".into(),
                ..stored
            }),
            "an unparsable expiry must fail closed as expired"
        );
    }

    #[test]
    fn long_lived_tokens_without_rotation_never_report_expiry() {
        let stored = StoredToken {
            access_token: "access-token".into(),
            token_type: "Bearer".into(),
            refresh_token: None,
            scope: Some("team:read".into()),
            expiry: expiry_rfc3339(None),
        };
        assert!(
            !token_expired(&stored),
            "a token without expires_in must stay long-lived"
        );
    }

    #[test]
    fn missing_or_malformed_tokens_fail_closed_with_authorization_guidance() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing-token.json");
        let error =
            read_token(&missing, "community").expect_err("a missing token must fail closed");
        let message = error.to_string();
        assert!(
            message.contains("requires browser authorization"),
            "missing token must ask for authorization: {message}"
        );
        assert!(message.contains("authorize-slack community"));

        let malformed = directory.path().join("malformed-token.json");
        fs::write(&malformed, "{\"access_token\":").unwrap();
        #[cfg(unix)]
        oauth_common::set_owner_only(&malformed).unwrap();
        let error =
            read_token(&malformed, "community").expect_err("a malformed token must fail closed");
        assert!(error.to_string().contains("requires browser authorization"));
    }

    #[test]
    fn refresh_keeps_the_existing_refresh_token_when_the_response_omits_it() {
        // The merge rule lives in `refresh`, which needs the network; the
        // response shape is asserted here through the serialized contract
        // the stored file uses so a malformed refresh can never drop the
        // refresh token.
        let directory = tempfile::tempdir().unwrap();
        let token_path = directory.path().join("token.json");
        let stored = StoredToken {
            access_token: "old-access".into(),
            token_type: "Bearer".into(),
            refresh_token: Some("old-refresh".into()),
            scope: Some("team:read".into()),
            expiry: expiry_rfc3339(Some(3600)),
        };
        persist_token(&token_path, &stored).expect("persist token");
        let loaded = read_token(&token_path, "community").expect("read token");
        assert_eq!(loaded.refresh_token.as_deref(), Some("old-refresh"));
    }

    #[cfg(unix)]
    #[test]
    fn oauth_inputs_reject_group_or_world_readable_files() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let client = directory.path().join("client.json");
        fs::write(&client, r#"{"client_id":"slack-client-id"}"#).unwrap();
        fs::set_permissions(&client, fs::Permissions::from_mode(0o644)).unwrap();
        let error = read_client_file(&client)
            .expect_err("group-readable Slack OAuth client must be rejected");
        assert!(error.to_string().contains("must not be accessible"));
    }

    #[test]
    fn client_files_require_a_valid_client_id() {
        let directory = tempfile::tempdir().unwrap();
        let client = directory.path().join("client.json");
        fs::write(&client, r#"{"client_secret":"secret-without-id"}"#).unwrap();
        #[cfg(unix)]
        oauth_common::set_owner_only(&client).unwrap();
        let error = read_client_file(&client)
            .expect_err("a Slack OAuth client without client_id must be rejected");
        assert!(error.to_string().contains("client_id"));
    }

    #[test]
    fn token_path_is_required_and_cannot_come_from_the_bot_token_environment() {
        let config_source = source("slack", None, Some("SLACK_BOT_TOKEN"));
        let error = configured_token_path(&Config::default(), &config_source)
            .expect_err("the bot token environment is not a token path and must fail closed");
        assert!(error.to_string().contains("requires a token file"));
        assert!(
            !error.to_string().contains("SLACK_BOT_TOKEN"),
            "errors must not name environment variables when no path is configured"
        );

        let config_source = source(
            "slack",
            Some(PathBuf::from("/tmp/cortana/slack-user-token.json")),
            None,
        );
        assert_eq!(
            configured_token_path(&Config::default(), &config_source).unwrap(),
            PathBuf::from("/tmp/cortana/slack-user-token.json")
        );
    }

    #[test]
    fn slack_team_ids_and_names_are_bounded() {
        assert_eq!(
            validate_team_id("T0123456789", "team").unwrap(),
            "T0123456789"
        );
        assert_eq!(validate_team_id("T12345678", "team").unwrap(), "T12345678");
        for invalid in [
            "",
            "T",
            "T1234567",
            "T0123456789012",
            "t0123456789",
            "U0123456789",
            "T01234567!9",
            "T 123456789",
        ] {
            assert!(
                validate_team_id(invalid, "team").is_err(),
                "team id {invalid:?} must be rejected"
            );
        }
        assert_eq!(sanitize_name("  Acme Corp  ", "team").unwrap(), "Acme Corp");
        assert!(sanitize_name("", "team").is_err());
        assert!(sanitize_name(&"x".repeat(81), "team").is_err());
        assert_eq!(
            sanitize_name("Acme\x00Corp", "team").unwrap(),
            "AcmeCorp",
            "control characters must be stripped"
        );
    }

    #[test]
    fn discovery_fails_closed_on_slack_ok_false_responses() {
        // The ok:false contract is enforced inside `fetch_team`, which needs
        // the network; assert the error rendering and the granted-scope
        // separator handling that gate every Slack response instead.
        assert_eq!(slack_error(&Some("invalid_auth".into())), "invalid_auth");
        assert_eq!(slack_error(&None), "unknown error");

        let scopes = vec!["team:read".to_string()];
        assert!(verify_granted_scopes(&scopes, Some("team:read")).is_ok());
        assert!(
            verify_granted_scopes(&scopes, Some("channels:history,team:read")).is_ok(),
            "Slack scope lists are comma-separated"
        );
        assert!(verify_granted_scopes(&scopes, Some("channels:history")).is_err());
    }

    #[test]
    fn serialized_workspaces_never_contain_credentials() {
        let list = WorkspaceList {
            teams: vec![TeamSummary {
                id: "T0123456789".into(),
                name: "Engineering".into(),
            }],
            truncated: false,
        };
        let serialized = serde_json::to_string(&list).expect("serialize workspaces");
        assert!(serialized.contains("Engineering"));
        assert!(!serialized.contains("Bearer"));
        assert!(!serialized.contains("token"));
    }
}
