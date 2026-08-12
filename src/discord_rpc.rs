//! Discord Desktop RPC authorization and read-only message discovery.
//!
//! Cortana intentionally uses Discord’s supported local RPC surface instead of
//! scraping a signed-in account through private REST endpoints. The
//! operator authorizes Cortana in the running Discord desktop client with the
//! `rpc`, `identify`, and `messages.read` scopes. RPC exposes only the guilds,
//! channels, and messages the signed-in client can access.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::{Client, redirect::Policy};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{
    config::{Config, SourceConfig},
    oauth_common,
};

const RPC_VERSION: u32 = 1;
const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_CLIENT_FILE_BYTES: u64 = 64 * 1024;
const MAX_TOKEN_FILE_BYTES: u64 = 64 * 1024;
const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;
const MAX_NAME_CHARS: usize = 100;
const MAX_SNOWFLAKE_CHARS: usize = 20;
const MAX_GUILDS: usize = 100;
const MAX_CHANNELS_PER_GUILD: usize = 100;
const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const TOKEN_ENDPOINT: &str = "https://discord.com/api/oauth2/token";
/// Discord requires the redirect URI used by the token exchange to match one
/// registered on the application. The desktop RPC flow returns the code over
/// the local socket, so no listener is needed for this loopback placeholder.
/// Register this exact URI in the application's OAuth2 settings.
const REDIRECT_URI: &str = "http://127.0.0.1/callback";
const IDENTIFY_SCOPE: &str = "identify";
const RPC_SCOPE: &str = "rpc";
const MESSAGES_READ_SCOPE: &str = "messages.read";

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

