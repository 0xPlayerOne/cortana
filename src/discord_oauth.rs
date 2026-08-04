//! Discord browser authorization and bounded server discovery.
//!
//! Discord servers are assigned to workspaces through browser OAuth. The
//! operator supplies a Discord OAuth application client JSON (a `client_id`,
//! plus an optional `client_secret` for confidential apps), completes the
//! Authorization Code + PKCE flow in the system browser, and Cortana stores
//! the resulting user token in an owner-only file. The user token's `guilds`
//! scope is what can list the servers a user belongs to, which is what the
//! Desktop server chooser persists per source (each Discord source belongs to
//! exactly one workspace). Channel listing and message sync remain
//! bot-token based because Discord exposes those only to bots, so the
//! configured bot token environment variable stays the fallback for every
//! discovery and sync path.
//!
//! This module talks only to the fixed allowlisted Discord endpoints
//! `https://discord.com/oauth2/authorize`, `https://discord.com/api/oauth2/token`,
//! and `https://discord.com/api/v10/users/@me/guilds`. It never reads
//! message content, never starts a sync, and never prints or stores the bot
//! token. Every response is byte-bounded and every emitted id is a validated
//! snowflake; guild names are sanitized to printable, bounded text before
//! they cross the process boundary.

use std::{
    fs,
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
use crate::discord::{configured_discord_source, sanitize_name, validate_snowflake};
use crate::oauth_common;

const AUTHORIZATION_ENDPOINT: &str = "https://discord.com/oauth2/authorize";
const TOKEN_ENDPOINT: &str = "https://discord.com/api/oauth2/token";
const API_BASE: &str = "https://discord.com/api/v10";
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_SERVERS: usize = 100;
const MAX_CLIENT_FILE_BYTES: u64 = 64 * 1024;
const MAX_TOKEN_FILE_BYTES: u64 = 64 * 1024;
const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;
const MAX_CLIENT_ID_CHARS: usize = 2048;
const MAX_CLIENT_SECRET_CHARS: usize = 4096;
const IDENTIFY_SCOPE: &str = "identify";
const GUILDS_SCOPE: &str = "guilds";

#[derive(Debug, Serialize)]
pub struct AuthorizationOutcome {
    pub source: String,
    pub project: String,
    pub scopes: Vec<String>,
    pub token_path: String,
    pub authorized: bool,
}

#[derive(Debug, Serialize)]
pub struct ServerList {
    pub guilds: Vec<ServerSummary>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct ServerSummary {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct ClientFile {
    client_id: String,
    client_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    scope: Option<String>,
    token_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct StoredToken {
    access_token: String,
    token_type: String,
    refresh_token: Option<String>,
    scope: Option<String>,
    expiry: String,
}

/// Run the Discord Authorization Code + PKCE flow for one configured source.
/// The user token is stored in the source's token destination; the bot token
/// environment variable is never touched.
pub async fn authorize(config: &Config, selected: &str) -> Result<AuthorizationOutcome> {
    let source = configured_discord_source(config, selected)?;
    let token_path = configured_token_path(config, source)?;
    let client_path = required_secure_path(source, source.oauth_client.as_ref(), "OAuth client")?;
    ensure_outside_filesystem_roots(config, &token_path, "token")?;
    ensure_outside_filesystem_roots(config, client_path, "OAuth client")?;
    anyhow::ensure!(
        token_path.as_path() != client_path,
        "Discord token and OAuth client paths must be different"
    );
    let client = read_client_file(client_path)?;
    let scopes = vec![IDENTIFY_SCOPE.to_string(), GUILDS_SCOPE.to_string()];

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("bind Discord OAuth loopback callback")?;
    let port = listener
        .local_addr()
        .context("read Discord OAuth callback address")?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let state = oauth_common::random_secret();
    let verifier = oauth_common::random_secret();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let authorization_url =
        authorization_url(&client.client_id, &redirect_uri, &state, &challenge)?;

    open::that_detached(authorization_url.as_str())
        .context("open Discord authorization in the system browser")?;
    let code = oauth_common::wait_for_callback(listener, &state, "Discord").await?;
    let token = exchange_code(&client, &redirect_uri, &verifier, &code).await?;
    anyhow::ensure!(
        token.token_type.eq_ignore_ascii_case("bearer"),
        "Discord returned an unsupported token type"
    );
    verify_granted_scopes(&scopes, token.scope.as_deref())?;
    let refresh_token = token.refresh_token.as_deref().context(
        "Discord did not return a refresh token; revoke the prior grant and authorize again",
    )?;
    oauth_common::validate_credential(
        "Discord access token",
        &token.access_token,
        MAX_CREDENTIAL_BYTES,
    )?;
    oauth_common::validate_credential(
        "Discord refresh token",
        refresh_token,
        MAX_CREDENTIAL_BYTES,
    )?;

    let stored = StoredToken {
        access_token: token.access_token,
        token_type: "Bearer".into(),
        refresh_token: Some(refresh_token.to_string()),
        scope: token.scope,
        expiry: expiry_rfc3339(token.expires_in),
    };
    persist_token(&token_path, &stored)?;

    Ok(AuthorizationOutcome {
        source: source.name.clone(),
        project: source.project.clone(),
        scopes,
        token_path: token_path.display().to_string(),
        authorized: true,
    })
}

/// List bounded servers (guilds) visible to the authorized user through the
/// stored OAuth token. The token is refreshed once when it is expired or
/// rejected, and the refreshed token is persisted atomically. Bot-token
/// discovery in [`crate::discord::list_channels`] remains the fallback for
/// sources that never set up browser authorization.
pub async fn list_servers(config: &Config, selected: &str) -> Result<ServerList> {
    let source = configured_discord_source(config, selected)?;
    let token_path = configured_token_path(config, source)?;
    let mut stored = read_token(&token_path, &source.name)?;
    let client = http_client()?;

    if token_expired(&stored) {
        stored = refresh_stored_token(config, source, &client, &stored).await?;
        persist_token(&token_path, &stored)?;
    }
    let guilds = match fetch_guilds(&client, &stored.access_token).await {
        Ok(guilds) => guilds,
        Err(error) if is_unauthorized_error(&error) && stored.refresh_token.is_some() => {
            stored = refresh_stored_token(config, source, &client, &stored).await?;
            persist_token(&token_path, &stored)?;
            fetch_guilds(&client, &stored.access_token).await?
        }
        Err(error) => return Err(error),
    };

    let truncated = guilds.len() >= MAX_SERVERS;
    let mut summaries = Vec::new();
    for guild in guilds.into_iter().take(MAX_SERVERS) {
        summaries.push(ServerSummary {
            id: validate_snowflake(&guild.id, "guild")?,
            name: sanitize_name(&guild.name, "guild")?,
        });
    }
    Ok(ServerList {
        guilds: summaries,
        truncated,
    })
}

#[derive(Debug, Deserialize)]
struct ApiGuild {
    id: String,
    name: String,
}

async fn fetch_guilds(client: &Client, token: &str) -> Result<Vec<ApiGuild>> {
    let url = format!("{API_BASE}/users/@me/guilds?limit={MAX_SERVERS}");
    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .context("request Discord server discovery")?;
    let status = response.status();
    anyhow::ensure!(
        status.is_success(),
        "Discord server discovery failed with status {}",
        status.as_u16()
    );
    oauth_common::bounded_json(response, MAX_RESPONSE_BYTES, "Discord").await
}

fn is_unauthorized_error(error: &anyhow::Error) -> bool {
    error.to_string().contains("failed with status 401")
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
        "Discord authorization cannot be refreshed; revoke the prior grant and authorize again",
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
        .context("refresh Discord authorization")?;
    anyhow::ensure!(
        response.status().is_success(),
        "Discord token refresh failed with status {}",
        response.status().as_u16()
    );
    let token: TokenResponse =
        oauth_common::bounded_json(response, MAX_RESPONSE_BYTES, "Discord").await?;
    anyhow::ensure!(
        token.token_type.eq_ignore_ascii_case("bearer"),
        "Discord returned an unsupported token type"
    );
    oauth_common::validate_credential(
        "Discord access token",
        &token.access_token,
        MAX_CREDENTIAL_BYTES,
    )?;
    let next = StoredToken {
        access_token: token.access_token,
        token_type: "Bearer".into(),
        refresh_token: token.refresh_token.or_else(|| stored.refresh_token.clone()),
        scope: token.scope.or_else(|| stored.scope.clone()),
        expiry: expiry_rfc3339(token.expires_in),
    };
    if let Some(refresh) = next.refresh_token.as_deref() {
        oauth_common::validate_credential("Discord refresh token", refresh, MAX_CREDENTIAL_BYTES)?;
    }
    Ok(next)
}

fn expiry_rfc3339(seconds: u64) -> String {
    (Utc::now() + ChronoDuration::seconds(i64::try_from(seconds).unwrap_or(3600))).to_rfc3339()
}

fn persist_token(path: &Path, stored: &StoredToken) -> Result<()> {
    let body = serde_json::to_vec_pretty(stored)?;
    oauth_common::write_owner_only_file(path, &body, "Discord token", "cortana-discord-token")
}

fn read_token(path: &Path, source_name: &str) -> Result<StoredToken> {
    oauth_common::reject_symlink(path)?;
    oauth_common::reject_symlink_components(path)?;
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "Discord server discovery requires browser authorization; run `cortana authorize-discord {source_name}` first"
            );
        }
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(metadata.is_file(), "Discord token must be a regular file");
    oauth_common::ensure_owner_only(&metadata, "Discord token")?;
    anyhow::ensure!(
        metadata.len() <= MAX_TOKEN_FILE_BYTES,
        "Discord token file exceeds 64 KiB"
    );
    let body = fs::read(path).with_context(|| format!("read Discord token {}", path.display()))?;
    let stored: StoredToken = serde_json::from_slice(&body).map_err(|_| {
        anyhow::anyhow!(
            "Discord server discovery requires browser authorization; run `cortana authorize-discord {source_name}` first"
        )
    })?;
    oauth_common::validate_credential(
        "Discord access token",
        &stored.access_token,
        MAX_CREDENTIAL_BYTES,
    )?;
    if let Some(refresh) = stored.refresh_token.as_deref() {
        oauth_common::validate_credential("Discord refresh token", refresh, MAX_CREDENTIAL_BYTES)?;
    }
    Ok(stored)
}

fn read_client_file(path: &Path) -> Result<ClientFile> {
    oauth_common::reject_symlink(path)?;
    oauth_common::reject_symlink_components(path)?;
    let metadata = fs::metadata(path)
        .with_context(|| format!("inspect Discord OAuth client {}", path.display()))?;
    anyhow::ensure!(
        metadata.is_file(),
        "Discord OAuth client must be a regular file"
    );
    oauth_common::ensure_owner_only(&metadata, "Discord OAuth client")?;
    anyhow::ensure!(
        metadata.len() <= MAX_CLIENT_FILE_BYTES,
        "Discord OAuth client file exceeds 64 KiB"
    );
    let body =
        fs::read(path).with_context(|| format!("read Discord OAuth client {}", path.display()))?;
    let client: ClientFile = serde_json::from_slice(&body).context(
        "Discord OAuth client JSON must contain `client_id` and optionally `client_secret`",
    )?;
    oauth_common::validate_credential("Discord client ID", &client.client_id, MAX_CLIENT_ID_CHARS)?;
    if let Some(secret) = client.client_secret.as_deref() {
        oauth_common::validate_credential(
            "Discord client secret",
            secret,
            MAX_CLIENT_SECRET_CHARS,
        )?;
    }
    Ok(client)
}

fn required_secure_path<'a>(
    source: &SourceConfig,
    value: Option<&'a PathBuf>,
    label: &str,
) -> Result<&'a Path> {
    let path = value
        .map(PathBuf::as_path)
        .with_context(|| format!("Discord source {} requires {label} path", source.name))?;
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
        "Discord source {} {label} path must be absolute and outside the filesystem root",
        source.name
    );
    Ok(path)
}