#[derive(Debug, Serialize)]
pub struct ChannelList {
    pub guilds: Vec<GuildChannels>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct GuildChannels {
    pub id: String,
    pub name: String,
    pub channels: Vec<ChannelSummary>,
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct ChannelSummary {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Deserialize)]
struct ClientFile {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default = "default_token_type")]
    token_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredToken {
    access_token: String,
    token_type: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    expiry: String,
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    #[serde(default)]
    cmd: Option<String>,
    #[serde(default)]
    evt: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct RpcGuild {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct RpcChannel {
    id: String,
    name: String,
    #[serde(rename = "type")]
    channel_type: u64,
}

/// Authorize the configured source through Discord Desktop RPC. No browser
/// redirect or pasted credential is involved; Discord displays its own consent
/// prompt in the running desktop client.
pub async fn authorize(config: &Config, selected: &str) -> Result<AuthorizationOutcome> {
    let source = configured_source(config, selected)?;
    let token_path = configured_token_path(source)?;
    let client_path = required_secure_path(source, source.oauth_client.as_ref(), "OAuth client")?;
    ensure_outside_filesystem_roots(config, &token_path, "token")?;
    ensure_outside_filesystem_roots(config, client_path, "OAuth client")?;
    anyhow::ensure!(
        token_path != client_path,
        "Discord token and OAuth client paths must differ"
    );
    let client = read_client_file(client_path)?;

    let mut rpc = RpcClient::connect(&client.client_id).await?;
    let scopes = vec![
        RPC_SCOPE.to_string(),
        IDENTIFY_SCOPE.to_string(),
        MESSAGES_READ_SCOPE.to_string(),
    ];
    let authorize = rpc
        .command(
            "AUTHORIZE",
            serde_json::json!({
                "client_id": client.client_id,
                "scopes": scopes,
            }),
        )
        .await?;
    let code = authorize
        .data
        .get("code")
        .and_then(serde_json::Value::as_str)
        .context("Discord RPC authorization did not return a code")?;
    let token = exchange_code(&client, code).await?;
    persist_token(&token_path, &token)?;

    Ok(AuthorizationOutcome {
        source: source.name.clone(),
        project: source.project.clone(),
        scopes,
        token_path: token_path.display().to_string(),
        authorized: true,
    })
}

/// List guilds visible to the authorized Discord desktop client.
pub async fn list_servers(config: &Config, selected: &str) -> Result<ServerList> {
    let source = configured_source(config, selected)?;
    let token_path = configured_token_path(source)?;
    let client = read_client_file(required_secure_path(
        source,
        source.oauth_client.as_ref(),
        "OAuth client",
    )?)?;
    let token = read_authorized_token(config, &client, &token_path).await?;
    let mut rpc = RpcClient::connect(&client.client_id).await?;
    rpc.authenticate(&token.access_token).await?;
    let response = rpc.command("GET_GUILDS", serde_json::json!({})).await?;
    parse_guilds(&response.data)
}

/// List bounded guild/channel metadata visible to the authorized desktop
/// client. This does not read message content.
pub async fn list_channels(config: &Config, selected: &str) -> Result<ChannelList> {
    let source = configured_source(config, selected)?;
    let token_path = configured_token_path(source)?;
    let client = read_client_file(required_secure_path(
        source,
        source.oauth_client.as_ref(),
        "OAuth client",
    )?)?;
    let token = read_authorized_token(config, &client, &token_path).await?;
    let mut rpc = RpcClient::connect(&client.client_id).await?;
    rpc.authenticate(&token.access_token).await?;
    let guild_response = rpc.command("GET_GUILDS", serde_json::json!({})).await?;
    let guilds = parse_guilds(&guild_response.data)?;
    let assigned = source
        .servers
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    let mut result = Vec::new();
    let mut truncated = guilds.truncated;
    for guild in guilds.guilds.into_iter().take(MAX_GUILDS) {
        if !assigned.is_empty() && !assigned.contains(guild.id.as_str()) {
            continue;
        }
        let response = rpc
            .command("GET_CHANNELS", serde_json::json!({"guild_id": guild.id}))
            .await?;
        let channels = response
            .data
            .get("channels")
            .and_then(serde_json::Value::as_array)
            .context("Discord RPC returned invalid channel data")?;
        let channels_truncated = channels.len() >= MAX_CHANNELS_PER_GUILD;
        truncated |= channels_truncated;
        let mut summaries = Vec::new();
        for channel in channels.iter().take(MAX_CHANNELS_PER_GUILD) {
            let channel: RpcChannel = serde_json::from_value(channel.clone())
                .context("Discord RPC returned an invalid channel")?;
            summaries.push(ChannelSummary {
                id: validate_snowflake(&channel.id, "channel")?,
                name: sanitize_name(&channel.name, "channel")?,
                kind: channel_kind(channel.channel_type).to_string(),
            });
        }
        result.push(GuildChannels {
            id: guild.id,
            name: guild.name,
            channels: summaries,
            truncated: channels_truncated,
        });
    }
    Ok(ChannelList {
        guilds: result,
        truncated,
    })
}

fn parse_guilds(value: &serde_json::Value) -> Result<ServerList> {
    let guilds = value
        .get("guilds")
        .and_then(serde_json::Value::as_array)
        .context("Discord RPC returned invalid guild data")?;
    let truncated = guilds.len() >= MAX_GUILDS;
    let mut result = Vec::new();
    for guild in guilds.iter().take(MAX_GUILDS) {
        let guild: RpcGuild = serde_json::from_value(guild.clone())
            .context("Discord RPC returned an invalid guild")?;
        result.push(ServerSummary {
            id: validate_snowflake(&guild.id, "guild")?,
            name: sanitize_name(&guild.name, "guild")?,
        });
    }
    Ok(ServerList {
        guilds: result,
        truncated,
    })
}

async fn exchange_code(client: &ClientFile, code: &str) -> Result<StoredToken> {
    let http = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(RPC_TIMEOUT)
        .redirect(Policy::none())
        .build()
        .context("build Discord OAuth client")?;
    let client_secret = client.client_secret.as_deref().context(
        "Discord OAuth client JSON must include client_secret for the authorization-code exchange",
    )?;
    let form = vec![
        ("client_id", client.client_id.as_str()),
        ("code", code),
        ("grant_type", "authorization_code"),
        ("redirect_uri", REDIRECT_URI),
        ("client_secret", client_secret),
    ];
    let response = http
        .post(TOKEN_ENDPOINT)
        .form(&form)
        .send()
        .await
        .context("exchange Discord RPC authorization code")?;
    anyhow::ensure!(
        response.status().is_success(),
        "Discord RPC token exchange failed with status {}",
        response.status().as_u16()
    );
    let token: TokenResponse =
        oauth_common::bounded_json(response, MAX_TOKEN_FILE_BYTES as usize, "Discord").await?;
    oauth_common::validate_credential(
        "Discord access token",
        &token.access_token,
        MAX_CREDENTIAL_BYTES,
    )?;
    if let Some(refresh) = token.refresh_token.as_deref() {
        oauth_common::validate_credential("Discord refresh token", refresh, MAX_CREDENTIAL_BYTES)?;
    }
    let expires = token.expires_in.unwrap_or(604_800);
    Ok(StoredToken {
        access_token: token.access_token,
        token_type: token.token_type,
        refresh_token: token.refresh_token,
        scope: token.scope,
        expiry: (Utc::now() + ChronoDuration::seconds(i64::try_from(expires).unwrap_or(604_800)))
            .to_rfc3339(),
    })
}

async fn read_authorized_token(
    config: &Config,
    client: &ClientFile,
    token_path: &Path,
) -> Result<StoredToken> {
    ensure_outside_filesystem_roots(config, token_path, "token")?;
    let mut token = read_token(token_path)?;
    if !token_expired(&token) {
        return Ok(token);
    }
    let refresh_token = token.refresh_token.as_deref().context(
        "Discord authorization cannot be refreshed and its token has expired; run `cortana authorize-discord` again",
    )?;
    let http = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(RPC_TIMEOUT)
        .redirect(Policy::none())
        .build()
        .context("build Discord OAuth refresh client")?;
    let mut form = vec![
        ("client_id", client.client_id.as_str()),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];
    if let Some(secret) = client.client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    let response = http
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
    let refreshed: TokenResponse =
        oauth_common::bounded_json(response, MAX_TOKEN_FILE_BYTES as usize, "Discord").await?;
    oauth_common::validate_credential(
        "Discord access token",
        &refreshed.access_token,
        MAX_CREDENTIAL_BYTES,
    )?;
    if let Some(refresh) = refreshed.refresh_token.as_deref() {
        oauth_common::validate_credential("Discord refresh token", refresh, MAX_CREDENTIAL_BYTES)?;
    }
    token.access_token = refreshed.access_token;
    token.token_type = refreshed.token_type;
    token.refresh_token = refreshed
        .refresh_token
        .or_else(|| token.refresh_token.clone());
    token.scope = refreshed.scope.or_else(|| token.scope.clone());
    token.expiry = (Utc::now()
        + ChronoDuration::seconds(
            i64::try_from(refreshed.expires_in.unwrap_or(604_800)).unwrap_or(604_800),
        ))
    .to_rfc3339();
    persist_token(token_path, &token)?;
    Ok(token)
}

fn token_expired(token: &StoredToken) -> bool {
    let Ok(expiry) = DateTime::parse_from_rfc3339(&token.expiry) else {
        return true;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    // Refresh slightly early so a long RPC discovery cannot cross expiry.
    expiry.timestamp() <= now.saturating_add(60)
}

fn persist_token(path: &Path, token: &StoredToken) -> Result<()> {
    let body = serde_json::to_vec_pretty(token)?;
    oauth_common::write_owner_only_file(
        path,
        &body,
        "Discord RPC token",
        "cortana-discord-rpc-token",
    )
}

fn read_token(path: &Path) -> Result<StoredToken> {
    oauth_common::reject_symlink(path)?;
    oauth_common::reject_symlink_components(path)?;
    let metadata =
        fs::metadata(path).with_context(|| format!("inspect Discord token {}", path.display()))?;
    oauth_common::ensure_owner_only(&metadata, "Discord RPC token")?;
    anyhow::ensure!(
        metadata.is_file(),
        "Discord RPC token must be a regular file"
    );
    anyhow::ensure!(
        metadata.len() <= MAX_TOKEN_FILE_BYTES,
        "Discord RPC token exceeds 64 KiB"
    );
    let token: StoredToken = serde_json::from_slice(&fs::read(path)?)
        .context("Discord RPC token JSON is invalid; authorize Discord again")?;
    oauth_common::validate_credential(
        "Discord access token",
        &token.access_token,
        MAX_CREDENTIAL_BYTES,
    )?;
    Ok(token)
}

fn read_client_file(path: &Path) -> Result<ClientFile> {
    oauth_common::reject_symlink(path)?;
    oauth_common::reject_symlink_components(path)?;
    let metadata = fs::metadata(path)
        .with_context(|| format!("inspect Discord OAuth client {}", path.display()))?;
    oauth_common::ensure_owner_only(&metadata, "Discord OAuth client")?;
    anyhow::ensure!(
        metadata.is_file(),
        "Discord OAuth client must be a regular file"
    );
    anyhow::ensure!(
        metadata.len() <= MAX_CLIENT_FILE_BYTES,
        "Discord OAuth client exceeds 64 KiB"
    );
    let client: ClientFile = serde_json::from_slice(&fs::read(path)?)
        .context("Discord OAuth client JSON must contain client_id and optional client_secret")?;
    oauth_common::validate_credential("Discord client id", &client.client_id, 2048)?;
    if let Some(secret) = client.client_secret.as_deref() {
        oauth_common::validate_credential("Discord client secret", secret, 4096)?;
    }
    Ok(client)
}

fn configured_source<'a>(config: &'a Config, selected: &str) -> Result<&'a SourceConfig> {
    anyhow::ensure!(
        !selected.is_empty() && selected.len() <= 64,
        "source name is invalid"
    );
    let source = config
        .sources
        .iter()
        .find(|source| source.name == selected)
        .with_context(|| format!("configured source {selected} was not found"))?;
    anyhow::ensure!(
        source.kind == "discord",
        "source {} is not a Discord connector",
        source.name
    );
    Ok(source)
}

fn configured_token_path(source: &SourceConfig) -> Result<PathBuf> {
    let path = source.token.as_ref().with_context(|| {
        format!(
            "Discord source {} requires a private token path",
            source.name
        )
    })?;
    required_secure_path(source, Some(path), "token").map(Path::to_path_buf)
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
            && !path.components().any(|component| matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )),
        "Discord source {} {label} path must be absolute and outside the filesystem root",
        source.name
    );
    Ok(path)
}

fn ensure_outside_filesystem_roots(config: &Config, path: &Path, label: &str) -> Result<()> {
    for source in config.sources.iter().filter(|source| {
        source.kind == "filesystem" && source.root.as_deref().is_some_and(Path::is_absolute)
    }) {
        if let Some(root) = source.root.as_deref() {
            anyhow::ensure!(
                !path.starts_with(root),
                "Discord {label} path must be outside filesystem source {}",
                source.name
            );
        }
    }
    Ok(())
}

fn validate_snowflake(value: &str, label: &str) -> Result<String> {
    anyhow::ensure!(
        !value.is_empty()
            && value.len() <= MAX_SNOWFLAKE_CHARS
            && value.bytes().all(|byte| byte.is_ascii_digit()),
        "Discord {label} returned an invalid id"
    );
    anyhow::ensure!(
        value.parse::<u64>().is_ok_and(|value| value > 0),
        "Discord {label} returned an invalid id"
    );
    Ok(value.to_string())
}

fn sanitize_name(value: &str, label: &str) -> Result<String> {
    let value = value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>()
        .trim()
        .to_string();
    anyhow::ensure!(
        !value.is_empty() && value.chars().count() <= MAX_NAME_CHARS,
        "Discord {label} returned an invalid name"
    );
    Ok(value)
}