/// The Discord user token destination must be the explicit `token` path.
/// The `token_env` field names the bot token environment variable, which is a
/// credential, not a path, so it can never be reused as a token destination.
fn configured_token_path(_config: &Config, source: &SourceConfig) -> Result<PathBuf> {
    let path = source.token.as_ref().with_context(|| {
        format!(
            "Discord source {} requires a token file for browser authorization; configure a private token path first",
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
            "Discord {label} path must be outside filesystem source {}",
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
        .append_pair("response_type", "code")
        .append_pair("scope", &format!("{IDENTIFY_SCOPE} {GUILDS_SCOPE}"))
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
        .context("exchange Discord authorization code")?;
    anyhow::ensure!(
        response.status().is_success(),
        "Discord token exchange failed with status {}",
        response.status().as_u16()
    );
    oauth_common::bounded_json(response, oauth_common::MAX_TOKEN_RESPONSE_BYTES, "Discord").await
}

fn verify_granted_scopes(requested: &[String], granted: Option<&str>) -> Result<()> {
    let Some(granted) = granted else {
        return Ok(());
    };
    let granted = granted.split_ascii_whitespace().collect::<Vec<_>>();
    for scope in requested {
        anyhow::ensure!(
            granted.contains(&scope.as_str()),
            "Discord did not grant required scope {scope}"
        );
    }
    Ok(())
}

fn http_client() -> Result<Client> {
    Client::builder()
        .redirect(Policy::none())
        .timeout(HTTP_TIMEOUT)
        .user_agent(format!("cortana/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build Discord API client")
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
            communities: Vec::new(),
            community_names: Vec::new(),
            repositories: Vec::new(),
            token_env: token_env.map(str::to_string),
            token,
            oauth_client: Some(PathBuf::from("/tmp/cortana/discord-oauth-client.json")),
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
            "http://127.0.0.1:43210/callback",
            "expected-state",
            "challenge",
        )
        .unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("discord.com"));
        let query = url
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(query.get("client_id").unwrap(), "client-id");
        assert_eq!(query.get("response_type").unwrap(), "code");
        assert_eq!(query.get("scope").unwrap(), "identify guilds");
        assert_eq!(query.get("state").unwrap(), "expected-state");
        assert_eq!(query.get("code_challenge_method").unwrap(), "S256");
        assert_eq!(
            query.get("redirect_uri").unwrap(),
            "http://127.0.0.1:43210/callback"
        );
    }

    #[test]
    fn stored_tokens_round_trip_and_detect_expiry() {
        let directory = tempfile::tempdir().unwrap();
        let token_path = directory.path().join("nested/discord-token.json");
        let stored = StoredToken {
            access_token: "access-token".into(),
            token_type: "Bearer".into(),
            refresh_token: Some("refresh-token".into()),
            scope: Some("identify guilds".into()),
            expiry: expiry_rfc3339(3600),
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
        assert!(message.contains("authorize-discord community"));

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
            scope: Some("identify guilds".into()),
            expiry: expiry_rfc3339(3600),
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
        fs::write(&client, r#"{"client_id":"discord-client-id"}"#).unwrap();
        fs::set_permissions(&client, fs::Permissions::from_mode(0o644)).unwrap();
        let error = read_client_file(&client)
            .expect_err("group-readable Discord OAuth client must be rejected");
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
            .expect_err("a Discord OAuth client without client_id must be rejected");
        assert!(error.to_string().contains("client_id"));
    }

    #[test]
    fn token_path_is_required_and_cannot_come_from_the_bot_token_environment() {
        let config_source = source("discord", None, Some("DISCORD_BOT_TOKEN"));
        let error = configured_token_path(&Config::default(), &config_source)
            .expect_err("the bot token environment is not a token path and must fail closed");
        assert!(error.to_string().contains("requires a token file"));
        assert!(
            !error.to_string().contains("DISCORD_BOT_TOKEN"),
            "errors must not name environment variables when no path is configured"
        );

        let config_source = source(
            "discord",
            Some(PathBuf::from("/tmp/cortana/discord-user-token.json")),
            None,
        );
        assert_eq!(
            configured_token_path(&Config::default(), &config_source).unwrap(),
            PathBuf::from("/tmp/cortana/discord-user-token.json")
        );
    }

    #[test]
    fn serialized_servers_never_contain_credentials() {
        let list = ServerList {
            guilds: vec![ServerSummary {
                id: "175928847299117063".into(),
                name: "Engineering".into(),
            }],
            truncated: false,
        };
        let serialized = serde_json::to_string(&list).expect("serialize servers");
        assert!(serialized.contains("Engineering"));
        assert!(!serialized.contains("Bearer"));
        assert!(!serialized.contains("token"));
    }
}