fn channel_kind(channel_type: u64) -> &'static str {
    match channel_type {
        0 => "text",
        2 => "voice",
        4 => "category",
        5 => "announcement",
        10 => "announcement-thread",
        11 => "public-thread",
        12 => "private-thread",
        13 => "stage",
        14 => "directory",
        15 => "forum",
        16 => "media",
        _ => "other",
    }
}

fn default_token_type() -> String {
    "Bearer".into()
}

#[cfg(unix)]
struct RpcClient {
    stream: tokio::net::UnixStream,
}

#[cfg(unix)]
impl RpcClient {
    async fn connect(client_id: &str) -> Result<Self> {
        for path in socket_paths() {
            if let Ok(Ok(stream)) =
                tokio::time::timeout(RPC_TIMEOUT, tokio::net::UnixStream::connect(&path)).await
            {
                return Self::handshake(stream, client_id, RPC_TIMEOUT).await;
            }
        }
        bail!("Discord Desktop RPC is unavailable; start Discord and authorize Cortana");
    }

    async fn handshake(
        stream: tokio::net::UnixStream,
        client_id: &str,
        timeout: Duration,
    ) -> Result<Self> {
        let mut client = Self { stream };
        let ready = match tokio::time::timeout(timeout, async {
            client
                .send(
                    OP_HANDSHAKE,
                    serde_json::json!({"v": RPC_VERSION, "client_id": client_id}),
                )
                .await?;
            client.read_response().await
        })
        .await
        {
            Ok(result) => result?,
            Err(_) => bail!(
                "Discord RPC handshake timed out after {} seconds",
                timeout.as_secs_f32()
            ),
        };
        anyhow::ensure!(
            ready.evt.as_deref() == Some("READY"),
            "Discord RPC handshake failed"
        );
        Ok(client)
    }

    async fn authenticate(&mut self, access_token: &str) -> Result<()> {
        let response = self
            .command(
                "AUTHENTICATE",
                serde_json::json!({"access_token": access_token}),
            )
            .await?;
        anyhow::ensure!(
            response.cmd.as_deref() == Some("AUTHENTICATE"),
            "Discord RPC authentication failed"
        );
        Ok(())
    }

    async fn command(&mut self, cmd: &str, args: serde_json::Value) -> Result<RpcResponse> {
        self.command_with_timeout(cmd, args, RPC_TIMEOUT).await
    }

    async fn command_with_timeout(
        &mut self,
        cmd: &str,
        args: serde_json::Value,
        response_timeout: Duration,
    ) -> Result<RpcResponse> {
        let nonce = uuid::Uuid::new_v4().to_string();
        match tokio::time::timeout(response_timeout, async {
            self.send(
                OP_FRAME,
                serde_json::json!({"cmd": cmd, "args": args, "nonce": nonce}),
            )
            .await?;
            loop {
                let response = self.read_response().await?;
                if response.nonce.as_deref() == Some(nonce.as_str()) {
                    if response.evt.as_deref() == Some("ERROR") {
                        bail!("Discord RPC {cmd} failed");
                    }
                    return Ok(response);
                }
            }
        })
        .await
        {
            Ok(result) => result,
            Err(_) => bail!(
                "Discord RPC {cmd} timed out after {} seconds",
                response_timeout.as_secs_f32()
            ),
        }
    }

    async fn send(&mut self, opcode: u32, payload: serde_json::Value) -> Result<()> {
        let body = serde_json::to_vec(&payload)?;
        anyhow::ensure!(
            body.len() <= MAX_FRAME_BYTES,
            "Discord RPC request exceeded safety limit"
        );
        self.stream.write_u32_le(opcode).await?;
        self.stream.write_u32_le(body.len() as u32).await?;
        self.stream.write_all(&body).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn read_response(&mut self) -> Result<RpcResponse> {
        let opcode = self.stream.read_u32_le().await?;
        let length = self.stream.read_u32_le().await? as usize;
        anyhow::ensure!(
            length <= MAX_FRAME_BYTES,
            "Discord RPC response exceeded safety limit"
        );
        let mut body = vec![0; length];
        self.stream.read_exact(&mut body).await?;
        anyhow::ensure!(
            opcode == OP_FRAME,
            "Discord RPC returned an unsupported opcode"
        );
        serde_json::from_slice(&body).context("Discord RPC returned invalid JSON")
    }
}

#[cfg(not(unix))]
struct RpcClient;

#[cfg(not(unix))]
impl RpcClient {
    async fn connect(_client_id: &str) -> Result<Self> {
        bail!("Discord Desktop RPC is supported only on Unix desktop builds")
    }

    async fn authenticate(&mut self, _access_token: &str) -> Result<()> {
        bail!("Discord Desktop RPC is supported only on Unix desktop builds")
    }

    async fn command(&mut self, _cmd: &str, _args: serde_json::Value) -> Result<RpcResponse> {
        bail!("Discord Desktop RPC is supported only on Unix desktop builds")
    }
}

#[cfg(unix)]
fn socket_paths() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for name in ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"] {
        if let Some(value) = std::env::var_os(name) {
            if !value.is_empty() {
                roots.push(PathBuf::from(value));
            }
        }
    }
    roots.push(PathBuf::from("/tmp"));
    roots.dedup();
    roots
        .into_iter()
        .flat_map(|root| (0..10).map(move |index| root.join(format!("discord-ipc-{index}"))))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{StoredToken, parse_guilds, sanitize_name, token_expired, validate_snowflake};

    #[test]
    fn guild_payloads_are_bounded_and_sanitized() {
        let payload = serde_json::json!({
            "guilds": [
                {"id": "123456789", "name": " Engineering\u{0000} "}
            ]
        });
        let result = parse_guilds(&payload).expect("valid guild payload");
        assert_eq!(result.guilds[0].id, "123456789");
        assert_eq!(result.guilds[0].name, "Engineering");
        assert!(!result.truncated);
    }

    #[test]
    fn malformed_discord_identifiers_fail_closed() {
        assert!(validate_snowflake("0", "guild").is_err());
        assert!(validate_snowflake("guild-id", "guild").is_err());
        assert!(sanitize_name("\n\t", "guild").is_err());
        assert!(sanitize_name(&"x".repeat(101), "guild").is_err());
    }

    #[test]
    fn guild_payloads_accept_only_the_documented_shape() {
        assert!(parse_guilds(&serde_json::json!([])).is_err());
        assert!(parse_guilds(&serde_json::json!({"guilds": "not-a-list"})).is_err());
    }

    #[test]
    fn expired_discord_tokens_are_detected_with_a_small_safety_window() {
        let expired = StoredToken {
            access_token: "access".into(),
            token_type: "Bearer".into(),
            refresh_token: Some("refresh".into()),
            scope: None,
            expiry: "2000-01-01T00:00:00Z".into(),
        };
        assert!(token_expired(&expired));

        let malformed = StoredToken {
            expiry: "not-a-timestamp".into(),
            ..expired
        };
        assert!(token_expired(&malformed));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn silent_rpc_commands_fail_closed_at_the_response_deadline() {
        let (client, _server) = tokio::net::UnixStream::pair().expect("Unix stream pair");
        let mut rpc = super::RpcClient { stream: client };
        let error = rpc
            .command_with_timeout(
                "GET_GUILDS",
                serde_json::json!({}),
                Duration::from_millis(1),
            )
            .await
            .expect_err("silent Discord RPC must time out");
        assert!(error.to_string().contains("timed out"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn silent_rpc_handshakes_fail_closed_at_the_handshake_deadline() {
        let (client, _server) = tokio::net::UnixStream::pair().expect("Unix stream pair");
        let error = match super::RpcClient::handshake(client, "client-id", Duration::from_millis(1))
            .await
        {
            Ok(_) => panic!("silent Discord RPC peer must time out during the handshake"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("handshake timed out"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn blocked_rpc_writes_fail_closed_at_the_response_deadline() {
        let (client, _server) = tokio::net::UnixStream::pair().expect("Unix stream pair");
        let mut rpc = super::RpcClient { stream: client };
        let payload = "x".repeat(super::MAX_FRAME_BYTES - 1024);
        let error = rpc
            .command_with_timeout(
                "GET_CHANNELS",
                serde_json::json!({"payload": payload}),
                Duration::from_millis(1),
            )
            .await
            .expect_err("a non-reading Discord peer must time out during the write");
        assert!(error.to_string().contains("timed out"));
    }
}
